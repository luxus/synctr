use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use crate::error::{Error, Result};
use crate::ignore::load_filters;
use crate::lock::{try_lock_profile, ProfileLock};
use crate::paths::Paths;
use crate::profile::Profile;
use crate::progress::ProgressSink;
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
    pub progress: ProgressSink,
    _lock: ProfileLock,
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
    let filter = prepare_filter_file(paths, profile)?;
    let argv = build_sync_argv(&resolved.path, profile, &filter, dry_run);
    let mut cmd = argv.command();
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let progress = match ProgressSink::start(paths, &profile.name, pid, dry_run) {
        Ok(progress) => progress,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    Ok(SyncChild {
        child,
        progress,
        _lock,
    })
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
    let mut spawned = spawn_sync(paths, profile, &resolved, dry_run)?;
    let (stdout, stderr) = drain_rclone(&mut spawned, inherit_stdio)?;
    let status = spawned.child.wait()?;
    let exit_code = status.code().unwrap_or(1);
    drop(spawned);
    write_last_run(paths, &profile.name, LastRun::now(exit_code))?;
    Ok(SyncOutcome {
        exit_code,
        rclone: resolved,
        stdout,
        stderr,
    })
}

fn drain_rclone(
    spawned: &mut SyncChild,
    inherit: bool,
) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>)> {
    let stdout = spawned.child.stdout.take();
    let stderr = spawned.child.stderr.take();
    let out_h = stdout.map(|r| {
        let progress = spawned.progress.clone();
        thread::spawn(move || pump_pipe(r, progress, inherit, false))
    });
    let err_h = stderr.map(|r| {
        let progress = spawned.progress.clone();
        thread::spawn(move || pump_pipe(r, progress, inherit, true))
    });
    let stdout_buf = match out_h {
        Some(h) => Some(join_pipe(h)?),
        None => None,
    };
    let stderr_buf = match err_h {
        Some(h) => Some(join_pipe(h)?),
        None => None,
    };
    if inherit {
        Ok((None, None))
    } else {
        Ok((stdout_buf, stderr_buf))
    }
}

fn join_pipe(h: thread::JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>> {
    h.join()
        .map_err(|_| {
            Error::Io(io::Error::new(
                io::ErrorKind::Other,
                "rclone pipe thread panicked",
            ))
        })?
        .map_err(Error::from)
}

fn pump_pipe<R: Read>(
    reader: R,
    progress: ProgressSink,
    inherit: bool,
    is_err: bool,
) -> io::Result<Vec<u8>> {
    let mut buf = BufReader::new(reader);
    let mut collected = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = buf.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        progress.push_line(line.trim_end_matches(['\n', '\r']));
        if inherit {
            if is_err {
                eprint!("{line}");
                let _ = io::stderr().flush();
            } else {
                print!("{line}");
                let _ = io::stdout().flush();
            }
        }
        collected.extend_from_slice(line.as_bytes());
    }
    Ok(collected)
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
    fn spawn_sync_parses_json_log_into_live_progress() {
        let (root, paths) = scratch("runner-progress");
        let bin = write_exec(
            &root,
            "rclone",
            r#"#!/bin/sh
echo '{"level":"info","msg":"Transferred","stats":{"bytes":100,"totalBytes":400,"speed":50,"eta":6,"transfers":1,"totalTransfers":4,"transferring":[{"name":"a.bin","percentage":25}]}}' >&2
while true; do sleep 1; done
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
        let mut child = spawn_sync(
            &paths,
            &profile,
            &ResolvedRclone {
                path: bin,
                source: ResolveSource::Profile,
            },
            false,
        )
        .unwrap();
        if let Some(err) = child.stderr.take() {
            let progress = child.progress.clone();
            std::thread::spawn(move || {
                let mut buf = std::io::BufReader::new(err);
                let mut line = String::new();
                while std::io::BufRead::read_line(&mut buf, &mut line).unwrap_or(0) > 0 {
                    progress.push_line(line.trim_end_matches(['\n', '\r']));
                    line.clear();
                }
            });
        }
        let start = std::time::Instant::now();
        let mut seen = None;
        while start.elapsed() < std::time::Duration::from_secs(2) {
            if let Some(p) = crate::progress::live_progress(&paths, "docs") {
                if p.bytes == 100 {
                    seen = Some(p);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let snap =
            crate::status::status_snapshot(&store, Some(&profile.rclone.clone().unwrap()), None)
                .unwrap();
        child.kill().unwrap();
        let _ = child.wait();
        drop(child);
        let p = seen.expect("live progress from rclone json log");
        assert_eq!(p.total_bytes, 400);
        assert_eq!(p.percent, Some(25));
        assert_eq!(p.file.as_deref(), Some("a.bin"));
        assert_eq!(
            snap.profiles[0].progress.as_ref().map(|x| x.bytes),
            Some(100)
        );
        assert!(crate::progress::live_progress(&paths, "docs").is_none());
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
}
