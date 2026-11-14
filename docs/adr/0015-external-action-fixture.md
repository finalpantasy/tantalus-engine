# Independently compiled action-mode fixture

The public SDK is a normal workspace crate under `sdk/rust/tantalus-sdk`; the logical transaction interface is declared in `sdk/wit/module.wit`. These are experimental source contracts, not stable or WIT-generated bindings.

`tests/fixtures/synthetic/action_mode` is an independent Cargo project. It compiles Rust to `wasm32-unknown-unknown`, exports the core ABI, parses length-delimited host input, returns serialized module state, and uses only `tantalus.emit` for world mutations and events. Its source, Cargo lockfile, release WASM, and SHA-256 are retained so later SDK changes can test behavioral compatibility against the same module.

The fixture demonstrates direct movement, stable target selection, attack cooldowns, enemy pursuit, affixed loot allocation, and HUD events across six host-driven frames. World input comes from previously committed host state. Loot is allocated once and not repeated after victory.

This is synthetic rules evidence only. It does not disable the vanilla combat provider, render a game, supply a campaign, or prove card replacement. Rebuild identity is verified locally; no performance or cross-target determinism claim is made.
