use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

pub struct PlaybackSnapshot {
    pub session: GlobalSystemMediaTransportControlsSession,
    pub was_playing: bool,
}

pub fn capture_playback() -> Vec<PlaybackSnapshot> {
    let mut snapshots = Vec::new();

    let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .and_then(|op| op.get())
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
    log::debug!("restart_playback: waiting {delay_secs}s before sending media commands");
    std::thread::sleep(std::time::Duration::from_secs(delay_secs as u64));

    for (idx, snapshot) in snapshots.iter().enumerate() {
        if !snapshot.was_playing {
            log::debug!("  session[{idx}]: was not playing — skipping");
            continue;
        }
        log::debug!("  session[{idx}]: sending Pause");
        if let Ok(op) = snapshot.session.TryPauseAsync() {
            let _ = op.get();
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
            let _ = op.get();
        }
    }
    log::debug!("restart_playback: done");
}
