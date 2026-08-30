use std::path::Path;
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
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    if rel.as_os_str().is_empty() {
        return true;
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    !filters.is_excluded(&s)
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
        assert!(path_should_wake(
            &filters,
            &root,
            &root.join("src/lib.rs")
        ));
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
}
