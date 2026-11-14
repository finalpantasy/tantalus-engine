# First real field route and UnityRaw evidence

Owned source: `E:/SteamLibrary/steamapps/common/FINAL FANTASY IX`. The initial field candidate is `FBG_N00_TSHP_MAP001_TH_CGR_0`, selected from the local `.ff9mapkit-field-index.json` into `p0data101.bin`.

## Observed format evidence

- The real bundle begins with `UnityRaw`, format 3, player version `5.x.x`, engine version `5.2.3p2`.
- `tantalusctl assets inspect-bundle` reads one entry, `CAB-ebad16f7dde378a09a285fac6d8f6fc3`, at relative offset 52 / absolute offset 112, size 249,871,040 bytes. That range ends exactly at the 249,871,152-byte file boundary.
- Entry offsets are relative to the streamed-content base at `headerSize`, not to the directory's end. The real read caught and corrected that mapping.
- UnityPy 1.25.0 was already installed and used only as a development reference. Its local `UnityPy/files/BundleFile.py:55-94` defines UnityRaw header/directory parsing; `File.py:76-90` defines entry reads. No Python runtime is used by Tantalus.
- Reference object inspection reports 501 TextAsset, 115 Texture2D, and one AssetBundle object in the serialized CAB. Field assets include the candidate's `.bgs` and `.bgi` data.

Only metadata and generated reports are committed. Original assets, CAB payloads, and extracted textures remain outside public fixtures.

## Serialized-file verification

The Rust format-15 reader now matches UnityPy's real CAB inventory: 617 objects (501 TextAsset, 115 Texture2D, one AssetBundle) across class types 28, 49, and 142. The implementation handles embedded 24-byte type-tree nodes, little-endian metadata after the big-endian header, and aligned 64-bit object IDs. Format 15 does not consume the older `big_id_enabled` field. Reference instrumentation established type boundaries at bytes 37/883/1234/2513 and object metadata at 2517.

Summary evidence is in `audit/serialized-p0data101.json`. Object payloads are not yet decoded or redistributed.

## Remaining dependency

Implement the specific TextAsset/Texture2D payload readers required by this route. Then validate field composition, event references, and presentation against the owned installation. Real gameplay has not been run.
