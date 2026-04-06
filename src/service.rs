use crate::config::{Config, SuspendState};
use crate::media::{capture_playback, restart_playback};
use crate::power::{get_idle_threshold, get_power_status, hibernate_enabled, idle_time_secs, suspend_system};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

pub enum WorkerMsg {
    StateChanged {
        enabled: bool,
        use_system_timer: bool,
        resume_playback: bool,
    },
    QuickSuspend,
    Exit,
}

/// Sent from worker → main thread to update label text in the tray menu.
pub struct LabelUpdate {
    pub suspend_state: String,
    pub suspend_after: String,
}

struct SuspendSettings {
    state: SuspendState,
    after_secs: u32,
}

impl Default for SuspendSettings {
    fn default() -> Self {
        Self {
            state: SuspendState::Disabled,
            after_secs: 3600,
        }
    }
}

fn compute_suspend_settings(config: &Config) -> SuspendSettings {
    if !config.use_system_timer {
        return SuspendSettings {
            state: config.manual_suspend_state.clone(),
            after_secs: config.manual_suspend_after as u32,
        };
    }

    let power_state = get_power_status();
    let mut result = SuspendSettings::default();
    result.after_secs = 60 * 60;

    let sleep_thresh = get_idle_threshold(&power_state, &SuspendState::Sleep);
    if sleep_thresh > 0 {
        result.after_secs = sleep_thresh;
        result.state = SuspendState::Sleep;
    }

    if hibernate_enabled() {
        let hib_thresh = get_idle_threshold(&power_state, &SuspendState::Hibernate);
        if hib_thresh > 0
            && (result.state == SuspendState::Disabled || hib_thresh < result.after_secs)
        {
            result.after_secs = hib_thresh;
            result.state = SuspendState::Hibernate;
        }
    }

    result
}

pub fn run_worker(
    receiver: Receiver<WorkerMsg>,
    label_sender: std::sync::mpsc::Sender<LabelUpdate>,
    mut config: Config,
    config_path: PathBuf,
) {
    let mut enabled = config.enabled;
    let mut resume_playback = config.resume_playback;
    let mut quick_suspend_pending = false;

    let mut suspend_settings = compute_suspend_settings(&config);
    let _ = label_sender.send(LabelUpdate {
        suspend_state: format!("      Suspend: {}", suspend_settings.state),
        suspend_after: format!("      After: {}s", suspend_settings.after_secs),
    });

    loop {
        let check_interval = Duration::from_secs_f64(config.check_interval);

        match receiver.recv_timeout(check_interval) {
            Ok(WorkerMsg::StateChanged {
                enabled: new_enabled,
                use_system_timer: new_system,
                resume_playback: new_resume,
            }) => {
                enabled = new_enabled;
                resume_playback = new_resume;
                config.enabled = new_enabled;
                config.use_system_timer = new_system;
                config.resume_playback = new_resume;
                let _ = config.save(&config_path);
                suspend_settings = compute_suspend_settings(&config);
                let _ = label_sender.send(LabelUpdate {
                    suspend_state: format!("      Suspend: {}", suspend_settings.state),
                    suspend_after: format!("      After: {}s", suspend_settings.after_secs),
                });
                continue;
            }
            Ok(WorkerMsg::QuickSuspend) => {
                quick_suspend_pending = true;
            }
            Ok(WorkerMsg::Exit) => break,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let should_suspend = if quick_suspend_pending {
            quick_suspend_pending = false;
            true
        } else {
            enabled
                && suspend_settings.state != SuspendState::Disabled
                && idle_time_secs() > suspend_settings.after_secs as f64
        };

        if should_suspend {
            let snapshots = if resume_playback {
                capture_playback()
            } else {
                Vec::new()
            };

            if let Err(e) = suspend_system(&suspend_settings.state) {
                log::warn!("Suspend failed: {e}");
            }

            if resume_playback && !snapshots.is_empty() {
                let delay = config.resume_playback_delay;
                std::thread::spawn(move || {
                    restart_playback(snapshots, delay);
                });
            }

            for (program, _) in &config.restarts {
                let _ = std::process::Command::new(program)
                    .creation_flags(0x08000000)
                    .spawn();
            }
        }
    }
}
