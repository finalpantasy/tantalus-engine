# WASM module-state migration

`ModuleMigration` binds a target `ProfileLock`, verified staged package, entry point, and module-owned save section. It clones the source envelope, restores that section's serialized state into a fresh `ModuleHost`, and invokes an explicit `operation: migrate` request with source/target schema versions.

Migration is pure with respect to gameplay: host state writes, ID allocations, and emitted events reject the transaction. Only the cloned section state/schema, migration history, and target profile are updated after success. Downgrades, missing sections/packages, traps, and validation failures leave the source envelope unchanged.

An independently compiled Rust WASM fixture proves schema 1-to-2 migration while preserving existing state, plus source preservation after a trap. Its source, lockfile, binary, and SHA-256 are retained.

Current scope is one explicitly bound section per call. Multi-module orchestration, module-specific section declarations, and composition with native format migration remain later work. WIT-generated bindings and full provider lifecycle are still open.
