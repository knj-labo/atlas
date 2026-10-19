use atlas::{
    DeclarationKind, DefinitionKind, DefinitionResolution, DiagnosticPhase, ExportRole,
    ImportSpecifier, ReferenceKind, Resolution, ScopeDomain, analyze, analyze_file,
    analyze_project,
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

#[test]
fn declaration_modules_implicitly_export_only_top_level_local_declarations() {
    let source = "import type { External as Imported } from './other.d.ts';\r\n\
interface 形<T> { value: T }\r\n\
type Alias = 形<Imported>;\r\n\
declare const value: Alias;\r\n\
declare function make(input: Alias): Alias;\r\n\
declare class Box { member: Alias; }\r\n\
declare enum Choice { First }\r\n\
declare namespace Nested { interface Hidden {} export const inside: number; }\r\n\
import PrivateAlias = Nested;\r\n\
declare global { interface GlobalOnly {} }\r\n\
declare module './augmentation' { interface Augmented {} }\r\n\
export interface Explicit {}";
    for path in ["types.d.ts", "types.d.mts"] {
        let facts = analyze_file(path, source).unwrap();
        let mut names: Vec<_> = facts
            .exports
            .iter()
            .map(|export| export.name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Alias", "Box", "Choice", "Explicit", "Nested", "make", "value", "形"
            ],
            "{path}"
        );
        for export in &facts.exports {
            let declaration = &facts.declarations[export.local.unwrap().0];
            assert_eq!(export.name, declaration.name, "{path}");
            assert_eq!(export.span, declaration.span, "{path}");
            assert_eq!(
                &source[export.span.start as usize..export.span.end as usize],
                export.name,
                "{path}"
            );
            assert_eq!(
                export.type_only,
                ["Alias", "Explicit", "形"].contains(&export.name.as_str()),
                "{path}: {}",
                export.name
            );
            assert_eq!(export.role, ExportRole::Other);
            assert!(export.source.is_none());
            assert!(export.imported.is_none());
        }
    }
}

#[test]
fn implicit_exports_require_a_declaration_module_without_an_explicit_export_surface() {
    for path in ["types.d.ts", "types.d.mts"] {
        for source in [
            "import './side.js'; interface Private {} export {};",
            "import './side.js'; interface Private {} export { Private as Public };",
            "import './side.js'; interface Private {} export type { Private as Public };",
            "import './side.js'; interface Private {} export * from './other.d.ts';",
            "import './side.js'; interface Private {} declare const value: number; export default value;",
        ] {
            let facts = analyze_file(path, source).unwrap();
            assert!(
                facts.exports.iter().all(|export| export.name != "Private"),
                "{path}: {source}"
            );
        }
        // A default declaration still has an export modifier, not an assignment.
        let facts =
            analyze_file(path, "interface Implicit {} export default class Named {}").unwrap();
        assert_eq!(
            facts
                .exports
                .iter()
                .map(|export| export.name.as_str())
                .collect::<Vec<_>>(),
            ["Implicit", "default"],
            "{path}"
        );
    }
    for path in ["types.ts", "types.mts"] {
        let facts = analyze_file(
            path,
            "import './side.js'; interface Private {} declare const value: number;",
        )
        .unwrap();
        assert!(facts.exports.is_empty(), "{path}");
    }
    let source = "interface Global {} declare namespace Nested { interface Inside {} }";
    assert!(
        analyze_file("types.d.ts", source)
            .unwrap()
            .exports
            .is_empty()
    );
    let facts = analyze_file("types.d.mts", source).unwrap();
    assert_eq!(
        facts
            .exports
            .iter()
            .map(|export| export.name.as_str())
            .collect::<Vec<_>>(),
        ["Global", "Nested"]
    );
    // Text mentioning imports and exports does not turn a global declaration
    // script into a module or disable a module's ambient export context.
    assert!(
        analyze_file(
            "types.d.ts",
            "// import './side.js';\ninterface Global { text: 'export {}' }"
        )
        .unwrap()
        .exports
        .is_empty()
    );
    let facts = analyze_file(
        "types.d.ts",
        "import './side.js'; // export {};\ninterface Implicit {}",
    )
    .unwrap();
    assert_eq!(facts.exports[0].name, "Implicit");
}

#[test]
fn implicit_type_exports_resolve_through_aliases_and_re_exports() {
    let source = "import type { Public as Local } from './barrel.ts'; let value: Local;";
    for path in ["types.d.ts", "types.d.mts"] {
        let barrel = format!("export type {{ Alias as Public }} from './{path}';");
        let project = analyze_project([
            (
                path,
                "import './side.js'; interface Shape {} type Alias = Shape;",
            ),
            ("side.js", "export {};"),
            ("barrel.ts", &barrel),
            ("use.ts", source),
        ]);
        let result = project.definition_at("use.ts", source.rfind("Local").unwrap() as u32);
        assert!(
            matches!(result, Some(DefinitionResolution::Resolved(definition))
                if definition.path == std::path::Path::new(path)
                    && matches!(definition.kind, DefinitionKind::Declaration(binding)
                        if binding.name == "Alias" && binding.kind == DeclarationKind::Type)),
            "{path}: {result:?}"
        );
    }
}

#[test]
fn implicit_and_explicit_declaration_exports_keep_source_order() {
    for path in ["types.d.ts", "types.d.mts"] {
        let facts = analyze_file(path, "export interface First {} interface Second {} export const third: number; type Fourth = number;").unwrap();
        assert_eq!(
            facts
                .exports
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second", "third", "Fourth"]
        );
        assert!(
            facts
                .exports
                .windows(2)
                .all(|exports| exports[0].span.start <= exports[1].span.start)
        );
    }
}
