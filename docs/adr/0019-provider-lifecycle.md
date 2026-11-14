# Executable provider lifecycle

Provider declarations bind a service to a package entry point. Resolution preserves those bindings alongside package order and exclusivity. A declaration without an entry point may refer to a native/source provider; an unambiguous sole WASM entry point is selected automatically.

`ProviderRegistry` stages and initializes each selected executable provider, then exposes dispatch, serialized module-state snapshots, and shutdown. The external card fixture proves initialization, a real rules dispatch, snapshot, and shutdown through this path.

A provider trap marks it `Failed`; later dispatch and snapshot are rejected. No vanilla or fallback implementation is substituted. Stopped providers likewise cannot continue. Services resolving to multiple providers are rejected by the current registry rather than partially committing several host transactions.

This is single-provider lifecycle evidence, not a universal multi-provider composition engine. WIT-generated component bindings require a component-model adapter distinct from the current core-WASM buffer ABI and are not claimed.
