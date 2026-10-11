use crate::{DesktopCapabilities, KeyDestination, KeyLeaseSnapshot, Mode, Output};
use serde::{Deserialize, Serialize};
use touchbar_model::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyOwner {
    Request(RequestId),
    Logic(InstanceId),
    Contact {
        instance: InstanceId,
        contact: ContactId,
    },
}
type Lease = KeyLeaseSnapshot;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeyTracker {
    leases: Vec<Lease>,
}

impl Lease {
    fn same_target(&self, other: &Self) -> bool {
        self.key == other.key
            && self.destination == other.destination
            && (self.destination == KeyDestination::Desktop || self.surface == other.surface)
    }
    fn output(&self, phase: KeyPhase) -> Output {
        match self.destination {
            KeyDestination::Surface => Output::SendKey {
                surface: self.surface,
                key: self.key,
                phase,
            },
            KeyDestination::Desktop => Output::DesktopKey {
                surface: self.surface,
                key: self.key,
                phase,
            },
        }
    }
}
impl KeyTracker {
    pub fn snapshot(&self) -> Vec<KeyLeaseSnapshot> {
        self.leases.clone()
    }
    pub fn press(
        &mut self,
        owner: KeyOwner,
        surface: &SurfaceInfo,
        mode: Mode,
        desktop: DesktopCapabilities,
        key: SystemKey,
    ) -> Result<Vec<Output>, EffectError> {
        if mode == Mode::Native {
            return Err(EffectError::Unavailable);
        }
        let destination = if surface.caps.inject_keys {
            KeyDestination::Surface
        } else if desktop.key {
            KeyDestination::Desktop
        } else {
            return Err(EffectError::Unsupported);
        };
        let lease = Lease {
            owner,
            surface: surface.id,
            key,
            destination,
        };
        if self
            .leases
            .iter()
            .any(|l| l.owner == owner && l.surface == surface.id && l.same_target(&lease))
        {
            return Ok(vec![]);
        }
        let output = if self.leases.iter().any(|l| l.same_target(&lease)) {
            vec![]
        } else {
            vec![lease.output(KeyPhase::Pressed)]
        };
        self.leases.push(lease);
        Ok(output)
    }
    pub fn release_owner(&mut self, owner: KeyOwner) -> Vec<Output> {
        self.release_matching(|l| l.owner == owner)
    }
    pub fn release_surface(&mut self, surface: SurfaceId) -> Vec<Output> {
        self.release_matching(|l| l.surface == surface)
    }
    pub fn release_desktop(&mut self) -> Vec<Output> {
        self.release_matching(|l| l.destination == KeyDestination::Desktop)
    }
    pub fn release_key(&mut self, owner: KeyOwner, key: SystemKey) -> Vec<Output> {
        self.release_matching(|l| l.owner == owner && l.key == key)
    }
    fn release_matching(&mut self, matches: impl Fn(&Lease) -> bool) -> Vec<Output> {
        let mut releases = vec![];
        let mut i = 0;
        while i < self.leases.len() {
            if !matches(&self.leases[i]) {
                i += 1;
                continue;
            }
            let lease = self.leases.remove(i);
            if !self.leases.iter().any(|other| other.same_target(&lease)) {
                releases.push(lease.output(KeyPhase::Released));
            }
        }
        releases
    }
}
