# Content identity and instance allocation

Tantalus uses validated namespaced `ContentId` values for author-facing content. Legacy numeric IDs are adapter data, not the native identity model.

Registry patches are transactional. `additions` introduce new records, `replacements` intentionally overwrite existing records, `renames` move a record and retain its old ID as an alias, and `aliases` add alternate names for canonical records. Aliases cannot target other aliases or create cycles.

`ContentRevision` is a local registry sequence, not a global content hash. A profile/content fingerprint will identify exact resolved packs in the persistence work.

`ItemInstanceId` allocation is deterministic within an inventory, starting at 1. Resolved affix values and custom properties are stored on each instance; a generator seed alone is not sufficient persistent state.
