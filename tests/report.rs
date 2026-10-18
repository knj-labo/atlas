#![cfg(feature = "serde")]

#[test]
fn report_lists_exports_imports_component_uses_and_unused_exports_by_key() {
    let card =
        "---\nexport interface Props { title: string }\nexport const variant = 'a';\n---\n<slot />";
    let page = "---\nimport Card from './Card.astro';\n---\n<Card title=\"x\" {...rest} client:visible slot=\"aside\" />";
    let report =
        atlas::analyze_project([("/v/Card.astro", card), ("/v/Page.astro", page)]).report();
    insta::assert_snapshot!(serde_json::to_string_pretty(&report).unwrap());
}

#[test]
fn report_follows_reexports_and_reports_unused_barrel_exports() {
    let card = "---\nexport const variant = 'a';\n---\n<slot />";
    let barrel =
        "---\nexport { default as Card, variant as unusedVariant } from './Card.astro';\n---";
    let page = "---\nimport { Card } from './index.astro';\n---\n<Card title=\"x\" />";
    let report = atlas::analyze_project([
        ("/v/Card.astro", card),
        ("/v/index.astro", barrel),
        ("/v/Page.astro", page),
    ])
    .report();

    let card_report = report
        .files
        .iter()
        .find(|f| f.path == "/v/Card.astro")
        .unwrap();
    assert_eq!(card_report.component_uses.len(), 1);
    let usage = &card_report.component_uses[0];
    assert_eq!(usage.name, "Card");
    assert_eq!(usage.props, ["title"]);
    assert_eq!(
        usage.key,
        format!(
            "/v/Page.astro#{}-{}",
            page.find("<Card").unwrap(),
            page.len()
        )
    );

    let barrel_report = report
        .files
        .iter()
        .find(|f| f.path == "/v/index.astro")
        .unwrap();
    let card_export = barrel_report
        .exports
        .iter()
        .find(|e| e.name == "Card")
        .unwrap();
    assert_eq!(card_export.source.as_deref(), Some("./Card.astro"));
    assert_eq!(card_export.imported, Some(atlas::ImportSpecifier::Default));
    let unused = barrel_report
        .exports
        .iter()
        .find(|e| e.name == "unusedVariant")
        .unwrap();
    assert_eq!(
        unused.imported,
        Some(atlas::ImportSpecifier::Named("variant".into()))
    );
    assert_eq!(report.unused_exports, std::slice::from_ref(&unused.key));
}
