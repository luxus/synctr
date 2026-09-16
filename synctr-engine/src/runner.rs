use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::error::Result;
use crate::ignore::load_filters;
use crate::lock::{try_lock_profile, ProfileLock};
use crate::paths::Paths;
use crate::profile::Profile;
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
    Ok(SyncChild {
        child: cmd.spawn()?,
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
    let _lock = try_lock_profile(paths, &profile.name)?;
    let filter = prepare_filter_file(paths, profile)?;
    let argv = build_sync_argv(&resolved.path, profile, &filter, dry_run);
    let mut cmd = argv.command();
    if inherit_stdio {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let status = cmd.status()?;
        let exit_code = status.code().unwrap_or(1);
        write_last_run(paths, &profile.name, LastRun::now(exit_code))?;
        Ok(SyncOutcome {
            exit_code,
            rclone: resolved,
            stdout: None,
            stderr: None,
        })
    } else {
        let output = cmd.output()?;
        let exit_code = output.status.code().unwrap_or(1);
        write_last_run(paths, &profile.name, LastRun::now(exit_code))?;
        Ok(SyncOutcome {
            exit_code,
            rclone: resolved,
            stdout: Some(output.stdout),
            stderr: Some(output.stderr),
        })
    }
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
}
