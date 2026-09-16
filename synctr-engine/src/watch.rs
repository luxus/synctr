use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::ignore::FilterSet;

#[derive(Debug, Clone)]
pub struct Debouncer {
    wait: Duration,
    deadline: Option<Instant>,
}

impl Debouncer {
    pub fn new(wait: Duration) -> Self {
        Self {
            wait,
            deadline: None,
        }
    }

    pub fn poke(&mut self, now: Instant) {
        self.deadline = Some(now + self.wait);
    }

    pub fn take_ready(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(d) if now >= d => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }

    pub fn pending(&self) -> bool {
        self.deadline.is_some()
    }
}

pub fn path_should_wake(filters: &FilterSet, root: &Path, path: &Path) -> bool {
    let Some(s) = relative_to_root(root, path) else {
        // Paths we cannot place under the watched root are not a local change.
        // Treating them as a wake (the old behavior) false-triggers on symlink
        // canonicalization (macOS /tmp → /private/tmp) and on notify events
        // whose path is outside the tree.
        return false;
    };
    if s.is_empty() {
        return true;
    }
    if filters.is_excluded(&s) {
        return false;
    }
    // notify/FSEvents often report the directory itself (no trailing slash)
    // when something inside it changes. `node_modules/` gitignore rules do
    // not match the bare name `node_modules`.
    if !s.ends_with('/') && filters.is_excluded(&format!("{s}/")) {
        return false;
    }
    true
}

fn relative_to_root(root: &Path, path: &Path) -> Option<String> {
    if let Some(s) = strip_to_rel(root, path) {
        return Some(s);
    }
    let root_c = root.canonicalize().ok()?;
    let path_c = canonicalize_maybe_deleted(path)?;
    strip_to_rel(&root_c, &path_c)
}

fn strip_to_rel(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
}

fn canonicalize_maybe_deleted(path: &Path) -> Option<PathBuf> {
    if let Ok(c) = path.canonicalize() {
        return Some(c);
    }
    let name = path.file_name()?;
    let parent = path.parent()?.canonicalize().ok()?;
    Some(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ignore::FilterSet;
    use std::path::PathBuf;

    #[test]
    fn debounce_fires_once_after_quiet_period() {
        let mut d = Debouncer::new(Duration::from_millis(40));
        let t0 = Instant::now();
        d.poke(t0);
        assert!(!d.take_ready(t0));
        d.poke(t0 + Duration::from_millis(20));
        assert!(!d.take_ready(t0 + Duration::from_millis(30)));
        assert!(d.take_ready(t0 + Duration::from_millis(70)));
        assert!(!d.take_ready(t0 + Duration::from_millis(70)));
        assert!(!d.pending());
    }

    #[test]
    fn ignored_paths_do_not_wake_watch() {
        let filters = FilterSet::from_gitignore_lines([]).with_defaults();
        let root = PathBuf::from("/tmp/docs");
        assert!(path_should_wake(&filters, &root, &root.join("src/lib.rs")));
        assert!(!path_should_wake(
            &filters,
            &root,
            &root.join("node_modules/left-pad/index.js")
        ));
        assert!(!path_should_wake(
            &filters,
            &root,
            &root.join("target/debug/synctr")
        ));
        assert!(path_should_wake(&filters, &root, &root));
    }

    #[test]
    fn ignored_directory_itself_does_not_wake() {
        let filters = FilterSet::from_gitignore_lines([]).with_defaults();
        let root = PathBuf::from("/tmp/docs");
        assert!(
            !path_should_wake(&filters, &root, &root.join("node_modules")),
            "notify reports the ignored directory with no trailing slash"
        );
        assert!(!path_should_wake(&filters, &root, &root.join(".git")));
        assert!(!path_should_wake(&filters, &root, &root.join("target")));
        assert!(!path_should_wake(&filters, &root, &root.join("dist")));
        assert!(path_should_wake(&filters, &root, &root.join("src")));
        assert!(path_should_wake(&filters, &root, &root.join("notes.md")));
    }

    #[test]
    fn path_outside_watched_root_does_not_wake() {
        let filters = FilterSet::from_gitignore_lines([]).with_defaults();
        let root = PathBuf::from("/tmp/docs");
        assert!(!path_should_wake(
            &filters,
            &root,
            Path::new("/tmp/other/file.txt")
        ));
        assert!(!path_should_wake(
            &filters,
            &root,
            Path::new("/var/log/syslog")
        ));
    }

    #[test]
    fn symlink_root_still_honors_ignores_on_real_path() {
        let (tmp, _) = crate::testutil::scratch("watch-symlink");
        let real = tmp.join("real");
        let link = tmp.join("link");
        std::fs::create_dir_all(real.join("node_modules/pkg")).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::write(real.join("readme.md"), "ok\n").unwrap();
        std::fs::write(real.join("node_modules/pkg/index.js"), "x\n").unwrap();

        let filters = FilterSet::from_gitignore_lines([]).with_defaults();
        assert!(
            !path_should_wake(&filters, &link, &real.join("node_modules/pkg/index.js")),
            "event on the canonical path of an ignored file must not wake"
        );
        assert!(path_should_wake(&filters, &link, &real.join("readme.md")));
        assert!(
            !path_should_wake(&filters, &link, &real.join("node_modules")),
            "canonical path of an ignored directory must not wake"
        );
    }
}
