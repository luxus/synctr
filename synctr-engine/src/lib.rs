mod doctor;
mod error;
mod ignore;
mod lock;
mod paths;
mod profile;
mod progress;
mod rclone;
mod runner;
mod schedule;
mod status;
mod watch;

pub use error::{Error, Result};
pub use doctor::{doctor_report, test_remote, DoctorReport, RemoteProbe};
pub use ignore::{load_filters, FilterRule, FilterSet, DEFAULT_PATTERNS};
pub use lock::{profile_is_busy, try_lock_profile, ProfileLock};
pub use paths::{make_absolute, Paths};
pub use profile::{validate_name, Mode, Profile, ProfileEdit, ProfileStore};
pub use progress::{
    clear_progress, display_rclone_line, live_progress, read_progress, write_progress,
    ProgressSink, TransferProgress,
};
pub use rclone::{
    resolve_rclone, resolve_rclone_live, well_known, FsProbe, RealFs, ResolveRequest,
    ResolveSource, ResolvedRclone,
};
pub use runner::{
    build_sync_argv, execute_sync, prepare_filter_file, run_sync, spawn_sync, SyncArgv, SyncChild,
    SyncOutcome,
};
pub use schedule::{
    default_install_dir, enable_hint, generate_schedule, install_schedule, schedule_filenames,
    uninstall_schedule, ScheduleFile, ScheduleKind, ScheduleSpec,
};
pub use status::{
    age_label, assert_status_json_contract, last_run_short, read_last_run, status_json,
    status_snapshot, write_last_run, write_status_json, LastRun, ProfileStatus, RcloneJson,
    StatusSnapshot,
};
pub use watch::{path_should_wake, Debouncer};

#[cfg(test)]
pub(crate) mod testutil {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::paths::Paths;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    pub fn scratch(name: &str) -> (PathBuf, Paths) {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("synctr-{}-{}-{}", name, std::process::id(), n));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let paths = Paths::from_config_dir(root.join("config"));
        fs::create_dir_all(&paths.profiles_dir).unwrap();
        fs::create_dir_all(&paths.state_dir).unwrap();
        fs::create_dir_all(&paths.cache_dir).unwrap();
        (root, paths)
    }

    pub fn write_exec(root: &Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }
}
