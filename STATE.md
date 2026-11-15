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
- `T04` — Importer/cache plus bounded real UnityRaw objects, TextAssets, DXT5 textures, AssetBundle atlas associations, BGI, and BGS verified; full dependency closure and real field presentation remain Building. Evidence: `tasks/T04.md` and `audit/real-field-route.md`.
- `T05` — Synthetic 2D/3D and coherent real selected-camera field capture verified; original-reference comparison remains open. Evidence: `tasks/T05.md`.
- `T06` — Event VM, field scheduling, BGI navigation, and real `.eb` archive parsing verified; real event execution and full opcode coverage remain pending. Evidence: `tasks/T06.md` and `audit/event-binary.json`.
- `T07` — Verified for additive packages, world actors, encounter ownership, persistence, and respawn. Evidence: `tasks/T07.md`.
- `T08` — Synthetic battle/action/effect/reward foundation verified; real original ability compatibility remains pending. Evidence: `tasks/T08.md`.
- `T09` — Synthetic encounter lifecycle, mapping, cancellation, and presentation-command foundation verified; real visual/camera integration remains pending. Evidence: `tasks/T09.md`.
- `T10` — Native atomic saves, recovery, migration boundaries, exactly-once outcomes, and Memoria binary sidecar codec verified. Base-save AES/schema decoding and real-save checks remain Building/NOT_RUN. Evidence: `tasks/T10.md`.
- `T11` — Resolver/profile locks, Wasmtime ABI, packaged entry points, external action/card fixtures, pure migration, and single-provider lifecycle verified. WIT component bindings, multi-provider transactions, and multi-module migration remain Building. Evidence: `tasks/T11.md`.
- `T12` — Typed headless real-field navigation scenario verified; event, battle, reward, and save integration remain Building. Evidence: `tasks/T12.md`.
- Architecture review — integrated in `docs/REVIEW_2026-11-14.md` and `audit/review-ledger.csv`; diagnostic policy verified, backend selection and real-reference probes remain open.
- Reference source checkout: verified at `c716e1faaeee8c856cc10921234d017f1187b45b`.
- Owned FFIX installation: PC copy inspected read-only; runtime original/unknown with an associated effect plugin, not confirmed Memoria. Original code execution remains NOT_RUN; bounded real asset decoding is verified. Evidence: `audit/real-install.json`.
- Public repository: https://github.com/finalpantasy/tantalus-engine.

## Next dependencies

Preserve entry 1 function tag 1's genuine truncated custom-variable error (`0xAF`, missing high byte at 92), preserve entry 6 function tag 1's verified truncated conditional jump starting at offset 526 and ending at 624, preserve entry 0's truncated `MESB` instruction at offset 1107 after correcting `0xD0`/`0xD1` mappings, then decode entry 3 function tag 1's unsupported event opcode `180` (`0xB4`) at offset 647; entries 4 and 5 remain rejected for truncating `BSFLOOR` before its argument flag, and connect interactions and encounters; then integrate the original battle route, rewards, and native save/reload into T12. Compare the coherent field render against an original reference. Real gameplay and visual parity remain unverified. T11 component bindings and multi-provider/migration orchestration remain later maturity work. T10 base-save AES/schema decoding and real-save round trips remain open.
