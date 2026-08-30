use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub ignore_file: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let config_dir = env_dir("SYNCTR_CONFIG_DIR")
            .or_else(|| env_dir("XDG_CONFIG_HOME").map(|p| p.join("synctr")))
            .unwrap_or_else(|| home().join(".config/synctr"));
        let state_dir = env_dir("SYNCTR_STATE_DIR")
            .or_else(|| env_dir("XDG_STATE_HOME").map(|p| p.join("synctr")))
            .unwrap_or_else(|| home().join(".local/state/synctr"));
        let cache_dir = env_dir("SYNCTR_CACHE_DIR")
            .or_else(|| env_dir("XDG_CACHE_HOME").map(|p| p.join("synctr")))
            .unwrap_or_else(|| home().join(".cache/synctr"));
        Self::assemble(config_dir, state_dir, cache_dir)
    }

    pub fn from_config_dir(config_dir: PathBuf) -> Self {
        let state_dir = env_dir("SYNCTR_STATE_DIR").unwrap_or_else(|| config_dir.join("state"));
        let cache_dir = env_dir("SYNCTR_CACHE_DIR").unwrap_or_else(|| config_dir.join("cache"));
        Self::assemble(config_dir, state_dir, cache_dir)
    }

    fn assemble(config_dir: PathBuf, state_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            profiles_dir: config_dir.join("profiles"),
            ignore_file: config_dir.join("ignore"),
            config_dir,
            state_dir,
            cache_dir,
        }
    }

    pub fn profile_toml(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{name}.toml"))
    }

    pub fn profile_ignore(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(format!("{name}.ignore"))
    }

    pub fn last_run(&self, name: &str) -> PathBuf {
        self.state_dir.join("runs").join(format!("{name}.toml"))
    }

    pub fn filter_file(&self, name: &str) -> PathBuf {
        self.cache_dir.join("filters").join(format!("{name}.filter"))
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn env_dir(key: &str) -> Option<PathBuf> {
    env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        return home();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home().join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_dir_nests_state_and_cache() {
        let p = Paths::from_config_dir(PathBuf::from("/tmp/synctr-cfg"));
        assert_eq!(p.profiles_dir, PathBuf::from("/tmp/synctr-cfg/profiles"));
        assert_eq!(p.ignore_file, PathBuf::from("/tmp/synctr-cfg/ignore"));
        assert_eq!(p.profile_toml("docs"), PathBuf::from("/tmp/synctr-cfg/profiles/docs.toml"));
        assert_eq!(p.profile_ignore("docs"), PathBuf::from("/tmp/synctr-cfg/profiles/docs.ignore"));
    }
}
