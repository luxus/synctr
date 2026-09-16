use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{make_absolute, Paths};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Copy,
    Sync,
    Bisync,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "copy" => Ok(Self::Copy),
            "sync" => Ok(Self::Sync),
            "bisync" => Ok(Self::Bisync),
            other => Err(Error::InvalidMode(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Sync => "sync",
            Self::Bisync => "bisync",
        }
    }

    pub fn rclone_subcommand(self) -> &'static str {
        self.as_str()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub local: PathBuf,
    pub remote: String,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rclone: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_ignore: Vec<String>,
    /// Omitted when true so existing tomls and idle `status --json` stay unchanged.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileEdit {
    pub local: Option<PathBuf>,
    pub remote: Option<String>,
    pub mode: Option<Mode>,
    pub rclone: Option<Option<PathBuf>>,
    pub extra_flags: Option<Vec<String>>,
    pub extra_ignore: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

impl ProfileEdit {
    pub fn is_empty(&self) -> bool {
        self.local.is_none()
            && self.remote.is_none()
            && self.mode.is_none()
            && self.rclone.is_none()
            && self.extra_flags.is_none()
            && self.extra_ignore.is_none()
            && self.enabled.is_none()
    }
}

impl Profile {
    pub fn new(
        name: String,
        local: PathBuf,
        remote: String,
        mode: Mode,
        rclone: Option<PathBuf>,
        extra_flags: Vec<String>,
        extra_ignore: Vec<String>,
    ) -> Result<Self> {
        validate_name(&name)?;
        if !remote.contains(':') {
            return Err(Error::InvalidRemote(remote));
        }
        let local = make_absolute(&local);
        let rclone = rclone.map(|p| make_absolute(&p));
        Ok(Self {
            name,
            local,
            remote,
            mode,
            rclone,
            extra_flags,
            extra_ignore,
            enabled: true,
        })
    }

    pub fn require_enabled(&self) -> Result<()> {
        if self.enabled {
            Ok(())
        } else {
            Err(Error::ProfileDisabled(self.name.clone()))
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    paths: Paths,
}

impl ProfileStore {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn add(&self, profile: &Profile) -> Result<()> {
        validate_name(&profile.name)?;
        fs::create_dir_all(&self.paths.profiles_dir)?;
        let dest = self.paths.profile_toml(&profile.name);
        if dest.exists() {
            return Err(Error::ProfileExists(profile.name.clone()));
        }
        fs::write(&dest, toml::to_string_pretty(profile)?)?;
        let ignore = self.paths.profile_ignore(&profile.name);
        if !ignore.exists() {
            fs::write(
                ignore,
                "# gitignore-style patterns, one per line. Translated to rclone --filter-from.\n",
            )?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        let mut out = Vec::new();
        if !self.paths.profiles_dir.is_dir() {
            return Ok(out);
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(&self.paths.profiles_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
            .collect();
        entries.sort();
        for path in entries {
            match load_file(&path) {
                Ok(profile) => out.push(profile),
                Err(_) => continue,
            }
        }
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Result<Profile> {
        validate_name(name)?;
        let path = self.paths.profile_toml(name);
        if !path.is_file() {
            return Err(Error::ProfileNotFound(name.to_string()));
        }
        load_file(&path)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let path = self.paths.profile_toml(name);
        if !path.is_file() {
            return Err(Error::ProfileNotFound(name.to_string()));
        }
        fs::remove_file(path)?;
        let _ = fs::remove_file(self.paths.profile_ignore(name));
        let _ = fs::remove_file(self.paths.last_run(name));
        Ok(())
    }

    pub fn edit(&self, name: &str, edit: ProfileEdit) -> Result<Profile> {
        if edit.is_empty() {
            return Err(Error::EmptyEdit);
        }
        let mut profile = self.get(name)?;
        if let Some(local) = edit.local {
            profile.local = make_absolute(&local);
        }
        if let Some(remote) = edit.remote {
            if !remote.contains(':') {
                return Err(Error::InvalidRemote(remote));
            }
            profile.remote = remote;
        }
        if let Some(mode) = edit.mode {
            profile.mode = mode;
        }
        if let Some(rclone) = edit.rclone {
            profile.rclone = rclone.map(|p| make_absolute(&p));
        }
        if let Some(extra_flags) = edit.extra_flags {
            profile.extra_flags = extra_flags;
        }
        if let Some(extra_ignore) = edit.extra_ignore {
            profile.extra_ignore = extra_ignore;
        }
        if let Some(enabled) = edit.enabled {
            profile.enabled = enabled;
        }
        let dest = self.paths.profile_toml(name);
        fs::write(dest, toml::to_string_pretty(&profile)?)?;
        Ok(profile)
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<Profile> {
        self.edit(
            name,
            ProfileEdit {
                enabled: Some(enabled),
                ..ProfileEdit::default()
            },
        )
    }

    pub fn rename(&self, old: &str, new: &str) -> Result<Profile> {
        validate_name(old)?;
        validate_name(new)?;
        if old == new {
            return self.get(old);
        }
        let src = self.paths.profile_toml(old);
        if !src.is_file() {
            return Err(Error::ProfileNotFound(old.to_string()));
        }
        let dest = self.paths.profile_toml(new);
        if dest.exists() {
            return Err(Error::ProfileExists(new.to_string()));
        }
        let mut profile = self.get(old)?;
        profile.name = new.to_string();
        fs::rename(&src, &dest)?;
        fs::write(&dest, toml::to_string_pretty(&profile)?)?;
        let old_ignore = self.paths.profile_ignore(old);
        if old_ignore.exists() {
            fs::rename(old_ignore, self.paths.profile_ignore(new))?;
        }
        let old_run = self.paths.last_run(old);
        if old_run.exists() {
            let new_run = self.paths.last_run(new);
            if let Some(parent) = new_run.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(old_run, new_run)?;
        }
        Ok(profile)
    }
}

fn load_file(path: &Path) -> Result<Profile> {
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(Error::InvalidName(name.to_string()));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(Error::InvalidName(name.to_string()));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(Error::InvalidName(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scratch;

    #[test]
    fn roundtrip_add_list_show_remove() {
        let (_root, paths) = scratch("profile-roundtrip");
        let store = ProfileStore::new(paths);
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec!["--checksum".into()],
            vec!["*.key".into()],
        )
        .unwrap();
        store.add(&p).unwrap();
        assert!(matches!(store.add(&p), Err(Error::ProfileExists(_))));
        let listed = store.list().unwrap();
        assert_eq!(listed, vec![p.clone()]);
        assert_eq!(store.get("docs").unwrap(), p);
        store.remove("docs").unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(store.get("docs"), Err(Error::ProfileNotFound(_))));
    }

    #[test]
    fn edit_rewrites_same_toml_and_keeps_last_run() {
        let (_root, paths) = scratch("profile-edit");
        let store = ProfileStore::new(paths.clone());
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec!["--checksum".into()],
            vec!["*.key".into()],
        )
        .unwrap();
        store.add(&p).unwrap();
        crate::status::write_last_run(&paths, "docs", crate::status::LastRun::now(0)).unwrap();
        let edited = store
            .edit(
                "docs",
                ProfileEdit {
                    remote: Some("b2:bucket/other".into()),
                    mode: Some(Mode::Copy),
                    extra_flags: Some(vec!["--fast-list".into()]),
                    extra_ignore: Some(vec!["*.tmp".into()]),
                    ..ProfileEdit::default()
                },
            )
            .unwrap();
        assert_eq!(edited.remote, "b2:bucket/other");
        assert_eq!(edited.mode, Mode::Copy);
        assert_eq!(edited.extra_flags, vec!["--fast-list"]);
        assert_eq!(store.get("docs").unwrap(), edited);
        assert!(paths.profile_toml("docs").is_file());
        assert!(crate::status::read_last_run(&paths, "docs")
            .unwrap()
            .is_some());
        assert!(matches!(
            store.edit("docs", ProfileEdit::default()),
            Err(Error::EmptyEdit)
        ));
    }

    #[test]
    fn rename_moves_toml_ignore_and_last_run() {
        let (_root, paths) = scratch("profile-rename");
        let store = ProfileStore::new(paths.clone());
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&p).unwrap();
        crate::status::write_last_run(&paths, "docs", crate::status::LastRun::now(0)).unwrap();
        fs::write(paths.profile_ignore("docs"), "*.tmp\n").unwrap();
        let renamed = store.rename("docs", "notes").unwrap();
        assert_eq!(renamed.name, "notes");
        assert!(matches!(store.get("docs"), Err(Error::ProfileNotFound(_))));
        assert_eq!(store.get("notes").unwrap().remote, "b2:bucket/docs");
        assert!(!paths.profile_toml("docs").exists());
        assert!(!paths.profile_ignore("docs").exists());
        assert!(!paths.last_run("docs").exists());
        assert_eq!(
            fs::read_to_string(paths.profile_ignore("notes")).unwrap(),
            "*.tmp\n"
        );
        assert!(crate::status::read_last_run(&paths, "notes")
            .unwrap()
            .is_some());
    }

    #[test]
    fn rejects_remote_without_colon() {
        let err = Profile::new(
            "x".into(),
            PathBuf::from("/tmp"),
            "not-a-remote".into(),
            Mode::Copy,
            None,
            vec![],
            vec![],
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidRemote(_)));
    }

    #[test]
    fn relative_local_and_rclone_are_stored_absolute() {
        let cwd = std::env::current_dir().unwrap();
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            Some(PathBuf::from("bin/rclone")),
            vec![],
            vec![],
        )
        .unwrap();
        assert_eq!(p.local, cwd.join("docs"));
        assert_eq!(p.rclone.as_deref(), Some(cwd.join("bin/rclone").as_path()));
        assert!(p.local.is_absolute());
    }

    #[test]
    fn list_skips_unreadable_toml_so_other_profiles_remain() {
        let (_root, paths) = scratch("profile-skip-bad");
        let store = ProfileStore::new(paths.clone());
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&p).unwrap();
        fs::write(paths.profile_toml("zzz-bad"), "this is not toml {{{").unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "docs");
    }

    #[test]
    fn enabled_defaults_true_and_disable_roundtrips() {
        let (_root, paths) = scratch("profile-enabled");
        let store = ProfileStore::new(paths.clone());
        let p = Profile::new(
            "docs".into(),
            PathBuf::from("/tmp/docs"),
            "b2:bucket/docs".into(),
            Mode::Sync,
            None,
            vec![],
            vec![],
        )
        .unwrap();
        store.add(&p).unwrap();
        assert!(store.get("docs").unwrap().enabled);
        let text = fs::read_to_string(paths.profile_toml("docs")).unwrap();
        assert!(
            !text.contains("enabled"),
            "enabled=true should be omitted from toml: {text}"
        );
        fs::write(
            paths.profile_toml("legacy"),
            "name = \"legacy\"\nlocal = \"/tmp/x\"\nremote = \"b2:x\"\nmode = \"copy\"\n",
        )
        .unwrap();
        assert!(store.get("legacy").unwrap().enabled);
        let off = store.set_enabled("docs", false).unwrap();
        assert!(!off.enabled);
        assert!(!store.get("docs").unwrap().enabled);
        off.require_enabled().unwrap_err();
        store.set_enabled("docs", true).unwrap();
        assert!(store.get("docs").unwrap().enabled);
    }
}
