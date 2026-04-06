use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SuspendState {
    Sleep,
    Hibernate,
    #[default]
    Disabled,
}

impl std::fmt::Display for SuspendState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuspendState::Sleep => write!(f, "sleep"),
            SuspendState::Hibernate => write!(f, "hibernate"),
            SuspendState::Disabled => write!(f, "disabled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RestartType {
    Popen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub use_system_timer: bool,
    #[serde(default = "default_suspend_after")]
    pub manual_suspend_after: f64,
    #[serde(default)]
    pub manual_suspend_state: SuspendState,
    #[serde(default = "default_check_interval")]
    pub check_interval: f64,
    #[serde(default)]
    pub resume_playback: bool,
    #[serde(default = "default_resume_delay")]
    pub resume_playback_delay: u32,
    #[serde(default)]
    pub suspend_button: bool,
    /// When true, `powercfg /requests` is checked before each suspend. If any
    /// active power requests are found that are *not* in `ignored_power_requests`,
    /// the suspend is skipped for this cycle (the system is considered busy).
    #[serde(default = "default_true")]
    pub respect_power_requests: bool,
    /// List of substrings to ignore when scanning `powercfg /requests` output.
    /// Any request whose `[DRIVER]/[PROCESS]` line or description contains one of
    /// these strings is treated as non-blocking. Defaults to ignoring the
    /// "Legacy Kernel Caller" audio-stream entry produced by apps like Elgato Wave Link.
    #[serde(default = "default_ignored_requests")]
    pub ignored_power_requests: Vec<String>,
    #[serde(default)]
    pub restarts: HashMap<String, RestartType>,
}

fn default_true() -> bool {
    true
}
fn default_suspend_after() -> f64 {
    600.0
}
fn default_check_interval() -> f64 {
    60.0
}
fn default_resume_delay() -> u32 {
    1
}
fn default_ignored_requests() -> Vec<String> {
    vec!["Legacy Kernel Caller".to_string()]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            use_system_timer: true,
            manual_suspend_after: 600.0,
            manual_suspend_state: SuspendState::Sleep,
            check_interval: 60.0,
            resume_playback: false,
            resume_playback_delay: 1,
            suspend_button: false,
            respect_power_requests: true,
            ignored_power_requests: default_ignored_requests(),
            restarts: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Self {
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match toml::from_str::<Config>(&content) {
                    Ok(mut cfg) => {
                        cfg.manual_suspend_after = cfg.manual_suspend_after.max(60.0);
                        cfg.check_interval = cfg.check_interval.max(1.0);
                        cfg
                    }
                    Err(e) => {
                        log::warn!("Failed to parse config: {e}");
                        Config::default()
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read config: {e}");
                    Config::default()
                }
            }
        } else {
            Config::default()
        }
    }

    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        match toml::to_string_pretty(self) {
            Ok(content) => std::fs::write(path, content),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    }
}
