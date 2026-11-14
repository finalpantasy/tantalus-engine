use std::path::Path;
use tantalus_legacy::*;

fn fixture_tree() -> SimpleJsonNode {
    SimpleJsonNode::Class(vec![
        (
            "text-日本語".to_owned(),
            SimpleJsonNode::String("Mognet ✓".to_owned()),
        ),
        (
            "values".to_owned(),
            SimpleJsonNode::Array(vec![
                SimpleJsonNode::Int(-42),
                SimpleJsonNode::Double(1.25),
                SimpleJsonNode::Bool(true),
                SimpleJsonNode::Float(-2.5),
            ]),
        ),
    ])
}

#[test]
fn source_backed_simple_json_binary_round_trips_all_value_types() {
    let document =
        SimpleJsonDocument::decode(&SimpleJsonDocument::encode(&fixture_tree()).unwrap()).unwrap();
    assert_eq!(document.root, fixture_tree());
    assert_eq!(document.trailing_bytes, 0);

    let mut bytes = SimpleJsonDocument::encode(&fixture_tree()).unwrap();
    bytes.extend_from_slice(b"trailing");
    let document = SimpleJsonDocument::decode(&bytes).unwrap();
    assert_eq!(document.trailing_bytes, 8);
}

#[test]
fn malformed_sidecars_report_their_byte_location() {
    let unsupported = 99_i32.to_le_bytes();
    let error = SimpleJsonDocument::decode(&unsupported).unwrap_err();
    assert_eq!(error.offset, 0);
    assert!(error.context.contains("99"));

    let truncated = [3_i32.to_le_bytes().as_slice(), &[5, b'a']].concat();
    let error = SimpleJsonDocument::decode(&truncated).unwrap_err();
    assert_eq!(error.offset, 4);
    assert!(error.context.contains("truncated UTF-8"));

    let negative = [1_i32.to_le_bytes().as_slice(), &(-1_i32).to_le_bytes()].concat();
    assert!(SimpleJsonDocument::decode(&negative).is_err());
}

#[test]
fn duplicate_class_keys_are_not_silently_overwritten() {
    let key = "same";
    let node = SimpleJsonNode::Class(vec![
        (key.to_owned(), SimpleJsonNode::Int(1)),
        (key.to_owned(), SimpleJsonNode::Int(2)),
    ]);
    assert!(SimpleJsonDocument::encode(&node).is_err());
}

#[test]
fn base_inspection_is_explicitly_not_decryption() {
    let report = LegacySaveImporter::inspect(
        &[0; 32],
        Some(&SimpleJsonDocument::encode(&fixture_tree()).unwrap()),
    )
    .unwrap();
    assert_eq!(report.base.bytes, 32);
    assert_eq!(report.base.sha256.len(), 64);
    assert!(!report.base.fixed_container_size_matches);
    assert!(!report.base.decoded);
    assert_eq!(report.pairing, PairingStatus::NotChecked);
    assert_eq!(report.extra.unwrap().root, fixture_tree());

    let fixed = vec![0; BASE_SAVE_CONTAINER_BYTES];
    assert!(
        LegacySaveImporter::inspect(&fixed, None)
            .unwrap()
            .base
            .fixed_container_size_matches
    );
}

#[test]
fn sidecar_pairing_uses_the_actual_slot_or_autosave_filename() {
    let base = Path::new("C:/ saves/SavedData_ww.dat");
    let manual = LegacySaveLocation {
        is_autosave: false,
        slot_id: 2,
        save_id: 14,
    };
    assert_eq!(
        LegacySaveImporter::validate_pairing(
            base,
            Path::new("C:/saves/SavedData_ww_Memoria_2_14.dat"),
            &manual
        ),
        PairingStatus::Matched
    );
    assert_eq!(
        LegacySaveImporter::validate_pairing(
            base,
            Path::new("C:/saves/SavedData_ww_Memoria_2_13.dat"),
            &manual
        ),
        PairingStatus::Mismatched
    );

    let autosave = LegacySaveLocation {
        is_autosave: true,
        slot_id: 0,
        save_id: 0,
    };
    assert_eq!(
        LegacySaveImporter::validate_pairing(
            base,
            Path::new("C:/saves/SavedData_ww_Memoria_Autosave.dat"),
            &autosave
        ),
        PairingStatus::Matched
    );
}
