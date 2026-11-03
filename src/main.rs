mod api;
mod demo;
mod model;
mod security;
mod ui;

use gtk::prelude::*;
use std::env;

#[derive(Clone, Debug)]
pub struct Args {
    pub api_host: String,
    pub site_ids: Vec<String>,
    pub device_ids: Vec<String>,
    pub fullscreen: bool,
    pub demo: bool,
}

fn main() {
    if let Err(err) = security::harden_process() {
        eprintln!("Sicherheitsinitialisierung fehlgeschlagen: {err}");
        std::process::exit(2);
    }

    let args = parse_args();
    let app = gtk::Application::builder()
        .application_id("io.github.xNeo92x.ServerDashboard")
        .build();
    app.connect_activate(move |app| ui::build(app, args.clone()));
    app.run();
}

fn parse_args() -> Args {
    let mut out = Args {
        api_host: "https://wwwgermany1.systemmonitor.eu.com".into(),
        site_ids: Vec::new(),
        device_ids: Vec::new(),
        fullscreen: false,
        demo: false,
    };
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--api-host" => if let Some(v) = it.next() { out.api_host = v; },
            "--site-ids" => if let Some(v) = it.next() { out.site_ids = split_ids(&v); },
            "--device-ids" => if let Some(v) = it.next() { out.device_ids = split_ids(&v); },
            "--fullscreen" => out.fullscreen = true,
            "--demo" => out.demo = true,
            "--help" | "-h" => {
                println!("server-dashboard [--api-host https://HOST] [--site-ids ID,ID] [--device-ids ID,ID] [--fullscreen] [--demo]\n\nDer API-Token wird absichtlich NICHT als Kommandozeilenargument akzeptiert.");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    out
}

fn split_ids(value: &str) -> Vec<String> {
    value.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .map(ToOwned::to_owned)
        .collect()
}
