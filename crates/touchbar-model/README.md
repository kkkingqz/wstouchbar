# touchbar-model

Shared domain data for the pure core, bindings, loaders and future runtime.
The only runtime dependency is serde. There is no I/O, clock access or renderer.

- IDs and plugin/set references serialize without losing generation ownership.
- `Timestamp` is monotonic milliseconds supplied by the caller.
- Surface dimensions and density drive derived facts and physical thresholds.
- `Length` accepts numbers, `Nu`, `N%w`, `N%h`; `UnitSize` accepts positive
  surface percentages. Conversion returns unrounded pixels; layout rounds later.
- Catalog vectors preserve declaration and candidate order. Validation reports
  individual issues, including conflicting profiles; loaders decide which plugin
  or selection target to exclude. A missing `requires` dependency is valid.
- Set/profile data contains only the parts consumed by stage 1. Rendering styles
  and TOML parsing are added in later stages.
- Trigger and generic binding data are defined here to avoid a dependency cycle:
  catalogs contain bindings, and the binding engine consumes model types.
- `LogicCall::Init` and `Handle` are host invocations. The WIT event variant has
  no `Init`; runtime dispatches to the corresponding export.
- Effect policy resolves optional overrides against kind defaults. It does not
  execute requests, read D-Bus JSON or manage subscriptions.

Canvas commands follow the original project's WIT at `d2f9dfd`, keeping curves
as path segments instead of renderer-specific tessellated vertices.
