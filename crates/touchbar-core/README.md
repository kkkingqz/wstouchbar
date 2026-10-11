# touchbar-core

Deterministic state machines with explicit `Timestamp` inputs and command outputs.
The crate performs no I/O and does not read clocks.

The T4 foundation provides configuration validation, independent surfaces and
modes, facts/catalog storage, generation-checked layout and frame responses,
timer scheduling, and reference-counted key leases with teardown releases.
Desktop key routing is explicit and requires the backend's capability.

T5 adds profile resolution with an explanation of candidates, conflicts and
user choices; the Active/Pending switch; and scene reconciliation per surface.
Placements preserve their IDs when slot, set, config and content revisions match.
Changes to a shared logic provider recreate its consumers. Static sets do not
create a wasm runtime instance.

The profile switch waits for captures and unfinished physical logic calls,
with an unchanged hard deadline (D46).

T6 adds instance lifecycle, description validation (including record bindings),
correlated Init/Handle calls, an ordered call queue, state patch validation, and
fault isolation with retry generations and capped exponential backoff. Adjacent
pending ValueChanged events for the same action/record coalesce; a running call
and activation barriers retain their order. Ordinary Handle errors advance the
queue; Init/load errors retry, schema errors wait for a catalog reload.

T7 adds surface/contact capture routing, button holds, frame-coalesced sliders,
scrubber scrolling/record selection and key widgets. Targets remain stable until
Up/Cancel. Layout removal, geometry changes, faults and teardown cancel captures;
key leases always release. Gesture thresholds use the surface density. Renderer
scrubber geometry and host interaction state implement D47 independently of plugin
state. Presentation/layer cancellation connects in T8/T9; effects in T10. T11 wires persisted
fields into Init and adds full CoreState restore. Until then Init receives an
empty persisted patch. Wasm execution and its time/memory budgets belong to the
runtime stage; this crate validates state types, finite geometry and field limits.
