use crate::gestures::{GestureArena, GestureEvent};
use crate::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Capture {
    pub target: HitTarget,
    pub identity: CaptureId,
    pub presentation: Option<PresentationId>,
    surface: SurfaceId,
    contact: ContactId,
    widget: WidgetSpec,
    by_column: bool,
    arena: GestureArena,
    hold_timer: Option<TimerId>,
    slider_emitted: bool,
    #[serde(with = "touchbar_model::wire_float::option_f64")]
    slider_pending: Option<f64>,
    #[serde(with = "touchbar_model::wire_float::option_f64")]
    slider_last: Option<f64>,
    scrubber_layout: Option<ScrubberLayout>,
    scrubber_start: ScrubberState,
    #[serde(with = "touchbar_model::wire_float::f32")]
    unit_px: f32,
}
impl Capture {
    fn snapshot(&self) -> CaptureSnapshot {
        CaptureSnapshot {
            id: self.identity,
            surface: self.surface,
            contact: self.contact,
            target: self.target.clone(),
        }
    }
}
impl Core {
    pub(crate) fn touch(
        &mut self,
        surface: SurfaceId,
        contact: ContactId,
        phase: TouchPhase,
        point: Point,
        output: &mut Vec<Output>,
    ) {
        if phase == TouchPhase::Cancel {
            self.cancel_contact(
                surface,
                contact,
                CancelReason::Hardware,
                Origin::Physical,
                output,
            );
            return;
        }
        if !point.x.is_finite() || !point.y.is_finite() {
            warn(output, "invalid touch coordinates");
            return;
        }
        if phase == TouchPhase::Down {
            self.touch_down(surface, contact, point, output);
            return;
        }
        let Some(mut capture) = self
            .surfaces
            .get_mut(&surface)
            .and_then(|s| s.captures.remove(&contact))
        else {
            return;
        };
        if phase == TouchPhase::Move {
            let gestures = capture.arena.motion(point, &self.config);
            match capture.widget.kind {
                WidgetKind::Slider { .. } => self.slider_value(&mut capture, point, false),
                WidgetKind::Scrubber if gestures.contains(&GestureEvent::PanChanged) => {
                    self.scrubber_move(&capture, point)
                }
                _ => {}
            }
            self.surfaces
                .get_mut(&surface)
                .unwrap()
                .captures
                .insert(contact, capture);
            return;
        }
        let previous_panel = self.surfaces[&surface]
            .presentation
            .as_ref()
            .map(|p| p.scene.id);
        output.push(Output::CaptureFinished {
            capture: capture.snapshot(),
            completion: CaptureCompletion::Up,
        });
        if let Some(timer) = capture.hold_timer.take() {
            self.timers.cancel(timer);
        }
        self.hold_gesture(&mut capture, output);
        let gestures = capture.arena.finish(point, self.now, &self.config);
        output.extend(self.keys.release_owner(KeyOwner::Contact {
            instance: capture.target.instance,
            contact,
        }));
        match capture.widget.kind {
            WidgetKind::Button => {
                if !capture.arena.held()
                    && (capture.target.rect.contains(point)
                        || capture.by_column
                            && capture.target.rect.vertical_distance(point).is_some())
                {
                    self.capture_action(
                        &capture,
                        ActionKind::Activated,
                        None,
                        Origin::Physical,
                        None,
                    );
                    self.widget_presentation(&capture, ActionKind::Activated, output);
                }
                self.capture_action(&capture, ActionKind::Released, None, Origin::Physical, None);
            }
            WidgetKind::Slider { .. } => {
                self.slider_value(&mut capture, point, true);
                self.capture_action(
                    &capture,
                    ActionKind::Activated,
                    None,
                    Origin::Physical,
                    None,
                );
                self.widget_presentation(&capture, ActionKind::Activated, output);
            }
            WidgetKind::Key { .. } if capture.target.rect.contains(point) => {
                self.widget_presentation(&capture, ActionKind::Activated, output);
            }
            WidgetKind::Scrubber
                if gestures
                    .iter()
                    .any(|g| matches!(g, GestureEvent::Tap | GestureEvent::PanEnded)) =>
            {
                self.scrubber_select(&capture, point, output);
            }
            _ => {}
        }
        self.set_pressed(&capture, false);
        // Release closes the panel that existed before Up, never one opened by Up.
        if self.surfaces[&surface]
            .presentation
            .as_ref()
            .map(|p| p.scene.id)
            == previous_panel
        {
            self.anchor_finished(surface, capture.identity, output);
        }
    }
    fn touch_down(
        &mut self,
        surface: SurfaceId,
        contact: ContactId,
        point: Point,
        output: &mut Vec<Output>,
    ) {
        if self
            .surfaces
            .get(&surface)
            .is_none_or(|s| s.mode != Mode::Display || s.captures.contains_key(&contact))
            || !self.panel_down(surface, point, output)
        {
            return;
        }
        let Some(s) = self
            .surfaces
            .get(&surface)
            .filter(|s| s.mode == Mode::Display)
        else {
            return;
        };
        let top = s
            .scene
            .items
            .iter()
            .rev()
            .find(|i| matches!(i.plane, ScenePlane::Layer(_)))
            .map(|i| (i.instance, i.plane));
        // Duplicate Down cannot silently replace a live capture.
        if s.captures.contains_key(&contact) {
            return;
        }
        let direct = s
            .hit_map
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                if s.presentation.is_some() {
                    h.plane == ScenePlane::Presentation
                } else {
                    h.plane != ScenePlane::Presentation
                        && top.is_none_or(|(instance, plane)| {
                            h.instance == instance && h.plane == plane
                        })
                }
            })
            .filter(|(_, h)| h.rect.contains(point))
            .max_by_key(|(order, h)| (h.plane, *order));
        let fallback = || {
            s.hit_map
                .iter()
                .enumerate()
                .filter(|(_, h)| {
                    if s.presentation.is_some() {
                        h.plane == ScenePlane::Presentation
                    } else {
                        h.plane != ScenePlane::Presentation
                            && top.is_none_or(|(instance, plane)| {
                                h.instance == instance && h.plane == plane
                            })
                    }
                })
                .filter_map(|(order, h)| {
                    h.rect
                        .vertical_distance(point)
                        .filter(|d| *d <= self.config.fallback_distance.to_px(s.info.px_per_mm))
                        .map(|d| (order, h, d))
                })
                .max_by(|(oa, a, da), (ob, b, db)| {
                    a.plane
                        .cmp(&b.plane)
                        .then_with(|| db.total_cmp(da))
                        .then_with(|| oa.cmp(ob))
                })
                .map(|(order, h, _)| (order, h))
        };
        let Some((_, target)) = direct.or_else(fallback) else {
            return;
        };
        let Some(instance) = self.instances.get(&target.instance.number).filter(|i| {
            i.id == target.instance && i.surface == surface && i.status == InstanceStatus::Visible
        }) else {
            return;
        };
        let Some(item) = s.scene.items.iter().find(|i| {
            i.instance == target.instance
                && (i.plane == target.plane
                    || target.plane == ScenePlane::Presentation
                        && s.presentation
                            .as_ref()
                            .is_some_and(|p| p.scene.owner == i.instance))
        }) else {
            return;
        };
        let mut widgets = vec![];
        if target.plane == ScenePlane::Presentation {
            let Some(panel) = s
                .presentation
                .as_ref()
                .filter(|p| p.scene.owner == target.instance)
            else {
                return;
            };
            for widget in &panel.scene.panel.widgets {
                widget.visit(&mut widgets);
            }
        } else {
            for widget in &item.set.widgets {
                widget.visit(&mut widgets);
            }
        }
        let Some(widget) = widgets.into_iter().find(|w| w.id == target.widget).cloned() else {
            return;
        };
        if !matches!(
            widget.kind,
            WidgetKind::Button
                | WidgetKind::Key { .. }
                | WidgetKind::Slider { .. }
                | WidgetKind::Scrubber
        ) {
            return;
        }
        if !widget_available(&instance.set, &instance.state, &widget.id) {
            return;
        }
        let scrubber_layout = s
            .scrubbers
            .iter()
            .find(|l| l.instance == target.instance && l.widget == target.widget)
            .cloned();
        if widget.kind == WidgetKind::Scrubber && scrubber_layout.is_none() {
            return;
        }
        if matches!(
            widget.kind,
            WidgetKind::Scrubber | WidgetKind::Slider { .. }
        ) && s.captures.values().any(|c| {
            c.target.instance == target.instance
                && c.target.widget == target.widget
                && c.target.record_key == target.record_key
        }) {
            return;
        }
        let scrubber_start = instance
            .interactions
            .iter()
            .find(|i| i.widget == widget.id && i.record_key == target.record_key)
            .map_or_else(ScrubberState::default, |i| i.scrubber.clone());
        let hold = if widget.kind == WidgetKind::Button
            && (widget.handlers.contains_key(&ActionKind::LongPressed)
                || widget.press_and_hold_panel.is_some())
        {
            widget.hold_ms
        } else {
            None
        };
        let arena = GestureArena::new(
            point,
            self.now,
            s.info.px_per_mm,
            hold,
            widget.kind == WidgetKind::Scrubber,
        );
        let hold_timer = arena.hold_deadline().map(|at| {
            self.timers.schedule(
                at,
                CoreTimer::Hold {
                    surface,
                    contact,
                    instance: target.instance,
                },
            )
        });
        let identity = CaptureId(self.next_capture);
        self.next_capture = self
            .next_capture
            .checked_add(1)
            .expect("capture id exhausted");
        let mut capture = Capture {
            identity,
            surface,
            contact,
            presentation: s.presentation.as_ref().map(|p| p.scene.id),
            target: target.clone(),
            widget,
            by_column: direct.is_none(),
            arena,
            hold_timer,
            slider_emitted: false,
            slider_pending: None,
            slider_last: None,
            scrubber_layout,
            scrubber_start,
            unit_px: Length::Unit(1.0).to_px(&s.info, item.units),
        };
        if let WidgetKind::Key { key } = capture.widget.kind {
            match self.keys.press(
                KeyOwner::Contact {
                    instance: capture.target.instance,
                    contact,
                },
                &s.info,
                s.mode,
                self.desktop,
                key,
            ) {
                Ok(events) => output.extend(events),
                Err(error) => {
                    warn(output, &format!("key widget: {error:?}"));
                    return;
                }
            }
        } else if capture.widget.kind != WidgetKind::Scrubber {
            self.capture_action(&capture, ActionKind::Pressed, None, Origin::Physical, None);
        }
        self.set_pressed(&capture, true);
        if matches!(capture.widget.kind, WidgetKind::Slider { .. }) {
            self.slider_value(&mut capture, point, false);
        }
        self.surfaces
            .get_mut(&surface)
            .unwrap()
            .captures
            .insert(contact, capture);
    }
    fn interaction(&mut self, capture: &Capture) -> Option<&mut WidgetInteraction> {
        let instance = self
            .instances
            .get_mut(&capture.target.instance.number)
            .filter(|i| i.id == capture.target.instance)?;
        let index = instance
            .interactions
            .iter()
            .position(|i| {
                i.widget == capture.target.widget && i.record_key == capture.target.record_key
            })
            .unwrap_or_else(|| {
                instance.interactions.push(WidgetInteraction {
                    widget: capture.target.widget.clone(),
                    record_key: capture.target.record_key.clone(),
                    pressed: false,
                    value: None,
                    scrubber: ScrubberState::default(),
                });
                instance.interactions.len() - 1
            });
        Some(&mut instance.interactions[index])
    }
    fn set_pressed(&mut self, capture: &Capture, pressed: bool) {
        let another = self.surfaces.values().any(|s| {
            s.captures.values().any(|c| {
                c.target.instance == capture.target.instance
                    && c.target.widget == capture.target.widget
                    && c.target.record_key == capture.target.record_key
            })
        });
        if let Some(state) = self.interaction(capture) {
            state.pressed = pressed || another;
        }
    }
    fn slider_value(&mut self, capture: &mut Capture, point: Point, final_value: bool) {
        let WidgetKind::Slider { minimum, maximum } = capture.widget.kind else {
            return;
        };
        let rect = capture.target.rect;
        if rect.width <= 0.0 {
            return;
        }
        let fraction =
            ((f64::from(point.x) - f64::from(rect.x)) / f64::from(rect.width)).clamp(0.0, 1.0);
        let value = minimum * (1.0 - fraction) + maximum * fraction;
        if let Some(state) = self.interaction(capture) {
            state.value = Some(value);
        }
        if final_value || !capture.slider_emitted {
            capture.slider_pending = None;
            if capture.slider_last != Some(value) {
                self.capture_action(
                    capture,
                    ActionKind::ValueChanged,
                    Some(Scalar::Number(value)),
                    Origin::Physical,
                    None,
                );
                capture.slider_last = Some(value);
                capture.slider_emitted = true;
            }
        } else {
            capture.slider_pending = Some(value);
        }
    }
    pub(crate) fn flush_slider_frame(&mut self, surface: SurfaceId) {
        let contacts: Vec<_> = self
            .surfaces
            .get(&surface)
            .map(|s| s.captures.keys().copied().collect())
            .unwrap_or_default();
        for contact in contacts {
            let mut capture = self
                .surfaces
                .get_mut(&surface)
                .unwrap()
                .captures
                .remove(&contact)
                .unwrap();
            capture.slider_emitted = false;
            if let Some(value) = capture.slider_pending.take()
                && capture.slider_last != Some(value)
            {
                self.capture_action(
                    &capture,
                    ActionKind::ValueChanged,
                    Some(Scalar::Number(value)),
                    Origin::Physical,
                    None,
                );
                capture.slider_last = Some(value);
                capture.slider_emitted = true;
            }
            self.surfaces
                .get_mut(&surface)
                .unwrap()
                .captures
                .insert(contact, capture);
        }
    }
    fn scrubber_move(&mut self, capture: &Capture, point: Point) {
        let Some(layout) = &capture.scrubber_layout else {
            return;
        };
        let offset = (capture.scrubber_start.offset_px - (point.x - capture.arena.origin.x))
            .clamp(0.0, (layout.content_width - layout.viewport.width).max(0.0));
        let highlighted = scrubber_item(layout, point, offset);
        let changed = self.interaction(capture).is_some_and(|s| {
            let changed = s.scrubber.offset_px != offset;
            s.scrubber.offset_px = offset;
            s.scrubber.highlighted = highlighted;
            changed
        });
        if changed {
            self.capture_action(
                capture,
                ActionKind::ValueChanged,
                Some(Scalar::Number(
                    f64::from(offset) / f64::from(capture.unit_px),
                )),
                Origin::Physical,
                None,
            );
        }
    }
    fn scrubber_select(&mut self, capture: &Capture, point: Point, output: &mut Vec<Output>) {
        let Some(layout) = &capture.scrubber_layout else {
            return;
        };
        let Some(state) = self.interaction(capture) else {
            return;
        };
        let selected = scrubber_item(layout, point, state.scrubber.offset_px);
        state.scrubber.highlighted = selected.clone();
        state.scrubber.selected = selected.clone();
        if let Some(record_key) = selected {
            let mut selected_capture = capture.clone();
            selected_capture.target.record_key = Some(record_key);
            self.capture_action(
                &selected_capture,
                ActionKind::Selected,
                None,
                Origin::Physical,
                None,
            );
            self.widget_presentation(&selected_capture, ActionKind::Selected, output);
        }
    }
    fn hold_gesture(&mut self, capture: &mut Capture, output: &mut Vec<Output>) {
        if capture
            .arena
            .advance(self.now, &self.config)
            .contains(&GestureEvent::LongPressed)
        {
            self.capture_action(
                capture,
                ActionKind::LongPressed,
                None,
                Origin::Physical,
                None,
            );
            self.widget_presentation(capture, ActionKind::LongPressed, output);
        }
    }
    pub(crate) fn hold_contact(
        &mut self,
        surface: SurfaceId,
        contact: ContactId,
        instance: InstanceId,
        timer: TimerId,
        output: &mut Vec<Output>,
    ) {
        let Some(mut capture) = self
            .surfaces
            .get_mut(&surface)
            .and_then(|s| s.captures.remove(&contact))
        else {
            return;
        };
        if capture.target.instance == instance && capture.hold_timer == Some(timer) {
            capture.hold_timer = None;
            // The live anchor must be present while accepting a transient open.
            self.surfaces
                .get_mut(&surface)
                .unwrap()
                .captures
                .insert(contact, capture.clone());
            self.hold_gesture(&mut capture, output);
            self.surfaces
                .get_mut(&surface)
                .unwrap()
                .captures
                .remove(&contact);
        }
        self.surfaces
            .get_mut(&surface)
            .unwrap()
            .captures
            .insert(contact, capture);
    }
    fn capture_action(
        &mut self,
        capture: &Capture,
        kind: ActionKind,
        value: Option<Scalar>,
        origin: Origin,
        cancel_reason: Option<CancelReason>,
    ) {
        if let Some(action) = capture.widget.handlers.get(&kind)
            && let Some(instance) = self
                .instances
                .get_mut(&capture.target.instance.number)
                .filter(|i| i.id == capture.target.instance)
        {
            instance.enqueue_physical(
                LogicEvent::Action(ActionEvent {
                    action: action.clone(),
                    kind,
                    value,
                    record_key: capture.target.record_key.clone(),
                    origin,
                    cancel_reason,
                }),
                (kind != ActionKind::Cancelled).then(|| crate::presentations::PhysicalContext {
                    surface: capture.surface,
                    contact: capture.contact,
                    capture: capture.identity,
                    target: capture.target.clone(),
                    presentation: capture.presentation,
                }),
            );
        }
    }
    fn widget_presentation(
        &mut self,
        capture: &Capture,
        kind: ActionKind,
        output: &mut Vec<Output>,
    ) {
        let has_callback = !matches!(capture.widget.kind, WidgetKind::Key { .. })
            && capture.widget.handlers.contains_key(&kind)
            && self
                .instances
                .get(&capture.target.instance.number)
                .is_some_and(|i| i.set.logic.is_some());
        let (panel, lifecycle) = match kind {
            ActionKind::Activated => (
                capture.widget.expanded_panel.clone(),
                PresentationLifecycle::Persistent,
            ),
            ActionKind::LongPressed => (
                capture.widget.press_and_hold_panel.clone(),
                PresentationLifecycle::Transient,
            ),
            _ => (None, PresentationLifecycle::Persistent),
        };
        if let Some(panel) = panel {
            self.open_presentation(
                capture.target.instance,
                crate::presentations::PhysicalContext {
                    surface: capture.surface,
                    contact: capture.contact,
                    capture: capture.identity,
                    target: capture.target.clone(),
                    presentation: capture.presentation,
                },
                panel,
                touchbar_model::Placement::Anchored,
                lifecycle,
                output,
            );
        }
        if !has_callback {
            self.dismiss_selection(capture.surface, capture.presentation, kind, output);
        }
    }
    pub(crate) fn cancel_contact(
        &mut self,
        surface: SurfaceId,
        contact: ContactId,
        reason: CancelReason,
        origin: Origin,
        output: &mut Vec<Output>,
    ) {
        let Some(capture) = self
            .surfaces
            .get_mut(&surface)
            .and_then(|s| s.captures.remove(&contact))
        else {
            return;
        };
        if let Some(instance) = self
            .instances
            .get_mut(&capture.target.instance.number)
            .filter(|i| i.id == capture.target.instance)
        {
            instance.invalidate_capture_context(capture.identity);
        }
        output.push(Output::CaptureFinished {
            capture: capture.snapshot(),
            completion: CaptureCompletion::Cancel(reason),
        });
        if let Some(timer) = capture.hold_timer {
            self.timers.cancel(timer);
        }
        output.extend(self.keys.release_owner(KeyOwner::Contact {
            instance: capture.target.instance,
            contact,
        }));
        if capture.widget.kind == WidgetKind::Scrubber
            && let Some(state) = self.interaction(&capture)
        {
            state.scrubber = capture.scrubber_start.clone();
        }
        self.set_pressed(&capture, false);
        self.capture_action(&capture, ActionKind::Cancelled, None, origin, Some(reason));
        // Deliver cleanup while still alive and visible when the serialized call slot is free.
        if let Some(instance) = self
            .instances
            .get_mut(&capture.target.instance.number)
            .filter(|i| i.id == capture.target.instance)
        {
            output.extend(instance.dispatch(&mut self.next_call));
        }
        self.anchor_finished(surface, capture.identity, output);
    }
    pub(crate) fn cancel_surface(
        &mut self,
        surface: SurfaceId,
        reason: CancelReason,
        output: &mut Vec<Output>,
    ) {
        let contacts: Vec<_> = self
            .surfaces
            .get(&surface)
            .map(|s| s.captures.keys().copied().collect())
            .unwrap_or_default();
        for contact in contacts {
            self.cancel_contact(surface, contact, reason, Origin::Synthetic, output);
        }
    }
    pub(crate) fn cancel_instance(
        &mut self,
        instance: InstanceId,
        reason: CancelReason,
        output: &mut Vec<Output>,
    ) {
        let contacts: Vec<_> = self
            .surfaces
            .iter()
            .flat_map(|(surface, s)| {
                s.captures
                    .iter()
                    .filter(move |(_, c)| c.target.instance == instance)
                    .map(move |(contact, _)| (*surface, *contact))
            })
            .collect();
        for (surface, contact) in contacts {
            self.cancel_contact(surface, contact, reason, Origin::Synthetic, output);
        }
    }
    pub(crate) fn profile_guards(&self, surface: SurfaceId) -> usize {
        self.surfaces.get(&surface).map_or(0, |s| s.captures.len())
            + self
                .instances
                .values()
                .filter(|i| i.surface == surface && i.has_physical_work())
                .count()
    }
    pub(crate) fn finish_pending_profiles(&mut self, output: &mut Vec<Output>) {
        let surfaces: Vec<_> = self.surfaces.keys().copied().collect();
        for id in surfaces {
            let guards = self.profile_guards(id);
            if self
                .surfaces
                .get_mut(&id)
                .unwrap()
                .profile
                .contacts_finished(guards)
            {
                self.sync_profile_timer(id);
                self.reconcile_scene(id, output);
            }
        }
    }
    pub(crate) fn synchronize_captures(&mut self, output: &mut Vec<Output>) {
        let invalid: Vec<_> = self
            .surfaces
            .iter()
            .flat_map(|(surface, s)| {
                s.captures.iter().filter_map(|(contact, c)| {
                    let instance = self
                        .instances
                        .get(&c.target.instance.number)
                        .filter(|i| i.id == c.target.instance);
                    let reason = match instance {
                        None => Some(CancelReason::Layout),
                        Some(i) if i.status == InstanceStatus::Faulted => Some(CancelReason::Fault),
                        Some(i)
                            if i.status != InstanceStatus::Visible
                                || !widget_available(&i.set, &i.state, &c.target.widget) =>
                        {
                            Some(CancelReason::Hidden)
                        }
                        Some(_)
                            if !s
                                .scene
                                .items
                                .iter()
                                .any(|i| i.instance == c.target.instance) =>
                        {
                            Some(CancelReason::Hidden)
                        }
                        Some(_)
                            if !((s.presentation.is_some() || s.layout_presentation.is_some())
                                && c.presentation.is_none())
                                && !s.hit_map.iter().any(|h| {
                                    h.instance == c.target.instance
                                        && h.widget == c.target.widget
                                        && h.record_key == c.target.record_key
                                        && (h.plane == c.target.plane
                                            || matches!(
                                                (h.plane, c.target.plane),
                                                (ScenePlane::Layer(_), ScenePlane::Layer(_))
                                            ))
                                }) =>
                        {
                            Some(CancelReason::Layout)
                        }
                        Some(_)
                            if !((s.presentation.is_some() || s.layout_presentation.is_some())
                                && c.presentation.is_none())
                                && c.scrubber_layout
                                    .as_ref()
                                    .is_some_and(|layout| !s.scrubbers.contains(layout)) =>
                        {
                            Some(CancelReason::Layout)
                        }
                        _ => None,
                    };
                    reason.map(|reason| (*surface, *contact, reason))
                })
            })
            .collect();
        for (surface, contact, reason) in invalid {
            self.cancel_contact(surface, contact, reason, Origin::Synthetic, output);
        }
    }
}
fn scrubber_item(layout: &ScrubberLayout, point: Point, offset: f32) -> Option<String> {
    if !layout.viewport.contains(point) {
        return None;
    }
    let content = Point {
        x: point.x - layout.viewport.x + offset,
        y: point.y - layout.viewport.y,
    };
    layout
        .items
        .iter()
        .find(|i| i.rect.contains(content))
        .map(|i| i.record_key.clone())
}
pub(crate) fn valid_rect(rect: Rect) -> bool {
    [
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        rect.x + rect.width,
        rect.y + rect.height,
    ]
    .iter()
    .all(|n| n.is_finite())
        && rect.width >= 0.0
        && rect.height >= 0.0
}
pub(crate) fn valid_scrubbers(layouts: &[ScrubberLayout], max_records: usize) -> bool {
    layouts.iter().enumerate().all(|(index, l)| {
        valid_rect(l.viewport)
            && l.viewport.width > 0.0
            && l.viewport.height > 0.0
            && l.content_width.is_finite()
            && l.content_width >= 0.0
            && l.items.len() <= max_records
            && !layouts[..index]
                .iter()
                .any(|prev| prev.instance == l.instance && prev.widget == l.widget)
            && l.items.iter().enumerate().all(|(index, item)| {
                valid_rect(item.rect)
                    && item.rect.x >= 0.0
                    && item.rect.y >= 0.0
                    && item.rect.x + item.rect.width <= l.content_width
                    && !item.record_key.is_empty()
                    && !l.items[..index]
                        .iter()
                        .any(|prev| prev.record_key == item.record_key)
            })
    })
}
pub(crate) fn widget_available(set: &SetSpec, state: &StatePatch, id: &WidgetId) -> bool {
    fn in_widgets(
        widgets: &[WidgetSpec],
        state: &StatePatch,
        id: &WidgetId,
        ancestors: bool,
    ) -> Option<bool> {
        for widget in widgets {
            let available = ancestors
                && ![StateBinding::Enabled, StateBinding::Visible]
                    .iter()
                    .any(|binding| {
                        widget.state_bindings.get(binding).is_some_and(|name| {
                            state.iter().any(|(n, v)| {
                                n == name && *v == StateValue::Scalar(Scalar::Boolean(false))
                            })
                        })
                    });
            if widget.id == *id {
                return Some(available);
            }
            if let Some(result) = in_widgets(&widget.children, state, id, available) {
                return Some(result);
            }
        }
        None
    }
    in_widgets(&set.widgets, state, id, true)
        .or_else(|| {
            set.panels
                .iter()
                .find_map(|p| in_widgets(&p.widgets, state, id, true))
        })
        .unwrap_or(false)
}
