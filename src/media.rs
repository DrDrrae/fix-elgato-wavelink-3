use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

/// `RPC_E_CHANGED_MODE` (0x80010106) – returned by `CoInitializeEx` when the
/// thread is already initialised in a different COM apartment model.
const RPC_E_CHANGED_MODE: windows::core::HRESULT = windows::core::HRESULT(0x80010106_u32 as i32);

/// RAII guard that initialises COM on the current thread and uninitialises it
/// on drop.  `CoUninitialize` is only called when `CoInitializeEx` succeeded
/// (i.e. returned `S_OK` or `S_FALSE`).  If the thread is already in a
/// different apartment (`RPC_E_CHANGED_MODE`) the existing state is left
/// untouched and a warning is logged.
struct ComInit {
    should_uninit: bool,
}

impl ComInit {
    fn new() -> Self {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let should_uninit = match result {
            Ok(()) => true,
            Err(e) => {
                if e.code() == RPC_E_CHANGED_MODE {
                    log::warn!("CoInitializeEx: thread already initialised with a different apartment model");
                } else {
                    log::warn!("CoInitializeEx failed: {e}");
                }
                false
            }
        };
        Self { should_uninit }
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

pub struct PlaybackSnapshot {
    pub session: GlobalSystemMediaTransportControlsSession,
    pub was_playing: bool,
}

pub fn capture_playback() -> Vec<PlaybackSnapshot> {
    let _com = ComInit::new();
    let mut snapshots = Vec::new();

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .and_then(|op| op.join())
    {
        Ok(m) => m,
        Err(e) => {
            log::warn!("Failed to get SMTC media manager: {e}");
            return snapshots;
        }
    };

    let sessions = match manager.GetSessions() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to get SMTC sessions: {e}");
            return snapshots;
        }
    };

    let count = match sessions.Size() {
        Ok(c) => c as usize,
        Err(_) => return snapshots,
    };

    log::debug!("capture_playback: found {count} SMTC session(s)");

    for i in 0..count {
        if let Ok(session) = sessions.GetAt(i as u32) {
            let was_playing = session
                .GetPlaybackInfo()
                .ok()
                .and_then(|info| info.PlaybackStatus().ok())
                .map(|s| {
                    s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
                })
                .unwrap_or(false);
            let app_id = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_else(|_| "<unknown>".to_string());
            log::debug!("  session[{i}] app={app_id} was_playing={was_playing}");
            snapshots.push(PlaybackSnapshot { session, was_playing });
        }
    }

    snapshots
}

pub fn restart_playback(snapshots: Vec<PlaybackSnapshot>, delay_secs: u32) {
    let _com = ComInit::new();
    log::debug!("restart_playback: waiting {delay_secs}s before sending media commands");
    std::thread::sleep(std::time::Duration::from_secs(delay_secs as u64));

    for (idx, snapshot) in snapshots.iter().enumerate() {
        if !snapshot.was_playing {
            log::debug!("  session[{idx}]: was not playing — skipping");
            continue;
        }
        log::debug!("  session[{idx}]: sending Pause");
        if let Ok(op) = snapshot.session.TryPauseAsync() {
            let _ = op.join();
            let mut counter = 0u32;
            loop {
                let status = snapshot
                    .session
                    .GetPlaybackInfo()
                    .ok()
                    .and_then(|i| i.PlaybackStatus().ok());
                if status
                    == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused)
                    || counter >= 50
                {
                    log::debug!("  session[{idx}]: paused after {counter} poll(s)");
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                counter += 1;
            }
        }
        log::debug!("  session[{idx}]: sending Play");
        if let Ok(op) = snapshot.session.TryPlayAsync() {
            let _ = op.join();
        }
    }
    log::debug!("restart_playback: done");
}
