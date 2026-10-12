use std::ops::Range;

use atlas::{
    Binding, DiagnosticPhase, DirectiveValueKind, FileFacts, ImportSpecifier, Reference,
    Resolution, analyze,
};

fn text<'a>(source: &'a str, span: &Range<u32>) -> &'a str {
    &source[span.start as usize..span.end as usize]
}

fn binding(reference: &Reference) -> &Binding {
    match &reference.resolution {
        Resolution::Resolved(binding) => binding,
        Resolution::Unresolved => panic!("expected resolved reference at {:?}", reference.span),
    }
}

#[test]
fn counter_facts_outlive_the_source_and_keep_import_and_parameter_distinct() {
    let facts = {
        let source = include_str!("fixtures/counter.astro").to_owned();
        analyze(&source).unwrap()
    };
    let source = include_str!("fixtures/counter.astro");
    assert_eq!(facts.imports.len(), 1);
    assert_eq!(facts.imports[0].source, "./Counter.tsx");
    assert_eq!(
        facts.imports[0].bindings[0].target.specifier,
        ImportSpecifier::Default
    );
    assert_eq!(facts.components.len(), 2);
    let imported = binding(&facts.components[0].reference);
    let parameter = binding(&facts.components[1].reference);
    assert_eq!(imported.span, facts.imports[0].bindings[0].span);
    assert_eq!(imported.import.as_ref().unwrap().source, "./Counter.tsx");
    assert_eq!(text(source, &parameter.span), "Counter");
    assert!(parameter.import.is_none());
    assert_ne!(imported.span, parameter.span);
    assert_eq!(facts.components[0].directives.len(), 1);
    assert_eq!(facts.components[0].directives[0].name, "client:visible");
    assert!(facts.components[1].directives.is_empty());
    assert_eq!(facts.scripts.len(), 1);
    assert!(
        facts.scripts[0]
            .references
            .iter()
            .any(|reference| reference.name == "Counter"
                && reference.resolution == Resolution::Unresolved)
    );
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FileFacts>();
}

#[test]
fn shadowed_components_follow_declaration_identity() {
    let source = include_str!("fixtures/shadowing.astro");
    let facts = analyze(source).unwrap();
    let bindings: Vec<_> = facts
        .components
        .iter()
        .map(|component| binding(&component.reference))
        .collect();
    assert_eq!(bindings.len(), 6);
    assert_eq!(bindings[0].span, bindings[5].span);
    assert_eq!(bindings[2].span, bindings[4].span);
    assert_ne!(bindings[2].span, bindings[3].span);
    assert_ne!(bindings[0].span, bindings[2].span);
    assert_eq!(
        bindings[1].import.as_ref().unwrap().specifier,
        ImportSpecifier::Named("Counter".into())
    );
    assert_eq!(bindings[1].name, "NamedCounter");
    assert!(
        bindings[2..5]
            .iter()
            .all(|binding| binding.import.is_none())
    );
    assert!(
        facts
            .components
            .windows(2)
            .all(|components| components[0].span.start < components[1].span.start)
    );
}

#[test]
fn imports_include_aliases_types_namespace_and_side_effects() {
    let source = include_str!("fixtures/imports.astro");
    let facts = analyze(source).unwrap();
    assert_eq!(facts.imports.len(), 5);
    assert_eq!(facts.imports[0].source, "./setup");
    assert!(facts.imports[0].bindings.is_empty());
    let named = &facts.imports[2].bindings;
    assert_eq!(named.len(), 3);
    assert_eq!(named[0].name, "Renamed");
    assert_eq!(
        named[0].target.specifier,
        ImportSpecifier::Named("Counter".into())
    );
    assert!(!named[0].target.type_only);
    assert!(named[1].target.type_only);
    assert_eq!(
        named[2].target.specifier,
        ImportSpecifier::Named("Widget-name".into())
    );
    assert!(facts.imports[3].bindings[0].target.type_only);
    assert_eq!(
        facts.imports[4].bindings[0].target.specifier,
        ImportSpecifier::Namespace
    );
    assert_eq!(facts.components.len(), 5);
    let member = &facts.components[3];
    assert_eq!(member.name, "UI.Forms.Counter");
    assert_eq!(member.reference.name, "UI");
    assert_eq!(text(source, &member.reference.span), "UI");
    assert_eq!(
        binding(&member.reference)
            .import
            .as_ref()
            .unwrap()
            .specifier,
        ImportSpecifier::Namespace
    );
    assert_eq!(
        facts.components[4].reference.resolution,
        Resolution::Unresolved
    );
}

#[test]
fn same_named_frontmatter_and_script_imports_have_separate_targets() {
    let source = "---\nimport Counter from './server';\n---\n<Counter />\n<script>import Counter from './browser'; Counter;</script>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.imports.len(), 2);
    assert_eq!(
        binding(&facts.components[0].reference)
            .import
            .as_ref()
            .unwrap()
            .source,
        "./server"
    );
    let browser = facts.scripts[0]
        .references
        .iter()
        .find(|reference| reference.name == "Counter")
        .unwrap();
    assert_eq!(
        binding(browser).import.as_ref().unwrap().source,
        "./browser"
    );
    assert_ne!(
        binding(browser).span,
        binding(&facts.components[0].reference).span
    );
}

#[test]
fn scripts_keep_local_closures_and_isolate_template_and_other_scripts() {
    let source = include_str!("fixtures/scripts.astro");
    let facts = analyze(source).unwrap();
    assert_eq!(facts.scripts.len(), 2);
    let mut local_bindings = Vec::new();
    for script in &facts.scripts {
        let shared: Vec<_> = script
            .references
            .iter()
            .filter(|reference| reference.name == "shared")
            .collect();
        assert!(!shared.is_empty());
        let local = binding(shared[0]);
        assert!(script.span.start <= local.span.start && local.span.end <= script.span.end);
        assert!(
            shared
                .iter()
                .all(|reference| binding(reference).span == local.span)
        );
        local_bindings.push(local.span.clone());
        for reference in script.references.iter().filter(|reference| {
            matches!(
                reference.name.as_str(),
                "Counter" | "serverOnly" | "TemplateOnly"
            )
        }) {
            assert_eq!(reference.resolution, Resolution::Unresolved);
        }
    }
    assert_ne!(local_bindings[0], local_bindings[1]);
    assert!(
        facts.scripts[0]
            .references
            .iter()
            .any(|reference| reference.name == "secondOnly"
                && reference.resolution == Resolution::Unresolved)
    );
    assert!(
        facts.scripts[1]
            .references
            .iter()
            .any(|reference| reference.name == "firstOnly"
                && reference.resolution == Resolution::Unresolved)
    );
    for script in &facts.scripts {
        assert!(
            script
                .references
                .windows(2)
                .all(|references| references[0].span.start <= references[1].span.start)
        );
    }
}

#[test]
fn component_uses_preserve_paired_tags_and_duplicate_directives() {
    let source = include_str!("fixtures/usage.astro");
    let facts = analyze(source).unwrap();
    assert_eq!(facts.components.len(), 3);
    assert!(facts.components[0].closing_name_span.is_some());
    assert!(
        facts.components[1..]
            .iter()
            .all(|component| component.closing_name_span.is_none())
    );
    let directives = &facts.components[0].directives;
    assert_eq!(
        directives
            .iter()
            .map(|directive| directive.name.as_str())
            .collect::<Vec<_>>(),
        [
            "client:visible",
            "client:idle",
            "client:visible",
            "custom:flag"
        ]
    );
    assert_ne!(directives[0].span, directives[2].span);
    assert!(
        directives[..3]
            .iter()
            .all(|directive| directive.value.is_none())
    );
    let value = directives[3].value.as_ref().unwrap();
    assert_eq!(value.kind, DirectiveValueKind::String);
    assert_eq!(value.raw, "\"yes\"");
    assert_eq!(text(source, &value.span), value.raw);
    assert_eq!(
        facts.components[1].directives[0]
            .value
            .as_ref()
            .unwrap()
            .raw,
        "\"react\""
    );
}

#[test]
fn directives_keep_expression_source_and_skip_plain_and_spread_attributes() {
    let source = "<Missing title=\"plain\" {...props} custom:expr={value} custom:text=\"和文 😀\" custom:flag />";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.components.len(), 1);
    let directives = &facts.components[0].directives;
    assert_eq!(directives.len(), 3);
    assert_eq!(
        directives[0].value.as_ref().unwrap().kind,
        DirectiveValueKind::Expression
    );
    assert_eq!(directives[0].value.as_ref().unwrap().raw, "{value}");
    assert_eq!(directives[1].value.as_ref().unwrap().raw, "\"和文 😀\"");
    assert!(directives[2].value.is_none());
    for directive in directives {
        assert_eq!(text(source, &directive.name_span), directive.name);
    }
}

#[test]
fn crlf_and_unicode_spans_reconstruct_original_source() {
    let source = std::str::from_utf8(include_bytes!("fixtures/positions.astro")).unwrap();
    let facts = analyze(source).unwrap();
    assert_eq!(facts.components.len(), 1);
    let component = &facts.components[0];
    assert_eq!(text(source, &component.name_span), "Counter");
    assert_eq!(
        text(source, component.closing_name_span.as_ref().unwrap()),
        "Counter"
    );
    assert_eq!(
        text(source, &component.span),
        "<Counter title=\"和文 😀\" client:media=\"(幅: 1px)\">{caption}</Counter>"
    );
    assert_eq!(text(source, &binding(&component.reference).span), "Counter");
    let directive = &component.directives[0];
    assert_eq!(text(source, &directive.name_span), "client:media");
    assert_eq!(directive.value.as_ref().unwrap().raw, "\"(幅: 1px)\"");
    assert_eq!(
        text(source, &facts.imports[0].source_span),
        "'./Counter.astro'"
    );
}

#[test]
fn frontmatter_and_script_jsx_are_not_template_uses() {
    let source = "---\nimport Counter from './Counter';\nconst frontmatterView = <Counter />;\n---\n<!-- <Counter /> -->\n{\"<Counter />\"}\n<Counter />\n<script>const scriptView = <Counter />; scriptView;</script>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.components.len(), 1);
    assert_eq!(facts.scripts.len(), 1);
    assert!(
        facts.scripts[0]
            .references
            .iter()
            .any(|reference| reference.name == "Counter"
                && reference.resolution == Resolution::Unresolved)
    );
}

#[test]
fn syntax_diagnostics_are_owned_and_distinct_from_unresolved_references() {
    for source in [
        include_str!("fixtures/invalid-frontmatter.astro"),
        include_str!("fixtures/invalid-template.astro"),
        include_str!("fixtures/invalid-script.astro"),
    ] {
        let error = {
            let temporary_source = source.to_owned();
            analyze(&temporary_source).unwrap_err()
        };
        assert!(!error.diagnostics.is_empty());
        for diagnostic in error.diagnostics {
            assert_eq!(diagnostic.phase, DiagnosticPhase::Parser);
            assert!(!diagnostic.message.is_empty());
            assert!(!diagnostic.labels.is_empty());
            for label in diagnostic.labels {
                assert!(label.span.end as usize <= source.len());
                assert!(
                    source
                        .get(label.span.start as usize..label.span.end as usize)
                        .is_some()
                );
            }
        }
    }
    let facts = analyze(include_str!("fixtures/undefined.astro")).unwrap();
    assert_eq!(
        facts.components[0].reference.resolution,
        Resolution::Unresolved
    );
}

#[test]
fn semantic_syntax_diagnostics_produce_an_error_without_partial_facts() {
    let source = "---\nconst Counter = 1;\nconst Counter = 2;\n---\n<Counter />";
    let error = analyze(source).unwrap_err();
    assert!(!error.parser_panicked);
    assert!(!error.diagnostics.is_empty());
    assert!(
        error
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.phase == DiagnosticPhase::Semantic)
    );
}

#[test]
fn empty_no_frontmatter_and_raw_scripts_return_empty_component_facts() {
    for source in [
        include_str!("fixtures/empty.astro"),
        include_str!("fixtures/no-frontmatter.astro"),
        include_str!("fixtures/raw-script.astro"),
    ] {
        let facts = analyze(source).unwrap();
        assert_eq!(facts, FileFacts::default());
    }
}

#[test]
fn inline_scripts_are_parsed_and_define_vars_keeps_the_pinned_limitation() {
    let inline = analyze(include_str!("fixtures/inline.astro")).unwrap();
    assert_eq!(inline.scripts.len(), 1);
    assert_eq!(inline.scripts[0].references.len(), 1);
    assert_eq!(binding(&inline.scripts[0].references[0]).name, "inlineOnly");
    let define_vars = analyze(include_str!("fixtures/define-vars.astro")).unwrap();
    assert_eq!(define_vars.scripts.len(), 1);
    assert!(define_vars.scripts[0].references.iter().any(
        |reference| reference.name == "title" && reference.resolution == Resolution::Unresolved
    ));
}
