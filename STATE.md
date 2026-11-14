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
- `T04` — Importer/cache and real UnityRaw directory reading verified; serialized Unity objects and real field presentation remain Building. Evidence: `tasks/T04.md` and `audit/real-field-route.md`.
- `T05` — Client/contracts verified; visual proof remains failed after blank Tantalus and official Bevy control captures. Evidence: `tasks/T05.md`.
- `T06` — Event VM and field scheduling foundation verified; real field execution remains pending. Evidence: `tasks/T06.md`.
- `T07` — Verified for additive packages, world actors, encounter ownership, persistence, and respawn. Evidence: `tasks/T07.md`.
- `T08` — Synthetic battle/action/effect/reward foundation verified; real original ability compatibility remains pending. Evidence: `tasks/T08.md`.
- `T09` — Synthetic encounter lifecycle, mapping, cancellation, and presentation-command foundation verified; real visual/camera integration remains pending. Evidence: `tasks/T09.md`.
- `T10` — Native atomic saves, recovery, migration boundaries, exactly-once outcomes, and Memoria binary sidecar codec verified. Base-save AES/schema decoding and real-save checks remain Building/NOT_RUN. Evidence: `tasks/T10.md`.
- `T11` — Resolver/profile locks, Wasmtime ABI, packaged entry points, external action/card fixtures, pure migration, and single-provider lifecycle verified. WIT component bindings, multi-provider transactions, and multi-module migration remain Building. Evidence: `tasks/T11.md`.
- Architecture review — integrated in `docs/REVIEW_2026-11-14.md` and `audit/review-ledger.csv`; diagnostic policy verified, backend selection and real-reference probes remain open.
- Reference source checkout: verified at `c716e1faaeee8c856cc10921234d017f1187b45b`.
- Owned FFIX installation: PC copy inspected read-only; runtime original/unknown with an associated effect plugin, not confirmed Memoria. Real execution/asset decoding remains NOT_RUN. Evidence: `audit/real-install.json`.
- Public repository: https://github.com/finalpantasy/tantalus-engine.

## Next dependencies

Parse the selected field's `.bgs`/`.bgi` structures and connect scene/navigation data to T05/T06. Real UnityRaw, serialized object-table, TextAsset, and DXT5 Texture2D decoding are verified. T11 component bindings and multi-provider/migration orchestration remain later maturity work. T10 base-save AES/schema decoding and real-save round trips remain open. T05 visual failure remains open.
