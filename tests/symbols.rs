use atlas::{
    DeclarationKind, DefinitionKind, DefinitionResolution, DefinitionUnresolved, ExportLocal,
    Meaning, ModuleDependency, ModuleRequest, ModuleResolution, ModuleResolver, ProjectSnapshot,
    ProjectSources, RelativeResolver, Resolution, TransferStatus, analyze, analyze_project,
    analyze_project_with_resolver,
};
use std::{path::Path, sync::Mutex};

fn target<'a>(project: &'a ProjectSnapshot, file: &str, offset: usize) -> atlas::Definition<'a> {
    match project.definition_at(file, offset as u32).unwrap() {
        DefinitionResolution::Resolved(target) => target,
        other => panic!("{file}:{offset}: {other:?}"),
    }
}

#[test]
fn type_import_and_same_name_const_select_physical_declarations_in_either_order() {
    for declarations in [
        "import type { Item } from './types.ts'; const Item = 1;",
        "const Item = 1; import type { Item } from './types.ts';",
    ] {
        let source = format!("{declarations} let typed: Item; type Q = typeof Item; Item;");
        let project = analyze_project([
            ("types.ts", "export interface Item { name: string }"),
            ("use.ts", &source),
        ]);
        let facts = project
            .file("use.ts")
            .unwrap()
            .analysis
            .as_ref()
            .as_ref()
            .unwrap();
        let bindings = facts.declarations_named("Item");
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].symbol, bindings[1].symbol);
        let symbol = &facts.symbols[bindings[0].symbol.0];
        assert_eq!(symbol.declarations.len(), 2);
        assert_eq!(symbol.declarations[0].0, 0, "original representative ID");
        let import = &facts.imports[0].bindings[0];
        assert_eq!(
            facts.declarations[import.declaration.0].kind,
            DeclarationKind::Import
        );
        let type_offset = source.find(": Item").unwrap() + 2;
        let value_offset = source.rfind("Item").unwrap();
        let query_offset = source.find("typeof Item").unwrap() + 7;
        for offset in [
            type_offset,
            value_offset,
            query_offset,
            value_offset,
            type_offset,
        ] {
            let definition = target(&project, "use.ts", offset);
            assert_eq!(
                definition.path,
                Path::new(if offset == type_offset {
                    "types.ts"
                } else {
                    "use.ts"
                })
            );
            assert!(
                matches!(definition.kind, DefinitionKind::Declaration(binding)
                if binding.kind == if offset == type_offset { DeclarationKind::Type } else { DeclarationKind::Variable })
            );
        }
        assert_eq!(project.references_to_file("types.ts").len(), 1);
        #[cfg(feature = "serde")]
        {
            let report = project.report();
            let references = &report
                .files
                .iter()
                .find(|f| f.path == "use.ts")
                .unwrap()
                .references;
            assert_eq!(references.len(), 1);
            assert_eq!(
                references[0].key,
                format!("use.ts#{type_offset}-{}", type_offset + 4)
            );
        }
    }
}

#[test]
fn transfer_uses_selected_const_initializer_after_interface_merges() {
    for declarations in [
        "interface value { field: string } const value = 7;",
        "const value = 7; interface value { field: string }",
    ] {
        let source = format!(
            "---\n{declarations}\n---\n<script define:vars={{{{ value }}}}>value;</script>"
        );
        let facts = analyze(&source).unwrap();
        let assessment = facts.assess_transfer(facts.boundaries[0].id).unwrap();
        assert_eq!(assessment.inputs[0].status, TransferStatus::Supported);
        let origin = &facts.boundaries[0].inputs[0].value.origins[0];
        assert_eq!(origin.initializer.raw, "7");
        assert_eq!(
            facts.declarations[origin.declaration.0].kind,
            DeclarationKind::Variable
        );
    }
}

#[test]
fn merged_interfaces_overloads_and_namespace_declarations_have_one_representative() {
    let source = "export interface Item { a: string } export interface Item { b: number }\n\
        export function call(x: string): string; export function call(x: number): number;\n\
        export function call(x: unknown) { return x; }\n\
        export class Box {} export namespace Box { export type Member = string; }\n\
        export enum Color { Red } export namespace Color { export const blue = 1; }";
    let use_source = "import { Item, call, Box, Color } from './source.ts'; let i: Item; call; Box; Color; let b: Box; let m: Box.Member; type C = typeof call;";
    let project = analyze_project([("source.ts", source), ("use.ts", use_source)]);
    let facts = project
        .file("source.ts")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    assert_eq!(facts.declarations_named("Item").len(), 2);
    assert_eq!(facts.declarations_named("call").len(), 3);
    let implementation = source.find("call(x: unknown)").unwrap();
    for reference in &project
        .file("use.ts")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap()
        .references
    {
        let DefinitionResolution::Resolved(definition) = project
            .definition_at("use.ts", reference.span.start)
            .unwrap()
        else {
            panic!("merged declaration: {reference:?}")
        };
        assert_eq!(definition.path, Path::new("source.ts"));
        if reference.name == "call" {
            assert_eq!(definition.span().start as usize, implementation);
        }
    }
    let namespace_offset = use_source.find("Box.Member").unwrap();
    assert!(matches!(target(&project, "use.ts", namespace_offset).kind,
        DefinitionKind::Declaration(b) if b.meanings.contains(&Meaning::Namespace)));
}

#[test]
fn signatures_without_implementation_choose_first_signature() {
    let source = "export declare function call(x: string): string; export declare function call(x: number): number;";
    let project = analyze_project([
        ("source.ts", source),
        ("use.ts", "import { call } from './source.ts'; call;"),
    ]);
    let use_source = &project.file("use.ts").unwrap().source;
    let definition = target(&project, "use.ts", use_source.rfind("call").unwrap());
    assert_eq!(
        definition.span().start as usize,
        source.find("call").unwrap()
    );
}

#[test]
fn explicit_export_meaning_does_not_fall_through_to_stars() {
    for local in [
        "export interface Item {}",
        "interface Item {} export { Item };",
        "interface Item {} export type { Item };",
        "export type { Item } from './types.ts';",
    ] {
        let middle = format!("{local} export * from './values.ts';");
        let use_source = "import { Item } from './middle.ts'; let typed: Item; Item;";
        let project = analyze_project([
            ("middle.ts", middle.as_str()),
            ("types.ts", "export interface Item {}"),
            ("values.ts", "export const Item = 1;"),
            ("use.ts", use_source),
        ]);
        let typed = target(&project, "use.ts", use_source.find(": Item").unwrap() + 2);
        assert_ne!(typed.path, Path::new("values.ts"));
        assert!(matches!(
            project.definition_at("use.ts", use_source.rfind("Item").unwrap() as u32),
            Some(DefinitionResolution::Unresolved(
                DefinitionUnresolved::UnresolvedReference
            ))
        ));
    }
}

#[test]
fn direct_exports_retain_their_physical_meaning_while_lists_expose_symbols() {
    for declarations in [
        "export interface Item {} const Item = 1;",
        "const Item = 1; export interface Item {}",
    ] {
        let project = analyze_project([
            ("source.ts", declarations),
            ("use.ts", "import { Item } from './source.ts'; Item;"),
        ]);
        let facts = project
            .file("source.ts")
            .unwrap()
            .analysis
            .as_ref()
            .as_ref()
            .unwrap();
        let export = &facts.exports[0];
        assert!(export.type_only);
        assert!(
            matches!(export.target, Some(ExportLocal::Declaration(id)) if facts.declarations[id.0].kind == DeclarationKind::Type)
        );
        assert!(matches!(
            project.definition_at(
                "use.ts",
                project
                    .file("use.ts")
                    .unwrap()
                    .source
                    .rfind("Item")
                    .unwrap() as u32
            ),
            Some(DefinitionResolution::Unresolved(
                DefinitionUnresolved::UnresolvedReference
            ))
        ));
    }
    let source = "interface Item {} const Item = 1; export { Item };";
    let use_source = "import { Item } from './source.ts'; let typed: Item; Item;";
    let project = analyze_project([("source.ts", source), ("use.ts", use_source)]);
    for (offset, kind) in [
        (
            use_source.find(": Item").unwrap() + 2,
            DeclarationKind::Type,
        ),
        (use_source.rfind("Item").unwrap(), DeclarationKind::Variable),
    ] {
        assert!(
            matches!(target(&project, "use.ts", offset).kind, DefinitionKind::Declaration(b) if b.kind == kind)
        );
    }
}

#[test]
fn type_queries_cross_type_only_aliases_but_runtime_values_do_not() {
    for middle in [
        "export type { value } from './source.ts';",
        "export type * from './source.ts';",
        "import type { value } from './source.ts'; export { value };",
    ] {
        let source = "import type { value } from './source.ts'; const value = 2; export { value };";
        let use_source = "import type { value as typed } from './middle.ts'; import { value as plain } from './middle.ts'; type T = typeof typed; type U = typeof plain; plain;";
        let project = analyze_project([
            ("origin.ts", "export const value = 1;"),
            ("source.ts", source),
            ("middle.ts", middle),
            ("use.ts", use_source),
        ]);
        for name in ["typed", "plain"] {
            let offset = use_source.find(&format!("typeof {name}")).unwrap() + 7;
            assert_eq!(
                project
                    .reference_at("use.ts", offset as u32)
                    .unwrap()
                    .reference
                    .meaning,
                Meaning::ValueAsType
            );
            let definition = target(&project, "use.ts", offset);
            assert_eq!(definition.path, Path::new("source.ts"));
            assert!(
                matches!(definition.kind, DefinitionKind::Declaration(b) if b.kind == DeclarationKind::Variable)
            );
        }
        assert!(matches!(
            project.definition_at("use.ts", use_source.rfind("plain").unwrap() as u32),
            Some(DefinitionResolution::Unresolved(
                DefinitionUnresolved::UnresolvedReference
            ))
        ));
    }
}

#[test]
fn browser_imports_share_the_declaration_selection_rules() {
    let source = "<script>import type { Item } from './types.ts'; const Item = 1; let t: Item; Item; type T = typeof Item;</script>";
    let project = analyze_project([
        ("types.ts", "export interface Item {}"),
        ("Page.astro", source),
    ]);
    let facts = project
        .file("Page.astro")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    for reference in &facts.scripts[0].references {
        let definition = target(&project, "Page.astro", reference.span.start as usize);
        assert_eq!(
            definition.path,
            Path::new(if reference.meaning == Meaning::Type {
                "types.ts"
            } else {
                "Page.astro"
            })
        );
    }
    assert_eq!(project.references_to_file("types.ts").len(), 1);
}

#[test]
fn resolver_receives_original_import_and_export_records_and_utf8_ranges() {
    #[derive(Default)]
    struct Inspect(Mutex<Vec<String>>);
    impl ModuleResolver for Inspect {
        fn resolve(
            &self,
            request: ModuleRequest<'_>,
            sources: &ProjectSources<'_>,
        ) -> ModuleResolution {
            let source = sources.source(request.importer).unwrap();
            let whole = &source[request.span.start as usize..request.span.end as usize];
            let literal =
                &source[request.source_span.start as usize..request.source_span.end as usize];
            assert_eq!(literal, "'./値.ts'");
            assert_eq!(request.source, "./値.ts");
            match request.dependency {
                ModuleDependency::Import(import) => {
                    assert_eq!(whole, "import type { 値 } from './値.ts';");
                    assert_eq!(request.span, import.span);
                    assert!(import.bindings[0].target.type_only);
                    assert!(import.bindings[0].declaration.0 < usize::MAX);
                }
                ModuleDependency::Export(export) => {
                    assert_eq!(request.span, export.declaration_span);
                    assert_eq!(request.source_span, export.source_span.clone().unwrap());
                    assert!(export.type_only);
                    assert!(whole.starts_with("export type "));
                }
            }
            self.0.lock().unwrap().push(whole.to_owned());
            RelativeResolver.resolve(request, sources)
        }
    }
    let resolver = Inspect::default();
    let source = "// 日本語\r\nimport type { 値 } from './値.ts';\r\nexport type { 値 as 名前 } from './値.ts';\r\nexport type * from './値.ts';\r\nlet x: 値;";
    let project = analyze_project_with_resolver(
        [("値.ts", "export interface 値 {}"), ("use.ts", source)],
        &resolver,
    );
    assert_eq!(resolver.0.lock().unwrap().len(), 3);
    for _ in 0..3 {
        target(&project, "use.ts", source.rfind("値").unwrap());
        project.unused_exports();
        project.component_uses_of("値.ts");
        #[cfg(feature = "serde")]
        {
            project.report();
        }
    }
    assert_eq!(resolver.0.lock().unwrap().len(), 3);
}

#[test]
fn snapshot_queries_are_consistent_across_threads_and_scc_entry_points() {
    let project = analyze_project([
        (
            "a.ts",
            "export * from './b.ts'; export * from './value.ts';",
        ),
        ("b.ts", "export * from './c.ts';"),
        ("c.ts", "export * from './a.ts';"),
        ("value.ts", "export const value = 1;"),
        (
            "use.ts",
            "import { value as a } from './a.ts'; import { value as b } from './b.ts'; import { value as c } from './c.ts'; a; b; c;",
        ),
    ]);
    std::thread::scope(|scope| {
        for reverse in [false, true, true, false, false, true, false, true] {
            let project = &project;
            scope.spawn(move || {
                let facts = project
                    .file("use.ts")
                    .unwrap()
                    .analysis
                    .as_ref()
                    .as_ref()
                    .unwrap();
                let mut references: Vec<_> = facts.references.iter().collect();
                if reverse {
                    references.reverse();
                }
                for reference in references {
                    assert!(matches!(&reference.resolution, Resolution::Resolved(_)));
                    assert_eq!(
                        target(project, "use.ts", reference.span.start as usize).path,
                        Path::new("value.ts")
                    );
                }
                #[cfg(feature = "serde")]
                {
                    project.report();
                }
            });
        }
    });
}

#[test]
fn failure_representatives_and_pure_cycles_are_independent_of_entry_and_order() {
    for stars in [
        "export * from './z.ts'; export * from './a.ts';",
        "export * from './a.ts'; export * from './z.ts';",
    ] {
        let project = analyze_project([
            ("a.ts", "export * from './absent.ts';"),
            ("z.ts", "export * from 'package';"),
            ("root.ts", stars),
            (
                "use.ts",
                "import { value as root } from './root.ts'; import { value as a } from './a.ts'; root; a;",
            ),
        ]);
        let source = &project.file("use.ts").unwrap().source;
        for name in ["root", "a", "a", "root"] {
            assert!(matches!(
                project.definition_at("use.ts", source.rfind(name).unwrap() as u32),
                Some(DefinitionResolution::Unresolved(
                    DefinitionUnresolved::UnresolvedModule
                ))
            ));
        }
    }
    for order in [["left", "right"], ["right", "left"]] {
        let project = analyze_project([
            ("a.ts", "export * from './b.ts';"),
            ("b.ts", "export * from './a.ts';"),
            (
                "use.ts",
                "import { value as left } from './a.ts'; import { value as right } from './b.ts'; left; right;",
            ),
        ]);
        let source = &project.file("use.ts").unwrap().source;
        for name in order {
            assert!(
                matches!(project.definition_at("use.ts", source.rfind(name).unwrap() as u32),
                Some(DefinitionResolution::Unresolved(DefinitionUnresolved::Cycle { path, name }))
                    if path == Path::new("a.ts") && name == "value")
            );
        }
    }
}

#[test]
fn primary_ids_across_server_and_browser_domains_precede_extra_declarations() {
    let source = "---\ninterface value {} const value = 7;\n---\n<script>const browser = 1; browser;</script><script define:vars={{ value }}>value;</script>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.symbols[0].declarations[0], atlas::DeclarationId(0));
    assert_eq!(
        facts.declarations_named("browser")[0].id,
        atlas::DeclarationId(1)
    );
    assert_eq!(facts.scripts[1].injected, [atlas::DeclarationId(2)]);
    assert_eq!(facts.declarations.len(), 4);
    let value = &facts.boundaries[0].inputs[0].value;
    assert_eq!(value.origins[0].declaration, atlas::DeclarationId(3));
    for reference in &facts.references {
        if let Resolution::Resolved(binding) = &reference.resolution {
            assert_eq!(&facts.declarations[binding.id.0], binding);
            assert!(
                facts.symbols[binding.symbol.0]
                    .declarations
                    .contains(&binding.id)
            );
        }
    }
    for import in &facts.imports {
        for binding in &import.bindings {
            assert_eq!(facts.declarations[binding.declaration.0].span, binding.span);
        }
    }
}

#[cfg(feature = "incremental")]
#[test]
fn retained_snapshots_keep_merged_ids_sources_and_results_after_all_update_kinds() {
    let mut db = atlas::AnalysisDatabase::new();
    db.set_source("types.ts", "export interface Item {}");
    let source = "import type { Item } from './types.ts'; const Item = 1; let t: Item; Item;";
    db.set_source("use.ts", source);
    let retained = db.analyze_project();
    let original = target(&retained, "use.ts", source.rfind("Item").unwrap());
    let span = original.span();
    db.set_source(
        "use.ts",
        "const Item = 2; import type { Item } from './types.ts'; let t: Item; Item;",
    );
    let updated = db.analyze_project();
    assert_eq!(
        target(
            &updated,
            "use.ts",
            updated
                .file("use.ts")
                .unwrap()
                .source
                .rfind("Item")
                .unwrap()
        )
        .path,
        Path::new("use.ts")
    );
    db.remove_file("types.ts");
    let removed = db.analyze_project();
    let offset = removed
        .file("use.ts")
        .unwrap()
        .source
        .find(": Item")
        .unwrap()
        + 2;
    assert!(matches!(
        removed.definition_at("use.ts", offset as u32),
        Some(DefinitionResolution::Unresolved(
            DefinitionUnresolved::UnresolvedModule
        ))
    ));
    db.set_source("types.ts", "export interface Item { changed: true }");
    let restored = db.analyze_project();
    assert_eq!(
        target(&restored, "use.ts", offset).path,
        Path::new("types.ts")
    );
    drop(db);
    assert_eq!(retained.file("use.ts").unwrap().source.as_ref(), source);
    assert_eq!(
        target(&retained, "use.ts", source.rfind("Item").unwrap()).span(),
        span
    );
    assert_eq!(
        target(&retained, "use.ts", source.find(": Item").unwrap() + 2).path,
        Path::new("types.ts")
    );
    #[cfg(feature = "serde")]
    {
        assert_eq!(retained.report(), retained.report());
    }
}

#[cfg(feature = "serde")]
#[test]
fn merged_facts_and_type_queries_preserve_schema_one_and_strict_toon_roundtrip() {
    let source = "import type { Item } from './types.ts'; const Item = 1; export { Item }; let t: Item; type T = typeof Item; Item;";
    let project = analyze_project([
        (
            "types.ts",
            "export interface Item {} export interface Item { name: string }",
        ),
        ("use.ts", source),
    ]);
    let report = project.report();
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema_version"], 1);
    let encoded = toon_format::encode_default(&report).unwrap();
    let decoded: serde_json::Value = toon_format::decode_strict(&encoded).unwrap();
    assert_eq!(decoded, json);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&report).unwrap())
            .unwrap(),
        json
    );
    let facts = project
        .file("use.ts")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    let serialized = serde_json::to_value(facts).unwrap();
    assert!(serialized.get("symbols").is_none());
    for declaration in serialized["declarations"].as_array().unwrap() {
        assert!(declaration.get("symbol").is_none());
        assert!(declaration.get("meanings").is_none());
        assert!(declaration.get("function_implementation").is_none());
    }
    assert!(
        serialized["imports"][0]["bindings"][0]
            .get("declaration")
            .is_none()
    );
    assert!(serialized["references"][0].get("meaning").is_none());
    for export in serialized["exports"].as_array().unwrap() {
        for field in ["target", "meanings", "declaration_span", "source_span"] {
            assert!(export.get(field).is_none());
        }
    }
}

#[test]
fn dependency_requests_preserve_default_namespace_alias_star_and_browser_forms() {
    #[derive(Default)]
    struct Inspect(Mutex<Vec<(String, usize, atlas::ScopeDomain)>>);
    impl ModuleResolver for Inspect {
        fn resolve(
            &self,
            request: ModuleRequest<'_>,
            sources: &ProjectSources<'_>,
        ) -> ModuleResolution {
            let source = sources.source(request.importer).unwrap();
            let full = &source[request.span.start as usize..request.span.end as usize];
            assert_eq!(
                &source[request.source_span.start as usize..request.source_span.end as usize],
                "'./source.ts'"
            );
            let pointer = match request.dependency {
                ModuleDependency::Import(import) => {
                    if full.contains("Default") {
                        assert_eq!(
                            import.bindings[0].target.specifier,
                            atlas::ImportSpecifier::Default
                        );
                        assert_eq!(
                            import.bindings[1].target.specifier,
                            atlas::ImportSpecifier::Named("value".into())
                        );
                        assert!(import.bindings[2].target.type_only);
                    }
                    if full.contains("* as") {
                        assert_eq!(
                            import.bindings[0].target.specifier,
                            atlas::ImportSpecifier::Namespace
                        );
                    }
                    import as *const _ as usize
                }
                ModuleDependency::Export(export) => {
                    if full.contains("* as") {
                        assert_eq!(export.imported, Some(atlas::ImportSpecifier::Namespace));
                    }
                    if full.contains("type") {
                        assert!(export.type_only);
                    }
                    export as *const _ as usize
                }
            };
            self.0
                .lock()
                .unwrap()
                .push((full.into(), pointer, request.domain));
            RelativeResolver.resolve(request, sources)
        }
    }
    let resolver = Inspect::default();
    let source = "import Default, { value as local, type Item } from './source.ts';\nimport * as ns from './source.ts';\nimport './source.ts';\nexport { default as renamed } from './source.ts';\nexport * as all from './source.ts';\nexport type * as types from './source.ts';\nexport * from './source.ts';";
    let browser = "---\nimport type { Item } from './source.ts';\n---\n<script>import * as browser from './source.ts'; browser;</script>";
    let project = analyze_project_with_resolver(
        [
            (
                "source.ts",
                "export default 1; export const value = 1; export interface Item {}",
            ),
            ("use.ts", source),
            ("Page.astro", browser),
        ],
        &resolver,
    );
    let seen = resolver.0.lock().unwrap();
    assert_eq!(seen.len(), 9);
    for path in ["use.ts", "Page.astro"] {
        for import in project.outgoing_imports(path) {
            assert!(seen.iter().any(|(_, pointer, domain)| *pointer
                == import.import as *const _ as usize
                && *domain == import.domain));
        }
        for export in project.outgoing_reexports(path) {
            assert!(
                seen.iter()
                    .any(|(_, pointer, _)| *pointer == export.export as *const _ as usize)
            );
        }
    }
    assert!(
        seen.iter()
            .any(|(_, _, domain)| *domain == atlas::ScopeDomain::BrowserModule)
    );
    assert!(
        seen.iter()
            .any(|(_, _, domain)| *domain == atlas::ScopeDomain::Server)
    );
}

#[test]
fn type_only_stars_do_not_block_value_eligible_sibling_stars() {
    for middle in [
        "export type * from './types.ts'; export * from './values.ts';",
        "export * from './values.ts'; export type * from './types.ts';",
    ] {
        let source = "import { Item } from './middle.ts'; Item;";
        let project = analyze_project([
            ("types.ts", "export interface Item {}"),
            ("values.ts", "export const Item = 1;"),
            ("middle.ts", middle),
            ("use.ts", source),
        ]);
        assert_eq!(
            target(&project, "use.ts", source.rfind("Item").unwrap()).path,
            Path::new("values.ts")
        );
    }
}

#[test]
fn local_type_only_value_exports_remain_available_to_type_queries() {
    let source = "const value = 1; export type { value };";
    let use_source = "import type { value } from './source.ts'; type T = typeof value;";
    let project = analyze_project([("source.ts", source), ("use.ts", use_source)]);
    let definition = target(&project, "use.ts", use_source.rfind("value").unwrap());
    assert_eq!(definition.path, Path::new("source.ts"));
    assert!(
        matches!(definition.kind, DefinitionKind::Declaration(b) if b.kind == DeclarationKind::Variable)
    );
}
