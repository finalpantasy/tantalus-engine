use tantalus_contracts::{ContentId, IdErrorKind, ItemInstanceId};

#[test]
fn valid_namespaced_identities_round_trip() {
    for text in [
        "ff9:item/potion",
        "my_mod:enemy/iron_raptor",
        "my-mod:stat/bleed_power",
        "mod2:slot/left-hand",
    ] {
        let id = ContentId::parse(text).unwrap();
        assert_eq!(id.as_str(), text);
        let json = serde_json::to_string(&id).unwrap();
        let restored: ContentId = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, id);
    }

    let id = ContentId::parse("my_mod:enemy/iron_raptor").unwrap();
    assert_eq!(id.namespace(), "my_mod");
    assert_eq!(id.path(), "enemy/iron_raptor");
}

#[test]
fn malformed_identities_report_their_specific_rule() {
    let cases = [
        ("", IdErrorKind::Empty),
        ("potion", IdErrorKind::MissingNamespace),
        ("MyMod:item/potion", IdErrorKind::InvalidNamespace),
        ("2mod:item/potion", IdErrorKind::InvalidNamespace),
        ("mod:Item/potion", IdErrorKind::InvalidPath),
        ("mod:item//potion", IdErrorKind::InvalidPath),
        ("mod:item/../potion", IdErrorKind::InvalidPath),
        ("mod:item/potion:copy", IdErrorKind::InvalidPath),
        ("mod:item/-potion", IdErrorKind::InvalidPath),
    ];

    for (text, expected) in cases {
        let error = ContentId::parse(text).unwrap_err();
        assert_eq!(error.kind, expected, "input: {text}");
        assert_eq!(error.input, text);
        assert!(error.to_string().contains(text));
    }
}

#[test]
fn oversized_identities_are_rejected() {
    let long_path = format!("mod:item/{}", "x".repeat(300));
    assert_eq!(
        ContentId::parse(&long_path).unwrap_err().kind,
        IdErrorKind::InvalidPath
    );

    let excessive_total = format!("mod:item/{}", "x".repeat(400));
    assert_eq!(
        ContentId::parse(&excessive_total).unwrap_err().kind,
        IdErrorKind::TooLong
    );
}

#[test]
fn item_instance_ids_are_nonzero_even_when_deserialized() {
    assert!(ItemInstanceId::from_u64(0).is_none());
    assert_eq!(ItemInstanceId::from_u64(7).unwrap().get(), 7);
    assert!(serde_json::from_str::<ItemInstanceId>("0").is_err());
    assert_eq!(
        serde_json::from_str::<ItemInstanceId>("7").unwrap().get(),
        7
    );
}
