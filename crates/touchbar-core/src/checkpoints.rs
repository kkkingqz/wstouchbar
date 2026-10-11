use crate::*;

/// Version of the complete continuation state, independent of the status view.
pub const CORE_STATE_VERSION: u32 = 1;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoreState {
    pub format_version: u32,
    core: Core,
}
impl CoreState {
    pub fn config(&self) -> &CoreConfig {
        &self.core.config
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreError {
    Version { found: u32, supported: u32 },
    Config(ModelError),
}
impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version { found, supported } => write!(
                f,
                "unsupported core state version {found}; supported: {supported}"
            ),
            Self::Config(error) => write!(f, "invalid checkpoint configuration: {error}"),
        }
    }
}
impl std::error::Error for RestoreError {}
impl Core {
    pub fn checkpoint(&self) -> CoreState {
        CoreState {
            format_version: CORE_STATE_VERSION,
            core: self.clone(),
        }
    }
    pub fn restore(state: CoreState) -> Result<Self, RestoreError> {
        if state.format_version != CORE_STATE_VERSION {
            return Err(RestoreError::Version {
                found: state.format_version,
                supported: CORE_STATE_VERSION,
            });
        }
        state.core.config.validate().map_err(RestoreError::Config)?;
        Ok(state.core)
    }
}
