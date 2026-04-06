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
            idle_ms as f64 / 1000.0
        } else {
            0.0
        }
    }
}

pub fn get_power_status() -> PowerStatus {
    unsafe {
        let mut status = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut status).is_ok() {
            match status.ACLineStatus {
                0 => PowerStatus::Battery,
                1 => PowerStatus::AcMode,
                _ => PowerStatus::Unknown,
            }
        } else {
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

    match power_state {
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
    }
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
    let mut in_unavailable = false;
    for line in text.lines() {
        let clean = line.trim();
        if clean.starts_with("The following sleep states are not available") {
            in_unavailable = true;
        }
        if !in_unavailable && clean == "Hibernate" {
            return true;
        }
    }
    false
}

pub fn suspend_system(state: &SuspendState) -> windows::core::Result<()> {
    if *state == SuspendState::Disabled {
        return Ok(());
    }
    let hibernate = *state == SuspendState::Hibernate;

    unsafe {
        let process = GetCurrentProcess();
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token)?;

        let mut luid = LUID::default();
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
