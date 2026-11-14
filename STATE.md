# Project state

## Direction

Tantalus Engine is an independent Rust-native PC FFIX runtime and creation platform. Vanilla gameplay parity comes first; enhancements are optional addons. No Unity, CLR, original executable, managed game assemblies, or original game-specific native plugin fallback.

## Coordination

- Integration owner: local Kimi session.
- Execution: local; no subagents or external workers active.
- Primary platform: Windows x64.
- Recovery: small task-scoped commits; game installations remain read-only.

## Task status

- `T01` — Verified for source audit, synthetic inspection, and CLI scope. Evidence: `tasks/T01.md`.
- `T02` — Verified for stable IDs, transactional registries, aliases/renames, and item instances. Evidence: `tasks/T02.md`.
- Reference source checkout: verified at `c716e1faaeee8c856cc10921234d017f1187b45b`.
- Owned FFIX installation: not supplied. Real-install checks are NOT_RUN.
- Public repository: https://github.com/finalpantasy/tantalus-engine.

## Next dependencies

T03 implements the headless simulation, explicit clocks, input tapes, command ordering, and causal traces. T02 supplies the stable identity and revision vocabulary. Real reference scenario selection still requires an owned installation or an explicitly bounded source-only substitute.
