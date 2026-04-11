use crate::config::{Config, RestartType, SuspendState};
use crate::media::{capture_playback, restart_playback};
use crate::power::{get_idle_threshold, get_power_status, has_blocking_power_requests, hibernate_enabled, idle_time_secs, suspend_system};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// Settle delay (seconds) after killing processes — applied both before suspend and on post-resume
/// kill-then-launch sequences — to allow the OS to fully clean up before the next action.
const KILL_SETTLE_SECS: u64 = 2;

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
            after_secs: 0,
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

fn kill_program(program: &str) {
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    log::info!("Killing process: {name}");
    match std::process::Command::new("taskkill")
        .args(["/F", "/IM", name])
        .creation_flags(0x08000000)
        .output()
    {
        Ok(output) if output.status.success() => {
            log::info!("Killed {name}");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("taskkill {name} failed ({}): {stderr}", output.status);
        }
        Err(e) => {
            log::warn!("Failed to run taskkill for {name}: {e}");
        }
    }
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
    log::info!(
        "Worker started: enabled={} suspend_state={} suspend_after={}s check_interval={}s",
        enabled,
        suspend_settings.state,
        suspend_settings.after_secs,
        config.check_interval
    );
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
                log::info!(
                    "State change received: enabled={new_enabled} use_system_timer={new_system} resume_playback={new_resume}"
                );
                enabled = new_enabled;
                resume_playback = new_resume;
                config.enabled = new_enabled;
                config.use_system_timer = new_system;
                config.resume_playback = new_resume;
                let _ = config.save(&config_path);
                suspend_settings = compute_suspend_settings(&config);
                log::info!(
                    "Suspend settings updated: state={} after={}s",
                    suspend_settings.state,
                    suspend_settings.after_secs
                );
                let _ = label_sender.send(LabelUpdate {
                    suspend_state: format!("      Suspend: {}", suspend_settings.state),
                    suspend_after: format!("      After: {}s", suspend_settings.after_secs),
                });
                continue;
            }
            Ok(WorkerMsg::QuickSuspend) => {
                log::info!("Quick-suspend flag set");
                quick_suspend_pending = true;
            }
            Ok(WorkerMsg::Exit) => {
                log::info!("Exit message received; worker shutting down");
                break;
            }
            Err(RecvTimeoutError::Disconnected) => {
                log::warn!("Channel disconnected; worker shutting down");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        let idle = idle_time_secs();

        let should_suspend = if quick_suspend_pending {
            log::info!("Quick-suspend triggered (bypasses idle and power-request checks)");
            quick_suspend_pending = false;
            true
        } else if !enabled {
            log::debug!("Check: skipping — service disabled (idle={idle:.1}s)");
            false
        } else if suspend_settings.state == SuspendState::Disabled {
            log::debug!("Check: skipping — suspend state is Disabled (idle={idle:.1}s)");
            false
        } else if idle <= suspend_settings.after_secs as f64 {
            let remaining = suspend_settings.after_secs as f64 - idle;
            log::debug!(
                "Check: not yet idle — {idle:.1}s / {}s threshold ({remaining:.1}s remaining)",
                suspend_settings.after_secs
            );
            false
        } else if config.respect_power_requests
            && has_blocking_power_requests(&config.ignored_power_requests)
        {
            log::info!(
                "Check: idle threshold met ({idle:.1}s) but a blocking power request is active — skipping suspend"
            );
            false
        } else {
            log::info!(
                "Check: idle threshold met ({idle:.1}s >= {}s) — suspending as {:?}",
                suspend_settings.after_secs,
                suspend_settings.state
            );
            true
        };

        if should_suspend {
            let snapshots = if resume_playback {
                log::debug!("Capturing media playback state before suspend");
                capture_playback()
            } else {
                Vec::new()
            };

            let mut killed_any = false;
            for (program, restart_type) in &config.restarts {
                if *restart_type == RestartType::KillBeforeSleep {
                    kill_program(program);
                    killed_any = true;
                }
            }
            if killed_any {
                log::debug!(
                    "Waiting {}s for killed processes to exit before suspending",
                    KILL_SETTLE_SECS
                );
                std::thread::sleep(Duration::from_secs(KILL_SETTLE_SECS));

                // Re-check conditions: user activity or a new power request during the settle
                // window should cancel the pending suspend.
                let idle_after_settle = idle_time_secs();
                let suspend_after = suspend_settings.after_secs as f64;
                if idle_after_settle < suspend_after {
                    log::info!(
                        "Suspend cancelled after settle delay: idle time dropped to {:.1}s (threshold {}s)",
                        idle_after_settle,
                        suspend_settings.after_secs
                    );
                    continue;
                }
                if config.respect_power_requests
                    && has_blocking_power_requests(&config.ignored_power_requests)
                {
                    log::info!(
                        "Suspend cancelled after settle delay: blocking power request detected"
                    );
                    continue;
                }
            }

            log::info!("Initiating suspend: {:?}", suspend_settings.state);
            match suspend_system(&suspend_settings.state) {
                Err(e) => {
                    log::warn!("Suspend failed: {e}");
                    continue;
                }
                Ok(()) => {
                    log::info!("Resumed from suspend");
                }
            }

            if resume_playback && !snapshots.is_empty() {
                log::debug!(
                    "Scheduling playback resume for {} session(s) after {}s delay",
                    snapshots.len(),
                    config.resume_playback_delay
                );
                let delay = config.resume_playback_delay;
                std::thread::spawn(move || {
                    restart_playback(snapshots, delay);
                });
            }

            for (program, restart_type) in &config.restarts {
                match restart_type {
                    RestartType::LaunchAfterSleep | RestartType::KillBeforeSleep => {
                        log::info!("Launching app after resume: {program}");
                        let _ = std::process::Command::new(program)
                            .creation_flags(0x08000000)
                            .spawn();
                    }
                    RestartType::RestartAfterSleep => {
                        kill_program(program);
                        log::debug!(
                            "Waiting {}s for killed process to exit before launching: {program}",
                            KILL_SETTLE_SECS
                        );
                        std::thread::sleep(Duration::from_secs(KILL_SETTLE_SECS));
                        log::info!("Launching app after resume: {program}");
                        let _ = std::process::Command::new(program)
                            .creation_flags(0x08000000)
                            .spawn();
                    }
                }
            }
        }
    }

    log::info!("Worker thread exited");
}
