# First real field route and UnityRaw evidence

Owned source: `E:/SteamLibrary/steamapps/common/FINAL FANTASY IX`. The initial field candidate is `FBG_N00_TSHP_MAP001_TH_CGR_0`, selected from the local `.ff9mapkit-field-index.json` into `p0data101.bin`.

## Observed format evidence

- The real bundle begins with `UnityRaw`, format 3, player version `5.x.x`, engine version `5.2.3p2`.
- `tantalusctl assets inspect-bundle` reads one entry, `CAB-ebad16f7dde378a09a285fac6d8f6fc3`, at relative offset 52 / absolute offset 112, size 249,871,040 bytes. That range ends exactly at the 249,871,152-byte file boundary.
- Entry offsets are relative to the streamed-content base at `headerSize`, not to the directory's end. The real read caught and corrected that mapping.
- UnityPy 1.25.0 was already installed and used only as a development reference. Its local `UnityPy/files/BundleFile.py:55-94` defines UnityRaw header/directory parsing; `File.py:76-90` defines entry reads. No Python runtime is used by Tantalus.
- Reference object inspection reports 501 TextAsset, 115 Texture2D, and one AssetBundle object in the serialized CAB. Field assets include the candidate's `.bgs` and `.bgi` data.

Only metadata and generated reports are committed. Original assets, CAB payloads, and extracted textures remain outside public fixtures.

## Remaining dependency

Implement Unity serialized-file headers, object tables, and the specific TextAsset/Texture2D readers required by this route. Then validate field composition, event references, and presentation against the owned installation. Real gameplay has not been run.
