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
