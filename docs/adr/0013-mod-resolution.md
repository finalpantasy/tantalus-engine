# Mod resolution and shared profile locks

Package identity is separate from content identity. Package IDs such as `fixture.raptor` remain stable strings; records and services use namespaced `ContentId` values. `ModuleLock` therefore stores package IDs as strings and includes the selected version, feature set, content hash, requirement status, and save-schema version.

`ModResolver` parses semantic versions and compatibility ranges, then uses deterministic backtracking when a later dependency rejects a previously selected version. Optional dependencies remain inactive unless an enabled feature declares them. Dependencies load before dependents; explicit load-order hints and package IDs break remaining ties. Cycles, missing packages, invalid features/ranges, duplicate candidate versions, and incompatible engine/SDK ranges are specific errors.

Duplicate additive declarations are rejected even when an earlier declaration was a replacement. Explicit replacement records are allowed in resolved order, including for base-game or legacy IDs not declared by a selected package. Multiple providers may share a service in load order unless any declaration is exclusive, in which case multiple providers conflict before activation.

Resolution emits the same `ProfileLock` consumed by native saves. Its content fingerprint commits to the base revision and ordered module locks. Missing required modules block ordinary save continuation without deleting module-owned data.

This milestone does not execute WebAssembly. WASM capability negotiation, deterministic limits, transactional rollback, trusted compiled-code caching, and external ARPG/card/loot fixtures remain T11 work.
