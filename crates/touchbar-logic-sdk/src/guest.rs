use touchbar_logic_types::CallContext;
use touchbar_model::*;

/// One logic value lives in one wasm instance. No host I/O or wall clock is exposed.
pub trait Logic: Sized {
    fn describe() -> LogicDescription;
    fn init(
        config: &str,
        persisted: StatePatch,
        ctx: &CallContext,
    ) -> Result<(Self, LogicUpdate), String>;
    fn handle(&mut self, event: LogicEvent, ctx: &CallContext) -> Result<LogicUpdate, String>;
}

pub struct GuestInstance<L> {
    logic: Option<L>,
}
impl<L> Default for GuestInstance<L> {
    fn default() -> Self {
        Self { logic: None }
    }
}
impl<L: Logic> GuestInstance<L> {
    pub fn init(
        &mut self,
        config: &str,
        persisted: StatePatch,
        ctx: &CallContext,
    ) -> Result<LogicUpdate, String> {
        self.logic = None;
        let (logic, update) = L::init(config, persisted, ctx)?;
        debug_budget(&update);
        self.logic = Some(logic);
        Ok(update)
    }
    pub fn handle(&mut self, event: LogicEvent, ctx: &CallContext) -> Result<LogicUpdate, String> {
        let update = self
            .logic
            .as_mut()
            .ok_or_else(|| "logic is not initialized".to_owned())?
            .handle(event, ctx)?;
        debug_budget(&update);
        Ok(update)
    }
}

/// Native exports are empty; the wasm bridge and generated ABI stay outside author logic.
#[macro_export]
macro_rules! export {
    ($logic:ty) => {
        #[cfg(target_arch = "wasm32")]
        mod __touchbar_guest {
            use super::*;
            use $crate::bindings::exports::wstouchbar::logic::api as api;
            std::thread_local! {
                static STATE: std::cell::RefCell<$crate::GuestInstance<$logic>> =
                    std::cell::RefCell::new($crate::GuestInstance::default());
            }
            struct Guest;
            impl api::Guest for Guest {
                fn describe() -> api::Description {
                    $crate::init_logging();
                    $crate::translation::description_to(<$logic as $crate::Logic>::describe())
                }
                fn init(config: String, persisted: api::StatePatch, ctx: api::CallContext) -> Result<api::Update, String> {
                    $crate::init_logging();
                    let persisted = $crate::translation::state_from(persisted);
                    for warning in persisted.warnings { $crate::log::warn!("{warning}"); }
                    STATE.with(|cell| {
                        let mut state = cell.try_borrow_mut().map_err(|_| "reentrant logic call".to_owned())?;
                        state.init(&config, persisted.value, &ctx.into()).map($crate::translation::update_to)
                    })
                }
                fn handle(event: api::Event, ctx: api::CallContext) -> Result<api::Update, String> {
                    $crate::init_logging();
                    let event = $crate::translation::event_from(event)?;
                    STATE.with(|cell| {
                        let mut state = cell.try_borrow_mut().map_err(|_| "reentrant logic call".to_owned())?;
                        state.handle(event, &ctx.into()).map($crate::translation::update_to)
                    })
                }
            }
            #[allow(unsafe_code)]
            mod bindings {
                use super::Guest;
                $crate::bindings::export!(Guest with_types_in $crate::bindings);
            }
        }
    };
}

fn debug_budget(update: &LogicUpdate) {
    debug_assert!(
        touchbar_logic_types::check_budget(update, &touchbar_logic_types::LogicLimits::default())
            .is_ok(),
        "logic update exceeds the default wire budget"
    );
}
