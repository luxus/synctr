use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use synctr_engine::{
    load_filters, path_should_wake, run_sync, Debouncer, Error, ProfileStore, Result,
};

pub fn run(
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
    name: &str,
    debounce_ms: u64,
) -> Result<()> {
    let profile = store.get(name)?;
    if !profile.local.is_dir() {
        return Err(Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("local directory missing: {}", profile.local.display()),
        )));
    }
    let filters = load_filters(store.paths(), &profile)?;
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
    watcher
        .watch(&profile.local, RecursiveMode::Recursive)
        .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
    let wait = Duration::from_millis(debounce_ms.max(1));
    let mut debounce = Debouncer::new(wait);
    eprintln!(
        "watching {} -> {} (debounce {}ms). Ctrl-C stops. TUI Enter-to-run is unchanged.",
        profile.local.display(),
        profile.remote,
        wait.as_millis()
    );
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(event)) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    continue;
                }
                for path in event.paths {
                    if path_should_wake(&filters, &profile.local, &path) {
                        debounce.poke(Instant::now());
                        break;
                    }
                }
            }
            Ok(Err(e)) => {
                return Err(Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if debounce.take_ready(Instant::now()) {
            eprintln!("change detected, syncing {name}");
            let outcome = run_sync(store.paths(), &profile, rclone_flag, true, false)?;
            if outcome.exit_code != 0 {
                eprintln!("{name} exit {}", outcome.exit_code);
            }
        }
    }
    Ok(())
}
