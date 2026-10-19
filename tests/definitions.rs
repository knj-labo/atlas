use atlas::{
    Definition, DefinitionKind, DefinitionResolution, DefinitionUnresolved, ModuleRequest,
    ModuleResolution, ModuleResolver, ProjectSnapshot, ProjectSources, RelativeResolver,
    ScopeDomain, analyze_project, analyze_project_with_resolver,
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

fn definition<'a>(
    project: &'a ProjectSnapshot,
    path: &str,
    needle: &str,
) -> DefinitionResolution<'a> {
    let source = &project.file(path).unwrap().source;
    project
        .definition_at(path, source.rfind(needle).unwrap() as u32)
        .unwrap()
}

fn resolved<'a>(project: &'a ProjectSnapshot, path: &str, needle: &str) -> Definition<'a> {
    match definition(project, path, needle) {
        DefinitionResolution::Resolved(target) => target,
        other => panic!("expected a definition for {path}:{needle}, got {other:?}"),
    }
}

fn assert_declaration(project: &ProjectSnapshot, from: &str, needle: &str, path: &str, name: &str) {
    let target = resolved(project, from, needle);
    assert_eq!(target.path, Path::new(path));
    let DefinitionKind::Declaration(binding) = target.kind else {
        panic!("{target:?}")
    };
    assert_eq!(binding.name, name);
    let facts = project
        .file(path)
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    assert!(std::ptr::eq(binding, &facts.declarations[binding.id.0]));
    assert_eq!(target.span(), binding.span);
}

#[test]
fn mixed_astro_js_ts_import_aliases_defaults_types_and_local_shadowing() {
    let page = "---\r\nimport Card from './Card.astro';\r\nimport main, { value as 名前, type Shape } from './values.ts';\r\nconst typed: Shape = {};\r\nfunction shadow(名前) { return 名前; }\r\n---\r\n<p>😀{名前}{main}</p><Card />";
    let project = analyze_project([
        ("Page.astro", page),
        ("Card.astro", "<slot />"),
        (
            "values.ts",
            "export interface Shape {} export const value = 1; const main = 2; export default main;",
        ),
        ("use.js", "import { value } from './values.ts'; value;"),
    ]);
    assert_declaration(&project, "Page.astro", "名前", "values.ts", "value");
    assert_declaration(&project, "Page.astro", "main", "values.ts", "main");
    assert_declaration(&project, "Page.astro", "Shape", "values.ts", "Shape");
    assert_declaration(&project, "use.js", "value", "values.ts", "value");
    let shadow = project
        .definition_at("Page.astro", page.find("return 名前").unwrap() as u32 + 7)
        .unwrap();
    let DefinitionResolution::Resolved(shadow) = shadow else {
        panic!("{shadow:?}")
    };
    assert_eq!(shadow.path, Path::new("Page.astro"));
    let DefinitionKind::Declaration(binding) = shadow.kind else {
        panic!("{shadow:?}")
    };
    assert_eq!(binding.kind, atlas::DeclarationKind::Parameter);
    let card = resolved(&project, "Page.astro", "Card");
    assert_eq!(card.path, Path::new("Card.astro"));
    let DefinitionKind::AstroComponent(file) = card.kind else {
        panic!("{card:?}")
    };
    assert!(std::ptr::eq(file, project.file("Card.astro").unwrap()));
    assert_eq!(card.span(), 0..8);
    assert!(
        project
            .definition_at("Page.astro", page.find("<p>").unwrap() as u32)
            .is_none()
    );
    assert!(project.definition_at("unknown", 0).is_none());
    assert!(
        project
            .definition_at("Page.astro", page.len() as u32)
            .is_none()
    );
    // Every UTF-8 byte of the imported reference has the same borrowed target.
    for byte in
        page.rfind("名前").unwrap() as u32..page.rfind("名前").unwrap() as u32 + "名前".len() as u32
    {
        let Some(DefinitionResolution::Resolved(d)) = project.definition_at("Page.astro", byte)
        else {
            panic!("{byte}")
        };
        assert_eq!(d.path, Path::new("values.ts"));
    }
}

#[test]
fn namespace_roots_and_anonymous_defaults_borrow_stored_records_without_member_inference() {
    let project = analyze_project([
        (
            "use.ts",
            "import expression from './expression.js'; import * as ns from './expression.js'; expression; ns.value; ns['dynamic'];",
        ),
        (
            "expression.js",
            "export default () => 42; export const value = 1;",
        ),
    ]);
    let target = resolved(&project, "use.ts", "expression;");
    let DefinitionKind::Export(export) = target.kind else {
        panic!("{target:?}")
    };
    let facts = project
        .file("expression.js")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    assert!(std::ptr::eq(export, &facts.exports[0]));
    assert_eq!(target.span(), export.span);
    let target = resolved(&project, "use.ts", "ns.value");
    assert!(matches!(target.kind, DefinitionKind::Module(_)));
    assert_eq!(target.path, Path::new("expression.js"));
    let source = &project.file("use.ts").unwrap().source;
    for name in ["value;", "dynamic"] {
        assert!(
            project
                .definition_at("use.ts", source.rfind(name).unwrap() as u32)
                .is_none()
        );
    }
}

#[test]
fn reexports_follow_imported_aliases_named_default_and_star_chains() {
    let project = analyze_project([
        (
            "value.ts",
            "export const value = 1; export interface Shape {} export default function main() {}",
        ),
        (
            "imported.mjs",
            "import { value as local } from './value.ts'; import main from './value.ts'; export { local as renamed, main as default };",
        ),
        (
            "named.ts",
            "export { renamed as result, default as main } from './imported.mjs'; export type { Shape } from './value.ts'; export * as all from './value.ts';",
        ),
        ("star.js", "export * from './named.ts';"),
        (
            "use.ts",
            "import { result, main, type Shape, all } from './star.js'; result; main; let shape: Shape; all.value;",
        ),
    ]);
    assert_declaration(&project, "use.ts", "result", "value.ts", "value");
    assert_declaration(&project, "use.ts", "main", "value.ts", "main");
    assert_declaration(&project, "use.ts", "Shape", "value.ts", "Shape");
    let namespace = resolved(&project, "use.ts", "all.value");
    assert_eq!(namespace.path, Path::new("value.ts"));
    assert!(matches!(namespace.kind, DefinitionKind::Module(_)));
}

#[test]
fn explicit_exports_win_and_duplicate_paths_to_one_declaration_are_merged() {
    let project = analyze_project([
        ("a.ts", "export const value = 1; export default value;"),
        ("b.ts", "export const value = 2;"),
        ("left.ts", "export * from './a.ts';"),
        (
            "right.ts",
            "import { value } from './a.ts'; export { value };",
        ),
        (
            "diamond.ts",
            "export * from './left.ts'; export * from './right.ts';",
        ),
        (
            "priority.ts",
            "export * from './b.ts'; export { value } from './a.ts';",
        ),
        (
            "local.ts",
            "export * from './a.ts'; export const value = 3;",
        ),
        (
            "ambiguous.ts",
            "export * from './a.ts'; export * from './b.ts';",
        ),
        (
            "use.ts",
            "import { value as diamond } from './diamond.ts'; import { value as priority } from './priority.ts'; import { value as local } from './local.ts'; import { value as ambiguous } from './ambiguous.ts'; diamond; priority; local; ambiguous;",
        ),
    ]);
    assert_declaration(&project, "use.ts", "diamond", "a.ts", "value");
    assert_declaration(&project, "use.ts", "priority", "a.ts", "value");
    assert_declaration(&project, "use.ts", "local", "local.ts", "value");
    let DefinitionResolution::Ambiguous(targets) = definition(&project, "use.ts", "ambiguous")
    else {
        panic!("expected ambiguity")
    };
    assert_eq!(
        targets.iter().map(|d| d.path).collect::<Vec<_>>(),
        [Path::new("a.ts"), Path::new("b.ts")]
    );
    for target in targets {
        let DefinitionKind::Declaration(binding) = target.kind else {
            panic!("{target:?}")
        };
        assert_eq!(binding.name, "value");
    }
}

#[test]
fn missing_unsupported_failed_exports_and_cycles_have_distinct_reasons() {
    let use_source = "import missing from './missing.ts'; import bad from './bad.ts'; import common from './legacy.cjs'; import typedCommon from './legacy.cts'; import declarationCommon from './legacy.d.cts'; import absent from './star.ts'; import { value as cycle } from './cycle-a.ts'; import pkg from 'package'; missing; bad; common; typedCommon; declarationCommon; absent; cycle; pkg;";
    let project = analyze_project([
        ("use.ts", use_source),
        ("bad.ts", "const =;"),
        ("legacy.cjs", "exports.value = 1;"),
        ("legacy.cts", "export = 1;"),
        ("legacy.d.cts", "export = 1;"),
        ("default.ts", "export default 1;"),
        ("star.ts", "export * from './default.ts';"),
        ("cycle-a.ts", "export { value } from './cycle-b.ts';"),
        (
            "cycle-b.ts",
            "import { value } from './cycle-a.ts'; export { value };",
        ),
    ]);
    for (name, expected) in [
        ("missing", DefinitionUnresolved::UnresolvedModule),
        (
            "bad",
            DefinitionUnresolved::AnalysisFailed {
                path: "bad.ts".into(),
            },
        ),
        (
            "common",
            DefinitionUnresolved::UnsupportedSourceType {
                path: "legacy.cjs".into(),
            },
        ),
        (
            "typedCommon",
            DefinitionUnresolved::UnsupportedSourceType {
                path: "legacy.cts".into(),
            },
        ),
        (
            "declarationCommon",
            DefinitionUnresolved::UnsupportedSourceType {
                path: "legacy.d.cts".into(),
            },
        ),
        (
            "absent",
            DefinitionUnresolved::MissingExport {
                path: "star.ts".into(),
                name: "default".into(),
            },
        ),
        (
            "cycle",
            DefinitionUnresolved::Cycle {
                path: "cycle-a.ts".into(),
                name: "value".into(),
            },
        ),
        ("pkg", DefinitionUnresolved::UnsupportedModule),
    ] {
        let DefinitionResolution::Unresolved(reason) = definition(&project, "use.ts", name) else {
            panic!("{name}")
        };
        assert_eq!(reason, expected, "{name}");
    }
    assert!(project.definition_at("bad.ts", 0).is_none());
}

#[test]
fn star_cycles_can_reach_explicit_definitions_but_failed_branches_cannot_be_guessed() {
    let project = analyze_project([
        (
            "a.ts",
            "export * from './b.ts'; export * from './value.ts';",
        ),
        ("b.ts", "export * from './a.ts';"),
        ("value.ts", "export const value = 1;"),
        (
            "broken-star.ts",
            "export * from './value.ts'; export * from './bad.ts';",
        ),
        ("bad.ts", "const =;"),
        (
            "use.ts",
            "import { value as cyclic, absent } from './a.ts'; import { value as uncertain } from './broken-star.ts'; cyclic; absent; uncertain;",
        ),
    ]);
    assert_declaration(&project, "use.ts", "cyclic", "value.ts", "value");
    assert!(matches!(
        definition(&project, "use.ts", "absent"),
        DefinitionResolution::Unresolved(DefinitionUnresolved::Cycle { .. })
    ));
    assert!(matches!(
        definition(&project, "use.ts", "uncertain"),
        DefinitionResolution::Unresolved(DefinitionUnresolved::AnalysisFailed { .. })
    ));
}

#[test]
fn queries_reuse_resolutions_and_reexport_resolver_receives_module_domain() {
    struct Counted(AtomicUsize);
    impl ModuleResolver for Counted {
        fn resolve(
            &self,
            request: ModuleRequest<'_>,
            sources: &ProjectSources<'_>,
        ) -> ModuleResolution {
            self.0.fetch_add(1, Ordering::Relaxed);
            assert_eq!(
                request.domain,
                if request.importer == Path::new("Page.astro") {
                    ScopeDomain::Server
                } else {
                    ScopeDomain::Module
                }
            );
            RelativeResolver.resolve(request, sources)
        }
    }
    let resolver = Counted(AtomicUsize::new(0));
    let project = analyze_project_with_resolver(
        [
            ("value.ts", "export const value = 1;"),
            ("barrel.js", "export { value } from './value.ts';"),
            ("use.ts", "import { value } from './barrel.js'; value;"),
            (
                "Page.astro",
                "---\nexport { value } from './barrel.js';\n---",
            ),
        ],
        &resolver,
    );
    assert_eq!(resolver.0.load(Ordering::Relaxed), 3);
    for _ in 0..3 {
        assert_declaration(&project, "use.ts", "value", "value.ts", "value");
        project.unused_exports();
        #[cfg(feature = "serde")]
        project.report();
    }
    assert_eq!(resolver.0.load(Ordering::Relaxed), 3);
}

#[test]
fn default_imports_use_defaults_and_stars_do_not_mark_defaults_used() {
    let project = analyze_project([
        ("default.ts", "export default 1; export const unused = 2;"),
        ("starred.ts", "export default 1; export const named = 2;"),
        ("namespace.ts", "export default 1; export const named = 2;"),
        ("barrel.ts", "export * from './starred.ts';"),
        (
            "use.ts",
            "import value from './default.ts'; import * as ns from './namespace.ts'; value; ns;",
        ),
    ]);
    assert_eq!(
        project
            .unused_exports()
            .iter()
            .map(|e| (e.path.to_str().unwrap(), e.export.name.as_str()))
            .collect::<Vec<_>>(),
        [("default.ts", "unused"), ("starred.ts", "default")]
    );
}

#[test]
fn local_uncertainty_and_ambiguous_modules_keep_their_reasons() {
    let project = analyze_project([
        (
            "use.ts",
            "import { value } from './a.ts'; value; unknown; function f() { return this; }",
        ),
        ("a.ts", "export const value = 1;"),
        ("dir/../a.ts", "export const value = 2;"),
        ("Page.astro", "<p>{Astro.url}</p>"),
    ]);
    assert!(
        matches!(definition(&project, "use.ts", "value;"), DefinitionResolution::Unresolved(DefinitionUnresolved::AmbiguousModule { candidates }) if candidates.len() == 2)
    );
    assert!(matches!(
        definition(&project, "use.ts", "unknown"),
        DefinitionResolution::Unresolved(DefinitionUnresolved::UnresolvedReference)
    ));
    assert!(matches!(
        definition(&project, "use.ts", "this"),
        DefinitionResolution::Unresolved(DefinitionUnresolved::RuntimeDependent)
    ));
    assert!(matches!(
        definition(&project, "Page.astro", "Astro"),
        DefinitionResolution::Unresolved(DefinitionUnresolved::Builtin)
    ));
}

#[test]
fn astro_browser_imports_keep_independent_bindings_and_follow_esm_defaults() {
    let page = "---\nimport value from './server.ts';\n---\n{value}<script>import value from './browser.js'; value;</script>";
    let project = analyze_project([
        ("Page.astro", page),
        ("server.ts", "export default function server() {}"),
        ("browser.js", "export default function browser() {}"),
    ]);
    assert_declaration(&project, "Page.astro", "value;", "browser.js", "browser");
    let Some(DefinitionResolution::Resolved(server)) =
        project.definition_at("Page.astro", page.find("{value}").unwrap() as u32 + 1)
    else {
        panic!()
    };
    assert_eq!(server.path, Path::new("server.ts"));
    assert_eq!(
        project
            .outgoing_imports("Page.astro")
            .iter()
            .map(|i| i.domain)
            .collect::<Vec<_>>(),
        [ScopeDomain::Server, ScopeDomain::BrowserModule]
    );
}

#[test]
fn literal_star_export_names_are_explicit_exports() {
    let project = analyze_project([
        ("value.ts", "const value = 1; export { value as '*' };"),
        (
            "barrel.ts",
            "export * from './other.ts'; export { '*' } from './value.ts';",
        ),
        ("other.ts", "const other = 2; export { other as '*' };"),
        (
            "use.ts",
            "import { '*' as literal } from './barrel.ts'; literal;",
        ),
        ("unused.ts", "const unused = 3; export { unused as '*' };"),
    ]);
    assert_declaration(&project, "use.ts", "literal", "value.ts", "value");
    assert_eq!(
        project
            .unused_exports()
            .iter()
            .map(|e| (e.path.to_str().unwrap(), e.export.name.as_str()))
            .collect::<Vec<_>>(),
        [("unused.ts", "*")]
    );
}
