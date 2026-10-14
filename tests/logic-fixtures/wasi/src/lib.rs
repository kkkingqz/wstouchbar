#![cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
use bindings::exports::wstouchbar::logic::api;
struct Guest;
fn empty() -> api::Update {
    api::Update {
        state: vec![],
        requests: vec![],
        cancel: vec![],
        subscriptions: None,
        presentation: None,
    }
}
impl api::Guest for Guest {
    fn describe() -> api::Description {
        api::Description {
            actions: vec![],
            state: vec![],
        }
    }
    fn init(_: String, _: api::StatePatch, _: api::CallContext) -> Result<api::Update, String> {
        let _ = bindings::wasi::cli::environment::get_environment();
        Ok(empty())
    }
    fn handle(_: api::Event, _: api::CallContext) -> Result<api::Update, String> {
        Ok(empty())
    }
}
#[allow(unsafe_code)]
mod exports {
    use super::Guest;
    crate::bindings::export!(Guest with_types_in crate::bindings);
}
