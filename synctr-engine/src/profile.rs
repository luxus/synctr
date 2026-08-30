use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::{expand_tilde, Paths};

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
        let local = expand_tilde(&local);
        let rclone = rclone.map(|p| expand_tilde(&p));
        Ok(Self {
            name,
            local,
            remote,
            mode,
            rclone,
            extra_flags,
            extra_ignore,
        })
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
            out.push(load_file(&path)?);
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
}
