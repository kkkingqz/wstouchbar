use crate::*;
use std::collections::BTreeSet;
use touchbar_bindings::{BindingEngine, BindingEvent, KeyInput};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layer {
    pub set: SetRef,
    pub latched: bool,
    pub held: BTreeSet<(KeySource, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SourceBindings {
    source: KeySource,
    engine: BindingEngine<LayerAction>,
    timer: Option<(TimerId, Timestamp)>,
}
impl Core {
    pub(crate) fn key_input(
        &mut self,
        source: KeySource,
        code: WatchKey,
        phase: KeyPhase,
        output: &mut Vec<Output>,
    ) {
        if let KeySource::Surface(id) = source
            && self
                .surfaces
                .get(&id)
                .is_none_or(|s| !s.info.caps.watch_keys || s.mode != Mode::Display)
        {
            return;
        }
        let index = self
            .bindings
            .iter()
            .position(|b| b.source == source)
            .unwrap_or_else(|| {
                let mut engine = BindingEngine::new(self.catalog.bindings.clone())
                    .expect("validated catalog bindings");
                engine.set_facts(self.now, self.binding_facts(source));
                self.bindings.push(SourceBindings {
                    source,
                    engine,
                    timer: None,
                });
                self.bindings.len() - 1
            });
        let events = self.bindings[index]
            .engine
            .handle(self.now, KeyInput { code, phase });
        self.binding_events(source, events, output);
    }
    fn binding_facts(&self, source: KeySource) -> Facts {
        let mut facts = self.facts.clone();
        if let KeySource::Surface(id) = source
            && let Some(surface) = self.surfaces.get(&id)
        {
            facts.extend(surface.info.facts());
        }
        facts
    }
    pub(crate) fn update_binding_facts(&mut self, output: &mut Vec<Output>) {
        let sources: Vec<_> = self.bindings.iter().map(|b| b.source).collect();
        for source in sources {
            let facts = self.binding_facts(source);
            let index = self
                .bindings
                .iter()
                .position(|b| b.source == source)
                .unwrap();
            let events = self.bindings[index].engine.set_facts(self.now, facts);
            self.binding_events(source, events, output);
        }
    }
    pub(crate) fn binding_timer(
        &mut self,
        source: KeySource,
        id: TimerId,
        output: &mut Vec<Output>,
    ) {
        let Some(current) = self
            .bindings
            .iter_mut()
            .find(|b| b.source == source && b.timer.is_some_and(|(timer, _)| timer == id))
        else {
            return;
        };
        current.timer = None;
        let events = current.engine.advance(self.now);
        self.binding_events(source, events, output);
    }
    pub(crate) fn sync_binding_timers(&mut self) {
        for source in &mut self.bindings {
            let at = source.engine.next_deadline();
            if source.timer.map(|(_, at)| at) == at {
                continue;
            }
            if let Some((id, _)) = source.timer.take() {
                self.timers.cancel(id);
            }
            if let Some(at) = at {
                source.timer = Some((
                    self.timers.schedule(
                        at,
                        CoreTimer::Binding {
                            source: source.source,
                        },
                    ),
                    at,
                ));
            }
        }
    }
    pub(crate) fn reset_binding_source(&mut self, source: KeySource, output: &mut Vec<Output>) {
        if let Some(index) = self.bindings.iter().position(|b| b.source == source) {
            let current = self.bindings.remove(index);
            if let Some((timer, _)) = current.timer {
                self.timers.cancel(timer);
            }
        }
        let ids: Vec<_> = self.surfaces.keys().copied().collect();
        for id in ids {
            let before = self.surfaces[&id].layers.clone();
            for layer in &mut self.surfaces.get_mut(&id).unwrap().layers {
                layer.held.retain(|(s, _)| *s != source);
            }
            self.layers_changed(id, before, output);
        }
    }
    pub(crate) fn clear_target_holds(&mut self, id: SurfaceId, output: &mut Vec<Output>) {
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        let before = surface.layers.clone();
        for layer in &mut surface.layers {
            layer.held.clear();
        }
        self.layers_changed(id, before, output);
    }
    pub(crate) fn reload_bindings(&mut self, output: &mut Vec<Output>) {
        let sources: Vec<_> = self.bindings.iter().map(|b| b.source).collect();
        for source in sources {
            self.reset_binding_source(source, output);
        }
        self.prune_layers(output);
    }
    pub(crate) fn prune_layers(&mut self, output: &mut Vec<Output>) {
        let ids: Vec<_> = self.surfaces.keys().copied().collect();
        for id in ids {
            let before = self.surfaces[&id].layers.clone();
            self.surfaces
                .get_mut(&id)
                .unwrap()
                .layers
                .retain(|layer| self.catalog.set_available(&layer.set));
            self.layers_changed(id, before, output);
        }
    }
    pub(crate) fn watch_binding_keys(&mut self, output: &mut Vec<Output>) {
        let set: Vec<_> = self
            .catalog
            .bindings
            .iter()
            .flat_map(|b| b.trigger.watch_keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for surface in self.surfaces.values_mut() {
            if surface.info.caps.watch_keys && surface.watched != set {
                surface.watched = set.clone();
                output.push(Output::WatchKeys {
                    surface: surface.info.id,
                    set: set.clone(),
                });
            } else if !surface.info.caps.watch_keys {
                surface.watched.clear();
            }
        }
    }
    fn binding_targets(&self, source: KeySource, surfaces: &LayerSurfaces) -> Vec<SurfaceId> {
        self.surfaces
            .iter()
            .filter(|(id, s)| {
                s.mode == Mode::Display
                    && match surfaces {
                        LayerSurfaces::Source => match source {
                            KeySource::Surface(source) => **id == source,
                            KeySource::Desktop => !s.info.caps.watch_keys,
                        },
                        LayerSurfaces::Names(names) => names.contains(&s.info.name),
                        LayerSurfaces::All => true,
                    }
            })
            .map(|(id, _)| *id)
            .collect()
    }
    fn binding_events(
        &mut self,
        source: KeySource,
        events: Vec<BindingEvent<LayerAction>>,
        output: &mut Vec<Output>,
    ) {
        for event in events {
            let (id, action, held, ended) = match event {
                BindingEvent::Began { id, action } => (id, action, true, false),
                BindingEvent::Ended { id, action } => (id, action, true, true),
                BindingEvent::Fired { id, action } => (id, action, false, false),
            };
            // Ended removes exactly this ownership, independent of changed targets/modes.
            if ended {
                let targets: Vec<_> = self.surfaces.keys().copied().collect();
                for target in targets {
                    let before = self.surfaces[&target].layers.clone();
                    for layer in &mut self.surfaces.get_mut(&target).unwrap().layers {
                        layer.held.remove(&(source, id.clone()));
                    }
                    self.layers_changed(target, before, output);
                }
                continue;
            }
            let targets = self.binding_targets(source, &action.surfaces);
            for target in targets {
                let before = self.surfaces[&target].layers.clone();
                match &action.operation {
                    LayerOperation::ClearLayers => {
                        self.surfaces.get_mut(&target).unwrap().layers.clear()
                    }
                    LayerOperation::Layer(set) | LayerOperation::ToggleLayer(set) => {
                        if !self.catalog.set_available(set) {
                            warn(output, "layer set is unavailable");
                            continue;
                        }
                        let layers = &mut self.surfaces.get_mut(&target).unwrap().layers;
                        let old_index = layers.iter().position(|l| &l.set == set);
                        let mut layer =
                            old_index
                                .map(|index| layers.remove(index))
                                .unwrap_or_else(|| Layer {
                                    set: set.clone(),
                                    latched: false,
                                    held: BTreeSet::new(),
                                });
                        let activated =
                            if held && matches!(action.operation, LayerOperation::Layer(_)) {
                                layer.held.insert((source, id.clone()));
                                true
                            } else {
                                layer.latched = !layer.latched;
                                layer.latched
                            };
                        if activated {
                            layers.push(layer);
                        } else {
                            layers.insert(old_index.unwrap_or(layers.len()), layer);
                        }
                    }
                }
                self.layers_changed(target, before, output);
            }
        }
    }
    fn layers_changed(&mut self, surface: SurfaceId, before: Vec<Layer>, output: &mut Vec<Output>) {
        let state = self.surfaces.get_mut(&surface).unwrap();
        state.layers.retain(|l| l.latched || !l.held.is_empty());
        let before_top = before.last().map(|l| &l.set);
        let after_top = state.layers.last().map(|l| &l.set);
        if before == state.layers {
            return;
        }
        if before_top != after_top {
            self.cancel_surface(surface, CancelReason::Layer, output);
            self.close_presentation(surface, output);
        }
        self.reconcile_scene(surface, output);
    }
    pub(crate) fn interactive_instance(&self, surface: SurfaceId, instance: InstanceId) -> bool {
        self.surfaces.get(&surface).is_some_and(|s| {
            s.scene
                .items
                .iter()
                .rev()
                .find(|i| matches!(i.plane, ScenePlane::Layer(_)))
                .is_none_or(|i| i.instance == instance)
        })
    }
}
