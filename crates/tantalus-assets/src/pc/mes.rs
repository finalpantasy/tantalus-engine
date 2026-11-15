//! `.mes` text parser, ported from the pinned Memoria C# sources:
//! `EmbadedSentenseLoader.cs` (entry splitting), `FF9TextTool.cs` (`[TXID=]`, `[STRT=]`,
//! `[LOADMES=]` handling) and `FFIXTextTag.cs` / `FFIXTextTagCode.cs` (tag decoding).
//!
//! On disk a `.mes` TextAsset is plain UTF-8 text. Entries end at `[ENDN]` (or start at
//! `[STRT=`); formatting is carried by bracket tags such as `[PAGE]` or `[FFCC00]`.
//! Anything that looks like a tag but is not defined by Memoria is kept as
//! [`MesToken::Unknown`] so it can be audited, never dropped.

use crate::install::ImportError;
use serde::{Deserialize, Serialize};

const MAX_TAG_LENGTH_HINT: usize = 64;

/// How a file is cut into entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MesLayout {
    /// `EmbadedSentenseLoader.ExtractSentenseEnd`: split on `[ENDN]`, optional `[TXID=n]`
    /// prefix per entry, ids count up from 0 (or from the last `[TXID]`).
    EndnSplit,
    /// `FF9TextTool.ExtractSentense`: entries begin at `[STRT=`; `[TXID=n]` sets the id of
    /// the following run of entries. Each entry keeps its trailing `[ENDN]`.
    StrtSplit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MesToken {
    /// Literal text (may contain `\n`).
    Text { text: String },
    /// A tag Memoria decodes (`FFIXTextTagCode`).
    Tag(MesTag),
    /// `[LOADMES=name]`: include another `.mes` file (FF9TextTool.ProcessLoadMessTags).
    LoadMes { name: String },
    /// A `[...]` or `{...}` span Memoria does not decode. Kept verbatim.
    Unknown { raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MesTag {
    /// `FFIXTextTagCode` member name, e.g. `DialogSize`, `NewPage`, `TextRGBA`.
    pub code: String,
    /// Parameters after Memoria normalization (character-name tags carry the keyword,
    /// color tags carry hex components as written).
    pub params: Vec<String>,
    /// `[/NAME]` closing form (Memoria stores it as the single param `Off`).
    pub closing: bool,
    /// Original source text of the tag.
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MesEntry {
    /// Effective text id (running index, or set by `[TXID=n]`).
    pub id: i32,
    pub tokens: Vec<MesToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MesFile {
    pub layout: MesLayout,
    pub entries: Vec<MesEntry>,
}

impl MesFile {
    /// Distinct unknown spans with occurrence counts, in first-seen order.
    pub fn unknown_tags(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for entry in &self.entries {
            for token in &entry.tokens {
                if let MesToken::Unknown { raw } = token {
                    match out.iter_mut().find(|(seen, _)| seen == raw) {
                        Some((_, count)) => *count += 1,
                        None => out.push((raw.clone(), 1)),
                    }
                }
            }
        }
        out
    }

    /// Re-renders the entry as the bracket text it was parsed from.
    pub fn entry_source(entry: &MesEntry) -> String {
        entry
            .tokens
            .iter()
            .map(|token| match token {
                MesToken::Text { text } => text.clone(),
                MesToken::Tag(tag) => tag.raw.clone(),
                MesToken::LoadMes { name } => format!("[LOADMES={name}]"),
                MesToken::Unknown { raw } => raw.clone(),
            })
            .collect()
    }
}

/// Chooses the layout the way the game's importers do: files that contain `[STRT=`
/// entries (field and battle dialogue) use `StrtSplit`, everything else `EndnSplit`.
pub fn detect_layout(text: &str) -> MesLayout {
    if text.contains("[STRT=") {
        MesLayout::StrtSplit
    } else {
        MesLayout::EndnSplit
    }
}

pub fn parse_mes_bytes(bytes: &[u8], source: &str) -> Result<MesFile, ImportError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| ImportError::invalid(source, format!("mes is not UTF-8: {error}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Ok(parse_mes(text, detect_layout(text)))
}

pub fn parse_mes(text: &str, layout: MesLayout) -> MesFile {
    let entries = match layout {
        MesLayout::EndnSplit => split_endn(text),
        MesLayout::StrtSplit => split_strt(text),
    }
    .into_iter()
    .map(|(id, body)| MesEntry {
        id,
        tokens: tokenize(&body),
    })
    .collect();
    MesFile { layout, entries }
}

/// `ExtractSentenseEnd` + `ProcessEntryId`.
fn split_endn(text: &str) -> Vec<(i32, String)> {
    let mut parts: Vec<&str> = text.split("[ENDN]").collect();
    if text.ends_with("[ENDN]") {
        parts.pop();
    }
    let mut id = 0_i32;
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let mut body = part;
        if body.starts_with("[TXID=")
            && let Some(end) = body.find(']')
        {
            // C# Int32.TryParse leaves `id` at 0 on failure.
            id = body[6..end].trim().parse().unwrap_or(0);
            body = &body[end + 1..];
        }
        out.push((id, body.to_owned()));
        id = id.wrapping_add(1);
    }
    out
}

/// `ExtractSentense`: blocks separated by `[TXID=`, then entries separated by `[STRT=`.
fn split_strt(text: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let starts_with_id = text.starts_with("[TXID=");
    let blocks: Vec<&str> = text.split("[TXID=").filter(|b| !b.is_empty()).collect();
    for (index, block) in blocks.iter().enumerate() {
        let mut id = 0_i32;
        let mut body = *block;
        if index > 0 || starts_with_id {
            let Some(end) = body.find(']') else { continue };
            id = body[..end].trim().parse().unwrap_or(0);
            body = &body[end + 1..];
            if body.is_empty() {
                continue;
            }
        }
        for sequence in body.split("[STRT=").skip(1) {
            out.push((id, format!("[STRT={sequence}")));
            id = id.wrapping_add(1);
        }
    }
    out
}

/// Scans text left to right exactly like `DialogBoxSymbols`: at each position try to read a
/// tag; if none, the character is literal.
pub fn tokenize(text: &str) -> Vec<MesToken> {
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut pos = 0;
    while pos < text.len() {
        let rest = &text[pos..];
        let first = rest.as_bytes()[0];
        if first == b'[' || first == b'{' {
            let close = if first == b'[' { ']' } else { '}' };
            if let Some(rel) = rest[1..].find(close) {
                let end = 1 + rel + 1;
                let raw = &rest[..end];
                match read_tag(raw) {
                    Some(token) => {
                        flush(&mut literal, &mut tokens);
                        tokens.push(token);
                    }
                    None if looks_like_tag(raw) => {
                        flush(&mut literal, &mut tokens);
                        tokens.push(MesToken::Unknown {
                            raw: raw.to_owned(),
                        });
                    }
                    None => {
                        literal.push_str(raw);
                    }
                }
                pos += end;
                continue;
            }
        }
        let ch = rest.chars().next().expect("non-empty");
        literal.push(ch);
        pos += ch.len_utf8();
    }
    flush(&mut literal, &mut tokens);
    tokens
}

fn flush(literal: &mut String, tokens: &mut Vec<MesToken>) {
    if !literal.is_empty() {
        tokens.push(MesToken::Text {
            text: std::mem::take(literal),
        });
    }
}

/// A failed span is reported as unknown only when it is short and has no whitespace or
/// newline inside: prose in brackets (rare) stays literal, as it does in the game.
fn looks_like_tag(raw: &str) -> bool {
    raw.len() <= MAX_TAG_LENGTH_HINT && !raw.contains('\n')
}

fn read_tag(raw: &str) -> Option<MesToken> {
    let inner = &raw[1..raw.len() - 1];
    if raw.starts_with('{') {
        return read_brace_tag(raw, inner);
    }
    let (name, params) = match inner.find('=') {
        Some(eq) => (&inner[..eq], Some(split_params(&inner[eq + 1..]))),
        None => (inner, None),
    };
    if name == "LOADMES"
        && let Some(params) = &params
    {
        return Some(MesToken::LoadMes {
            name: params.first().cloned().unwrap_or_default(),
        });
    }
    convert_original_tag(name, params).map(|(code, params, closing)| {
        MesToken::Tag(MesTag {
            code: code.to_owned(),
            params,
            closing,
            raw: raw.to_owned(),
        })
    })
}

fn split_params(text: &str) -> Vec<String> {
    text.split(',')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// `FFIXTextTag.ConvertOriginalTag`.
fn convert_original_tag(
    name: &str,
    params: Option<Vec<String>>,
) -> Option<(&'static str, Vec<String>, bool)> {
    if CHARACTER_NAME_TAGS.contains(&name) {
        return Some(("CharacterName", vec![name.to_owned()], false));
    }
    if let Some(index) = name.strip_prefix("PTY")
        && !index.is_empty()
        && index.parse::<i32>().is_ok()
    {
        return Some(("Party", vec![index.to_owned()], false));
    }
    if params.is_none() {
        let chars: Vec<char> = name.chars().collect();
        let all_hex = chars.iter().all(|c| is_hex(*c));
        match chars.len() {
            1 if chars[0] == '-' => return Some(("TextRGBA", Vec::new(), false)),
            2 if all_hex => return Some(("TextRGBA", vec![name.to_owned()], false)),
            6 | 8 if all_hex => {
                let comps = chars
                    .chunks(2)
                    .map(|pair| pair.iter().collect::<String>())
                    .collect();
                return Some(("TextRGBA", comps, false));
            }
            _ => {}
        }
    }
    let (name, closing) = match name.strip_prefix('/') {
        Some(stripped) => (stripped, true),
        None => (name, false),
    };
    let mut params = if closing {
        Vec::new()
    } else {
        params.unwrap_or_default()
    };
    if let Some((_, code)) = ORIGINAL_TAG_NAMES.iter().find(|(tag, _)| *tag == name) {
        if *code == "DialogF"
            && params.len() == 1
            && params[0]
                .parse::<i32>()
                .is_ok_and(|n| n > i32::from(i8::MAX))
        {
            params[0] = "0".to_owned();
        }
        return Some((code, params, closing));
    }
    if name == "YSUB" && !params.is_empty() && !closing {
        params[0] = format!("-{}", params[0]);
        return Some(("DialogY", params, false));
    }
    if name == "CENTER" {
        return Some(("Center", params, closing));
    }
    None
}

/// `{...}` Memoria-style tags: `{W..H..}`, `{y..}`, `{x..}`, `{f..}` or `{CodeName a,b}`.
fn read_brace_tag(raw: &str, inner: &str) -> Option<MesToken> {
    let make = |code: &str, params: Vec<String>| {
        Some(MesToken::Tag(MesTag {
            code: code.to_owned(),
            params,
            closing: false,
            raw: raw.to_owned(),
        }))
    };
    let bytes = inner.as_bytes();
    if bytes.len() >= 2 {
        let digit_or_minus = |b: u8| b == b'-' || b.is_ascii_digit();
        if bytes[0] == b'W' && bytes[1].is_ascii_digit() {
            let parts: Vec<&str> = inner[1..].split('H').collect();
            if parts.len() == 2 {
                return make(
                    "DialogSize",
                    parts.iter().map(|p| (*p).to_owned()).collect(),
                );
            }
        }
        for (lead, code) in [(b'y', "DialogY"), (b'x', "DialogX"), (b'f', "DialogF")] {
            if bytes[0] == lead && digit_or_minus(bytes[1]) {
                return make(code, vec![inner[1..].to_owned()]);
            }
        }
    }
    let (name, params) = match inner.find(' ') {
        Some(space) => (&inner[..space], split_params(&inner[space + 1..])),
        None => (inner, Vec::new()),
    };
    let name = match name.strip_suffix('+') {
        Some(base) => format!("{base}Ex"),
        None => name.to_owned(),
    };
    TAG_CODE_NAMES
        .iter()
        .find(|code| **code == name)
        .and_then(|code| make(code, params))
}

/// Character keyword tags: `NGUIText.Zidane`..`Amarant`. The game registers extra
/// keywords at runtime from `CharacterParameters.csv`; those are not defined in the
/// source and would surface as unknown.
const CHARACTER_NAME_TAGS: &[&str] = &[
    "ZDNE", "VIVI", "DGGR", "STNR", "FRYA", "QUIN", "EIKO", "AMRT",
];

/// `FFIXTextTag.OriginalTagNames` (NGUIText constants resolved).
const ORIGINAL_TAG_NAMES: &[(&str, &str)] = &[
    ("STRT", "DialogSize"),
    ("CHOO", "Choice"),
    ("TIME", "Time"),
    ("FLIM", "Flash"),
    ("NANI", "NoAnimation"),
    ("IMME", "Instantly"),
    ("SPED", "Speed"),
    ("ANIM", "TagAnimation"),
    ("HSHD", "ShadowToggle"),
    ("NSHD", "ShadowOff"),
    ("BCOL", "BackgroundRGBA"),
    ("FONT", "ChangeFont"),
    ("DBTN", "DefaultButton"),
    ("CBTN", "CustomButton"),
    ("NFOC", "NoFocus"),
    ("INCS", "IncreaseSignalEx"),
    ("PNEW", "IconEx"),
    ("MOVE", "MoveCaret"),
    ("ENDN", "End"),
    ("TEXT", "Text"),
    ("ITEM", "Item"),
    ("NUMB", "Variable"),
    ("WAIT", "Wait"),
    ("FEED", "DialogF"),
    ("XTAB", "DialogX"),
    ("YADD", "DialogY"),
    ("FRAM", "TextFrame"),
    ("ICON", "Icon"),
    ("PCHC", "PreChoose"),
    ("PCHM", "PreChooseMask"),
    ("MPOS", "Position"),
    ("OFFT", "Offset"),
    ("TAIL", "TailPosition"),
    ("TBLE", "Table"),
    ("WDTH", "Widths"),
    ("CENT", "Center"),
    ("PSND", "Sound"),
    ("SIGL", "Signal"),
    ("PAGE", "NewPage"),
    ("MOBI", "Mobile"),
    ("SPAY", "SpacingY"),
    ("KCBT", "KeyboardButton"),
    ("JCBT", "JoyStickButton"),
    ("NTUR", "TurboOff"),
    ("SPRT", "Sprite"),
    ("JSTF", "Justified"),
    ("MIRR", "Mirrored"),
    ("sup", "Superscript"),
    ("sub", "Subscript"),
    ("url", "Hyperlink"),
    ("b", "Bold"),
    ("i", "Italic"),
    ("u", "Underline"),
    ("s", "Strikethrough"),
    ("c", "IgnoreColor"),
];

/// Every `FFIXTextTagCode` member name (for the `{Name args}` form).
const TAG_CODE_NAMES: &[&str] = &[
    "End",
    "Instantly",
    "Flash",
    "NoFocus",
    "NoAnimation",
    "NewPage",
    "Choice",
    "PreChoose",
    "PreChooseMask",
    "Tab",
    "MoveCaret",
    "Zidane",
    "Vivi",
    "Dagger",
    "Steiner",
    "Freya",
    "Fraya",
    "Quina",
    "Eiko",
    "Amarant",
    "CharacterName",
    "Party",
    "Text",
    "Variable",
    "Item",
    "TextRGBA",
    "ShadowToggle",
    "ShadowOff",
    "White",
    "Pink",
    "Cyan",
    "Brown",
    "Yellow",
    "Green",
    "Grey",
    "BackgroundRGBA",
    "ChangeFont",
    "IncreaseSignal",
    "IncreaseSignalEx",
    "TailPosition",
    "LowerRight",
    "LowerLeft",
    "UpperRight",
    "UpperLeft",
    "LowerCenter",
    "UpperCenter",
    "LowerRightForce",
    "LowerLeftForce",
    "UpperRightForce",
    "UpperLeftForce",
    "DialogPosition",
    "DialogSize",
    "DialogY",
    "DialogX",
    "DialogF",
    "Position",
    "SpacingY",
    "TextFrame",
    "Sprite",
    "DefaultButton",
    "CustomButton",
    "KeyboardButton",
    "JoyStickButton",
    "Up",
    "Down",
    "Left",
    "Right",
    "Circle",
    "Cross",
    "Triangle",
    "Square",
    "R1",
    "R2",
    "L1",
    "L2",
    "Select",
    "Start",
    "Pad",
    "UpEx",
    "DownEx",
    "LeftEx",
    "RightEx",
    "CircleEx",
    "CrossEx",
    "TriangleEx",
    "SquareEx",
    "R1Ex",
    "R2Ex",
    "L1Ex",
    "L2Ex",
    "SelectEx",
    "StartEx",
    "PadEx",
    "Sound",
    "Signal",
    "Time",
    "Wait",
    "TurboOff",
    "Center",
    "Justified",
    "Mirrored",
    "Superscript",
    "Subscript",
    "Hyperlink",
    "Bold",
    "Italic",
    "Underline",
    "Strikethrough",
    "IgnoreColor",
    "Icon",
    "IconEx",
    "Mobile",
    "Speed",
    "TagAnimation",
    "Widths",
    "Offset",
    "Table",
    "ResetTags",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(code: &str, params: &[&str], raw: &str) -> MesToken {
        MesToken::Tag(MesTag {
            code: code.to_owned(),
            params: params.iter().map(|p| (*p).to_owned()).collect(),
            closing: false,
            raw: raw.to_owned(),
        })
    }

    fn text(value: &str) -> MesToken {
        MesToken::Text {
            text: value.to_owned(),
        }
    }

    #[test]
    fn endn_split_drops_trailing_empty_and_numbers_entries() {
        let file = parse_mes("Hello[ENDN]World[ENDN]", MesLayout::EndnSplit);
        assert_eq!(file.entries.len(), 2);
        assert_eq!(file.entries[0].id, 0);
        assert_eq!(file.entries[1].id, 1);
        assert_eq!(file.entries[1].tokens, vec![text("World")]);
    }

    #[test]
    fn endn_split_keeps_unterminated_tail() {
        let file = parse_mes("A[ENDN]B", MesLayout::EndnSplit);
        assert_eq!(file.entries.len(), 2);
        assert_eq!(file.entries[1].tokens, vec![text("B")]);
    }

    #[test]
    fn txid_sets_running_id() {
        let file = parse_mes(
            "[TXID=10]A[ENDN]B[ENDN][TXID=3]C[ENDN]",
            MesLayout::EndnSplit,
        );
        let ids: Vec<i32> = file.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![10, 11, 3]);
        assert_eq!(file.entries[0].tokens, vec![text("A")]);
    }

    #[test]
    fn strt_split_numbers_entries_and_keeps_end_tag() {
        let src = "[STRT=40,2]Hi[PAGE]There[ENDN][STRT=10,1][ZDNE]![ENDN]";
        let file = parse_mes(src, MesLayout::StrtSplit);
        assert_eq!(file.entries.len(), 2);
        assert_eq!(file.entries[1].id, 1);
        assert_eq!(
            file.entries[0].tokens,
            vec![
                tag("DialogSize", &["40", "2"], "[STRT=40,2]"),
                text("Hi"),
                tag("NewPage", &[], "[PAGE]"),
                text("There"),
                tag("End", &[], "[ENDN]"),
            ]
        );
        assert_eq!(
            file.entries[1].tokens[1],
            tag("CharacterName", &["ZDNE"], "[ZDNE]")
        );
    }

    #[test]
    fn strt_split_honors_txid_blocks() {
        let src = "[TXID=5][STRT=1,1]a[ENDN][STRT=1,1]b[ENDN][TXID=9][STRT=1,1]c[ENDN]";
        let file = parse_mes(src, MesLayout::StrtSplit);
        let ids: Vec<i32> = file.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![5, 6, 9]);
    }

    #[test]
    fn color_party_and_closing_tags() {
        let tokens = tokenize("[FFCC00FF]x[-][PTY2][/c][C8C8C8][80]");
        assert_eq!(
            tokens[0],
            tag("TextRGBA", &["FF", "CC", "00", "FF"], "[FFCC00FF]")
        );
        assert_eq!(tokens[2], tag("TextRGBA", &[], "[-]"));
        assert_eq!(tokens[3], tag("Party", &["2"], "[PTY2]"));
        match &tokens[4] {
            MesToken::Tag(t) => {
                assert_eq!(t.code, "IgnoreColor");
                assert!(t.closing);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(tokens[5], tag("TextRGBA", &["C8", "C8", "C8"], "[C8C8C8]"));
        assert_eq!(tokens[6], tag("TextRGBA", &["80"], "[80]"));
    }

    #[test]
    fn ysub_negates_and_feed_clamps() {
        let tokens = tokenize("[YSUB=4][FEED=255][FEED=12][YADD=3]");
        assert_eq!(tokens[0], tag("DialogY", &["-4"], "[YSUB=4]"));
        assert_eq!(tokens[1], tag("DialogF", &["0"], "[FEED=255]"));
        assert_eq!(tokens[2], tag("DialogF", &["12"], "[FEED=12]"));
        assert_eq!(tokens[3], tag("DialogY", &["3"], "[YADD=3]"));
    }

    #[test]
    fn unknown_tag_is_kept_and_reported() {
        let file = parse_mes("a[WXYZ=1]b[ENDN]", MesLayout::EndnSplit);
        assert_eq!(
            file.entries[0].tokens[1],
            MesToken::Unknown {
                raw: "[WXYZ=1]".to_owned()
            }
        );
        assert_eq!(file.unknown_tags(), vec![("[WXYZ=1]".to_owned(), 1)]);
    }

    #[test]
    fn unmatched_bracket_is_literal() {
        let tokens = tokenize("a [ b");
        assert_eq!(tokens, vec![text("a [ b")]);
    }

    #[test]
    fn brace_tags_decode() {
        let tokens = tokenize("{W40H3}{y-2}{Zidane}{Sound 1,2}");
        assert_eq!(tokens[0], tag("DialogSize", &["40", "3"], "{W40H3}"));
        assert_eq!(tokens[1], tag("DialogY", &["-2"], "{y-2}"));
        assert_eq!(tokens[2], tag("Zidane", &[], "{Zidane}"));
        assert_eq!(tokens[3], tag("Sound", &["1", "2"], "{Sound 1,2}"));
    }

    #[test]
    fn loadmes_and_utf8_text() {
        let tokens = tokenize("[LOADMES=shared]é\n日本");
        assert_eq!(
            tokens[0],
            MesToken::LoadMes {
                name: "shared".to_owned()
            }
        );
        assert_eq!(tokens[1], text("é\n日本"));
    }

    #[test]
    fn source_round_trips() {
        let src = "[STRT=40,2][ZDNE]: Hi[PAGE]bye[ENDN]";
        let file = parse_mes(src, MesLayout::StrtSplit);
        assert_eq!(MesFile::entry_source(&file.entries[0]), src);
    }

    /// Parses every `.mes` TextAsset in the real `resources.assets`. Skips when the
    /// install is absent. Unknown tags are limited to the untranslated multibyte
    /// escapes `[B0x..,0x..:n]`, which Memoria does not define.
    #[test]
    fn real_install_mes_files_parse_with_only_known_unknowns() {
        let path = std::path::Path::new(
            "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data/resources.assets",
        );
        if !path.is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let mut file =
            crate::pc::serialized_inventory::StandaloneSerialized::open(path).expect("open");
        let mut parsed = 0;
        for (name, object) in file.text_assets().expect("names") {
            if !name.ends_with(".mes") {
                continue;
            }
            let asset = file.read_text_asset(&object).expect("text asset");
            let mes = parse_mes_bytes(&asset.script, &name).expect("mes parses");
            for (raw, _) in mes.unknown_tags() {
                assert!(
                    raw.starts_with("[B0x"),
                    "unexpected unknown tag {raw} in {name}"
                );
            }
            parsed += 1;
        }
        assert!(parsed > 4000, "parsed only {parsed} .mes assets");
    }

    #[test]
    fn rejects_non_utf8() {
        assert!(parse_mes_bytes(&[0xff, 0xfe, 0x41], "t").is_err());
    }
}
