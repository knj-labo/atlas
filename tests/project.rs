use atlas::{
    Builtin, ModuleRequest, ModuleResolution, ModuleResolver, ProjectSnapshot, ProjectSources,
    ReferenceKind, RelativeResolver, Resolution, ScopeDomain, analyze, analyze_project,
    analyze_project_with_resolver,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn offset(source: &str, needle: &str) -> u32 {
    source.find(needle).unwrap() as u32
}

fn resolved(path: &str) -> ModuleResolution {
    ModuleResolution::Resolved(path.into())
}

#[test]
fn aliases_types_namespaces_and_unused_imports_share_a_file_index() {
    let source = "---\nimport Button from '../components/Button.astro';\nimport { Button as Alias, type Props as P } from '../components/Button.astro';\nimport * as UI from '../components/Button.astro';\nimport Unused from '../components/Button.astro';\nimport '../components/Button.astro';\nconst props: P = {};\n---\n<Button></Button><Alias /><UI.Button />\n{[1].map((Button) => Button)}";
    let mut sources = BTreeMap::new();
    sources.insert("/v/pages/Page.astro", source);
    sources.insert("/v/components/Button.astro", "<button />");
    let snapshot = analyze_project(sources.clone());
    let imports = snapshot.incoming_imports("/v/components/Button.astro");
    assert_eq!(imports.len(), 5);
    assert!(
        imports
            .iter()
            .all(|import| import.domain == ScopeDomain::Server)
    );
    assert!(imports.last().unwrap().import.bindings.is_empty());
    let facts = snapshot
        .file("/v/pages/Page.astro")
        .unwrap()
        .analysis
        .as_ref()
        .as_ref()
        .unwrap();
    assert_eq!(facts, &analyze(source).unwrap());
    for (import, canonical) in imports.iter().zip(&facts.imports) {
        assert!(std::ptr::eq(import.import, canonical));
        assert_eq!(import.resolution, &resolved("/v/components/Button.astro"));
    }
    let uses = snapshot.references_to_file("/v/components/Button.astro");
    assert_eq!(
        uses.iter()
            .map(|usage| usage.reference.name.as_str())
            .collect::<Vec<_>>(),
        ["P", "Button", "Button", "Alias", "UI"]
    );
    assert_eq!(uses[0].reference.kind, ReferenceKind::Type);
    for usage in uses {
        assert_eq!(usage.path, Path::new("/v/pages/Page.astro"));
        assert!(std::ptr::eq(
            usage.reference,
            &facts.references[usage.reference.id.0]
        ));
        let by_position = snapshot
            .reference_at(usage.path, usage.reference.span.start)
            .unwrap();
        assert!(std::ptr::eq(usage.reference, by_position.reference));
        assert!(std::ptr::eq(
            usage.import.unwrap().import,
            by_position.import.unwrap().import
        ));
    }
    assert!(
        snapshot
            .reference_at("/v/pages/Page.astro", offset(source, "=> Button") + 3)
            .unwrap()
            .import
            .is_none()
    );
    // This is module lookup: the empty target need not declare a named export P.
    assert!(
        snapshot
            .file("/v/components/Button.astro")
            .unwrap()
            .analysis
            .as_ref()
            .as_ref()
            .unwrap()
            .declarations_named("P")
            .is_empty()
    );
}

#[test]
fn server_browser_and_uncertain_references_remain_separate() {
    let source = "---\nimport Button from './Server.astro';\n---\n<Button />{Astro.url}<Missing />\n<script>import Button from './Browser.astro'; Button;</script>\n<script>Button;</script>\n<script is:inline>const Button = 1; Button;</script>\n<script is:inline>Button; this;</script>";
    let mut sources = BTreeMap::new();
    sources.insert("Page.astro", source);
    sources.insert("Server.astro", "<div />");
    sources.insert("Browser.astro", "<div />");
    let snapshot = analyze_project(sources.clone());
    let server = snapshot.references_to_file("Server.astro");
    let browser = snapshot.references_to_file("Browser.astro");
    assert_eq!(server.len(), 1);
    assert_eq!(browser.len(), 1);
    assert_eq!(
        server[0].reference.span.start,
        offset(source, "<Button") + 1
    );
    assert_eq!(
        browser[0].reference.span.start,
        offset(source, "Button;</script>")
    );
    assert_eq!(server[0].import.unwrap().domain, ScopeDomain::Server);
    assert_eq!(
        browser[0].import.unwrap().domain,
        ScopeDomain::BrowserModule
    );
    let builtin = snapshot
        .reference_at("Page.astro", offset(source, "Astro.url"))
        .unwrap();
    assert_eq!(
        builtin.reference.resolution,
        Resolution::Builtin(Builtin::Astro)
    );
    assert!(builtin.import.is_none());
    for needle in ["Missing", "<script>Button"] {
        let byte = offset(source, needle) + if needle.starts_with('<') { 8 } else { 0 };
        let usage = snapshot.reference_at("Page.astro", byte).unwrap();
        assert_eq!(usage.reference.resolution, Resolution::Unresolved);
        assert!(usage.import.is_none());
    }
    let runtime = snapshot
        .reference_at("Page.astro", offset(source, "Button; this"))
        .unwrap();
    assert!(matches!(
        runtime.reference.resolution,
        Resolution::RuntimeDependent { .. }
    ));
    assert!(runtime.import.is_none());
}

#[test]
fn reverse_indexes_use_path_then_source_order_and_handle_cycles() {
    let mut sources = BTreeMap::new();
    sources.insert(
        "z.astro",
        "---\nimport A from './a.astro';\n---\n<A /><A />",
    );
    sources.insert("m.astro", "---\nimport A from './a.astro';\n---\n<A />");
    sources.insert("a.astro", "---\nimport Z from './z.astro';\n---\n<Z />");
    let snapshot = analyze_project(sources.clone());
    assert_eq!(
        snapshot.files().map(|(path, _)| path).collect::<Vec<_>>(),
        [
            Path::new("a.astro"),
            Path::new("m.astro"),
            Path::new("z.astro")
        ]
    );
    assert_eq!(
        snapshot
            .incoming_imports("a.astro")
            .iter()
            .map(|import| import.path)
            .collect::<Vec<_>>(),
        [Path::new("m.astro"), Path::new("z.astro")]
    );
    let uses = snapshot.references_to_file("a.astro");
    assert_eq!(
        uses.iter().map(|usage| usage.path).collect::<Vec<_>>(),
        [
            Path::new("m.astro"),
            Path::new("z.astro"),
            Path::new("z.astro")
        ]
    );
    assert!(uses[1].reference.span.start < uses[2].reference.span.start);
    assert_eq!(snapshot.references_to_file("z.astro").len(), 1);
    assert!(snapshot.references_to_file("m.astro").is_empty());
    assert!(snapshot.outgoing_imports("unknown").is_empty());
    assert!(snapshot.incoming_imports("unknown").is_empty());
    assert!(snapshot.references_to_file("unknown").is_empty());
    assert!(snapshot.reference_at("unknown", 0).is_none());
    assert!(snapshot.reference_at("a.astro", u32::MAX).is_none());
}

#[test]
fn relative_resolution_preserves_exact_keys_and_reports_ambiguity() {
    let mut sources = BTreeMap::new();
    sources.insert(
        "src/pages/Page.astro",
        "---\nimport Button from '../components/./Button.astro';\n---\n<Button />",
    );
    sources.insert("src/tmp/../components/Button.astro", "<button />");
    let first = analyze_project(sources.clone());
    assert_eq!(
        first.outgoing_imports("src/pages/Page.astro")[0].resolution,
        &resolved("src/tmp/../components/Button.astro")
    );
    assert!(first.file("src/components/Button.astro").is_none());
    assert_eq!(
        first
            .references_to_file("src/tmp/../components/Button.astro")
            .len(),
        1
    );

    sources.insert("src/components/Button.astro", "<button />");
    let ambiguous = analyze_project(sources.clone());
    assert_eq!(
        ambiguous.outgoing_imports("src/pages/Page.astro")[0].resolution,
        &ModuleResolution::Ambiguous(vec![
            "src/components/Button.astro".into(),
            "src/tmp/../components/Button.astro".into()
        ])
    );
    assert!(
        ambiguous
            .incoming_imports("src/components/Button.astro")
            .is_empty()
    );
    assert!(
        ambiguous
            .references_to_file("src/tmp/../components/Button.astro")
            .is_empty()
    );
    assert!(
        sources
            .remove("src/tmp/../components/Button.astro")
            .is_some()
    );
    assert_eq!(
        analyze_project(sources.clone())
            .references_to_file("src/components/Button.astro")
            .len(),
        1
    );

    sources.insert(
        "../pages/Relative.astro",
        "---\nimport B from '../Button.astro';\n---\n<B />",
    );
    sources.insert("../Button.astro", "<button />");
    sources.insert(
        "/pages/Absolute.astro",
        "---\nimport B from '../../Button.astro';\n---\n<B />",
    );
    sources.insert("/Button.astro", "<button />");
    let paths = analyze_project(sources.clone());
    assert_eq!(paths.references_to_file("../Button.astro").len(), 1);
    assert_eq!(paths.references_to_file("/Button.astro").len(), 1);
}

#[test]
fn default_resolver_does_not_guess_packages_extensions_indexes_or_query_suffixes() {
    for (specifier, expected) in [
        ("./Button.astro", resolved("Button.astro")),
        (".//Button.astro", resolved("Button.astro")),
        ("./Missing.astro", ModuleResolution::Unresolved),
        ("./Button", ModuleResolution::Unresolved),
        ("./button.astro", ModuleResolution::Unresolved),
        ("package", ModuleResolution::Unsupported),
        ("/Button.astro", ModuleResolution::Unsupported),
        (
            "https://example.test/Button.astro",
            ModuleResolution::Unsupported,
        ),
        ("./Button.astro?raw", ModuleResolution::Unsupported),
        ("./Button.astro#hash", ModuleResolution::Unsupported),
        ("./Button/", ModuleResolution::Unsupported),
        ("./Button.astro/.", ModuleResolution::Unsupported),
        ("./Button.astro/..", ModuleResolution::Unsupported),
    ] {
        let source = format!("---\nimport Button from {specifier:?};\n---\n<Button />");
        let snapshot = analyze_project([
            ("Button.astro", "<button />"),
            ("Button/index.astro", "<button />"),
            ("Page.astro", source.as_str()),
        ]);
        assert_eq!(
            snapshot.outgoing_imports("Page.astro")[0].resolution,
            &expected,
            "{specifier}"
        );
        let usage = snapshot
            .reference_at("Page.astro", offset(&source, "<Button") + 1)
            .unwrap();
        assert!(matches!(
            usage.reference.resolution,
            Resolution::Resolved(_)
        ));
        assert_eq!(usage.import.unwrap().resolution, &expected);
    }
}

#[test]
fn syntax_errors_are_visible_without_discarding_other_files_or_incoming_imports() {
    let mut sources = BTreeMap::new();
    sources.insert(
        "Page.astro",
        "---\nimport Broken from './Broken.astro';\n---\n<Broken />",
    );
    sources.insert("Broken.astro", "---\nconst =;\n---");
    let snapshot = analyze_project(sources.clone());
    assert!(snapshot.file("Broken.astro").unwrap().analysis.is_err());
    assert_eq!(snapshot.incoming_imports("Broken.astro").len(), 1);
    assert_eq!(snapshot.references_to_file("Broken.astro").len(), 1);
    assert!(snapshot.outgoing_imports("Broken.astro").is_empty());
    assert!(snapshot.reference_at("Broken.astro", 5).is_none());
    sources.insert("Broken.astro", "<button />");
    let fixed = analyze_project(sources.clone());
    assert!(fixed.file("Broken.astro").unwrap().analysis.is_ok());
    assert!(snapshot.file("Broken.astro").unwrap().analysis.is_err());
    sources.insert(
        "Page.astro",
        "---\nimport Broken from './Broken.astro';\nconst =;\n---",
    );
    let error = analyze_project(sources.clone());
    assert!(error.file("Page.astro").unwrap().analysis.is_err());
    assert!(error.incoming_imports("Broken.astro").is_empty());
    assert!(error.references_to_file("Broken.astro").is_empty());
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ProjectSnapshot>();
}

#[test]
fn resolver_receives_import_domain_even_for_side_effect_imports() {
    struct DomainResolver;
    impl ModuleResolver for DomainResolver {
        fn resolve(&self, request: ModuleRequest<'_>, _: &ProjectSources<'_>) -> ModuleResolution {
            match request.domain {
                ScopeDomain::Server => resolved("Server.astro"),
                ScopeDomain::BrowserModule => resolved("Browser.astro"),
                other => panic!("unexpected import domain: {other:?}"),
            }
        }
    }
    let mut sources = BTreeMap::new();
    sources.insert("Page.astro", "---\nimport '@setup';\n---\n<script>import '@setup';</script>\n<script is:inline type={\"module\"}>import '@setup';</script>");
    sources.insert("Server.astro", "");
    sources.insert("Browser.astro", "");
    let snapshot = analyze_project_with_resolver(sources, &DomainResolver);
    assert_eq!(snapshot.incoming_imports("Server.astro").len(), 1);
    assert_eq!(snapshot.incoming_imports("Browser.astro").len(), 2);
    assert!(snapshot.references_to_file("Server.astro").is_empty());
    assert!(snapshot.references_to_file("Browser.astro").is_empty());
}

#[test]
fn self_imports_in_server_and_browser_domains_keep_distinct_bindings() {
    let source = "---\nimport Self from './Page.astro';\n---\n<Self />\n<script>import Self from './Page.astro'; Self;</script>\n<script is:inline type={\"module\"}>import Self from './Page.astro'; Self;</script>";
    let mut sources = BTreeMap::new();
    sources.insert("Page.astro", source);
    let snapshot = analyze_project(sources.clone());
    let imports = snapshot.incoming_imports("Page.astro");
    assert_eq!(imports.len(), 3);
    assert_eq!(
        imports
            .iter()
            .map(|import| import.domain)
            .collect::<Vec<_>>(),
        [
            ScopeDomain::Server,
            ScopeDomain::BrowserModule,
            ScopeDomain::BrowserModule
        ]
    );
    let uses = snapshot.references_to_file("Page.astro");
    assert_eq!(uses.len(), 3);
    let mut ids = std::collections::HashSet::new();
    for (import, usage) in imports.iter().zip(uses) {
        assert!(std::ptr::eq(import.import, usage.import.unwrap().import));
        let Resolution::Resolved(binding) = &usage.reference.resolution else {
            panic!("expected a definite self-import")
        };
        assert!(ids.insert(binding.id));
    }
}

#[test]
fn empty_inputs_and_duplicate_paths_have_explicit_results() {
    let empty = analyze_project(std::iter::empty::<(&str, &str)>());
    assert_eq!(empty.files().len(), 0);
    assert_eq!(empty.revision(), None);
    assert!(empty.outgoing_imports("unknown").is_empty());
    assert!(empty.incoming_imports("unknown").is_empty());
    assert!(empty.references_to_file("unknown").is_empty());
    assert!(empty.reference_at("unknown", 0).is_none());

    let source = "---\nimport Button from './Button.astro';\n---\n<Button />";
    let project = analyze_project(vec![
        (
            PathBuf::from("Page.astro"),
            String::from("---\nconst =;\n---"),
        ),
        (PathBuf::from("Button.astro"), String::from("<button />")),
        (PathBuf::from("Page.astro"), source.to_owned()),
    ]);
    assert_eq!(project.files().len(), 2);
    assert_eq!(project.revision(), None);
    assert_eq!(project.file("Page.astro").unwrap().source.as_ref(), source);
    assert!(project.file("Page.astro").unwrap().analysis.is_ok());
    assert_eq!(project.references_to_file("Button.astro").len(), 1);
}

#[test]
fn uncached_snapshots_own_unicode_sources_and_preserve_byte_positions() {
    let source = "---\r\nimport 名前 from './部品.astro';\r\n---\r\n<p>😀</p>{名前}";
    let project = {
        let text: Arc<str> = Arc::from(source);
        let sources = vec![
            (PathBuf::from("Page.astro"), Arc::clone(&text)),
            (PathBuf::from("部品.astro"), Arc::from("<button />")),
        ];
        let project = analyze_project(sources);
        assert!(Arc::ptr_eq(
            &text,
            &project.file("Page.astro").unwrap().source
        ));
        project
    };
    assert_eq!(project.file("Page.astro").unwrap().source.as_ref(), source);
    let uses = project.references_to_file("部品.astro");
    assert_eq!(uses.len(), 1);
    let reference = uses[0].reference;
    assert_eq!(reference.span.start, offset(source, "{名前}") + 1);
    for byte in reference.span.clone() {
        assert!(std::ptr::eq(
            project.reference_at("Page.astro", byte).unwrap().reference,
            reference,
        ));
    }
    assert!(
        project
            .reference_at("Page.astro", reference.span.end)
            .is_none()
    );
    assert!(
        project
            .reference_at("Page.astro", source.len() as u32)
            .is_none()
    );
}

#[test]
fn borrowed_resolvers_run_per_analysis_and_are_not_retained_by_queries() {
    struct BorrowedResolver<'a> {
        target: &'a str,
        calls: &'a AtomicUsize,
    }
    impl ModuleResolver for BorrowedResolver<'_> {
        fn resolve(
            &self,
            request: ModuleRequest<'_>,
            sources: &ProjectSources<'_>,
        ) -> ModuleResolution {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if request.import.source == "@button" {
                resolved(self.target)
            } else {
                RelativeResolver.resolve(request, sources)
            }
        }
    }
    let (first, second) = {
        let target = String::from("Button.astro");
        let calls = AtomicUsize::new(0);
        let resolver = BorrowedResolver {
            target: &target,
            calls: &calls,
        };
        let source = "---\nimport Button from '@button';\n---\n<Button />";
        let sources = [("Page.astro", source), ("Button.astro", "<button />")];
        let first = analyze_project_with_resolver(sources, &resolver);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let second = analyze_project_with_resolver(sources, &resolver);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        for project in [&first, &second] {
            assert_eq!(project.revision(), None);
            assert_eq!(project.outgoing_imports("Page.astro").len(), 1);
            assert_eq!(project.incoming_imports("Button.astro").len(), 1);
            assert_eq!(project.references_to_file("Button.astro").len(), 1);
            assert!(
                project
                    .reference_at("Page.astro", offset(source, "<Button") + 1)
                    .unwrap()
                    .import
                    .is_some()
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        (first, second)
    };
    assert_eq!(first.references_to_file("Button.astro").len(), 1);
    assert_eq!(second.references_to_file("Button.astro").len(), 1);
}

#[test]
fn component_uses_carry_element_attributes_and_skip_shadowed_names() {
    let page = "---\nimport Card from './Card.astro';\nimport * as ui from './Card.astro';\n---\n<Card title=\"a\" client:load slot=\"x\" />\n<ui.Card />\n{[1].map((Card) => <Card />)}";
    let snapshot = analyze_project([("/v/Page.astro", page), ("/v/Card.astro", "<div />")]);
    let uses = snapshot.component_uses_of("/v/Card.astro");
    let names: Vec<_> = uses.iter().map(|u| u.component.name.as_str()).collect();
    assert_eq!(names, ["Card", "ui.Card"]);
    assert_eq!(uses[0].path, Path::new("/v/Page.astro"));
    assert_eq!(uses[0].element.attributes[0].name.as_deref(), Some("title"));
    assert_eq!(uses[0].component.directives.len(), 1);
    assert!(uses[0].element.slot.is_some());
}

#[test]
fn unused_exports_ignore_astro_roles_and_respect_named_and_namespace_imports() {
    let lib = "---\nexport interface Props {}\nexport const prerender = true;\nexport const used = 1;\nexport const unused = 2;\n---";
    let ns = "---\nexport const viaNamespace = 1;\n---";
    let page = "---\nimport { used } from './Lib.astro';\nimport * as all from './Ns.astro';\n---\n{used}{all}";
    let snapshot = analyze_project([
        ("/v/Lib.astro", lib),
        ("/v/Ns.astro", ns),
        ("/v/Page.astro", page),
    ]);
    let unused: Vec<_> = snapshot
        .unused_exports()
        .iter()
        .map(|e| (e.path.to_str().unwrap(), e.export.name.as_str()))
        .collect();
    assert_eq!(unused, [("/v/Lib.astro", "unused")]);
}
