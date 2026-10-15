//! Bounded logging policy and replay dump rotation, driven by the session clock.
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use touchbar_core::ReplayDump;
use touchbar_model::{InstanceId, Timestamp};
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub level: Level,
    pub instance: Option<InstanceId>,
    pub message: String,
    pub suppressed: u64,
}
struct Repeat {
    last: Timestamp,
    count: u64,
}
struct Guest {
    times: VecDeque<Timestamp>,
    report_at: Option<Timestamp>,
    dropped: u64,
}
#[derive(Default)]
pub struct Diagnostics {
    repeats: BTreeMap<(Option<InstanceId>, String, Level), Repeat>,
    guests: BTreeMap<InstanceId, Guest>,
}
fn message(text: &str) -> String {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}
impl Diagnostics {
    pub fn error(
        &mut self,
        now: Timestamp,
        instance: Option<InstanceId>,
        text: &str,
    ) -> Vec<Record> {
        self.report(now, instance, text, Level::Error)
    }
    pub fn report(
        &mut self,
        now: Timestamp,
        instance: Option<InstanceId>,
        text: &str,
        level: Level,
    ) -> Vec<Record> {
        let text = message(text);
        let key = (instance, text.clone(), level);
        let mut output = self.tick(now);
        if let Some(repeat) = self.repeats.get_mut(&key) {
            repeat.count = repeat.count.saturating_add(1);
            return output;
        }
        if self.repeats.len() >= 1024
            && let Some(key) = self
                .repeats
                .iter()
                .min_by_key(|(_, r)| r.last)
                .map(|(k, _)| k.clone())
        {
            self.repeats.remove(&key);
        }
        self.repeats.insert(
            key,
            Repeat {
                last: now,
                count: 0,
            },
        );
        output.push(Record {
            level,
            instance,
            message: text,
            suppressed: 0,
        });
        output
    }
    pub fn guest(&mut self, now: Timestamp, instance: InstanceId, text: &str) -> Vec<Record> {
        let mut output = self.tick(now);
        let guest = self.guests.entry(instance).or_insert(Guest {
            times: VecDeque::new(),
            report_at: None,
            dropped: 0,
        });
        while guest
            .times
            .front()
            .is_some_and(|at| now.elapsed_since(*at) >= 1000)
        {
            guest.times.pop_front();
        }
        if guest.times.len() >= 100 {
            guest.dropped = guest.dropped.saturating_add(1);
            guest.report_at.get_or_insert(now.after(1000));
        } else {
            guest.times.push_back(now);
            output.push(Record {
                level: Level::Info,
                instance: Some(instance),
                message: message(text),
                suppressed: 0,
            });
        }
        output
    }
    pub fn forget(&mut self, instance: InstanceId) {
        self.guests.remove(&instance);
    }
    pub fn deadline(&self) -> Option<Timestamp> {
        self.repeats
            .values()
            .filter(|r| r.count > 0)
            .map(|r| r.last.after(60000))
            .chain(self.guests.values().filter_map(|g| g.report_at))
            .min()
    }
    pub fn tick(&mut self, now: Timestamp) -> Vec<Record> {
        let mut output = Vec::new();
        for ((instance, text, level), repeat) in &mut self.repeats {
            if repeat.count > 0 && now.elapsed_since(repeat.last) >= 60000 {
                output.push(Record {
                    level: *level,
                    instance: *instance,
                    message: text.clone(),
                    suppressed: repeat.count,
                });
                repeat.last = now;
                repeat.count = 0;
            }
        }
        self.repeats
            .retain(|_, r| r.count > 0 || now.elapsed_since(r.last) < 180000);
        for (instance, guest) in &mut self.guests {
            if guest.report_at.is_some_and(|at| now >= at) {
                output.push(Record {
                    level: Level::Warn,
                    instance: Some(*instance),
                    message: "guest log messages dropped".into(),
                    suppressed: guest.dropped,
                });
                guest.dropped = 0;
                guest.report_at = None;
            }
        }
        output
    }
}
pub struct DumpStore {
    directory: PathBuf,
    last_fault: Option<Timestamp>,
    serial: u64,
}
impl DumpStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            last_fault: None,
            serial: 0,
        }
    }
    pub fn fault(&mut self, now: Timestamp, dump: &ReplayDump) -> Result<Option<PathBuf>, String> {
        if self
            .last_fault
            .is_some_and(|last| now.elapsed_since(last) < 60_000)
        {
            return Ok(None);
        }
        // Failed writes are throttled too, avoiding a tight disk-full retry loop.
        self.last_fault = Some(now);
        self.manual(dump).map(Some)
    }
    pub fn manual(&mut self, dump: &ReplayDump) -> Result<PathBuf, String> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)
            .map_err(|e| e.to_string())?;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        self.serial = self.serial.checked_add(1).ok_or("dump serial exhausted")?;
        let name = format!(
            "dump-{epoch:030}-{:010}-{:020}.json",
            std::process::id(),
            self.serial
        );
        let path = self.directory.join(name);
        let temporary = path.with_extension("tmp");
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(dump.to_json().map_err(|e| e.to_string())?.as_bytes())
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
            let mut files = fs::read_dir(&self.directory)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("dump-") && n.ends_with(".json"))
                })
                .collect::<Vec<_>>();
            files.sort();
            let excess = files.len().saturating_sub(10);
            for old in files.into_iter().take(excess) {
                fs::remove_file(old).map_err(|e| e.to_string())?;
            }
            Ok(path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
