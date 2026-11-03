#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Health {
    Critical,
    Warning,
    Unknown,
    Healthy,
}

impl Health {
    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "Kritisch",
            Self::Warning => "Warnung",
            Self::Unknown => "Unbekannt",
            Self::Healthy => "Gesund",
        }
    }

    pub fn css(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Check {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub raw_status: String,
    pub severity: Health,
    pub time: String,
    pub consecutive_fails: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub cpu: Option<f32>,
    pub memory: Option<f32>,
    pub disk: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub client: String,
    pub site: String,
    pub os: String,
    pub health: Health,
    pub online: Option<bool>,
    pub last_response: String,
    pub last_boot: String,
    pub checks: Vec<Check>,
    pub metrics: Metrics,
}

impl Server {
    pub fn issue_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| matches!(c.severity, Health::Critical | Health::Warning))
            .count()
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub servers: Vec<Server>,
    pub updated_at: String,
    pub source_warning: Option<String>,
}
