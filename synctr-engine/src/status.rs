use std::fs;
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
        let finished_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            finished_at: unix_to_rfc3339(finished_at_unix),
            finished_at_unix,
            exit_code,
            ok: exit_code == 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RcloneJson {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileStatus {
    pub name: String,
    pub local: PathBuf,
    pub remote: String,
    pub mode: Mode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rclone: Option<PathBuf>,
    pub extra_flags: Vec<String>,
    pub extra_ignore: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusSnapshot {
    pub rclone: RcloneJson,
    pub profiles: Vec<ProfileStatus>,
}

pub fn status_snapshot(
    store: &ProfileStore,
    rclone_flag: Option<&std::path::Path>,
    profile_rclone: Option<&std::path::Path>,
) -> Result<StatusSnapshot> {
    let rclone = match resolve_rclone_live(rclone_flag, profile_rclone) {
        Ok(r) => RcloneJson::from(&r),
        Err(crate::error::Error::RcloneNotFound) => RcloneJson::missing(),
        Err(e) => return Err(e),
    };
    let profiles = store
        .list()?
        .into_iter()
        .map(|p| ProfileStatus::from_profile(store.paths(), p))
        .collect::<Result<Vec<_>>>()?;
    Ok(StatusSnapshot { rclone, profiles })
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
    Ok(Some(toml::from_str(&fs::read_to_string(path)?)?))
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

    #[test]
    fn rfc3339_unix_epoch() {
        assert_eq!(unix_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_rfc3339(1), "1970-01-01T00:00:01Z");
        assert_eq!(unix_to_rfc3339(86400), "1970-01-02T00:00:00Z");
    }
}
