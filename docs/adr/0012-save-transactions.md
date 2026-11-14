# Native save transactions and legacy boundaries

Native saves are versioned `SaveEnvelope` values containing the exact profile lock, checkpoint semantics, stable entity/item/encounter identities, independent item properties, RNG state, module-owned sections, migration history, unknown optional sections, and committed outcome markers. Public contracts do not contain renderer or mod-runtime types. Package locks use stable package-ID strings; namespaced `ContentId` values identify module state sections and content records.

`SaveStore` writes an immutable digest-protected generation and syncs it before atomically replacing `current.json`. Generations link to their predecessors. An interruption before pointer publication leaves the previous save authoritative; an interruption reported after pointer publication requires recovery to observe whether the commit became durable. Orphaned generations are never promoted merely because they are newer.

`commit_outcome` requires exactly one matching resolution marker, retains prior outcomes, and rejects a resolution already present in the authoritative save. The reward effects and marker must already be represented together in the supplied snapshot. This is the persistence boundary; individual game adapters still need to construct that snapshot correctly.

Loading returns a `LoadPlan`. Missing required modules, profile changes, and format migrations block ordinary continuation. Migration is an explicit in-memory operation that leaves source generations untouched. Unknown optional sections remain in the envelope.

The legacy adapter currently decodes the source-backed binary SimpleJSON sidecar and validates its location-derived filename pairing. Original base saves use a separate encrypted fixed-layout container. Base AES decryption, schema mapping, real-save verification, and representable legacy export remain pending.
