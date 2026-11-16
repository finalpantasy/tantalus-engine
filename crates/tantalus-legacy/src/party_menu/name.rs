//! Name entry rules (`Global/NameSettingUI.cs`): who is renameable, the
//! length limit, input filtering, confirm and reset.
//!
//! The 12-character limit is `NameInputField.characterLimit = 12`
//! (`NameSettingUI.cs:217`); `ff9play.FF9PLAY_NAME_MAX` (10,
//! `Global/ff9play.cs:50`) is not what the screen enforces.

/// `NameInputField.characterLimit` (`NameSettingUI.cs:217`).
pub const NAME_LIMIT: usize = 12;

/// `NameSettingUI.IsDefaultName` (`NameSettingUI.cs:33-36`): character ids
/// below 12 are the stock cast, which have default names to reset to.
pub fn is_default_name(character_id: u32) -> bool {
    character_id < 12
}

/// `InitID` (`NameSettingUI.cs:147-157`): the menu id from the script is
/// the character id for the stock cast, and `id - 12` otherwise.
/// Returns `(is_default_name, character_id)`.
pub fn init_id(id: u32) -> (bool, u32) {
    if id < 12 {
        (true, id)
    } else {
        (false, id - 12)
    }
}

/// `GetBackgroundSpritePath` (`NameSettingUI.cs:174-183`): Freya (4) uses
/// `name06`, Quina (5) `name04`, Eiko (6) `name05`; everyone else
/// `name` plus the id in two digits.
pub fn background_sprite(character_id: u32) -> String {
    match character_id {
        4 => "name06".to_string(),
        5 => "name04".to_string(),
        6 => "name05".to_string(),
        n => format!("name{n:02}"),
    }
}

/// `ValidateInput` (`NameSettingUI.cs:191-205`): returns the accepted
/// character, or `None` for the `'\0'` the source returns.
///
/// Letters pass; a space passes only after the first character; anything
/// else must fall in the kana, CJK, ASCII `!` to `~`, Latin-1 letter or
/// full-width lowercase ranges of the source's regex.
///
/// Differences: C# `Char.IsLetter` is the Unicode letter categories;
/// Rust has no category lookup, so letters are `is_alphabetic` and not
/// `is_numeric`, which also admits combining vowel signs that C# rejects.
/// The Arabic bidi-class branch is not modeled (Arabic letters pass as
/// letters, Arabic digits do not).
pub fn validate_char(index: usize, c: char) -> Option<char> {
    if (c.is_alphabetic() && !c.is_numeric()) || (c == ' ' && index > 0) {
        return Some(c);
    }
    let u = u32::from(c);
    let allowed = (0x3041..=0x3096).contains(&u)
        || (0x30A0..=0x30FF).contains(&u)
        || (0x3400..=0x4DB5).contains(&u)
        || (0x4E00..=0x9FCB).contains(&u)
        || (0xF900..=0xFA6A).contains(&u)
        || (0x21..=0x7E).contains(&u)
        || (0xC0..=0xFF).contains(&u)
        || (0xFF41..=0xFF5A).contains(&u);
    allowed.then_some(c)
}

/// Why a name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// Blank after trimming: the warning shows and the beep plays
    /// (`NameSettingUI.cs:119-130`).
    Empty,
}

/// Typing into the field: filter each character by position and stop at the
/// limit, as the input widget does with `onValidate` and `characterLimit`.
pub fn filter_input(text: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in text.chars() {
        if n >= NAME_LIMIT {
            break;
        }
        if let Some(c) = validate_char(n, c) {
            out.push(c);
            n += 1;
        }
    }
    out
}

/// `OnConfirmButtonClick` (`NameSettingUI.cs:119-130`): trim, refuse an
/// empty result, else that is the new name (`SetCharacterName`, `:185-188`).
/// `str::trim` removes Unicode White_Space and C# `Trim` removes
/// `Char.IsWhiteSpace`; the two sets match for ordinary text.
pub fn confirm_name(input: &str) -> Result<String, NameError> {
    let t = input.trim();
    if t.is_empty() {
        Err(NameError::Empty)
    } else {
        Ok(t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `IsDefaultName` and `InitID`: ids 0..11 are stock; 12 and above map
    /// back by subtracting 12 and have no default name.
    #[test]
    fn ids() {
        assert!(is_default_name(0));
        assert!(is_default_name(11));
        assert!(!is_default_name(12));
        assert_eq!(init_id(3), (true, 3));
        assert_eq!(init_id(12), (false, 0));
        assert_eq!(init_id(17), (false, 5));
    }

    /// Background names: Freya 06, Quina 04, Eiko 05, others zero padded.
    #[test]
    fn backgrounds() {
        assert_eq!(background_sprite(0), "name00");
        assert_eq!(background_sprite(3), "name03");
        assert_eq!(background_sprite(4), "name06");
        assert_eq!(background_sprite(5), "name04");
        assert_eq!(background_sprite(6), "name05");
        assert_eq!(background_sprite(7), "name07");
    }

    /// Letters pass; a leading space does not, an inner space does;
    /// punctuation and digits in `!`..`~` pass; control characters, emoji
    /// and a bullet are refused by the regex branch.
    #[test]
    fn validation() {
        assert_eq!(validate_char(0, 'Z'), Some('Z'));
        assert_eq!(validate_char(0, ' '), None);
        assert_eq!(validate_char(3, ' '), Some(' '));
        assert_eq!(validate_char(0, '!'), Some('!'));
        assert_eq!(validate_char(0, '7'), Some('7'));
        assert_eq!(validate_char(0, 'é'), Some('é'));
        assert_eq!(validate_char(0, 'ア'), Some('ア'));
        assert_eq!(validate_char(0, '\u{7}'), None);
        assert_eq!(validate_char(0, '\u{1F600}'), None);
        // U+00D7 (multiplication sign) is in the Latin-1 range of the regex.
        assert_eq!(validate_char(0, '\u{D7}'), Some('\u{D7}'));
        // U+2022 bullet: not a letter, not in any allowed range.
        assert_eq!(validate_char(0, '\u{2022}'), None);
    }

    #[test]
    fn typing_and_limit() {
        assert_eq!(filter_input(" Dagger"), "Dagger");
        assert_eq!(filter_input("Abcdefghijklmnop"), "Abcdefghijkl");
        assert_eq!(filter_input("A B"), "A B");
        assert_eq!(filter_input("a\u{1F600}b"), "ab");
    }

    /// Confirm trims, then refuses blanks (`NameSettingUI.cs:119-130`).
    #[test]
    fn confirming() {
        assert_eq!(confirm_name("  Zidane "), Ok("Zidane".to_string()));
        assert_eq!(confirm_name("   "), Err(NameError::Empty));
        assert_eq!(confirm_name(""), Err(NameError::Empty));
    }
}
