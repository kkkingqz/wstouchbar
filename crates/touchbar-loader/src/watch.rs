use crate::LoadRequest;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};
struct WatchRecord {
    descriptor: WatchDescriptor,
    device: u64,
    inode: u64,
}
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Nonblocking watcher. The caller supplies monotonic elapsed time and schedules a poll at deadline().
pub struct ReloadWatcher {
    inotify: Inotify,
    request: LoadRequest,
    watches: BTreeMap<PathBuf, WatchRecord>,
    pending: Option<Duration>,
}
impl ReloadWatcher {
    pub fn new(request: &LoadRequest) -> Result<Self, String> {
        let mut this = Self {
            inotify: Inotify::init().map_err(|e| e.to_string())?,
            request: request.clone(),
            watches: BTreeMap::new(),
            pending: None,
        };
        this.refresh()?;
        Ok(this)
    }
    pub fn deadline(&self) -> Option<Duration> {
        self.pending.map(|t| t.saturating_add(DEBOUNCE))
    }
    pub fn poll(&mut self, now: Duration) -> Result<bool, String> {
        let mut buffer = [0; 64 * 1024];
        let mut changed = false;
        loop {
            match self.inotify.read_events(&mut buffer) {
                Ok(events) => {
                    let mut count = 0;
                    for event in events {
                        count += 1;
                        if event.mask.contains(EventMask::IGNORED) {
                            self.watches.retain(|_, wd| wd.descriptor != event.wd);
                        } else {
                            changed = true;
                        }
                    }
                    if count == 0 {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if changed {
            self.pending = Some(now);
            self.refresh()?;
        }
        if self.deadline().is_some_and(|t| now >= t) {
            self.pending = None;
            return Ok(true);
        }
        Ok(false)
    }
    fn add(&mut self, path: &Path) -> Result<(), String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if self
            .watches
            .get(path)
            .is_some_and(|wd| wd.device == metadata.dev() && wd.inode == metadata.ino())
        {
            return Ok(());
        }
        if let Some(old) = self.watches.remove(path) {
            let _ = self.inotify.watches().remove(old.descriptor);
        }
        if self.watches.len() >= 4096 {
            return Err("plugin watcher exceeds 4096 directories".into());
        }
        let wd = self
            .inotify
            .watches()
            .add(
                path,
                WatchMask::CLOSE_WRITE
                    | WatchMask::CREATE
                    | WatchMask::MOVED_TO
                    | WatchMask::MOVED_FROM
                    | WatchMask::DELETE
                    | WatchMask::DELETE_SELF
                    | WatchMask::MOVE_SELF
                    | WatchMask::ATTRIB,
            )
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.watches.insert(
            path.to_owned(),
            WatchRecord {
                descriptor: wd,
                device: metadata.dev(),
                inode: metadata.ino(),
            },
        );
        Ok(())
    }
    fn refresh(&mut self) -> Result<(), String> {
        let mut wanted = BTreeSet::new();
        for file in [&self.request.config_file, &self.request.bindings_file] {
            let mut parent = file.parent().unwrap_or(Path::new("."));
            while !parent.is_dir() {
                parent = parent.parent().ok_or("no existing config parent")?;
            }
            wanted.insert(parent.canonicalize().map_err(|e| e.to_string())?);
        }
        for root in &self.request.paths {
            let mut ancestor = root.as_path();
            while !ancestor.is_dir() {
                ancestor = ancestor.parent().ok_or("no existing plugin parent")?;
            }
            wanted.insert(ancestor.canonicalize().map_err(|e| e.to_string())?);
            let mut stack = if root.is_dir() {
                vec![root.clone()]
            } else {
                vec![]
            };
            let mut visited = BTreeSet::new();
            while let Some(path) = stack.pop() {
                let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                if !visited.insert(canonical.clone()) {
                    continue;
                }
                wanted.insert(canonical.clone());
                if wanted.len() > 4096 {
                    return Err("plugin watcher exceeds 4096 directories".into());
                }
                for entry in std::fs::read_dir(&canonical).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.path().is_dir() {
                        stack.push(entry.path());
                    }
                }
            }
        }
        // Canonical names track symlink targets; obsolete watches are removed on directory replacement.
        let stale: Vec<_> = self
            .watches
            .keys()
            .filter(|p| !wanted.contains(*p))
            .cloned()
            .collect();
        for path in stale {
            if let Some(wd) = self.watches.remove(&path) {
                let _ = self.inotify.watches().remove(wd.descriptor);
            }
        }
        for path in wanted {
            self.add(&path)?;
        }
        Ok(())
    }
}

impl std::os::fd::AsFd for ReloadWatcher {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}
