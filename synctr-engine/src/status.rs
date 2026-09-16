use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::Paths;
use crate::profile::{Mode, Profile, ProfileStore};
use crate::rclone::{resolve_rclone_live, ResolvedRclone};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub finished_at_unix: i64,
    pub finished_at: String,
    pub exit_code: i32,
    pub ok: bool,
}

impl LastRun {
    pub fn now(exit_code: i32) -> Self {
        let finished_at_unix = unix_now();
        Self {
            finished_at: unix_to_rfc3339(finished_at_unix),
            finished_at_unix,
            exit_code,
            ok: exit_code == 0,
        }
    }

    pub fn short_status(&self) -> String {
        if self.ok {
            format!("ok {}", self.finished_at)
        } else {
            format!("exit {} {}", self.exit_code, self.finished_at)
        }
    }
}

pub fn last_run_short(run: Option<&LastRun>) -> String {
    match run {
        Some(run) => run.short_status(),
        None => "never".into(),
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn age_label(unix: i64) -> String {
    age_label_at(unix, unix_now())
}

pub fn age_label_at(unix: i64, now_unix: i64) -> String {
    let d = (now_unix - unix).max(0);
    if d < 60 {
        format!("{d}s ago")
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86400)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RcloneJson {
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl From<&ResolvedRclone> for RcloneJson {
    fn from(r: &ResolvedRclone) -> Self {
        Self {
            found: true,
            path: Some(r.path.clone()),
            source: Some(r.source.as_str().to_string()),
            detail: Some(r.source.explain().to_string()),
        }
    }
}

impl RcloneJson {
    pub fn missing() -> Self {
        Self {
            found: false,
            path: None,
            source: None,
            detail: None,
        }
    }

    pub fn from_live(
        flag: Option<&std::path::Path>,
        profile_rclone: Option<&std::path::Path>,
    ) -> Result<Self> {
        match resolve_rclone_live(flag, profile_rclone) {
            Ok(r) => Ok(Self::from(&r)),
            Err(crate::error::Error::RcloneNotFound)
            | Err(crate::error::Error::RcloneOverrideMissing { .. }) => Ok(Self::missing()),
            Err(e) => Err(e),
        }
    }

    pub fn human_line(&self) -> String {
        match (self.found, self.path.as_ref(), self.detail.as_ref()) {
            (true, Some(path), Some(detail)) => format!("{} ({})", path.display(), detail),
            _ => "not found".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStatus {
    pub name: String,
    pub local: PathBuf,
    pub remote: String,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rclone: Option<PathBuf>,
    pub extra_flags: Vec<String>,
    pub extra_ignore: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
}

impl ProfileStatus {
    pub fn from_profile(paths: &Paths, profile: Profile) -> Result<Self> {
        let last_run = read_last_run(paths, &profile.name)?;
        Ok(Self {
            name: profile.name,
            local: profile.local,
            remote: profile.remote,
            mode: profile.mode,
            rclone: profile.rclone,
            extra_flags: profile.extra_flags,
            extra_ignore: profile.extra_ignore,
            last_run,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub rclone: RcloneJson,
    pub profiles: Vec<ProfileStatus>,
}

pub fn status_snapshot(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    profile_rclone: Option<&std::path::Path>,
) -> Result<StatusSnapshot> {
    let rclone = RcloneJson::from_live(rclone_flag, profile_rclone)?;
    let profiles = store
        .list()?
        .into_iter()
        .map(|p| ProfileStatus::from_profile(store.paths(), p))
        .collect::<Result<Vec<_>>>()?;
    Ok(StatusSnapshot { rclone, profiles })
}

/// Compact JSON plus a trailing newline. This is the `synctr status --json` contract.
pub fn write_status_json<W: Write>(mut w: W, snap: &StatusSnapshot) -> Result<()> {
    serde_json::to_writer(&mut w, snap).map_err(json_err)?;
    w.write_all(b"\n")?;
    Ok(())
}

pub fn status_json(snap: &StatusSnapshot) -> Result<String> {
    let mut buf = Vec::new();
    write_status_json(&mut buf, snap)?;
    String::from_utf8(buf).map_err(|e| {
        crate::error::Error::Io(io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    })
}

/// Field checks for the Noctalia / menu-bar contract. Panics on mismatch (test helper).
pub fn assert_status_json_contract(v: &serde_json::Value) {
    let rclone = v.get("rclone").expect("rclone");
    assert!(rclone.get("found").and_then(|x| x.as_bool()).is_some());
    let profiles = v
        .get("profiles")
        .and_then(|p| p.as_array())
        .expect("profiles array");
    for p in profiles {
        for key in [
            "name",
            "local",
            "remote",
            "mode",
            "extra_flags",
            "extra_ignore",
        ] {
            assert!(p.get(key).is_some(), "missing profile.{key}");
        }
        if let Some(last) = p.get("last_run") {
            if !last.is_null() {
                for key in ["finished_at_unix", "finished_at", "exit_code", "ok"] {
                    assert!(last.get(key).is_some(), "missing last_run.{key}");
                }
            }
        }
    }
    serde_json::from_value::<StatusSnapshot>(v.clone()).expect("StatusSnapshot deserialize");
}

fn json_err(e: serde_json::Error) -> crate::error::Error {
    crate::error::Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string()))
}

pub fn write_last_run(paths: &Paths, name: &str, run: LastRun) -> Result<()> {
    let dest = paths.last_run(name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, toml::to_string_pretty(&run)?)?;
    Ok(())
}

pub fn read_last_run(paths: &Paths, name: &str) -> Result<Option<LastRun>> {
    let path = paths.last_run(name);
    if !path.is_file() {
        return Ok(None);
    }
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(None);
    };
    Ok(toml::from_str(&text).ok())
}

fn unix_to_rfc3339(secs: i64) -> String {
    let z = secs.max(0) as u64;
    let days = (z / 86400) as i64;
    let rem = z % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Mode, Profile, ProfileStore};
    use crate::testutil::{scratch, write_exec};
    use std::path::PathBuf;

    #[test]
    fn rfc3339_unix_epoch() {
        assert_eq!(unix_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_rfc3339(1), "1970-01-01T00:00:01Z");
        assert_eq!(unix_to_rfc3339(86400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn age_label_buckets() {
        assert_eq!(age_label_at(100, 100), "0s ago");
        assert_eq!(age_label_at(100, 159), "59s ago");
        assert_eq!(age_label_at(100, 160), "1m ago");
        assert_eq!(age_label_at(0, 3600), "1h ago");
        assert_eq!(age_label_at(0, 86400), "1d ago");
        assert_eq!(age_label_at(50, 10), "0s ago");
    }

    #[test]
    fn last_run_short_ok_fail_never() {
        let ok = LastRun {
            finished_at_unix: 0,
            finished_at: "1970-01-01T00:00:00Z".into(),
            exit_code: 0,
            ok: true,
        };
        let fail = LastRun {
            finished_at_unix: 0,
            finished_at: "1970-01-01T00:00:00Z".into(),
            exit_code: 7,
            ok: false,
        };
        assert_eq!(last_run_short(Some(&ok)), "ok 1970-01-01T00:00:00Z");
        assert_eq!(last_run_short(Some(&fail)), "exit 7 1970-01-01T00:00:00Z");
        assert_eq!(last_run_short(None), "never");
    }

    #[test]
    fn status_json_omits_missing_rclone_and_last_run() {
        let snap = StatusSnapshot {
            rclone: RcloneJson::missing(),
            profiles: vec![ProfileStatus {
                name: "docs".into(),
                local: PathBuf::from("/tmp/docs"),
                remote: "b2:bucket/docs".into(),
                mode: Mode::Sync,
                rclone: None,
                extra_flags: vec!["--checksum".into()],
                extra_ignore: vec!["*.key".into()],
                last_run: None,
            }],
        };
        let json = status_json(&snap).unwrap();
        assert!(json.ends_with('\n'));
        assert_eq!(
            json,
            "{\"rclone\":{\"found\":false},\"profiles\":[{\"name\":\"docs\",\"local\":\"/tmp/docs\",\"remote\":\"b2:bucket/docs\",\"mode\":\"sync\",\"extra_flags\":[\"--checksum\"],\"extra_ignore\":[\"*.key\"]}]}\n"
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_status_json_contract(&v);
        assert!(v["rclone"].get("path").is_none());
        assert!(v["profiles"][0].get("last_run").is_none());
        assert!(v["profiles"][0].get("rclone").is_none());
        let round: StatusSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(round, snap);
    }

    #[test]
    fn status_json_includes_rclone_and_last_run_when_present() {
        let snap = StatusSnapshot {
            rclone: RcloneJson {
                found: true,
                path: Some(PathBuf::from("/usr/bin/rclone")),
                source: Some("PATH".into()),
                detail: Some("PATH".into()),
            },
            profiles: vec![ProfileStatus {
                name: "docs".into(),
                local: PathBuf::from("/tmp/docs"),
                remote: "b2:bucket/docs".into(),
                mode: Mode::Copy,
                rclone: Some(PathBuf::from("/custom/rclone")),
                extra_flags: vec![],
                extra_ignore: vec![],
                last_run: Some(LastRun {
                    finished_at_unix: 0,
                    finished_at: "1970-01-01T00:00:00Z".into(),
                    exit_code: 0,
                    ok: true,
                }),
            }],
        };
        let json = status_json(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_status_json_contract(&v);
        assert_eq!(v["rclone"]["found"], true);
        assert_eq!(v["rclone"]["path"], "/usr/bin/rclone");
        assert_eq!(v["rclone"]["source"], "PATH");
        assert_eq!(v["profiles"][0]["last_run"]["ok"], true);
        assert_eq!(v["profiles"][0]["rclone"], "/custom/rclone");
        let round: StatusSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(round, snap);
    }

    #[test]
    fn snapshot_json_uses_flag_rclone_and_omits_never_run() {
        let (root, paths) = scratch("status-snapshot-json");
        let bin = write_exec(&root, "rclone", "#!/bin/sh\nexit 0\n");
        let store = ProfileStore::new(paths);
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec!["--checksum".into()],
            vec!["*.key".into()],
        )
        .unwrap();
        store.add(&profile).unwrap();
        let snap = status_snapshot(&store, Some(&bin), None).unwrap();
        let json = status_json(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_status_json_contract(&v);
        assert_eq!(v["rclone"]["found"], true);
        assert_eq!(v["rclone"]["path"], bin.to_str().unwrap());
        assert_eq!(v["profiles"][0]["name"], "docs");
        assert!(v["profiles"][0].get("last_run").is_none());
        assert_eq!(
            snap.rclone.human_line(),
            format!("{} (--rclone)", bin.display())
        );
    }

    #[test]
    fn from_live_missing_override_is_found_false() {
        let missing = PathBuf::from("/no/such/rclone-for-status-json");
        let json = RcloneJson::from_live(Some(&missing), None).unwrap();
        assert!(!json.found);
        assert!(json.path.is_none());
        assert!(json.source.is_none());
        assert!(json.detail.is_none());
        assert_eq!(json.human_line(), "not found");
    }

    #[test]
    fn snapshot_skips_corrupt_last_run_and_keeps_good_profiles() {
        let (root, paths) = scratch("status-skip-corrupt");
        let bin = write_exec(&root, "rclone", "#!/bin/sh\nexit 0\n");
        let store = ProfileStore::new(paths.clone());
        let profile = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&profile).unwrap();
        crate::status::write_last_run(&paths, "docs", LastRun::now(0)).unwrap();
        fs::write(paths.last_run("docs"), "not a last-run table {{{").unwrap();
        fs::write(paths.profile_toml("zzz-bad"), "nope").unwrap();
        let snap = status_snapshot(&store, Some(&bin), None).unwrap();
        assert_eq!(snap.profiles.len(), 1);
        assert_eq!(snap.profiles[0].name, "docs");
        assert!(snap.profiles[0].last_run.is_none());
        let json = status_json(&snap).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_status_json_contract(&v);
        assert!(v["profiles"][0].get("last_run").is_none());
    }
}
