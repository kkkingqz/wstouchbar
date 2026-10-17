//! Device selection independent of PipeWire object lifetimes.
use std::collections::BTreeMap;
use touchbar_model::*;
#[derive(Clone, Debug)]
pub struct Volume {
    pub channels: Vec<f32>,
    pub muted: bool,
}
impl Volume {
    pub fn level(&self) -> f64 {
        from_linear(self.channels.first().copied().unwrap_or(0.0))
    }
}
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub description: String,
    pub device: Option<(u32, i32)>,
    pub volume: Option<Volume>,
}
#[derive(Clone, Debug)]
pub struct Route {
    pub profile_device: i32,
    pub volume: Volume,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Node(u32),
    Route {
        device: u32,
        index: i32,
        profile_device: i32,
    },
}
#[derive(Default)]
pub struct Graph {
    pub nodes: BTreeMap<u32, Node>,
    pub routes: BTreeMap<(u32, i32), Route>,
    defaults: [Option<String>; 2],
}
pub fn target_index(target: AudioTarget) -> usize {
    match target {
        AudioTarget::DefaultSink => 0,
        AudioTarget::DefaultSource => 1,
    }
}
pub fn unavailable() -> AudioState {
    AudioState {
        available: false,
        volume: 0.0,
        muted: false,
        name: String::new(),
    }
}
pub fn from_linear(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value.max(0.0)).cbrt()
    } else {
        0.0
    }
}
// SPA stores amplitudes as f32; validated values are at most 3.375.
#[allow(clippy::cast_possible_truncation)]
pub fn to_linear(value: f64) -> Result<f32, EffectError> {
    if !value.is_finite() || !(0.0..=1.5).contains(&value) {
        return Err(EffectError::Failed(
            "audio volume must be in 0.0..=1.5".into(),
        ));
    }
    Ok(value.powi(3) as f32)
}
impl Graph {
    pub fn metadata(&mut self, key: &str, value: Option<&str>) {
        let index = match key {
            "default.audio.sink" => 0,
            "default.audio.source" => 1,
            _ => return,
        };
        self.defaults[index] = value
            .filter(|v| v.len() <= 4096)
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .and_then(|v| {
                v.get("name")
                    .and_then(|v| v.as_str())
                    .filter(|n| n.len() <= 1024)
                    .map(str::to_owned)
            });
    }
    pub fn remove(&mut self, id: u32) {
        self.nodes.remove(&id);
        self.routes.retain(|(device, _), _| *device != id);
    }
    pub fn control(&self, target: AudioTarget) -> Option<(Control, &Volume)> {
        let name = self.defaults[target_index(target)].as_ref()?;
        let (id, node) = self.nodes.iter().find(|(_, node)| &node.name == name)?;
        if let Some((device, profile_device)) = node.device
            && let Some(((device, index), route)) = self
                .routes
                .iter()
                .find(|((d, _), r)| *d == device && r.profile_device == profile_device)
        {
            return Some((
                Control::Route {
                    device: *device,
                    index: *index,
                    profile_device,
                },
                &route.volume,
            ));
        }
        Some((Control::Node(*id), node.volume.as_ref()?))
    }
    pub fn state(&self, target: AudioTarget) -> AudioState {
        let Some((_, volume)) = self.control(target) else {
            return unavailable();
        };
        let name = self.defaults[target_index(target)]
            .as_ref()
            .expect("selected default");
        let node = self
            .nodes
            .values()
            .find(|n| &n.name == name)
            .expect("selected node");
        AudioState {
            available: true,
            volume: volume.level(),
            muted: volume.muted,
            name: node.description.clone(),
        }
    }
}
