use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::lock::profile_is_busy;
use crate::paths::Paths;
use crate::profile::{Mode, Profile, ProfileStore};
use crate::rclone::{resolve_rclone_live, ResolvedRclone};
use crate::status::{read_last_run, LastRun, RcloneJson};

const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorRclone {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl DoctorRclone {
    fn from_json(json: RcloneJson, version: Option<String>) -> Self {
        Self {
            found: json.found,
            path: json.path,
            source: json.source,
            detail: json.detail,
            version,
        }
    }

    fn missing() -> Self {
        Self {
            found: false,
            path: None,
            source: None,
            detail: None,
            version: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorProfile {
    pub name: String,
    pub ok: bool,
    pub mode: Mode,
    pub local: PathBuf,
    pub local_exists: bool,
    pub remote: String,
    pub busy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub rclone: DoctorRclone,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub profiles: Vec<DoctorProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteProbe {
    pub ok: bool,
    pub name: String,
    pub remote: String,
    pub detail: String,
}

pub fn doctor_report(
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
) -> crate::error::Result<DoctorReport> {
    let paths = store.paths();
    let rclone = match resolve_rclone_live(rclone_flag, None) {
        Ok(found) => {
            let version = rclone_version(&found.path, DEFAULT_PROBE_TIMEOUT);
            DoctorRclone::from_json(RcloneJson::from(&found), version)
        }
        Err(crate::error::Error::RcloneNotFound)
        | Err(crate::error::Error::RcloneOverrideMissing { .. }) => DoctorRclone::missing(),
        Err(e) => return Err(e),
    };
    let profiles = store
        .list()?
        .into_iter()
        .map(|p| doctor_profile(paths, p))
        .collect::<Vec<_>>();
    let ok = rclone.found && profiles.iter().all(|p| p.ok);
    Ok(DoctorReport {
        ok,
        rclone,
        config_dir: paths.config_dir.clone(),
        state_dir: paths.state_dir.clone(),
        profiles,
    })
}

fn doctor_profile(paths: &Paths, profile: Profile) -> DoctorProfile {
    let local_exists = profile.local.is_dir();
    let busy = profile_is_busy(paths, &profile.name);
    let last_run = read_last_run(paths, &profile.name).ok().flatten();
    let mut issues = Vec::new();
    if !local_exists {
        issues.push(format!("local path missing: {}", profile.local.display()));
    }
    DoctorProfile {
        name: profile.name,
        ok: issues.is_empty(),
        mode: profile.mode,
        local: profile.local,
        local_exists,
        remote: profile.remote,
        busy,
        last_run,
        issues,
    }
}

pub fn test_remote(
    store: &ProfileStore,
    rclone_flag: Option<&Path>,
    name: &str,
    timeout: Duration,
) -> crate::error::Result<RemoteProbe> {
    let profile = store.get(name)?;
    let found = resolve_rclone_live(rclone_flag, profile.rclone.as_deref())?;
    Ok(probe_remote(&found, &profile.name, &profile.remote, timeout))
}

pub fn probe_remote(
    rclone: &ResolvedRclone,
    name: &str,
    remote: &str,
    timeout: Duration,
) -> RemoteProbe {
    let outcome = run_rclone_timeout(&rclone.path, &["lsd", remote], timeout);
    let (ok, detail) = match outcome {
        TimedRun::TimedOut => (
            false,
            format!("timeout after {}s", timeout.as_secs().max(1)),
        ),
        TimedRun::Finished {
            code,
            stderr,
            stdout,
        } => {
            let tail = first_line(&stderr).or_else(|| first_line(&stdout));
            if code == 0 {
                (true, tail.unwrap_or_else(|| "lsd ok".into()))
            } else {
                (false, tail.unwrap_or_else(|| format!("lsd exit {code}")))
            }
        }
        TimedRun::Spawn(err) => (false, err),
    };
    RemoteProbe {
        ok,
        name: name.to_string(),
        remote: remote.to_string(),
        detail,
    }
}

fn rclone_version(bin: &Path, timeout: Duration) -> Option<String> {
    match run_rclone_timeout(bin, &["version"], timeout) {
        TimedRun::Finished { code: 0, stdout, .. } => first_line(&stdout),
        _ => None,
    }
}

#[derive(Debug)]
enum TimedRun {
    Finished {
        code: i32,
        stdout: String,
        stderr: String,
    },
    TimedOut,
    Spawn(String),
}

fn run_rclone_timeout(bin: &Path, args: &[&str], timeout: Duration) -> TimedRun {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return TimedRun::Spawn(e.to_string()),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = read_pipe(child.stdout.take());
                let stderr = read_pipe(child.stderr.take());
                return TimedRun::Finished {
                    code: status.code().unwrap_or(1),
                    stdout,
                    stderr,
                };
            }
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return TimedRun::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return TimedRun::Spawn(e.to_string()),
        }
    }
}

fn read_pipe(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn first_line(s: &str) -> Option<String> {
    let line = s.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

impl DoctorReport {
    pub fn write_human<W: Write>(&self, mut w: W) -> std::io::Result<()> {
        match (&self.rclone.found, &self.rclone.path, &self.rclone.version) {
            (true, Some(path), Some(ver)) => {
                writeln!(w, "rclone: {} ({})", path.display(), ver)?;
            }
            (true, Some(path), None) => {
                writeln!(w, "rclone: {}", path.display())?;
            }
            _ => writeln!(w, "rclone: not found")?,
        }
        writeln!(w, "config: {}", self.config_dir.display())?;
        writeln!(w, "state:  {}", self.state_dir.display())?;
        if self.profiles.is_empty() {
            writeln!(w, "no profiles")?;
            return Ok(());
        }
        for p in &self.profiles {
            let local = if p.local_exists { "local=ok" } else { "local=MISSING" };
            let busy = if p.busy { "busy" } else { "idle" };
            let last = match &p.last_run {
                Some(run) if run.ok => format!("ok {}", run.finished_at),
                Some(run) => format!("exit {} {}", run.exit_code, run.finished_at),
                None => "never".into(),
            };
            writeln!(
                w,
                "{}\t{}\t{}\t->\t{}\t{local}\t{busy}\t{last}",
                p.name,
                p.mode,
                p.local.display(),
                p.remote
            )?;
            for issue in &p.issues {
                writeln!(w, "  - {issue}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::rclone::ResolveSource;
    use crate::testutil::{scratch, write_exec};
    use crate::ProfileStore;

    #[test]
    fn doctor_reports_missing_rclone_and_missing_local() {
        let (_root, paths) = scratch("doctor-missing");
        let store = ProfileStore::new(paths.clone());
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/no/such/synctr-local"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&p).unwrap();
        let report = doctor_report(&store, Some(Path::new("/no/such/rclone"))).unwrap();
        assert!(!report.ok);
        assert!(!report.rclone.found);
        assert_eq!(report.profiles.len(), 1);
        assert!(!report.profiles[0].ok);
        assert!(!report.profiles[0].local_exists);
        assert!(!report.profiles[0].busy);
        assert!(report.profiles[0].issues[0].contains("local path missing"));
    }

    #[test]
    fn doctor_ok_when_rclone_and_local_exist() {
        let (root, paths) = scratch("doctor-ok");
        let local = root.join("docs");
        std::fs::create_dir_all(&local).unwrap();
        let bin = write_exec(
            &root,
            "rclone",
            "#!/bin/sh\nif [ \"$1\" = version ]; then echo rclone v9.9.9; exit 0; fi\nexit 0\n",
        );
        let store = ProfileStore::new(paths);
        store
            .add(
                &Profile::new(
                    "docs".into(),
                    local,
                    "b2:bucket/docs".into(),
                    Mode::Copy,
                    Some(bin.clone()),
                    vec![],
                    vec![],
                )
                .unwrap(),
            )
            .unwrap();
        let report = doctor_report(&store, Some(&bin)).unwrap();
        assert!(report.ok);
        assert!(report.rclone.found);
        assert_eq!(report.rclone.version.as_deref(), Some("rclone v9.9.9"));
        assert!(report.profiles[0].ok);
        assert!(report.profiles[0].local_exists);
    }

    #[test]
    fn probe_remote_ok_and_timeout() {
        let (root, _paths) = scratch("doctor-probe");
        let ok = write_exec(&root, "rclone-ok", "#!/bin/sh\necho listed\nexit 0\n");
        let hang = write_exec(
            &root,
            "rclone-hang",
            "#!/bin/sh\nwhile true; do sleep 1; done\n",
        );
        let fail = write_exec(&root, "rclone-fail", "#!/bin/sh\necho nope >&2\nexit 3\n");
        let found_ok = ResolvedRclone {
            path: ok,
            source: ResolveSource::Flag,
        };
        let p = probe_remote(&found_ok, "docs", "b2:bucket/docs", Duration::from_secs(2));
        assert!(p.ok);
        assert_eq!(p.remote, "b2:bucket/docs");
        assert_eq!(p.detail, "listed");

        let found_hang = ResolvedRclone {
            path: hang,
            source: ResolveSource::Flag,
        };
        let p = probe_remote(
            &found_hang,
            "docs",
            "b2:bucket/docs",
            Duration::from_millis(200),
        );
        assert!(!p.ok);
        assert!(p.detail.contains("timeout"), "{}", p.detail);

        let found_fail = ResolvedRclone {
            path: fail,
            source: ResolveSource::Flag,
        };
        let p = probe_remote(&found_fail, "docs", "b2:x", Duration::from_secs(2));
        assert!(!p.ok);
        assert_eq!(p.detail, "nope");
    }
}
