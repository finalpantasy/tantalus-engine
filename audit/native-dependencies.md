# Native dependency inventory

The independent runtime must not use these original or reference-specific components as a hidden fallback.

| Component | Current evidence | Project treatment |
|---|---|---|
| Unity runtime | Memoria architecture documentation; source audit in progress | Reference only; replace required behavior |
| Mono/CLR and original managed assemblies | Memoria architecture documentation; source audit in progress | Reference only; never loaded by the independent runtime |
| Original FFIX executable and game-specific native plugins | Master-plan constraint | Reference only; fingerprint separately when an installation is available |
| `FF9SpecialEffectPlugin` | Pinned source contains a dedicated project; detailed audit pending | Reimplement required effect behavior in Rust |
| `SaXAudio` / SoLoud | Memoria project overview identifies an audio API and dynamic library; exact current linkage pending | Audit codecs and replace required behavior with an explicitly selected stack |
| FMV and audio codecs | Unknown | Inventory actual files and decoder dependencies before selecting a runtime stack |
| Memoria release binaries | Not acquired or hashed | Never infer source/binary equivalence from an upload date |

Third-party Rust crate native/transitive dependencies will be recorded when introduced. A Rust wrapper around a native library is not a Rust reimplementation of that library.
