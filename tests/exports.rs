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

#[test]
fn type_only_follows_symbol_meaning_and_nested_exports_are_ignored() {
    let source = r#"---
export enum Status { Ready }
export abstract class Base {}
export type Alias = string;
interface Shape {}
export { Shape };
namespace Internal { export const hidden = 1; }
export * as ui from "./ui";
export type * as types from "./types";
---"#;
    let facts = analyze(source).unwrap();
    let summary: Vec<_> = facts
        .exports
        .iter()
        .map(|e| (e.name.as_str(), e.type_only))
        .collect();
    assert_eq!(
        summary,
        [
            ("Status", false),
            ("Base", false),
            ("Alias", true),
            ("Shape", true),
            ("ui", false),
            ("types", true),
        ]
    );
}

#[test]
fn roles_require_the_matching_type_or_value_meaning() {
    let source = "---\nexport const Props = 1;\nexport type prerender = boolean;\n---";
    let facts = analyze(source).unwrap();
    assert!(facts.exports.iter().all(|e| e.role == ExportRole::Other));
}
