use atlas::{ExportRole, analyze};

#[test]
fn frontmatter_exports_record_role_local_declaration_and_type_only() {
    let source = r#"---
import { a } from "./a";
export interface Props { title: string }
export const prerender = true;
export async function getStaticPaths() { return []; }
const local = 1;
export { local as renamed, a };
export { b } from "./b";
export type { Shape } from "./shape";
---
<p>{local}</p>
"#;
    let facts = analyze(source).unwrap();
    let summary: Vec<_> = facts
        .exports
        .iter()
        .map(|e| {
            let local = e.local.map(|id| facts.declarations[id.0].name.as_str());
            (
                e.name.as_str(),
                local,
                e.source.as_deref(),
                e.type_only,
                e.role,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Props", Some("Props"), None, true, ExportRole::Props),
            (
                "prerender",
                Some("prerender"),
                None,
                false,
                ExportRole::Astro
            ),
            (
                "getStaticPaths",
                Some("getStaticPaths"),
                None,
                false,
                ExportRole::Astro
            ),
            ("renamed", Some("local"), None, false, ExportRole::Other),
            ("a", Some("a"), None, false, ExportRole::Other),
            ("b", None, Some("./b"), false, ExportRole::Other),
            ("Shape", None, Some("./shape"), true, ExportRole::Other),
        ]
    );
}

#[test]
fn client_script_exports_are_not_component_exports() {
    let facts = analyze("<script>export const x = 1;</script>").unwrap();
    assert!(facts.exports.is_empty());
}
