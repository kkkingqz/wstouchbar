//! Observes public transitions; does not read private state or execute effects.
use crate::TraceEntry;
use std::collections::BTreeSet;
use touchbar_core::*;
use touchbar_model::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invariant {
    I1,
    I2,
    I3,
    I4,
    I5,
    I6,
    I7,
    I8,
    I9,
    I10,
    I11,
    I12,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantViolation {
    pub invariant: Invariant,
    pub step: usize,
    pub message: String,
}
impl std::fmt::Display for InvariantViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at input {}: {}",
            self.invariant, self.step, self.message
        )
    }
}
impl std::error::Error for InvariantViolation {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyTarget {
    surface: Option<SurfaceId>,
    key: SystemKey,
}
fn target(lease: &KeyLeaseSnapshot) -> KeyTarget {
    KeyTarget {
        surface: (lease.destination == KeyDestination::Surface).then_some(lease.surface),
        key: lease.key,
    }
}
fn instance(snapshot: &CoreSnapshot, id: InstanceId) -> Option<&InstanceSnapshot> {
    snapshot.instances.iter().find(|i| i.id == id)
}
fn surface(snapshot: &CoreSnapshot, id: SurfaceId) -> Option<&SurfaceSnapshot> {
    snapshot.surfaces.iter().find(|s| s.info.id == id)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct InvariantCoverage {
    pub inputs: usize,
    pub panel: bool,
    pub pending: bool,
    pub faulted: bool,
}
#[derive(Clone, Debug)]
pub struct InvariantChecker {
    pub coverage: InvariantCoverage,
    before: CoreSnapshot,
    step: usize,
    first: Option<InvariantViolation>,
    retired: BTreeSet<CaptureId>,
    pressed: Vec<KeyTarget>,
    bindings: Vec<Binding<LayerAction>>,
    desktop: Option<DesktopCapabilities>,
}
impl InvariantChecker {
    pub fn new(initial: CoreSnapshot) -> Self {
        let mut pressed = vec![];
        for lease in &initial.key_leases {
            let key = target(lease);
            if !pressed.contains(&key) {
                pressed.push(key);
            }
        }
        Self {
            coverage: InvariantCoverage::default(),
            before: initial,
            step: 0,
            first: None,
            retired: BTreeSet::new(),
            pressed,
            bindings: vec![],
            desktop: None,
        }
    }
    pub fn observe(
        &mut self,
        entry: &TraceEntry,
        after: &CoreSnapshot,
    ) -> Result<(), InvariantViolation> {
        if let Some(error) = &self.first {
            return Err(error.clone());
        }
        self.coverage.inputs += 1;
        self.coverage.panel |= after.surfaces.iter().any(|s| s.presentation.is_some());
        self.coverage.pending |= after.surfaces.iter().any(|s| s.profile.pending.is_some());
        self.coverage.faulted |= after
            .instances
            .iter()
            .any(|i| i.status == InstanceStatus::Faulted);
        let result = self.check(entry, after);
        match result {
            Ok(()) => {
                self.before = after.clone();
                self.step += 1;
                Ok(())
            }
            Err((invariant, message)) => {
                let error = InvariantViolation {
                    invariant,
                    step: self.step,
                    message,
                };
                self.first = Some(error.clone());
                Err(error)
            }
        }
    }
    pub fn finish(&self) -> Result<(), InvariantViolation> {
        if let Some(error) = &self.first {
            return Err(error.clone());
        }
        let violation = if !self.before.captures.is_empty() {
            Some((Invariant::I2, "unfinished captures"))
        } else if !self.before.key_leases.is_empty() || !self.pressed.is_empty() {
            Some((Invariant::I9, "unreleased keys"))
        } else {
            None
        };
        match violation {
            Some((invariant, message)) => Err(InvariantViolation {
                invariant,
                step: self.step,
                message: message.into(),
            }),
            None => Ok(()),
        }
    }
    fn check(
        &mut self,
        entry: &TraceEntry,
        after: &CoreSnapshot,
    ) -> Result<(), (Invariant, String)> {
        macro_rules! require {
            ($test:expr,$id:ident,$message:expr) => {
                if !$test {
                    return Err((Invariant::$id, $message.into()));
                }
            };
        }
        if let Input::Catalog(catalog) = &entry.input
            && !catalog
                .validate()
                .iter()
                .any(|i| i.code != CatalogIssueCode::ProfileConflict)
        {
            self.bindings = catalog.bindings.clone();
        }
        if let Input::DesktopCapabilities(caps) = entry.input {
            self.desktop = Some(caps);
        }
        let mut ended = BTreeSet::new();
        for output in &entry.outputs {
            if let Output::CaptureFinished {
                capture,
                completion,
            } = output
            {
                require!(
                    ended.insert(capture.id) && !self.retired.contains(&capture.id),
                    I1,
                    "capture completed more than once"
                );
                require!(
                    self.before.captures.iter().any(|c| c == capture),
                    I1,
                    "completion changed its captured target"
                );
                require!(
                    !after.captures.iter().any(|c| c.id == capture.id),
                    I2,
                    "completed capture still present"
                );
                if *completion == CaptureCompletion::Up {
                    require!(
                        matches!(entry.input,Input::Touch {surface,contact,phase:TouchPhase::Up,..} if surface==capture.surface && contact==capture.contact),
                        I1,
                        "Up completed another contact"
                    );
                }
            }
        }
        for old in &self.before.captures {
            if let Some(current) = after.captures.iter().find(|c| c.id == old.id) {
                require!(current == old, I1, "capture retargeted");
            } else {
                require!(
                    ended.contains(&old.id),
                    I1,
                    "capture vanished without completion"
                );
            }
        }
        let mut contacts = vec![];
        for current in &after.captures {
            require!(
                instance(after, current.target.instance).is_some_and(|i| i.status
                    == InstanceStatus::Visible
                    && i.surface == current.surface),
                I5,
                "capture on hidden/dead instance"
            );
            require!(
                surface(after, current.surface).is_some_and(|s| s.mode == Mode::Display),
                I11,
                "capture on native/removed surface"
            );
            require!(
                !self.retired.contains(&current.id),
                I1,
                "completed capture resurrected"
            );
            require!(
                !contacts.contains(&(current.surface, current.contact)),
                I2,
                "duplicate contact capture"
            );
            contacts.push((current.surface, current.contact));
            if !self.before.captures.iter().any(|c| c.id == current.id) {
                require!(
                    matches!(entry.input,Input::Touch {surface,contact,phase:TouchPhase::Down,..} if surface==current.surface && contact==current.contact),
                    I1,
                    "capture without Down"
                );
            }
        }
        if let Input::Touch {
            surface,
            contact,
            phase,
            x,
            y,
        } = entry.input
            && (phase == TouchPhase::Cancel
                || phase == TouchPhase::Up && x.is_finite() && y.is_finite())
        {
            require!(
                !after
                    .captures
                    .iter()
                    .any(|c| c.surface == surface && c.contact == contact),
                I2,
                "terminal contact remains captured"
            );
        }
        self.retired.extend(ended);
        for s in &after.surfaces {
            // The host may deliver another job at exactly the deadline before Timer.
            // Strictly later is always too late; Timer at the deadline must settle it.
            if let Some(pending) = &s.profile.pending {
                require!(
                    pending.deadline >= after.now
                        && !(pending.deadline == after.now
                            && matches!(entry.input, Input::Timer { .. })),
                    I3,
                    "profile missed deadline"
                );
                if let Some(old) =
                    surface(&self.before, s.info.id).and_then(|s| s.profile.pending.as_ref())
                {
                    require!(
                        pending.deadline <= old.deadline,
                        I3,
                        "pending deadline extended"
                    );
                }
            }
            require!(
                s.scene.presentation == s.presentation,
                I4,
                "panel projections disagree"
            );
            if let Some(panel) = &s.presentation {
                require!(
                    instance(after, panel.owner).is_some_and(
                        |i| i.surface == s.info.id && i.status == InstanceStatus::Visible
                    ),
                    I4,
                    "panel owner not Visible on surface"
                );
            }
        }
        for sub in &after.subscriptions {
            require!(
                instance(after, sub.owner).is_some(),
                I7,
                "subscription owned by dead generation"
            );
            if sub.active.is_some() && !sub.spec.background {
                require!(
                    instance(after, sub.owner).is_some_and(|i| i.status == InstanceStatus::Visible),
                    I5,
                    "hidden instance has active foreground subscription"
                );
            }
        }
        for request in &self.before.requests {
            if request.policy.lifetime != Lifetime::Committed
                || after.requests.iter().any(|r| r.id == request.id)
            {
                continue;
            }
            let completed = matches!(entry.input,Input::EffectDone {request:id,..} if id==request.id && request.running);
            let timed_out = matches!(entry.input, Input::Timer { .. })
                && request.deadline.is_some_and(|at| at <= after.now);
            let explicit = if let Input::LogicUpdate {
                instance: owner,
                call,
                update,
            } = &entry.input
            {
                let accepted = instance(&self.before, *owner).is_some_and(|i| {
                    i.in_flight == Some(*call) && i.call.as_ref().is_some_and(|c| !c.discard_update)
                });
                accepted
                    && *owner == request.id.instance
                    && (update.cancel.contains(&request.tag)
                        || !request.running
                            && request.policy.coalesce.as_ref().is_some_and(|key| {
                                update.requests.iter().any(|r| {
                                    r.validate().is_ok()
                                        && r.policy().coalesce.as_ref() == Some(key)
                                })
                            }))
            } else {
                false
            };
            require!(
                completed || timed_out || explicit,
                I6,
                "committed request disappeared through UI change"
            );
        }
        let mut disposed = BTreeSet::new();
        for output in &entry.outputs {
            match output {
                Output::DisposeInstance { instance } => {
                    disposed.insert(*instance);
                }
                Output::CallLogic {
                    instance: id,
                    invocation,
                    ..
                } => {
                    let prior = instance(&self.before, *id);
                    let current = instance(after, *id);
                    let cleanup = matches!(
                        invocation,
                        LogicCall::Handle(
                            LogicEvent::Hidden
                                | LogicEvent::Action(ActionEvent {
                                    kind: ActionKind::Cancelled,
                                    ..
                                })
                        )
                    );
                    let was_visible = prior.is_some_and(|i| i.status == InstanceStatus::Visible);
                    let live = current.or(prior);
                    if matches!(
                        invocation,
                        LogicCall::Handle(
                            LogicEvent::EffectDone { .. }
                                | LogicEvent::SubscriptionData { .. }
                                | LogicEvent::SubscriptionLost { .. }
                        )
                    ) {
                        require!(
                            live.is_some() && !disposed.contains(id),
                            I7,
                            "result delivered to dead generation"
                        );
                    }
                    if matches!(invocation, LogicCall::Init { .. }) {
                        require!(
                            !disposed.contains(id)
                                && live.is_some_and(|i| i.status == InstanceStatus::Loading),
                            I8,
                            "Init on dead or initialized instance"
                        );
                    } else {
                        require!(
                            !disposed.contains(id)
                                && live.is_some_and(|i| matches!(
                                    i.status,
                                    InstanceStatus::Visible | InstanceStatus::Hidden
                                ))
                                || cleanup && was_visible && !disposed.contains(id),
                            I8,
                            "event on Loading/Faulted/Disposed"
                        );
                    }
                    if matches!(invocation, LogicCall::Handle(LogicEvent::Action(_))) {
                        require!(
                            current.is_some_and(|i| i.status == InstanceStatus::Visible)
                                || cleanup && was_visible
                                || matches!(
                                    invocation,
                                    LogicCall::Handle(LogicEvent::Action(ActionEvent {
                                        kind: ActionKind::Cancelled,
                                        ..
                                    }))
                                ) && current.is_some_and(|i| i.status == InstanceStatus::Hidden)
                                    && prior.is_some_and(|i| i.queue.contains(invocation))
                                    && !disposed.contains(id),
                            I5,
                            "action delivered to hidden instance"
                        );
                    }
                }
                Output::Render { surface: id, .. } => {
                    require!(
                        surface(after, *id).is_some_and(|s| s.mode == Mode::Display),
                        I11,
                        "Render in native or removed surface"
                    );
                }
                Output::WatchKeys { surface: id, .. } => {
                    require!(
                        surface(after, *id).is_some_and(|s| s.info.caps.watch_keys),
                        I11,
                        "unsupported WatchKeys"
                    );
                }
                Output::Backlight { surface: id, .. } => {
                    require!(
                        surface(after, *id).is_some_and(|s| s.info.caps.backlight),
                        I11,
                        "unsupported Backlight"
                    );
                }
                Output::SendKey {
                    surface: id,
                    key,
                    phase,
                }
                | Output::DesktopKey {
                    surface: id,
                    key,
                    phase,
                } => {
                    let desktop = matches!(output, Output::DesktopKey { .. });
                    let key_target = KeyTarget {
                        surface: (!desktop).then_some(*id),
                        key: *key,
                    };
                    match phase {
                        KeyPhase::Pressed => {
                            require!(
                                surface(after, *id).is_some_and(|s| s.mode == Mode::Display
                                    && (desktop || s.info.caps.inject_keys)),
                                I11,
                                "new key press in native or unsupported surface"
                            );
                            require!(
                                !desktop || self.desktop.is_none_or(|caps| caps.key),
                                I11,
                                "unsupported desktop key"
                            );
                            require!(
                                !self.pressed.contains(&key_target),
                                I9,
                                "duplicate physical press"
                            );
                            self.pressed.push(key_target);
                        }
                        KeyPhase::Released => {
                            require!(
                                self.pressed.contains(&key_target),
                                I9,
                                "release without press"
                            );
                            self.pressed.retain(|k| *k != key_target);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut expected = vec![];
        for lease in &after.key_leases {
            let key = target(lease);
            if !expected.contains(&key) {
                expected.push(key);
            }
            require!(
                surface(after, lease.surface).is_some_and(|s| s.mode == Mode::Display),
                I9,
                "lease remains on native/removed surface"
            );
            let owner = match lease.owner {
                KeyOwner::Logic(id)
                | KeyOwner::Request(RequestId { instance: id, .. })
                | KeyOwner::Contact { instance: id, .. } => id,
            };
            require!(
                instance(after, owner).is_some_and(
                    |i| i.status == InstanceStatus::Visible && i.surface == lease.surface
                ),
                I9,
                "lease remains after hiding/disposal/fault"
            );
            if let KeyOwner::Contact {
                instance: id,
                contact,
            } = lease.owner
            {
                require!(
                    after.captures.iter().any(|c| c.surface == lease.surface
                        && c.contact == contact
                        && c.target.instance == id),
                    I9,
                    "contact lease without capture"
                );
            }
        }
        require!(
            expected.len() == self.pressed.len()
                && expected.iter().all(|k| self.pressed.contains(k)),
            I9,
            "physical keys differ from live leases"
        );
        let fault = match entry.input {
            Input::LogicFault { instance, .. } | Input::InstanceFault { instance, .. } => {
                Some(instance)
            }
            _ => None,
        };
        if let Some(fault) = fault {
            for old in &self.before.instances {
                if old.id == fault {
                    continue;
                }
                require!(
                    instance(after, old.id)
                        .is_some_and(|i| i.state == old.state && i.status == old.status),
                    I10,
                    "fault changed another instance"
                );
            }
        }
        let source = match &entry.input {
            Input::Touch { surface, .. }
            | Input::Mode { surface, .. }
            | Input::Revoked { surface }
            | Input::Layout { surface, .. }
            | Input::FrameDone { surface, .. } => Some(*surface),
            Input::SurfaceAdded(info) | Input::SurfaceChanged(info) => Some(info.id),
            Input::SurfaceRemoved(id)
            | Input::Key {
                source: KeySource::Surface(id),
                ..
            } => Some(*id),
            Input::LogicUpdate { instance: id, .. }
            | Input::LogicFault { instance: id, .. }
            | Input::InstanceFault { instance: id, .. }
            | Input::LogicError { instance: id, .. } => {
                instance(&self.before, *id).map(|i| i.surface)
            }
            _ => None,
        };
        let explicit_targets = self
            .bindings
            .iter()
            .any(|b| b.action.surfaces != LayerSurfaces::Source);
        if let Some(source) = source {
            for old in &self.before.surfaces {
                if old.info.id == source {
                    continue;
                }
                let cross_bindings = explicit_targets
                    && matches!(
                        entry.input,
                        Input::Key { .. }
                            | Input::Mode { .. }
                            | Input::Revoked { .. }
                            | Input::SurfaceRemoved(_)
                            | Input::SurfaceChanged(_)
                    );
                if cross_bindings {
                    continue;
                }
                require!(
                    surface(after, old.info.id).is_some_and(
                        |s| s.layers == old.layers && s.presentation == old.presentation
                    ),
                    I12,
                    "event changed another surface's panel/layers"
                );
                let previous: Vec<_> = self
                    .before
                    .captures
                    .iter()
                    .filter(|c| c.surface == old.info.id)
                    .collect();
                let next: Vec<_> = after
                    .captures
                    .iter()
                    .filter(|c| c.surface == old.info.id)
                    .collect();
                require!(
                    previous == next,
                    I12,
                    "event changed another surface's captures"
                );
            }
        }
        Ok(())
    }
}
