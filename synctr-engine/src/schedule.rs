use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::profile::validate_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    Systemd,
    Launchd,
}

impl ScheduleKind {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "systemd" => Ok(Self::Systemd),
            "launchd" => Ok(Self::Launchd),
            other => Err(Error::InvalidScheduleKind(other.to_string())),
        }
    }

    pub fn default_for_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Launchd
        } else {
            Self::Systemd
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::Launchd => "launchd",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleFile {
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleSpec {
    pub kind: ScheduleKind,
    pub interval_secs: u64,
    pub bin: PathBuf,
    pub profile: String,
    pub files: Vec<ScheduleFile>,
}

pub fn generate_schedule(
    kind: ScheduleKind,
    name: &str,
    bin: &Path,
    interval_secs: u64,
    config_dir: Option<&Path>,
) -> Result<ScheduleSpec> {
    validate_name(name)?;
    if interval_secs == 0 {
        return Err(Error::InvalidInterval);
    }
    let files = match kind {
        ScheduleKind::Systemd => systemd_files(name, bin, interval_secs, config_dir),
        ScheduleKind::Launchd => vec![launchd_file(name, bin, interval_secs, config_dir)],
    };
    Ok(ScheduleSpec {
        kind,
        interval_secs,
        bin: bin.to_path_buf(),
        profile: name.to_string(),
        files,
    })
}

pub fn schedule_filenames(kind: ScheduleKind, name: &str) -> Vec<String> {
    match kind {
        ScheduleKind::Systemd => vec![
            format!("synctr-{name}.service"),
            format!("synctr-{name}.timer"),
        ],
        ScheduleKind::Launchd => vec![format!("dev.luxus.synctr.{name}.plist")],
    }
}

pub fn default_install_dir(kind: ScheduleKind) -> Result<PathBuf> {
    install_dir_from(
        kind,
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        dirs::home_dir(),
    )
}

fn install_dir_from(
    kind: ScheduleKind,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf> {
    match kind {
        ScheduleKind::Systemd => {
            if let Some(xdg) = xdg_config_home {
                return Ok(xdg.join("systemd/user"));
            }
            let home = home.ok_or(Error::HomeNotFound)?;
            Ok(home.join(".config/systemd/user"))
        }
        ScheduleKind::Launchd => {
            let home = home.ok_or(Error::HomeNotFound)?;
            Ok(home.join("Library/LaunchAgents"))
        }
    }
}

pub fn enable_hint(kind: ScheduleKind, dest: &Path, name: &str, used_default: bool) -> String {
    match kind {
        ScheduleKind::Systemd => {
            let mut hint =
                format!("not enabled. to start: systemctl --user enable --now synctr-{name}.timer");
            if !used_default {
                hint.push_str(&format!(
                    "\nwrote under {}; systemd --user only loads from the user unit dir unless you link these files there",
                    dest.display()
                ));
            }
            hint
        }
        ScheduleKind::Launchd => {
            let plist = dest.join(format!("dev.luxus.synctr.{name}.plist"));
            format!("not loaded. to start: launchctl load {}", plist.display())
        }
    }
}

pub fn install_schedule(dir: &Path, spec: &ScheduleSpec) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for file in &spec.files {
        let dest = dir.join(&file.name);
        std::fs::write(&dest, &file.body)?;
        written.push(dest);
    }
    Ok(written)
}

pub fn uninstall_schedule(dir: &Path, kind: ScheduleKind, name: &str) -> Result<Vec<PathBuf>> {
    validate_name(name)?;
    let mut removed = Vec::new();
    for file in schedule_filenames(kind, name) {
        let dest = dir.join(file);
        if dest.exists() {
            std::fs::remove_file(&dest)?;
            removed.push(dest);
        }
    }
    Ok(removed)
}

fn systemd_files(
    name: &str,
    bin: &Path,
    interval_secs: u64,
    config_dir: Option<&Path>,
) -> Vec<ScheduleFile> {
    let exec = systemd_exec(bin, name, config_dir);
    let service = format!(
        "[Unit]\nDescription=synctr sync {name}\n\n[Service]\nType=oneshot\nExecStart={exec}\n"
    );
    let timer = format!(
        "[Unit]\nDescription=synctr timer for {name}\n\n[Timer]\nOnBootSec=1min\nOnUnitActiveSec={interval_secs}s\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"
    );
    vec![
        ScheduleFile {
            name: format!("synctr-{name}.service"),
            body: service,
        },
        ScheduleFile {
            name: format!("synctr-{name}.timer"),
            body: timer,
        },
    ]
}

fn systemd_exec(bin: &Path, name: &str, config_dir: Option<&Path>) -> String {
    let mut parts = vec![systemd_quote(&bin.display().to_string())];
    if let Some(dir) = config_dir {
        parts.push("--config-dir".into());
        parts.push(systemd_quote(&dir.display().to_string()));
    }
    parts.push("sync".into());
    parts.push(name.to_string());
    parts.join(" ")
}

fn launchd_file(
    name: &str,
    bin: &Path,
    interval_secs: u64,
    config_dir: Option<&Path>,
) -> ScheduleFile {
    let label = format!("dev.luxus.synctr.{name}");
    let bin_xml = xml_escape(&bin.display().to_string());
    let name_xml = xml_escape(name);
    let config_args = match config_dir {
        Some(dir) => format!(
            "\n\t\t<string>--config-dir</string>\n\t\t<string>{}</string>",
            xml_escape(&dir.display().to_string())
        ),
        None => String::new(),
    };
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{bin_xml}</string>{config_args}
		<string>sync</string>
		<string>{name_xml}</string>
	</array>
	<key>StartInterval</key>
	<integer>{interval_secs}</integer>
	<key>RunAtLoad</key>
	<false/>
</dict>
</plist>
"#
    );
    ScheduleFile {
        name: format!("{label}.plist"),
        body,
    }
}

fn systemd_quote(s: &str) -> String {
    let needs_quote = s
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '%' | ';'));
    if needs_quote {
        // systemd still expands $ and % inside double quotes; $$ / %% are literals.
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "$$")
            .replace('%', "%%");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    #[test]
    fn systemd_unit_calls_synctr_sync_on_interval() {
        let spec = generate_schedule(
            ScheduleKind::Systemd,
            "docs",
            Path::new("/opt/synctr/bin/synctr"),
            1800,
            None,
        )
        .unwrap();
        assert_eq!(spec.files.len(), 2);
        assert_eq!(spec.files[0].name, "synctr-docs.service");
        assert!(spec.files[0]
            .body
            .contains("ExecStart=/opt/synctr/bin/synctr sync docs"));
        assert!(spec.files[0].body.contains("Type=oneshot"));
        assert_eq!(spec.files[1].name, "synctr-docs.timer");
        assert!(spec.files[1].body.contains("OnUnitActiveSec=1800s"));
        assert!(spec.files[1].body.contains("WantedBy=timers.target"));
        assert!(!spec.files.iter().any(|f| f.body.contains("systemctl")));
    }

    #[test]
    fn launchd_plist_uses_program_arguments_not_a_shell() {
        let spec = generate_schedule(
            ScheduleKind::Launchd,
            "docs",
            Path::new("/opt/synctr/bin/synctr"),
            600,
            None,
        )
        .unwrap();
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].name, "dev.luxus.synctr.docs.plist");
        let body = &spec.files[0].body;
        assert!(body.contains("<string>dev.luxus.synctr.docs</string>"));
        assert!(body.contains("<string>/opt/synctr/bin/synctr</string>"));
        assert!(body.contains("<string>sync</string>"));
        assert!(body.contains("<string>docs</string>"));
        assert!(body.contains("<integer>600</integer>"));
        assert!(!body.contains("launchctl"));
    }

    #[test]
    fn install_writes_and_uninstall_removes_without_starting_anything() {
        let (root, _paths) = scratch("schedule-install");
        let dir = root.join("units");
        let spec = generate_schedule(
            ScheduleKind::Systemd,
            "docs",
            Path::new("/bin/synctr"),
            3600,
            None,
        )
        .unwrap();
        let written = install_schedule(&dir, &spec).unwrap();
        assert_eq!(written.len(), 2);
        assert!(dir.join("synctr-docs.service").is_file());
        assert!(dir.join("synctr-docs.timer").is_file());
        let removed = uninstall_schedule(&dir, ScheduleKind::Systemd, "docs").unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!dir.join("synctr-docs.service").exists());
        assert!(!dir.join("synctr-docs.timer").exists());
    }

    #[test]
    fn rejects_zero_interval_and_bad_kind() {
        assert!(matches!(
            generate_schedule(
                ScheduleKind::Systemd,
                "docs",
                Path::new("/bin/synctr"),
                0,
                None
            ),
            Err(Error::InvalidInterval)
        ));
        assert!(matches!(
            ScheduleKind::parse("cron"),
            Err(Error::InvalidScheduleKind(_))
        ));
    }

    #[test]
    fn xml_escapes_bin_path() {
        let spec = generate_schedule(
            ScheduleKind::Launchd,
            "docs",
            Path::new("/tmp/syn&ctr"),
            60,
            None,
        )
        .unwrap();
        assert!(spec.files[0].body.contains("/tmp/syn&amp;ctr"));
    }

    #[test]
    fn missing_home_is_an_error_not_root() {
        assert!(matches!(
            install_dir_from(ScheduleKind::Launchd, None, None),
            Err(Error::HomeNotFound)
        ));
        assert!(matches!(
            install_dir_from(ScheduleKind::Systemd, None, None),
            Err(Error::HomeNotFound)
        ));
        let systemd =
            install_dir_from(ScheduleKind::Systemd, Some(PathBuf::from("/xdg")), None).unwrap();
        assert_eq!(systemd, PathBuf::from("/xdg/systemd/user"));
        let launchd = install_dir_from(
            ScheduleKind::Launchd,
            None,
            Some(PathBuf::from("/Users/luxus")),
        )
        .unwrap();
        assert_eq!(launchd, PathBuf::from("/Users/luxus/Library/LaunchAgents"));
    }

    #[test]
    fn launchd_hint_uses_the_install_dir() {
        let hint = enable_hint(
            ScheduleKind::Launchd,
            Path::new("/tmp/agents"),
            "docs",
            false,
        );
        assert!(hint.contains("launchctl load /tmp/agents/dev.luxus.synctr.docs.plist"));
        assert!(!hint.contains("~/Library/LaunchAgents"));
        let systemd = enable_hint(
            ScheduleKind::Systemd,
            Path::new("/tmp/units"),
            "docs",
            false,
        );
        assert!(systemd.contains("systemctl --user enable --now synctr-docs.timer"));
        assert!(systemd.contains("/tmp/units"));
    }

    #[test]
    fn units_bake_config_dir_ahead_of_sync() {
        let spec = generate_schedule(
            ScheduleKind::Systemd,
            "docs",
            Path::new("/opt/synctr/bin/synctr"),
            60,
            Some(Path::new("/custom/synctr")),
        )
        .unwrap();
        assert!(spec.files[0]
            .body
            .contains("ExecStart=/opt/synctr/bin/synctr --config-dir /custom/synctr sync docs"));
        let spec = generate_schedule(
            ScheduleKind::Launchd,
            "docs",
            Path::new("/opt/synctr/bin/synctr"),
            60,
            Some(Path::new("/custom/synctr")),
        )
        .unwrap();
        let body = &spec.files[0].body;
        assert!(body.contains("<string>--config-dir</string>"));
        assert!(body.contains("<string>/custom/synctr</string>"));
        let sync_at = body.find("<string>sync</string>").unwrap();
        let cfg_at = body.find("<string>--config-dir</string>").unwrap();
        assert!(cfg_at < sync_at);
    }

    #[test]
    fn systemd_quotes_dollar_and_spaces_in_paths() {
        let spec = generate_schedule(
            ScheduleKind::Systemd,
            "docs",
            Path::new("/opt/My Synctr/bin/synctr"),
            60,
            Some(Path::new("/home/user/cfg$x")),
        )
        .unwrap();
        let body = &spec.files[0].body;
        assert!(
            body.contains(
                "ExecStart=\"/opt/My Synctr/bin/synctr\" --config-dir \"/home/user/cfg$$x\" sync docs"
            ),
            "got {body}"
        );
    }
}
