# Diagnostic and verification policy

Normal play may omit full-state hashing and detailed trace storage. `SimulationConfig.diagnostics` controls per-step hashing and trace retention; `TickInput::requesting_digest()` requests verification at a selected checkpoint. `Simulation::state_digest()` remains available for replay-envelope initialization.

`TickResult.state_digest` is optional. Replay scenarios enable `hash_every_step`, while ordinary runs can request only the checkpoints they need. Detailed domain events retain causal IDs even when detailed trace storage is disabled.

Diagnostic settings are excluded from the gameplay configuration digest. A regression test runs identical input through normal and verification profiles and requires identical events and final state. No performance improvement is claimed from this policy without a separate measured benchmark.
