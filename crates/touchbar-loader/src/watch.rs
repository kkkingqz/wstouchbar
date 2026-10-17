use crate::LoadRequest;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};
struct WatchRecord {
    descriptor: WatchDescriptor,
    device: u64,
    inode: u64,
    interest: Interest,
}
#[derive(Default)]
struct Interest {
    tree: bool,
    names: BTreeSet<OsString>,
}
const DEBOUNCE: Duration = Duration::from_millis(200);
const RETRY: Duration = Duration::from_secs(1);
const MAX_DIRECTORIES: usize = 4096;
/// Nonblocking watcher. Only transport failures are fatal; tree maintenance is
/// best-effort with bounded diagnostics and a monotonic retry deadline.
pub struct ReloadWatcher {
    inotify: Inotify,
    request: LoadRequest,
    watches: BTreeMap<PathBuf, WatchRecord>,
    pending: Option<Duration>,
    retry: Option<Duration>,
    issues: Vec<String>,
}
impl ReloadWatcher {
    pub fn new(request: &LoadRequest) -> Result<Self, String> {
        let mut this = Self {
            inotify: Inotify::init().map_err(|e| e.to_string())?,
            request: request.clone(),
            watches: BTreeMap::new(),
            pending: None,
            retry: None,
            issues: Vec::new(),
        };
        if !this.refresh() {
            this.retry = Some(RETRY);
        }
        Ok(this)
    }
    pub fn deadline(&self) -> Option<Duration> {
        self.pending
            .map(|t| t.saturating_add(DEBOUNCE))
            .into_iter()
            .chain(self.retry)
            .min()
    }
    pub fn take_issues(&mut self) -> Vec<String> {
        std::mem::take(&mut self.issues)
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
                        } else if event.mask.contains(EventMask::Q_OVERFLOW)
                            || self.watches.values().any(|watch| {
                                watch.descriptor == event.wd
                                    && (watch.interest.tree
                                        || event.name.is_none()
                                        || event.name.is_some_and(|name| {
                                            watch.interest.names.contains(name)
                                        }))
                            })
                        {
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
        let retry_due = self.retry.is_some_and(|t| now >= t);
        if changed || retry_due {
            let complete = self.refresh();
            self.retry = (!complete).then(|| now.saturating_add(RETRY));
            if changed || (retry_due && complete) {
                self.pending = Some(now);
            }
        }
        if self
            .pending
            .is_some_and(|t| now >= t.saturating_add(DEBOUNCE))
        {
            self.pending = None;
            return Ok(true);
        }
        Ok(false)
    }
    fn add(&mut self, path: &Path, interest: Interest) -> Result<(), String> {
        let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if self
            .watches
            .get(path)
            .is_some_and(|wd| wd.device == metadata.dev() && wd.inode == metadata.ino())
        {
            self.watches.get_mut(path).unwrap().interest = interest;
            return Ok(());
        }
        if let Some(old) = self.watches.remove(path) {
            let _ = self.inotify.watches().remove(old.descriptor);
        }
        if self.watches.len() >= MAX_DIRECTORIES {
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
                interest,
            },
        );
        Ok(())
    }
    fn refresh(&mut self) -> bool {
        let mut wanted: BTreeMap<PathBuf, Interest> = BTreeMap::new();
        let mut complete = true;
        for target in [&self.request.config_file, &self.request.bindings_file]
            .into_iter()
            .chain(&self.request.paths)
        {
            match parent_watch(target) {
                Ok((path, name))
                    if wanted.contains_key(&path) || wanted.len() < MAX_DIRECTORIES =>
                {
                    wanted.entry(path).or_default().names.insert(name);
                }
                Ok(_) => {
                    issue(
                        &mut self.issues,
                        "plugin watcher exceeds 4096 directories".into(),
                    );
                    complete = false;
                }
                Err(error) => {
                    issue(&mut self.issues, error);
                    complete = false;
                }
            }
        }
        let mut visited = BTreeSet::new();
        for root in &self.request.paths {
            let mut stack = if root.is_dir() {
                vec![root.clone()]
            } else {
                vec![]
            };
            while let Some(path) = stack.pop() {
                let canonical = match path.canonicalize() {
                    Ok(path) => path,
                    Err(e) => {
                        issue(&mut self.issues, format!("{}: {e}", path.display()));
                        complete = false;
                        continue;
                    }
                };
                if !visited.insert(canonical.clone()) {
                    continue;
                }
                if !wanted.contains_key(&canonical) && wanted.len() >= MAX_DIRECTORIES {
                    issue(
                        &mut self.issues,
                        "plugin watcher exceeds 4096 directories".into(),
                    );
                    complete = false;
                    break;
                }
                wanted.entry(canonical.clone()).or_default().tree = true;
                let entries = match std::fs::read_dir(&canonical) {
                    Ok(entries) => entries,
                    Err(e) => {
                        issue(&mut self.issues, format!("{}: {e}", canonical.display()));
                        complete = false;
                        continue;
                    }
                };
                for entry in entries {
                    match entry {
                        Ok(entry) if entry.path().is_dir() => {
                            if visited.len() + stack.len() >= MAX_DIRECTORIES {
                                issue(
                                    &mut self.issues,
                                    "plugin watcher exceeds 4096 directories".into(),
                                );
                                complete = false;
                                break;
                            }
                            stack.push(entry.path());
                        }
                        Ok(_) => {}
                        Err(e) => {
                            issue(&mut self.issues, format!("{}: {e}", canonical.display()));
                            complete = false;
                        }
                    }
                }
            }
        }
        // An incomplete scan cannot prove that an existing watch is obsolete.
        if complete {
            let stale: Vec<_> = self
                .watches
                .keys()
                .filter(|p| !wanted.contains_key(*p))
                .cloned()
                .collect();
            for path in stale {
                if let Some(wd) = self.watches.remove(&path) {
                    let _ = self.inotify.watches().remove(wd.descriptor);
                }
            }
        }
        for (path, interest) in wanted {
            if let Err(error) = self.add(&path, interest) {
                issue(&mut self.issues, error);
                complete = false;
            }
        }
        complete
    }
}
// Keep the lexical child name while climbing; canonicalization is only for
// the directory actually watched (including symlinked plugin roots).
fn parent_watch(target: &Path) -> Result<(PathBuf, OsString), String> {
    let mut child = target;
    loop {
        let parent = child
            .parent()
            .ok_or_else(|| format!("no existing parent: {}", target.display()))?;
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if parent.is_dir() {
            return Ok((
                parent.canonicalize().map_err(|e| e.to_string())?,
                child
                    .file_name()
                    .ok_or_else(|| format!("no child name: {}", child.display()))?
                    .to_owned(),
            ));
        }
        child = parent;
    }
}
fn issue(issues: &mut Vec<String>, message: String) {
    if issues.len() < 32 && !issues.contains(&message) {
        issues.push(message);
    }
}
impl std::os::fd::AsFd for ReloadWatcher {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.inotify.as_fd()
    }
}
