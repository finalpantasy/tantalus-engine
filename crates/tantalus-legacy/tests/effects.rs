use std::collections::BTreeSet;
use tantalus_contracts::ContentId;
use tantalus_legacy::effects::OriginalEffectPath;

#[test]
fn missing_effect_resources_fail_validation_instead_of_becoming_empty_success() {
    let effect = OriginalEffectPath {
        id: ContentId::parse("ff9:effect/fire").unwrap(),
        required_resources: vec![
            ContentId::parse("ff9:mesh/fire").unwrap(),
            ContentId::parse("ff9:camera/fire").unwrap(),
        ],
    };

    let report = effect.validate(&BTreeSet::new()).unwrap_err();
    assert_eq!(report.issues.len(), 2);
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.code == "missing-effect-resource")
    );

    let available = effect.required_resources.iter().cloned().collect();
    assert!(effect.validate(&available).is_ok());
}
