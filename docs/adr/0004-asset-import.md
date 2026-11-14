# Asset import and cache publication

The importer produces content-addressed objects and immutable revision directories. A revision is visible only after all assets, references, hashes, and its manifest have been written. Incomplete staging directories and unreferenced objects are harmless and never replace a complete revision.

The source audit shows that Memoria passes `p0data*.bin` files to Unity AssetBundle APIs. Tantalus recognizes the `UnityFS` container signature but does not claim to decode it yet. Real object decoding, codec coverage, and FFIX field/model verification are separate gates requiring source-compatible fixtures and an owned installation.

The initial general importer supports raw binary and UTF-8 loose resources. Other codecs fail explicitly. Stable content identities and source paths are recorded independently, and request enumeration order does not affect the resulting manifest.
