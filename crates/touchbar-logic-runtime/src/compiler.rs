use crate::{HostState, LoadError, bindings};
use wasmtime::{
    Engine,
    component::{Component, Linker},
};

/// Worker-owned compiler; preparation needs no access to live instances.
#[derive(Clone)]
pub struct Compiler {
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<HostState>,
}
/// Validated compiled component. Can be sent back to its runtime thread.
pub struct PreparedModule {
    pub(crate) engine: Engine,
    pub(crate) pre: bindings::LogicPre<HostState>,
}
impl Compiler {
    pub fn prepare(&self, bytes: &[u8]) -> Result<PreparedModule, LoadError> {
        let component =
            Component::new(&self.engine, bytes).map_err(|e| LoadError(format!("{e:#}")))?;
        let ty = component.component_type();
        let exports: Vec<_> = ty
            .exports(&self.engine)
            .map(|(name, _)| name.to_owned())
            .collect();
        if !exports
            .iter()
            .any(|name| name == "wstouchbar:logic/api@1.0.0")
        {
            return Err(LoadError(format!(
                "expected export wstouchbar:logic/api@1.0.0; found {exports:?}"
            )));
        }
        for (name, import) in ty.imports(&self.engine) {
            let allowed = matches!(
                (name, import.ty),
                (
                    "log",
                    wasmtime::component::types::ComponentItem::ComponentFunc(_)
                ) | (
                    "log-level",
                    wasmtime::component::types::ComponentItem::Type(_)
                )
            );
            if !allowed {
                return Err(LoadError(format!("forbidden import {name}")));
            }
        }
        let pre = self
            .linker
            .instantiate_pre(&component)
            .and_then(bindings::LogicPre::new)
            .map_err(|e| LoadError(format!("v1 interface types: {e:#}")))?;
        Ok(PreparedModule {
            engine: self.engine.clone(),
            pre,
        })
    }
}
