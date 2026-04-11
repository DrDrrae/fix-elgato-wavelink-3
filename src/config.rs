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
    #[serde(rename = "launch_after_sleep")]
    LaunchAfterSleep,
    #[serde(rename = "kill_before_sleep")]
    KillBeforeSleep,
    #[serde(rename = "restart_after_sleep")]
    RestartAfterSleep,
}

/// Verbosity level for the log file. Maps directly to the `log` crate's `LevelFilter`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    #[default]
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn  => log::LevelFilter::Warn,
            LogLevel::Info  => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LogLevel::Error => "error",
            LogLevel::Warn  => "warn",
            LogLevel::Info  => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        write!(f, "{s}")
    }
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
    /// Write a log file next to the exe. Overwritten on each launch.
    #[serde(default)]
    pub log_to_file: bool,
    /// Verbosity of the log file when `log_to_file = true`.
    /// Allowed values: "error", "warn", "info", "debug", "trace".
    #[serde(default)]
    pub log_level: LogLevel,
    /// When true, `powercfg /requests` is checked before each suspend.If any
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
            log_to_file: false,
            log_level: LogLevel::Debug,
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
                        if !cfg.manual_suspend_after.is_finite() || !cfg.check_interval.is_finite() {
                            let defaults = Config::default();
                            if !cfg.manual_suspend_after.is_finite() {
                                log::warn!(
                                    "manual_suspend_after is not finite ({}); using default {}",
                                    cfg.manual_suspend_after,
                                    defaults.manual_suspend_after
                                );
                                cfg.manual_suspend_after = defaults.manual_suspend_after;
                            }
                            if !cfg.check_interval.is_finite() {
                                log::warn!(
                                    "check_interval is not finite ({}); using default {}",
                                    cfg.check_interval,
                                    defaults.check_interval
                                );
                                cfg.check_interval = defaults.check_interval;
                            }
                        }
                        if cfg.manual_suspend_after < 60.0 {
                            log::debug!(
                                "Clamping manual_suspend_after {} → 60",
                                cfg.manual_suspend_after
                            );
                            cfg.manual_suspend_after = 60.0;
                        }
                        if cfg.check_interval < 1.0 {
                            log::debug!(
                                "Clamping check_interval {} → 1",
                                cfg.check_interval
                            );
                            cfg.check_interval = 1.0;
                        }
                        cfg.manual_suspend_after = cfg.manual_suspend_after.max(60.0);
                        cfg.check_interval = cfg.check_interval.max(1.0);
                        log::debug!("Config loaded from {}", path.display());
                        log::debug!("  enabled={} use_system_timer={} manual_suspend_after={} manual_suspend_state={}",
                            cfg.enabled, cfg.use_system_timer, cfg.manual_suspend_after, cfg.manual_suspend_state);
                        log::debug!("  check_interval={} resume_playback={} resume_playback_delay={}",
                            cfg.check_interval, cfg.resume_playback, cfg.resume_playback_delay);
                        log::debug!("  suspend_button={} log_to_file={} log_level={}",
                            cfg.suspend_button, cfg.log_to_file, cfg.log_level);
                        log::debug!("  respect_power_requests={} ignored_power_requests={:?}",
                            cfg.respect_power_requests, cfg.ignored_power_requests);
                        log::debug!("  restarts={:?}", cfg.restarts.keys().collect::<Vec<_>>());
                        cfg
                    }
                    Err(e) => {
                        log::warn!("Failed to parse config at {}: {e}", path.display());
                        log::warn!("Using default config");
                        Config::default()
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read config at {}: {e}", path.display());
                    log::warn!("Using default config");
                    Config::default()
                }
            }
        } else {
            log::info!("No config file found at {}; creating defaults", path.display());
            Config::default()
        }
    }

    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        log::debug!("Saving config to {}", path.display());
        match toml::to_string_pretty(self) {
            Ok(content) => std::fs::write(path, content),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    }
}
