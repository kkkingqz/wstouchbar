//! touchbar-logic-sdk: stage 2 implementation.

/// Generated guest ABI; ordinary SDK code inherits the deny lint.
#[allow(unsafe_code)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "logic",
        pub_export_macro: true,
    });
}
