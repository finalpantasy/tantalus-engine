<div align="center">

# TANTALUS ENGINE

### One stage. Infinite stories.

**An independent Rust runtime and creation platform for Final Fantasy IX.**

[![Early development](https://img.shields.io/badge/status-early%20development-blueviolet)](https://github.com/finalpantasy/tantalus-engine)
[![Rust](https://img.shields.io/badge/rust-1.96-orange?logo=rust)](https://www.rust-lang.org/)
[![Windows x64](https://img.shields.io/badge/initial%20platform-Windows%20x64-blue)](https://github.com/finalpantasy/tantalus-engine)
[![Support on Ko-fi](https://img.shields.io/badge/Ko--fi-support%20the%20project-ff5e5b?logo=kofi)](https://ko-fi.com/finalpantasy)

</div>

---

## The vision

Tantalus Engine is a ground-up Rust implementation of the PC FFIX runtime. The first destination is vanilla gameplay parity. After that, the same engine services will power optional enhancements, new content, and total conversions.

The original game is the compatibility target—not the ceiling.

- **Play faithfully.** Original fields, events, battles, progression, audio, UI, saves, and minigames.
- **Create freely.** Add enemies, attacks, weapons, quests, locations, characters, and entirely new rules.
- **Replace systems.** Build action combat, ARPG loot, new card games, or other total conversions without forking the engine.
- **Use modern assets.** Open authoring formats, high-resolution textures, new rigs, 3D scenes, and editable effects.
- **Choose your presentation.** Classic transitions or seamless encounters; original arenas or the current world scene.

## Current status

**The engine is at its first foundation milestone. It does not run FFIX yet.**

The current source includes:

- A read-only PC installation inspector.
- Streaming SHA-256 fingerprints and structured diagnostics.
- A JSON-capable `tantalusctl` command-line tool.
- Deterministic content packs with bounded imports, atomic cache revisions, and dependency validation.
- Synthetic tests for missing resources, incorrect file types, non-ASCII paths, and unchanged source files.
- A pinned Memoria reference baseline with 22 initial capability groups and 204 configuration bindings.
- A Bevy 0.19.1 client driven by renderer-independent scene snapshots.
- A source-backed original-event VM subset and deterministic field scheduler.
- Optional per-tick diagnostics with verification-on-request instead of mandatory normal-play hashing.
- A synthetic battle engine with multi-hit actions, semantic effects/audio, interruption, and exactly-once rewards.

No original game assets are included. UnityFS containers are recognized, but real AssetBundle decoding is not implemented. Headless visual verification is failing after blank Tantalus and official Bevy control captures; see [`tasks/T05.md`](tasks/T05.md). No compatibility or performance claim is being made beyond the checks recorded in [`tasks/`](tasks/).

## Planned platform

| Area | Destination |
|---|---|
| Runtime | Independent Rust simulation and native presentation |
| Compatibility | Complete vanilla FFIX gameplay and audited Memoria capabilities |
| Rendering | High-FPS presentation, layered 2D, hybrid scenes, and full 3D |
| Encounters | Classic battles, visible world enemies, and seamless in-world combat |
| Content | Additive enemies, attacks, equipment, item instances, quests, and campaigns |
| Modding | Versioned packages, sandboxed modules, trusted extensions, and replaceable providers |
| Authoring | Live workshop, asset importers, validation, undo, packaging, and migration tools |
| Automation | Shared CLI, API, headless scenarios, replay tools, and optional MCP support |
| Reliability | Atomic saves, profile rollback, migrations, diagnostics, and reproducible tests |

## Roadmap

- [x] **T01 — Baseline and inspection:** reference pins, capability ledger, and read-only installation reports.
- [x] **T02 — Content identity:** stable IDs, additive registries, and distinct item instances.
- [x] **T03 — Simulation:** deterministic clocks, input tapes, commands, and traces.
- [x] **T04 foundation:** deterministic asset packs, atomic cache revisions, and UnityFS recognition.
- [ ] **Real PC assets:** Unity AssetBundle decoding and real field/model verification.
- [ ] **T05 — Native presentation:** structural client checks pass; visual proof remains unresolved.
- [x] **T06 foundation:** original-event decoding, expressions, waits, windows, movement, and field scheduling.
- [ ] **T04–T06 — First field:** real asset decoding, verified native presentation, and the complete event VM.
- [ ] **Independent game loop:** field → interaction → battle → rewards → save/reload, without Unity or Memoria.
- [x] **T07 — Additive world content:** staged packages, visible actors, perception, pursuit, and persistent outcomes.
- [x] **T08 — Battle action foundation:** costs, targeting, multi-hit effects, outcomes, and reward recovery.
- [ ] **Creation architecture:** seamless encounters and external total-conversion fixtures.
- [ ] **Full compatibility:** complete FFIX behavior and the audited Memoria baseline.
- [ ] **Creator release:** launcher, workshop, stable SDK, documentation, API, and MCP.

Detailed status lives in [`STATE.md`](STATE.md). The fidelity rules are in [`docs/PRESERVATION.md`](docs/PRESERVATION.md). Reference provenance is tracked in [`audit/provenance.md`](audit/provenance.md).

## Try the current inspector

Tantalus requires Rust 1.96 or newer for the pinned toolchain.

```bash
cargo test --workspace
cargo run -p tantalus-tools --bin tantalusctl -- --help
```

Inspect an owned PC installation:

```bash
cargo run -p tantalus-tools --bin tantalusctl -- \
  install inspect --root "C:\Path\To\FINAL FANTASY IX" --json
```

Inspection reads files only. It does not install Tantalus, modify the game, or import assets yet.

## Architecture

```text
                 ┌────────────────────────────┐
                 │ Launcher / Workshop / CLI  │
                 │            MCP             │
                 └──────────────┬─────────────┘
                                │ Shared commands
                 ┌──────────────▼─────────────┐
                 │ Versioned public contracts │
                 └──────────────┬─────────────┘
                                │
          ┌─────────────────────┼─────────────────────┐
          │                     │                     │
┌─────────▼─────────┐ ┌─────────▼─────────┐ ┌─────────▼─────────┐
│ Headless simulation│ │ Assets and legacy │ │ Native presentation│
│ and game modules   │ │ compatibility     │ │ and mod host       │
└────────────────────┘ └───────────────────┘ └────────────────────┘
```

Bevy is the initial private presentation backend. Wasmtime is planned for sandboxed executable mods. Neither framework's private types will become the save format or public mod contract.

## For creators

The platform is being designed for different skill levels:

- Players select reproducible profiles and recover saves safely.
- Artists import models, textures, animation, and audio.
- Writers and translators edit dialogue with context and previews.
- Designers create content through structured editors.
- Programmers use a documented SDK or replace entire systems.
- Automation users access the same operations through CLI, API, and MCP.

AI assistance will be optional. Local packages and offline play will not require an account.

## Support the project

If you want to support development, you can do so on Ko-fi:

**[ko-fi.com/finalpantasy](https://ko-fi.com/finalpantasy)**

## References and ownership

Tantalus uses the original PC game and [Memoria](https://github.com/Albeoris/Memoria) as development references. It is an independent implementation, not a Unity wrapper or a Memoria fork.

Final Fantasy IX and related marks belong to their respective owners. Tantalus is an unofficial project and is not affiliated with or endorsed by Square Enix. Users must provide their own legally owned game assets. Existing mod assets remain their creators' property.
