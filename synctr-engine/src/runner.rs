use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use crate::error::Result;
use crate::ignore::load_filters;
use crate::lock::{try_lock_profile, ProfileLock};
use crate::paths::Paths;
use crate::profile::Profile;
use crate::progress::{clear_inflight, tee_rclone_stderr, write_inflight, Inflight};
use crate::rclone::{resolve_rclone_live, ResolvedRclone};
use crate::status::{write_last_run, LastRun};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncArgv {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl SyncArgv {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        cmd
    }

    pub fn args_lossy(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub exit_code: i32,
    pub rclone: ResolvedRclone,
    pub stdout: Option<Vec<u8>>,
    pub stderr: Option<Vec<u8>>,
}

pub fn build_sync_argv(
    rclone: &Path,
    profile: &Profile,
    filter_file: &Path,
    dry_run: bool,
) -> SyncArgv {
    let mut args: Vec<OsString> = vec![
        profile.mode.rclone_subcommand().into(),
        profile.local.clone().into(),
        profile.remote.clone().into(),
        "--filter-from".into(),
        filter_file.as_os_str().to_os_string(),
        "--verbose".into(),
        "--use-json-log".into(),
        "--stats".into(),
        "1s".into(),
    ];
    for flag in &profile.extra_flags {
        args.push(flag.into());
    }
    if dry_run {
        args.push("--dry-run".into());
    }
    SyncArgv {
        program: rclone.to_path_buf(),
        args,
    }
}

pub fn prepare_filter_file(paths: &Paths, profile: &Profile) -> Result<PathBuf> {
    let filters = load_filters(paths, profile)?;
    let dest = paths.filter_file(&profile.name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&dest, filters.to_filter_from())?;
    Ok(dest)
}

#[derive(Debug)]
pub struct SyncChild {
    pub child: Child,
    _lock: ProfileLock,
    paths: Paths,
    name: String,
}

impl Drop for SyncChild {
    fn drop(&mut self) {
        let _ = clear_inflight(&self.paths, &self.name);
    }
}

impl std::ops::Deref for SyncChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for SyncChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

pub fn spawn_sync(
    paths: &Paths,
    profile: &Profile,
    resolved: &ResolvedRclone,
    dry_run: bool,
) -> Result<SyncChild> {
    let _lock = try_lock_profile(paths, &profile.name)?;
    write_inflight(paths, &profile.name, &Inflight::start(dry_run))?;
    let filter = prepare_filter_file(paths, profile)?;
    let argv = build_sync_argv(&resolved.path, profile, &filter, dry_run);
    let mut cmd = argv.command();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match cmd.spawn() {
        Ok(child) => Ok(SyncChild {
            child,
            _lock,
            paths: paths.clone(),
            name: profile.name.clone(),
        }),
        Err(e) => {
            let _ = clear_inflight(paths, &profile.name);
            Err(e.into())
        }
    }
}

pub fn run_sync(
    paths: &Paths,
    profile: &Profile,
    rclone_flag: Option<&Path>,
    inherit_stdio: bool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    let resolved = resolve_rclone_live(rclone_flag, profile.rclone.as_deref())?;
    execute_sync(paths, profile, resolved, inherit_stdio, dry_run)
}

pub fn execute_sync(
    paths: &Paths,
    profile: &Profile,
    resolved: ResolvedRclone,
    inherit_stdio: bool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    let _lock = try_lock_profile(paths, &profile.name)?;
    write_inflight(paths, &profile.name, &Inflight::start(dry_run))?;
    let result = execute_sync_locked(paths, profile, resolved, inherit_stdio, dry_run);
    let _ = clear_inflight(paths, &profile.name);
    result
}

fn execute_sync_locked(
    paths: &Paths,
    profile: &Profile,
    resolved: ResolvedRclone,
    inherit_stdio: bool,
    dry_run: bool,
) -> Result<SyncOutcome> {
    let filter = prepare_filter_file(paths, profile)?;
    let argv = build_sync_argv(&resolved.path, profile, &filter, dry_run);
    let mut cmd = argv.command();
    cmd.stdin(Stdio::null());
    if inherit_stdio {
        cmd.stdout(Stdio::inherit());
    } else {
        cmd.stdout(Stdio::piped());
    }
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take();
    let stdout_thread = child.stdout.take().map(|mut pipe| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    });
    let paths_err = paths.clone();
    let name = profile.name.clone();
    let stderr_thread = stderr.map(move |err| {
        thread::spawn(move || tee_rclone_stderr(&paths_err, &name, err, inherit_stdio))
    });
    let status = child.wait()?;
    if let Some(t) = stderr_thread {
        match t.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {}
        }
    }
    let stdout = stdout_thread.and_then(|t| t.join().ok());
    let exit_code = status.code().unwrap_or(1);
    write_last_run(paths, &profile.name, LastRun::now(exit_code))?;
    Ok(SyncOutcome {
        exit_code,
        rclone: resolved,
        stdout,
        stderr: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Mode, Profile, ProfileStore};
    use crate::rclone::ResolveSource;
    use crate::testutil::{scratch, write_exec};

    #[test]
    fn argv_is_separate_args_not_a_shell_string() {
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec!["--checksum".into()],
            vec![],
        )
        .unwrap();
        let argv = build_sync_argv(
            Path::new("/usr/bin/rclone"),
            &profile,
            Path::new("/tmp/docs.filter"),
            false,
        );
        assert_eq!(argv.program, PathBuf::from("/usr/bin/rclone"));
        let args = argv.args_lossy();
        assert_eq!(
            args,
            vec![
                "sync",
                "/tmp/docs",
                "b2:bucket/docs",
                "--filter-from",
                "/tmp/docs.filter",
                "--verbose",
                "--use-json-log",
                "--stats",
                "1s",
                "--checksum",
            ]
        );
        assert!(args.iter().all(|a| !a.contains(" --")));
        assert!(!args.iter().any(|a| a == "--dry-run"));

        let dry = build_sync_argv(
            Path::new("/usr/bin/rclone"),
            &profile,
            Path::new("/tmp/docs.filter"),
            true,
        );
        let dry_args = dry.args_lossy();
        assert_eq!(&dry_args[..args.len()], args.as_slice());
        assert_eq!(dry_args.last().map(String::as_str), Some("--dry-run"));
    }

    #[test]
    fn run_sync_records_fake_rclone_exit() {
        let (root, paths) = scratch("runner-exit");
        let bin = write_exec(&root, "rclone", "#!/bin/sh\nexit 7\n");
        let store = ProfileStore::new(paths.clone());
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:x".into(),
            Mode::Copy,
            Some(bin.clone()),
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&profile).unwrap();
        let resolved = ResolvedRclone {
            path: bin,
            source: ResolveSource::Profile,
        };
        let outcome = execute_sync(&paths, &profile, resolved, false, false).unwrap();
        assert_eq!(outcome.exit_code, 7);
        let last = crate::status::read_last_run(&paths, "docs")
            .unwrap()
            .unwrap();
        assert_eq!(last.exit_code, 7);
        assert!(!last.ok);
        let filters = crate::ignore::load_filters(&paths, &profile).unwrap();
        assert_eq!(
            fs::read_to_string(paths.filter_file("docs")).unwrap(),
            filters.to_filter_from()
        );
        assert!(filters.to_filter_from().contains("- node_modules/**"));
    }

    #[test]
    fn spawn_sync_can_be_killed() {
        let (root, paths) = scratch("runner-spawn");
        let bin = write_exec(
            &root,
            "rclone",
            "#!/bin/sh\necho started\nwhile true; do sleep 1; done\n",
        );
        let store = ProfileStore::new(paths.clone());
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:x".into(),
            Mode::Copy,
            Some(bin.clone()),
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&profile).unwrap();
        let resolved = ResolvedRclone {
            path: bin,
            source: ResolveSource::Profile,
        };
        let mut child = spawn_sync(&paths, &profile, &resolved, false).unwrap();
        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn overlapping_sync_returns_profile_busy() {
        let (root, paths) = scratch("runner-lock");
        let wait_bin = write_exec(
            &root,
            "rclone-wait",
            "#!/bin/sh\necho started\nwhile true; do sleep 1; done\n",
        );
        let ok_bin = write_exec(&root, "rclone-ok", "#!/bin/sh\nexit 0\n");
        let store = ProfileStore::new(paths.clone());
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:x".into(),
            Mode::Copy,
            Some(wait_bin.clone()),
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&profile).unwrap();
        let mut first = spawn_sync(
            &paths,
            &profile,
            &ResolvedRclone {
                path: wait_bin,
                source: ResolveSource::Profile,
            },
            false,
        )
        .unwrap();
        let err = execute_sync(
            &paths,
            &profile,
            ResolvedRclone {
                path: ok_bin.clone(),
                source: ResolveSource::Profile,
            },
            false,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, crate::error::Error::ProfileBusy(name) if name == "docs"));
        first.kill().unwrap();
        first.wait().unwrap();
        drop(first);
        execute_sync(
            &paths,
            &profile,
            ResolvedRclone {
                path: ok_bin,
                source: ResolveSource::Profile,
            },
            false,
            false,
        )
        .unwrap();
    }

    #[test]
    fn execute_sync_parses_json_log_into_inflight_then_clears() {
        let (root, paths) = scratch("runner-progress");
        let bin = write_exec(
            &root,
            "rclone",
            r#"#!/bin/sh
echo '{"stats":{"bytes":50,"totalBytes":200,"eta":4,"speed":10,"transferring":[{"name":"a.bin","percentage":25}]}}' >&2
sleep 2
exit 0
"#,
        );
        let store = ProfileStore::new(paths.clone());
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:x".into(),
            Mode::Copy,
            Some(bin.clone()),
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&profile).unwrap();
        let resolved = ResolvedRclone {
            path: bin,
            source: ResolveSource::Profile,
        };
        let paths_thread = paths.clone();
        let profile_thread = profile.clone();
        let handle = std::thread::spawn(move || {
            execute_sync(&paths_thread, &profile_thread, resolved, false, false)
        });
        let start = std::time::Instant::now();
        let mut saw = false;
        while start.elapsed() < std::time::Duration::from_secs(3) {
            if let Ok(Some(inf)) = crate::progress::read_inflight(&paths, "docs") {
                if inf.progress.as_ref().is_some_and(|p| p.bytes == 50) {
                    saw = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let outcome = handle.join().unwrap().unwrap();
        assert!(saw, "inflight progress never appeared");
        assert_eq!(outcome.exit_code, 0);
        assert!(crate::progress::read_inflight(&paths, "docs")
            .unwrap()
            .is_none());
    }
}
