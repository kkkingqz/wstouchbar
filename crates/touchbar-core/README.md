# touchbar-core

Deterministic state machines with explicit `Timestamp` inputs and command outputs.
The crate performs no I/O and does not read clocks.

The T4 foundation provides configuration validation, independent surfaces and
modes, facts/catalog storage, generation-checked layout and frame responses,
timer scheduling, and reference-counted key leases with teardown releases.
Desktop key routing is explicit and requires the backend's capability.

Profile resolution, instance lifecycle, touch captures, panels, bindings, and
effects are added in T5–T10. Their input variants are defined here; until the
corresponding state machines exist they have no recipients. Timer purposes and
fact notifications are connected as those subsystems are implemented.
