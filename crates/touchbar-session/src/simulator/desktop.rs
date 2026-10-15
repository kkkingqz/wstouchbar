use touchbar_desktop::{BackendEvent, CommandFuture, DesktopBackend, DesktopCommand, DesktopError};
/// Simulator commands must never inject keys into the user's desktop.
pub struct SimulatorDesktop {
    inner: Box<dyn DesktopBackend>,
}
impl SimulatorDesktop {
    pub fn new(inner: Box<dyn DesktopBackend>) -> Self {
        Self { inner }
    }
}
impl DesktopBackend for SimulatorDesktop {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn start(&mut self, events: async_channel::Sender<BackendEvent>) -> Result<(), DesktopError> {
        self.inner.start(events)
    }
    fn command(&self, command: DesktopCommand) -> CommandFuture {
        if matches!(
            command,
            DesktopCommand::Key { .. } | DesktopCommand::Shortcut { .. }
        ) {
            tracing::info!(event = "simulator_desktop_command", ?command);
            Box::pin(std::future::ready(Ok(())))
        } else {
            self.inner.command(command)
        }
    }
}
