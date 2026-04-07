use crate::config::SuspendState;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SetSuspendState, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::core::PCWSTR;
use windows::core::w;

#[derive(Debug, Clone, PartialEq)]
pub enum PowerStatus {
    Battery,
    AcMode,
    Unknown,
}

pub fn idle_time_secs() -> f64 {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut info).as_bool() {
            let tick = GetTickCount();
            let idle_ms = tick.wrapping_sub(info.dwTime);
            let secs = idle_ms as f64 / 1000.0;
            log::trace!("idle_time_secs: {secs:.2}s (tick={tick} last_input={})", info.dwTime);
            secs
        } else {
            log::warn!("GetLastInputInfo failed; assuming idle=0");
            0.0
        }
    }
}

pub fn get_power_status() -> PowerStatus {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            let ps = match status.ACLineStatus {
                0 => PowerStatus::Battery,
                1 => PowerStatus::AcMode,
                _ => PowerStatus::Unknown,
            };
            log::debug!("Power status: {ps:?} (ACLineStatus={})", status.ACLineStatus);
            ps
        } else {
            log::warn!("GetSystemPowerStatus failed; assuming Unknown");
            PowerStatus::Unknown
        }
    }
}

/// Run `powercfg /query SCHEME_CURRENT SUB_SLEEP <alias>` and return the
/// AC or DC idle threshold in seconds (0 = disabled / not found).
pub fn get_idle_threshold(power_state: &PowerStatus, idle_type: &SuspendState) -> u32 {
    let alias = match idle_type {
        SuspendState::Sleep => "STANDBYIDLE",
        SuspendState::Hibernate => "HIBERNATEIDLE",
        SuspendState::Disabled => return 0,
    };

    let output = match Command::new("powercfg")
        .args(["/query", "SCHEME_CURRENT", "SUB_SLEEP", alias])
        .creation_flags(0x08000000)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("powercfg query failed: {e}");
            return 0;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    log::trace!("powercfg /query output for {alias}:\n{text}");
    let mut ac_value: Option<u32> = None;
    let mut dc_value: Option<u32> = None;

    for line in text.lines() {
        if line.contains("Power Setting Index: ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // e.g. ["Current", "AC", "Power", "Setting", "Index:", "0x00000384"]
            if let Some(hex_str) = parts.last() {
                let hex = hex_str.trim_start_matches("0x");
                let value = u32::from_str_radix(hex, 16).unwrap_or(0);
                if parts.get(1) == Some(&"AC") {
                    ac_value = Some(value);
                } else if parts.get(1) == Some(&"DC") {
                    dc_value = Some(value);
                }
            }
        }
    }

    let ac = ac_value.unwrap_or(0);
    let dc = dc_value.unwrap_or(0);
    log::debug!("get_idle_threshold({alias}): AC={ac}s DC={dc}s power_state={power_state:?}");

    let result = match power_state {
        PowerStatus::AcMode => ac,
        PowerStatus::Battery => dc,
        PowerStatus::Unknown => {
            let min_val = ac.min(dc);
            if min_val == 0 {
                ac.max(dc)
            } else {
                min_val
            }
        }
    };
    log::debug!("get_idle_threshold({alias}) result: {result}s");
    result
}

pub fn hibernate_enabled() -> bool {
    let output = match Command::new("powercfg")
        .arg("/a")
        .creation_flags(0x08000000)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("powercfg /a failed: {e}");
            return false;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    log::trace!("powercfg /a output:\n{text}");
    let mut in_unavailable = false;
    for line in text.lines() {
        let clean = line.trim();
        if clean.starts_with("The following sleep states are not available") {
            in_unavailable = true;
        }
        if !in_unavailable && clean == "Hibernate" {
            log::debug!("hibernate_enabled: true");
            return true;
        }
    }
    log::debug!("hibernate_enabled: false");
    false
}

/// Run `powercfg /requests` and return `true` if any active power request is found
/// that is NOT covered by the `ignored` substring list.
///
/// `powercfg /requests` output contains section headers (`DISPLAY:`, `SYSTEM:`, …)
/// followed by either `None.` or one or more request entries. Each entry is two lines:
///   `[PROCESS] \Device\...\app.exe`  (or `[DRIVER]` / `[SERVICE]`)
///   `Description of the request`
///
/// A request is considered ignored when either of its two lines contains any string in
/// `ignored`.  The default ignore list contains `"Legacy Kernel Caller"` so that the
/// Elgato Wave Link / Voicemeeter audio-stream block does not prevent sleep.
pub fn has_blocking_power_requests(ignored: &[String]) -> bool {
    let output = match Command::new("powercfg")
        .arg("/requests")
        .creation_flags(0x08000000)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("powercfg /requests failed: {e}");
            return false;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    log::trace!("powercfg /requests output:\n{text}");
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Request entries start with `[` (e.g. `[PROCESS]`, `[DRIVER]`, `[SERVICE]`).
        if line.starts_with('[') {
            let description = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");

            let is_ignored = ignored
                .iter()
                .any(|pat| line.contains(pat.as_str()) || description.contains(pat.as_str()));

            if is_ignored {
                log::debug!("Ignoring power request: {line} / {description}");
            } else {
                log::debug!("Blocking power request detected: {line} / {description}");
                return true;
            }

            i += 2; // skip the description line
        } else {
            i += 1;
        }
    }

    false
}

pub fn suspend_system(state: &SuspendState) -> windows::core::Result<()> {
    if *state == SuspendState::Disabled {
        return Ok(());
    }
    let hibernate = *state == SuspendState::Hibernate;
    log::info!("suspend_system: requesting {} (hibernate={hibernate})", state);

    unsafe {
        let process = GetCurrentProcess();
        let mut token = HANDLE::default();
        log::debug!("suspend_system: opening process token");
        OpenProcessToken(process, TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token)?;

        let mut luid = LUID::default();
        log::debug!("suspend_system: looking up SeShutdownPrivilege");
        LookupPrivilegeValueW(PCWSTR::null(), w!("SeShutdownPrivilege"), &mut luid)?;

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let tp_size = std::mem::size_of::<TOKEN_PRIVILEGES>() as u32;
        let mut prev_tp: TOKEN_PRIVILEGES = std::mem::zeroed();
        let mut ret_len = 0u32;

        let _ = AdjustTokenPrivileges(
            token,
            false,
            Some(&tp as *const TOKEN_PRIVILEGES),
            tp_size,
            Some(&mut prev_tp as *mut TOKEN_PRIVILEGES),
            Some(&mut ret_len),
        );

        SetSuspendState(hibernate, true, false);
        log::debug!("suspend_system: SetSuspendState called — system will suspend momentarily");

        let _ = AdjustTokenPrivileges(
            token,
            false,
            Some(&prev_tp as *const TOKEN_PRIVILEGES),
            tp_size,
            None,
            None,
        );

        let _ = CloseHandle(token);
    }

    Ok(())
}
