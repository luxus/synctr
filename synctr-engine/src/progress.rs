use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::Paths;
use crate::profile::validate_name;
use crate::status::{unix_now, unix_to_rfc3339};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TransferProgress {
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl TransferProgress {
    pub fn short_line(&self) -> String {
        let mut s = match (self.percent, self.total_bytes) {
            (Some(p), Some(total)) => format!(
                "{}%  {} / {}",
                p,
                human_bytes(self.bytes),
                human_bytes(total)
            ),
            (Some(p), None) => format!("{}%  {}", p, human_bytes(self.bytes)),
            (None, Some(total)) => format!("{} / {}", human_bytes(self.bytes), human_bytes(total)),
            (None, None) => human_bytes(self.bytes),
        };
        if let Some(speed) = self.speed_bps {
            s.push_str(&format!("  {}/s", human_bytes(speed)));
        }
        if let Some(eta) = self.eta_secs {
            s.push_str(&format!("  eta {eta}s"));
        }
        if let Some(name) = &self.name {
            s.push_str(&format!("  {name}"));
        }
        s
    }
}

fn human_bytes(n: u64) -> String {
    const K: u64 = 1024;
    if n < K {
        format!("{n} B")
    } else if n < K * K {
        format!("{:.1} KiB", n as f64 / K as f64)
    } else if n < K * K * K {
        format!("{:.1} MiB", n as f64 / (K * K) as f64)
    } else {
        format!("{:.1} GiB", n as f64 / (K * K * K) as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inflight {
    pub pid: u32,
    pub started_at_unix: i64,
    pub started_at: String,
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TransferProgress>,
}

impl Inflight {
    pub fn start(dry_run: bool) -> Self {
        let started_at_unix = unix_now();
        Self {
            pid: std::process::id(),
            started_at_unix,
            started_at: unix_to_rfc3339(started_at_unix),
            dry_run,
            progress: None,
        }
    }
}

pub fn write_inflight(paths: &Paths, name: &str, inflight: &Inflight) -> Result<()> {
    validate_name(name)?;
    let dest = paths.inflight(name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, toml::to_string_pretty(inflight)?)?;
    Ok(())
}

pub fn read_inflight(paths: &Paths, name: &str) -> Result<Option<Inflight>> {
    validate_name(name)?;
    let path = paths.inflight(name);
    if !path.is_file() {
        return Ok(None);
    }
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(None);
    };
    Ok(toml::from_str(&text).ok())
}

pub fn clear_inflight(paths: &Paths, name: &str) -> Result<()> {
    validate_name(name)?;
    let path = paths.inflight(name);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn update_inflight_progress(
    paths: &Paths,
    name: &str,
    progress: TransferProgress,
) -> Result<()> {
    let mut inflight = match read_inflight(paths, name)? {
        Some(i) => i,
        None => Inflight::start(false),
    };
    inflight.progress = Some(progress);
    write_inflight(paths, name, &inflight)
}

/// Apply one rclone `--use-json-log` line. Updates inflight when it contains stats.
/// Returns the parsed progress when the line was a stats object.
pub fn apply_rclone_log_line(
    paths: &Paths,
    name: &str,
    line: &str,
) -> Result<Option<TransferProgress>> {
    let Some(progress) = parse_rclone_json_line(line) else {
        return Ok(None);
    };
    update_inflight_progress(paths, name, progress.clone())?;
    Ok(Some(progress))
}

/// Copy rclone stderr to our stderr while parsing stats into inflight.
pub fn tee_rclone_stderr<R: Read>(
    paths: &Paths,
    name: &str,
    stderr: R,
    echo: bool,
) -> Result<()> {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\n', '\r']);
                let _ = apply_rclone_log_line(paths, name, trimmed);
                if echo {
                    let mut out = io::stderr().lock();
                    out.write_all(line.as_bytes())?;
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}

pub fn parse_rclone_json_line(line: &str) -> Option<TransferProgress> {
    let line = line.trim();
    if line.is_empty() || !line.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let stats = v.get("stats")?;
    let bytes = json_u64(stats.get("bytes")).unwrap_or(0);
    let total_bytes = json_u64(stats.get("totalBytes")).filter(|n| *n > 0);
    let eta_secs = json_u64(stats.get("eta"));
    let speed_bps = stats
        .get("speed")
        .and_then(|x| x.as_f64())
        .map(|f| f.max(0.0) as u64)
        .or_else(|| json_u64(stats.get("speed")));
    let transferring = stats.get("transferring").and_then(|t| t.as_array());
    let first = transferring.and_then(|t| t.first());
    let name = first
        .and_then(|o| o.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);
    let file_pct = first
        .and_then(|o| o.get("percentage"))
        .and_then(|p| p.as_f64())
        .map(|f| f.clamp(0.0, 100.0) as u32);
    let percent = match total_bytes {
        Some(total) => Some(((bytes.saturating_mul(100)) / total).min(100) as u32),
        None => file_pct,
    };
    Some(TransferProgress {
        bytes,
        total_bytes,
        percent,
        eta_secs,
        speed_bps,
        name,
    })
}

fn json_u64(v: Option<&serde_json::Value>) -> Option<u64> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    #[test]
    fn parse_stats_object() {
        let line = r#"{"level":"info","msg":"","stats":{"bytes":250,"totalBytes":1000,"eta":12,"speed":80.5,"transferring":[{"name":"docs/a.bin","percentage":25}]}}"#;
        let p = parse_rclone_json_line(line).unwrap();
        assert_eq!(p.bytes, 250);
        assert_eq!(p.total_bytes, Some(1000));
        assert_eq!(p.percent, Some(25));
        assert_eq!(p.eta_secs, Some(12));
        assert_eq!(p.speed_bps, Some(80));
        assert_eq!(p.name.as_deref(), Some("docs/a.bin"));
        assert!(p.short_line().contains("25%"));
    }

    #[test]
    fn parse_ignores_null_eta_and_non_stats() {
        assert!(parse_rclone_json_line(r#"{"level":"info","msg":"Copied (new)","object":"a"}"#).is_none());
        let p = parse_rclone_json_line(
            r#"{"stats":{"bytes":1,"totalBytes":0,"eta":null,"speed":0}}"#,
        )
        .unwrap();
        assert_eq!(p.bytes, 1);
        assert!(p.total_bytes.is_none());
        assert!(p.eta_secs.is_none());
    }

    #[test]
    fn inflight_roundtrip_and_apply() {
        let (_root, paths) = scratch("inflight-roundtrip");
        let inf = Inflight::start(true);
        write_inflight(&paths, "docs", &inf).unwrap();
        let got = read_inflight(&paths, "docs").unwrap().unwrap();
        assert_eq!(got.dry_run, true);
        assert!(got.progress.is_none());
        apply_rclone_log_line(
            &paths,
            "docs",
            r#"{"stats":{"bytes":10,"totalBytes":40,"eta":3,"speed":2}}"#,
        )
        .unwrap();
        let got = read_inflight(&paths, "docs").unwrap().unwrap();
        assert_eq!(got.progress.unwrap().bytes, 10);
        clear_inflight(&paths, "docs").unwrap();
        assert!(read_inflight(&paths, "docs").unwrap().is_none());
    }
}
