use tantalus_legacy::{EventBinary, InstructionKind, VmErrorKind};

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 144];
    bytes[..2].copy_from_slice(b"EV");
    bytes[2] = 2;
    bytes[3] = 2;
    bytes[128..136].copy_from_slice(&[16, 0, 14, 0, 3, 1, 0, 0]);
    bytes[136..144].copy_from_slice(&[30, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(&[7, 2, 0, 0, 8, 0, 3, 0, 11, 0, 0, 0, 4, 4]);
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
    assert_eq!(
        binary.entry_bytes(0),
        Some(&[7, 2, 0, 0, 8, 0, 3, 0, 11, 0, 0, 0, 4, 4][..])
    );
    assert_eq!(binary.entry_bytes(1), Some(&[][..]));

    let entry = binary.decode_entry(0).unwrap();
    assert_eq!(entry.entry_type, 7);
    assert_eq!(entry.functions.len(), 2);
    assert_eq!(entry.functions[0].tag, 0);
    assert_eq!(entry.functions[0].offset, 10);
    assert_eq!(entry.functions[1].tag, 3);
    assert_eq!(entry.functions[1].offset, 13);
    assert!(matches!(
        entry.functions[0].program.instructions[&0].kind,
        InstructionKind::Nop
    ));
    assert!(matches!(
        entry.functions[0].program.instructions[&2].kind,
        InstructionKind::Return
    ));
    assert!(matches!(
        entry.functions[1].program.instructions[&0].kind,
        InstructionKind::Return
    ));

    let empty = binary.decode_entry(1).unwrap();
    assert_eq!(empty.entry_type, 0xff);
    assert!(empty.functions.is_empty());
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

#[test]
fn malformed_function_tables_and_ranges_are_rejected() {
    let mut missing_count = fixture();
    missing_count[130] = 1;
    let binary = EventBinary::parse(&missing_count).unwrap();
    let error = binary.decode_entry(0).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Decode);
    assert_eq!(error.offset, 1);

    let mut truncated_table = fixture();
    truncated_table[145] = 4;
    let binary = EventBinary::parse(&truncated_table).unwrap();
    let error = binary.decode_entry(0).unwrap_err();
    assert_eq!(error.offset, 2);
    assert!(error.message.contains("truncated function table"));

    let mut overlapping_table = fixture();
    overlapping_table[148..150].copy_from_slice(&0_u16.to_le_bytes());
    let binary = EventBinary::parse(&overlapping_table).unwrap();
    let error = binary.decode_entry(0).unwrap_err();
    assert_eq!(error.offset, 2);
    assert!(error.message.contains("invalid range"));

    let mut reversed = fixture();
    reversed[152..154].copy_from_slice(&7_u16.to_le_bytes());
    let binary = EventBinary::parse(&reversed).unwrap();
    let error = binary.decode_entry(0).unwrap_err();
    assert_eq!(error.offset, 2);
    assert!(error.message.contains("function 0 (tag 0)"));
    assert!(error.message.contains("invalid range"));
}

#[test]
fn function_decode_errors_include_the_tag_and_entry_relative_offset() {
    let mut unsupported = fixture();
    unsupported[154] = 0x0c;
    let binary = EventBinary::parse(&unsupported).unwrap();
    let error = binary.decode_entry(0).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::UnsupportedOpcode);
    assert_eq!(error.offset, 10);
    assert_eq!(error.opcode, Some(0x0c));
    assert!(error.message.contains("function 0 (tag 0)"));
}
