use atlas::{
    DeclarationKind, DiagnosticPhase, ExportRole, ImportSpecifier, ReferenceKind, Resolution,
    ScopeDomain, analyze, analyze_file,
};

#[test]
fn path_selects_esm_grammar_without_reading_a_file() {
    for path in ["/virtual/a.js", "a.mjs", "a.jsx", "a.ts", "a.mts", "a.tsx"] {
        let facts = analyze_file(path, "export const value = 1; value;").unwrap();
        assert_eq!(facts.exports[0].name, "value", "{path}");
        assert!(facts.scopes.iter().all(|s| s.domain == ScopeDomain::Module));
        assert_eq!(facts.references.len(), 1);
        assert!(facts.elements.is_empty());
        assert!(facts.components.is_empty());
        assert!(facts.scripts.is_empty());
        assert!(facts.styles.is_empty());
        assert!(facts.boundaries.is_empty());
    }
    for path in ["a.ts", "a.mts", "a.tsx", "a.d.ts", "a.d.mts"] {
        let facts =
            analyze_file(path, "export interface Shape {} export type Alias = Shape;").unwrap();
        assert!(
            facts
                .exports
                .iter()
                .all(|e| e.type_only && e.role == ExportRole::Other)
        );
        assert_eq!(facts.references[0].kind, ReferenceKind::Type);
        assert!(
            matches!(&facts.references[0].resolution, Resolution::Resolved(b) if b.name == "Shape")
        );
    }
    for path in ["a.js", "a.mjs", "a.jsx"] {
        assert!(
            analyze_file(path, "const value: number = 1;").is_err(),
            "{path}"
        );
    }
    assert!(analyze_file("a.ts", "const view = <Button />;").is_err());
    assert!(analyze_file("a.d.ts", "export const x: number;").is_ok());
    assert!(analyze_file("a.d.mts", "export const x: number;").is_ok());
    assert!(analyze_file("a.ts", "export const x: number;").is_err());
}

#[test]
fn commonjs_is_explicitly_unsupported_and_virtual_keys_remain_astro() {
    for path in ["a.cjs", "a.cts", "a.d.cts"] {
        // The diagnostic is independent of source validity; never parse as Astro.
        for source in ["exports.value = 1;", "<p />", "const =;"] {
            let error = analyze_file(path, source).unwrap_err();
            assert!(!error.parser_panicked);
            assert_eq!(error.diagnostics.len(), 1);
            assert_eq!(
                error.diagnostics[0].code.as_deref(),
                Some("atlas/unsupported-source-type")
            );
        }
    }
    let source = "---\nconst x = 1;\n---\n<p>{x}</p>";
    for path in ["virtual", "virtual.unknown", "Page.astro"] {
        assert_eq!(
            analyze_file(path, source).unwrap(),
            analyze(source).unwrap()
        );
    }
}

#[test]
fn jsx_references_keep_lexical_bindings_and_original_unicode_crlf_spans() {
    let source = "import { Button as 部品 } from './ui.ts';\r\nconst 名前 = '😀';\r\nfunction view(名前) { return <部品 title={名前} />; }\r\nconst outside = <部品>{名前}</部品>;";
    for path in ["View.jsx", "View.tsx"] {
        let facts = analyze_file(path, source).unwrap();
        let names = facts.declarations_named("名前");
        assert_eq!(names.len(), 2);
        assert_ne!(names[0].id, names[1].id);
        assert_eq!(names[1].kind, DeclarationKind::Parameter);
        let inner = facts
            .reference_at(source.find("{名前}").unwrap() as u32 + 1)
            .unwrap();
        assert!(matches!(&inner.resolution, Resolution::Resolved(b) if b.id == names[1].id));
        let outer = facts
            .reference_at(source.rfind("{名前}").unwrap() as u32 + 1)
            .unwrap();
        assert!(matches!(&outer.resolution, Resolution::Resolved(b) if b.id == names[0].id));
        let component = facts.declarations_named("部品")[0];
        assert_eq!(
            component.import.as_ref().unwrap().specifier,
            ImportSpecifier::Named("Button".into())
        );
        assert!(facts.references_to(component.id).len() >= 2);
        for binding in &facts.declarations {
            assert_eq!(
                &source[binding.span.start as usize..binding.span.end as usize],
                binding.name
            );
        }
        for reference in &facts.references {
            assert_eq!(
                &source[reference.span.start as usize..reference.span.end as usize],
                reference.name
            );
            for byte in reference.span.clone() {
                assert_eq!(facts.reference_at(byte), Some(reference));
            }
        }
        assert!(facts.reference_at(source.len() as u32).is_none());
    }
}

#[test]
fn defaults_keep_named_declarations_or_whole_anonymous_exports() {
    for (source, name) in [
        ("const local = 1; export default local;", "local"),
        ("const local = 1; export default (local);", "local"),
        ("export default function named() {}", "named"),
        ("export default class Named {}", "Named"),
        ("export default interface Shape {}", "Shape"),
        ("const local = 1; export { local as default };", "local"),
    ] {
        let facts = analyze_file("a.ts", source).unwrap();
        let export = &facts.exports[0];
        assert_eq!(export.name, "default");
        assert_eq!(export.local, Some(facts.declarations_named(name)[0].id));
        assert_eq!(export.type_only, name == "Shape");
    }
    for source in [
        "export default function () {}",
        "export default class {}",
        "export default () => 1;",
        "export default { value: 1 };",
        "export default 42;",
    ] {
        let facts = analyze_file("a.js", source).unwrap();
        assert_eq!(facts.exports.len(), 1);
        let export = &facts.exports[0];
        assert_eq!(export.name, "default");
        assert_eq!(export.local, None);
        assert_eq!(export.span, 0..source.len() as u32);
    }
}

#[test]
fn external_modules_do_not_receive_astro_builtins_roles_or_template_validation() {
    let source = "export interface Props {} export const prerender = true; export function getStaticPaths() {} export const partial = false; Astro.url; Fragment; const x = <Unknown client:visible />;";
    let facts = analyze_file("a.tsx", source).unwrap();
    assert!(facts.exports.iter().all(|e| e.role == ExportRole::Other));
    assert!(facts.builtin_uses.is_empty());
    assert!(facts.diagnostics.is_empty());
    assert!(
        facts
            .references
            .iter()
            .filter(|r| ["Astro", "Fragment", "Unknown"].contains(&r.name.as_str()))
            .all(|r| r.resolution == Resolution::Unresolved)
    );
    assert!(facts.references.iter().any(|r| r.name == "Unknown"));
    let facts = analyze_file(
        "a.js",
        "const Astro = {}; Astro; const Fragment = 1; Fragment;",
    )
    .unwrap();
    assert!(
        facts
            .references
            .iter()
            .all(|r| matches!(r.resolution, Resolution::Resolved(_)))
    );
}

#[test]
fn external_parser_and_semantic_failures_have_original_spans_and_shared_display() {
    for (source, phase) in [
        ("// 日本語😀\r\nexport const =;", DiagnosticPhase::Parser),
        (
            "// 日本語😀\r\nlet value; let value;",
            DiagnosticPhase::Semantic,
        ),
    ] {
        let error = analyze_file("bad.ts", source).unwrap_err();
        assert!(error.to_string().starts_with("Source analysis failed:"));
        assert!(error.diagnostics.iter().all(|d| d.phase == phase));
        assert!(!error.diagnostics[0].labels.is_empty());
        for label in error.diagnostics.iter().flat_map(|d| &d.labels) {
            assert!(
                source
                    .get(label.span.start as usize..label.span.end as usize)
                    .is_some()
            );
            assert!(label.span.start as usize >= source.find('\n').unwrap());
        }
    }
}
