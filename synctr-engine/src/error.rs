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
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("toml: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
