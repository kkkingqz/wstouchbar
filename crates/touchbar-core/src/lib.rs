//! Pure Touch Bar state machines: inputs in, commands out.
#![forbid(unsafe_code)]
mod api;
mod config;
mod instances;
mod keys;
mod profiles;
mod scenes;
mod timers;
pub use api::*;
pub use config::*;
use instances::Instance;
pub use keys::*;
pub use profiles::*;
use scenes::{Placement, PlacementChange, compose};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use timers::*;
use touchbar_model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SurfaceState {
    info: SurfaceInfo,
    mode: Mode,
    generation: Generation,
    scene: Scene,
    layout_generation: Option<Generation>,
    hit_map: HitMap,
    representations: Representations,
    frame_pending: bool,
    dirty: bool,
    resolution: Resolution,
    profile: ProfileSwitch,
    profile_timer: Option<(TimerId, Timestamp)>,
    placements: Vec<Placement>,
}
// Timer purposes are added by the dependent state machines starting in T5.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum CoreTimer {
    Profile { surface: SurfaceId },
    InstanceRetry { instance: InstanceId },
}
#[derive(Clone, Debug)]
pub struct Core {
    config: CoreConfig,
    now: Timestamp,
    catalog: Catalog,
    facts: Facts,
    surfaces: BTreeMap<SurfaceId, SurfaceState>,
    next_generation: u64,
    next_instance: u64,
    next_call: u64,
    instances: BTreeMap<u64, Instance>,
    instance_timers: BTreeMap<u64, (InstanceId, TimerId, Timestamp)>,
    timers: TimerRegistry<CoreTimer>,
    keys: KeyTracker,
    desktop: DesktopCapabilities,
}
impl Core {
    /// Configuration must be validated by the caller before constructing the core.
    pub fn new(config: CoreConfig) -> Self {
        assert!(config.validate().is_ok(), "invalid CoreConfig");
        Self {
            config,
            now: Timestamp(0),
            catalog: Catalog::default(),
            facts: Facts::new(),
            surfaces: BTreeMap::new(),
            next_generation: 1,
            next_instance: 1,
            next_call: 1,
            instances: BTreeMap::new(),
            instance_timers: BTreeMap::new(),
            timers: TimerRegistry::default(),
            keys: KeyTracker::default(),
            desktop: DesktopCapabilities::default(),
        }
    }
    pub fn snapshot(&self) -> CoreSnapshot {
        CoreSnapshot {
            instances: self.instances.values().map(Instance::snapshot).collect(),
            now: self.now,
            facts: self.facts.clone(),
            plugins: self.catalog.plugins.iter().map(|p| p.id.clone()).collect(),
            surfaces: self
                .surfaces
                .values()
                .map(|s| SurfaceSnapshot {
                    info: s.info.clone(),
                    mode: s.mode,
                    generation: s.generation,
                    layout_generation: s.layout_generation,
                    scene: s.scene.clone(),
                    resolution: s.resolution.clone(),
                    profile: s.profile.clone(),
                })
                .collect(),
        }
    }
    pub fn handle(&mut self, now: Timestamp, input: Input) -> Vec<Output> {
        let mut output = vec![];
        if now < self.now {
            warn(&mut output, "timestamp moved backwards");
            return output;
        }
        self.now = now;
        match input {
            Input::SurfaceAdded(info) => {
                if !self.valid_surface(&info, &mut output) {
                    return output;
                }
                if self.surfaces.contains_key(&info.id)
                    || self.surfaces.len() >= self.config.max_surfaces
                    || self.surfaces.values().any(|s| s.info.name == info.name)
                {
                    warn(&mut output, "duplicate surface or surface limit exceeded");
                    return output;
                }
                let id = info.id;
                let mode = if info.caps.modes {
                    Mode::Native
                } else {
                    Mode::Display
                };
                self.surfaces.insert(
                    id,
                    SurfaceState {
                        info,
                        mode,
                        generation: Generation(0),
                        scene: Scene::default(),
                        layout_generation: None,
                        hit_map: vec![],
                        representations: vec![],
                        frame_pending: false,
                        dirty: true,
                        resolution: Resolution::default(),
                        profile: ProfileSwitch::default(),
                        profile_timer: None,
                        placements: vec![],
                    },
                );
                self.resolve_surface(id, &mut output);
            }
            Input::SurfaceChanged(info) => {
                if !self.valid_surface(&info, &mut output) {
                    return output;
                }
                if self
                    .surfaces
                    .values()
                    .any(|s| s.info.id != info.id && s.info.name == info.name)
                {
                    warn(&mut output, "duplicate surface name");
                    return output;
                }
                if let Some(state) = self.surfaces.get_mut(&info.id) {
                    if state.info.caps != info.caps {
                        output.extend(self.keys.release_surface(info.id));
                    }
                    if !info.caps.modes {
                        state.mode = Mode::Display;
                    }
                    let id = info.id;
                    state.info = info;
                    state.dirty = true;
                    self.resolve_surface(id, &mut output);
                }
            }
            Input::SurfaceRemoved(id) | Input::Revoked { surface: id } => {
                output.extend(self.keys.release_surface(id));
                if let Some(surface) = self.surfaces.remove(&id) {
                    if let Some((timer, _)) = surface.profile_timer {
                        self.timers.cancel(timer);
                    }
                    for item in surface.scene.items {
                        self.placement_change(
                            id,
                            PlacementChange::Remove {
                                instance: item.instance,
                            },
                            &mut output,
                        );
                    }
                }
            }
            Input::Mode { surface, mode } => {
                if let Some(state) = self.surfaces.get_mut(&surface) {
                    if !state.info.caps.modes {
                        warn(&mut output, "surface does not support modes");
                        return output;
                    }
                    if state.mode != mode {
                        if mode == Mode::Native {
                            output.extend(self.keys.release_surface(surface));
                            state.hit_map.clear();
                            state.representations.clear();
                            state.layout_generation = None;
                            state.frame_pending = false;
                        }
                        state.mode = mode;
                        state.dirty = true;
                    }
                }
            }
            Input::Layout {
                surface,
                generation,
                hit_map,
                representations,
            } => {
                if let Some(state) = self.surfaces.get_mut(&surface)
                    && state.mode == Mode::Display
                    && state.generation == generation
                {
                    if hit_map.iter().any(|hit| {
                        [hit.rect.x, hit.rect.y, hit.rect.width, hit.rect.height]
                            .iter()
                            .any(|n| !n.is_finite())
                            || hit.rect.width < 0.0
                            || hit.rect.height < 0.0
                    }) {
                        warn(&mut output, "invalid hit map");
                        return output;
                    }
                    state.hit_map = hit_map;
                    state.representations = representations;
                    state.layout_generation = Some(generation);
                }
            }
            Input::FrameDone {
                surface,
                generation,
            } => {
                if let Some(state) = self.surfaces.get_mut(&surface)
                    && state.mode == Mode::Display
                    && state.generation == generation
                {
                    state.frame_pending = false;
                }
            }
            Input::Fact { name, value } => {
                if name.0.is_empty()
                    || name.0.starts_with("surface.")
                    || value.as_ref().is_some_and(|v| v.validate().is_err())
                {
                    warn(&mut output, "invalid or reserved fact");
                    return output;
                }
                match value {
                    Some(value) => {
                        self.facts.insert(name, value);
                    }
                    None => {
                        self.facts.remove(&name);
                    }
                }
                self.resolve_surfaces(&mut output);
            }
            Input::Catalog(catalog) => {
                let issues = catalog.validate();
                let invalid = issues
                    .iter()
                    .any(|i| i.code != CatalogIssueCode::ProfileConflict);
                for issue in issues {
                    warn(
                        &mut output,
                        &format!("{}: {}", issue.location, issue.detail),
                    );
                }
                if !invalid {
                    self.catalog = catalog;
                    self.resolve_surfaces(&mut output);
                    let ids: Vec<_> = self
                        .instances
                        .values()
                        .filter(|i| i.schema_fault)
                        .map(|i| i.id)
                        .collect();
                    for id in ids {
                        self.restart_instance(id, &mut output);
                    }
                }
            }
            Input::DesktopCapabilities(capabilities) => {
                if self.desktop.key && !capabilities.key {
                    output.extend(self.keys.release_desktop());
                }
                self.desktop = capabilities;
            }
            Input::Timer { id } => {
                for (_, task) in self.timers.take_due(now, id) {
                    match task {
                        CoreTimer::Profile { surface } => {
                            if let Some(state) = self.surfaces.get_mut(&surface) {
                                state.profile_timer = None;
                                if state.profile.expire(now) {
                                    // T7 cancels captures before the deadline commits.
                                    self.reconcile_scene(surface, &mut output);
                                }
                            }
                        }
                        CoreTimer::InstanceRetry { instance } => {
                            if self.instances.get(&instance.number).is_some_and(|i| {
                                i.id == instance && i.retry_at.is_some_and(|at| at <= now)
                            }) {
                                self.restart_instance(instance, &mut output);
                            }
                        }
                    }
                }
            }
            Input::LogicDescribed {
                instance,
                description,
            } => {
                if let Some(current) = self
                    .instances
                    .get_mut(&instance.number)
                    .filter(|i| i.id == instance)
                {
                    output.extend(current.describe(description, vec![], now, &self.config));
                }
            }
            Input::LogicUpdate {
                instance,
                call,
                update,
            } => {
                if let Some(current) = self
                    .instances
                    .get_mut(&instance.number)
                    .filter(|i| i.id == instance)
                {
                    // Accepted non-state parts are connected to presentations/effects in T8/T10.
                    let _ = current.update(call, update, &self.config, &mut output);
                }
            }
            Input::LogicError {
                instance,
                call,
                message,
            } => {
                if let Some(current) = self
                    .instances
                    .get_mut(&instance.number)
                    .filter(|i| i.id == instance)
                {
                    output.extend(current.error(call, message, now, &self.config));
                }
            }
            Input::LogicFault {
                instance,
                call,
                reason,
            } => {
                if let Some(current) = self
                    .instances
                    .get_mut(&instance.number)
                    .filter(|i| i.id == instance)
                {
                    output.extend(current.call_fault(call, reason, now, &self.config));
                }
            }
            Input::InstanceFault { instance, reason } => {
                if let Some(current) = self
                    .instances
                    .get_mut(&instance.number)
                    .filter(|i| i.id == instance)
                {
                    output.extend(current.load_fault(reason, now, &self.config));
                }
            }
            // These inputs have no recipient until the dependent state machines create one.
            Input::Touch { .. }
            | Input::Key { .. }
            | Input::EffectDone { .. }
            | Input::SubscriptionData { .. }
            | Input::SubscriptionLost { .. } => {}
        }
        self.synchronize_instances(&mut output);
        self.synchronize_instance_timers();
        for instance in self.instances.values_mut() {
            output.extend(instance.dispatch(&mut self.next_call));
        }
        let surfaces: Vec<_> = self.surfaces.keys().copied().collect();
        for surface in surfaces {
            self.render(surface, &mut output);
        }
        output.extend(self.timers.notify().into_iter().map(Output::from));
        output
    }
    fn valid_surface(&self, info: &SurfaceInfo, output: &mut Vec<Output>) -> bool {
        if let Err(error) = info.validate() {
            warn(output, &error.to_string());
            false
        } else {
            true
        }
    }
    fn resolve_surfaces(&mut self, output: &mut Vec<Output>) {
        let surfaces: Vec<_> = self.surfaces.keys().copied().collect();
        for id in surfaces {
            self.resolve_surface(id, output);
        }
    }
    fn resolve_surface(&mut self, id: SurfaceId, output: &mut Vec<Output>) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        surface.resolution = resolve(&self.catalog, &self.facts, &surface.info);
        // Captures and their membership notifications are connected in T7.
        surface.profile.request(
            self.now,
            surface.resolution.selected.clone(),
            0,
            self.config.profile_pending_ms,
        );
        self.sync_profile_timer(id);
        self.reconcile_scene(id, output);
    }
    fn sync_profile_timer(&mut self, id: SurfaceId) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        let deadline = surface.profile.pending.as_ref().map(|p| p.deadline);
        if surface.profile_timer.map(|(_, at)| at) == deadline {
            return;
        }
        if let Some((timer, _)) = surface.profile_timer.take() {
            self.timers.cancel(timer);
        }
        if let Some(at) = deadline {
            surface.profile_timer = Some((
                self.timers.schedule(at, CoreTimer::Profile { surface: id }),
                at,
            ));
        }
    }
    fn reconcile_scene(&mut self, id: SurfaceId, output: &mut Vec<Output>) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        let composition = compose(
            &self.catalog,
            &surface.info,
            surface.profile.active.as_ref(),
            &surface.scene,
            &surface.placements,
            &mut self.next_instance,
        );
        if surface.scene != composition.scene {
            surface.dirty = true;
        }
        surface.scene = composition.scene;
        surface.placements = composition.placements;
        for change in composition.changes {
            self.placement_change(id, change, output);
        }
    }
    fn placement_change(
        &mut self,
        surface: SurfaceId,
        change: PlacementChange,
        output: &mut Vec<Output>,
    ) {
        match change {
            PlacementChange::Create { instance, set } => {
                let state = self.surfaces.get(&surface).expect("placement surface");
                let slot = state
                    .scene
                    .items
                    .iter()
                    .find(|i| i.instance == instance)
                    .expect("new placement")
                    .slot
                    .clone();
                let logic =
                    Instance::new(instance, surface, slot, set, state.mode == Mode::Display);
                output.extend(logic.creation());
                self.instances.insert(instance.number, logic);
            }
            PlacementChange::Remove { instance } => {
                output.extend(self.keys.release_owner(KeyOwner::Logic(instance)));
                if let Some(mut logic) = self.instances.remove(&instance.number) {
                    output.extend(logic.dispose(&mut self.next_call));
                }
            }
        }
    }
    fn synchronize_instances(&mut self, output: &mut Vec<Output>) {
        for instance in self.instances.values_mut() {
            let visible = self.surfaces.get(&instance.surface).is_some_and(|s| {
                s.mode == Mode::Display && s.scene.items.iter().any(|i| i.instance == instance.id)
            });
            instance.set_visible(visible);
            if instance.status == InstanceStatus::Faulted {
                output.extend(self.keys.release_owner(KeyOwner::Logic(instance.id)));
            }
        }
        for surface in self.surfaces.values_mut() {
            for item in &mut surface.scene.items {
                if let Some(instance) = self
                    .instances
                    .get_mut(&item.instance.number)
                    .filter(|i| i.id == item.instance)
                {
                    instance.set = item.set.clone();
                    if item.status != instance.status || item.state != instance.state {
                        item.status = instance.status;
                        item.state = instance.state.clone();
                        surface.dirty = true;
                    }
                }
            }
        }
    }
    fn synchronize_instance_timers(&mut self) {
        let obsolete: Vec<_> = self
            .instance_timers
            .iter()
            .filter(|(number, (instance, _, at))| {
                !self
                    .instances
                    .get(number)
                    .is_some_and(|i| i.id == *instance && i.retry_at == Some(*at))
            })
            .map(|(number, _)| *number)
            .collect();
        for number in obsolete {
            if let Some((_, timer, _)) = self.instance_timers.remove(&number) {
                self.timers.cancel(timer);
            }
        }
        for instance in self.instances.values() {
            if let Some(at) = instance.retry_at
                && !self.instance_timers.contains_key(&instance.id.number)
            {
                let timer = self.timers.schedule(
                    at,
                    CoreTimer::InstanceRetry {
                        instance: instance.id,
                    },
                );
                self.instance_timers
                    .insert(instance.id.number, (instance.id, timer, at));
            }
        }
    }
    fn restart_instance(&mut self, id: InstanceId, output: &mut Vec<Output>) {
        let Some(instance) = self.instances.get_mut(&id.number).filter(|i| i.id == id) else {
            return;
        };
        if let Some(create) = instance.restart() {
            let new = instance.id;
            if let Some(surface) = self.surfaces.get_mut(&instance.surface) {
                for placement in &mut surface.placements {
                    if placement.instance == id {
                        placement.instance = new;
                    }
                }
                for item in &mut surface.scene.items {
                    if item.instance == id {
                        item.instance = new;
                    }
                }
                surface.dirty = true;
            }
            output.push(create);
        }
    }
    fn render(&mut self, id: SurfaceId, output: &mut Vec<Output>) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        if surface.mode == Mode::Native || surface.frame_pending || !surface.dirty {
            return;
        }
        surface.generation = Generation(self.next_generation);
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("render generation exhausted");
        surface.dirty = false;
        surface.frame_pending = true;
        output.push(Output::Render {
            surface: id,
            generation: surface.generation,
            scene: surface.scene.clone(),
        });
    }
}
fn warn(output: &mut Vec<Output>, event: &str) {
    output.push(Output::Log {
        level: LogLevel::Warn,
        event: event.into(),
    });
}
impl From<TimerChange> for Output {
    fn from(change: TimerChange) -> Self {
        match change {
            TimerChange::Set { id, at } => Self::SetTimer { id, at },
            TimerChange::Clear { id } => Self::ClearTimer { id },
        }
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    fn surface() -> SurfaceInfo {
        SurfaceInfo {
            id: SurfaceId(1),
            name: "touchbar".parse().unwrap(),
            kind: SurfaceKind::Touchbar,
            width_px: 2008,
            height_px: 60,
            px_per_mm: 6.0,
            caps: SurfaceCaps {
                modes: true,
                inject_keys: true,
                ..SurfaceCaps::default()
            },
        }
    }
    #[test]
    fn mode_native_revocation_and_removal_release_existing_keys() {
        for input in [
            Input::Mode {
                surface: SurfaceId(1),
                mode: Mode::Native,
            },
            Input::Revoked {
                surface: SurfaceId(1),
            },
            Input::SurfaceRemoved(SurfaceId(1)),
        ] {
            let mut core = Core::new(CoreConfig::default());
            let info = surface();
            core.handle(Timestamp(0), Input::SurfaceAdded(info.clone()));
            core.handle(
                Timestamp(1),
                Input::Mode {
                    surface: info.id,
                    mode: Mode::Display,
                },
            );
            // Leases normally originate in the widgets/effects added in T7/T10.
            core.keys
                .press(
                    KeyOwner::Logic(InstanceId {
                        number: 1,
                        generation: Generation(1),
                    }),
                    &info,
                    Mode::Display,
                    DesktopCapabilities::default(),
                    SystemKey::F1,
                )
                .unwrap();
            let output = core.handle(Timestamp(2), input);
            assert_eq!(
                output,
                vec![Output::SendKey {
                    surface: info.id,
                    key: SystemKey::F1,
                    phase: KeyPhase::Released
                }]
            );
        }
    }
    #[test]
    fn desktop_capability_loss_releases_existing_desktop_key() {
        let mut core = Core::new(CoreConfig::default());
        let mut info = surface();
        info.caps.inject_keys = false;
        core.handle(Timestamp(0), Input::SurfaceAdded(info.clone()));
        core.handle(
            Timestamp(1),
            Input::DesktopCapabilities(DesktopCapabilities { key: true }),
        );
        core.keys
            .press(
                KeyOwner::Logic(InstanceId {
                    number: 1,
                    generation: Generation(1),
                }),
                &info,
                Mode::Display,
                core.desktop,
                SystemKey::F1,
            )
            .unwrap();
        assert_eq!(
            core.handle(
                Timestamp(2),
                Input::DesktopCapabilities(DesktopCapabilities::default())
            ),
            vec![Output::DesktopKey {
                surface: info.id,
                key: SystemKey::F1,
                phase: KeyPhase::Released
            }]
        );
    }
}

#[cfg(test)]
mod profile_timer_tests {
    use super::*;
    #[test]
    fn pending_target_updates_keep_the_same_advertised_timer() {
        let mut core = Core::new(CoreConfig::default());
        let info = SurfaceInfo {
            id: SurfaceId(1),
            name: "headless:main".parse().unwrap(),
            kind: SurfaceKind::Headless,
            width_px: 100,
            height_px: 10,
            px_per_mm: 5.0,
            caps: SurfaceCaps::default(),
        };
        core.handle(Timestamp(0), Input::SurfaceAdded(info));
        // T7 supplies real contact membership. Exercise the profile subsystem here.
        let state = core.surfaces.get_mut(&SurfaceId(1)).unwrap();
        state
            .profile
            .request(Timestamp(0), Some("p/a".parse().unwrap()), 0, 1500);
        state
            .profile
            .request(Timestamp(10), Some("p/b".parse().unwrap()), 1, 1500);
        core.sync_profile_timer(SurfaceId(1));
        assert!(matches!(
            core.timers.notify().as_slice(),
            [TimerChange::Set {
                at: Timestamp(1510),
                ..
            }]
        ));
        core.surfaces
            .get_mut(&SurfaceId(1))
            .unwrap()
            .profile
            .request(Timestamp(1000), Some("p/c".parse().unwrap()), 1, 1500);
        core.sync_profile_timer(SurfaceId(1));
        assert!(core.timers.notify().is_empty());
    }
}
