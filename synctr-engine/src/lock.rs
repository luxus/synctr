use std::fs::{File, OpenOptions};

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::profile::validate_name;

/// Exclusive per-profile lock. Released when dropped.
#[derive(Debug)]
pub struct ProfileLock {
    _file: File,
}

pub fn try_lock_profile(paths: &Paths, name: &str) -> Result<ProfileLock> {
    validate_name(name)?;
    let dir = paths.state_dir.join("locks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    if !try_exclusive(&file)? {
        return Err(Error::ProfileBusy(name.to_string()));
    }
    Ok(ProfileLock { _file: file })
}

/// True when another synctr process holds the exclusive per-profile lock.
/// Opens a separate fd so the holder is not interrupted.
pub fn profile_is_busy(paths: &Paths, name: &str) -> bool {
    if validate_name(name).is_err() {
        return false;
    }
    let path = paths.state_dir.join("locks").join(format!("{name}.lock"));
    let Ok(file) = File::open(path) else {
        return false;
    };
    matches!(try_exclusive(&file), Ok(false))
}

#[cfg(unix)]
fn try_exclusive(file: &File) -> Result<bool> {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(true)
    } else {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            Ok(false)
        } else {
            Err(err.into())
        }
    }
}

#[cfg(not(unix))]
fn try_exclusive(_file: &File) -> Result<bool> {
    Ok(true)
}
