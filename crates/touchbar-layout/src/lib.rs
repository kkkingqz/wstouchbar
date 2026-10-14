//! Deterministic, serialization-independent layout for a composed Touch Bar.
//!
//! Plugins register items and bars elsewhere. This crate resolves an already
//! composed, ordered bar into rectangles without knowing how items are
//! discovered, launched, rendered, or persisted.

mod panels;
mod responsive;
pub use panels::{PanelPlacement, PanelSizing, PixelRect, panel_bounds};
pub use responsive::{Representation, ResponsiveVariant, resolve_responsive};

use std::{collections::BTreeSet, error::Error, fmt};

pub const VISIBILITY_PRIORITY_LOW: i32 = -1000;
pub const VISIBILITY_PRIORITY_NORMAL: i32 = 0;
pub const VISIBILITY_PRIORITY_HIGH: i32 = 1000;
pub const MAX_GROUP_DEPTH: usize = 8;
/// Largest logical canvas any supported Touch Bar may present.
///
/// The canvas follows the attached panel, so manifests, profiles, and replay
/// scenarios are all bounded by this ceiling at parse time and then resolved
/// against the live width. It is a sanity limit, not the size of any real
/// panel: the Apple silicon strip is 2008 logical pixels.
pub const MAX_CANVAS_WIDTH: u32 = 8192;
/// Logical width of the Apple silicon Touch Bar.
///
/// The canvas follows the attached panel, so this is not a bound. It is the
/// reference full-strip width: what headless paths compose at when no panel
/// is attached, and the widest representative width `touchbarctl plugin test`
/// renders each item at.
pub const REFERENCE_CANVAS_WIDTH: u32 = 2008;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ItemId(String);

impl ItemId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ItemId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ItemId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemSpec {
    /// Relative share of spare width; zero preserves the preferred width.
    pub grow: f32,
    pub id: ItemId,
    pub min_width: u32,
    pub preferred_width: u32,
    pub max_width: u32,
    pub visibility_priority: i32,
    pub compression_priority: i32,
    /// Items with the same nonempty group disappear as one atomic unit.
    pub visibility_group: Option<String>,
}

impl ItemSpec {
    pub fn new(
        id: impl Into<ItemId>,
        min_width: u32,
        preferred_width: u32,
        max_width: u32,
    ) -> Self {
        Self {
            id: id.into(),
            grow: 0.0,
            min_width,
            preferred_width,
            max_width,
            visibility_priority: VISIBILITY_PRIORITY_NORMAL,
            compression_priority: VISIBILITY_PRIORITY_NORMAL,
            visibility_group: None,
        }
    }

    pub fn grow(mut self, weight: f32) -> Self {
        self.grow = weight;
        self
    }

    pub fn visibility_priority(mut self, priority: i32) -> Self {
        self.visibility_priority = priority;
        self
    }

    pub fn compression_priority(mut self, priority: i32) -> Self {
        self.compression_priority = priority;
        self
    }

    pub fn visibility_group(mut self, group: impl Into<String>) -> Self {
        self.visibility_group = Some(group.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GroupLayout {
    #[default]
    Natural,
    EqualWidth,
}

/// Nested composition supports the same slots and spaces as the root.
pub type GroupElement = Element;

#[derive(Clone, Debug, PartialEq)]
pub struct GroupSpec {
    pub id: String,
    pub elements: Vec<GroupElement>,
    pub layout: GroupLayout,
    pub spacing: u32,
    pub visibility_priority: i32,
    pub compression_priority: i32,
}

impl GroupSpec {
    pub fn new(id: impl Into<String>, elements: Vec<GroupElement>) -> Self {
        Self {
            id: id.into(),
            elements,
            layout: GroupLayout::Natural,
            spacing: 0,
            visibility_priority: VISIBILITY_PRIORITY_NORMAL,
            compression_priority: VISIBILITY_PRIORITY_NORMAL,
        }
    }

    pub fn layout(mut self, layout: GroupLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn visibility_priority(mut self, priority: i32) -> Self {
        self.visibility_priority = priority;
        self
    }

    pub fn compression_priority(mut self, priority: i32) -> Self {
        self.compression_priority = priority;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Item(ItemSpec),
    Group(GroupSpec),
    FixedSpace { width: u32 },
    FlexibleSpace { minimum: u32, weight: u32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarSpec {
    pub elements: Vec<Element>,
    pub principal_item: Option<ItemId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Placement {
    pub id: ItemId,
    pub x: u32,
    pub width: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedLayout {
    pub placements: Vec<Placement>,
    pub hidden_items: Vec<ItemId>,
    /// The right edge of the last element. It can be less than the bar width
    /// when no flexible space or principal item consumes trailing room.
    pub content_width: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    InvalidGroupFlexibleSpace { group: String, element: usize },
    InvalidPanelGeometry,
    InvalidPanelSizing,
    EmptyItemId,
    DuplicateItem(ItemId),
    InvalidSizing(ItemId),
    InvalidFlexibleSpace { element: usize },
    UnknownPrincipal(ItemId),
    UnknownRequired(ItemId),
    EmptyGroupId,
    DuplicateGroup(String),
    GroupDepthExceeded(String),
    InvalidEqualWidthGroup(String),
    InconsistentVisibilityGroup(String),
    InsufficientWidth { available: u32, required: u32 },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGroupFlexibleSpace { group, element } => write!(
                formatter,
                "flexible space {element} in group {group} has zero weight"
            ),
            Self::InvalidPanelGeometry => write!(
                formatter,
                "panel rectangles must be nonempty and inside the surface"
            ),
            Self::InvalidPanelSizing => write!(
                formatter,
                "panel sizing must satisfy minimum <= preferred <= maximum"
            ),
            Self::EmptyItemId => write!(formatter, "item identifiers must not be empty"),
            Self::DuplicateItem(id) => write!(formatter, "duplicate item identifier {id}"),
            Self::InvalidSizing(id) => write!(
                formatter,
                "item {id} must have finite nonnegative grow and min <= preferred <= max"
            ),
            Self::InvalidFlexibleSpace { element } => {
                write!(
                    formatter,
                    "flexible space at element {element} has zero weight"
                )
            }
            Self::UnknownPrincipal(id) => {
                write!(formatter, "principal item {id} is not in the bar")
            }
            Self::UnknownRequired(id) => write!(formatter, "required item {id} is not in the bar"),
            Self::EmptyGroupId => write!(formatter, "group identifiers must not be empty"),
            Self::DuplicateGroup(id) => write!(formatter, "duplicate group identifier {id}"),
            Self::GroupDepthExceeded(id) => {
                write!(formatter, "group {id} exceeds the maximum nesting depth")
            }
            Self::InvalidEqualWidthGroup(id) => write!(
                formatter,
                "equal-width group {id} has no width valid for every direct child"
            ),
            Self::InconsistentVisibilityGroup(group) => write!(
                formatter,
                "visibility group {group} contains different visibility priorities"
            ),
            Self::InsufficientWidth {
                available,
                required,
            } => write!(
                formatter,
                "bar has {available} pixels but its required content needs {required}"
            ),
        }
    }
}

impl Error for LayoutError {}

pub fn resolve(
    bar: &BarSpec,
    available_width: u32,
    required_items: &BTreeSet<ItemId>,
) -> Result<ResolvedLayout, LayoutError> {
    validate(bar, required_items)?;
    let mut units = visibility_units(bar, required_items)?;
    let mut visible = bar
        .elements
        .iter()
        .map(|element| matches!(element, Element::Item(_) | Element::Group(_)))
        .collect::<Vec<_>>();
    let sizing = bar
        .elements
        .iter()
        .map(element_sizing)
        .collect::<Result<Vec<_>, _>>()?;
    let mut minimum_total = total_width(bar, &visible, &sizing);

    if minimum_total > u64::from(available_width) {
        units.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| right.trailing_element.cmp(&left.trailing_element))
        });
        for unit in units.into_iter().filter(|unit| !unit.required) {
            for element in unit.elements {
                if visible[element] {
                    visible[element] = false;
                    minimum_total -= sizing[element].minimum;
                }
            }
            if minimum_total <= u64::from(available_width) {
                break;
            }
        }
    }

    if minimum_total > u64::from(available_width) {
        return Err(LayoutError::InsufficientWidth {
            available: available_width,
            required: u32::try_from(minimum_total).unwrap_or(u32::MAX),
        });
    }

    let mut widths = bar
        .elements
        .iter()
        .enumerate()
        .map(|(index, element)| match element {
            Element::Item(_) | Element::Group(_) if visible[index] => sizing[index].preferred,
            Element::Item(_) | Element::Group(_) => 0,
            Element::FixedSpace { width } => u64::from(*width),
            Element::FlexibleSpace { minimum, .. } => u64::from(*minimum),
        })
        .collect::<Vec<_>>();
    compress_to_fit(bar, &visible, &sizing, &mut widths, available_width);

    let principal = bar.principal_item.as_ref().and_then(|principal| {
        bar.elements
            .iter()
            .enumerate()
            .find(|(index, element)| visible[*index] && contains_item(element, principal))
            .and_then(|(index, element)| {
                principal_metrics(element, widths[index], principal)
                    .map(|(offset, width)| (index, offset, width))
            })
    });
    let base_total = widths.iter().copied().sum::<u64>();
    let extra = u64::from(available_width) - base_total;
    let principal_x = if let Some((principal, offset, principal_width)) = principal {
        let before = widths[..principal].iter().copied().sum::<u64>();
        let after = widths[principal + 1..].iter().copied().sum::<u64>();
        let desired_leaf_x = (u64::from(available_width) - principal_width) / 2;
        let desired = desired_leaf_x.saturating_sub(offset);
        let maximum = u64::from(available_width) - widths[principal] - after;
        let x = desired.clamp(before, maximum);
        let principal_element_width = widths[principal];
        distribute_flexible_space(bar, &visible, &mut widths, 0..principal, x - before);
        distribute_flexible_space(
            bar,
            &visible,
            &mut widths,
            principal + 1..bar.elements.len(),
            u64::from(available_width) - x - principal_element_width - after,
        );
        Some((principal, x))
    } else {
        distribute_flexible_space(bar, &visible, &mut widths, 0..bar.elements.len(), extra);
        None
    };

    let mut placements = Vec::new();
    let mut hidden_items = Vec::new();
    let mut x = 0_u64;
    for (index, element) in bar.elements.iter().enumerate() {
        if principal_x.is_some_and(|(principal, _)| principal == index) {
            x = principal_x.expect("checked above").1;
        }
        match element {
            Element::Item(_) | Element::Group(_) if visible[index] => {
                layout_element(element, x, widths[index], &mut placements);
            }
            Element::Item(_) | Element::Group(_) => collect_item_ids(element, &mut hidden_items),
            Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => {}
        }
        x += widths[index];
    }

    Ok(ResolvedLayout {
        placements,
        hidden_items,
        content_width: u32::try_from(x).expect("resolved content fits the available width"),
    })
}

#[derive(Clone, Copy, Debug)]
struct Sizing {
    minimum: u64,
    preferred: u64,
    maximum: u64,
}

fn total_width(bar: &BarSpec, visible: &[bool], sizing: &[Sizing]) -> u64 {
    bar.elements
        .iter()
        .enumerate()
        .map(|(index, element)| match element {
            Element::Item(_) | Element::Group(_) if visible[index] => sizing[index].minimum,
            Element::Item(_) | Element::Group(_) => 0,
            Element::FixedSpace { width } => u64::from(*width),
            Element::FlexibleSpace { minimum, .. } => u64::from(*minimum),
        })
        .sum()
}

fn element_sizing(element: &Element) -> Result<Sizing, LayoutError> {
    match element {
        Element::Item(item) => Ok(item_sizing(item)),
        Element::Group(group) => group_sizing(group),
        Element::FixedSpace { width } => Ok(fixed_sizing(*width)),
        Element::FlexibleSpace { minimum, .. } => Ok(Sizing {
            minimum: u64::from(*minimum),
            preferred: u64::from(*minimum),
            maximum: u64::from(u32::MAX),
        }),
    }
}

fn group_element_sizing(element: &GroupElement) -> Result<Sizing, LayoutError> {
    match element {
        GroupElement::Item(item) => Ok(item_sizing(item)),
        GroupElement::Group(group) => group_sizing(group),
        GroupElement::FixedSpace { width } => Ok(fixed_sizing(*width)),
        GroupElement::FlexibleSpace { minimum, .. } => Ok(Sizing {
            minimum: u64::from(*minimum),
            preferred: u64::from(*minimum),
            maximum: u64::from(u32::MAX),
        }),
    }
}

fn item_sizing(item: &ItemSpec) -> Sizing {
    Sizing {
        minimum: u64::from(item.min_width),
        preferred: u64::from(item.preferred_width),
        maximum: u64::from(item.max_width),
    }
}

fn fixed_sizing(width: u32) -> Sizing {
    let width = u64::from(width);
    Sizing {
        minimum: width,
        preferred: width,
        maximum: width,
    }
}

fn group_sizing(group: &GroupSpec) -> Result<Sizing, LayoutError> {
    let children = group
        .elements
        .iter()
        .map(group_element_sizing)
        .collect::<Result<Vec<_>, _>>()?;
    let children = children
        .into_iter()
        .filter(|child| child.maximum > 0)
        .collect::<Vec<_>>();
    if children.is_empty() {
        return Ok(fixed_sizing(0));
    }
    let gaps = u64::from(group.spacing) * (children.len().saturating_sub(1) as u64);
    match group.layout {
        GroupLayout::Natural => Ok(Sizing {
            minimum: children.iter().map(|child| child.minimum).sum::<u64>() + gaps,
            preferred: children.iter().map(|child| child.preferred).sum::<u64>() + gaps,
            maximum: children.iter().map(|child| child.maximum).sum::<u64>() + gaps,
        }),
        GroupLayout::EqualWidth => {
            let common_minimum = children
                .iter()
                .map(|child| child.minimum)
                .max()
                .expect("nonempty group");
            let common_maximum = children
                .iter()
                .map(|child| child.maximum)
                .min()
                .expect("nonempty group");
            if common_minimum > common_maximum {
                return Err(LayoutError::InvalidEqualWidthGroup(group.id.clone()));
            }
            let common_preferred = children
                .iter()
                .map(|child| child.preferred)
                .max()
                .expect("nonempty group")
                .clamp(common_minimum, common_maximum);
            let count = children.len() as u64;
            Ok(Sizing {
                minimum: common_minimum * count + gaps,
                preferred: common_preferred * count + gaps,
                maximum: common_maximum * count + gaps,
            })
        }
    }
}

fn validate(bar: &BarSpec, required_items: &BTreeSet<ItemId>) -> Result<(), LayoutError> {
    let mut identifiers = BTreeSet::new();
    let mut group_identifiers = BTreeSet::new();
    for (index, element) in bar.elements.iter().enumerate() {
        match element {
            Element::Item(item) => validate_item(item, &mut identifiers)?,
            Element::Group(group) => {
                validate_group(group, 1, &mut identifiers, &mut group_identifiers)?;
            }
            Element::FlexibleSpace { weight: 0, .. } => {
                return Err(LayoutError::InvalidFlexibleSpace { element: index });
            }
            Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => {}
        }
    }
    if let Some(principal) = &bar.principal_item
        && !identifiers.contains(principal)
    {
        return Err(LayoutError::UnknownPrincipal(principal.clone()));
    }
    if let Some(unknown) = required_items
        .iter()
        .find(|required| !identifiers.contains(*required))
    {
        return Err(LayoutError::UnknownRequired(unknown.clone()));
    }
    Ok(())
}

fn validate_item(item: &ItemSpec, identifiers: &mut BTreeSet<ItemId>) -> Result<(), LayoutError> {
    if item.id.as_str().is_empty() {
        return Err(LayoutError::EmptyItemId);
    }
    if !item.grow.is_finite()
        || item.grow < 0.0
        || item.min_width > item.preferred_width
        || item.preferred_width > item.max_width
    {
        return Err(LayoutError::InvalidSizing(item.id.clone()));
    }
    if !identifiers.insert(item.id.clone()) {
        return Err(LayoutError::DuplicateItem(item.id.clone()));
    }
    Ok(())
}

fn validate_group(
    group: &GroupSpec,
    depth: usize,
    item_identifiers: &mut BTreeSet<ItemId>,
    group_identifiers: &mut BTreeSet<String>,
) -> Result<(), LayoutError> {
    if group.id.is_empty() {
        return Err(LayoutError::EmptyGroupId);
    }
    if depth > MAX_GROUP_DEPTH {
        return Err(LayoutError::GroupDepthExceeded(group.id.clone()));
    }
    if !group_identifiers.insert(group.id.clone()) {
        return Err(LayoutError::DuplicateGroup(group.id.clone()));
    }
    for (index, element) in group.elements.iter().enumerate() {
        match element {
            GroupElement::FlexibleSpace { weight: 0, .. } => {
                return Err(LayoutError::InvalidGroupFlexibleSpace {
                    group: group.id.clone(),
                    element: index,
                });
            }
            GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => {}
            GroupElement::Item(item) => validate_item(item, item_identifiers)?,
            GroupElement::Group(child) => {
                validate_group(child, depth + 1, item_identifiers, group_identifiers)?
            }
        }
    }
    group_sizing(group)?;
    Ok(())
}

struct VisibilityUnit {
    group: Option<String>,
    elements: Vec<usize>,
    priority: i32,
    trailing_element: usize,
    required: bool,
}

fn visibility_units(
    bar: &BarSpec,
    required_items: &BTreeSet<ItemId>,
) -> Result<Vec<VisibilityUnit>, LayoutError> {
    let mut units: Vec<VisibilityUnit> = Vec::new();
    for (index, element) in bar.elements.iter().enumerate() {
        match element {
            Element::Item(item) => {
                let existing = item.visibility_group.as_ref().and_then(|group| {
                    units
                        .iter_mut()
                        .find(|unit| unit.group.as_ref() == Some(group))
                });
                if let Some(unit) = existing {
                    if unit.priority != item.visibility_priority {
                        return Err(LayoutError::InconsistentVisibilityGroup(
                            item.visibility_group.clone().expect("matched group"),
                        ));
                    }
                    unit.elements.push(index);
                    unit.trailing_element = index;
                    unit.required |= required_items.contains(&item.id);
                } else {
                    units.push(VisibilityUnit {
                        group: item.visibility_group.clone(),
                        elements: vec![index],
                        priority: item.visibility_priority,
                        trailing_element: index,
                        required: required_items.contains(&item.id),
                    });
                }
            }
            Element::Group(group) => units.push(VisibilityUnit {
                group: None,
                elements: vec![index],
                priority: group.visibility_priority,
                trailing_element: index,
                required: group_contains_required(group, required_items),
            }),
            Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => {}
        }
    }
    Ok(units)
}

fn compress_to_fit(
    bar: &BarSpec,
    visible: &[bool],
    sizing: &[Sizing],
    widths: &mut [u64],
    available_width: u32,
) {
    let total = widths.iter().copied().sum::<u64>();
    let mut excess = total.saturating_sub(u64::from(available_width));
    let mut candidates = bar
        .elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| match element {
            Element::Item(item) if visible[index] => {
                Some((item.compression_priority, index, sizing[index].minimum))
            }
            Element::Group(group) if visible[index] => {
                Some((group.compression_priority, index, sizing[index].minimum))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    for (_, index, minimum) in candidates {
        if excess == 0 {
            break;
        }
        let reduction = (widths[index] - minimum).min(excess);
        widths[index] -= reduction;
        excess -= reduction;
    }
    debug_assert_eq!(excess, 0);
}

fn distribute_flexible_space(
    bar: &BarSpec,
    visible: &[bool],
    widths: &mut [u64],
    range: std::ops::Range<usize>,
    extra: u64,
) {
    if extra == 0 {
        return;
    }
    let sizing = bar
        .elements
        .iter()
        .map(element_sizing)
        .collect::<Result<Vec<_>, _>>()
        .expect("validated sizing");
    let weights = bar
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if !visible[i] && matches!(e, Element::Item(_) | Element::Group(_)) {
                0.0
            } else {
                element_grow(e)
            }
        })
        .collect::<Vec<_>>();
    weighted_grow(widths, &sizing, &weights, range, extra);
}

fn element_grow(element: &Element) -> f64 {
    match element {
        Element::Item(item) => f64::from(item.grow),
        Element::Group(group) => group.elements.iter().map(group_element_grow).sum(),
        Element::FlexibleSpace { weight, .. } => f64::from(*weight),
        Element::FixedSpace { .. } => 0.0,
    }
}
fn group_element_grow(element: &GroupElement) -> f64 {
    match element {
        GroupElement::Item(item) => f64::from(item.grow),
        GroupElement::Group(group) => group.elements.iter().map(group_element_grow).sum(),
        GroupElement::FlexibleSpace { weight, .. } => f64::from(*weight),
        GroupElement::FixedSpace { .. } => 0.0,
    }
}
// Ratios are bounded by remaining u32 surface width; the fractional part is
// deliberately dropped, with deterministic last-active-item remainder.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn weighted_grow(
    widths: &mut [u64],
    sizing: &[Sizing],
    weights: &[f64],
    range: std::ops::Range<usize>,
    mut extra: u64,
) {
    while extra > 0 {
        let active = range
            .clone()
            .filter(|i| weights[*i] > 0.0 && widths[*i] < sizing[*i].maximum)
            .collect::<Vec<_>>();
        if active.is_empty() {
            break;
        }
        let total = active.iter().map(|i| weights[*i]).sum::<f64>();
        let budget = extra;
        let mut assigned = 0;
        let mut nominal = 0;
        for (position, index) in active.iter().copied().enumerate() {
            let share = if position + 1 == active.len() {
                budget - nominal
            } else {
                (((budget as f64) * weights[index] / total).floor() as u64).min(budget - nominal)
            };
            nominal += share;
            let addition = share
                .min(sizing[index].maximum - widths[index])
                .min(budget - assigned);
            widths[index] += addition;
            assigned += addition;
        }
        if assigned == 0 {
            break;
        }
        extra -= assigned;
    }
}

fn contains_item(element: &Element, id: &ItemId) -> bool {
    match element {
        Element::Item(item) => &item.id == id,
        Element::Group(group) => group_contains_item(group, id),
        Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => false,
    }
}

fn group_contains_item(group: &GroupSpec, id: &ItemId) -> bool {
    group.elements.iter().any(|element| match element {
        GroupElement::Item(item) => &item.id == id,
        GroupElement::Group(group) => group_contains_item(group, id),
        GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => false,
    })
}

fn group_contains_required(group: &GroupSpec, required_items: &BTreeSet<ItemId>) -> bool {
    group.elements.iter().any(|element| match element {
        GroupElement::Item(item) => required_items.contains(&item.id),
        GroupElement::Group(group) => group_contains_required(group, required_items),
        GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => false,
    })
}

fn collect_item_ids(element: &Element, identifiers: &mut Vec<ItemId>) {
    match element {
        Element::Item(item) => identifiers.push(item.id.clone()),
        Element::Group(group) => collect_group_item_ids(group, identifiers),
        Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => {}
    }
}

fn collect_group_item_ids(group: &GroupSpec, identifiers: &mut Vec<ItemId>) {
    for element in &group.elements {
        match element {
            GroupElement::Item(item) => identifiers.push(item.id.clone()),
            GroupElement::Group(group) => collect_group_item_ids(group, identifiers),
            GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => {}
        }
    }
}

fn principal_metrics(element: &Element, width: u64, principal: &ItemId) -> Option<(u64, u64)> {
    match element {
        Element::Item(item) => (&item.id == principal).then_some((0, width)),
        Element::Group(group) => {
            let mut placements = Vec::new();
            layout_group(group, 0, width, &mut placements);
            placements
                .into_iter()
                .find(|placement| &placement.id == principal)
                .map(|placement| (u64::from(placement.x), u64::from(placement.width)))
        }
        Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => None,
    }
}

fn layout_element(element: &Element, x: u64, width: u64, placements: &mut Vec<Placement>) {
    match element {
        Element::Item(item) => placements.push(Placement {
            id: item.id.clone(),
            x: u32::try_from(x).expect("resolved positions fit the available width"),
            width: u32::try_from(width).expect("resolved widths fit the available width"),
        }),
        Element::Group(group) => layout_group(group, x, width, placements),
        Element::FixedSpace { .. } | Element::FlexibleSpace { .. } => {}
    }
}

fn layout_group(group: &GroupSpec, x: u64, width: u64, placements: &mut Vec<Placement>) {
    let active = group
        .elements
        .iter()
        .filter(|element| {
            group_element_sizing(element)
                .expect("validated group sizing")
                .maximum
                > 0
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        return;
    }
    let gaps = u64::from(group.spacing) * (active.len().saturating_sub(1) as u64);
    let content_width = width
        .checked_sub(gaps)
        .expect("validated group width includes its spacing");
    let child_widths = match group.layout {
        GroupLayout::Natural => natural_child_widths(&active, content_width),
        GroupLayout::EqualWidth => equal_child_widths(active.len(), content_width),
    };
    let mut child_x = x;
    for (element, child_width) in active.into_iter().zip(child_widths) {
        match element {
            GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => {}
            GroupElement::Item(item) => placements.push(Placement {
                id: item.id.clone(),
                x: u32::try_from(child_x).expect("resolved positions fit the available width"),
                width: u32::try_from(child_width).expect("resolved widths fit the available width"),
            }),
            GroupElement::Group(group) => {
                layout_group(group, child_x, child_width, placements);
            }
        }
        child_x += child_width + u64::from(group.spacing);
    }
}

fn natural_child_widths(elements: &[&GroupElement], content_width: u64) -> Vec<u64> {
    let sizing = elements
        .iter()
        .map(|element| group_element_sizing(element))
        .collect::<Result<Vec<_>, _>>()
        .expect("validated group sizing");
    let mut widths = sizing
        .iter()
        .map(|child| child.preferred)
        .collect::<Vec<_>>();
    let preferred = widths.iter().sum::<u64>();
    if content_width < preferred {
        let mut excess = preferred - content_width;
        let mut candidates = elements
            .iter()
            .enumerate()
            .map(|(index, element)| (group_element_compression_priority(element), index))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
        for (_, index) in candidates {
            let reduction = (widths[index] - sizing[index].minimum).min(excess);
            widths[index] -= reduction;
            excess -= reduction;
            if excess == 0 {
                break;
            }
        }
        debug_assert_eq!(excess, 0);
    } else if content_width > preferred {
        let weights = elements
            .iter()
            .map(|element| group_element_grow(element))
            .collect::<Vec<_>>();
        if weights.iter().any(|w| *w > 0.0) {
            let count = widths.len();
            weighted_grow(
                &mut widths,
                &sizing,
                &weights,
                0..count,
                content_width - preferred,
            );
        } else {
            // A natural group assigned a wider equal-width parent cell keeps
            // the original contract: fill that cell up to child maxima.
            grow_widths(&mut widths, &sizing, content_width - preferred);
        }
    }
    widths
}

fn group_element_compression_priority(element: &GroupElement) -> i32 {
    match element {
        GroupElement::Item(item) => item.compression_priority,
        GroupElement::Group(group) => group.compression_priority,
        GroupElement::FixedSpace { .. } | GroupElement::FlexibleSpace { .. } => i32::MAX,
    }
}

fn grow_widths(widths: &mut [u64], sizing: &[Sizing], mut extra: u64) {
    while extra > 0 {
        let growable = widths
            .iter()
            .zip(sizing)
            .enumerate()
            .filter_map(|(index, (width, sizing))| (*width < sizing.maximum).then_some(index))
            .collect::<Vec<_>>();
        if growable.is_empty() {
            break;
        }
        let share = (extra / growable.len() as u64).max(1);
        let mut assigned = 0;
        for index in growable {
            let addition = (sizing[index].maximum - widths[index])
                .min(share)
                .min(extra - assigned);
            widths[index] += addition;
            assigned += addition;
            if assigned == extra {
                break;
            }
        }
        debug_assert!(assigned > 0);
        extra -= assigned;
    }
    debug_assert_eq!(extra, 0);
}

fn equal_child_widths(count: usize, content_width: u64) -> Vec<u64> {
    let base = content_width / count as u64;
    let remainder = content_width % count as u64;
    (0..count)
        .map(|index| base + u64::from((index as u64) < remainder))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, minimum: u32, preferred: u32) -> Element {
        Element::Item(ItemSpec::new(id, minimum, preferred, preferred))
    }

    fn ids(values: &[&str]) -> BTreeSet<ItemId> {
        values.iter().copied().map(ItemId::from).collect()
    }

    fn group_item(id: &str, minimum: u32, preferred: u32, maximum: u32) -> GroupElement {
        GroupElement::Item(ItemSpec::new(id, minimum, preferred, maximum))
    }

    #[test]
    fn flexible_space_pushes_trailing_content_to_the_edge() {
        let bar = BarSpec {
            elements: vec![
                item("leading", 40, 40),
                Element::FlexibleSpace {
                    minimum: 0,
                    weight: 1,
                },
                item("trailing", 40, 40),
            ],
            principal_item: None,
        };
        let layout = resolve(&bar, 200, &ids(&[])).unwrap();
        assert_eq!(layout.placements[0].x, 0);
        assert_eq!(layout.placements[1].x, 160);
        assert_eq!(layout.content_width, 200);
    }

    #[test]
    fn principal_item_centers_without_left_center_right_zones() {
        let bar = BarSpec {
            elements: vec![
                item("wide-leading", 200, 200),
                item("principal", 100, 100),
                item("short-trailing", 50, 50),
            ],
            principal_item: Some("principal".into()),
        };
        let layout = resolve(&bar, 1000, &ids(&[])).unwrap();
        assert_eq!(layout.placements[0].x, 0);
        assert_eq!(layout.placements[1].x, 450);
        assert_eq!(layout.placements[2].x, 550);
    }

    #[test]
    fn principal_item_clamps_when_one_side_is_too_large() {
        let bar = BarSpec {
            elements: vec![item("leading", 480, 480), item("principal", 100, 100)],
            principal_item: Some("principal".into()),
        };
        let layout = resolve(&bar, 600, &ids(&[])).unwrap();
        assert_eq!(layout.placements[1].x, 480);
    }

    #[test]
    fn compresses_lower_resistance_before_hiding_items() {
        let bar = BarSpec {
            elements: vec![
                Element::Item(ItemSpec::new("elastic", 40, 100, 100).compression_priority(-10)),
                Element::Item(ItemSpec::new("rigid", 80, 100, 100).compression_priority(10)),
            ],
            principal_item: None,
        };
        let layout = resolve(&bar, 160, &ids(&[])).unwrap();
        assert_eq!(layout.hidden_items, vec![]);
        assert_eq!(layout.placements[0].width, 60);
        assert_eq!(layout.placements[1].width, 100);
    }

    #[test]
    fn hides_a_low_priority_group_atomically() {
        let low = VISIBILITY_PRIORITY_LOW;
        let bar = BarSpec {
            elements: vec![
                Element::Item(
                    ItemSpec::new("group-a", 50, 50, 50)
                        .visibility_priority(low)
                        .visibility_group("tools"),
                ),
                Element::Item(
                    ItemSpec::new("group-b", 50, 50, 50)
                        .visibility_priority(low)
                        .visibility_group("tools"),
                ),
                item("core", 100, 100),
            ],
            principal_item: None,
        };
        let layout = resolve(&bar, 150, &ids(&["core"])).unwrap();
        assert_eq!(
            layout.hidden_items,
            ids(&["group-a", "group-b"]).into_iter().collect::<Vec<_>>()
        );
        assert_eq!(layout.placements.len(), 1);
        assert_eq!(layout.placements[0].id.as_str(), "core");
    }

    #[test]
    fn equal_priority_overflow_hides_the_trailing_item_first() {
        let bar = BarSpec {
            elements: vec![item("a", 80, 80), item("b", 80, 80), item("c", 80, 80)],
            principal_item: None,
        };
        let layout = resolve(&bar, 160, &ids(&[])).unwrap();
        assert_eq!(layout.hidden_items, vec![ItemId::from("c")]);
    }

    #[test]
    fn reports_when_required_content_cannot_fit() {
        let bar = BarSpec {
            elements: vec![item("a", 100, 100), item("b", 100, 100)],
            principal_item: None,
        };
        assert_eq!(
            resolve(&bar, 150, &ids(&["a", "b"])),
            Err(LayoutError::InsufficientWidth {
                available: 150,
                required: 200,
            })
        );
    }

    #[test]
    fn rejects_inconsistent_atomic_group_priorities() {
        let bar = BarSpec {
            elements: vec![
                Element::Item(
                    ItemSpec::new("a", 40, 40, 40)
                        .visibility_priority(-1)
                        .visibility_group("group"),
                ),
                Element::Item(
                    ItemSpec::new("b", 40, 40, 40)
                        .visibility_priority(1)
                        .visibility_group("group"),
                ),
            ],
            principal_item: None,
        };
        assert_eq!(
            resolve(&bar, 40, &ids(&[])),
            Err(LayoutError::InconsistentVisibilityGroup("group".into()))
        );
    }

    #[test]
    fn lays_out_nested_natural_groups_and_spacing() {
        let nested = GroupSpec::new(
            "nested",
            vec![group_item("b", 20, 30, 40), group_item("c", 30, 40, 50)],
        )
        .spacing(3);
        let bar = BarSpec {
            elements: vec![Element::Group(
                GroupSpec::new(
                    "root",
                    vec![group_item("a", 10, 20, 30), GroupElement::Group(nested)],
                )
                .spacing(5),
            )],
            principal_item: None,
        };

        let layout = resolve(&bar, 98, &ids(&[])).unwrap();
        assert_eq!(
            layout.placements,
            vec![
                Placement {
                    id: "a".into(),
                    x: 0,
                    width: 20,
                },
                Placement {
                    id: "b".into(),
                    x: 25,
                    width: 30,
                },
                Placement {
                    id: "c".into(),
                    x: 58,
                    width: 40,
                },
            ]
        );
    }

    #[test]
    fn equal_width_group_assigns_each_direct_child_the_same_space() {
        let bar = BarSpec {
            elements: vec![Element::Group(
                GroupSpec::new(
                    "actions",
                    vec![
                        group_item("short", 20, 30, 80),
                        group_item("wide", 40, 60, 80),
                        group_item("other", 25, 45, 80),
                    ],
                )
                .layout(GroupLayout::EqualWidth)
                .spacing(2),
            )],
            principal_item: None,
        };

        let layout = resolve(&bar, 184, &ids(&[])).unwrap();
        assert_eq!(
            layout
                .placements
                .iter()
                .map(|placement| placement.width)
                .collect::<Vec<_>>(),
            vec![60, 60, 60]
        );
        assert_eq!(layout.placements[2].x, 124);
    }

    #[test]
    fn equal_width_group_treats_a_nested_group_as_one_member() {
        let nested = GroupSpec::new(
            "transport",
            vec![
                group_item("previous", 20, 30, 60),
                group_item("next", 20, 30, 60),
            ],
        );
        let bar = BarSpec {
            elements: vec![Element::Group(
                GroupSpec::new(
                    "columns",
                    vec![GroupElement::Group(nested), group_item("play", 40, 60, 120)],
                )
                .layout(GroupLayout::EqualWidth),
            )],
            principal_item: None,
        };

        let layout = resolve(&bar, 120, &ids(&[])).unwrap();
        assert_eq!(layout.placements[0].width, 30);
        assert_eq!(layout.placements[1].width, 30);
        assert_eq!(layout.placements[2].width, 60);
        assert_eq!(layout.placements[2].x, 60);
    }

    #[test]
    fn empty_nested_groups_collapse_without_spacing_or_equal_width() {
        let bar = BarSpec {
            elements: vec![Element::Group(
                GroupSpec::new(
                    "columns",
                    vec![
                        GroupElement::Group(GroupSpec::new("empty-slot", vec![])),
                        group_item("active", 40, 60, 80),
                    ],
                )
                .layout(GroupLayout::EqualWidth)
                .spacing(12),
            )],
            principal_item: None,
        };

        let layout = resolve(&bar, 60, &ids(&[])).unwrap();
        assert_eq!(layout.content_width, 60);
        assert_eq!(layout.placements[0].x, 0);
        assert_eq!(layout.placements[0].width, 60);
    }

    #[test]
    fn hides_a_composition_group_atomically() {
        let bar = BarSpec {
            elements: vec![
                Element::Group(
                    GroupSpec::new(
                        "optional-tools",
                        vec![group_item("a", 40, 40, 40), group_item("b", 40, 40, 40)],
                    )
                    .visibility_priority(VISIBILITY_PRIORITY_LOW),
                ),
                item("core", 100, 100),
            ],
            principal_item: None,
        };

        let layout = resolve(&bar, 100, &ids(&["core"])).unwrap();
        assert_eq!(
            layout.hidden_items,
            vec![ItemId::from("a"), ItemId::from("b")]
        );
        assert_eq!(layout.placements[0].id.as_str(), "core");
    }

    #[test]
    fn required_descendant_keeps_its_whole_group_visible() {
        let bar = BarSpec {
            elements: vec![
                Element::Group(
                    GroupSpec::new(
                        "required-tools",
                        vec![group_item("a", 40, 40, 40), group_item("b", 40, 40, 40)],
                    )
                    .visibility_priority(VISIBILITY_PRIORITY_LOW),
                ),
                item("optional", 100, 100),
            ],
            principal_item: None,
        };

        let layout = resolve(&bar, 80, &ids(&["a"])).unwrap();
        assert!(layout.hidden_items.contains(&ItemId::from("optional")));
        assert_eq!(layout.placements.len(), 2);
    }

    #[test]
    fn centers_a_principal_item_nested_inside_a_group() {
        let bar = BarSpec {
            elements: vec![
                item("leading", 100, 100),
                Element::FlexibleSpace {
                    minimum: 0,
                    weight: 1,
                },
                Element::Group(GroupSpec::new(
                    "center",
                    vec![
                        group_item("before", 20, 20, 20),
                        group_item("principal", 40, 40, 40),
                    ],
                )),
                Element::FlexibleSpace {
                    minimum: 0,
                    weight: 1,
                },
                item("trailing", 20, 20),
            ],
            principal_item: Some("principal".into()),
        };

        let layout = resolve(&bar, 400, &ids(&[])).unwrap();
        let principal = layout
            .placements
            .iter()
            .find(|placement| placement.id.as_str() == "principal")
            .unwrap();
        assert_eq!(principal.x, 180);
    }

    #[test]
    fn rejects_equal_width_children_without_a_common_valid_width() {
        let bar = BarSpec {
            elements: vec![Element::Group(
                GroupSpec::new(
                    "impossible",
                    vec![
                        group_item("small", 20, 20, 30),
                        group_item("large", 40, 40, 50),
                    ],
                )
                .layout(GroupLayout::EqualWidth),
            )],
            principal_item: None,
        };

        assert_eq!(
            resolve(&bar, 100, &ids(&[])),
            Err(LayoutError::InvalidEqualWidthGroup("impossible".into()))
        );
    }

    #[test]
    fn rejects_duplicate_nested_identifiers_and_excessive_depth() {
        let duplicate = BarSpec {
            elements: vec![
                item("same", 20, 20),
                Element::Group(GroupSpec::new(
                    "group",
                    vec![group_item("same", 20, 20, 20)],
                )),
            ],
            principal_item: None,
        };
        assert_eq!(
            resolve(&duplicate, 100, &ids(&[])),
            Err(LayoutError::DuplicateItem("same".into()))
        );

        let mut group = GroupSpec::new("depth-9", vec![]);
        for depth in (1..=8).rev() {
            group = GroupSpec::new(format!("depth-{depth}"), vec![GroupElement::Group(group)]);
        }
        let too_deep = BarSpec {
            elements: vec![Element::Group(group)],
            principal_item: None,
        };
        assert_eq!(
            resolve(&too_deep, 100, &ids(&[])),
            Err(LayoutError::GroupDepthExceeded("depth-9".into()))
        );
    }
}
