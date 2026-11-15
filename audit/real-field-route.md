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

DXT5 color always uses four-color interpolation. Alpha interpolation now uses seven interpolated entries when `first > second` and five otherwise; the endpoint weights are `(7 - index)` and `(5 - index)` respectively. Synthetic tests cover both branches.

## AssetBundle container and selected-field atlas

The format-15 AssetBundle reader decodes its preload table, path-to-asset container, main asset, compatibility fields, dependencies, and streamed-scene flag. For `FBG_N00_TSHP_MAP001_TH_CGR_0`, the real container path `assets/resources/fieldmaps/fbg_n00_tshp_map001_th_cgr_0/atlas.png` resolves to Texture2D path ID `-7866888658012748271` (preload index 111, size 1, file ID 0).

That selected atlas is 2048×2048 DXT5 with 4,194,304 compressed bytes. Its compressed SHA-256 is `8b816640497f6d323a4db1dbe3284016e62a30b2a0bd665dd192b5bf51103bb7`; its vertically oriented RGBA SHA-256 is `bd5ede6e0a9864d3692db045915f43c726500fbb0ee1391ede6c72abc44e3506`. Both match UnityPy 1.25.0 exactly. The CLI can resolve the pointer with `--atlas-for`, select it unambiguously with `--texture-id`, and optionally write a local diagnostic buffer with `--rgba-output`.

## BGI navigation verification

The source-backed BGI parser handles triangles, vertex/edge lists, floors, normals, and animation frame references. The selected real field contains 203 triangles, 164 vertices, 609 edges, 3 floors, 2 normals, and no animation list. Independent raw-header decoding matches every count and offset. Evidence: `audit/bgi-field.json`.

## BGS scene verification

The source-backed BGS parser handles scene headers, cameras, overlays, packed sprite parameters/locations, lights, and animation frames. The selected real field has 2 cameras, 22 overlays, 1,704 sprites, 2 animations with 10 frames, and no lights. Scene length 21,888 and all header counts/offsets match independent raw decoding. Evidence: `audit/bgs-field.json`.

## Renderer-independent mapping

`FieldMapper` now maps the real BGS+BGI data into public field-scene contracts. The real result preserves projection 529, actor anchor (-130, 0, -648), 203 navigation triangles, 22 overlays, and 1,704 sprites. Mapping covers atlas rectangles, pixel offsets, shifted depth, alpha/transparency, visibility, and camera identity. Synthetic tests cover invalid cameras and depth overflow. Evidence: `audit/field-mapping.json`.

## Native capture evidence

`BundledField` resolves the actual BGS/BGI/Texture2D dependency set and gives the atlas a stable `ffix:field/fbg_n00_tshp_map001_th_cgr_0/atlas` identity. `tantalus-client capture-field` uploads its decoded RGBA to Bevy and applies the mapped sprites without a placeholder image. Atlas rectangles retain their source coordinates while native sprite dimensions remain independent of texture resolution.

After capture warmup was changed to count received GPU frames, two independent 320×240 runs produced byte-identical nonempty PNGs with SHA-256 `94d9867915f025045010f318ab97b3ae043c89b00c16c46d0847dcc528445f52`. Inspection shows incorrect tile placement concentrated in part of the image. This proves real texture upload/render/readback only, not correct field composition or parity. Synthetic 2D capture also passes; synthetic 3D remains blank.

## Remaining dependency

Correct field origin/projection, camera/overlay filtering, alpha/transparency, and composition against reference evidence; then connect BGI navigation to movement and real event execution. Real gameplay has not been run.
