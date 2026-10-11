//! Pure Touch Bar state machines: inputs in, commands out.
#![forbid(unsafe_code)]
mod api;
mod config;
mod keys;
mod profiles;
mod scenes;
mod timers;
pub use api::*;
pub use config::*;
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
            timers: TimerRegistry::default(),
            keys: KeyTracker::default(),
            desktop: DesktopCapabilities::default(),
        }
    }
    pub fn snapshot(&self) -> CoreSnapshot {
        CoreSnapshot {
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
                                set: item.set,
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
                        self.render(surface, &mut output);
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
                    self.render(surface, &mut output);
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
                    }
                }
            }
            // These inputs have no recipient until the dependent state machines create one.
            Input::Touch { .. }
            | Input::Key { .. }
            | Input::LogicDescribed { .. }
            | Input::LogicUpdate { .. }
            | Input::LogicFault { .. }
            | Input::EffectDone { .. }
            | Input::SubscriptionData { .. }
            | Input::SubscriptionLost { .. } => {}
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
        self.render(id, output);
    }
    fn placement_change(
        &mut self,
        surface: SurfaceId,
        change: PlacementChange,
        output: &mut Vec<Output>,
    ) {
        match change {
            PlacementChange::Create { instance, set } => {
                if let Some(plugin) = set.logic {
                    output.push(Output::CreateInstance {
                        instance,
                        surface,
                        plugin,
                        set: set.reference,
                    });
                }
            }
            PlacementChange::Remove { instance, set } => {
                output.extend(self.keys.release_owner(KeyOwner::Logic(instance)));
                if set.logic.is_some() {
                    output.push(Output::DisposeInstance { instance });
                }
            }
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
