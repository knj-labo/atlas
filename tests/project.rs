#![cfg(feature = "incremental")]

use atlas::{
    AnalysisDatabase, Builtin, ModuleRequest, ModuleResolution, ModuleResolver, ProjectSnapshot,
    ProjectSources, ReferenceKind, RelativeResolver, Resolution, ScopeDomain, analyze,
};
use std::{
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
    let mut db = AnalysisDatabase::new();
    db.set_source("/v/pages/Page.astro", source);
    db.set_source("/v/components/Button.astro", "<button />");
    let snapshot = db.analyze_project();
    let imports = snapshot.imports_to("/v/components/Button.astro");
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
    let mut db = AnalysisDatabase::new();
    db.set_source("Page.astro", source);
    db.set_source("Server.astro", "<div />");
    db.set_source("Browser.astro", "<div />");
    let snapshot = db.analyze_project();
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
    let mut db = AnalysisDatabase::new();
    db.set_source(
        "z.astro",
        "---\nimport A from './a.astro';\n---\n<A /><A />",
    );
    db.set_source("m.astro", "---\nimport A from './a.astro';\n---\n<A />");
    db.set_source("a.astro", "---\nimport Z from './z.astro';\n---\n<Z />");
    let snapshot = db.analyze_project();
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
            .imports_to("a.astro")
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
    assert!(snapshot.imports_from("unknown").is_empty());
    assert!(snapshot.imports_to("unknown").is_empty());
    assert!(snapshot.references_to_file("unknown").is_empty());
    assert!(snapshot.reference_at("unknown", 0).is_none());
    assert!(snapshot.reference_at("a.astro", u32::MAX).is_none());
}

#[test]
fn relative_resolution_preserves_exact_keys_and_reports_ambiguity() {
    let mut db = AnalysisDatabase::new();
    db.set_source(
        "src/pages/Page.astro",
        "---\nimport Button from '../components/./Button.astro';\n---\n<Button />",
    );
    db.set_source("src/tmp/../components/Button.astro", "<button />");
    let first = db.analyze_project();
    assert_eq!(
        first.imports_from("src/pages/Page.astro")[0].resolution,
        &resolved("src/tmp/../components/Button.astro")
    );
    assert!(db.analyze_file("src/components/Button.astro").is_none());
    assert_eq!(
        first
            .references_to_file("src/tmp/../components/Button.astro")
            .len(),
        1
    );

    db.set_source("src/components/Button.astro", "<button />");
    let ambiguous = db.analyze_project();
    assert_eq!(
        ambiguous.imports_from("src/pages/Page.astro")[0].resolution,
        &ModuleResolution::Ambiguous(vec![
            "src/components/Button.astro".into(),
            "src/tmp/../components/Button.astro".into()
        ])
    );
    assert!(
        ambiguous
            .imports_to("src/components/Button.astro")
            .is_empty()
    );
    assert!(
        ambiguous
            .references_to_file("src/tmp/../components/Button.astro")
            .is_empty()
    );
    assert!(db.remove_file("src/tmp/../components/Button.astro"));
    assert_eq!(
        db.analyze_project()
            .references_to_file("src/components/Button.astro")
            .len(),
        1
    );

    db.set_source(
        "../pages/Relative.astro",
        "---\nimport B from '../Button.astro';\n---\n<B />",
    );
    db.set_source("../Button.astro", "<button />");
    db.set_source(
        "/pages/Absolute.astro",
        "---\nimport B from '../../Button.astro';\n---\n<B />",
    );
    db.set_source("/Button.astro", "<button />");
    let paths = db.analyze_project();
    assert_eq!(paths.references_to_file("../Button.astro").len(), 1);
    assert_eq!(paths.references_to_file("/Button.astro").len(), 1);
}

#[test]
fn default_resolver_does_not_guess_packages_extensions_indexes_or_query_suffixes() {
    let mut db = AnalysisDatabase::new();
    db.set_source("Button.astro", "<button />");
    db.set_source("Button/index.astro", "<button />");
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
        db.set_source("Page.astro", &source);
        let snapshot = db.analyze_project();
        assert_eq!(
            snapshot.imports_from("Page.astro")[0].resolution,
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
fn additions_removals_updates_and_readdition_invalidate_project_results() {
    let source = "---\r\nimport 名前 from './部品.astro';\r\n---\r\n<p>😀</p>{名前}";
    let mut db = AnalysisDatabase::new();
    let empty = db.analyze_project();
    assert_eq!(empty.files().len(), 0);
    assert!(Arc::ptr_eq(&empty, &db.analyze_project()));
    db.set_source("Page.astro", source);
    let original_analysis = db.analyze_file("Page.astro").unwrap();
    let missing = db.analyze_project();
    assert_eq!(
        missing.imports_from("Page.astro")[0].resolution,
        &ModuleResolution::Unresolved
    );
    db.set_source("Page.astro", source);
    assert!(!db.remove_file("unknown"));
    assert!(Arc::ptr_eq(&missing, &db.analyze_project()));

    db.set_source("部品.astro", "<button />");
    let found = db.analyze_project();
    assert_eq!(found.revision().sources, missing.revision().sources + 1);
    assert_eq!(found.revision().resolver, missing.revision().resolver);
    assert!(Arc::ptr_eq(
        &original_analysis,
        &found.file("Page.astro").unwrap().analysis
    ));
    assert!(Arc::ptr_eq(
        &missing.file("Page.astro").unwrap().source,
        &found.file("Page.astro").unwrap().source
    ));
    let uses = found.references_to_file("部品.astro");
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].reference.span.start, offset(source, "{名前}") + 1);
    for byte in uses[0].reference.span.clone() {
        assert!(std::ptr::eq(
            found.reference_at("Page.astro", byte).unwrap().reference,
            uses[0].reference
        ));
    }
    assert!(
        found
            .reference_at("Page.astro", uses[0].reference.span.end)
            .is_none()
    );
    assert!(
        found
            .reference_at("Page.astro", source.len() as u32)
            .is_none()
    );
    assert!(db.remove_file("部品.astro"));
    assert!(db.analyze_file("部品.astro").is_none());
    let removed = db.analyze_project();
    assert!(removed.references_to_file("部品.astro").is_empty());
    assert_eq!(
        removed.imports_from("Page.astro")[0].resolution,
        &ModuleResolution::Unresolved
    );
    db.set_source("部品.astro", "<button>new</button>");
    let readded = db.analyze_project();
    assert_eq!(readded.references_to_file("部品.astro").len(), 1);
    db.set_source("Page.astro", "<p />");
    assert!(db.analyze_project().imports_to("部品.astro").is_empty());
    db.remove_file("Page.astro");
    assert!(db.analyze_project().reference_at("Page.astro", 0).is_none());
    drop(db);
    assert_eq!(found.file("Page.astro").unwrap().source.as_ref(), source);
    assert_eq!(found.references_to_file("部品.astro").len(), 1);
    assert_eq!(
        found.file("部品.astro").unwrap().source.as_ref(),
        "<button />"
    );
    assert_eq!(
        readded.file("部品.astro").unwrap().source.as_ref(),
        "<button>new</button>"
    );
    assert!(missing.references_to_file("部品.astro").is_empty());
}

struct AliasResolver {
    target: PathBuf,
    calls: Arc<AtomicUsize>,
}

impl ModuleResolver for AliasResolver {
    fn resolve(
        &self,
        request: ModuleRequest<'_>,
        sources: &ProjectSources<'_>,
    ) -> ModuleResolution {
        self.calls.fetch_add(1, Ordering::Relaxed);
        assert!(sources.contains(request.importer));
        assert!(sources.source(request.importer).unwrap().contains("import"));
        assert!(sources.paths().any(|path| path == request.importer));
        if request.import.source == "@button" {
            // Deliberately return a configured key even if it has not been registered.
            ModuleResolution::Resolved(self.target.clone())
        } else {
            RelativeResolver.resolve(request, sources)
        }
    }
}

#[test]
fn resolver_replacement_and_source_changes_refresh_resolution_without_reparsing_unchanged_files() {
    let mut db = AnalysisDatabase::new();
    db.set_source(
        "Page.astro",
        "---\nimport Button from '@button';\nimport './Other.astro';\n---\n<Button />",
    );
    db.set_source("Other.astro", "<div />");
    db.set_source("A.astro", "<button />");
    db.set_source("B.astro", "<button />");
    let first = db.analyze_project();
    assert_eq!(
        first.imports_from("Page.astro")[0].resolution,
        &ModuleResolution::Unsupported
    );
    let calls = Arc::new(AtomicUsize::new(0));
    db.set_module_resolver(AliasResolver {
        target: "A.astro".into(),
        calls: Arc::clone(&calls),
    });
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let a = db.analyze_project();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(a.references_to_file("A.astro").len(), 1);
    assert_eq!(a.imports_to("Other.astro").len(), 1);
    assert!(Arc::ptr_eq(&a, &db.analyze_project()));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(a.revision().sources, first.revision().sources);
    assert_eq!(a.revision().resolver, first.revision().resolver + 1);
    db.set_module_resolver(AliasResolver {
        target: "B.astro".into(),
        calls: Arc::clone(&calls),
    });
    let b = db.analyze_project();
    assert_eq!(calls.load(Ordering::Relaxed), 4);
    assert!(b.references_to_file("A.astro").is_empty());
    assert_eq!(b.references_to_file("B.astro").len(), 1);
    assert!(Arc::ptr_eq(
        &a.file("Page.astro").unwrap().analysis,
        &b.file("Page.astro").unwrap().analysis
    ));
    assert!(Arc::ptr_eq(
        &a.file("Page.astro").unwrap().source,
        &b.file("Page.astro").unwrap().source
    ));

    db.set_module_resolver(AliasResolver {
        target: "Missing.astro".into(),
        calls: Arc::clone(&calls),
    });
    assert_eq!(
        db.analyze_project().imports_from("Page.astro")[0].resolution,
        &ModuleResolution::Unresolved
    );
    db.set_source("Missing.astro", "<button />");
    let created = db.analyze_project();
    assert_eq!(created.references_to_file("Missing.astro").len(), 1);
    db.set_source("Missing.astro", "<button>updated</button>");
    let updated = db.analyze_project();
    assert_eq!(calls.load(Ordering::Relaxed), 10);
    assert!(Arc::ptr_eq(
        &created.file("Page.astro").unwrap().analysis,
        &updated.file("Page.astro").unwrap().analysis
    ));
    assert!(!Arc::ptr_eq(
        &created.file("Missing.astro").unwrap().analysis,
        &updated.file("Missing.astro").unwrap().analysis
    ));
    assert_eq!(a.references_to_file("A.astro").len(), 1);
}

#[test]
fn syntax_errors_are_visible_without_discarding_other_files_or_incoming_imports() {
    let mut db = AnalysisDatabase::new();
    db.set_source(
        "Page.astro",
        "---\nimport Broken from './Broken.astro';\n---\n<Broken />",
    );
    db.set_source("Broken.astro", "---\nconst =;\n---");
    let snapshot = db.analyze_project();
    assert!(snapshot.file("Broken.astro").unwrap().analysis.is_err());
    assert_eq!(snapshot.imports_to("Broken.astro").len(), 1);
    assert_eq!(snapshot.references_to_file("Broken.astro").len(), 1);
    assert!(snapshot.imports_from("Broken.astro").is_empty());
    assert!(snapshot.reference_at("Broken.astro", 5).is_none());
    db.set_source("Broken.astro", "<button />");
    let fixed = db.analyze_project();
    assert!(fixed.file("Broken.astro").unwrap().analysis.is_ok());
    assert!(snapshot.file("Broken.astro").unwrap().analysis.is_err());
    db.set_source(
        "Page.astro",
        "---\nimport Broken from './Broken.astro';\nconst =;\n---",
    );
    let error = db.analyze_project();
    assert!(error.file("Page.astro").unwrap().analysis.is_err());
    assert!(error.imports_to("Broken.astro").is_empty());
    assert!(error.references_to_file("Broken.astro").is_empty());
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ProjectSnapshot>();
}

#[test]
fn resolver_reads_of_other_source_buffers_are_invalidated_with_the_source_set() {
    struct SourceAwareResolver;
    impl ModuleResolver for SourceAwareResolver {
        fn resolve(&self, _: ModuleRequest<'_>, sources: &ProjectSources<'_>) -> ModuleResolution {
            if sources.source("Settings.astro") == Some("<p>B</p>") {
                resolved("B.astro")
            } else {
                resolved("A.astro")
            }
        }
    }
    let mut db = AnalysisDatabase::new();
    db.set_module_resolver(SourceAwareResolver);
    db.set_source(
        "Page.astro",
        "---\nimport Button from '@button';\n---\n<Button />",
    );
    db.set_source("A.astro", "<button />");
    db.set_source("B.astro", "<button />");
    let a = db.analyze_project();
    assert_eq!(a.references_to_file("A.astro").len(), 1);
    db.set_source("Settings.astro", "<p>B</p>");
    let b = db.analyze_project();
    assert_eq!(b.references_to_file("B.astro").len(), 1);
    assert!(b.references_to_file("A.astro").is_empty());
    db.set_source("Settings.astro", "<p>A</p>");
    assert_eq!(db.analyze_project().references_to_file("A.astro").len(), 1);
    db.set_source("Settings.astro", "<p>B</p>");
    assert_eq!(db.analyze_project().references_to_file("B.astro").len(), 1);
    db.remove_file("Settings.astro");
    let removed = db.analyze_project();
    assert_eq!(removed.references_to_file("A.astro").len(), 1);
    assert!(Arc::ptr_eq(
        &a.file("Page.astro").unwrap().analysis,
        &removed.file("Page.astro").unwrap().analysis
    ));
    assert_eq!(b.references_to_file("B.astro").len(), 1);
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
    let mut db = AnalysisDatabase::new();
    db.set_module_resolver(DomainResolver);
    db.set_source("Page.astro", "---\nimport '@setup';\n---\n<script>import '@setup';</script>\n<script is:inline type={\"module\"}>import '@setup';</script>");
    db.set_source("Server.astro", "");
    db.set_source("Browser.astro", "");
    let snapshot = db.analyze_project();
    assert_eq!(snapshot.imports_to("Server.astro").len(), 1);
    assert_eq!(snapshot.imports_to("Browser.astro").len(), 2);
    assert!(snapshot.references_to_file("Server.astro").is_empty());
    assert!(snapshot.references_to_file("Browser.astro").is_empty());
}

#[test]
fn self_imports_in_server_and_browser_domains_keep_distinct_bindings() {
    let source = "---\nimport Self from './Page.astro';\n---\n<Self />\n<script>import Self from './Page.astro'; Self;</script>\n<script is:inline type={\"module\"}>import Self from './Page.astro'; Self;</script>";
    let mut db = AnalysisDatabase::new();
    db.set_source("Page.astro", source);
    let snapshot = db.analyze_project();
    let imports = snapshot.imports_to("Page.astro");
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
