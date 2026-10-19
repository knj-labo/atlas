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

#[test]
fn composite_report_roundtrips_through_default_toon_with_strict_decoding() {
    use atlas::{DefinitionReport, DefinitionTargetKind, DefinitionUnresolved};
    let project = atlas::analyze_project([
        ("日本語,\n#[]:|.cts", "export = 1;"),
        ("a.ts", "export const value = 1; export default value;"),
        ("b.ts", "export const value = 2;"),
        (
            "ambiguous.ts",
            "export * from './a.ts'; export * from './b.ts';",
        ),
        ("barrel.js", "export { default as main } from './a.ts';"),
        (
            "use.ts",
            "import { main } from './barrel.js'; import { value as ambiguous } from './ambiguous.ts'; import missing from './missing.ts'; import * as ns from './a.ts'; main; ambiguous; missing; ns.value;",
        ),
        (
            "Page.astro",
            "---\nimport Card from './Card.astro';\n---\n<Card title=\"日本語,|:\" client:visible />",
        ),
        ("Card.astro", "<slot />"),
        ("bad.ts", "export const =;"),
    ]);
    let report = project.report();
    assert_eq!(report.schema_version, 1);
    let use_file = report.files.iter().find(|f| f.path == "use.ts").unwrap();
    assert_eq!(use_file.references.len(), 4);
    assert!(
        matches!(&use_file.references[0].definition, DefinitionReport::Resolved(target) if target.key == "a.ts#13-18" && target.kind == DefinitionTargetKind::Declaration)
    );
    assert!(
        matches!(&use_file.references[1].definition, DefinitionReport::Ambiguous(candidates) if candidates.len() == 2)
    );
    assert_eq!(
        use_file.references[2].definition,
        DefinitionReport::Unresolved(DefinitionUnresolved::UnresolvedModule)
    );
    assert!(
        matches!(&use_file.references[3].definition, DefinitionReport::Resolved(target) if target.kind == DefinitionTargetKind::Module)
    );
    for reference in &use_file.references {
        assert!(use_file.imports.iter().any(|i| i.key == reference.import));
    }
    let failed = report
        .files
        .iter()
        .find(|f| f.path.starts_with("日本語"))
        .unwrap();
    assert!(!failed.analyzed);
    assert!(failed.diagnostics[0].message.contains("日本語,\n#[]:|"));
    assert!(failed.references.is_empty());
    let toon = toon_format::encode_default(&report).unwrap();
    let decoded: serde_json::Value = toon_format::decode_strict(&toon).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(decoded, json);
    assert!(toon.contains("null"));
    insta::assert_snapshot!(serde_json::to_string_pretty(&report).unwrap());
}
