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
