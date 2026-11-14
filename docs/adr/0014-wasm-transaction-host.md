# Experimental WASM transaction host

Tantalus pins Wasmtime 49.0.5 with reduced features for the initial Windows/Rust 1.96 target. The host compiles portable WASM bytes through `Module::new`; it does not deserialize mod-supplied native code or treat a hash as proof that a compiled artifact is trustworthy.

The experimental ABI version 1 exports `memory`, `alloc`, `tantalus_abi_version`, and `tantalus_invoke`. JSON input/output buffers and the sole `tantalus.emit` host import are bounded and validated. No clock, random, filesystem, network, or arbitrary-memory host imports are provided.

Every callback runs in a fresh guest instance with explicit serialized module state. Guest memory and private globals therefore cannot become hidden authoritative state across callbacks. Guest output and host commands are staged; state writes, ID allocation, events, and module state commit only after the callback and every command validate. Traps, fuel exhaustion, invalid output, resource failures, and command-limit violations leave the last committed host/module state intact.

This deliberately favors rollback clarity over callback speed. Batched APIs, WIT-generated SDK bindings, package entry-point resolution, module migrations, released-module regression fixtures, and useful-work benchmarks remain necessary before SDK stability or ARPG suitability claims. External ARPG/card/loot fixtures have not yet been built.
