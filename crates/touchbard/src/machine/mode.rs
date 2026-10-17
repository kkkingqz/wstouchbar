//! USB/display transition policy. The shell tags every asynchronous result with its generation.
use serde::Serialize;
use touchbar_model::Timestamp;
use touchbar_protocol::Mode;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum State {
    Native {
        safe: bool,
    },
    ToDisplay {
        generation: u64,
        deadline: Timestamp,
    },
    Display {
        generation: u64,
    },
    ToNative {
        generation: u64,
        deadline: Timestamp,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum DriverState {
    Pending { deadline: Timestamp },
    Ready { mode: u8 },
    Missing,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Device(Option<u64>),
    Target(Mode),
    RefreshDisplay,
    DisplayReady { generation: u64 },
    NativeSafe { generation: u64 },
    Failed { generation: u64, message: String },
    Driver { device: u64, mode: Option<u8> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    RefreshSwapchain { generation: u64, device: u64 },
    ToDisplay { generation: u64, device: u64 },
    ToNative { generation: u64, device: u64 },
    ReleaseHardware { generation: u64 },
}
#[derive(Clone, Debug)]
pub struct ModeMachine {
    state: State,
    driver: DriverState,
    device: Option<u64>,
    discovered: bool,
    stable: Option<Timestamp>,
    requested: Mode,
    failures: u8,
    retry: Option<Timestamp>,
    generation: u64,
    native_operation: Option<u64>,
    error: Option<String>,
    now: Timestamp,
}
impl Default for ModeMachine {
    fn default() -> Self {
        Self::new()
    }
}
impl ModeMachine {
    pub fn new() -> Self {
        Self {
            state: State::Native { safe: false },
            driver: DriverState::Missing,
            device: None,
            discovered: false,
            stable: None,
            requested: Mode::Native,
            failures: 0,
            retry: None,
            generation: 0,
            native_operation: None,
            error: None,
            now: Timestamp(0),
        }
    }
    pub fn discovered(&self) -> bool {
        self.discovered
    }
    pub fn device(&self) -> Option<u64> {
        self.device
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn driver(&self) -> &DriverState {
        &self.driver
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn failures(&self) -> u8 {
        self.failures
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn target_for_available_device(&self, desired: Mode) -> Mode {
        if self.device.is_some() {
            desired
        } else {
            Mode::Native
        }
    }
    pub fn target(&self) -> Mode {
        if self.device.is_some() {
            self.requested
        } else {
            Mode::Native
        }
    }
    pub fn native_safe(&self) -> bool {
        matches!(self.state, State::Native { safe: true })
    }
    pub fn handle(&mut self, now: Timestamp, input: Input) -> Vec<Command> {
        self.now = self.now.max(now);
        let mut commands = Vec::new();
        self.expire(&mut commands);
        let previous = self.target();
        match input {
            Input::Device(device) if !self.discovered || device != self.device => {
                self.discovered = true;
                self.device = device;
                self.stable = device.map(|_| self.now.after(2000));
                self.driver = DriverState::Missing;
                self.error = None;
                if let Some(device) = device {
                    self.begin_native(device, &mut commands);
                } else {
                    self.bump();
                    self.native_operation = None;
                    self.state = State::Native { safe: true };
                    commands.push(Command::ReleaseHardware {
                        generation: self.generation,
                    });
                }
            }
            Input::Target(mode) => self.requested = mode,
            Input::RefreshDisplay if matches!(self.state, State::Display { .. }) => {
                self.bump();
                self.native_operation = None;
                self.driver = DriverState::Missing;
                self.state = State::ToDisplay {
                    generation: self.generation,
                    deadline: self.now.after(5000),
                };
                commands.push(Command::RefreshSwapchain {
                    generation: self.generation,
                    device: self.device.expect("display has device"),
                });
            }
            Input::DisplayReady { generation } if matches!(self.state,State::ToDisplay{generation:g,..} if g==generation) =>
            {
                self.state = State::Display { generation };
                self.failures = 0;
                self.retry = None;
                self.error = None;
            }
            Input::NativeSafe { generation } if self.native_operation == Some(generation) => {
                self.native_operation = None;
                self.state = State::Native { safe: true };
                if !matches!(self.driver, DriverState::Ready { .. }) {
                    self.driver = DriverState::Pending {
                        deadline: self.now.after(10000),
                    };
                }
            }
            Input::Failed {
                generation,
                message,
            } if generation == self.generation => {
                self.error = Some(message);
                if matches!(self.state, State::ToDisplay { .. } | State::Display { .. }) {
                    self.fail_display(&mut commands);
                } else if matches!(self.state, State::ToNative { .. }) {
                    self.state = State::Native { safe: false };
                }
            }
            Input::Driver { device, mode } if self.device == Some(device) => {
                self.driver = match mode {
                    Some(mode @ 0..=3) => DriverState::Ready { mode },
                    _ => DriverState::Missing,
                };
            }
            _ => {}
        }
        if self.target() != previous {
            self.failures = 0;
            self.retry = None;
        }
        self.reconcile(&mut commands);
        commands
    }
    pub fn advance(&mut self, now: Timestamp) -> Vec<Command> {
        self.now = self.now.max(now);
        let mut commands = Vec::new();
        self.expire(&mut commands);
        self.reconcile(&mut commands);
        commands
    }
    fn bump(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("device operation id exhausted");
    }
    fn begin_native(&mut self, device: u64, commands: &mut Vec<Command>) {
        self.bump();
        self.native_operation = Some(self.generation);
        self.state = State::ToNative {
            generation: self.generation,
            deadline: self.now.after(3000),
        };
        commands.push(Command::ToNative {
            generation: self.generation,
            device,
        });
    }
    fn fail_display(&mut self, commands: &mut Vec<Command>) {
        self.failures = self.failures.saturating_add(1);
        self.retry = if self.failures < 3 {
            Some(self.now.after(1000u64 << (self.failures - 1)))
        } else {
            None
        };
        if let Some(device) = self.device {
            self.begin_native(device, commands);
        }
    }
    fn expire(&mut self, commands: &mut Vec<Command>) {
        match self.state {
            State::ToDisplay { deadline, .. } if deadline <= self.now => {
                self.error = Some("display transition timed out".into());
                self.fail_display(commands);
            }
            State::ToNative { deadline, .. } if deadline <= self.now => {
                self.error = Some("native safety transition timed out".into());
                self.state = State::Native { safe: false };
            }
            _ => {}
        }
        if matches!(self.driver,DriverState::Pending{deadline} if deadline<=self.now) {
            self.driver = DriverState::Missing;
        }
    }
    fn reconcile(&mut self, commands: &mut Vec<Command>) {
        if self.target() == Mode::Native {
            if matches!(self.state, State::Display { .. } | State::ToDisplay { .. })
                && let Some(device) = self.device
            {
                self.begin_native(device, commands);
            }
        } else if matches!(self.state, State::Native { .. })
            && self.failures < 3
            && self.stable.is_some_and(|at| at <= self.now)
            && self.retry.is_none_or(|at| at <= self.now)
            && let Some(device) = self.device
        {
            self.bump();
            self.native_operation = None;
            self.driver = DriverState::Missing;
            self.state = State::ToDisplay {
                generation: self.generation,
                deadline: self.now.after(5000),
            };
            commands.push(Command::ToDisplay {
                generation: self.generation,
                device,
            });
        }
    }
    pub fn next_deadline(&self) -> Option<Timestamp> {
        let transition = match self.state {
            State::ToDisplay { deadline, .. } | State::ToNative { deadline, .. } => Some(deadline),
            _ => None,
        };
        let driver = if let DriverState::Pending { deadline } = self.driver {
            Some(deadline)
        } else {
            None
        };
        let retry = if self.target() == Mode::Display
            && self.failures < 3
            && matches!(self.state, State::Native { .. })
        {
            self.stable.map(|at| at.max(self.retry.unwrap_or(at)))
        } else {
            None
        };
        [transition, driver, retry].into_iter().flatten().min()
    }
}
