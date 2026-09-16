use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{event::ModifyKind, Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use synctr_engine::{
    load_filters, path_should_wake, resolve_rclone_live, run_sync, Debouncer, Error, ProfileStore,
    Result,
};

pub fn run(
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
    name: &str,
    debounce_ms: u64,
) -> Result<()> {
    let profile = store.get(name)?;
    profile.require_enabled()?;
    if !profile.local.is_dir() {
        return Err(Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("local directory missing: {}", profile.local.display()),
        )));
    }
    // Canonicalize so notify events (often real paths) strip against the same root.
    // macOS /tmp → /private/tmp is the usual mismatch.
    let local = profile
        .local
        .canonicalize()
        .unwrap_or_else(|_| profile.local.clone());
    resolve_rclone_live(rclone_flag, profile.rclone.as_deref())?;
    let filters = load_filters(store.paths(), &profile)?;
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
    watcher
        .watch(&local, RecursiveMode::Recursive)
        .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
    let wait = Duration::from_millis(debounce_ms.max(1));
    let mut debounce = Debouncer::new(wait);
    eprintln!(
        "watching {} -> {} (debounce {}ms). Ctrl-C stops. TUI Enter-to-run is unchanged.",
        local.display(),
        profile.remote,
        wait.as_millis()
    );
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) => {
                if !event_kind_is_content_change(event.kind) {
                    continue;
                }
                for path in event.paths {
                    if path_should_wake(&filters, &local, &path) {
                        debounce.poke(Instant::now());
                        break;
                    }
                }
            }
            Ok(Err(e)) => {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    e.to_string(),
                )));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if debounce.take_ready(Instant::now()) {
            eprintln!("change detected, syncing {name}");
            match run_sync(store.paths(), &profile, rclone_flag, true, false) {
                Ok(outcome) => {
                    if outcome.exit_code != 0 {
                        eprintln!("{name} exit {}", outcome.exit_code);
                    }
                }
                Err(Error::ProfileBusy(_)) => {
                    eprintln!("skipped {name}: already running");
                }
                Err(e) => eprintln!("{name}: {e}"),
            }
        }
    }
    Ok(())
}

fn event_kind_is_content_change(kind: EventKind) -> bool {
    // Access (including close-write on some backends) and metadata-only
    // events (atime/chmod) are the usual watch false-positives. Create,
    // data modify, rename, and remove still wake.
    !matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, DataChange, MetadataKind};

    #[test]
    fn access_and_metadata_do_not_count_as_content_changes() {
        assert!(!event_kind_is_content_change(EventKind::Access(
            AccessKind::Any
        )));
        assert!(!event_kind_is_content_change(EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::WriteTime)
        )));
        assert!(event_kind_is_content_change(EventKind::Modify(
            ModifyKind::Data(DataChange::Content)
        )));
        assert!(event_kind_is_content_change(EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(event_kind_is_content_change(EventKind::Remove(
            notify::event::RemoveKind::File
        )));
    }
}
