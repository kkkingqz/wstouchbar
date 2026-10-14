use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Request {
    id: RequestId,
    surface: SurfaceId,
    tag: String,
    effect: Effect,
    policy: EffectPolicy,
    running: bool,
    timer: Option<TimerId>,
}
impl Request {
    pub(crate) fn snapshot(&self, timers: &TimerRegistry<CoreTimer>) -> RequestSnapshot {
        RequestSnapshot {
            id: self.id,
            surface: self.surface,
            tag: self.tag.clone(),
            effect: self.effect.clone(),
            policy: self.policy.clone(),
            running: self.running,
            deadline: self.timer.and_then(|id| timers.deadline(id)),
        }
    }
}
impl Core {
    pub(crate) fn request_update(
        &mut self,
        owner: InstanceId,
        requests: Vec<EffectRequest>,
        cancel: Vec<String>,
        output: &mut Vec<Output>,
    ) {
        let ids: Vec<_> = self
            .requests
            .iter()
            .filter(|r| r.id.instance == owner && cancel.contains(&r.tag))
            .map(|r| r.id)
            .collect();
        for id in ids {
            self.cancel_request(id, EffectError::Cancelled, output);
        }
        let Some(surface) = self
            .instances
            .get(&owner.number)
            .filter(|i| i.id == owner)
            .map(|i| i.surface)
        else {
            return;
        };
        if requests.len() > self.config.max_requests_per_update {
            warn(output, "too many effect requests");
            return;
        }
        for request in requests {
            if request.validate().is_err()
                || touchbar_logic_types::wire_size_event(&LogicEvent::EffectDone {
                    tag: request.tag.clone(),
                    result: Err(EffectError::Failed("too large".into())),
                }) > self.config.max_update_bytes
                || matches!(&request.effect, Effect::SpawnRun(spec) if touchbar_logic_types::wire_size_spawn_result(&request.tag, spec.max_output) > self.config.max_update_bytes)
            {
                warn(output, "invalid effect request");
                continue;
            }
            let policy = request.policy();
            if policy.lifetime == Lifetime::UntilHidden
                && self.instances[&owner.number].status != InstanceStatus::Visible
            {
                warn(output, "until-hidden request requires a visible instance");
                continue;
            }
            if let Some(key) = &policy.coalesce {
                let replaced: Vec<_> = self
                    .requests
                    .iter()
                    .filter(|r| {
                        r.id.instance == owner
                            && !r.running
                            && r.policy.coalesce.as_ref() == Some(key)
                    })
                    .map(|r| r.id)
                    .collect();
                for id in replaced {
                    self.cancel_request(id, EffectError::Cancelled, output);
                }
            }
            let id = RequestId {
                instance: owner,
                number: self.next_request,
            };
            self.next_request = self
                .next_request
                .checked_add(1)
                .expect("request id exhausted");
            self.requests.push(Request {
                id,
                surface,
                tag: request.tag,
                effect: request.effect,
                policy,
                running: false,
                timer: None,
            });
        }
    }
    fn effect_result(
        &mut self,
        owner: InstanceId,
        tag: String,
        result: EffectOutcome,
        output: &mut Vec<Output>,
    ) {
        if let Some(instance) = self
            .instances
            .get_mut(&owner.number)
            .filter(|i| i.id == owner)
        {
            let mut event = LogicEvent::EffectDone { tag, result };
            if touchbar_logic_types::wire_size_event(&event) > self.config.max_update_bytes {
                warn(output, "effect result exceeds input budget");
                if let LogicEvent::EffectDone { result, .. } = &mut event {
                    *result = Err(EffectError::Failed("too large".into()));
                }
            }
            if touchbar_logic_types::wire_size_event(&event) <= self.config.max_update_bytes {
                instance.enqueue(event);
            }
        }
    }
    pub(crate) fn effect_done(
        &mut self,
        id: RequestId,
        result: EffectOutcome,
        output: &mut Vec<Output>,
    ) {
        let Some(index) = self.requests.iter().position(|r| r.id == id && r.running) else {
            return;
        };
        let request = self.requests.remove(index);
        if let Some(timer) = request.timer {
            self.timers.cancel(timer);
        }
        self.effect_result(id.instance, request.tag, result, output);
    }
    pub(crate) fn cancel_request(
        &mut self,
        id: RequestId,
        error: EffectError,
        output: &mut Vec<Output>,
    ) {
        let Some(index) = self.requests.iter().position(|r| r.id == id) else {
            return;
        };
        let request = self.requests.remove(index);
        if let Some(timer) = request.timer {
            self.timers.cancel(timer);
        }
        if request.running {
            output.push(Output::CancelEffect { request: id });
        }
        self.effect_result(id.instance, request.tag, Err(error), output);
    }
    pub(crate) fn synchronize_effects(&mut self, output: &mut Vec<Output>) {
        let cancel: Vec<_> = self
            .requests
            .iter()
            .filter(|r| {
                r.policy.lifetime == Lifetime::UntilHidden
                    && !self.instances.get(&r.id.instance.number).is_some_and(|i| {
                        i.id == r.id.instance && i.status == InstanceStatus::Visible
                    })
            })
            .map(|r| r.id)
            .collect();
        for id in cancel {
            self.cancel_request(id, EffectError::Cancelled, output);
        }
        self.pump_requests(output);
    }
    fn can_start_request(&self, request: &Request) -> bool {
        let owner = request.id.instance;
        let running = self
            .requests
            .iter()
            .filter(|r| r.running && r.id.instance == owner)
            .count();
        if running >= self.config.effect_slots {
            return false;
        }
        if request.policy.coalesce.as_ref().is_some_and(|key| {
            self.requests.iter().any(|r| {
                r.running && r.id.instance == owner && r.policy.coalesce.as_ref() == Some(key)
            })
        }) {
            return false;
        }
        !matches!(request.effect, Effect::SpawnRun(_)) || self.spawn_slot_available(owner)
    }
    pub(crate) fn spawn_slot_available(&self, owner: InstanceId) -> bool {
        let running: Vec<_> = self
            .requests
            .iter()
            .filter(|r| r.running && matches!(r.effect, Effect::SpawnRun(_)))
            .collect();
        let subscriptions: Vec<_> = self
            .subscriptions
            .iter()
            .filter(|s| s.active.is_some() && s.is_spawn())
            .collect();
        running.len() + subscriptions.len() < self.config.session_spawn_slots
            && running.iter().filter(|r| r.id.instance == owner).count()
                + subscriptions.iter().filter(|s| s.owner() == owner).count()
                < self.config.spawn_slots
    }
    fn pump_requests(&mut self, output: &mut Vec<Output>) {
        while let Some(index) = self
            .requests
            .iter()
            .position(|r| !r.running && self.can_start_request(r))
        {
            let request = self.requests[index].clone();
            self.requests[index].running = true;
            if matches!(
                request.effect,
                Effect::KeyTap { .. } | Effect::KeyDown { .. } | Effect::KeyUp { .. }
            ) {
                let result = self.key_request(&request, output);
                self.effect_done(request.id, result, output);
                continue;
            }
            if let Some(ms) = request.policy.timeout_ms {
                self.requests[index].timer = Some(self.timers.schedule(
                    self.now.after(ms),
                    CoreTimer::EffectTimeout {
                        request: request.id,
                    },
                ));
            }
            output.push(Output::StartEffect {
                request: request.id,
                effect: request.effect,
                policy: request.policy,
            });
        }
    }
    fn key_request(&mut self, request: &Request, output: &mut Vec<Output>) -> EffectOutcome {
        let owner = KeyOwner::Logic(request.id.instance);
        if let Effect::KeyUp { key } = request.effect {
            output.extend(self.keys.release_key(owner, key));
            return Ok(EffectResult::None);
        }
        if matches!(request.effect, Effect::KeyDown { .. })
            && !self
                .instances
                .get(&request.id.instance.number)
                .is_some_and(|i| i.id == request.id.instance && i.status == InstanceStatus::Visible)
        {
            return Err(EffectError::Unavailable);
        }
        let Some(surface) = self.surfaces.get(&request.surface) else {
            return Err(EffectError::Unavailable);
        };
        let (key, lease) = match request.effect {
            Effect::KeyTap { key } => (key, KeyOwner::Request(request.id)),
            Effect::KeyDown { key } => (key, owner),
            _ => unreachable!(),
        };
        output.extend(
            self.keys
                .press(lease, &surface.info, surface.mode, self.desktop, key)?,
        );
        if matches!(request.effect, Effect::KeyTap { .. }) {
            output.extend(self.keys.release_owner(lease));
        }
        Ok(EffectResult::None)
    }
}
