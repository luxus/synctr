use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolveSource {
    Flag,
    Profile,
    Env,
    Path,
    NixDarwinUser,
    NixSystem,
    HomebrewOpt,
    HomebrewUsrLocal,
}

impl ResolveSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Profile => "profile",
            Self::Env => "env",
            Self::Path => "PATH",
            Self::NixDarwinUser => "nix-darwin-user",
            Self::NixSystem => "nix-system",
            Self::HomebrewOpt => "homebrew-opt",
            Self::HomebrewUsrLocal => "homebrew-usr-local",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            Self::Flag => "--rclone",
            Self::Profile => "profile rclone field",
            Self::Env => "SYNCTR_RCLONE",
            Self::Path => "PATH",
            Self::NixDarwinUser => "nix-darwin /etc/profiles/per-user/$USER/bin",
            Self::NixSystem => "nix /run/current-system/sw/bin",
            Self::HomebrewOpt => "homebrew /opt/homebrew/bin",
            Self::HomebrewUsrLocal => "homebrew /usr/local/bin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRclone {
    pub path: PathBuf,
    pub source: ResolveSource,
}

#[derive(Debug, Clone)]
pub struct ResolveRequest<'a> {
    pub flag: Option<&'a Path>,
    pub profile: Option<&'a Path>,
    pub env: Option<&'a Path>,
    pub path_var: Option<&'a OsStr>,
    pub user: &'a str,
}

impl Default for ResolveRequest<'_> {
    fn default() -> Self {
        Self {
            flag: None,
            profile: None,
            env: None,
            path_var: None,
            user: "",
        }
    }
}

pub trait FsProbe {
    fn is_executable(&self, path: &Path) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealFs;

impl FsProbe for RealFs {
    fn is_executable(&self, path: &Path) -> bool {
        is_executable(path)
    }
}

pub fn resolve_rclone(req: &ResolveRequest<'_>, fs: &dyn FsProbe) -> Result<ResolvedRclone> {
    if let Some(path) = req.flag {
        return require(path, ResolveSource::Flag, fs);
    }
    if let Some(path) = req.profile {
        return require(path, ResolveSource::Profile, fs);
    }
    if let Some(path) = req.env {
        return require(path, ResolveSource::Env, fs);
    }
    if let Some(path_var) = req.path_var {
        for dir in env::split_paths(path_var) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let candidate = dir.join("rclone");
            if fs.is_executable(&candidate) {
                return Ok(ResolvedRclone {
                    path: candidate,
                    source: ResolveSource::Path,
                });
            }
        }
    }
    for (candidate, source) in well_known(req.user) {
        if fs.is_executable(&candidate) {
            return Ok(ResolvedRclone {
                path: candidate,
                source,
            });
        }
    }
    Err(Error::RcloneNotFound)
}

pub fn resolve_rclone_live(
    flag: Option<&Path>,
    profile: Option<&Path>,
) -> Result<ResolvedRclone> {
    let env_buf = env::var_os("SYNCTR_RCLONE").map(PathBuf::from);
    let path_var = env::var_os("PATH");
    let user = env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_default();
    let req = ResolveRequest {
        flag,
        profile,
        env: env_buf.as_deref(),
        path_var: path_var.as_deref(),
        user: &user,
    };
    resolve_rclone(&req, &RealFs)
}

pub fn well_known(user: &str) -> Vec<(PathBuf, ResolveSource)> {
    vec![
        (
            PathBuf::from(format!("/etc/profiles/per-user/{user}/bin/rclone")),
            ResolveSource::NixDarwinUser,
        ),
        (
            PathBuf::from("/run/current-system/sw/bin/rclone"),
            ResolveSource::NixSystem,
        ),
        (
            PathBuf::from("/opt/homebrew/bin/rclone"),
            ResolveSource::HomebrewOpt,
        ),
        (
            PathBuf::from("/usr/local/bin/rclone"),
            ResolveSource::HomebrewUsrLocal,
        ),
    ]
}

fn require(path: &Path, source: ResolveSource, fs: &dyn FsProbe) -> Result<ResolvedRclone> {
    if fs.is_executable(path) {
        Ok(ResolvedRclone {
            path: path.to_path_buf(),
            source,
        })
    } else {
        Err(Error::RcloneOverrideMissing {
            origin: source.as_str().to_string(),
            path: path.to_path_buf(),
        })
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::ffi::OsString;

    struct MapFs(HashSet<PathBuf>);

    impl FsProbe for MapFs {
        fn is_executable(&self, path: &Path) -> bool {
            self.files().contains(path)
        }
    }

    impl MapFs {
        fn files(&self) -> &HashSet<PathBuf> {
            &self.0
        }
    }

    fn req<'a>(path_var: Option<&'a OsStr>, user: &'a str) -> ResolveRequest<'a> {
        ResolveRequest {
            flag: None,
            profile: None,
            env: None,
            path_var,
            user,
        }
    }

    #[test]
    fn flag_wins_over_path_and_well_known() {
        let flag = PathBuf::from("/custom/rclone");
        let fs = MapFs(HashSet::from([
            flag.clone(),
            PathBuf::from("/usr/bin/rclone"),
            PathBuf::from("/opt/homebrew/bin/rclone"),
        ]));
        let path = OsString::from("/usr/bin");
        let found = resolve_rclone(
            &ResolveRequest {
                flag: Some(&flag),
                path_var: Some(&path),
                user: "luxus",
                ..Default::default()
            },
            &fs,
        )
        .unwrap();
        assert_eq!(found.path, flag);
        assert_eq!(found.source, ResolveSource::Flag);
    }

    #[test]
    fn profile_then_env_then_path() {
        let profile = PathBuf::from("/profile/rclone");
        let envp = PathBuf::from("/env/rclone");
        let path_hit = PathBuf::from("/usr/bin/rclone");
        let fs = MapFs(HashSet::from([
            profile.clone(),
            envp.clone(),
            path_hit.clone(),
        ]));
        let path = OsString::from("/usr/bin");
        let found = resolve_rclone(
            &ResolveRequest {
                profile: Some(&profile),
                env: Some(&envp),
                path_var: Some(&path),
                user: "luxus",
                ..Default::default()
            },
            &fs,
        )
        .unwrap();
        assert_eq!(found.source, ResolveSource::Profile);

        let found = resolve_rclone(
            &ResolveRequest {
                env: Some(&envp),
                path_var: Some(&path),
                user: "luxus",
                ..Default::default()
            },
            &fs,
        )
        .unwrap();
        assert_eq!(found.source, ResolveSource::Env);

        let found = resolve_rclone(&req(Some(&path), "luxus"), &fs).unwrap();
        assert_eq!(found.path, path_hit);
        assert_eq!(found.source, ResolveSource::Path);
    }

    #[test]
    fn empty_path_still_finds_nix_darwin() {
        let user = "luxus";
        let nix = PathBuf::from(format!("/etc/profiles/per-user/{user}/bin/rclone"));
        let fs = MapFs(HashSet::from([
            nix.clone(),
            PathBuf::from("/opt/homebrew/bin/rclone"),
        ]));
        let empty = OsString::from("");
        let found = resolve_rclone(&req(Some(&empty), user), &fs).unwrap();
        assert_eq!(found.path, nix);
        assert_eq!(found.source, ResolveSource::NixDarwinUser);
    }

    #[test]
    fn nix_system_before_homebrew_when_path_misses() {
        let fs = MapFs(HashSet::from([
            PathBuf::from("/run/current-system/sw/bin/rclone"),
            PathBuf::from("/opt/homebrew/bin/rclone"),
            PathBuf::from("/usr/local/bin/rclone"),
        ]));
        let path = OsString::from("/usr/bin:/bin");
        let found = resolve_rclone(&req(Some(&path), "luxus"), &fs).unwrap();
        assert_eq!(found.source, ResolveSource::NixSystem);
        assert_eq!(
            found.path,
            PathBuf::from("/run/current-system/sw/bin/rclone")
        );
    }

    #[test]
    fn homebrew_opt_then_usr_local() {
        let fs = MapFs(HashSet::from([
            PathBuf::from("/opt/homebrew/bin/rclone"),
            PathBuf::from("/usr/local/bin/rclone"),
        ]));
        let found = resolve_rclone(&req(None, "luxus"), &fs).unwrap();
        assert_eq!(found.source, ResolveSource::HomebrewOpt);

        let fs = MapFs(HashSet::from([PathBuf::from("/usr/local/bin/rclone")]));
        let found = resolve_rclone(&req(None, "luxus"), &fs).unwrap();
        assert_eq!(found.source, ResolveSource::HomebrewUsrLocal);
    }

    #[test]
    fn missing_override_does_not_fall_through() {
        let flag = PathBuf::from("/nope/rclone");
        let fs = MapFs(HashSet::from([PathBuf::from("/usr/bin/rclone")]));
        let path = OsString::from("/usr/bin");
        let err = resolve_rclone(
            &ResolveRequest {
                flag: Some(&flag),
                path_var: Some(&path),
                user: "luxus",
                ..Default::default()
            },
            &fs,
        )
        .unwrap_err();
        assert!(matches!(err, Error::RcloneOverrideMissing { .. }));
    }

    #[test]
    fn not_found_when_nothing_exists() {
        let fs = MapFs(HashSet::new());
        let path = OsString::from("/usr/bin:/bin");
        let err = resolve_rclone(&req(Some(&path), "luxus"), &fs).unwrap_err();
        assert!(matches!(err, Error::RcloneNotFound));
    }
}
