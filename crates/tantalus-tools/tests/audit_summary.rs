//! The committed audit reports must produce a complete W9 summary.

use std::path::Path;
use tantalus_tools::audit_summary::{failure_counts, summarize};

#[test]
fn committed_audit_reports_fill_every_summary_row() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../audit");
    let rows = summarize(&dir);
    let missing: Vec<_> = rows
        .iter()
        .filter(|row| row.counts.is_none())
        .map(|row| (row.class, row.source))
        .collect();
    assert!(missing.is_empty(), "missing rows: {missing:?}");
    assert_eq!(failure_counts(&rows).1, 0);
    // Known-good anchor from the report, so a silent field rename cannot zero the table.
    let field = rows
        .iter()
        .find(|row| row.class == "Field composition")
        .unwrap();
    assert_eq!(field.counts, Some((674, 0)));
}
