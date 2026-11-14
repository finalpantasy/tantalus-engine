# External card-provider fixture

`tests/fixtures/synthetic/card_mode` is an independent Rust Cargo project compiled to `wasm32-unknown-unknown`. Its source, lockfile, release WASM, and SHA-256 are retained alongside the action fixture for future behavioral regression checks.

The package resolves as the sole exclusive provider for `fixture:service/minigame_cards`. No vanilla card provider is selected in that profile. The unchanged generic `ModuleHost` loads the module and mediates all state and reward mutations through the public ABI.

The synthetic rules use named card definitions and unique card instances. A two-round match verifies deterministic highest-power AI selection, scoring, match completion, one reward allocation, progression/UI events, and no duplicate reward on a later callback. Two hosts produce identical results.

This is not Tetra Master compatibility, a complete card-game UI, or evidence of full scene/provider lifecycle integration. Those remain separate compatibility and presentation work.
