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
    let path = paths.lock_file(name);
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

/// Snapshot: true if another process holds the exclusive flock.
/// Briefly takes and drops the lock when idle; do not call while this
/// process already holds `ProfileLock` for the same name (would self-busy).
pub fn profile_lock_held(paths: &Paths, name: &str) -> Result<bool> {
    validate_name(name)?;
    let path = paths.lock_file(name);
    if !path.is_file() {
        return Ok(false);
    }
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    Ok(!try_exclusive(&file)?)
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
