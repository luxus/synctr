use std::fs;
use std::io;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::lock::profile_is_busy;
use crate::paths::Paths;
use crate::status::{unix_now, unix_to_rfc3339};

/// Live rclone transfer for one profile. Omitted from `status --json` when idle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferProgress {
    pub bytes: u64,
    pub total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<u64>,
    pub transfers: u64,
    pub total_transfers: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub dry_run: bool,
    pub updated_at_unix: i64,
    pub updated_at: String,
}

impl TransferProgress {
    pub fn starting(pid: Option<u32>, dry_run: bool) -> Self {
        let updated_at_unix = unix_now();
        Self {
            bytes: 0,
            total_bytes: 0,
            percent: None,
            speed_bps: None,
            eta_secs: None,
            transfers: 0,
            total_transfers: 0,
            file: None,
            pid,
            dry_run,
            updated_at_unix,
            updated_at: unix_to_rfc3339(updated_at_unix),
        }
    }

    fn touch(&mut self) {
        self.updated_at_unix = unix_now();
        self.updated_at = unix_to_rfc3339(self.updated_at_unix);
        if let Some(pct) = percent(self.bytes, self.total_bytes) {
            self.percent = Some(pct);
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(bar) = self.bar(20) {
            parts.push(bar);
        }
        if self.total_bytes > 0 {
            parts.push(format!(
                "{} / {}",
                format_bytes(self.bytes),
                format_bytes(self.total_bytes)
            ));
        } else if self.bytes > 0 {
            parts.push(format_bytes(self.bytes));
        }
        if let Some(pct) = self.percent {
            parts.push(format!("{pct}%"));
        }
        if let Some(speed) = self.speed_bps {
            parts.push(format!("{}/s", format_bytes(speed)));
        }
        if let Some(eta) = self.eta_secs {
            parts.push(format!("ETA {}", format_eta(eta)));
        }
        if parts.is_empty() {
            "starting".into()
        } else {
            parts.join("  ")
        }
    }

    pub fn bar(&self, width: usize) -> Option<String> {
        let pct = self.percent?;
        let width = width.max(1);
        let filled = (pct as usize * width) / 100;
        let filled = filled.min(width);
        Some(format!(
            "[{}{}]",
            "#".repeat(filled),
            "-".repeat(width - filled)
        ))
    }
}

fn percent(bytes: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    Some(((bytes.min(total).saturating_mul(100)) / total) as u8)
}

pub fn format_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

pub fn format_eta(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

#[derive(Debug, Deserialize)]
struct JsonLog {
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    stats: Option<RcloneStats>,
}

#[derive(Debug, Deserialize)]
struct RcloneStats {
    #[serde(default)]
    bytes: u64,
    #[serde(default, rename = "totalBytes")]
    total_bytes: u64,
    #[serde(default)]
    speed: Option<f64>,
    #[serde(default)]
    eta: Option<f64>,
    #[serde(default)]
    transfers: u64,
    #[serde(default, rename = "totalTransfers")]
    total_transfers: u64,
    #[serde(default)]
    transferring: Vec<RcloneTransferring>,
}

#[derive(Debug, Deserialize)]
struct RcloneTransferring {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    percentage: Option<i64>,
}

/// Human TUI/log line: rclone JSON `msg` / current file / stats summary.
pub fn display_rclone_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Ok(log) = serde_json::from_str::<JsonLog>(trimmed) else {
        return trimmed.to_string();
    };
    if let Some(stats) = log.stats {
        let mut tmp = TransferProgress::starting(None, false);
        apply_stats(&mut tmp, &stats);
        tmp.touch();
        return tmp.summary();
    }
    match (
        log.msg.as_deref().map(str::trim).unwrap_or(""),
        log.object.as_deref(),
    ) {
        (msg, Some(obj)) if !msg.is_empty() => format!("{msg}  {obj}"),
        (msg, _) if !msg.is_empty() => msg.to_string(),
        (_, Some(obj)) => obj.to_string(),
        _ => trimmed.to_string(),
    }
}

fn apply_stats(progress: &mut TransferProgress, stats: &RcloneStats) {
    progress.bytes = stats.bytes;
    progress.total_bytes = stats.total_bytes;
    progress.transfers = stats.transfers;
    progress.total_transfers = stats.total_transfers;
    progress.speed_bps = stats.speed.and_then(finite_u64);
    progress.eta_secs = stats.eta.and_then(finite_u64);
    if let Some(name) = stats
        .transferring
        .iter()
        .find_map(|t| t.name.as_ref())
        .cloned()
    {
        progress.file = Some(name);
    }
    if progress.percent.is_none() {
        if let Some(pct) = stats.transferring.iter().find_map(|t| t.percentage) {
            if (0..=100).contains(&pct) {
                progress.percent = Some(pct as u8);
            }
        }
    }
}

fn finite_u64(v: f64) -> Option<u64> {
    if v.is_finite() && v >= 0.0 {
        Some(v.round() as u64)
    } else {
        None
    }
}

fn apply_log_line(progress: &mut TransferProgress, line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Ok(log) = serde_json::from_str::<JsonLog>(trimmed) else {
        return false;
    };
    let mut changed = false;
    if let Some(stats) = log.stats.as_ref() {
        apply_stats(progress, stats);
        changed = true;
    }
    if let Some(obj) = log.object {
        if progress.file.as_deref() != Some(&obj) {
            progress.file = Some(obj);
            changed = true;
        }
    }
    if changed {
        progress.touch();
    }
    changed
}

pub fn write_progress(paths: &Paths, name: &str, progress: &TransferProgress) -> Result<()> {
    let dest = paths.progress_file(name);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("json.tmp");
    let bytes = serde_json::to_vec(progress).map_err(json_err)?;
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, dest)?;
    Ok(())
}

pub fn read_progress(paths: &Paths, name: &str) -> Option<TransferProgress> {
    let path = paths.progress_file(name);
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn clear_progress(paths: &Paths, name: &str) {
    let _ = fs::remove_file(paths.progress_file(name));
}

/// Live progress only while the profile lock is held. Stale files are ignored.
pub fn live_progress(paths: &Paths, name: &str) -> Option<TransferProgress> {
    if !profile_is_busy(paths, name) {
        return None;
    }
    Some(read_progress(paths, name).unwrap_or_else(|| TransferProgress::starting(None, false)))
}

fn json_err(e: serde_json::Error) -> crate::error::Error {
    crate::error::Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string()))
}

/// Parses rclone `--use-json-log` lines and writes `state/progress/<name>.json`.
/// Cleared when the last clone is dropped (sync child finished).
#[derive(Clone, Debug)]
pub struct ProgressSink {
    inner: Arc<ProgressInner>,
}

#[derive(Debug)]
struct ProgressInner {
    paths: Paths,
    name: String,
    state: Mutex<TransferProgress>,
}

impl Drop for ProgressInner {
    fn drop(&mut self) {
        clear_progress(&self.paths, &self.name);
    }
}

impl ProgressSink {
    pub fn start(paths: &Paths, name: &str, pid: u32, dry_run: bool) -> Result<Self> {
        let progress = TransferProgress::starting(Some(pid), dry_run);
        write_progress(paths, name, &progress)?;
        Ok(Self {
            inner: Arc::new(ProgressInner {
                paths: paths.clone(),
                name: name.to_string(),
                state: Mutex::new(progress),
            }),
        })
    }

    pub fn push_line(&self, line: &str) {
        let Ok(mut g) = self.inner.state.lock() else {
            return;
        };
        if apply_log_line(&mut g, line) {
            let _ = write_progress(&self.inner.paths, &self.inner.name, &g);
        }
    }

    pub fn snapshot(&self) -> Option<TransferProgress> {
        self.inner.state.lock().ok().map(|g| g.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::try_lock_profile;
    use crate::testutil::scratch;

    const STATS: &str = r#"{"level":"info","msg":"Transferred","stats":{"bytes":100,"totalBytes":400,"speed":50.4,"eta":6,"transfers":1,"totalTransfers":4,"transferring":[{"name":"a.bin","percentage":25,"bytes":100,"size":400}]}}"#;

    #[test]
    fn parses_rclone_json_stats_line() {
        let mut p = TransferProgress::starting(Some(9), false);
        assert!(apply_log_line(&mut p, STATS));
        assert_eq!(p.bytes, 100);
        assert_eq!(p.total_bytes, 400);
        assert_eq!(p.percent, Some(25));
        assert_eq!(p.speed_bps, Some(50));
        assert_eq!(p.eta_secs, Some(6));
        assert_eq!(p.transfers, 1);
        assert_eq!(p.total_transfers, 4);
        assert_eq!(p.file.as_deref(), Some("a.bin"));
        assert_eq!(p.pid, Some(9));
    }

    #[test]
    fn percent_from_bytes_when_no_transferring_pct() {
        let mut p = TransferProgress::starting(None, false);
        let line = r#"{"level":"info","stats":{"bytes":250,"totalBytes":1000,"transfers":2,"totalTransfers":8}}"#;
        assert!(apply_log_line(&mut p, line));
        assert_eq!(p.percent, Some(25));
        assert!(p.file.is_none());
    }

    #[test]
    fn object_line_sets_current_file() {
        let mut p = TransferProgress::starting(None, true);
        assert!(apply_log_line(
            &mut p,
            r#"{"level":"info","msg":"Copied (new)","object":"notes.txt","size":12}"#
        ));
        assert_eq!(p.file.as_deref(), Some("notes.txt"));
        assert!(p.dry_run);
    }

    #[test]
    fn eta_null_and_non_json_are_ignored() {
        let mut p = TransferProgress::starting(None, false);
        apply_log_line(&mut p, STATS);
        assert!(apply_log_line(
            &mut p,
            r#"{"level":"info","stats":{"bytes":100,"totalBytes":400,"eta":null,"transfers":1,"totalTransfers":4}}"#
        ));
        assert_eq!(p.eta_secs, None);
        assert!(!apply_log_line(&mut p, "not json"));
        assert_eq!(display_rclone_line("plain log"), "plain log");
        let shown = display_rclone_line(STATS);
        assert!(shown.contains("25%"), "{shown}");
        assert!(shown.contains("a.bin") || shown.contains("100 B"));
    }

    #[test]
    fn summary_and_bar() {
        let p = TransferProgress {
            bytes: 10 * 1024 * 1024,
            total_bytes: 40 * 1024 * 1024,
            percent: Some(25),
            speed_bps: Some(1024 * 1024),
            eta_secs: Some(90),
            transfers: 1,
            total_transfers: 4,
            file: Some("a.bin".into()),
            pid: Some(1),
            dry_run: false,
            updated_at_unix: 0,
            updated_at: "1970-01-01T00:00:00Z".into(),
        };
        assert_eq!(p.bar(4).as_deref(), Some("[#---]"));
        let s = p.summary();
        assert!(s.contains("25%"), "{s}");
        assert!(s.contains("ETA 1m"), "{s}");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_eta(5), "5s");
    }

    #[test]
    fn live_progress_requires_lock() {
        let (_root, paths) = scratch("progress-stale");
        let stale = TransferProgress::starting(Some(1), false);
        write_progress(&paths, "docs", &stale).unwrap();
        assert!(live_progress(&paths, "docs").is_none());
        let _lock = try_lock_profile(&paths, "docs").unwrap();
        let live = live_progress(&paths, "docs").unwrap();
        assert_eq!(live.pid, Some(1));
        drop(_lock);
        assert!(live_progress(&paths, "docs").is_none());
    }

    #[test]
    fn sink_writes_then_clears_on_drop() {
        let (_root, paths) = scratch("progress-sink");
        let path = paths.progress_file("docs");
        {
            let sink = ProgressSink::start(&paths, "docs", 42, true).unwrap();
            assert!(path.is_file());
            sink.push_line(STATS);
            let on_disk = read_progress(&paths, "docs").unwrap();
            assert_eq!(on_disk.bytes, 100);
            assert_eq!(on_disk.pid, Some(42));
            assert!(on_disk.dry_run);
            assert_eq!(sink.snapshot().unwrap().bytes, 100);
        }
        assert!(!path.exists());
    }
}
