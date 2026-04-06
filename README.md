# sleeper_service

A minimal Windows system tray utility written in Rust that forces sleep or hibernate
based on the active power plan's "Sleep After" setting — overriding the Windows bug
that allows audio streams (e.g. Elgato Wave Link, Voicemeeter) to block sleep
indefinitely via the "Legacy Kernel Caller" power request.

> **Background:** `powercfg /requests` may show an `[SYSTEM] An audio stream is
> currently in use. [DRIVER] Legacy Kernel Caller` entry. Windows honours this request
> and silently ignores your configured sleep timer. This utility works around that by
> monitoring user idle time directly and forcing the suspend itself.

This is a Rust port of the original Python
**[sleeper_service](https://github.com/BuongiornoTexas/sleeper_service)** by
**[BuongiornoTexas](https://github.com/BuongiornoTexas)**, rewritten for performance
and to remove the Python runtime dependency. All credit for the original concept,
design, and implementation goes to BuongiornoTexas.

---

## Change Log

- **v0.2.0** Added power-request awareness (`respect_power_requests`,
  `ignored_power_requests`): legitimate sleep blockers such as video playback are now
  respected while nuisance blockers like Elgato Wave Link's `Legacy Kernel Caller`
  audio-stream entry continue to be ignored.
- **v0.1.0** Initial Rust port of
  [sleeper_service v0.20.0](https://github.com/BuongiornoTexas/sleeper_service).
  Feature-equivalent with the Python release: system/manual timers, tray interface,
  SMTC media resume, app restart on resume, single-instance mutex.

---

## Features

- **System tray interface** — enable/disable the service, switch timer modes, toggle
  playback resume, and exit from the right-click menu.
- **System timer mode** — reads your active Windows power plan's "Sleep after" /
  "Hibernate after" values via `powercfg` and uses whichever fires first.
- **Manual timer mode** — specify your own suspend threshold and state
  (`sleep`, `hibernate`, or `disabled`) in the config file.
- **Resume media playback** — optionally uses the Windows SMTC (System Media Transport
  Controls) WinRT API to detect which apps were playing before suspend and resume them
  afterwards.
- **App restart on resume** — optionally re-launches listed programs/apps after every
  suspend triggered by this service (useful for apps like Elgato Wave Link that crash or
  break on resume).
- **Single instance guard** — uses a named Windows mutex to prevent running multiple
  copies simultaneously.
- **"Suspend now(-ish)!" button** — an optional tray menu button for testing, enabled
  only via the config file.

---

## Building

**Prerequisites:**

1. [Rust](https://rustup.rs) — install via `rustup`, selecting the **MSVC** toolchain
   (required for Windows APIs).
2. [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   — install the **"Desktop development with C++"** workload to provide the MSVC linker.

**Build:**

```powershell
# Clone the repository
git clone https://github.com/BuongiornoTexas/sleeper_service
cd sleeper_service

# Debug build (faster compile, larger binary)
cargo build

# Release build (optimised, recommended for daily use)
cargo build --release
```

The output binary will be at:
- Debug: `target\debug\sleeper_service.exe`
- Release: `target\release\sleeper_service.exe`

The binary is a Windows GUI application (no console window).

---

## Installation and Usage

1. Copy `sleeper_service.exe` to a permanent location (e.g. `C:\Tools\sleeper_service\`).
2. Optionally add it to your Windows startup programs
   (Win+R → `shell:startup`, paste a shortcut there).
3. Run it — a tray icon will appear (a "ZZZ" sleep symbol).

**Tray icon:**
- **ZZZ icon** — service is enabled and monitoring idle time.
- **ZZZ with red X** — service is disabled (will not force suspend).

**Right-click menu:**

| Item | Description |
|------|-------------|
| **Enabled** ✓ | Toggle the service on/off. Left-click the icon also toggles this. |
| **Use System Timers** ✓ | When checked, reads sleep/hibernate thresholds from your active Windows power plan. When unchecked, uses `manual_suspend_after` and `manual_suspend_state` from the config file. |
| `Suspend: sleep` | Read-only label showing the active suspend mode. |
| `After: 600s` | Read-only label showing the active idle threshold in seconds. |
| **Resume playback** ✓ | When checked, attempts to resume SMTC-compatible media players after waking. |
| **Suspend now(-ish)!** | *(Optional)* Forces a suspend at the end of the current check interval. Only visible when `suspend_button = true` in the config. |
| **Exit** | Exits the service and saves current settings. |

---

## Configuration File

The config file is `config.toml`. Its location is resolved in this priority order:

1. `--config <file>` CLI argument — uses the specified file directly.
2. `--config <folder>` CLI argument — looks for `config.toml` inside that folder.
3. Same directory as the `sleeper_service.exe` binary *(default)*.

If the file does not exist it is created automatically with defaults on first run.
**Exit the service before editing the file manually.**

### Full default config

```toml
enabled = true
use_system_timer = true
manual_suspend_after = 600.0
manual_suspend_state = "sleep"
check_interval = 60.0
resume_playback = false
resume_playback_delay = 1
suspend_button = false
respect_power_requests = true
ignored_power_requests = ["Legacy Kernel Caller"]

[restarts]
```

### Options reference

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable or disable the service. Togglable from the tray. |
| `use_system_timer` | bool | `true` | Use Windows power plan timers. Togglable from the tray. |
| `manual_suspend_after` | float (seconds) | `600.0` | Idle time before suspend when `use_system_timer = false`. Minimum enforced value is 60 s. |
| `manual_suspend_state` | string | `"sleep"` | Suspend action when not using system timers. Must be `"sleep"`, `"hibernate"`, or `"disabled"`. |
| `check_interval` | float (seconds) | `60.0` | How often the service checks idle time. Minimum enforced value is 1 s. |
| `resume_playback` | bool | `false` | Attempt to resume SMTC media players after wake. Togglable from the tray. |
| `resume_playback_delay` | integer (seconds) | `1` | Time to wait after waking before issuing pause/play commands. Increase this (e.g. to `15`) for players with large buffers (Firefox/YouTube). |
| `suspend_button` | bool | `false` | Show a "Suspend now(-ish)!" button in the tray menu. Only takes effect on restart. |
| `respect_power_requests` | bool | `true` | When `true`, runs `powercfg /requests` before each potential suspend. If any active power request is found that is **not** in `ignored_power_requests`, the suspend is skipped for that cycle. This allows legitimate blockers (e.g. a playing video, a presentation) to prevent sleep while still ignoring nuisance blockers like Elgato Wave Link. |
| `ignored_power_requests` | list of strings | `["Legacy Kernel Caller"]` | Substrings matched against each entry in `powercfg /requests` output. Any request whose identifier line or description contains one of these strings is ignored. The default ignores the audio-stream entry created by apps like Elgato Wave Link and Voicemeeter. |
| `[restarts]` | table | *(empty)* | Programs to relaunch after every service-triggered resume. See below. |

---

## Respecting Legitimate Sleep Blockers (`respect_power_requests`)

Windows exposes active sleep-blocking requests via `powercfg /requests`. This output
has two distinct kinds of entries:

- **Nuisance blockers** — persistent audio streams from apps like Elgato Wave Link that
  register a `[DRIVER] Legacy Kernel Caller` entry even when nothing useful is happening.
  These are what this utility was originally designed to work around.

- **Legitimate blockers** — requests raised because the user is genuinely active, e.g.:
  - A video is playing in a media player (`[PROCESS]` entry under `DISPLAY:`)
  - A game is running full-screen
  - A presentation is being shown
  - A background download/task has requested the system stay awake (`EXECUTION:`)

With `respect_power_requests = true` (the default), the service runs `powercfg /requests`
before each potential suspend. If any entry is found that is **not** covered by
`ignored_power_requests`, the suspend is skipped for that check cycle. The idle timer
continues running normally; suspend will happen once the blocking activity ends.

To see what is currently blocking sleep on your system, run in an elevated terminal:

```powershell
powercfg /requests
```

Example output with both types of blocker active:

```
DISPLAY:
[PROCESS] \Device\HarddiskVolume3\Windows\SystemApps\Microsoft.ZuneVideo_...\Video.UI.exe
Video playback

SYSTEM:
[DRIVER] Legacy Kernel Caller
An audio stream is currently in use.
```

With the default config the `Legacy Kernel Caller` line is ignored (allowing the
sleep override to proceed), while the `Video.UI.exe` display request is respected
(preventing sleep while the video plays).

### Customising the ignore list

Add any substring that appears in the `[TYPE] identifier` line or the description line
of a request you want to ignore:

```toml
ignored_power_requests = [
    "Legacy Kernel Caller",   # Elgato Wave Link / Voicemeeter audio streams
    "SteelSeriesEngine.exe",  # SteelSeries Sonar (if it causes spurious requests)
]
```

To disable the feature entirely and always force sleep regardless of any power request:

```toml
respect_power_requests = false
```

---

## App Restart on Resume (`[restarts]`)

The `[restarts]` table lets you automatically relaunch one or more programs every time
the service wakes from a suspend it triggered. This is useful for apps that crash or
break audio routing after sleep (e.g. Elgato Wave Link 3).

**Format:**

```toml
[restarts]
"<executable-or-alias>" = "Popen"
```

Each entry is a key-value pair:
- **Key** — the program name or path passed to the OS to launch the process.
- **Value** — the launch method. Currently only `"Popen"` is supported, which spawns
  the process detached from the service (equivalent to Python's `subprocess.Popen`).

**Example — restart Elgato Wave Link on resume:**

```toml
[restarts]
"Elgato.Wavelink.exe" = "Popen"
```

Wave Link registers a Windows app execution alias (`Elgato.Wavelink.exe`), so no full
UWP path is needed.

**Multiple apps:**

```toml
[restarts]
"Elgato.Wavelink.exe" = "Popen"
"VoicemeeterPro.exe" = "Popen"
```

**Limitations:**
- Restarts are only triggered on suspends initiated by *this service*. Manual sleep
  from the Start menu or Windows power button will not trigger restarts.
- The launched program inherits the working directory of `sleeper_service.exe`. Apps
  that rely on a specific working directory may not start correctly.
- Programs requiring administrator privileges cannot be launched this way.
- The app will not start minimised unless it supports that natively.

---

## Resume Playback

When `resume_playback = true` the service uses the Windows **System Media Transport
Controls (SMTC)** WinRT API to:

1. Record which apps were actively playing immediately before suspend.
2. After waking, wait `resume_playback_delay` seconds.
3. For each app that was playing: send a **Pause** command, wait up to 5 s for it to
   take effect, then send a **Play** command.

The pause-before-play step works around a Windows/player quirk where some apps report
`Playing` status after resume even though the stream is stalled.

**Requirements:**
- Your media app must support the SMTC API. See the
  [Modern Flyouts SMTC compatibility list](https://github.com/ModernFlyouts-Community/ModernFlyouts/blob/main/docs/GSMTC-Support-And-Popular-Apps.md)
  for hints. Many modern players (Spotify, Deezer, browser media) work; older or
  niche players may not.

**Tuning for buffered players (e.g. YouTube in Firefox):**

Firefox buffers 10+ seconds of video. With the default `resume_playback_delay = 1`,
the pause/play commands arrive while the buffer is still draining and are ignored.
Increase the delay past the buffer length:

```toml
resume_playback_delay = 20
```

---

## Command-Line Options

```
sleeper_service.exe [--config <CONFIG_PATH>]
```

| Flag | Description |
|------|-------------|
| `--config <path>` | Path to a `config.toml` file, or a folder containing one. |

---

## Differences from the Python Version

| Area | Python (`v0.20`) | Rust (this port) |
|------|-----------------|-----------------|
| Runtime | Requires Python ≥ 3.11 + pip packages | Single self-contained `.exe`, no runtime needed |
| Single-instance | `psutil` process scan | Named Windows mutex (`Global\sleeper_service_mutex`) |
| Tray library | `tray-manager` (pystray wrapper) | `tray-icon` + `muda` |
| Icon generation | PIL `ImageDraw` + Segoe UI font | Programmatic line drawing (no font dependency) |
| Config format | Identical `config.toml` — **fully compatible** | Same TOML schema; values are preserved on upgrade |
| `[restarts]` values | `"Popen"` (string enum) | `"Popen"` (same string) — compatible |
| `manual_suspend_after` | `int \| float` | `float` — values like `600` and `600.0` both work |
| `check_interval` | `int \| float` | `float` — same |
| SMTC media API | `winrt` Python package | `windows` crate WinRT bindings |
| Logging | `print` / implicit | `env_logger`; set `RUST_LOG=warn` or `RUST_LOG=debug` |
| `suspend_button` | Immediately suspends (v0.11) / end-of-interval (v0.20) | End of current check interval |
| Power request awareness | Not present | **New:** `respect_power_requests` + `ignored_power_requests` — skips suspend when a legitimate blocker (e.g. video playback) is active, while still ignoring nuisance blockers like `Legacy Kernel Caller` |

> **Config compatibility:** The TOML schema is intentionally kept compatible with the
> Python version. If you already have a `config.toml` from the Python release, place it
> next to the Rust binary and it will be read without modification. The two new keys
> (`respect_power_requests`, `ignored_power_requests`) will be added with their defaults
> the next time settings are saved.

---

## Debugging / Logging

The binary is built as a Windows GUI app (no console). To see log output, launch from
a terminal with the `RUST_LOG` environment variable set:

```powershell
$env:RUST_LOG = "warn"   # warnings and errors only (default-ish)
$env:RUST_LOG = "debug"  # verbose — shows suspend decisions, powercfg output, etc.
.\sleeper_service.exe
```

---

## Package Rationale

Windows allows audio streams to block the system sleep timer via a mechanism called
power requests. Running `powercfg /requests` when an affected app is open typically
shows:

```
[SYSTEM]
An audio stream is currently in use.
[DRIVER] Legacy Kernel Caller
```

When this request is active, Windows ignores your configured "Sleep after" power plan
setting entirely. The official workaround (`powercfg /requestsoverride`) requires
elevated privileges and does not reliably work. This utility sidesteps the issue
entirely by monitoring user idle time directly and calling `SetSuspendState` itself.

Known affected software includes: **Elgato Wave Link**, older versions of
**Voicemeeter**, **SteelSeries Sonar**, and many other apps that open persistent audio
streams.

---

## Credits and Original Work

This project is a Rust port of
**[sleeper_service](https://github.com/BuongiornoTexas/sleeper_service)**
by **[BuongiornoTexas](https://github.com/BuongiornoTexas)**.

The original Python implementation (MIT licensed) introduced the core idea of
monitoring user idle time to force Windows suspend, the `[restarts]` app-relaunch
mechanism, SMTC-based media playback resume, and the tray interface design. Please
visit the original repository for the Python version, release binaries, and the
upstream issue tracker.

---

## License

MIT — see [LICENSE](LICENSE).
