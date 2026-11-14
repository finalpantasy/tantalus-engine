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
- `T03` — Verified for deterministic simulation, replay, readiness barriers, clocks, and traces. Evidence: `tasks/T03.md`.
- `T04` — Importer/cache foundation verified; real Unity AssetBundle decoding remains Building. Evidence: `tasks/T04.md`.
- `T05` — Client/contracts verified; visual proof remains failed after blank Tantalus and official Bevy control captures. Evidence: `tasks/T05.md`.
- `T06` — Event VM and field scheduling foundation verified; real field execution remains pending. Evidence: `tasks/T06.md`.
- `T07` — Verified for additive packages, world actors, encounter ownership, persistence, and respawn. Evidence: `tasks/T07.md`.
- Architecture review — integrated in `docs/REVIEW_2026-11-14.md` and `audit/review-ledger.csv`; diagnostic policy verified, backend selection and real-reference probes remain open.
- Reference source checkout: verified at `c716e1faaeee8c856cc10921234d017f1187b45b`.
- Owned FFIX installation: not supplied. Real-install checks are NOT_RUN.
- Public repository: https://github.com/finalpantasy/tantalus-engine.

## Next dependencies

T08 supplies a synthetic battle/action path. Preserve the review's fidelity, cinematic, localization, reimport, and SDK-behavior requirements as those subsystems arrive. Real FFIX verification still requires codec work and an owned installation; T05 visual failure remains open.
