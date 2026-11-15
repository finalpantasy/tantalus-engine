# Text rendering path (A8)

## Finding: where the vanilla install gets its dialog font

The unmodified install carries its dialog font. The earlier note (font bundle absent from the
install tree) looked for a loose file; the bundle is a TextAsset.

- `resources.assets` holds TextAssets `i_fa.mpc`, `p_fa.mpc` and `a_fa.mpc` (about 1.84 MB each).
  `AssetManagerUtil.GetPlatformPrefix` maps Windows to `p`, so `EncryptFontManager.LoadFont`
  reads `EmbeddedAsset/FA/p_fa.mpc`.
- The vanilla `Managed/Assembly-CSharp.dll` contains `EncryptFontManager`, `ByteEncryption`, the
  strings `EmbeddedAsset/FA`, `_fa.mpc` and the NGUI types (`UIFont`, `BMFont`, `NGUIText`).
- Decrypt is the keyless block rotation already in `font_bundle::decrypt`. The result is a
  `UnityWeb` format 3 bundle with one LZMA level (59 bytes of header, then an LZMA-alone stream:
  1,838,609 bytes compressed, 3,647,672 uncompressed). It was not `UnityRaw` like the Memoria
  copy that was inventoried earlier. `font_bundle::open_bundle` now decompresses it with
  `lzma-rs` 0.3.0 (MIT) and the existing type-tree reader takes over.
- Content: one Unity `Font` object, `TBUDGoStd-Bold` (m_FontData 3,639,592 bytes, an sfnt file),
  empty texture, no characters. All three platform bundles carry the same font name. Garnet and
  Alexandria (in the Memoria copy) are not in the install.
- No class 128 (Font) object exists in any of `resources.assets`, `sharedassets*`, `level*` or
  `mainData`; no UIFont MonoBehaviour exists either (script counts in
  `audit/ui-atlas-inventory.json`). The 2,879 UILabels use a TrueType font, not an NGUI bitmap
  font, and the 22 atlases hold no glyph sprites.
- Therefore the glyph source is a TrueType rasterizer fed by `TBUDGoStd-Bold`. NGUI measures and
  places glyphs from Unity `CharacterInfo` (advance, minX/maxX, minY/maxY) at the label's pixel
  size.

Caveat: the vanilla `UILabel` code is decompiled-binary only; the code path above is read from
the pinned Memoria source, which replaces the original font with this same default font when
`Configuration.Font.Names` is empty. That the vanilla binary reaches `TBUDGoStd-Bold` through the
same call is inferred from the matching strings, not read from vanilla source.

## Dialog label settings (install data)

The dialog prefab in `sharedassets2.assets` has consecutive objects: `Tail` (path id 336),
`Phrase` (338), `Border` (339), `Body` (340). Decoded by reading the MonoBehaviour tail fields
(no embedded type trees; layout matched against the `UILabel` field order in Memoria):

| Field | Value |
|---|---|
| UILabel font size | 42 |
| UILabel spacing X / Y | 0 / 26 (42 + 26 = 68 = `Dialog.DialogLineHeight`) |
| Alignment | 1 (Left) |
| Effect | 1 (Shadow), distance (4, 4), color (40, 40, 40) / 255 |
| Overflow | 1 (ClampContent) |
| Widget pivot | 0 (TopLeft); gradient off |
| `Body` UISprite | sprite `dialog_bg`, type 2 (Tiled), white, no anchors |
| `Border` UISprite | sprite `dialog_frame_chat`, type 1 (Sliced), anchored to the Body transform (path id 210) with absolute offsets -18, +18 on every side |
| `Tail` UISprite | sprite `dialog_pointer_*` family (not drawn) |

Border sprite names also come from `Dialog.Style`: `dialog_frame_chat` (Auto, NoTail),
`dialog_frame_info` (Plain), none (Transparent).

## Atlas

`FF9UIDataTool.WindowAtlas` returns `Gray Atlas` when `cfg.win_type == 0`, else `Blue Atlas`
(both in `sharedassets2.assets`, 13 sprites each, 512 x 256). Sprites used (name, x, y, w, h,
border l/r/t/b, padding l/r/t/b):

- `dialog_bg` 0,20,128,128, border 1, padding 0
- `dialog_frame_chat` 195,64,81,81, border 30, padding 9,0,9,0
- `dialog_frame_info` 195,147,109,109, border 40, padding 4,1,4,1
- `dialog_pointer_downleft|downright|topleft|topright` (tail, not drawn)

## Layout rules used (all from Memoria source)

- `NGUIText.WrapText`: width wrapping applies only when `dialog == null`; dialog labels break
  lines only at `\n`. Line top = previous top + `finalLineHeight` (68). `Line.Width` = pen x;
  `HeightBelow` starts at 68 and grows to `baseline - glyph.minY + yOffset`.
- `NGUIText.UpdateFontSizes`: `baseline = round_half_even((minY + maxY + size) / 2)` measured on
  `)` (or `A`).
- `GenerateTextRender`: glyph quad left = pen + minX, top = lineTop + baseline - maxY; the pen
  advances by `finalSpacingX + advance`. A character with no glyph, or advance 0, is skipped.
  The shadow copy is drawn at (+dx, +dy) = (4, 4) in the effect color before the glyph;
  `[HSHD]` shifts the text by a third of the distance.
- `DialogBoxSymbols.ParseSingleGenericTag`: `TextRGBA` (3 or 4 hex components push, no params
  pop, 1 param is global alpha), named colors keep the previous alpha, `MoveCaret`/`Tab`
  add `(dx - 4) * 4.8219` to x and `dy * 4.8214` to y, `DialogF` adds x, `DialogX` sets x,
  `DialogY` adds y.
- `ParseTextSplitTags`: `[PAGE]` starts a page, `[ENDN]` ends the entry.
- `Dialog.AutomaticSize`: width = `(maxLineWidth + 32) / m + 1`, times `m`, floor 140 px (from
  `WindowMinWidth - AdjustWidth` with m = 1543/320); height = `lines * 68 + 20`, lines =
  `ceil(rawHeight / 68)` over all pages, at least the `[STRT]` line hint. Hint mode:
  `(w + 3) * m` wide and `l * 68 + 20` tall.
- Body is the Tiled fill (tile = sprite minus 1 px border, grid origin at the body center);
  Border is the Sliced fill, rectangle = body plus 18 px per side, shrunk by the sprite padding
  (`UISprite.drawingDimensions`). Text origin = body left + 16, body top + 10.
- `UIManager.ResourceXMultipier` is `UIContentSize.x / PsxFieldWidth`; the non-widescreen
  content size is 1543 x 1080 and the field is 320 x 224. Widescreen changes the multiplier.

## Unknowns (kept, not guessed)

1. Glyph pixels use `fontdue` 0.9.5 (MIT OR Apache-2.0 OR Zlib; pulls `ttf-parser` 0.25.1, MIT OR
   Apache-2.0). Unity rasterizes with FreeType and its own hinting; no reference capture exists,
   so pixel agreement is unverified. Advance = `round(advance_width)` is an assumption about
   `CharacterInfo.advance`.
2. `win_type` default (Gray or Blue) is not in the source read; Blue is the CLI default.
3. `Dialog.CanAutoResize` depends on live event state; both sizing modes exist, Auto is default.
4. The `.mes` text assets repeat per language under the same name; variant index to language
   mapping is not established (CLI `--dialog-variant`).
5. Character names, party names, `[NUMB]`, `[ITEM]`, `[TEXT]`, icons, sprites, buttons, choice
   indent, alignment tags (`[CENT]`, `[JSTF]`, `[MIRR]`), `[WDTH]`, tail pointer and on-screen
   position are recorded or ignored, not rendered.
6. Speed, wait, time and sound markers are recorded with glyph indexes, not executed.
7. Prefab fields were read without embedded type trees; field order was matched to the Memoria
   `UILabel`/`UISprite` serialized order and the values were cross-checked (line height 68,
   `dialog_bg` and `dialog_frame_chat` strings present in the prefab bytes). Other prefab fields
   (depth, aspect ratio, tail anchors) were not interpreted.
8. `audit/font-inventory.json` still describes the Memoria `UnityRaw` copy (Garnet, TBUDGoStd-Bold,
   Alexandria); it was not regenerated for the install's `UnityWeb` bundle.

## Command

```
tantalusctl assets inspect-serialized --path <FF9_Data>/resources.assets \
  --render-dialog <mes name> <entry id> <out.png> \
  [--dialog-variant <n>] [--dialog-page <n>] [--dialog-window blue|gray] [--dialog-size auto|hint]
```

Output images are not committed.
