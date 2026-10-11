use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginSpec {
    pub id: PluginId,
    /// Stable content identity supplied by the loader, not the manifest version.
    pub revision: String,
    pub enabled: bool,
    pub has_logic: bool,
    pub requires: Vec<PluginId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WidgetKind {
    Button,
    Slider {
        #[serde(with = "crate::wire_float::f64")]
        minimum: f64,
        #[serde(with = "crate::wire_float::f64")]
        maximum: f64,
    },
    Scrubber,
    Key {
        key: SystemKey,
    },
    Canvas,
    Noninteractive,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StateBinding {
    Text,
    Value,
    Enabled,
    Selected,
    Visible,
    Symbol,
    Canvas,
    Records,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetSpec {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub children: Vec<WidgetSpec>,
    pub width: Option<Length>,
    pub hold_ms: Option<u64>,
    pub handlers: BTreeMap<ActionKind, String>,
    pub state_bindings: BTreeMap<StateBinding, String>,
    pub expanded_panel: Option<PanelId>,
    pub press_and_hold_panel: Option<PanelId>,
}
impl WidgetSpec {
    pub fn new(id: WidgetId, kind: WidgetKind) -> Self {
        Self {
            id,
            kind,
            children: vec![],
            width: None,
            hold_ms: None,
            handlers: BTreeMap::new(),
            state_bindings: BTreeMap::new(),
            expanded_panel: None,
            press_and_hold_panel: None,
        }
    }
    pub fn visit<'a>(&'a self, out: &mut Vec<&'a WidgetSpec>) {
        out.push(self);
        for child in &self.children {
            child.visit(out);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DismissalPolicy {
    pub release: bool,
    pub selection: bool,
    pub outside_press: bool,
    pub timeout_ms: Option<u64>,
}
impl Default for DismissalPolicy {
    fn default() -> Self {
        Self {
            release: true,
            selection: false,
            outside_press: true,
            timeout_ms: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSpec {
    pub id: PanelId,
    pub widgets: Vec<WidgetSpec>,
    pub dismiss_on_selection: bool,
    pub principal_item: Option<WidgetId>,
    pub dismissal: DismissalPolicy,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetSpec {
    pub reference: SetRef,
    pub widgets: Vec<WidgetSpec>,
    pub panels: Vec<PanelSpec>,
    pub logic: Option<PluginId>,
    pub logic_config: String,
    pub persist: Vec<String>,
    pub units: Option<UnitSize>,
}
impl SetSpec {
    pub fn default_for(reference: SetRef) -> Self {
        Self {
            reference,
            widgets: vec![],
            panels: vec![],
            logic: None,
            logic_config: String::new(),
            persist: vec![],
            units: None,
        }
    }
    pub fn all_widgets(&self) -> Vec<&WidgetSpec> {
        let mut out = vec![];
        for w in &self.widgets {
            w.visit(&mut out);
        }
        for panel in &self.panels {
            for w in &panel.widgets {
                w.visit(&mut out);
            }
        }
        out
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProfileTarget {
    App(String),
    Category(String),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotSpec {
    pub id: SlotId,
    pub candidates: Vec<SetRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileSpec {
    pub reference: ProfileRef,
    pub for_target: Option<ProfileTarget>,
    pub when: Option<Condition>,
    pub priority: i32,
    pub default: bool,
    pub locked: bool,
    pub slots: Vec<SlotSpec>,
    pub units: Option<UnitSize>,
}
impl ProfileSpec {
    pub fn default_for(reference: ProfileRef) -> Self {
        Self {
            reference,
            for_target: None,
            when: None,
            priority: 0,
            default: false,
            locked: false,
            slots: vec![],
            units: None,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub plugins: Vec<PluginSpec>,
    pub sets: Vec<SetSpec>,
    pub profiles: Vec<ProfileSpec>,
    pub bindings: Vec<Binding<LayerAction>>,
    #[serde(default)]
    pub profile_for: Vec<ProfileOverride>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileOverride {
    pub surface: Option<SurfaceName>,
    pub target: ProfileTarget,
    pub profile: ProfileRef,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogIssueCode {
    DuplicatePlugin,
    DuplicateSet,
    DuplicateProfile,
    DuplicateSlot,
    UnknownPlugin,
    UnknownSet,
    UnknownLogic,
    ProfileConflict,
    DuplicateWidget,
    DuplicatePanel,
    UnknownPanel,
    CanvasBindingRequired,
    InvalidCondition,
    InvalidLength,
    InvalidWidget,
    InvalidProfile,
    DuplicateBinding,
    InvalidBinding,
    DuplicatePersist,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogIssue {
    pub code: CatalogIssueCode,
    pub location: String,
    pub detail: String,
}
impl Catalog {
    pub fn validate(&self) -> Vec<CatalogIssue> {
        let mut issues = vec![];
        duplicates(
            self.plugins.iter().map(|p| &p.id),
            CatalogIssueCode::DuplicatePlugin,
            "plugins",
            &mut issues,
        );
        duplicates(
            self.sets.iter().map(|s| &s.reference),
            CatalogIssueCode::DuplicateSet,
            "sets",
            &mut issues,
        );
        duplicates(
            self.profiles.iter().map(|p| &p.reference),
            CatalogIssueCode::DuplicateProfile,
            "profiles",
            &mut issues,
        );
        for set in &self.sets {
            let location = set.reference.to_string();
            if self.plugin(&set.reference.plugin).is_none() {
                issue(
                    &mut issues,
                    CatalogIssueCode::UnknownPlugin,
                    &location,
                    "set owner is missing",
                );
            }
            if let Some(logic) = &set.logic
                && !self.plugin(logic).is_some_and(|p| p.has_logic)
            {
                issue(
                    &mut issues,
                    CatalogIssueCode::UnknownLogic,
                    &location,
                    "logic provider is missing or has no module",
                );
            }
            check_units(set.units, &location, &mut issues);
            duplicates(
                set.persist.iter(),
                CatalogIssueCode::DuplicatePersist,
                &location,
                &mut issues,
            );
            duplicates(
                set.panels.iter().map(|p| &p.id),
                CatalogIssueCode::DuplicatePanel,
                &location,
                &mut issues,
            );
            let widgets = set.all_widgets();
            if set.logic.is_none()
                && (!set.persist.is_empty()
                    || widgets
                        .iter()
                        .any(|w| !w.handlers.is_empty() || !w.state_bindings.is_empty()))
            {
                issue(
                    &mut issues,
                    CatalogIssueCode::InvalidWidget,
                    &location,
                    "logic handlers, state bindings and persist require a logic provider",
                );
            }
            duplicates(
                widgets.iter().map(|w| &w.id),
                CatalogIssueCode::DuplicateWidget,
                &location,
                &mut issues,
            );
            for widget in widgets {
                let location = format!("{location}/{}", widget.id);
                if widget
                    .width
                    .is_some_and(|length| length.validate().is_err())
                {
                    issue(
                        &mut issues,
                        CatalogIssueCode::InvalidLength,
                        &location,
                        "invalid widget width",
                    );
                }
                if widget.hold_ms == Some(0) {
                    issue(
                        &mut issues,
                        CatalogIssueCode::InvalidWidget,
                        &location,
                        "hold duration must be positive",
                    );
                }
                if let WidgetKind::Slider { minimum, maximum } = widget.kind
                    && (!minimum.is_finite() || !maximum.is_finite() || minimum >= maximum)
                {
                    issue(
                        &mut issues,
                        CatalogIssueCode::InvalidWidget,
                        &location,
                        "invalid slider range",
                    );
                }
                if widget.kind == WidgetKind::Canvas
                    && !widget.state_bindings.contains_key(&StateBinding::Canvas)
                {
                    issue(
                        &mut issues,
                        CatalogIssueCode::CanvasBindingRequired,
                        &location,
                        "canvas requires canvas_from",
                    );
                }
                for panel in [&widget.expanded_panel, &widget.press_and_hold_panel]
                    .into_iter()
                    .flatten()
                {
                    if !set.panels.iter().any(|p| &p.id == panel) {
                        issue(
                            &mut issues,
                            CatalogIssueCode::UnknownPanel,
                            &location,
                            &panel.to_string(),
                        );
                    }
                }
            }
            for panel in &set.panels {
                let mut widgets = vec![];
                for widget in &panel.widgets {
                    widget.visit(&mut widgets);
                }
                if panel
                    .principal_item
                    .as_ref()
                    .is_some_and(|id| !widgets.iter().any(|w| &w.id == id))
                    || panel.dismissal.timeout_ms == Some(0)
                {
                    issue(
                        &mut issues,
                        CatalogIssueCode::InvalidWidget,
                        &location,
                        "invalid panel principal item or timeout",
                    );
                }
            }
        }
        for profile in &self.profiles {
            let location = profile.reference.to_string();
            let owner = self.plugin(&profile.reference.plugin);
            if owner.is_none() {
                issue(
                    &mut issues,
                    CatalogIssueCode::UnknownPlugin,
                    &location,
                    "profile owner is missing",
                );
            }
            if u8::from(profile.default)
                + u8::from(profile.locked)
                + u8::from(profile.for_target.is_some())
                > 1
            {
                issue(
                    &mut issues,
                    CatalogIssueCode::InvalidProfile,
                    &location,
                    "profile has multiple selection targets",
                );
            }
            check_condition(profile.when.as_ref(), &location, &mut issues);
            check_units(profile.units, &location, &mut issues);
            duplicates(
                profile.slots.iter().map(|slot| &slot.id),
                CatalogIssueCode::DuplicateSlot,
                &location,
                &mut issues,
            );
            for slot in &profile.slots {
                for reference in &slot.candidates {
                    let missing_required = self.plugin(&reference.plugin).is_none()
                        && owner.is_some_and(|p| p.requires.contains(&reference.plugin));
                    if !missing_required && !self.sets.iter().any(|s| &s.reference == reference) {
                        issue(
                            &mut issues,
                            CatalogIssueCode::UnknownSet,
                            &location,
                            &reference.to_string(),
                        );
                    }
                }
            }
        }
        for (i, a) in self.profiles.iter().enumerate() {
            if !self.plugin(&a.reference.plugin).is_some_and(|p| p.enabled) {
                continue;
            }
            for b in &self.profiles[i + 1..] {
                if !self.plugin(&b.reference.plugin).is_some_and(|p| p.enabled) {
                    continue;
                }
                let same_target = (a.default && b.default)
                    || (a.locked && b.locked)
                    || (a.for_target.is_some() && a.for_target == b.for_target);
                let ambiguous = (a.when.is_none() && b.when.is_none())
                    || (a.when.is_some() && b.when.is_some() && a.priority == b.priority);
                if same_target && ambiguous {
                    issue(
                        &mut issues,
                        CatalogIssueCode::ProfileConflict,
                        &a.reference.to_string(),
                        &b.reference.to_string(),
                    );
                }
            }
        }
        for binding_issue in validate_bindings(&self.bindings) {
            let (code, location, detail) = match binding_issue {
                BindingIssue::DuplicateId(id) => (
                    CatalogIssueCode::DuplicateBinding,
                    id,
                    "duplicate binding id".to_owned(),
                ),
                BindingIssue::Invalid { id, message } => {
                    (CatalogIssueCode::InvalidBinding, id, message)
                }
                BindingIssue::Conflict { first, second } => (
                    CatalogIssueCode::InvalidBinding,
                    first,
                    format!("conflicts with {second}"),
                ),
            };
            issue(&mut issues, code, &location, &detail);
        }
        for binding in &self.bindings {
            if let LayerOperation::Layer(reference) | LayerOperation::ToggleLayer(reference) =
                &binding.action.operation
                && !self.sets.iter().any(|s| &s.reference == reference)
            {
                issue(
                    &mut issues,
                    CatalogIssueCode::UnknownSet,
                    &binding.id,
                    &reference.to_string(),
                );
            }
        }
        for (i, choice) in self.profile_for.iter().enumerate() {
            if !self.profiles.iter().any(|p| p.reference == choice.profile)
                || self.profile_for[..i].iter().any(|previous| {
                    previous.surface == choice.surface && previous.target == choice.target
                })
            {
                issue(
                    &mut issues,
                    CatalogIssueCode::InvalidProfile,
                    "profile-for",
                    "missing profile or duplicate selection for the same surface and target",
                );
            }
        }
        issues
    }
    pub fn set_available(&self, reference: &SetRef) -> bool {
        self.plugin(&reference.plugin).is_some_and(|p| p.enabled)
            && self.sets.iter().any(|s| {
                &s.reference == reference
                    && s.logic.as_ref().is_none_or(|logic| {
                        self.plugin(logic).is_some_and(|p| p.enabled && p.has_logic)
                    })
            })
    }
    pub fn plugin(&self, id: &PluginId) -> Option<&PluginSpec> {
        self.plugins.iter().find(|p| &p.id == id)
    }
}

fn issue(out: &mut Vec<CatalogIssue>, code: CatalogIssueCode, location: &str, detail: &str) {
    out.push(CatalogIssue {
        code,
        location: location.into(),
        detail: detail.into(),
    });
}
fn duplicates<'a, T: Ord + std::fmt::Display + 'a>(
    values: impl Iterator<Item = &'a T>,
    code: CatalogIssueCode,
    location: &str,
    out: &mut Vec<CatalogIssue>,
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            issue(out, code, location, &value.to_string());
        }
    }
}
fn check_condition(condition: Option<&Condition>, location: &str, out: &mut Vec<CatalogIssue>) {
    if let Some(condition) = condition
        && let Err(error) = condition.validate()
    {
        issue(
            out,
            CatalogIssueCode::InvalidCondition,
            location,
            &error.to_string(),
        );
    }
}
fn check_units(units: Option<UnitSize>, location: &str, out: &mut Vec<CatalogIssue>) {
    if let Some(units) = units
        && let Err(error) = units.validate()
    {
        issue(
            out,
            CatalogIssueCode::InvalidLength,
            location,
            &error.to_string(),
        );
    }
}
