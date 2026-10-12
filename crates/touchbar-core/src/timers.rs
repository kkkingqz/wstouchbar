use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use touchbar_model::{TimerId, Timestamp};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerChange {
    Set { id: TimerId, at: Timestamp },
    Clear { id: TimerId },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Scheduled<T> {
    at: Timestamp,
    task: T,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimerRegistry<T> {
    next_id: u64,
    entries: BTreeMap<TimerId, Scheduled<T>>,
    advertised: Option<(TimerId, Timestamp)>,
}
impl<T> Default for TimerRegistry<T> {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: BTreeMap::new(),
            advertised: None,
        }
    }
}
impl<T> TimerRegistry<T> {
    pub(crate) fn has_due(&self, now: Timestamp) -> bool {
        self.advertised.is_some_and(|(_, at)| at <= now)
    }
    pub fn deadline(&self, id: TimerId) -> Option<Timestamp> {
        self.entries.get(&id).map(|job| job.at)
    }
    pub fn schedule(&mut self, at: Timestamp, task: T) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("timer id exhausted");
        self.entries.insert(id, Scheduled { at, task });
        id
    }
    pub fn cancel(&mut self, id: TimerId) -> Option<T> {
        self.entries.remove(&id).map(|job| job.task)
    }
    pub fn notify(&mut self) -> Vec<TimerChange> {
        let next = self
            .entries
            .iter()
            .map(|(id, job)| (*id, job.at))
            .min_by_key(|(id, at)| (*at, *id));
        if next == self.advertised {
            return vec![];
        }
        let mut changes = vec![];
        if let Some((id, _)) = self.advertised {
            changes.push(TimerChange::Clear { id });
        }
        if let Some((id, at)) = next {
            changes.push(TimerChange::Set { id, at });
        }
        self.advertised = next;
        changes
    }
    pub fn take_due(&mut self, now: Timestamp, id: TimerId) -> Vec<(TimerId, T)> {
        if !self.advertised.is_some_and(|(current, _)| current == id) {
            return vec![];
        }
        self.advertised = None;
        if !self.entries.contains_key(&id) {
            return vec![];
        }
        let mut due: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, job)| job.at <= now)
            .map(|(id, job)| (job.at, *id))
            .collect();
        due.sort();
        due.into_iter()
            .filter_map(|(_, id)| self.entries.remove(&id).map(|job| (id, job.task)))
            .collect()
    }
}
