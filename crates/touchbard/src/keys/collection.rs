//! Hotplug evdev collection. A single nested epoll/inotify wait, no idle timer.
use super::{Batch, Changes, InputKind, KeyReader, LinuxKeyQueries, linux};
use crate::device::{FsMonitor, Roots};
use rustix::fs::{Mode, OFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io,
    os::{
        fd::AsFd,
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
};
use touchbar_model::WatchKey;
struct Registered {
    id: u64,
    dev: u64,
    inode: u64,
}
pub struct InputFault {
    pub path: PathBuf,
    pub error: io::Error,
}
pub struct InputBatch {
    pub changes: Changes,
    pub faults: Vec<InputFault>,
}
pub struct KeyInputs {
    roots: Roots,
    reader: KeyReader<LinuxKeyQueries>,
    watcher: FsMonitor,
    nodes: BTreeMap<PathBuf, Registered>,
    next_id: u64,
    faults: Vec<InputFault>,
}
fn numbered(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("event"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
impl KeyInputs {
    pub fn open(roots: &Roots) -> io::Result<Self> {
        let watcher = FsMonitor::new(&[roots.dev.join("input")])?;
        let reader = KeyReader::new(LinuxKeyQueries)?;
        reader.watch(&watcher)?;
        let mut this = Self {
            roots: roots.clone(),
            reader,
            watcher,
            nodes: BTreeMap::new(),
            next_id: 1,
            faults: Vec::new(),
        };
        this.scan()?;
        Ok(this)
    }
    pub fn set_wanted(&mut self, keys: BTreeSet<WatchKey>) {
        self.reader.set_wanted(keys);
    }
    fn scan(&mut self) -> io::Result<()> {
        let input = self.roots.dev.join("input");
        let paths = match fs::read_dir(&input) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| numbered(p))
                .take(257)
                .collect::<BTreeSet<_>>(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e),
        };
        let stale: Vec<_> = self
            .nodes
            .iter()
            .filter(|(path, node)| {
                !paths.contains(*path)
                    || fs::symlink_metadata(path)
                        .map_or(true, |m| m.dev() != node.dev || m.ino() != node.inode)
            })
            .map(|(p, _)| p.clone())
            .collect();
        for path in stale {
            if let Some(n) = self.nodes.remove(&path) {
                self.reader.remove(n.id);
            }
        }
        for path in paths {
            if self.nodes.contains_key(&path) {
                continue;
            }
            if self.nodes.len() >= 256 {
                self.faults.push(InputFault {
                    path,
                    error: io::Error::new(io::ErrorKind::OutOfMemory, "evdev node limit reached"),
                });
                break;
            }
            match self.open_node(&path) {
                Ok(Some(n)) => {
                    self.nodes.insert(path, n);
                }
                Ok(None) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => self.faults.push(InputFault { path, error }),
            }
        }
        Ok(())
    }
    fn open_node(&mut self, path: &Path) -> io::Result<Option<Registered>> {
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )?;
        let file = File::from(fd);
        let m = file.metadata()?;
        if !m.file_type().is_char_device() {
            return Ok(None);
        }
        let canonical = self
            .roots
            .sys
            .join(format!(
                "dev/char/{}:{}",
                rustix::fs::major(m.rdev()),
                rustix::fs::minor(m.rdev())
            ))
            .canonicalize()?;
        let kind = linux::classify_input(&self.roots.sys, &canonical, &linux::name(file.as_fd())?)?;
        // Never listen to our own output: otherwise an autorepeating virtual key could
        // keep the display awake. Physical inputs with this name remain physical.
        if kind == InputKind::Own {
            return Ok(None);
        }
        let Some(caps) = linux::capabilities(file.as_fd())? else {
            return Ok(None);
        };
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("evdev ID exhausted"))?;
        self.reader.insert(id, file.into(), kind, caps)?;
        Ok(Some(Registered {
            id,
            dev: m.dev(),
            inode: m.ino(),
        }))
    }
    fn convert(&mut self, batch: Batch) -> InputBatch {
        for (id, error) in batch.faults {
            if let Some(path) = self
                .nodes
                .iter()
                .find(|(_, n)| n.id == id)
                .map(|(p, _)| p.clone())
            {
                self.nodes.remove(&path);
                self.faults.push(InputFault { path, error });
            }
        }
        InputBatch {
            changes: batch.changes,
            faults: std::mem::take(&mut self.faults),
        }
    }
    pub fn refresh(&mut self) -> InputBatch {
        let batch = self.reader.refresh();
        self.convert(batch)
    }
    pub async fn next(&mut self) -> io::Result<InputBatch> {
        let mut batch = self.reader.next().await?;
        if batch.rescan {
            // Drop/recreate the watch after one bounded batch. A fresh full scan covers any
            // coalesced/overflowed notifications and gains a narrower watch after mkdir.
            let _ = self.watcher.try_changed();
            let input = self.roots.dev.join("input");
            match FsMonitor::new(std::slice::from_ref(&input)).and_then(|watcher| {
                self.reader.watch(&watcher)?;
                Ok(watcher)
            }) {
                Ok(watcher) => {
                    self.reader.unwatch(&self.watcher);
                    self.watcher = watcher;
                }
                Err(error) => self.faults.push(InputFault { path: input, error }),
            }
            if let Err(error) = self.scan() {
                self.faults.push(InputFault {
                    path: self.roots.dev.join("input"),
                    error,
                });
            }
            let extra = self.reader.refresh();
            batch.changes.keys.extend(extra.changes.keys);
            batch.changes.activity |= extra.changes.activity;
            if extra.changes.lid.is_some() {
                batch.changes.lid = extra.changes.lid;
            }
            batch.faults.extend(extra.faults);
        }
        Ok(self.convert(batch))
    }
}
