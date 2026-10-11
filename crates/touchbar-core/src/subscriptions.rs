use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Subscription {
    owner: InstanceId,
    spec: SubscriptionSpec,
    pub active: Option<SubscriptionId>,
    failures: u32,
    retry_at: Option<Timestamp>,
    timer: Option<TimerId>,
}
impl Subscription {
    pub(crate) fn snapshot(&self) -> SubscriptionSnapshot {
        SubscriptionSnapshot {
            owner: self.owner,
            spec: self.spec.clone(),
            active: self.active,
            failures: self.failures,
            retry_at: self.retry_at,
        }
    }
    pub(crate) fn is_spawn(&self) -> bool {
        matches!(self.spec.kind, SubscriptionKind::SpawnLines(_))
    }
    pub(crate) fn owner(&self) -> InstanceId {
        self.owner
    }
}
impl Core {
    pub(crate) fn subscription_update(
        &mut self,
        owner: InstanceId,
        desired: Option<Vec<SubscriptionSpec>>,
        output: &mut Vec<Output>,
    ) {
        let Some(desired) = desired else {
            return;
        };
        if desired.len() > self.config.max_subscriptions
            || desired.iter().enumerate().any(|(index, spec)| {
                spec.tag.is_empty()
                    || spec.kind.validate().is_err()
                    || desired[..index].iter().any(|old| old.tag == spec.tag)
            })
        {
            warn(output, "invalid desired subscriptions");
            return;
        }
        let mut index = 0;
        while index < self.subscriptions.len() {
            let record = &self.subscriptions[index];
            if record.owner == owner
                && !desired
                    .iter()
                    .any(|d| d.tag == record.spec.tag && d.kind == record.spec.kind)
            {
                let old = self.subscriptions.remove(index);
                self.stop_subscription(old, output);
            } else {
                index += 1;
            }
        }
        for spec in desired {
            if let Some(old) = self
                .subscriptions
                .iter_mut()
                .find(|s| s.owner == owner && s.spec.tag == spec.tag)
            {
                old.spec = spec;
            } else {
                self.subscriptions.push(Subscription {
                    owner,
                    spec,
                    active: None,
                    failures: 0,
                    retry_at: None,
                    timer: None,
                });
            }
        }
    }
    fn stop_subscription(&mut self, record: Subscription, output: &mut Vec<Output>) {
        if let Some(id) = record.active {
            output.push(Output::Unsubscribe { subscription: id });
        }
        if let Some(timer) = record.timer {
            self.timers.cancel(timer);
        }
    }
    fn subscription_eligible(&self, s: &Subscription) -> bool {
        self.instances.get(&s.owner.number).is_some_and(|i| {
            i.id == s.owner
                && (i.status == InstanceStatus::Visible
                    || i.status == InstanceStatus::Hidden && s.spec.background)
        })
    }
    pub(crate) fn prepare_subscriptions(&mut self, output: &mut Vec<Output>) {
        let mut index = 0;
        while index < self.subscriptions.len() {
            let record = &self.subscriptions[index];
            let alive = self.instances.get(&record.owner.number).is_some_and(|i| {
                i.id == record.owner
                    && matches!(i.status, InstanceStatus::Hidden | InstanceStatus::Visible)
            });
            if !alive {
                let old = self.subscriptions.remove(index);
                self.stop_subscription(old, output);
                continue;
            }
            let eligible = self.subscription_eligible(record);
            let record = &mut self.subscriptions[index];
            if !eligible {
                if let Some(id) = record.active.take() {
                    output.push(Output::Unsubscribe { subscription: id });
                }
                if let Some(timer) = record.timer.take() {
                    self.timers.cancel(timer);
                }
            } else if record.timer.is_none()
                && let Some(at) = record.retry_at.filter(|at| *at > self.now)
            {
                record.timer = Some(self.timers.schedule(
                    at,
                    CoreTimer::SubscriptionRetry {
                        owner: record.owner,
                        tag: record.spec.tag.clone(),
                    },
                ));
            }
            index += 1;
        }
    }
    pub(crate) fn start_subscriptions(&mut self, output: &mut Vec<Output>) {
        for index in 0..self.subscriptions.len() {
            let record = &self.subscriptions[index];
            if record.active.is_some()
                || !self.subscription_eligible(record)
                || record.retry_at.is_some_and(|at| at > self.now)
                || record.is_spawn() && !self.spawn_slot_available(record.owner)
            {
                continue;
            }
            let id = SubscriptionId {
                instance: record.owner,
                number: self.next_subscription,
            };
            self.next_subscription = self
                .next_subscription
                .checked_add(1)
                .expect("subscription id exhausted");
            let record = &mut self.subscriptions[index];
            record.active = Some(id);
            record.retry_at = None;
            if let Some(timer) = record.timer.take() {
                self.timers.cancel(timer);
            }
            output.push(Output::Subscribe {
                subscription: id,
                kind: record.spec.kind.clone(),
            });
        }
    }
    pub(crate) fn subscription_lost(&mut self, id: SubscriptionId, error: EffectError) {
        let Some(record) = self.subscriptions.iter_mut().find(|s| s.active == Some(id)) else {
            return;
        };
        record.active = None;
        let delay = self
            .config
            .backoff_initial_ms
            .saturating_mul(
                1_u64
                    .checked_shl(record.failures.min(63))
                    .unwrap_or(u64::MAX),
            )
            .min(self.config.backoff_max_ms);
        record.failures = record.failures.saturating_add(1);
        record.retry_at = Some(self.now.after(delay));
        if let Some(instance) = self
            .instances
            .get_mut(&id.instance.number)
            .filter(|i| i.id == id.instance)
        {
            instance.enqueue(LogicEvent::SubscriptionLost {
                tag: record.spec.tag.clone(),
                error,
            });
        }
    }
    pub(crate) fn subscription_timer(&mut self, owner: InstanceId, tag: &str, timer: TimerId) {
        if let Some(record) = self
            .subscriptions
            .iter_mut()
            .find(|s| s.owner == owner && s.spec.tag == tag && s.timer == Some(timer))
        {
            record.timer = None;
        }
    }
    pub(crate) fn subscription_data(
        &mut self,
        id: SubscriptionId,
        data: SubscriptionData,
        output: &mut Vec<Output>,
    ) {
        let Some(index) = self.subscriptions.iter().position(|s| s.active == Some(id)) else {
            return;
        };
        let record = &self.subscriptions[index];
        if !self.subscription_eligible(record)
            || !valid_data(&record.spec.kind, &data, &self.config)
        {
            warn(output, "invalid subscription data");
            return;
        }
        self.subscriptions[index].failures = 0;
        if let Some(instance) = self
            .instances
            .get_mut(&id.instance.number)
            .filter(|i| i.id == id.instance)
        {
            instance.enqueue(LogicEvent::SubscriptionData {
                tag: self.subscriptions[index].spec.tag.clone(),
                data,
            });
        }
    }
}
fn valid_data(kind: &SubscriptionKind, data: &SubscriptionData, config: &CoreConfig) -> bool {
    let text = |s: &str| s.len() <= config.max_text_bytes;
    match (kind, data) {
        (SubscriptionKind::AudioWatch { .. }, SubscriptionData::Audio(audio)) => {
            audio.volume.is_finite() && text(&audio.name)
        }
        (SubscriptionKind::MediaWatch, SubscriptionData::Media(m)) => [
            m.player.as_deref().unwrap_or(""),
            &m.title,
            &m.artist,
            &m.album,
            m.art_uri.as_deref().unwrap_or(""),
        ]
        .iter()
        .all(|s| text(s)),
        (SubscriptionKind::DesktopFact { .. }, SubscriptionData::Fact(value)) => {
            value.as_ref().is_none_or(|v| {
                v.validate().is_ok()
                    && match v {
                        FactValue::Text(s) => text(s),
                        FactValue::List(v) => {
                            v.len() <= config.max_records && v.iter().all(|s| text(s))
                        }
                        FactValue::Records(v) => {
                            v.len() <= config.max_records
                                && v.iter().all(|r| {
                                    r.iter().all(|(name, value)| {
                                        text(name)
                                            && match value {
                                                Scalar::Text(s) => text(s),
                                                _ => true,
                                            }
                                    })
                                })
                        }
                        _ => true,
                    }
            })
        }
        (SubscriptionKind::TimerInterval { .. }, SubscriptionData::Tick) => true,
        (SubscriptionKind::SpawnLines(_), SubscriptionData::Line(line)) => text(line),
        (
            SubscriptionKind::DbusSignal(_) | SubscriptionKind::DbusProperty(_),
            SubscriptionData::Dbus(value),
        ) => text(&value.signature) && value.json.len() <= config.max_update_bytes,
        _ => false,
    }
}
