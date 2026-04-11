#![windows_subsystem = "windows"]

mod config;
mod icons;
mod media;
mod power;
mod service;

use clap::Parser;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use service::{LabelUpdate, WorkerMsg};
use simplelog::{CombinedLogger, Config as LogConfig, SharedLogger, WriteLogger};
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MB_ICONERROR, MB_OK, MB_SYSTEMMODAL, MessageBoxW, PeekMessageW,
    TranslateMessage, MSG, PM_REMOVE,
};
use windows::core::w;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(about = "Simple suspend service to force hibernate/sleep after a set time.")]
struct Args {
    /// Specify the configuration file or folder path.
    #[arg(long, value_name = "CONFIG_PATH")]
    config: Option<PathBuf>,
}

fn resolve_config_path(arg: Option<PathBuf>) -> PathBuf {
    if let Some(path) = arg {
        let abs = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(&path)
        };
        if abs.is_file() {
            return abs;
        }
        return abs.join("config.toml");
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    exe_dir.join("config.toml")
}

fn init_logging(config: &config::Config, config_path: &PathBuf) {
    let level = config.log_level.to_level_filter();
    let log_cfg = LogConfig::default();

    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();

    if config.log_to_file {
        let log_path = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("sleeper_service.log");

        match File::create(&log_path) {
            Ok(file) => {
                loggers.push(WriteLogger::new(level, log_cfg, file));
                // Bootstrap message goes straight to the file logger after init,
                // so we store the path and log it below.
                let _ = CombinedLogger::init(loggers);
                log::info!("sleeper_service v{VERSION} starting");
                log::info!("Log file: {}", log_path.display());
                log::info!("Log level: {level}");
                return;
            }
            Err(e) => {
                // Can't open log file — fall through to env_logger so startup
                // isn't completely silent.
                let _ = env_logger::try_init();
                log::warn!("Could not create log file at {}: {e}", log_path.display());
                return;
            }
        }
    }

    // File logging disabled — honour RUST_LOG env var for terminal sessions.
    let _ = env_logger::try_init();
}

fn main() {
    let args = Args::parse();

    // Single-instance guard via named mutex
    let _mutex = unsafe {
        match CreateMutexW(None, true, w!("Global\\sleeper_service_mutex")) {
            Ok(h) => {
                if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
                    std::process::exit(0);
                }
                h
            }
            Err(_) => std::process::exit(1),
        }
    };

    let config_path = resolve_config_path(args.config);
    let config = config::Config::load(&config_path);
    let _ = config.save(&config_path);

    // Initialise logging now that we have the config.
    init_logging(&config, &config_path);

    log::info!("sleeper_service v{VERSION}");
    log::info!("Config: {}", config_path.display());

    // Require admin elevation when respect_power_requests is enabled, because
    // `powercfg /requests` needs elevated privileges and cannot be bypassed in code.
    if config.respect_power_requests && !power::is_elevated() {
        log::error!(
            "respect_power_requests = true but process is not elevated; exiting with user prompt"
        );
        unsafe {
            MessageBoxW(
                None,
                w!("The 'respect_power_requests' setting is enabled, but \
powercfg /requests requires administrator privileges to run.\r\n\r\n\
To resolve this, choose one of:\r\n\
  \u{2022}  Run sleeper_service.exe as Administrator\r\n\
  \u{2022}  Set respect_power_requests = false in config.toml to disable this check\r\n\r\n\
The application will now exit."),
                w!("sleeper_service \u{2014} Administrator required"),
                MB_OK | MB_ICONERROR | MB_SYSTEMMODAL,
            );
        }
        std::process::exit(1);
    }

    // Build tray icons
    log::debug!("Building tray icons");
    let (enabled_rgba, disabled_rgba) = icons::create_icons();
    let enabled_icon =
        Icon::from_rgba(enabled_rgba, 64, 64).expect("failed to create enabled icon");
    let disabled_icon =
        Icon::from_rgba(disabled_rgba, 64, 64).expect("failed to create disabled icon");

    // Build tray menu
    log::debug!("Building tray menu");
    let menu = Menu::new();
    let enabled_item = CheckMenuItem::new("Enabled", true, config.enabled, None);
    let use_system_item = CheckMenuItem::new("Use System Timers", true, config.use_system_timer, None);
    let suspend_state_item = MenuItem::new("      Suspend: disabled", false, None);
    let suspend_after_item = MenuItem::new("      After: 0s", false, None);
    let resume_item = CheckMenuItem::new("Resume playback", true, config.resume_playback, None);
    let suspend_btn: Option<MenuItem> = if config.suspend_button {
        Some(MenuItem::new("Suspend now(-ish)!", true, None))
    } else {
        None
    };
    let exit_item = MenuItem::new("Exit", true, None);

    menu.append(&enabled_item).unwrap();
    menu.append(&use_system_item).unwrap();
    menu.append(&suspend_state_item).unwrap();
    menu.append(&suspend_after_item).unwrap();
    menu.append(&resume_item).unwrap();
    if let Some(ref btn) = suspend_btn {
        menu.append(btn).unwrap();
    }
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&exit_item).unwrap();

    let initial_icon = if config.enabled {
        enabled_icon.clone()
    } else {
        disabled_icon.clone()
    };

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(initial_icon)
        .with_tooltip("Sleeper Service")
        .build()
        .expect("failed to create tray icon");

    log::info!("Tray icon created; spawning worker thread");

    // Spawn worker thread
    let (worker_tx, worker_rx) = mpsc::channel::<WorkerMsg>();
    let (label_tx, label_rx) = mpsc::channel::<LabelUpdate>();
    let config_path_clone = config_path.clone();
    let config_clone = config.clone();
    std::thread::spawn(move || {
        service::run_worker(worker_rx, label_tx, config_clone, config_path_clone);
    });

    // Message pump
    let mut should_exit = false;
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Handle tray icon left-click: toggle Enabled
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let new_enabled = !enabled_item.is_checked();
                enabled_item.set_checked(new_enabled);
                let new_system = use_system_item.is_checked();
                let new_resume = resume_item.is_checked();
                log::info!("Tray icon left-click: toggling enabled={new_enabled}");
                let _ = worker_tx.send(WorkerMsg::StateChanged {
                    enabled: new_enabled,
                    use_system_timer: new_system,
                    resume_playback: new_resume,
                });
                if new_enabled {
                    let _ = tray.set_icon(Some(enabled_icon.clone()));
                } else {
                    let _ = tray.set_icon(Some(disabled_icon.clone()));
                }
            }
        }

        // Handle menu events
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *exit_item.id() {
                log::info!("Exit selected from tray menu");
                let _ = worker_tx.send(WorkerMsg::Exit);
                should_exit = true;
            } else if event.id == *enabled_item.id()
                || event.id == *use_system_item.id()
                || event.id == *resume_item.id()
            {
                let new_enabled = enabled_item.is_checked();
                let new_system = use_system_item.is_checked();
                let new_resume = resume_item.is_checked();
                log::info!(
                    "Tray state changed: enabled={new_enabled} use_system_timer={new_system} resume_playback={new_resume}"
                );
                let _ = worker_tx.send(WorkerMsg::StateChanged {
                    enabled: new_enabled,
                    use_system_timer: new_system,
                    resume_playback: new_resume,
                });
                if new_enabled {
                    let _ = tray.set_icon(Some(enabled_icon.clone()));
                } else {
                    let _ = tray.set_icon(Some(disabled_icon.clone()));
                }
            } else if let Some(ref btn) = suspend_btn {
                if event.id == *btn.id() {
                    log::info!("Quick-suspend requested from tray menu");
                    let _ = worker_tx.send(WorkerMsg::QuickSuspend);
                }
            }
        }

        // Apply label updates from the worker thread
        while let Ok(update) = label_rx.try_recv() {
            suspend_state_item.set_text(&update.suspend_state);
            suspend_after_item.set_text(&update.suspend_after);
        }

        if should_exit {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    log::info!("Exiting sleeper_service");
    // Drop the tray explicitly before exit
    drop(tray);
}
