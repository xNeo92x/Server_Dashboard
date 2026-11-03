use crate::{
    model::{Check, Health, Metrics, Server, Snapshot},
    security::EncryptedSecret,
};
use chrono::Local;
use regex::Regex;
use reqwest::{blocking::Client, redirect::Policy};
use roxmltree::{Document, Node};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    str,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use url::Url;

const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
const MIN_CALL_GAP: Duration = Duration::from_millis(1100);

#[derive(Clone, Debug)]
pub struct ApiOptions {
    pub host: String,
    pub device_ids: Vec<String>,
    pub site_ids: Vec<String>,
}

#[derive(Clone, Debug)]
struct Summary {
    id: String,
    name: String,
    client: String,
    site: String,
    status: String,
    online: Option<bool>,
    active_247: bool,
    status_247: i32,
    dsc_active: bool,
    dsc_status: i32,
    missed_247: bool,
    last_response: String,
    last_boot: String,
}

pub struct ApiClient {
    host: Url,
    http: Client,
    secret: Arc<EncryptedSecret>,
    last_call: Option<Instant>,
}

impl ApiClient {
    pub fn new(options: &ApiOptions, secret: Arc<EncryptedSecret>) -> Result<Self, String> {
        let host = validate_origin(&options.host)?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(12))
            .redirect(Policy::none())
            .https_only(true)
            .no_proxy()
            .user_agent("Server-Dashboard-Rust/0.3")
            .build()
            .map_err(safe_reqwest_error)?;
        Ok(Self {
            host,
            http,
            secret,
            last_call: None,
        })
    }

    pub fn fetch_snapshot(mut self, options: &ApiOptions) -> Result<Snapshot, String> {
        let mut summaries = self.discover(options)?;
        summaries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let selected: HashSet<&str> = options.device_ids.iter().map(String::as_str).collect();
        if !selected.is_empty() {
            summaries.retain(|s| selected.contains(s.id.as_str()));
        }

        let mut servers = Vec::with_capacity(summaries.len());
        let mut detail_failures = 0usize;
        for summary in summaries {
            match self.call(
                "list_device_monitoring_details",
                &[("deviceid", summary.id.as_str())],
            ) {
                Ok(xml) => match parse_detail(&summary, &xml) {
                    Ok(server) => servers.push(server),
                    Err(_) => {
                        detail_failures += 1;
                        servers.push(server_from_summary(summary));
                    }
                },
                Err(_) => {
                    detail_failures += 1;
                    servers.push(server_from_summary(summary));
                }
            }
        }

        servers.sort_by_key(|s| (s.health, s.name.to_lowercase()));
        Ok(Snapshot {
            servers,
            updated_at: unix_time_label(),
            source_warning: (detail_failures > 0).then(|| {
                format!(
                    "{detail_failures} Detailabfrage(n) fehlgeschlagen – Status kann unvollständig sein."
                )
            }),
        })
    }

    fn discover(&mut self, options: &ApiOptions) -> Result<Vec<Summary>, String> {
        let mut by_id = HashMap::<String, Summary>::new();
        let site_ids: Vec<String> = if options.site_ids.is_empty() {
            vec!["0".into()]
        } else {
            options.site_ids.clone()
        };

        for site_id in site_ids {
            let xml = self.call("list_servers", &[("siteid", site_id.as_str())])?;
            for summary in parse_server_list(&xml)? {
                by_id.insert(summary.id.clone(), summary);
            }
        }
        Ok(by_id.into_values().collect())
    }

    fn call(&mut self, service: &str, params: &[(&str, &str)]) -> Result<String, String> {
        match service {
            "list_servers" | "list_device_monitoring_details" => {}
            _ => return Err("Nicht erlaubter N-sight API-Service.".into()),
        }

        if let Some(last) = self.last_call {
            let elapsed = last.elapsed();
            if elapsed < MIN_CALL_GAP {
                thread::sleep(MIN_CALL_GAP - elapsed);
            }
        }
        self.last_call = Some(Instant::now());

        let token = self.secret.expose()?;
        let token_str = str::from_utf8(token.as_slice())
            .map_err(|_| "Der API-Token ist nicht UTF-8-kodiert.".to_string())?;
        let mut url = self
            .host
            .join("api/")
            .map_err(|_| "Ungültige API-URL.".to_string())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("apikey", token_str);
            query.append_pair("service", service);
            for (key, value) in params {
                query.append_pair(key, value);
            }
        }
        drop(token);

        let response = self
            .http
            .get(url)
            .header("Accept", "application/xml")
            .send()
            .map_err(safe_reqwest_error)?;

        if !response.status().is_success() {
            return Err(format!(
                "N-sight antwortete mit HTTP {}.",
                response.status().as_u16()
            ));
        }
        if response.content_length().is_some_and(|n| n > MAX_RESPONSE) {
            return Err("Die N-sight-Antwort ist größer als 8 MiB.".into());
        }

        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "N-sight-Antwort konnte nicht gelesen werden.".to_string())?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err("Die N-sight-Antwort ist größer als 8 MiB.".into());
        }
        let xml = String::from_utf8(bytes)
            .map_err(|_| "N-sight lieferte kein gültiges UTF-8 XML.".to_string())?;
        validate_xml_envelope(&xml)?;
        Ok(xml)
    }
}

fn validate_origin(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let url = Url::parse(trimmed).map_err(|_| "Die API-Serveradresse ist ungültig.".to_string())?;
    let clean_path = url.path().is_empty() || url.path() == "/";
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !clean_path
    {
        return Err(
            "API-Server muss ein HTTPS-Ursprung ohne Pfad, Query, Benutzername oder Passwort sein."
                .into(),
        );
    }
    Ok(url)
}

fn safe_reqwest_error(err: reqwest::Error) -> String {
    if err.is_timeout() {
        "Zeitüberschreitung bei der N-sight-Verbindung.".into()
    } else if err.is_connect() {
        "Verbindung zu N-sight konnte nicht hergestellt werden.".into()
    } else if let Some(status) = err.status() {
        format!("N-sight antwortete mit HTTP {}.", status.as_u16())
    } else {
        // Intentionally do not format the reqwest error itself: it may carry the request URL,
        // and N-sight authentication places the API token in that URL's query string.
        "Die N-sight-Anfrage ist fehlgeschlagen.".into()
    }
}

fn validate_xml_envelope(xml: &str) -> Result<(), String> {
    let upper = xml.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err("DTD/Entities in API-Antwort abgelehnt.".into());
    }
    let doc = Document::parse(xml).map_err(|_| "Ungültiges XML von N-sight.".to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name("result") {
        return Err("Unerwartete N-sight API-Antwort.".into());
    }
    if root.attribute("status").unwrap_or("OK").to_ascii_uppercase() != "OK"
        || root.children().any(|n| n.has_tag_name("error"))
    {
        return Err(
            "N-sight hat die Anfrage abgelehnt. API-Token und Berechtigungen prüfen.".into(),
        );
    }
    Ok(())
}

fn parse_server_list(xml: &str) -> Result<Vec<Summary>, String> {
    let doc = Document::parse(xml)
        .map_err(|_| "Serverliste ist kein gültiges XML.".to_string())?;
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("server")) {
        let id = child_text(node, "serverid")
            .or_else(|| child_text(node, "id"))
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let status = child_text(node, "status")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let online = match child_text(node, "online").as_deref() {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ if matches!(status.as_str(), "offline" | "overdue" | "ispdownown") => Some(false),
            _ if matches!(status.as_str(), "ok" | "error") => Some(true),
            _ => None,
        };
        out.push(Summary {
            id,
            name: child_text(node, "name").unwrap_or_else(|| "Unbenannter Server".into()),
            client: ancestor_text(node, "client", "name").unwrap_or_default(),
            site: ancestor_text(node, "site", "name").unwrap_or_default(),
            status,
            online,
            active_247: child_text(node, "active_247").as_deref() == Some("1"),
            status_247: child_i32(node, "status_247", -1),
            dsc_active: child_text(node, "dsc_active").as_deref() == Some("1"),
            dsc_status: child_i32(node, "dsc_status", -1),
            missed_247: child_text(node, "missed_247").as_deref() == Some("1"),
            last_response: child_text(node, "utc_time_247").unwrap_or_default(),
            last_boot: child_text(node, "last_boot_time").unwrap_or_default(),
        });
    }
    Ok(out)
}

fn parse_detail(summary: &Summary, xml: &str) -> Result<Server, String> {
    let doc = Document::parse(xml)
        .map_err(|_| "Detailantwort ist kein gültiges XML.".to_string())?;
    let node = doc
        .descendants()
        .find(|n| n.has_tag_name("server"))
        .ok_or_else(|| "Keine Serverdetails in API-Antwort.".to_string())?;

    let checks: Vec<Check> = node
        .descendants()
        .filter(|n| n.has_tag_name("check"))
        .map(parse_check)
        .collect();
    let health = derive_health(summary, &checks, true);
    let metrics = extract_metrics(&checks);

    Ok(Server {
        id: summary.id.clone(),
        name: child_text(node, "name")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| summary.name.clone()),
        client: summary.client.clone(),
        site: summary.site.clone(),
        os: child_text(node, "os").unwrap_or_default(),
        health,
        online: summary.online,
        last_response: child_text(node, "lastresponse")
            .unwrap_or_else(|| summary.last_response.clone()),
        last_boot: child_text(node, "lastboot").unwrap_or_else(|| summary.last_boot.clone()),
        checks,
        metrics,
    })
}

fn server_from_summary(summary: Summary) -> Server {
    let health = derive_health(&summary, &[], false);
    Server {
        id: summary.id,
        name: summary.name,
        client: summary.client,
        site: summary.site,
        os: String::new(),
        health,
        online: summary.online,
        last_response: summary.last_response,
        last_boot: summary.last_boot,
        checks: Vec::new(),
        metrics: Metrics::default(),
    }
}

fn parse_check(node: Node<'_, '_>) -> Check {
    let raw = child_text(node, "checkstatus").unwrap_or_default();
    let severity = check_health(&raw);
    Check {
        id: child_text(node, "checkid").unwrap_or_default(),
        name: child_text(node, "description").unwrap_or_else(|| "Monitoring-Check".into()),
        detail: child_text(node, "extra").unwrap_or_default(),
        raw_status: raw,
        severity,
        time: child_text(node, "datetime").unwrap_or_default(),
        consecutive_fails: child_text(node, "consecutive_fails")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    }
}

fn check_health(raw: &str) -> Health {
    match raw.to_ascii_lowercase().as_str() {
        "testerror" | "testerror_inactive" | "overdue_error" => Health::Critical,
        "testalertdelayed"
        | "testcleared"
        | "overdue"
        | "test_inactive"
        | "testok_inactive" => Health::Warning,
        "testok" => Health::Healthy,
        _ => Health::Unknown,
    }
}

fn derive_health(summary: &Summary, checks: &[Check], have_detail: bool) -> Health {
    if summary.online == Some(false) || summary.missed_247 {
        return Health::Critical;
    }
    if checks.iter().any(|c| c.severity == Health::Critical) {
        return Health::Critical;
    }
    if summary.status_247 == 1 || (summary.dsc_active && summary.dsc_status == 1) {
        return Health::Critical;
    }
    if matches!(
        summary.status.as_str(),
        "error" | "overdue" | "ispdownown" | "offline"
    ) {
        return Health::Critical;
    }
    if checks.iter().any(|c| c.severity == Health::Warning) {
        return Health::Warning;
    }
    if !have_detail {
        return Health::Unknown;
    }
    if summary.active_247 && summary.status_247 != 5 {
        return Health::Unknown;
    }
    if summary.dsc_active && summary.dsc_status != 5 {
        return Health::Unknown;
    }
    if matches!(summary.status.as_str(), "maintenance" | "inactive") {
        return Health::Unknown;
    }
    if summary.online.is_none() && checks.is_empty() {
        return Health::Unknown;
    }
    Health::Healthy
}

fn extract_metrics(checks: &[Check]) -> Metrics {
    Metrics {
        cpu: find_metric(checks, MetricKind::Cpu),
        memory: find_metric(checks, MetricKind::Memory),
        disk: find_metric(checks, MetricKind::Disk),
    }
}

#[derive(Clone, Copy)]
enum MetricKind {
    Cpu,
    Memory,
    Disk,
}

fn find_metric(checks: &[Check], kind: MetricKind) -> Option<f32> {
    let used_re = Regex::new(
        r"(?i)(?:current\s+)?(?:cpu\s+)?(?:usage|utilization|used|load|auslastung|belegt|verwendet)\s*(?:is|:|=)?\s*(\d+(?:[.,]\d+)?)\s*%",
    ).ok()?;
    let free_re = Regex::new(
        r"(?i)(?:free\s+space|space\s+free|available\s+space|frei(?:er|e|es)?\s+speicher(?:platz)?)\s*(?:is|:|=)?\s*(\d+(?:[.,]\d+)?)\s*%",
    ).ok()?;
    for check in checks {
        let name = check.name.to_ascii_lowercase();
        let relevant = match kind {
            MetricKind::Cpu => {
                name.contains("cpu")
                    || name.contains("processor")
                    || name.contains("prozessor")
            }
            MetricKind::Memory => {
                name.contains("memory")
                    || name.contains("ram")
                    || name.contains("arbeitsspeicher")
                    || name.contains("speicherauslastung")
            }
            MetricKind::Disk => {
                name.contains("disk")
                    || name.contains("drive")
                    || name.contains("volume")
                    || name.contains("laufwerk")
                    || name.contains("datenträger")
                    || name.contains("filesystem")
            }
        };
        if !relevant {
            continue;
        }
        if let Some(caps) = used_re.captures(&check.detail) {
            if let Some(v) = parse_percent(caps.get(1).map(|m| m.as_str())) {
                return Some(v);
            }
        }
        if matches!(kind, MetricKind::Disk) {
            if let Some(caps) = free_re.captures(&check.detail) {
                if let Some(v) = parse_percent(caps.get(1).map(|m| m.as_str())) {
                    return Some((100.0 - v).clamp(0.0, 100.0));
                }
            }
        }
    }
    None
}

fn parse_percent(raw: Option<&str>) -> Option<f32> {
    let value: f32 = raw?.replace(',', ".").parse().ok()?;
    (0.0..=100.0).contains(&value).then_some(value)
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn child_i32(node: Node<'_, '_>, tag: &str, default: i32) -> i32 {
    child_text(node, tag)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn ancestor_text(node: Node<'_, '_>, ancestor_tag: &str, child_tag: &str) -> Option<String> {
    let ancestor = node
        .ancestors()
        .find(|a| a.is_element() && a.tag_name().name() == ancestor_tag)?;
    child_text(ancestor, child_tag)
}

fn unix_time_label() -> String {
    Local::now().format("%d.%m.%Y %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_used_and_free_metrics() {
        let mk = |name: &str, detail: &str| Check {
            id: "1".into(),
            name: name.into(),
            detail: detail.into(),
            raw_status: "testok".into(),
            severity: Health::Healthy,
            time: String::new(),
            consecutive_fails: 0,
        };
        let checks = vec![
            mk("Performance Monitoring Check - CPU Usage", "CPU Usage: 23%"),
            mk(
                "Performance Monitoring Check - Memory Usage",
                "Memory Usage: 61%",
            ),
            mk("Disk Space Check - C:", "Free Space: 12%"),
        ];
        let m = extract_metrics(&checks);
        assert_eq!(m.cpu, Some(23.0));
        assert_eq!(m.memory, Some(61.0));
        assert_eq!(m.disk, Some(88.0));
    }

    #[test]
    fn rejects_non_https_origin() {
        assert!(validate_origin("http://example.invalid").is_err());
        assert!(validate_origin("https://example.invalid/path").is_err());
        assert!(validate_origin("https://example.invalid").is_ok());
    }
}
