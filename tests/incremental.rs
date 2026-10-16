#![cfg(feature = "incremental")]

use atlas::{AnalysisDatabase, analyze};
use std::{path::PathBuf, sync::Arc};

#[path = "support/registry.rs"]
mod registry;

#[test]
fn all_registered_fixtures_match_standalone_analysis() {
    let mut db = AnalysisDatabase::new();
    let cases = registry::cases();
    assert!(!cases.is_empty());
    for case in &cases {
        db.set_source(
            registry::root().join(case["file"].as_str().unwrap()),
            registry::source(case),
        );
    }
    for case in &cases {
        let path = registry::root().join(case["file"].as_str().unwrap());
        let cached = db.analyze_file(&path).unwrap();
        assert_eq!(
            cached.as_ref(),
            &analyze(&registry::source(case)),
            "fixture {}",
            case["id"]
        );
        assert!(Arc::ptr_eq(&cached, &db.analyze_file(path).unwrap()));
    }
}

#[test]
fn results_outlive_source_updates_and_the_database() {
    let original = String::from("---\nimport Button from './Button.astro';\n---\n<Button />");
    let expected_original = analyze(&original);
    let updated = String::from("<Updated />");
    let expected_updated = analyze(&updated);
    let (original_result, updated_result) = {
        let mut db = AnalysisDatabase::default();
        db.set_source(PathBuf::from("Page.astro"), original);
        let first = db.analyze_file("Page.astro").unwrap();
        db.set_source("Page.astro", updated);
        let second = db.analyze_file("Page.astro").unwrap();
        assert_eq!(first.as_ref(), &expected_original);
        (first, second)
    };
    assert_eq!(original_result.as_ref(), &expected_original);
    assert_eq!(updated_result.as_ref(), &expected_updated);
}

#[test]
fn queries_match_cached_analysis_and_retained_results_after_updates() {
    let source = "---\r\nconst 名前 = '😀';\r\n---\r\n{名前}";
    let mut db = AnalysisDatabase::new();
    db.set_source("Page.astro", source);
    let first = db.analyze_file("Page.astro").unwrap();
    db.set_source("Page.astro", source);
    let cached = db.analyze_file("Page.astro").unwrap();
    assert!(Arc::ptr_eq(&first, &cached));

    let original = first.as_ref().as_ref().unwrap();
    let memo = cached.as_ref().as_ref().unwrap();
    let standalone = analyze(source).unwrap();
    let declaration = original.declarations_named("名前")[0];
    assert_eq!(
        memo.declarations_named("名前"),
        standalone.declarations_named("名前")
    );
    assert_eq!(
        memo.references_to(declaration.id),
        standalone.references_to(standalone.declarations_named("名前")[0].id)
    );
    let position = source.find("{名前}").unwrap() as u32 + 1;
    assert_eq!(
        memo.reference_at(position),
        standalone.reference_at(position)
    );
    assert!(std::ptr::eq(
        original.reference_at(position).unwrap(),
        memo.reference_at(position).unwrap()
    ));
    assert_eq!(original.references_to(declaration.id).len(), 1);

    let updated_source = "---\r\nconst unused = 0;\r\nconst 名前 = '変更';\r\n---\r\n{名前} {名前}";
    db.set_source("Page.astro", updated_source);
    let updated = db.analyze_file("Page.astro").unwrap();
    assert!(!Arc::ptr_eq(&first, &updated));
    drop(db);

    let current = updated.as_ref().as_ref().unwrap();
    let current_declaration = current.declarations_named("名前")[0];
    let expected = analyze(updated_source).unwrap();
    let expected_declaration = expected.declarations_named("名前")[0];
    assert_eq!(
        current.references_to(current_declaration.id),
        expected.references_to(expected_declaration.id)
    );
    assert_eq!(current.references_to(current_declaration.id).len(), 2);
    assert_eq!(
        current.reference_at(updated_source.find("{名前}").unwrap() as u32 + 1),
        current
            .references_to(current_declaration.id)
            .first()
            .copied()
    );
    // Earlier records and IDs still refer to the retained earlier result.
    assert_eq!(original.declarations_named("名前"), vec![declaration]);
    assert_eq!(original.references_to(declaration.id).len(), 1);
    assert_eq!(
        original.reference_at(position),
        original.references_to(declaration.id).first().copied()
    );
}

#[test]
fn databases_and_caller_supplied_path_keys_are_independent() {
    let mut first = AnalysisDatabase::new();
    let mut second = AnalysisDatabase::new();
    let path = PathBuf::from("virtual/not-on-disk/../Page.astro");
    assert!(first.analyze_file(&path).is_none());
    first.set_source(&path, "<First />");
    second.set_source(&path, "<Second />");
    assert_eq!(
        first.analyze_file(&path).unwrap().as_ref(),
        &analyze("<First />")
    );
    assert_eq!(
        second.analyze_file(&path).unwrap().as_ref(),
        &analyze("<Second />")
    );
    assert!(first.analyze_file("virtual/Page.astro").is_none());
    assert!(first.analyze_file("unregistered.astro").is_none());

    first.set_source("virtual/not-on-disk/./../Page.astro", "<Updated />");
    assert_eq!(
        first.analyze_file(&path).unwrap().as_ref(),
        &analyze("<Updated />")
    );
    assert_eq!(
        second.analyze_file(&path).unwrap().as_ref(),
        &analyze("<Second />")
    );
}
