//! Strict profile documents and their projection into the core catalog.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use touchbar_model::{
    Length, ProfileRef, ProfileSpec, ProfileTarget, SetRef, SlotId, SlotSpec, UnitSize,
};
use touchbar_sets::{
    DocumentError, Units,
    config::{When, error, locate_error, parse_document},
};
fn zero() -> Length {
    Length::Unit(0.0)
}
fn one() -> u32 {
    1
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    pub version: u32,
    pub id: String,
    #[serde(rename = "for")]
    pub for_target: Option<ProfileFor>,
    pub when: Option<When>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub priority: i32,
    pub include: Option<ProfileRef>,
    pub units: Option<Units>,
    #[serde(default, rename = "element")]
    pub elements: Vec<ProfileElement>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ProfileFor {
    App { app: String },
    Category { category: String },
    Locked { locked: bool },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupLayout {
    #[default]
    Natural,
    EqualWidth,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProfileElement {
    Slot {
        id: SlotId,
        items: Vec<SetRef>,
        #[serde(default)]
        visibility_priority: i32,
        #[serde(default)]
        compression_priority: i32,
    },
    FixedSpace {
        width: Length,
    },
    FlexibleSpace {
        #[serde(default = "zero")]
        minimum: Length,
        #[serde(default = "one")]
        weight: u32,
    },
    Group {
        id: String,
        #[serde(default)]
        layout: GroupLayout,
        #[serde(default = "zero")]
        spacing: Length,
        #[serde(default)]
        visibility_priority: i32,
        #[serde(default)]
        compression_priority: i32,
        #[serde(default, rename = "element")]
        elements: Vec<ProfileElement>,
    },
}
impl ProfileFile {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse_document(file.as_ref(), source, Self::validate)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        check(self.version == 1, "version", "unsupported profile version")?;
        check(
            self.id.len() <= 128 && format!("p/{}", self.id).parse::<ProfileRef>().is_ok(),
            "id",
            "invalid profile id",
        )?;
        if let Some(target) = &self.for_target {
            check(
                !self.default,
                "for",
                "profile has multiple selection targets",
            )?;
            match target {
                ProfileFor::App { app } => target_name(app, "for.app")?,
                ProfileFor::Category { category } => target_name(category, "for.category")?,
                ProfileFor::Locked { locked } => {
                    check(*locked, "for.locked", "locked target requires true")?
                }
            }
        }
        if let Some(when) = &self.when {
            when.to_condition("when")?;
        }
        if let Some(units) = self.units {
            units.u.validate().map_err(|e| error("units.u", e.0))?;
        }
        check(
            self.include.is_none() || self.elements.is_empty(),
            "include",
            "include cannot be combined with element",
        )?;
        let mut budget = Budget::default();
        validate_elements(&self.elements, "element", 0, &mut budget)
    }
}
fn check(valid: bool, path: &str, message: &str) -> Result<(), DocumentError> {
    if valid {
        Ok(())
    } else {
        Err(error(path, message))
    }
}
fn target_name(name: &str, path: &str) -> Result<(), DocumentError> {
    check(
        !name.is_empty()
            && name.len() <= 128
            && !name.chars().any(|c| c.is_control() || c.is_whitespace()),
        path,
        "target name is empty, oversized or contains whitespace/control characters",
    )
}
#[derive(Default)]
struct Budget {
    nodes: usize,
    slots: BTreeSet<SlotId>,
    groups: BTreeSet<String>,
}
fn validate_elements(
    elements: &[ProfileElement],
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), DocumentError> {
    for (i, e) in elements.iter().enumerate() {
        let p = format!("{path}[{i}]");
        budget.nodes += 1;
        check(budget.nodes <= 512, &p, "profile exceeds 512 elements")?;
        match e {
            ProfileElement::Slot { id, items, .. } => {
                check(
                    id.as_str().len() <= 128,
                    &format!("{p}.id"),
                    "slot identifier too long",
                )?;
                check(
                    budget.slots.insert(id.clone()),
                    &format!("{p}.id"),
                    "duplicate slot id",
                )?;
                check(budget.slots.len() <= 256, &p, "profile exceeds 256 slots")?;
                check(
                    !items.is_empty(),
                    &format!("{p}.items"),
                    "slot requires at least one candidate",
                )?;
                let mut seen = BTreeSet::new();
                for (i, item) in items.iter().enumerate() {
                    check(
                        seen.insert(item),
                        &format!("{p}.items[{i}]"),
                        "duplicate slot candidate",
                    )?;
                }
            }
            ProfileElement::FixedSpace { width } => width
                .validate()
                .map_err(|e| error(&format!("{p}.width"), e.0))?,
            ProfileElement::FlexibleSpace { minimum, weight } => {
                minimum
                    .validate()
                    .map_err(|e| error(&format!("{p}.minimum"), e.0))?;
                check(
                    *weight > 0,
                    &format!("{p}.weight"),
                    "weight must be positive",
                )?;
            }
            ProfileElement::Group {
                id,
                spacing,
                elements,
                ..
            } => {
                check(depth < 8, &p, "group depth exceeds eight")?;
                check(
                    id.len() <= 128 && id.parse::<SlotId>().is_ok(),
                    &format!("{p}.id"),
                    "invalid group id",
                )?;
                check(
                    budget.groups.insert(id.clone()),
                    &format!("{p}.id"),
                    "duplicate group id",
                )?;
                spacing
                    .validate()
                    .map_err(|e| error(&format!("{p}.spacing"), e.0))?;
                validate_elements(elements, &format!("{p}.element"), depth + 1, budget)?;
            }
        }
    }
    Ok(())
}

/// Source metadata is kept separately from serializable user configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileDocument {
    pub file: PathBuf,
    pub source: String,
    pub profile: ProfileFile,
}
impl ProfileDocument {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        let profile = ProfileFile::from_toml(file.as_ref(), source)?;
        Ok(Self {
            file: file.as_ref().into(),
            source: source.into(),
            profile,
        })
    }
    fn error(&self, path: &str, message: &str) -> DocumentError {
        locate_error(&self.file, &self.source, error(path, message))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProfile {
    pub spec: ProfileSpec,
    pub elements: Vec<ProfileElement>,
}
/// Resolve only layout and units through includes; selection belongs to the
/// referring document. No catalog mutation occurs before the entire graph passes.
pub fn resolve_profiles(
    documents: &BTreeMap<ProfileRef, ProfileDocument>,
) -> Result<BTreeMap<ProfileRef, ResolvedProfile>, DocumentError> {
    let mut out = BTreeMap::new();
    for (reference, doc) in documents {
        let (elements, units) = resolve_layout(reference, documents, &mut Vec::new())?;
        let profile = &doc.profile;
        let mut slots = Vec::new();
        collect_slots(&elements, &mut slots);
        let (for_target, locked) = match &profile.for_target {
            Some(ProfileFor::App { app }) => (Some(ProfileTarget::App(app.clone())), false),
            Some(ProfileFor::Category { category }) => {
                (Some(ProfileTarget::Category(category.clone())), false)
            }
            Some(ProfileFor::Locked { locked }) => (None, *locked),
            None => (None, false),
        };
        let spec = ProfileSpec {
            reference: reference.clone(),
            for_target,
            when: profile
                .when
                .as_ref()
                .map(|w| w.to_condition("when"))
                .transpose()?,
            priority: profile.priority,
            default: profile.default,
            locked,
            slots,
            units,
        };
        out.insert(reference.clone(), ResolvedProfile { spec, elements });
    }
    Ok(out)
}
fn resolve_layout(
    reference: &ProfileRef,
    documents: &BTreeMap<ProfileRef, ProfileDocument>,
    chain: &mut Vec<ProfileRef>,
) -> Result<(Vec<ProfileElement>, Option<UnitSize>), DocumentError> {
    let doc = &documents[reference];
    let profile = &doc.profile;
    profile
        .validate()
        .map_err(|e| locate_error(&doc.file, &doc.source, e))?;
    if reference.id() != profile.id {
        return Err(doc.error("id", "manifest profile id differs from document"));
    }
    chain.push(reference.clone());
    let (elements, inherited) = if let Some(include) = &profile.include {
        if chain.contains(include) {
            return Err(doc.error("include", "profile include cycle"));
        }
        if chain.len() >= 8 {
            return Err(doc.error("include", "profile include chain exceeds eight"));
        }
        if !documents.contains_key(include) {
            return Err(doc.error("include", "unknown included profile"));
        }
        resolve_layout(include, documents, chain)?
    } else {
        (profile.elements.clone(), None)
    };
    chain.pop();
    Ok((elements, profile.units.map(|u| u.u).or(inherited)))
}
fn collect_slots(elements: &[ProfileElement], out: &mut Vec<SlotSpec>) {
    for element in elements {
        match element {
            ProfileElement::Slot { id, items, .. } => out.push(SlotSpec {
                id: id.clone(),
                candidates: items.clone(),
            }),
            ProfileElement::Group { elements, .. } => collect_slots(elements, out),
            ProfileElement::FixedSpace { .. } | ProfileElement::FlexibleSpace { .. } => {}
        }
    }
}
