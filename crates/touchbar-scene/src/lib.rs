//! Pure scene projection into UI primitives and core Layout input.
mod builder;
mod canvas;
mod composition;
mod renderer;
mod sizing;
pub use renderer::{CacheStats, SceneRenderer};
use std::{collections::BTreeMap, sync::Arc};
use touchbar_core::{PresentationLayout, Representations, ScrubberLayout, SlotBounds};
use touchbar_model::{HitMap, InstanceId, ProfileRef, SetRef, SlotId};
use touchbar_profiles::ResolvedProfile;
use touchbar_sets::SetFile;
use touchbar_ui::{DisplayList, Image};

#[derive(Clone, Debug)]
pub struct SetDocument {
    pub file: SetFile,
    pub assets: BTreeMap<String, Image>,
}
#[derive(Clone, Debug, Default)]
pub struct Documents {
    pub sets: BTreeMap<SetRef, Arc<SetDocument>>,
    pub profiles: BTreeMap<ProfileRef, ResolvedProfile>,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrameLayout {
    pub hit_map: HitMap,
    pub representations: Representations,
    pub scrubbers: Vec<ScrubberLayout>,
    pub slots: Vec<SlotBounds>,
    pub presentation: Option<PresentationLayout>,
    pub named_slots: Vec<NamedSlot>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NamedSlot {
    pub slot: SlotId,
    pub instance: Option<InstanceId>,
    pub bounds: Option<touchbar_model::Rect>,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub display_list: DisplayList,
    pub layout: FrameLayout,
    pub animate: bool,
    pub faults: Vec<(InstanceId, String)>,
    pub warnings: Vec<String>,
}
