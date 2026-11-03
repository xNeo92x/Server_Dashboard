use crate::model::{Check, Health, Metrics, Server, Snapshot};

pub fn snapshot() -> Snapshot {
    let specs = [
        ("DC-01", "Praxis Gruppe", Health::Healthy, 18.0, 42.0, 38.0),
        ("APP-01", "Praxis Gruppe", Health::Healthy, 34.0, 61.0, 57.0),
        ("SQL-01", "Praxis Gruppe", Health::Warning, 24.0, 78.0, 72.0),
        ("FS-01", "Praxis Gruppe", Health::Critical, 12.0, 46.0, 88.0),
        ("HV-01", "Infrastructure", Health::Healthy, 38.0, 54.0, 49.0),
        ("BKP-01", "Infrastructure", Health::Healthy, 8.0, 32.0, 65.0),
        ("WEB-01", "Infrastructure", Health::Healthy, 21.0, 37.0, 43.0),
        ("RDS-01", "Praxis Gruppe", Health::Critical, 0.0, 0.0, 0.0),
        ("DC-02", "Infrastructure", Health::Healthy, 11.0, 39.0, 35.0),
    ];
    let servers = specs
        .into_iter()
        .enumerate()
        .map(|(i, (name, client, health, cpu, memory, disk))| {
            let mut checks = vec![Check {
                id: format!("{}-1", i + 1),
                name: "Monitoring / Systemzustand".into(),
                detail: "Alle Prüfungen erfolgreich".into(),
                raw_status: "testok".into(),
                severity: Health::Healthy,
                time: "gerade eben".into(),
                consecutive_fails: 0,
            }];
            if health == Health::Warning {
                checks.push(Check {
                    id: format!("{}-2", i + 1),
                    name: "Performance Monitoring Check - Memory Usage".into(),
                    detail: "Memory Usage: 78%".into(),
                    raw_status: "testalertdelayed".into(),
                    severity: Health::Warning,
                    time: "gerade eben".into(),
                    consecutive_fails: 1,
                });
            }
            if name == "FS-01" {
                checks.push(Check {
                    id: format!("{}-3", i + 1),
                    name: "Disk Space Check - C:".into(),
                    detail: "Disk Usage: 88%".into(),
                    raw_status: "testerror".into(),
                    severity: Health::Critical,
                    time: "gerade eben".into(),
                    consecutive_fails: 3,
                });
            }
            Server {
                id: format!("10{:02}", i + 1),
                name: name.into(),
                client: client.into(),
                site: "Hauptstandort".into(),
                os: "Windows Server 2022".into(),
                health,
                online: Some(name != "RDS-01"),
                last_response: if name == "RDS-01" { "vor 14 Minuten".into() } else { "gerade eben".into() },
                last_boot: "vor 12 Tagen".into(),
                checks,
                metrics: if name == "RDS-01" {
                    Metrics::default()
                } else {
                    Metrics {
                        cpu: Some(cpu),
                        memory: Some(memory),
                        disk: Some(disk),
                    }
                },
            }
        })
        .collect();
    Snapshot {
        servers,
        updated_at: "Demo".into(),
        source_warning: None,
    }
}
