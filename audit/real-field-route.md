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

## TextAsset payload verification

The real TextAsset layout is length-prefixed `m_Name`, raw `m_Script`, and `m_PathName`, with four-byte alignment. Rust decodes the selected `.bgi` (12,544 bytes) and `.bgs` (21,888 bytes); SHA-256 values match UnityPy exactly. Metadata is recorded in `audit/text-assets-p0data101.json`. Payloads remain local and are not committed.

## Texture2D payload verification

All 115 real textures use Unity format 12 (DXT5) with embedded data. The reader handles the exact format-15 layout and validates block-compressed sizes. A real 1024×1024 atlas matches UnityPy's compressed SHA-256 and vertically oriented RGBA SHA-256 exactly. See `audit/texture-p0data101.json`. No image payload is committed.

## BGI navigation verification

The source-backed BGI parser handles triangles, vertex/edge lists, floors, normals, and animation frame references. The selected real field contains 203 triangles, 164 vertices, 609 edges, 3 floors, 2 normals, and no animation list. Independent raw-header decoding matches every count and offset. Evidence: `audit/bgi-field.json`.

## Remaining dependency

Parse BGS scene/camera/overlay structures, connect BGI navigation to movement, and validate field composition and events against the owned installation. Real gameplay has not been run.
