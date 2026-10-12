use serde::{Deserialize, Serialize};
use touchbar_model::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionTarget {
    Locked,
    App(String),
    Category(String),
    Default,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateExplanation {
    pub profile: ProfileRef,
    pub available: bool,
    pub conditional: bool,
    pub when_matches: bool,
    pub priority: i32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolutionStep {
    pub target: ResolutionTarget,
    pub candidates: Vec<CandidateExplanation>,
    pub conflict: bool,
    pub override_profile: Option<ProfileRef>,
    pub selected: Option<ProfileRef>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    pub facts: Facts,
    pub steps: Vec<ResolutionStep>,
    pub selected: Option<ProfileRef>,
    pub slots: Vec<SlotExplanation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetAvailability {
    Available,
    MissingPlugin,
    DisabledPlugin,
    MissingSet,
    MissingLogic,
    DisabledLogic,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetCandidateExplanation {
    pub set: SetRef,
    pub availability: SetAvailability,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotExplanation {
    pub slot: SlotId,
    pub candidates: Vec<SetCandidateExplanation>,
    pub selected: Option<SetRef>,
}
pub fn resolve(catalog: &Catalog, facts: &Facts, surface: &SurfaceInfo) -> Resolution {
    let mut facts = facts.clone();
    facts.extend(surface.facts());
    let mut targets = vec![];
    if surface.kind != SurfaceKind::Touchbar
        && facts.get(&"session.locked".into()) == Some(&FactValue::Boolean(true))
    {
        targets.push(ResolutionTarget::Locked);
    }
    if let Some(FactValue::Text(app)) = facts.get(&"app.id".into()) {
        targets.push(ResolutionTarget::App(app.clone()));
    }
    if let Some(FactValue::List(categories)) = facts.get(&"app.categories".into()) {
        targets.extend(categories.iter().cloned().map(ResolutionTarget::Category));
    }
    targets.push(ResolutionTarget::Default);
    let mut result = Resolution {
        facts,
        ..Resolution::default()
    };
    for target in targets {
        let selection_target = match &target {
            ResolutionTarget::App(app) => Some(ProfileTarget::App(app.clone())),
            ResolutionTarget::Category(category) => Some(ProfileTarget::Category(category.clone())),
            _ => None,
        };
        let override_profile = selection_target.as_ref().and_then(|target| {
            catalog
                .profile_for
                .iter()
                .find(|o| &o.target == target && o.surface.as_ref() == Some(&surface.name))
                .or_else(|| {
                    catalog
                        .profile_for
                        .iter()
                        .find(|o| &o.target == target && o.surface.is_none())
                })
                .map(|o| o.profile.clone())
        });
        let mut candidates: Vec<_> = catalog
            .profiles
            .iter()
            .filter(|p| match &target {
                ResolutionTarget::Locked => p.locked,
                ResolutionTarget::Default => p.default,
                _ => p.for_target == selection_target,
            })
            .map(|p| explanation(catalog, &result.facts, p))
            .collect();
        let available: Vec<_> = candidates.iter().filter(|p| p.available).collect();
        let conflict = available.iter().enumerate().any(|(i, a)| {
            available[i + 1..].iter().any(|b| {
                (!a.conditional && !b.conditional)
                    || (a.conditional && b.conditional && a.priority == b.priority)
            })
        });
        let selected = if let Some(reference) = &override_profile {
            catalog
                .profiles
                .iter()
                .find(|p| &p.reference == reference)
                .map(|p| explanation(catalog, &result.facts, p))
                .filter(|p| p.available && p.when_matches)
                .map(|p| p.profile)
        } else if conflict {
            None
        } else {
            available
                .into_iter()
                .filter(|p| p.when_matches)
                .max_by_key(|p| (p.conditional, p.priority))
                .map(|p| p.profile.clone())
        };
        if let Some(reference) = &override_profile
            && !candidates.iter().any(|p| &p.profile == reference)
            && let Some(profile) = catalog.profiles.iter().find(|p| &p.reference == reference)
        {
            candidates.push(explanation(catalog, &result.facts, profile));
        }
        result.steps.push(ResolutionStep {
            target,
            candidates,
            conflict,
            override_profile,
            selected: selected.clone(),
        });
        if selected.is_some() {
            result.selected = selected;
            break;
        }
    }
    if let Some(profile) = result
        .selected
        .as_ref()
        .and_then(|reference| catalog.profiles.iter().find(|p| &p.reference == reference))
    {
        result.slots = profile
            .slots
            .iter()
            .map(|slot| {
                let candidates: Vec<_> = slot
                    .candidates
                    .iter()
                    .map(|reference| SetCandidateExplanation {
                        set: reference.clone(),
                        availability: set_availability(catalog, reference),
                    })
                    .collect();
                let selected = candidates
                    .iter()
                    .find(|c| c.availability == SetAvailability::Available)
                    .map(|c| c.set.clone());
                SlotExplanation {
                    slot: slot.id.clone(),
                    candidates,
                    selected,
                }
            })
            .collect();
    }
    result
}
fn set_availability(catalog: &Catalog, reference: &SetRef) -> SetAvailability {
    let Some(owner) = catalog.plugin(&reference.plugin) else {
        return SetAvailability::MissingPlugin;
    };
    if !owner.enabled {
        return SetAvailability::DisabledPlugin;
    }
    let Some(set) = catalog.sets.iter().find(|s| &s.reference == reference) else {
        return SetAvailability::MissingSet;
    };
    if let Some(provider) = &set.logic {
        let Some(logic) = catalog.plugin(provider) else {
            return SetAvailability::MissingLogic;
        };
        if !logic.has_logic {
            return SetAvailability::MissingLogic;
        }
        if !logic.enabled {
            return SetAvailability::DisabledLogic;
        }
    }
    SetAvailability::Available
}
fn explanation(catalog: &Catalog, facts: &Facts, profile: &ProfileSpec) -> CandidateExplanation {
    CandidateExplanation {
        profile: profile.reference.clone(),
        available: catalog
            .plugin(&profile.reference.plugin)
            .is_some_and(|p| p.enabled),
        conditional: profile.when.is_some(),
        when_matches: profile.when.as_ref().is_none_or(|c| c.evaluate(facts)),
        priority: profile.priority,
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingProfile {
    pub target: Option<ProfileRef>,
    pub deadline: Timestamp,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileSwitch {
    pub active: Option<ProfileRef>,
    pub pending: Option<PendingProfile>,
}
impl ProfileSwitch {
    /// Returns true when the active profile changes. Pending target changes retain the deadline.
    /// Pending expiry is explicit: the caller performs capture/callback cleanup before `expire`.
    pub fn request(
        &mut self,
        now: Timestamp,
        target: Option<ProfileRef>,
        contacts: usize,
        delay_ms: u64,
    ) -> bool {
        if target == self.active {
            self.pending = None;
            return false;
        }
        if contacts == 0 {
            self.active = target;
            self.pending = None;
            return true;
        }
        let deadline = self
            .pending
            .as_ref()
            .map_or(now.after(delay_ms), |p| p.deadline);
        self.pending = Some(PendingProfile { target, deadline });
        // The core must cancel captures and discard physical work before expiry.
        false
    }
    pub fn contacts_finished(&mut self, contacts: usize) -> bool {
        if contacts != 0 {
            return false;
        }
        self.commit_pending()
    }
    pub fn expire(&mut self, now: Timestamp) -> bool {
        if self.pending.as_ref().is_some_and(|p| p.deadline <= now) {
            self.commit_pending()
        } else {
            false
        }
    }
    fn commit_pending(&mut self) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        self.active = pending.target;
        true
    }
}
