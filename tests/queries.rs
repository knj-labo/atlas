use atlas::{
    Builtin, DeclarationId, DeclarationKind, FileFacts, ImportSpecifier, Reference, ReferenceKind,
    Resolution, ScopeDomain, analyze,
};

fn offset(source: &str, text: &str) -> u32 {
    source.find(text).unwrap().try_into().unwrap()
}

fn starts(references: &[&Reference]) -> Vec<u32> {
    references
        .iter()
        .map(|reference| reference.span.start)
        .collect()
}

#[test]
fn exact_names_and_shadowed_uses_preserve_identity_and_borrow_records() {
    let source = "---\nimport { Widget as View } from './ui';\nconst views = [View];\nfunction render(View) { return View; }\n---\n<View />\n{views.map((View) => <View />)}\n<View />";
    let facts = analyze(source).unwrap();
    let before = facts.clone();
    let declarations = facts.declarations_named("View");
    assert_eq!(declarations.len(), 3);
    assert_eq!(declarations[0].kind, DeclarationKind::Import);
    assert_eq!(
        declarations[0].import.as_ref().unwrap().specifier,
        ImportSpecifier::Named("Widget".into())
    );
    assert_eq!(declarations[1].kind, DeclarationKind::Parameter);
    assert_eq!(declarations[2].kind, DeclarationKind::Parameter);
    for pair in declarations.windows(2) {
        assert_ne!(pair[0].id, pair[1].id);
        assert_ne!(pair[0].scope, pair[1].scope);
    }
    for name in ["Widget", "view", "Vie", "View ", "missing", ""] {
        assert!(facts.declarations_named(name).is_empty(), "{name:?}");
    }

    let expected = [
        vec![
            offset(source, "View]"),
            offset(source, "<View") + 1,
            source.rfind("<View").unwrap() as u32 + 1,
        ],
        vec![offset(source, "View;")],
        vec![offset(source, "<View />)}") + 1],
    ];
    for (declaration, expected) in declarations.iter().zip(expected) {
        assert!(std::ptr::eq(
            *declaration,
            &facts.declarations[declaration.id.0]
        ));
        let uses = facts.references_to(declaration.id);
        assert_eq!(starts(&uses), expected);
        for reference in uses {
            assert!(std::ptr::eq(reference, &facts.references[reference.id.0]));
            assert!(std::ptr::eq(
                facts.reference_at(reference.span.start).unwrap(),
                reference
            ));
        }
    }
    let unused = facts.declarations_named("render")[0];
    assert!(facts.references_to(unused.id).is_empty());
    assert_eq!(facts, before);
}

#[test]
fn source_order_interleaves_template_bindings_and_isolated_browser_scripts() {
    let source = "---\nconst Item = 0;\n---\n{Item}\n<script>const Item = 1; Item;</script>\n{[0].map((Item) => Item)}\n<script is:inline>const Item = 2; Item;</script>\n<script is:inline type={\"module\"}>const Item = 3; Item;</script>\n{Item}";
    let facts = analyze(source).unwrap();
    let positions: Vec<_> = source
        .match_indices("Item")
        .map(|(index, _)| index as u32)
        .collect();
    let declarations = facts.declarations_named("Item");
    assert_eq!(
        declarations
            .iter()
            .map(|binding| binding.span.start)
            .collect::<Vec<_>>(),
        [0, 2, 4, 6, 8].map(|index| positions[index])
    );
    assert_eq!(
        declarations
            .iter()
            .map(|binding| facts.scopes[binding.scope.0].domain)
            .collect::<Vec<_>>(),
        [
            ScopeDomain::Server,
            ScopeDomain::BrowserModule,
            ScopeDomain::Server,
            ScopeDomain::BrowserClassic,
            ScopeDomain::BrowserModule
        ]
    );
    for (declaration, expected) in declarations.iter().zip([
        vec![positions[1], positions[10]],
        vec![positions[3]],
        vec![positions[5]],
        vec![positions[7]],
        vec![positions[9]],
    ]) {
        assert_eq!(starts(&facts.references_to(declaration.id)), expected);
    }
}

#[test]
fn uncertain_references_keep_their_resolution_and_are_not_definite_uses() {
    let source = "---\nconst shared = 0;\n---\n{shared} {Astro.url} <Fragment /> <Missing />\n<script is:inline>const shared = 1; shared;</script>\n<script is:inline>shared; missing; this;</script>\n<script>shared;</script>";
    let facts = analyze(source).unwrap();
    let declarations = facts.declarations_named("shared");
    assert_eq!(declarations.len(), 2);
    assert_eq!(
        starts(&facts.references_to(declarations[0].id)),
        [offset(source, "{shared}") + 1]
    );
    assert_eq!(
        starts(&facts.references_to(declarations[1].id)),
        [offset(source, "shared;</script>")]
    );

    let builtin = facts.reference_at(offset(source, "Astro.url")).unwrap();
    assert_eq!(builtin.resolution, Resolution::Builtin(Builtin::Astro));
    assert_eq!(
        facts
            .reference_at(offset(source, "Fragment"))
            .unwrap()
            .resolution,
        Resolution::Builtin(Builtin::Fragment)
    );
    for name in ["Missing", "missing"] {
        assert_eq!(
            facts.reference_at(offset(source, name)).unwrap().resolution,
            Resolution::Unresolved
        );
    }
    let runtime = facts
        .reference_at(offset(source, "shared; missing"))
        .unwrap();
    let Resolution::RuntimeDependent { candidates, .. } = &runtime.resolution else {
        panic!("expected a possible classic-script global: {runtime:?}");
    };
    assert_eq!(candidates, &[declarations[1].id]);
    assert!(matches!(
        facts
            .reference_at(offset(source, "this;"))
            .unwrap()
            .resolution,
        Resolution::RuntimeDependent { .. }
    ));
    assert_eq!(
        facts
            .reference_at(offset(source, "<script>shared") + 8)
            .unwrap()
            .resolution,
        Resolution::Unresolved
    );
}

#[test]
fn injected_bindings_and_type_references_use_the_same_queries() {
    let source = "---\nimport type { Props as Input } from './types';\nconst title: Input = Astro.props;\n---\n{title}\n<script define:vars={{ title }}>title; ((title) => title)(title);</script>";
    let facts = analyze(source).unwrap();
    let input = facts.declarations_named("Input")[0];
    assert!(input.import.as_ref().unwrap().type_only);
    let types = facts.references_to(input.id);
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].kind, ReferenceKind::Type);
    assert_eq!(types[0].span.start, offset(source, "Input ="));

    let declarations = facts.declarations_named("title");
    assert_eq!(declarations.len(), 3);
    let injected = declarations[1];
    assert_eq!(injected.kind, DeclarationKind::Injected);
    assert_eq!(
        facts.scopes[injected.scope.0].domain,
        ScopeDomain::InjectedScript
    );
    assert_eq!(
        starts(&facts.references_to(declarations[0].id)),
        [offset(source, "{title}") + 1, offset(source, "title }}")]
    );
    assert_eq!(
        starts(&facts.references_to(injected.id)),
        [offset(source, "title;"), offset(source, "(title);") + 1]
    );
    assert_eq!(
        starts(&facts.references_to(declarations[2].id)),
        [offset(source, "=> title") + 3]
    );
    // A shorthand injection key is both an injected declaration and a server reference.
    let origin = facts.reference_at(injected.span.start).unwrap();
    assert!(
        matches!(&origin.resolution, Resolution::Resolved(binding) if binding.id == declarations[0].id)
    );
}

#[test]
fn byte_positions_are_half_open_in_unicode_and_crlf_source() {
    let source = "---\r\nconst 名前 = '😀';\r\n---\r\n<p>{名前}</p>\r\n<script>const 名前 = 1; 名前;</script>\r\n";
    let facts = analyze(source).unwrap();
    let declarations = facts.declarations_named("名前");
    assert_eq!(declarations.len(), 2);
    let expected = [
        offset(source, "{名前}") + 1,
        offset(source, "名前;</script>"),
    ];
    for (declaration, start) in declarations.iter().zip(expected) {
        let uses = facts.references_to(declaration.id);
        assert_eq!(uses.len(), 1);
        let reference = uses[0];
        assert_eq!(reference.span, start..start + "名前".len() as u32);
        for byte in reference.span.clone() {
            assert!(std::ptr::eq(facts.reference_at(byte).unwrap(), reference));
        }
        assert!(facts.reference_at(start - 1).is_none());
        assert!(facts.reference_at(reference.span.end).is_none());
        assert!(facts.reference_at(declaration.span.start).is_none());
    }
    for byte in [0, source.len() as u32, source.len() as u32 + 1, u32::MAX] {
        assert!(facts.reference_at(byte).is_none());
    }
}

#[test]
fn only_recorded_reference_spans_match_and_unknown_ids_are_empty() {
    let source = "---\nimport * as UI from './ui';\n---\n<UI.Card title=\"UI\">UI</UI.Card>\n<!-- UI -->\n<script type=\"application/json\">\"UI\"</script>\n<style>.UI { --UI: red; color: var(--UI); }</style>";
    let facts = analyze(source).unwrap();
    let ui = facts.declarations_named("UI")[0];
    assert_eq!(
        starts(&facts.references_to(ui.id)),
        [offset(source, "<UI") + 1, offset(source, "</UI") + 2]
    );
    assert!(facts.reference_at(offset(source, "<UI") + 1).is_some());
    assert!(facts.reference_at(offset(source, "</UI") + 2).is_some());
    for needle in [
        "Card",
        "title",
        "\"UI\"",
        "UI</",
        "Card>",
        "UI -->",
        "--UI",
        "var(--UI)",
    ] {
        assert!(
            facts.reference_at(offset(source, needle)).is_none(),
            "{needle}"
        );
    }
    let css = facts.styles[0].css.as_ref().unwrap();
    assert_eq!(css.references.len(), 1);
    assert!(!css.references[0].candidates.is_empty());
    assert!(facts.reference_at(css.references[0].span.start).is_none());
    assert!(
        facts
            .reference_at(offset(source, ">\"UI\"</script>") + 2)
            .is_none()
    );
    assert!(facts.declarations_named("--UI").is_empty());
    assert!(
        facts
            .references_to(DeclarationId(facts.declarations.len()))
            .is_empty()
    );
    assert!(facts.references_to(DeclarationId(usize::MAX)).is_empty());
    for empty in [FileFacts::default(), analyze("").unwrap()] {
        assert!(empty.declarations_named("UI").is_empty());
        assert!(empty.reference_at(0).is_none());
        assert!(empty.references_to(DeclarationId(0)).is_empty());
    }
}
