#![windows_subsystem = "windows"]

mod config;
mod icons;
mod media;
mod power;
mod service;

use clap::Parser;
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use service::{LabelUpdate, WorkerMsg};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, HWND};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};
use windows::core::w;

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

fn main() {
    env_logger::init();
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

    // Build tray icons
    let (enabled_rgba, disabled_rgba) = icons::create_icons();
    let enabled_icon =
        Icon::from_rgba(enabled_rgba, 64, 64).expect("failed to create enabled icon");
    let disabled_icon =
        Icon::from_rgba(disabled_rgba, 64, 64).expect("failed to create disabled icon");

    // Build tray menu
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
        .with_icon(initial_icon)
        .with_tooltip("Sleeper Service")
        .build()
        .expect("failed to create tray icon");

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
            while PeekMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Drain tray icon events (hover, click, etc.)
        while TrayIconEvent::receiver().try_recv().is_ok() {}

        // Handle menu events
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *exit_item.id() {
                let _ = worker_tx.send(WorkerMsg::Exit);
                should_exit = true;
            } else if event.id == *enabled_item.id()
                || event.id == *use_system_item.id()
                || event.id == *resume_item.id()
            {
                let _ = worker_tx.send(WorkerMsg::StateChanged {
                    enabled: enabled_item.is_checked(),
                    use_system_timer: use_system_item.is_checked(),
                    resume_playback: resume_item.is_checked(),
                });
                if enabled_item.is_checked() {
                    let _ = tray.set_icon(Some(enabled_icon.clone()));
                } else {
                    let _ = tray.set_icon(Some(disabled_icon.clone()));
                }
            } else if let Some(ref btn) = suspend_btn {
                if event.id == *btn.id() {
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

    // Drop the tray explicitly before exit
    drop(tray);
}
