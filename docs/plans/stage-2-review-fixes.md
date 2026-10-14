# Stage 2 review fixes

Source: REVIEW-STAGE-2-2026-10-13.md. Execute sequentially, commit and push each item.

1. Admission budgets: shared wire estimates, core catalog/Init/subscription/effect guards, spawn output bounds, TOML configuration boundary, runtime host rejection without guest destruction. D78: reset all restored state when Init exceeds budget, warn, retain config. Prove overflow and recovery with regression tests, then workspace fmt/clippy/tests.
2. Epoch ticker: sleep indefinitely when idle, RAII activation around create/call, retain hard interruption. Test idle counter stability and real timeout/isolation, run workspace checks.
3. Worker compiler: Send compiler and opaque prepared module; install on runtime thread; preserve immutable revision/error caching and reject foreign engines. Test worker handoff and unchanged live instance, run workspace checks.
4. Module retention: evict unused successful and failed revisions after accepted catalog; preserve live Store. Test eviction and live calls, run workspace checks.
5. Fault disposal: remember full failed instance generations until disposal acknowledgment; retain unknown/duplicate warnings. Test call and creation failures, run workspace checks.

Review focus: custom budgets, envelope overhead, stale effect/subscription messages, idle ticker races and shutdown, worker engine identity, eviction lifetime, failed creation versus duplicate live creation. Final fresh whole-change review after all five items.
