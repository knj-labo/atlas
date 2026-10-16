#![cfg(feature = "incremental")]

use atlas::{
    AnalysisDatabase, ModuleRequest, ModuleResolution, ModuleResolver, ProjectSnapshot,
    ProjectSources, RelativeResolver, analyze_project_with_resolver,
};
use std::{
    path::PathBuf,
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
        missing.outgoing_imports("Page.astro")[0].resolution,
        &ModuleResolution::Unresolved
    );
    db.set_source("Page.astro", source);
    assert!(!db.remove_file("unknown"));
    assert!(Arc::ptr_eq(&missing, &db.analyze_project()));

    db.set_source("部品.astro", "<button />");
    let found = db.analyze_project();
    assert_eq!(
        found.revision().unwrap().sources,
        missing.revision().unwrap().sources + 1
    );
    assert_eq!(
        found.revision().unwrap().resolver,
        missing.revision().unwrap().resolver
    );
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
        removed.outgoing_imports("Page.astro")[0].resolution,
        &ModuleResolution::Unresolved
    );
    db.set_source("部品.astro", "<button>new</button>");
    let readded = db.analyze_project();
    assert_eq!(readded.references_to_file("部品.astro").len(), 1);
    db.set_source("Page.astro", "<p />");
    assert!(
        db.analyze_project()
            .incoming_imports("部品.astro")
            .is_empty()
    );
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
        first.outgoing_imports("Page.astro")[0].resolution,
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
    assert_eq!(a.incoming_imports("Other.astro").len(), 1);
    assert!(Arc::ptr_eq(&a, &db.analyze_project()));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        a.revision().unwrap().sources,
        first.revision().unwrap().sources
    );
    assert_eq!(
        a.revision().unwrap().resolver,
        first.revision().unwrap().resolver + 1
    );
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
        db.analyze_project().outgoing_imports("Page.astro")[0].resolution,
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

fn assert_import_matches(left: atlas::ProjectImport<'_>, right: atlas::ProjectImport<'_>) {
    assert_eq!(left.path, right.path);
    assert_eq!(left.import, right.import);
    assert_eq!(left.domain, right.domain);
    assert_eq!(left.resolution, right.resolution);
}

fn assert_reference_matches(left: atlas::ProjectReference<'_>, right: atlas::ProjectReference<'_>) {
    assert_eq!(left.path, right.path);
    assert_eq!(left.reference, right.reference);
    match (left.import, right.import) {
        (Some(left), Some(right)) => assert_import_matches(left, right),
        (None, None) => {}
        imports => panic!("different originating imports: {imports:?}"),
    }
}

fn assert_queries_match(left: &ProjectSnapshot, right: &ProjectSnapshot) {
    assert_eq!(
        left.files().map(|(path, _)| path).collect::<Vec<_>>(),
        right.files().map(|(path, _)| path).collect::<Vec<_>>(),
    );
    for (path, file) in left.files() {
        let other = right.file(path).unwrap();
        assert_eq!(file.source, other.source);
        assert_eq!(file.analysis, other.analysis);
        for (left, right) in [
            (left.outgoing_imports(path), right.outgoing_imports(path)),
            (left.incoming_imports(path), right.incoming_imports(path)),
        ] {
            assert_eq!(left.len(), right.len());
            for (left, right) in left.into_iter().zip(right) {
                assert_import_matches(left, right);
            }
        }
        let left_uses = left.references_to_file(path);
        let right_uses = right.references_to_file(path);
        assert_eq!(left_uses.len(), right_uses.len());
        for (left, right) in left_uses.into_iter().zip(right_uses) {
            assert_reference_matches(left, right);
        }
        // Compare all reference kinds, including those excluded from file-use indexes.
        for byte in 0..=file.source.len() as u32 {
            match (
                left.reference_at(path, byte),
                right.reference_at(path, byte),
            ) {
                (Some(left), Some(right)) => assert_reference_matches(left, right),
                (None, None) => {}
                references => panic!("different references at {path:?}:{byte}: {references:?}"),
            }
        }
    }
}

#[test]
fn uncached_and_cached_project_queries_match() {
    let source = "---\r\nimport 名前 from '@button';\r\nimport { type Props as P } from './Button.astro';\r\nimport * as UI from './Button.astro';\r\nimport './Broken.astro';\r\nimport './Missing.astro';\r\nconst props: P = {};\r\n---\r\n<p>😀</p><名前 /><UI.Button />{[1].map((名前) => 名前)}{Astro.url}<Unknown />\r\n<script>import 名前 from './Button.astro'; 名前;</script>\r\n<script is:inline>const shared = 1;</script><script is:inline>shared;</script>";
    let sources = [
        ("Page.astro", source),
        ("Button.astro", "<button />"),
        ("Broken.astro", "---\nconst =;\n---"),
    ];
    for target in ["Button.astro", "Missing.astro"] {
        let mut db = AnalysisDatabase::new();
        for (path, source) in sources {
            db.set_source(path, source);
        }
        let uncached = atlas::analyze_project(sources);
        let cached = db.analyze_project();
        assert_eq!(uncached.revision(), None);
        assert!(cached.revision().is_some());
        assert_queries_match(&uncached, &cached);

        db.set_module_resolver(AliasResolver {
            target: target.into(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let resolver = AliasResolver {
            target: target.into(),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let uncached = analyze_project_with_resolver(sources, &resolver);
        let cached = db.analyze_project();
        assert_eq!(uncached.revision(), None);
        assert!(cached.revision().is_some());
        assert_queries_match(&uncached, &cached);
        assert!(Arc::ptr_eq(&cached, &db.analyze_project()));

        db.set_source("Missing.astro", "<div />");
        let updated = analyze_project_with_resolver(
            sources.into_iter().chain([("Missing.astro", "<div />")]),
            &resolver,
        );
        assert_queries_match(&updated, &db.analyze_project());
    }
}
