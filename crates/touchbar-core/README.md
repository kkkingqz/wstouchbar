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

The profile switch and timer integration are tested independently. Touch capture
membership and deadline cancellation connect in T7. Instance lifecycle, panels,
bindings and effects are added in T6–T10; their inputs acquire recipients there.
