use crate::{InstanceStatus, Scene, SceneItem};
use serde::{Deserialize, Serialize};
use touchbar_model::*;

/// A placement's logic identity, independent of its rendering units and profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlacementIdentity {
    slot: SlotId,
    set: SetRef,
    config: String,
    owner_revision: String,
    provider: Option<(PluginId, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Placement {
    pub instance: InstanceId,
    pub identity: PlacementIdentity,
}
pub(crate) enum PlacementChange {
    Create { instance: InstanceId, set: SetSpec },
    Remove { instance: InstanceId },
}
pub(crate) struct Composition {
    pub scene: Scene,
    pub placements: Vec<Placement>,
    pub changes: Vec<PlacementChange>,
}
pub(crate) fn compose(
    catalog: &Catalog,
    surface: &SurfaceInfo,
    profile: Option<&ProfileRef>,
    previous: &Scene,
    placements: &[Placement],
    next_instance: &mut u64,
) -> Composition {
    let mut scene = Scene {
        profile: profile.cloned(),
        ..Scene::default()
    };
    let mut retained = vec![];
    let mut creates = vec![];
    if let Some(profile) =
        profile.and_then(|reference| catalog.profiles.iter().find(|p| &p.reference == reference))
    {
        for slot in &profile.slots {
            let Some(set) = slot
                .candidates
                .iter()
                .find(|r| catalog.set_available(r))
                .and_then(|reference| catalog.sets.iter().find(|s| &s.reference == reference))
            else {
                continue;
            };
            let identity = PlacementIdentity {
                slot: slot.id.clone(),
                set: set.reference.clone(),
                config: set.logic_config.clone(),
                owner_revision: catalog
                    .plugin(&set.reference.plugin)
                    .expect("available set owner")
                    .revision
                    .clone(),
                provider: set.logic.as_ref().map(|id| {
                    (
                        id.clone(),
                        catalog
                            .plugin(id)
                            .expect("available logic provider")
                            .revision
                            .clone(),
                    )
                }),
            };
            let old = placements.iter().find(|p| p.identity == identity);
            let instance = if let Some(old) = old {
                old.instance
            } else {
                let instance = InstanceId {
                    number: *next_instance,
                    generation: Generation(1),
                };
                *next_instance = next_instance.checked_add(1).expect("instance id exhausted");
                creates.push(PlacementChange::Create {
                    instance,
                    set: set.clone(),
                });
                instance
            };
            let state = previous
                .items
                .iter()
                .find(|i| i.instance == instance)
                .map_or_else(StatePatch::default, |i| i.state.clone());
            scene.items.push(SceneItem {
                instance,
                slot: slot.id.clone(),
                set: set.clone(),
                state,
                interactions: previous
                    .items
                    .iter()
                    .find(|i| i.instance == instance)
                    .map_or_else(Vec::new, |i| i.interactions.clone()),
                plane: ScenePlane::Profile,
                units: UnitSize::resolve(surface, set.units, profile.units),
                status: previous
                    .items
                    .iter()
                    .find(|i| i.instance == instance)
                    .map_or(
                        if set.logic.is_some() {
                            InstanceStatus::Loading
                        } else {
                            InstanceStatus::Visible
                        },
                        |i| i.status,
                    ),
            });
            retained.push(Placement { instance, identity });
        }
    }
    let mut changes: Vec<_> = previous
        .items
        .iter()
        .filter(|i| !retained.iter().any(|p| p.instance == i.instance))
        .map(|i| PlacementChange::Remove {
            instance: i.instance,
        })
        .collect();
    scene.presentation = previous.presentation.clone();
    changes.extend(creates);
    Composition {
        scene,
        placements: retained,
        changes,
    }
}
