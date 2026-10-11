use crate::*;
use touchbar_model::Placement;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PhysicalContext {
    pub surface: SurfaceId,
    pub contact: ContactId,
    pub capture: CaptureId,
    pub target: HitTarget,
    pub presentation: Option<PresentationId>,
}
pub(crate) fn valid_bounds(
    slots: &[SlotBounds],
    presentation: Option<PresentationLayout>,
    info: &SurfaceInfo,
) -> bool {
    let within = |rect: Rect| {
        touches::valid_rect(rect)
            && rect.x >= 0.0
            && rect.y >= 0.0
            && rect.x + rect.width <= info.width_px as f32
            && rect.y + rect.height <= info.height_px as f32
    };
    slots.iter().enumerate().all(|(index, slot)| {
        within(slot.rect)
            && !slots[..index]
                .iter()
                .any(|prev| prev.instance == slot.instance)
    }) && presentation.is_none_or(|p| within(p.bounds))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Presentation {
    pub scene: PresentationScene,
    pub source: PhysicalContext,
    pub bounds: Option<Rect>,
    pub timer: Option<(TimerId, Timestamp)>,
}
impl Core {
    pub(crate) fn presentation_reply(
        &mut self,
        owner: InstanceId,
        completed: &instances::CompletedCall,
        command: Option<PresentationCommand>,
        output: &mut Vec<Output>,
    ) {
        if let Some(command) = command {
            match command {
                PresentationCommand::Close => {
                    if let Some(surface) = self
                        .instances
                        .get(&owner.number)
                        .filter(|i| i.id == owner)
                        .map(|i| i.surface)
                        && self.surfaces.get(&surface).is_some_and(|s| {
                            s.presentation
                                .as_ref()
                                .is_some_and(|p| p.scene.owner == owner)
                        })
                    {
                        self.close_presentation(surface, output);
                    }
                }
                PresentationCommand::Open {
                    panel,
                    placement,
                    lifecycle,
                } => {
                    if matches!(
                        completed.invocation,
                        LogicCall::Handle(LogicEvent::Action(ActionEvent {
                            origin: Origin::Physical,
                            ..
                        }))
                    ) && let Some(source) = &completed.physical
                    {
                        self.open_presentation(
                            owner,
                            source.clone(),
                            panel,
                            placement,
                            lifecycle,
                            output,
                        );
                    } else {
                        warn(output, "presentation.open requires a physical action");
                    }
                }
            }
        }
        if let Some(source) = &completed.physical
            && let LogicCall::Handle(LogicEvent::Action(event)) = &completed.invocation
        {
            self.dismiss_selection(source.surface, source.presentation, event.kind, output);
        }
    }
    pub(crate) fn dismiss_selection(
        &mut self,
        surface: SurfaceId,
        panel: Option<PresentationId>,
        kind: ActionKind,
        output: &mut Vec<Output>,
    ) {
        if matches!(kind, ActionKind::Activated | ActionKind::Selected)
            && self.surfaces.get(&surface).is_some_and(|s| {
                s.presentation.as_ref().is_some_and(|p| {
                    Some(p.scene.id) == panel
                        && (p.scene.panel.dismiss_on_selection || p.scene.panel.dismissal.selection)
                })
            })
        {
            self.close_presentation(surface, output);
        }
    }
    pub(crate) fn open_presentation(
        &mut self,
        owner: InstanceId,
        source: PhysicalContext,
        panel: PanelId,
        placement: Placement,
        lifecycle: PresentationLifecycle,
        output: &mut Vec<Output>,
    ) {
        if !self.interactive_instance(source.surface, owner) {
            return;
        }
        let Some(instance) = self.instances.get(&owner.number).filter(|i| {
            i.id == owner && i.surface == source.surface && i.status == InstanceStatus::Visible
        }) else {
            return;
        };
        let Some(panel) = instance.set.panels.iter().find(|p| p.id == panel).cloned() else {
            warn(output, "unknown panel");
            return;
        };
        let Some(surface) = self
            .surfaces
            .get(&source.surface)
            .filter(|s| s.mode == Mode::Display)
        else {
            return;
        };
        if source.presentation.is_some()
            && source.presentation != surface.presentation.as_ref().map(|p| p.scene.id)
        {
            warn(output, "panel source has closed or been replaced");
            return;
        }
        let alive = surface
            .captures
            .get(&source.contact)
            .is_some_and(|c| c.identity == source.capture);
        if lifecycle == PresentationLifecycle::Transient && !alive {
            warn(output, "transient panel's original capture has ended");
            return;
        }
        let region = match placement {
            Placement::Full => Rect {
                x: 0.0,
                y: 0.0,
                width: surface.info.width_px as f32,
                height: surface.info.height_px as f32,
            },
            Placement::Region | Placement::Anchored => {
                let Some(bounds) = surface.slots.iter().find(|b| b.instance == owner) else {
                    warn(output, "panel requires owner slot bounds");
                    return;
                };
                bounds.rect
            }
        };
        self.close_presentation(source.surface, output);
        let id = PresentationId(self.next_presentation);
        self.next_presentation = self
            .next_presentation
            .checked_add(1)
            .expect("presentation id exhausted");
        let scene = PresentationScene {
            id,
            owner,
            panel,
            placement,
            lifecycle,
            anchor: source.target.rect,
            region,
        };
        let timer = scene.panel.dismissal.timeout_ms.map(|ms| {
            let at = self.now.after(ms);
            (
                self.timers.schedule(
                    at,
                    CoreTimer::Presentation {
                        surface: source.surface,
                        id,
                    },
                ),
                at,
            )
        });
        let state = self.surfaces.get_mut(&source.surface).unwrap();
        state.presentation = Some(Presentation {
            scene: scene.clone(),
            source,
            bounds: None,
            timer,
        });
        state.scene.presentation = Some(scene);
        state.dirty = true;
    }
    pub(crate) fn close_presentation(&mut self, surface: SurfaceId, output: &mut Vec<Output>) {
        let Some(panel) = self
            .surfaces
            .get_mut(&surface)
            .and_then(|s| s.presentation.take())
        else {
            return;
        };
        if let Some((timer, _)) = panel.timer {
            self.timers.cancel(timer);
        }
        let contacts: Vec<_> = self
            .surfaces
            .get(&surface)
            .unwrap()
            .captures
            .iter()
            .filter(|(_, c)| c.presentation == Some(panel.scene.id))
            .map(|(contact, _)| *contact)
            .collect();
        for contact in contacts {
            self.cancel_contact(
                surface,
                contact,
                CancelReason::Presentation,
                Origin::Synthetic,
                output,
            );
        }
        let state = self.surfaces.get_mut(&surface).unwrap();
        state.scene.presentation = None;
        state.dirty = true;
        if let Some(owner) = self
            .instances
            .get_mut(&panel.scene.owner.number)
            .filter(|i| i.id == panel.scene.owner)
        {
            owner.enqueue(LogicEvent::PresentationClosed(panel.scene.panel.id));
        }
    }
    pub(crate) fn anchor_finished(
        &mut self,
        surface: SurfaceId,
        identity: CaptureId,
        output: &mut Vec<Output>,
    ) {
        if self.surfaces.get(&surface).is_some_and(|s| {
            s.presentation.as_ref().is_some_and(|p| {
                p.source.capture == identity
                    && (p.scene.lifecycle == PresentationLifecycle::Transient
                        || p.scene.panel.dismissal.release)
            })
        }) {
            self.close_presentation(surface, output);
        }
    }
    pub(crate) fn panel_down(
        &mut self,
        surface: SurfaceId,
        point: Point,
        output: &mut Vec<Output>,
    ) -> bool {
        let Some(panel) = self
            .surfaces
            .get(&surface)
            .and_then(|s| s.presentation.as_ref())
        else {
            return true;
        };
        let Some(bounds) = panel.bounds else {
            return false;
        };
        if !bounds.contains(point) {
            if panel.scene.panel.dismissal.outside_press {
                self.close_presentation(surface, output);
            }
            // The dismissing press is consumed; it never activates an obscured control.
            return false;
        }
        true
    }
    pub(crate) fn synchronize_presentations(&mut self, output: &mut Vec<Output>) {
        let invalid: Vec<_> = self
            .surfaces
            .iter()
            .filter_map(|(id, s)| {
                s.presentation.as_ref().and_then(|p| {
                    let valid = s.mode == Mode::Display
                        && self.instances.get(&p.scene.owner.number).is_some_and(|i| {
                            i.id == p.scene.owner
                                && i.status == InstanceStatus::Visible
                                && touches::widget_available(
                                    &i.set,
                                    &i.state,
                                    &p.source.target.widget,
                                )
                        })
                        && s.scene.items.iter().any(|i| i.instance == p.scene.owner);
                    (!valid).then_some(*id)
                })
            })
            .collect();
        for id in invalid {
            self.close_presentation(id, output);
        }
    }
}
