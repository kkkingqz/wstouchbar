//! touchbar-logic-runtime: stage 2 implementation.

/// Generated host ABI from the same versioned contract as the guest.
#[allow(unsafe_code)]
pub mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "logic" });
}
