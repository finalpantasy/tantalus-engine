# Simulation and presentation clock

The authoritative simulation has no Bevy, GPU, or live asset-worker dependencies. Resource readiness and failure enter as tick-stamped `RecordedExternalEvent` values tied to session and content revision.

`Simulation::step` is transactional. Invalid input, stale completions, and malformed timelines leave state unchanged. State digests use canonical serialization of authoritative state and configuration; trace text and presentation output are excluded.

The presentation clock emits single-tick `TickInput` bundles. This preserves command timing when one rendered frame catches up multiple simulation ticks. `TickInput::stride` remains available for explicit authoritative timeline batching; input-sensitive runners should not merge commands from different ticks into one bundle.

Compatibility mode freezes at an unresolved readiness barrier. Continuing-world mode advances while consuming the same recorded readiness/failure tape. Neither mode polls Bevy or GPU state.

Detailed traces have a fixed 1024-entry default bound. RNG configuration and seed are recorded, but no gameplay RNG algorithm is implemented yet; only the explicit `none` algorithm is accepted.
