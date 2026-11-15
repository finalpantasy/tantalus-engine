use tantalus_legacy::{EventBinary, InstructionKind};

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 144];
    bytes[..2].copy_from_slice(b"EV");
    bytes[2] = 2;
    bytes[3] = 2;
    bytes[128..136].copy_from_slice(&[16, 0, 2, 0, 3, 1, 0, 0]);
    bytes[136..144].copy_from_slice(&[18, 0, 1, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 4, 0]);
    bytes
}

#[test]
fn vanilla_entries_use_the_header_as_their_offset_base() {
    let binary = EventBinary::parse(&fixture()).unwrap();
    assert_eq!(binary.version, 2);
    assert_eq!(binary.entries.len(), 2);
    assert_eq!(binary.entries[0].absolute_offset, 144);
    assert_eq!(binary.entries[0].variable_count, 3);
    assert_eq!(binary.entries[0].flags, 1);
    assert_eq!(binary.entry_bytes(0), Some(&[0, 4][..]));
    assert_eq!(binary.entry_bytes(1), Some(&[0][..]));

    let program = binary.decode_entry(0).unwrap();
    assert!(matches!(
        program.instructions[&0].kind,
        InstructionKind::Nop
    ));
    assert!(matches!(
        program.instructions[&1].kind,
        InstructionKind::Return
    ));
}

#[test]
fn malformed_headers_tables_and_entry_ranges_are_rejected() {
    assert!(EventBinary::parse(b"NO").is_err());
    assert!(EventBinary::parse(&[0; 127]).is_err());

    let mut truncated = fixture();
    truncated.truncate(130);
    assert!(EventBinary::parse(&truncated).is_err());

    let mut outside = fixture();
    outside[128..130].copy_from_slice(&1000_u16.to_le_bytes());
    assert!(EventBinary::parse(&outside).is_err());
}
