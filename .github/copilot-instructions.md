# Copilot Instructions

## Build Commands

```powershell
# Debug build
cargo build

# Release build (recommended for testing real behaviour)
cargo build --release
```

Binaries land at `target\debug\sleeper_service.exe` and `target\release\sleeper_service.exe`.

There are no automated tests. To validate behaviour, run the binary directly and observe the system tray and log output.

**Toolchain requirement:** MSVC toolchain only (not GNU). The `windows` crate requires the MSVC linker from Visual Studio Build Tools ("Desktop development with C++" workload).

## Architecture

The app is a Windows-only GUI binary (`#![windows_subsystem = "windows"]`) with no console window.

**Thread model — two threads:**
- **Main thread** (`main.rs`): owns the Win32 message pump, the `tray-icon`/`muda` tray icon and menu, and the two `mpsc` channels. Sends `WorkerMsg` to the worker; receives `LabelUpdate` to refresh menu text.
- **Worker thread** (`service.rs` → `run_worker`): sleeps for `check_interval` seconds between cycles using `recv_timeout`. On each wake it checks idle time via `GetLastInputInfo`, optionally runs `powercfg /requests` to detect blocking power requests, and calls `suspend_system` when the threshold is crossed.

**Module responsibilities:**
| Module | Responsibility |
|---|---|
| `main.rs` | Entry point, single-instance mutex, logging init, tray/menu setup, message pump |
| `service.rs` | Worker loop, suspend decision logic, post-resume app restarts |
| `config.rs` | `Config` struct (serde TOML), `SuspendState` / `LogLevel` enums, load/save with validation |
| `power.rs` | All Win32/`powercfg` calls: idle time, power status, suspend thresholds, `has_blocking_power_requests`, `suspend_system` |
| `media.rs` | WinRT SMTC (`GlobalSystemMediaTransportControlsSessionManager`) capture and resume |
| `icons.rs` | Procedurally draws 64×64 RGBA tray icons at runtime using the `image` crate |

## Key Conventions

**Windows API usage:** All `unsafe` Win32 calls are in `power.rs`. Use RAII guards (see `HandleGuard`) to close `HANDLE`s on all paths rather than explicit `CloseHandle` calls. Check `GetLastError` after `AdjustTokenPrivileges` even when it returns `TRUE`.

**Child processes (no console window):** Every `std::process::Command` that spawns an external process must set `.creation_flags(0x08000000)` (`CREATE_NO_WINDOW`) to suppress console flicker.

**Config serialisation:** `SuspendState` and `LogLevel` use `#[serde(rename_all = "lowercase")]` so TOML values are lowercase strings (`"sleep"`, `"debug"`, etc.). New enum variants must follow this pattern.

**Config validation in `Config::load`:** After deserialising, clamp/replace invalid float values (`NaN`/`inf`) and enforce minimums (`manual_suspend_after` ≥ 60 s, `check_interval` ≥ 1 s) before returning. Do not panic on bad config values.

**WinRT COM apartments:** `CoInitializeEx(None, COINIT_MULTITHREADED)` must be called on every thread that uses WinRT/COM (`media.rs` does this in both `capture_playback` and `restart_playback`).

**Tray label formatting:** Status labels are prefixed with six non-breaking spaces (`"      Suspend: …"`) to indent them visually under the checkboxes in the menu.

**Logging:** Use `log::trace!` for per-tick noise, `log::debug!` for per-cycle decisions, `log::info!` for state changes and lifecycle events, `log::warn!` for recoverable failures. Runtime logging is via `simplelog::WriteLogger` (file) or `env_logger` (terminal via `RUST_LOG`).
