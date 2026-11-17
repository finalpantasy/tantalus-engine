//! `plugin_effect_flags` maps the table's RVA through the PE section table.

use tantalus_assets::pc::special_effect::plugin_effect_flags;

#[test]
fn reads_the_flag_words_through_the_section_table() {
    let pe = 0x40;
    let raw = 0x200;
    let mut dll = vec![0u8; raw + 0x1000];
    dll[0x3C..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
    dll[pe..pe + 4].copy_from_slice(b"PE\x00\x00");
    dll[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
    // No optional header: the section table follows the file header.
    let section = pe + 24;
    dll[section + 8..section + 12].copy_from_slice(&0x1000u32.to_le_bytes());
    dll[section + 12..section + 16].copy_from_slice(&0x4F000u32.to_le_bytes());
    dll[section + 16..section + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    dll[section + 20..section + 24].copy_from_slice(&(raw as u32).to_le_bytes());
    let table = raw + 0x20;
    dll[table + 2 * 257..table + 2 * 257 + 2].copy_from_slice(&0x30F8u16.to_le_bytes());
    let flags = plugin_effect_flags(&dll).expect("flags");
    assert_eq!(flags.len(), 1024);
    assert_eq!(flags[257], 0x30F8);
    assert_eq!(flags[257] & 8, 8);
    assert_eq!(flags[0], 0);
    assert!(plugin_effect_flags(&dll[..0x100]).is_none());
}
