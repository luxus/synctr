use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("profile `{0}` not found")]
    ProfileNotFound(String),
    #[error("profile `{0}` already exists")]
    ProfileExists(String),
    #[error("invalid profile name `{0}` (use letters, digits, `_`, `-`)")]
    InvalidName(String),
    #[error("invalid remote `{0}` (want remote:path)")]
    InvalidRemote(String),
    #[error("unknown mode `{0}` (want copy, sync, or bisync)")]
    InvalidMode(String),
    #[error("rclone not found (flag, SYNCTR_RCLONE, PATH, nix-darwin, homebrew)")]
    RcloneNotFound,
    #[error("rclone override not found ({origin}): {path}")]
    RcloneOverrideMissing { origin: String, path: PathBuf },
    #[error("nothing to change")]
    EmptyEdit,
    #[error("unknown schedule kind `{0}` (want systemd or launchd)")]
    InvalidScheduleKind(String),
    #[error("interval must be greater than zero seconds")]
    InvalidInterval,
    #[error("home directory not found (set HOME or XDG_CONFIG_HOME, or pass --dir)")]
    HomeNotFound,
    #[error("profile `{0}` is already running")]
    ProfileBusy(String),
    #[error("profile `{0}` is disabled (synctr profile enable {0})")]
    ProfileDisabled(String),
    #[error("`--resync` is only for bisync profiles (profile `{0}` is {1})")]
    ResyncNotBisync(String, &'static str),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("toml: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
