use crate::{Documents, sizing};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use touchbar_core::{Scene, SceneItem};
use touchbar_layout::{self as l, BarSpec, Element, GroupSpec, ItemId, ItemSpec};
use touchbar_model as m;
use touchbar_profiles::ProfileElement;
#[derive(Clone)]
pub(crate) struct Composition {
    pub bars: Vec<(m::ScenePlane, BarSpec)>,
    pub slots: BTreeMap<ItemId, (m::SlotId, Option<m::InstanceId>, m::ScenePlane)>,
}
fn id(item: &SceneItem) -> ItemId {
    ItemId::new(format!(
        "instance-{}-{}",
        item.instance.number, item.instance.generation.0
    ))
}
pub(crate) fn compose(
    surface: &m::SurfaceInfo,
    scene: &Scene,
    docs: &Documents,
) -> Result<Composition> {
    let mut out = Composition {
        bars: vec![],
        slots: BTreeMap::new(),
    };
    if let Some(profile) = scene.profile.as_ref().and_then(|p| docs.profiles.get(p)) {
        let units = profile
            .spec
            .units
            .unwrap_or_else(|| m::UnitSize::resolve(surface, None, None));
        let elements = elements(
            &profile.elements,
            surface,
            scene,
            docs,
            units,
            &mut out.slots,
        )?;
        out.bars.push((
            m::ScenePlane::Profile,
            BarSpec {
                elements,
                principal_item: None,
            },
        ));
    } else {
        let mut elements = vec![];
        for item in scene
            .items
            .iter()
            .filter(|i| i.plane == m::ScenePlane::Profile)
        {
            elements.push(item_element(item, surface, docs)?);
            out.slots.insert(
                id(item),
                (item.slot.clone(), Some(item.instance), item.plane),
            );
        }
        out.bars.push((
            m::ScenePlane::Profile,
            BarSpec {
                elements,
                principal_item: None,
            },
        ));
    }
    let mut layers: BTreeMap<m::ScenePlane, Vec<Element>> = BTreeMap::new();
    for item in scene
        .items
        .iter()
        .filter(|i| matches!(i.plane, m::ScenePlane::Layer(_)))
    {
        layers
            .entry(item.plane)
            .or_default()
            .push(item_element(item, surface, docs)?);
        out.slots.insert(
            id(item),
            (item.slot.clone(), Some(item.instance), item.plane),
        );
    }
    out.bars.extend(layers.into_iter().map(|(plane, elements)| {
        (
            plane,
            BarSpec {
                elements,
                principal_item: None,
            },
        )
    }));
    let hidden: BTreeSet<_> = scene
        .items
        .iter()
        .filter(|item| {
            docs.sets
                .get(&item.set.reference)
                .and_then(|d| d.file.layout.visible_from.as_ref())
                .is_some_and(|name| {
                    item.state.iter().any(|(n, v)| {
                        n == name && matches!(v, m::StateValue::Scalar(m::Scalar::Boolean(false)))
                    })
                })
        })
        .map(id)
        .collect();
    for (_, bar) in &mut out.bars {
        prune(&mut bar.elements, &hidden);
    }
    Ok(out)
}
fn item_element(item: &SceneItem, surface: &m::SurfaceInfo, docs: &Documents) -> Result<Element> {
    let doc = docs
        .sets
        .get(&item.set.reference)
        .context("scene set absent from accepted documents")?;
    Ok(Element::Item(sizing::root_item(
        id(item),
        &doc.file.layout,
        surface,
        item.units,
    )?))
}
fn elements(
    nodes: &[ProfileElement],
    surface: &m::SurfaceInfo,
    scene: &Scene,
    docs: &Documents,
    units: m::UnitSize,
    slots: &mut BTreeMap<ItemId, (m::SlotId, Option<m::InstanceId>, m::ScenePlane)>,
) -> Result<Vec<Element>> {
    nodes
        .iter()
        .map(|node| {
            Ok(match node {
                ProfileElement::Slot {
                    id: slot,
                    items,
                    visibility_priority,
                    compression_priority,
                } => {
                    let item = scene
                        .items
                        .iter()
                        .find(|i| i.slot == *slot && i.plane == m::ScenePlane::Profile);
                    let key = item.map_or_else(|| ItemId::new(format!("vacant-{slot}")), id);
                    let sizing = if let Some(item) = item {
                        let doc = docs
                            .sets
                            .get(&item.set.reference)
                            .context("set document absent")?;
                        sizing::root_item(key.clone(), &doc.file.layout, surface, item.units)?
                    } else if let Some(doc) =
                        items.iter().find_map(|candidate| docs.sets.get(candidate))
                    {
                        sizing::root_item(
                            key.clone(),
                            &doc.file.layout,
                            surface,
                            doc.file.units.map_or(units, |u| u.u),
                        )?
                    } else {
                        let width = sizing::pixels(m::Length::Unit(800.0 / 3.0), surface, units)?;
                        ItemSpec::new(key.clone(), width, width, width)
                    };
                    slots.insert(
                        key,
                        (
                            slot.clone(),
                            item.map(|i| i.instance),
                            m::ScenePlane::Profile,
                        ),
                    );
                    Element::Item(
                        sizing
                            .visibility_priority(*visibility_priority)
                            .compression_priority(*compression_priority),
                    )
                }
                ProfileElement::FixedSpace { width } => Element::FixedSpace {
                    width: sizing::pixels(*width, surface, units)?,
                },
                ProfileElement::FlexibleSpace { minimum, weight } => Element::FlexibleSpace {
                    minimum: sizing::pixels(*minimum, surface, units)?,
                    weight: *weight,
                },
                ProfileElement::Group {
                    id,
                    layout,
                    spacing,
                    visibility_priority,
                    compression_priority,
                    elements: children,
                } => Element::Group(
                    GroupSpec::new(id, elements(children, surface, scene, docs, units, slots)?)
                        .layout(match layout {
                            touchbar_profiles::GroupLayout::Natural => l::GroupLayout::Natural,
                            touchbar_profiles::GroupLayout::EqualWidth => {
                                l::GroupLayout::EqualWidth
                            }
                        })
                        .spacing(sizing::pixels(*spacing, surface, units)?)
                        .visibility_priority(*visibility_priority)
                        .compression_priority(*compression_priority),
                ),
            })
        })
        .collect()
}

fn prune(elements: &mut Vec<Element>, hidden: &BTreeSet<ItemId>) {
    for element in elements.iter_mut() {
        if let Element::Group(group) = element {
            prune(&mut group.elements, hidden);
        }
    }
    elements.retain(|element| match element {
        Element::Item(item) => !hidden.contains(&item.id),
        Element::Group(group) => !group.elements.is_empty(),
        _ => true,
    });
}
