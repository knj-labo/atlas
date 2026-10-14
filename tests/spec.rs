use atlas::*;

fn binding(reference: &Reference) -> &Binding {
    let Resolution::Resolved(binding) = &reference.resolution else {
        panic!("expected resolved {:?}", reference)
    };
    binding
}
fn text<'a>(source: &'a str, span: &std::ops::Range<u32>) -> &'a str {
    source
        .get(span.start as usize..span.end as usize)
        .expect("original UTF-8 span")
}
fn codes(facts: &FileFacts) -> Vec<&str> {
    facts
        .diagnostics
        .iter()
        .filter_map(|d| d.code.as_deref())
        .collect()
}

#[test]
fn all_references_scopes_types_and_declarations_are_owned() {
    let source = "---\ninterface Props { count: number }; function identity<T>(value:T):T {return value}; const {count}=Astro.props; const result=identity(count);\n---\n<p data-count={result}>{count}</p>";
    let facts = {
        let source = source.to_owned();
        analyze(&source).unwrap()
    };
    assert!(
        facts
            .declarations
            .iter()
            .any(|d| d.name == "Props" && d.kind == DeclarationKind::Type)
    );
    assert!(
        facts
            .references
            .iter()
            .any(|r| r.name == "T" && r.kind == ReferenceKind::Type)
    );
    assert!(
        facts
            .references
            .iter()
            .any(|r| r.name == "count" && r.span.start > source.find("---\n<p").unwrap() as u32)
    );
    assert!(
        facts
            .builtin_uses
            .iter()
            .any(|u| u.member.as_deref() == Some("props"))
    );
    for reference in &facts.references {
        assert_eq!(facts.references[reference.id.0].span, reference.span);
        assert_eq!(facts.scopes[reference.scope.0].domain, ScopeDomain::Server);
        if let Resolution::Resolved(b) = &reference.resolution {
            assert_eq!(facts.declarations[b.id.0], *b);
        }
        assert_eq!(text(source, &reference.span), reference.name);
    }
    for scope in &facts.scopes {
        if let Some(parent) = scope.parent {
            assert!(parent.0 < facts.scopes.len());
        }
    }
}

#[test]
fn every_template_element_attributes_spreads_fragments_and_slots_are_kept() {
    let source = "---\nimport C from './C.astro'; const tag='section'; const items=[];\n---\n<><div class:list={items} {...attrs}><C><Fragment slot=\"header\">title</Fragment><slot name=\"footer\">fallback</slot></C></div><custom-element xml:lang=\"ja\" /></>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.elements.len(), 6);
    assert_eq!(facts.elements[0].kind, ElementKind::Fragment);
    assert_eq!(facts.elements[1].parent, Some(facts.elements[0].id));
    assert!(facts.elements[1].attributes.iter().any(|a| a.spread));
    assert_eq!(facts.elements[1].directives[0].name, "class:list");
    assert_eq!(facts.elements[3].kind, ElementKind::Fragment);
    assert_eq!(
        facts.elements[3].slot.as_ref().unwrap().name.as_deref(),
        Some("header")
    );
    assert_eq!(facts.elements[4].kind, ElementKind::Slot);
    assert_eq!(facts.elements[5].kind, ElementKind::CustomElement);
    assert!(facts.elements[4].text.iter().any(|t| t.raw == "fallback"));
}

#[test]
fn astro_self_and_fragment_are_builtins_but_local_fragment_can_shadow() {
    let facts = analyze("---\nconst yes=true;\n---\n<Fragment />{yes && <Astro.self />}").unwrap();
    assert_eq!(facts.elements[0].kind, ElementKind::Fragment);
    assert_eq!(facts.elements[1].kind, ElementKind::SelfComponent);
    assert!(
        facts
            .builtin_uses
            .iter()
            .any(|u| u.builtin == Builtin::AstroSelf)
    );
    let shadow = analyze("---\nconst Fragment=()=>null;\n---\n<Fragment />").unwrap();
    assert_eq!(shadow.elements[0].kind, ElementKind::Dynamic);
    assert!(matches!(
        shadow.components[0].reference.resolution,
        Resolution::Resolved(_)
    ));
}

#[test]
fn raw_children_comments_and_strings_never_create_uses_or_references() {
    let source = "<!-- <Fake /> -->{\"<Fake />\"}<div is:raw>{notCode}<Fake /></div><p>{real}</p>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.elements.len(), 2);
    assert!(facts.components.is_empty());
    assert!(facts.references.iter().all(|r| r.name == "real"));
    assert_eq!(facts.elements[0].text[0].raw, "{notCode}<Fake />");
}

#[test]
fn processed_inline_module_classic_external_data_and_dynamic_modes() {
    let sources = [
        (
            "<script>let x:number=1;</script>",
            ScriptMode::Processed,
            ScriptLanguage::TypeScript,
        ),
        (
            "<script type=\"module\">let x:number=1;</script>",
            ScriptMode::Processed,
            ScriptLanguage::TypeScript,
        ),
        (
            "<script is:inline>let x=1;</script>",
            ScriptMode::InlineClassic,
            ScriptLanguage::JavaScript,
        ),
        (
            "<script is:inline type=\"module\">export const x=1;</script>",
            ScriptMode::InlineModule,
            ScriptLanguage::JavaScript,
        ),
        (
            "<script defer>let x=1;</script>",
            ScriptMode::InlineClassic,
            ScriptLanguage::JavaScript,
        ),
        (
            "<script src=\"./a.ts\"></script>",
            ScriptMode::External,
            ScriptLanguage::TypeScript,
        ),
        (
            "<script src=\"./a.js\" type=\"module\"></script>",
            ScriptMode::External,
            ScriptLanguage::JavaScript,
        ),
        (
            "<script type=\"application/json\">{not valid js}</script>",
            ScriptMode::Data,
            ScriptLanguage::Raw,
        ),
        (
            "<script type={kind}>not JavaScript @</script>",
            ScriptMode::Dynamic,
            ScriptLanguage::Unknown,
        ),
    ];
    for (source, mode, language) in sources {
        let facts = analyze(source).unwrap();
        assert_eq!(facts.scripts.len(), 1, "{source}");
        assert_eq!(facts.scripts[0].mode, mode, "{source}");
        assert_eq!(facts.scripts[0].language, language, "{source}");
    }
    for source in [
        "<script is:inline>let x:number=1;</script>",
        "<script is:inline type=\"module\">let x:number=1;</script>",
        "<script is:inline>import X from './x';</script>",
    ] {
        let error = analyze(source).unwrap_err();
        assert!(
            error
                .diagnostics
                .iter()
                .all(|d| matches!(d.phase, DiagnosticPhase::Parser | DiagnosticPhase::Semantic))
        );
    }
}

#[test]
fn scripts_in_non_hoistable_context_are_browser_javascript() {
    for parent in ["svg", "noscript", "template"] {
        let source =
            format!("<{parent}><script>let x=1;</script><style>.a{{color:red}}</style></{parent}>");
        let facts = analyze(&source).unwrap();
        assert_eq!(facts.scripts[0].mode, ScriptMode::InlineClassic);
        assert_eq!(facts.styles[0].mode, StyleMode::Inline);
    }
}

#[test]
fn inline_classic_globals_are_candidates_not_definite_lexical_bindings() {
    let facts =
        analyze("<script is:inline>let shared=1;</script><script is:inline>shared;</script>")
            .unwrap();
    let reference = &facts.scripts[1].references[0];
    let Resolution::RuntimeDependent { candidates, .. } = &reference.resolution else {
        panic!()
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(facts.declarations[candidates[0].0].name, "shared");
}

#[test]
fn define_vars_aliases_keys_and_nested_shadowing_have_distinct_identity() {
    let source =
        "---\nconst title='hello';\n---\n<script define:vars={{message:title, 'bg-color':title}}>
message; bgColor; function local(message){return message;} local(1);
</script>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.scripts[0].injected.len(), 2);
    let title = facts
        .declarations
        .iter()
        .find(|d| d.name == "title")
        .unwrap();
    let message = facts.scripts[0]
        .references
        .iter()
        .find(|r| r.name == "message")
        .unwrap();
    let injected = binding(message);
    assert_eq!(injected.kind, DeclarationKind::Injected);
    assert_ne!(injected.id, title.id);
    let origin = injected.injection.as_ref().unwrap();
    assert_eq!(text(source, &origin.key_span), "message");
    assert_eq!(origin.value.raw, "title");
    assert!(
        origin
            .references
            .iter()
            .any(|id| binding(&facts.references[id.0]).id == title.id)
    );
    let local = facts.scripts[0]
        .references
        .iter()
        .filter(|r| r.name == "message")
        .next_back()
        .unwrap();
    assert_ne!(binding(local).id, injected.id);
    assert!(binding(local).injection.is_none());
}

#[test]
fn define_vars_static_collision_is_an_error_and_dynamic_keys_remain_unknown() {
    assert!(analyze("<script define:vars={{x:1}}>const x=2;</script>").is_err());
    let facts=analyze("---\nconst vars={}; const key='x';\n---\n<script define:vars={{...vars,[key]:1}}>x;</script>").unwrap();
    assert!(facts.scripts[0].dynamic_injection);
    assert!(matches!(
        facts.scripts[0].references[0].resolution,
        Resolution::RuntimeDependent { .. }
    ));
    let invalid = analyze("<script define:vars={{'123':1}}>ok;</script>").unwrap();
    assert!(codes(&invalid).contains(&"astro/invalid-injected-name"));
}

#[test]
fn script_discovery_covers_array_object_and_iife_return_expressions() {
    for source in [
        "{[<script>const local=1; local;</script>]}",
        "{({view: <script>const local=1; local;</script>}).view}",
        "{(()=>{return <script>const local=1; local;</script>})()}",
    ] {
        let facts = analyze(source).unwrap();
        assert_eq!(facts.scripts.len(), 1, "{source}");
        assert_eq!(binding(&facts.scripts[0].references[0]).name, "local");
    }
}

#[test]
fn static_directive_violations_are_diagnostics_and_dynamic_values_are_kept() {
    let source = "<div client:visible is:global define:vars={{x:1}} /><C client:media={42} custom:flag /><C client:media={query} /><C client:custom />";
    let facts = analyze(source).unwrap();
    let diagnostics = codes(&facts);
    for expected in [
        "astro/client-directive-target",
        "astro/style-directive-target",
        "astro/define-vars-target",
        "astro/client-media-value",
    ] {
        assert!(diagnostics.contains(&expected), "{expected}");
    }
    assert!(
        facts.elements[2]
            .directives
            .iter()
            .any(|d| d.status == ValidationStatus::RequiresProjectContext
                || d.status == ValidationStatus::Dynamic)
    );
    assert_eq!(
        facts.elements[1].directives[1].status,
        ValidationStatus::UnknownDirective
    );
    assert_eq!(
        facts.elements[3].directives[0].status,
        ValidationStatus::RequiresProjectContext
    );
}

#[test]
fn dynamic_tags_cannot_be_used_for_client_hydration() {
    let facts = analyze("---\nconst Element='div';\n---\n<Element client:load />").unwrap();
    assert_eq!(facts.elements[0].kind, ElementKind::Dynamic);
    assert!(codes(&facts).contains(&"astro/dynamic-hydration"));
}

#[test]
fn astro_members_are_recorded_and_unknown_members_are_diagnosed() {
    let facts = analyze(
        "---\nconst p=Astro.props; const x=Astro.unknown;\n---\n{Astro.slots.has('header')}",
    )
    .unwrap();
    assert!(
        facts
            .builtin_uses
            .iter()
            .any(|u| u.member.as_deref() == Some("slots.has"))
    );
    assert!(codes(&facts).contains(&"astro/unknown-api-member"));
    assert!(
        facts
            .references
            .iter()
            .filter(|r| r.name == "Astro")
            .all(|r| r.resolution == Resolution::Builtin(Builtin::Astro))
    );
}

#[test]
fn style_modes_preprocessors_and_dynamic_style_values_are_explicit() {
    let facts=analyze("<style>.a{color:red}</style><style is:global>.b{color:blue}</style><style is:inline>.c{color:red}</style><style lang=\"scss\">$x:red;</style><div style={css} />").unwrap();
    assert_eq!(
        facts.styles.iter().map(|s| s.mode).collect::<Vec<_>>(),
        [
            StyleMode::Scoped,
            StyleMode::Global,
            StyleMode::Inline,
            StyleMode::Scoped,
            StyleMode::Dynamic
        ]
    );
    assert_eq!(facts.styles[3].language, "scss");
    assert!(facts.styles[3].css.is_none());
    assert!(facts.styles[4].css.is_none());
}

#[test]
fn css_rules_selectors_duplicate_declarations_important_conditions_and_nesting() {
    let source = "<style>.a, :global(.b){--tone:red;color:blue!important;color:red;@media (width > 1px){&:hover{color:var(--tone)}}}</style>";
    let facts = analyze(source).unwrap();
    let css = facts.styles[0].css.as_ref().unwrap();
    assert_eq!(css.rules.len(), 3);
    assert_eq!(css.rules[0].selectors.len(), 2);
    assert_eq!(css.rules[0].selectors[0].classes[0].name, "a");
    assert!(css.rules[0].selectors[1].global);
    assert_eq!(
        css.declarations
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["--tone", "color", "color", "color"]
    );
    assert!(css.declarations[1].important);
    assert!(!css.declarations[2].important);
    assert_eq!(css.declarations[1].value.raw, "blue");
    assert_eq!(css.declarations[2].value.raw, "red");
    assert!(css.rules[2].selectors[0].nesting);
    assert_eq!(css.rules[2].conditions.len(), 1);
    assert_eq!(css.references[0].name, "--tone");
    assert_eq!(css.references[0].candidates.len(), 1);
    assert!(css.references[0].runtime_dependent);
    for declaration in &css.declarations {
        assert_eq!(text(source, &declaration.name_span), declaration.name);
        assert_eq!(text(source, &declaration.value.span), declaration.value.raw);
    }
}

#[test]
fn css_custom_properties_registrations_keyframes_urls_fallbacks_and_injection() {
    let source = "---\nconst tone='red';\n---\n<style define:vars={{accent:tone}}>@import url('./base.css'); @property --tone{syntax:'<color>';inherits:false;initial-value:red} .a{--tone:blue;color:var(--tone,var(--accent,red));background:url('./bg.png');animation:spin 1s linear}@keyframes spin{from{opacity:0}to{opacity:1}}</style>";
    let facts = analyze(source).unwrap();
    let style = &facts.styles[0];
    let css = style.css.as_ref().unwrap();
    assert_eq!(style.injected[0].name, "--accent");
    assert_eq!(style.injected[0].origin.value.raw, "tone");
    assert!(
        css.definitions
            .iter()
            .any(|d| d.kind == CssDefinitionKind::RegisteredProperty)
    );
    assert!(
        css.definitions
            .iter()
            .any(|d| d.kind == CssDefinitionKind::Keyframes)
    );
    assert!(
        css.dependencies
            .iter()
            .any(|d| d.kind == CssDependencyKind::Import && d.value == "./base.css")
    );
    assert!(
        css.dependencies
            .iter()
            .any(|d| d.kind == CssDependencyKind::Url && d.value == "./bg.png")
    );
    let reference = css.references.iter().find(|r| r.name == "--tone").unwrap();
    assert_eq!(reference.candidates.len(), 2);
    assert_eq!(
        reference.fallback.as_ref().unwrap().raw,
        "var(--accent,red)"
    );
    assert!(
        css.references
            .iter()
            .any(|r| r.name == "--accent" && r.candidates.len() == 1)
    );
    assert!(
        css.references
            .iter()
            .any(|r| r.kind == CssReferenceKind::Animation
                && r.name == "spin"
                && r.candidates.len() == 1)
    );
}

#[test]
fn css_variable_uses_in_comments_and_strings_are_not_references() {
    let facts=analyze("<style>.a{/*var(--fake)*/content:'var(--fake)';color:var(--real,);--v:var(--real)}</style>").unwrap();
    let css = facts.styles[0].css.as_ref().unwrap();
    assert!(css.references.iter().all(|r| r.name == "--real"));
    assert_eq!(css.references.len(), 2);
    assert_eq!(css.references[0].fallback.as_ref().unwrap().raw, "");
}

#[test]
fn css_html_entities_javascript_escapes_and_unicode_keep_original_spans() {
    let source = concat!(
        "<p>日本語 😀</p>\r\n",
        r#"<div style="--tone:red;content:&quot;😀&amp;和文&quot;;color:var(--tone)"/><div style={'--tone:blue;content:\"😀\";color:var(--tone)'} />"#
    );
    let facts = analyze(source).unwrap();
    assert_eq!(facts.styles.len(), 2);
    for style in &facts.styles {
        let css = style.css.as_ref().unwrap();
        let variable = css.references.iter().find(|r| r.name == "--tone").unwrap();
        assert_eq!(text(source, &variable.span), "--tone");
        for declaration in &css.declarations {
            assert_eq!(text(source, &declaration.value.span), declaration.value.raw);
        }
    }
    assert_eq!(
        facts.styles[0].css.as_ref().unwrap().declarations[1]
            .value
            .raw,
        "&quot;😀&amp;和文&quot;"
    );
}

#[test]
fn css_syntax_errors_are_owned_with_original_unicode_crlf_positions() {
    let source = "<p>日本語 😀</p>\r\n<style>\r\n.日本語 { color: rgb(1; }\r\n</style>";
    let error = analyze(source).unwrap_err();
    assert!(
        error
            .diagnostics
            .iter()
            .all(|d| d.phase == DiagnosticPhase::Css)
    );
    for diagnostic in error.diagnostics {
        for label in diagnostic.labels {
            assert!(
                source
                    .get(label.span.start as usize..label.span.end as usize)
                    .is_some()
            );
            assert!(label.span.start >= source.find("<style>").unwrap() as u32);
        }
    }
}

#[test]
fn ids_parent_links_and_reference_order_are_consistent_across_script_and_template() {
    let source = "---\nimport C from './c';\n---\n<script>const x=1;x;</script><C /><div>{value}</div><script is:inline>const y=2;y;</script>";
    let first = analyze(source).unwrap();
    let second = analyze(source).unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .references
            .windows(2)
            .all(|r| r[0].span.start <= r[1].span.start)
    );
    for (index, r) in first.references.iter().enumerate() {
        assert_eq!(r.id, ReferenceId(index));
    }
    for script in &first.scripts {
        for r in &script.references {
            assert_eq!(first.references[r.id.0], *r);
        }
    }
    for element in &first.elements {
        if let Some(id) = element.reference {
            assert!(id.0 < first.references.len());
        }
        if let Some(parent) = element.parent {
            assert!(parent.0 < element.id.0);
        }
    }
}

#[test]
fn receivers_type_scopes_and_module_expression_attributes_are_explicit() {
    let facts=analyze("---\ntype Item<T> = T extends infer U ? U : never; class Base {}; class Child extends Base { value(){return this;} }\n---\n<this.Widget /><script type={'module'}>export const x=1;</script><style lang={language}>not CSS @</style>").unwrap();
    assert!(facts.scopes.iter().any(|s| s.kind == ScopeKind::Type));
    assert_eq!(facts.elements[0].kind, ElementKind::Dynamic);
    assert!(
        facts
            .references
            .iter()
            .any(|r| r.name == "this" && r.kind == ReferenceKind::Receiver)
    );
    assert_eq!(facts.scripts[0].mode, ScriptMode::InlineModule);
    assert!(!facts.scripts[0].bundled);
    assert_eq!(facts.styles[0].mode, StyleMode::Dynamic);
}

#[test]
fn unquoted_paths_and_self_close_boundaries_are_distinct() {
    let facts = analyze("<a href=/about>link</a><input value=4/>").unwrap();
    assert_eq!(
        facts.elements[0].attributes[0].literal,
        Some(Literal::String("/about".into()))
    );
    assert_eq!(
        facts.elements[1].attributes[0].literal,
        Some(Literal::String("4/".into()))
    );
    let error = analyze("<Comp value= />").unwrap_err();
    assert!(
        error
            .diagnostics
            .iter()
            .flat_map(|d| &d.labels)
            .any(|l| l.span.start == 15)
    );
}

#[test]
fn animation_names_with_keyword_spellings_and_keyframe_steps_are_kept() {
    let facts=analyze("<style>.x{animation-name:linear;animation:'ease' 1s}@keyframes linear{from{opacity:0}}@keyframes ease{to{opacity:1}}</style>").unwrap();
    let css = facts.styles[0].css.as_ref().unwrap();
    assert_eq!(
        css.references
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["linear", "ease"]
    );
    assert!(css.rules.iter().any(|r| r.kind == CssRuleKind::Keyframe));
}

#[test]
fn define_vars_duplicate_and_computed_literal_keys_follow_object_semantics() {
    let source = "---\nconst before=1;const after=2;\n---\n<script define:vars={{message:before,message:after,['other']:after}}>message;other;</script>";
    let facts = analyze(source).unwrap();
    assert_eq!(facts.scripts[0].injected.len(), 2);
    assert!(!facts.scripts[0].dynamic_injection);
    for reference in &facts.scripts[0].references {
        let injected = binding(reference);
        assert_eq!(injected.injection.as_ref().unwrap().value.raw, "after");
        let origin = &facts.references[injected.injection.as_ref().unwrap().references[0].0];
        assert_eq!(binding(origin).name, "after");
    }
}

#[test]
fn css_escaped_names_conditions_and_cross_style_candidates_keep_raw_positions() {
    let source = r#"<style>@supports(display:grid){@container card (width>2px){.\65 moji{--caf\e9:red;color:var(--caf\e9,black)}}}</style><style>.b{--café:blue}</style><div style="--caf&#xE9;:green;color:var(--caf&#xE9;)"/>"#;
    let facts = analyze(source).unwrap();
    let css = facts.styles[0].css.as_ref().unwrap();
    assert_eq!(css.definitions[0].name, "--café");
    assert_eq!(css.definitions[0].conditions.len(), 2);
    assert_eq!(css.rules[2].selectors[0].classes[0].name, "emoji");
    assert_eq!(css.references[0].name, "--café");
    assert_eq!(css.references[0].candidates.len(), 3);
    assert_eq!(text(source, &css.references[0].span), r"--caf\e9");
    let inline = facts.styles[2].css.as_ref().unwrap();
    assert_eq!(text(source, &inline.references[0].span), "--caf&#xE9;");
    assert_eq!(inline.references[0].candidates.len(), 3);
}
