use super::{Device, DisplayNodes, NativeNode, Roots, SwitchPlan, discover};
use std::io;
use touchbar_protocol::Mode;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Device(Option<u64>),
    NativeSafe {
        generation: u64,
    },
    DisplayNodesReady {
        generation: u64,
        device: u64,
        nodes: DisplayNodes,
    },
    NativeDriver {
        device: u64,
        node: Option<NativeNode>,
    },
}
#[derive(Clone, Debug)]
struct Pending {
    generation: u64,
    device: u64,
    mode: Mode,
    selected: bool,
    reported: bool,
}
#[derive(Clone, Debug)]
pub struct Tracker {
    roots: Roots,
    current: Option<(u64, Device)>,
    serial: u64,
    scanned: bool,
    pending: Option<Pending>,
    native: Option<NativeNode>,
}
impl Tracker {
    pub fn new(roots: Roots) -> Self {
        Self {
            roots,
            current: None,
            serial: 0,
            scanned: false,
            pending: None,
            native: None,
        }
    }
    pub fn current(&self) -> Option<(u64, &Device)> {
        self.current.as_ref().map(|(id, device)| (*id, device))
    }
    pub fn roots(&self) -> &Roots {
        &self.roots
    }
    pub fn begin_switch(
        &mut self,
        generation: u64,
        device: u64,
        mode: Mode,
    ) -> io::Result<SwitchPlan> {
        let Some((id, current)) = &self.current else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Touch Bar is absent",
            ));
        };
        if *id != device {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "stale Touch Bar identity",
            ));
        }
        let plan = SwitchPlan::new(current.clone(), mode);
        self.pending = Some(Pending {
            generation,
            device,
            mode,
            selected: false,
            reported: false,
        });
        Ok(plan)
    }
    /// Called only when release/modprobe/USB writes completed successfully on the worker.
    pub fn selected(&mut self, generation: u64) -> bool {
        if let Some(p) = &mut self.pending
            && p.generation == generation
        {
            p.selected = true;
            true
        } else {
            false
        }
    }
    pub fn failed(&mut self, generation: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.generation == generation)
        {
            self.pending = None;
        }
    }
    pub fn rescan(&mut self) -> io::Result<Vec<Change>> {
        let snapshot = discover(&self.roots)?;
        let mut changes = Vec::new();
        let changed = self.current.as_ref().map(|(_, d)| d) != snapshot.as_ref().map(|s| &s.device);
        if changed || !self.scanned {
            self.scanned = true;
            self.pending = None;
            self.native = None;
            self.current = if let Some(s) = &snapshot {
                self.serial = self
                    .serial
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("device identity exhausted"))?;
                Some((self.serial, s.device.clone()))
            } else {
                None
            };
            changes.push(Change::Device(self.current.as_ref().map(|(id, _)| *id)));
        }
        if let Some((snapshot, (device, _))) = snapshot.as_ref().zip(self.current.as_ref()) {
            if let Some(p) = &mut self.pending
                && p.device == *device
                && p.selected
                && !p.reported
            {
                match p.mode {
                    Mode::Native if snapshot.native_safe() => {
                        p.reported = true;
                        changes.push(Change::NativeSafe {
                            generation: p.generation,
                        });
                    }
                    Mode::Display if snapshot.config == Some(2) && snapshot.display.is_some() => {
                        p.reported = true;
                        changes.push(Change::DisplayNodesReady {
                            generation: p.generation,
                            device: *device,
                            nodes: snapshot.display.clone().expect("checked"),
                        });
                    }
                    _ => {}
                }
            }
            if self.native != snapshot.native {
                self.native = snapshot.native.clone();
                changes.push(Change::NativeDriver {
                    device: *device,
                    node: self.native.clone(),
                });
            }
        }
        Ok(changes)
    }
    /// Watch roots are exact directories; the monitor filters missing ancestors by component.
    pub fn watch_paths(&self) -> Vec<std::path::PathBuf> {
        let mut paths = [
            "bus/usb/devices",
            "class/drm",
            "class/input",
            "bus/hid/drivers/hid-appletb-kbd",
            "bus/usb/drivers/appletbdrm",
        ]
        .into_iter()
        .map(|p| self.roots.sys.join(p))
        .collect::<Vec<_>>();
        if let Some((_, device)) = &self.current {
            paths.push(device.path().into());
        }
        paths
    }
}
