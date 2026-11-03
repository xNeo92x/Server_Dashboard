use crate::{
    api::{ApiClient, ApiOptions},
    demo,
    model::{Health, Server, Snapshot},
    security::EncryptedSecret,
    Args,
};
use glib::ControlFlow;
use gtk::{gdk, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const PAGE_SIZE: usize = 10;
const REFRESH_SECONDS: u64 = 120;
const PAGE_SECONDS: u64 = 22;

#[derive(Debug)]
enum WorkerMessage {
    Success(Snapshot),
    Error(String),
}

struct Session {
    secret: Option<Arc<EncryptedSecret>>,
    options: Option<ApiOptions>,
    snapshot: Option<Snapshot>,
    busy: bool,
    page: usize,
    demo: bool,
}

pub fn build(app: &gtk::Application, args: Args) {
    install_css();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Server Dashboard")
        .default_width(1600)
        .default_height(940)
        .build();
    window.add_css_class("window-root");

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(180)
        .build();
    window.set_child(Some(&stack));

    let login = LoginWidgets::new(&args);
    stack.add_named(&login.root, Some("login"));

    let dashboard = DashboardWidgets::new(&window);
    stack.add_named(&dashboard.root, Some("dashboard"));

    let session = Rc::new(RefCell::new(Session {
        secret: None,
        options: None,
        snapshot: None,
        busy: false,
        page: 0,
        demo: args.demo,
    }));

    let (tx, rx) = mpsc::channel::<WorkerMessage>();

    {
        let session = session.clone();
        let login = login.clone();
        let tx = tx.clone();
        login.connect_button.connect_clicked(move |_| {
            if session.borrow().busy {
                return;
            }

            login.set_error("");
            login.set_busy(true);

            let host = login.host.text().trim().to_string();
            let site_ids = csv_ids(&login.site_ids.text());
            let device_ids = csv_ids(&login.device_ids.text());
            let mut token = Zeroizing::new(login.token.text().as_bytes().to_vec());
            login.token.set_text("");

            match EncryptedSecret::seal(&mut token) {
                Ok(secret) => {
                    token.zeroize();
                    let secret = Arc::new(secret);
                    let options = ApiOptions {
                        host,
                        site_ids,
                        device_ids,
                    };
                    {
                        let mut state = session.borrow_mut();
                        state.secret = Some(secret.clone());
                        state.options = Some(options.clone());
                        state.busy = true;
                        state.demo = false;
                    }
                    spawn_fetch(secret, options, tx.clone());
                }
                Err(err) => {
                    token.zeroize();
                    login.set_busy(false);
                    login.set_error(&err);
                }
            }
        });
    }

    {
        let session = session.clone();
        let stack = stack.clone();
        let login = login.clone();
        let dashboard = dashboard.clone();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            while let Ok(message) = rx.try_recv() {
                let mut state = session.borrow_mut();
                state.busy = false;
                login.set_busy(false);
                dashboard.set_refreshing(false);

                match message {
                    WorkerMessage::Success(snapshot) => {
                        state.page = 0;
                        state.snapshot = Some(snapshot.clone());
                        dashboard.render(&snapshot, 0);
                        stack.set_visible_child_name("dashboard");
                    }
                    WorkerMessage::Error(err) => {
                        if state.snapshot.is_none() {
                            state.secret = None;
                            state.options = None;
                            login.set_error(&err);
                            stack.set_visible_child_name("login");
                        } else {
                            dashboard.set_source_error(&err);
                        }
                    }
                }
            }
            ControlFlow::Continue
        });
    }

    {
        let session = session.clone();
        let dashboard = dashboard.clone();
        let tx = tx.clone();
        dashboard.refresh.connect_clicked(move |_| {
            let mut state = session.borrow_mut();
            if state.busy || state.demo {
                return;
            }
            let (Some(secret), Some(options)) = (state.secret.clone(), state.options.clone()) else {
                return;
            };
            state.busy = true;
            dashboard.set_refreshing(true);
            spawn_fetch(secret, options, tx.clone());
        });
    }

    {
        let session = session.clone();
        let stack = stack.clone();
        let login = login.clone();
        dashboard.disconnect.connect_clicked(move |_| {
            let mut state = session.borrow_mut();
            state.secret = None;
            state.options = None;
            state.snapshot = None;
            state.busy = false;
            state.page = 0;
            login.token.set_text("");
            login.set_error("");
            stack.set_visible_child_name("login");
        });
    }

    {
        let session = session.clone();
        let dashboard = dashboard.clone();
        glib::timeout_add_local(Duration::from_secs(PAGE_SECONDS), move || {
            let mut state = session.borrow_mut();
            let Some(snapshot) = state.snapshot.clone() else {
                return ControlFlow::Continue;
            };
            let pages = page_count(snapshot.servers.len());
            if pages > 1 {
                state.page = (state.page + 1) % pages;
                dashboard.render(&snapshot, state.page);
            }
            ControlFlow::Continue
        });
    }

    {
        let session = session.clone();
        let dashboard = dashboard.clone();
        let tx = tx.clone();
        glib::timeout_add_local(Duration::from_secs(REFRESH_SECONDS), move || {
            let mut state = session.borrow_mut();
            if state.busy || state.demo {
                return ControlFlow::Continue;
            }
            let (Some(secret), Some(options)) = (state.secret.clone(), state.options.clone()) else {
                return ControlFlow::Continue;
            };
            state.busy = true;
            dashboard.set_refreshing(true);
            spawn_fetch(secret, options, tx.clone());
            ControlFlow::Continue
        });
    }

    if args.demo {
        let snapshot = demo::snapshot();
        session.borrow_mut().snapshot = Some(snapshot.clone());
        dashboard.render(&snapshot, 0);
        stack.set_visible_child_name("dashboard");
    } else {
        stack.set_visible_child_name("login");
    }

    let fullscreen_state = Rc::new(Cell::new(args.fullscreen));
    if args.fullscreen {
        window.fullscreen();
    }
    {
        let window = window.clone();
        let fullscreen_state = fullscreen_state.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::F11 {
                let next = !fullscreen_state.get();
                fullscreen_state.set(next);
                if next {
                    window.fullscreen();
                } else {
                    window.unfullscreen();
                }
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && fullscreen_state.get() {
                fullscreen_state.set(false);
                window.unfullscreen();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    window.present();
}

fn spawn_fetch(
    secret: Arc<EncryptedSecret>,
    options: ApiOptions,
    tx: mpsc::Sender<WorkerMessage>,
) {
    thread::spawn(move || {
        let result =
            ApiClient::new(&options, secret).and_then(|client| client.fetch_snapshot(&options));
        let _ = tx.send(match result {
            Ok(snapshot) => WorkerMessage::Success(snapshot),
            Err(err) => WorkerMessage::Error(err),
        });
    });
}

fn csv_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .map(ToOwned::to_owned)
        .collect()
}

fn page_count(server_count: usize) -> usize {
    (server_count.max(1) + PAGE_SIZE - 1) / PAGE_SIZE
}

#[derive(Clone)]
struct LoginWidgets {
    root: gtk::Box,
    host: gtk::Entry,
    site_ids: gtk::Entry,
    device_ids: gtk::Entry,
    token: gtk::PasswordEntry,
    connect_button: gtk::Button,
    spinner: gtk::Spinner,
    error: gtk::Label,
}

impl LoginWidgets {
    fn new(args: &Args) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("login-background");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let brand = gtk::Box::new(gtk::Orientation::Vertical, 12);
        brand.add_css_class("login-brand");
        brand.set_size_request(330, -1);
        brand.set_vexpand(true);

        let brand_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let logo = gtk::Label::new(Some("S"));
        logo.add_css_class("brand-logo");
        let brand_name = gtk::Label::new(Some("Server Dashboard"));
        brand_name.add_css_class("brand-name");
        brand_row.append(&logo);
        brand_row.append(&brand_name);
        brand.append(&brand_row);

        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        brand.append(&spacer);

        let caption = gtk::Label::new(Some(
            "Native N-sight monitoring\nfor a dedicated Linux wallboard",
        ));
        caption.add_css_class("muted");
        caption.set_halign(gtk::Align::Start);
        caption.set_wrap(true);
        brand.append(&caption);
        root.append(&brand);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("login-content");
        content.set_hexpand(true);
        content.set_vexpand(true);
        content.set_valign(gtk::Align::Center);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 16);
        card.add_css_class("login-card");
        card.set_halign(gtk::Align::Center);
        card.set_size_request(560, -1);

        let eyebrow = gtk::Label::new(Some("N-ABLE N-SIGHT RMM"));
        eyebrow.add_css_class("overline");
        eyebrow.set_halign(gtk::Align::Start);
        card.append(&eyebrow);

        let title = gtk::Label::new(Some("Mit N-sight verbinden"));
        title.add_css_class("login-title");
        title.set_halign(gtk::Align::Start);
        card.append(&title);

        let intro = gtk::Label::new(Some(
            "Der API-Token wird nicht gespeichert. Zwischen API-Aufrufen liegt er nur verschlüsselt im Arbeitsspeicher.",
        ));
        intro.add_css_class("muted");
        intro.set_halign(gtk::Align::Start);
        intro.set_wrap(true);
        card.append(&intro);

        let host = entry(&args.api_host, "https://wwwgermany1.systemmonitor.eu.com");
        card.append(&field("API-Server", &host));

        let site_ids = entry(&args.site_ids.join(","), "z. B. 12345, 67890");
        card.append(&field("Site-IDs (optional)", &site_ids));

        let device_ids = entry(&args.device_ids.join(","), "z. B. 11111, 22222");
        card.append(&field("Server-IDs (optional)", &device_ids));

        let token = gtk::PasswordEntry::new();
        token.set_show_peek_icon(false);
        token.add_css_class("text-field");
        card.append(&field("API-Token", &token));

        let error = gtk::Label::new(None);
        error.add_css_class("error-text");
        error.set_halign(gtk::Align::Start);
        error.set_wrap(true);
        error.set_visible(false);
        card.append(&error);

        let action = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let spinner = gtk::Spinner::new();
        let connect_button = gtk::Button::with_label("Verbinden");
        connect_button.add_css_class("primary-button");
        connect_button.set_hexpand(true);
        action.append(&spinner);
        action.append(&connect_button);
        card.append(&action);

        content.append(&card);
        root.append(&content);

        Self {
            root,
            host,
            site_ids,
            device_ids,
            token,
            connect_button,
            spinner,
            error,
        }
    }

    fn set_busy(&self, busy: bool) {
        self.host.set_sensitive(!busy);
        self.site_ids.set_sensitive(!busy);
        self.device_ids.set_sensitive(!busy);
        self.token.set_sensitive(!busy);
        self.connect_button.set_sensitive(!busy);
        if busy {
            self.spinner.start();
            self.connect_button.set_label("Verbindung wird geprüft …");
        } else {
            self.spinner.stop();
            self.connect_button.set_label("Verbinden");
        }
    }

    fn set_error(&self, text: &str) {
        self.error.set_text(text);
        self.error.set_visible(!text.is_empty());
    }
}

fn entry(initial: &str, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_placeholder_text(Some(placeholder));
    entry.add_css_class("text-field");
    entry
}

fn field<W: IsA<gtk::Widget>>(name: &str, widget: &W) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let label = gtk::Label::new(Some(name));
    label.add_css_class("field-label");
    label.set_halign(gtk::Align::Start);
    box_.append(&label);
    box_.append(widget);
    box_
}

#[derive(Clone)]
struct DashboardWidgets {
    root: gtk::Box,
    hero: gtk::Box,
    hero_icon: gtk::Label,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    problems: gtk::Box,
    problem_list: gtk::Box,
    rows: gtk::Box,
    source: gtk::Label,
    updated: gtk::Label,
    page_label: gtk::Label,
    side_total: gtk::Label,
    side_healthy: gtk::Label,
    side_warning: gtk::Label,
    side_critical: gtk::Label,
    side_unknown: gtk::Label,
    figure_total: gtk::Label,
    figure_healthy: gtk::Label,
    figure_attention: gtk::Label,
    figure_checks: gtk::Label,
    refresh: gtk::Button,
    disconnect: gtk::Button,
    parent: gtk::ApplicationWindow,
}

impl DashboardWidgets {
    fn new(parent: &gtk::ApplicationWindow) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("app-background");
        root.set_hexpand(true);
        root.set_vexpand(true);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("sidebar");
        sidebar.set_size_request(224, -1);
        sidebar.set_vexpand(true);

        let workspace = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        workspace.add_css_class("workspace");
        let logo = gtk::Label::new(Some("S"));
        logo.add_css_class("brand-logo");
        let name = gtk::Label::new(Some("Server Dashboard"));
        name.add_css_class("workspace-name");
        name.set_hexpand(true);
        name.set_halign(gtk::Align::Start);
        workspace.append(&logo);
        workspace.append(&name);
        sidebar.append(&workspace);

        sidebar.append(&nav_row("▦", "Übersicht", true));

        let status_heading = gtk::Label::new(Some("STATUS"));
        status_heading.add_css_class("sidebar-heading");
        status_heading.set_halign(gtk::Align::Start);
        sidebar.append(&status_heading);

        let side_total = gtk::Label::new(Some("0"));
        sidebar.append(&count_row("●", "Alle Server", "neutral", &side_total));
        let side_healthy = gtk::Label::new(Some("0"));
        sidebar.append(&count_row("●", "Gesund", "healthy", &side_healthy));
        let side_warning = gtk::Label::new(Some("0"));
        sidebar.append(&count_row("●", "Warnung", "warning", &side_warning));
        let side_critical = gtk::Label::new(Some("0"));
        sidebar.append(&count_row("●", "Kritisch", "critical", &side_critical));
        let side_unknown = gtk::Label::new(Some("0"));
        sidebar.append(&count_row("●", "Unbekannt", "unknown", &side_unknown));

        let source_heading = gtk::Label::new(Some("QUELLE"));
        source_heading.add_css_class("sidebar-heading");
        source_heading.set_halign(gtk::Align::Start);
        sidebar.append(&source_heading);
        sidebar.append(&nav_row("◈", "N-able N-sight", false));

        let side_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side_spacer.set_vexpand(true);
        sidebar.append(&side_spacer);

        let auto = gtk::Label::new(Some("◷  Auto-Refresh · 2 min"));
        auto.add_css_class("sidebar-foot");
        auto.set_halign(gtk::Align::Start);
        sidebar.append(&auto);
        root.append(&sidebar);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_hexpand(true);
        panel.set_vexpand(true);

        let topbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        topbar.add_css_class("topbar");
        let crumb = gtk::Label::new(Some("▦  Übersicht  /  Server"));
        crumb.add_css_class("crumb");
        crumb.set_hexpand(true);
        crumb.set_halign(gtk::Align::Start);

        let refresh = gtk::Button::with_label("↻  Aktualisieren");
        refresh.add_css_class("topbar-button");
        let disconnect = gtk::Button::with_label("Sitzung beenden");
        disconnect.add_css_class("topbar-button");

        topbar.append(&crumb);
        topbar.append(&refresh);
        topbar.append(&disconnect);
        panel.append(&topbar);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.add_css_class("main-content");
        content.set_margin_top(26);
        content.set_margin_bottom(32);
        content.set_margin_start(30);
        content.set_margin_end(30);

        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        hero.add_css_class("status-hero");
        let hero_icon = gtk::Label::new(Some("●"));
        hero_icon.add_css_class("hero-icon");
        hero_icon.set_valign(gtk::Align::Start);
        let hero_copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        hero_copy.set_hexpand(true);
        let hero_title = gtk::Label::new(Some("Status wird geladen"));
        hero_title.add_css_class("hero-title");
        hero_title.set_halign(gtk::Align::Start);
        let hero_subtitle = gtk::Label::new(Some("N-sight wird abgefragt …"));
        hero_subtitle.add_css_class("muted");
        hero_subtitle.set_halign(gtk::Align::Start);
        hero_subtitle.set_wrap(true);
        hero_copy.append(&hero_title);
        hero_copy.append(&hero_subtitle);
        hero.append(&hero_icon);
        hero.append(&hero_copy);
        content.append(&hero);

        let source = gtk::Label::new(None);
        source.add_css_class("warning-banner");
        source.set_halign(gtk::Align::Fill);
        source.set_wrap(true);
        source.set_visible(false);
        content.append(&source);

        let problems = gtk::Box::new(gtk::Orientation::Vertical, 8);
        problems.set_visible(false);
        problems.append(&section_heading(
            "Probleme",
            "Aktive Warnungen und kritische Server",
        ));
        let problem_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        problem_list.add_css_class("inbox");
        problems.append(&problem_list);
        content.append(&problems);

        let figures_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        figures_section.append(&section_heading(
            "Heute",
            "Aktueller Snapshot aus N-sight",
        ));
        let figures = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        figures.add_css_class("figures");
        let (total_card, figure_total) = figure_card("Server");
        let (healthy_card, figure_healthy) = figure_card("Gesund");
        let (attention_card, figure_attention) = figure_card("Aufmerksamkeit");
        let (checks_card, figure_checks) = figure_card("Checks");
        figures.append(&total_card);
        figures.append(&healthy_card);
        figures.append(&attention_card);
        figures.append(&checks_card);
        figures_section.append(&figures);
        content.append(&figures_section);

        let servers_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        servers_section.append(&section_heading(
            "Server",
            "Klick öffnet die Monitoring-Details",
        ));
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.add_css_class("server-table");
        table.append(&table_header());
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.append(&rows);
        servers_section.append(&table);
        content.append(&servers_section);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let updated = gtk::Label::new(None);
        updated.add_css_class("muted");
        updated.set_hexpand(true);
        updated.set_halign(gtk::Align::Start);
        let page_label = gtk::Label::new(None);
        page_label.add_css_class("muted");
        footer.append(&updated);
        footer.append(&page_label);
        content.append(&footer);

        scroll.set_child(Some(&content));
        panel.append(&scroll);
        root.append(&panel);

        Self {
            root,
            hero,
            hero_icon,
            hero_title,
            hero_subtitle,
            problems,
            problem_list,
            rows,
            source,
            updated,
            page_label,
            side_total,
            side_healthy,
            side_warning,
            side_critical,
            side_unknown,
            figure_total,
            figure_healthy,
            figure_attention,
            figure_checks,
            refresh,
            disconnect,
            parent: parent.clone(),
        }
    }

    fn render(&self, snapshot: &Snapshot, page: usize) {
        clear_box(&self.problem_list);
        clear_box(&self.rows);
        self.refresh.set_sensitive(true);

        let healthy = count_health(snapshot, Health::Healthy);
        let warning = count_health(snapshot, Health::Warning);
        let critical = count_health(snapshot, Health::Critical);
        let unknown = count_health(snapshot, Health::Unknown);
        let attention = warning + critical;
        let checks: usize = snapshot.servers.iter().map(|s| s.checks.len()).sum();

        self.side_total.set_text(&snapshot.servers.len().to_string());
        self.side_healthy.set_text(&healthy.to_string());
        self.side_warning.set_text(&warning.to_string());
        self.side_critical.set_text(&critical.to_string());
        self.side_unknown.set_text(&unknown.to_string());

        self.figure_total.set_text(&snapshot.servers.len().to_string());
        self.figure_healthy.set_text(&healthy.to_string());
        self.figure_attention.set_text(&attention.to_string());
        self.figure_checks.set_text(&checks.to_string());

        let overall = if critical > 0 {
            Health::Critical
        } else if warning > 0 {
            Health::Warning
        } else if !snapshot.servers.is_empty() && unknown == snapshot.servers.len() {
            Health::Unknown
        } else {
            Health::Healthy
        };
        set_health_class(&self.hero, overall);
        set_health_class(&self.hero_icon, overall);

        match overall {
            Health::Healthy => {
                self.hero_title.set_text("Alle überwachten Server sind stabil");
                self.hero_subtitle.set_text(&format!(
                    "{} von {} Servern gesund · letzter Snapshot {}",
                    healthy,
                    snapshot.servers.len(),
                    snapshot.updated_at
                ));
            }
            Health::Warning => {
                self.hero_title
                    .set_text(&format!("{attention} Server benötigen Aufmerksamkeit"));
                self.hero_subtitle.set_text(&format!(
                    "{warning} Warnung(en), {critical} kritisch · letzter Snapshot {}",
                    snapshot.updated_at
                ));
            }
            Health::Critical => {
                self.hero_title
                    .set_text(&format!("{critical} kritische Server erkannt"));
                self.hero_subtitle.set_text(&format!(
                    "{warning} Warnung(en) zusätzlich · letzter Snapshot {}",
                    snapshot.updated_at
                ));
            }
            Health::Unknown => {
                self.hero_title.set_text("Serverstatus ist noch nicht eindeutig");
                self.hero_subtitle.set_text(&format!(
                    "Für {unknown} Server fehlen ausreichende Zustandsdaten · letzter Snapshot {}",
                    snapshot.updated_at
                ));
            }
        }

        let problematic: Vec<&Server> = snapshot
            .servers
            .iter()
            .filter(|server| matches!(server.health, Health::Warning | Health::Critical))
            .collect();

        self.problems.set_visible(!problematic.is_empty());
        for server in problematic.iter().take(4) {
            self.problem_list
                .append(&problem_row(server, &self.parent));
        }
        if problematic.len() > 4 {
            let more = gtk::Label::new(Some(&format!(
                "+ {} weitere Server mit Aufmerksamkeit",
                problematic.len() - 4
            )));
            more.add_css_class("more-problems");
            more.set_halign(gtk::Align::Start);
            self.problem_list.append(&more);
        }

        let pages = page_count(snapshot.servers.len());
        let page = page.min(pages.saturating_sub(1));
        let start = page * PAGE_SIZE;
        for server in snapshot.servers.iter().skip(start).take(PAGE_SIZE) {
            self.rows.append(&server_row(server, &self.parent));
        }

        self.updated.set_text(&format!(
            "Aktualisiert {} · {} Server · Seitenwechsel nach {} s",
            snapshot.updated_at,
            snapshot.servers.len(),
            PAGE_SECONDS
        ));
        self.page_label
            .set_text(&format!("Seite {} / {}", page + 1, pages));

        if let Some(warning) = &snapshot.source_warning {
            self.set_source_error(warning);
        } else {
            self.source.set_visible(false);
        }
    }

    fn set_source_error(&self, text: &str) {
        self.source.set_text(text);
        self.source.set_visible(true);
    }

    fn set_refreshing(&self, refreshing: bool) {
        self.refresh.set_sensitive(!refreshing);
        if refreshing {
            self.refresh.set_label("↻  Lädt …");
        } else {
            self.refresh.set_label("↻  Aktualisieren");
        }
    }
}

fn count_health(snapshot: &Snapshot, health: Health) -> usize {
    snapshot
        .servers
        .iter()
        .filter(|server| server.health == health)
        .count()
}

fn nav_row(icon: &str, title: &str, selected: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("nav-row");
    if selected {
        row.add_css_class("selected");
    }
    let icon = gtk::Label::new(Some(icon));
    icon.add_css_class("nav-icon");
    let title = gtk::Label::new(Some(title));
    title.add_css_class("nav-title");
    title.set_hexpand(true);
    title.set_halign(gtk::Align::Start);
    row.append(&icon);
    row.append(&title);
    row
}

fn count_row(icon: &str, title: &str, class: &str, count: &gtk::Label) -> gtk::Box {
    let row = nav_row(icon, title, false);
    row.add_css_class(class);
    count.add_css_class("nav-count");
    row.append(count);
    row
}

fn section_heading(title: &str, hint: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(title));
    title.add_css_class("section-title");
    title.set_halign(gtk::Align::Start);
    let hint = gtk::Label::new(Some(hint));
    hint.add_css_class("section-hint");
    hint.set_hexpand(true);
    hint.set_halign(gtk::Align::End);
    row.append(&title);
    row.append(&hint);
    row
}

fn figure_card(title: &str) -> (gtk::Box, gtk::Label) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
    card.add_css_class("figure-card");
    card.set_hexpand(true);
    let value = gtk::Label::new(Some("0"));
    value.add_css_class("figure-value");
    value.set_halign(gtk::Align::Start);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("figure-label");
    label.set_halign(gtk::Align::Start);
    card.append(&value);
    card.append(&label);
    (card, value)
}

fn table_header() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("table-header");
    row.append(&header_cell("Server", 240));
    row.append(&header_cell("Status", 105));
    row.append(&header_cell("CPU", 110));
    row.append(&header_cell("RAM", 110));
    row.append(&header_cell("Datenträger", 120));
    row.append(&header_cell("Checks", 88));
    let last = gtk::Label::new(Some("Letzte Rückmeldung"));
    last.add_css_class("table-header-label");
    last.set_hexpand(true);
    last.set_halign(gtk::Align::Start);
    row.append(&last);
    row
}

fn header_cell(text: &str, width: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("table-header-label");
    label.set_size_request(width, -1);
    label.set_halign(gtk::Align::Start);
    label
}

fn problem_row(server: &Server, parent: &gtk::ApplicationWindow) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("problem-row");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let dot = gtk::Label::new(Some("●"));
    dot.add_css_class("problem-dot");
    dot.add_css_class(server.health.css());

    let copy = gtk::Box::new(gtk::Orientation::Vertical, 1);
    copy.set_hexpand(true);
    let name = gtk::Label::new(Some(&server.name));
    name.add_css_class("problem-title");
    name.set_halign(gtk::Align::Start);

    let issue_count = server.issue_count();
    let detail = gtk::Label::new(Some(&format!(
        "{} · {} Problem{} in {} Checks",
        server_context(server),
        issue_count,
        if issue_count == 1 { "" } else { "e" },
        server.checks.len()
    )));
    detail.add_css_class("section-hint");
    detail.set_halign(gtk::Align::Start);

    copy.append(&name);
    copy.append(&detail);
    row.append(&dot);
    row.append(&copy);
    row.append(&status_pill(server.health));
    button.set_child(Some(&row));

    let server = server.clone();
    let parent = parent.clone();
    button.connect_clicked(move |_| show_detail(&parent, &server));
    button
}

fn server_row(server: &Server, parent: &gtk::ApplicationWindow) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("server-row");
    button.add_css_class(server.health.css());

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(10);
    row.set_margin_end(10);

    let server_cell = gtk::Box::new(gtk::Orientation::Vertical, 1);
    server_cell.set_size_request(230, -1);
    let name = gtk::Label::new(Some(&server.name));
    name.add_css_class("server-name");
    name.set_halign(gtk::Align::Start);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let context_text = server_context(server);
    let context = gtk::Label::new(Some(&context_text));
    context.add_css_class("server-context");
    context.set_halign(gtk::Align::Start);
    context.set_ellipsize(gtk::pango::EllipsizeMode::End);
    server_cell.append(&name);
    server_cell.append(&context);
    row.append(&server_cell);

    let state = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    state.set_size_request(105, -1);
    state.set_valign(gtk::Align::Center);
    state.append(&status_pill(server.health));
    row.append(&state);

    row.append(&metric_cell(server.metrics.cpu, 110));
    row.append(&metric_cell(server.metrics.memory, 110));
    row.append(&metric_cell(server.metrics.disk, 120));

    let checks = gtk::Box::new(gtk::Orientation::Vertical, 1);
    checks.set_size_request(88, -1);
    checks.set_valign(gtk::Align::Center);
    let checks_value = gtk::Label::new(Some(&server.checks.len().to_string()));
    checks_value.add_css_class("cell-value");
    checks_value.set_halign(gtk::Align::Start);
    let issues = gtk::Label::new(Some(&format!("{} offen", server.issue_count())));
    issues.add_css_class("cell-subtitle");
    issues.set_halign(gtk::Align::Start);
    checks.append(&checks_value);
    checks.append(&issues);
    row.append(&checks);

    let last_text = if server.last_response.is_empty() {
        "—".to_string()
    } else {
        server.last_response.clone()
    };
    let last = gtk::Label::new(Some(&last_text));
    last.add_css_class("last-response");
    last.set_hexpand(true);
    last.set_halign(gtk::Align::Start);
    last.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&last);

    button.set_child(Some(&row));

    let server = server.clone();
    let parent = parent.clone();
    button.connect_clicked(move |_| show_detail(&parent, &server));
    button
}

fn metric_cell(value: Option<f32>, width: i32) -> gtk::Box {
    let cell = gtk::Box::new(gtk::Orientation::Vertical, 3);
    cell.set_size_request(width, -1);
    cell.set_valign(gtk::Align::Center);

    let text = gtk::Label::new(None);
    text.add_css_class("cell-value");
    text.set_halign(gtk::Align::Start);

    let bar = gtk::ProgressBar::new();
    bar.add_css_class("metric-bar");
    bar.set_size_request(width - 20, -1);

    match value {
        Some(value) => {
            text.set_text(&format!("{value:.0}%"));
            bar.set_fraction((value / 100.0).clamp(0.0, 1.0) as f64);
        }
        None => {
            text.set_text("—");
            bar.set_fraction(0.0);
            bar.add_css_class("empty");
        }
    }

    cell.append(&text);
    cell.append(&bar);
    cell
}

fn status_pill(health: Health) -> gtk::Label {
    let label = gtk::Label::new(Some(health.label()));
    label.add_css_class("status-pill");
    label.add_css_class(health.css());
    label
}

fn server_context(server: &Server) -> String {
    let context = [server.client.as_str(), server.site.as_str()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if context.is_empty() {
        format!("ID {}", server.id)
    } else {
        context
    }
}

fn set_health_class<W: IsA<gtk::Widget>>(widget: &W, health: Health) {
    for class in ["healthy", "warning", "critical", "unknown"] {
        widget.remove_css_class(class);
    }
    widget.add_css_class(health.css());
}

fn show_detail(parent: &gtk::ApplicationWindow, server: &Server) {
    let window = gtk::Window::builder()
        .title(format!("{} – Details", server.name))
        .transient_for(parent)
        .modal(true)
        .default_width(900)
        .default_height(720)
        .build();
    window.add_css_class("detail-window");

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("detail-root");

    let topbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    topbar.add_css_class("topbar");
    let crumb = gtk::Label::new(Some("Server  /  Monitoring-Details"));
    crumb.add_css_class("crumb");
    crumb.set_hexpand(true);
    crumb.set_halign(gtk::Align::Start);
    let close = gtk::Button::with_label("Schließen");
    close.add_css_class("topbar-button");
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    topbar.append(&crumb);
    topbar.append(&close);
    root.append(&topbar);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(26);
    content.set_margin_end(26);

    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some(&server.name));
    title.add_css_class("detail-title");
    title.set_halign(gtk::Align::Start);

    let os = if server.os.is_empty() {
        "Betriebssystem unbekannt".to_string()
    } else {
        server.os.clone()
    };
    let boot = if server.last_boot.is_empty() {
        "Boot: —".to_string()
    } else {
        format!("Boot: {}", server.last_boot)
    };
    let meta = gtk::Label::new(Some(&format!(
        "{} · {} · {}",
        server_context(server),
        os,
        boot
    )));
    meta.add_css_class("muted");
    meta.set_halign(gtk::Align::Start);
    meta.set_wrap(true);

    copy.append(&title);
    copy.append(&meta);
    heading.append(&copy);
    heading.append(&status_pill(server.health));
    content.append(&heading);

    let metrics = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    metrics.add_css_class("figures");
    metrics.append(&detail_metric("CPU", server.metrics.cpu));
    metrics.append(&detail_metric("RAM", server.metrics.memory));
    metrics.append(&detail_metric("Datenträger", server.metrics.disk));
    content.append(&metrics);

    content.append(&section_heading(
        "Checks",
        "Letzte von N-sight gelieferte Zustände",
    ));

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("check-list");

    if server.checks.is_empty() {
        let empty = gtk::Label::new(Some("Keine Check-Details verfügbar."));
        empty.add_css_class("empty-state");
        empty.set_halign(gtk::Align::Start);
        list.append(&empty);
    } else {
        for check in &server.checks {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("check-row");

            let dot = gtk::Label::new(Some("●"));
            dot.add_css_class("problem-dot");
            dot.add_css_class(check.severity.css());

            let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
            copy.set_hexpand(true);
            let name = gtk::Label::new(Some(&check.name));
            name.add_css_class("check-title");
            name.set_halign(gtk::Align::Start);
            name.set_wrap(true);
            copy.append(&name);

            if !check.detail.is_empty() {
                let detail = gtk::Label::new(Some(&check.detail));
                detail.add_css_class("muted");
                detail.set_halign(gtk::Align::Start);
                detail.set_wrap(true);
                copy.append(&detail);
            }

            let extra = gtk::Label::new(Some(&format!(
                "ID {} · {} · Folgefehler {} · {}",
                check.id, check.raw_status, check.consecutive_fails, check.time
            )));
            extra.add_css_class("tiny");
            extra.set_halign(gtk::Align::Start);
            copy.append(&extra);

            row.append(&dot);
            row.append(&copy);
            row.append(&status_pill(check.severity));
            list.append(&row);
        }
    }

    scroll.set_child(Some(&list));
    content.append(&scroll);
    root.append(&content);
    window.set_child(Some(&root));
    window.present();
}

fn detail_metric(title: &str, value: Option<f32>) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
    card.add_css_class("figure-card");
    card.set_hexpand(true);
    let value_text = value
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| "—".into());
    let value = gtk::Label::new(Some(&value_text));
    value.add_css_class("figure-value");
    value.set_halign(gtk::Align::Start);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("figure-label");
    label.set_halign(gtk::Align::Start);
    card.append(&value);
    card.append(&label);
    card
}

fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("../assets/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
