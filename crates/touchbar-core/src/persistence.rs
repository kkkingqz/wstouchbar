use crate::*;

// A vector keeps structured keys JSON-serializable and preserves deterministic order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedState {
    key: PersistKey,
    fields: StatePatch,
    saved: StatePatch,
    owner: Option<InstanceId>,
    timer: Option<TimerId>,
}
fn merge(fields: &mut StatePatch, name: String, value: StateValue) {
    if let Some((_, old)) = fields.iter_mut().find(|(n, _)| *n == name) {
        *old = value;
    } else {
        fields.push((name, value));
    }
}
fn dirty(record: &PersistedState) -> StatePatch {
    record
        .fields
        .iter()
        .filter(|(name, value)| !record.saved.iter().any(|(n, v)| n == name && v == value))
        .cloned()
        .collect()
}
impl Core {
    /// Read unsaved patches without changing timers or replay state. The shell
    /// flushes these patches before destroying instances at session shutdown.
    pub fn pending_persist(&self) -> Vec<(PersistKey, StatePatch)> {
        self.persisted
            .iter()
            .filter_map(|record| {
                let fields = dirty(record);
                (!fields.is_empty()).then(|| (record.key.clone(), fields))
            })
            .collect()
    }

    pub(crate) fn load_persisted(
        &mut self,
        key: &PersistKey,
        disk: StatePatch,
        description: &LogicDescription,
    ) -> StatePatch {
        let candidate = self
            .persisted
            .iter()
            .find(|p| &p.key == key)
            .map(|p| p.fields.clone())
            .unwrap_or(disk);
        let mut fields = vec![];
        for (name, value) in candidate {
            if description
                .state
                .iter()
                .any(|f| f.name == name && f.persist && f.kind.accepts(&value))
                && instances::value_within_limits(&value, &self.config)
            {
                let mut next = fields.clone();
                merge(&mut next, name, value);
                if serde_json::to_vec(&next)
                    .is_ok_and(|bytes| bytes.len() <= self.config.max_persist_bytes)
                {
                    fields = next;
                }
            }
        }
        if !self.persisted.iter().any(|p| &p.key == key) {
            self.persisted.push(PersistedState {
                key: key.clone(),
                fields: fields.clone(),
                saved: fields.clone(),
                owner: None,
                timer: None,
            });
        }
        fields
    }
    pub(crate) fn persist_update(&mut self, owner: InstanceId, output: &mut Vec<Output>) {
        let Some(instance) = self.instances.get(&owner.number).filter(|i| i.id == owner) else {
            return;
        };
        let fields = instance.persistent_fields();
        let key = instance.persist_key.clone();
        let Some(record) = self.persisted.iter_mut().find(|p| p.key == key) else {
            return;
        };
        let mut next = record.fields.clone();
        for (name, value) in fields {
            merge(&mut next, name, value);
        }
        if next == record.fields {
            return;
        }
        if !serde_json::to_vec(&next)
            .is_ok_and(|bytes| bytes.len() <= self.config.max_persist_bytes)
        {
            warn(output, "persistent state exceeds byte limit");
            return;
        }
        record.fields = next;
        record.owner = Some(owner);
        if let Some(timer) = record.timer.take() {
            self.timers.cancel(timer);
        }
        if !dirty(record).is_empty() {
            record.timer = Some(self.timers.schedule(
                self.now.after(self.config.persist_delay_ms),
                CoreTimer::Persist { key },
            ));
        }
    }
    pub(crate) fn persist_timer(
        &mut self,
        key: &PersistKey,
        timer: TimerId,
        output: &mut Vec<Output>,
    ) {
        let Some(record) = self
            .persisted
            .iter_mut()
            .find(|p| &p.key == key && p.timer == Some(timer))
        else {
            return;
        };
        record.timer = None;
        let fields = dirty(record);
        if let Some(owner) = record.owner
            && !fields.is_empty()
        {
            output.push(Output::Persist {
                instance: owner,
                key: key.clone(),
                fields,
            });
            record.saved = record.fields.clone();
        }
    }
}
