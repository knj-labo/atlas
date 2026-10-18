use atlas::{
    BoundaryFact, BoundaryId, BoundaryKind, FileFacts, ReferenceId, Resolution, ScopeDomain,
    ScriptMode, SourceText, TransferConditionKind as C, TransferMethod, TransferReasonKind as R,
    TransferStatus as S, TransferUnknownReason as U, TransferValue, TransferValueKind as V,
    analyze,
};

fn statuses(facts: &FileFacts, boundary: usize) -> Vec<S> {
    facts
        .assess_transfer(facts.boundaries[boundary].id)
        .unwrap()
        .inputs
        .iter()
        .map(|input| input.status)
        .collect()
}

fn conditions(facts: &FileFacts, boundary: usize) -> Vec<C> {
    facts
        .assess_transfer(facts.boundaries[boundary].id)
        .unwrap()
        .assumptions
        .into_iter()
        .map(|condition| condition.kind)
        .collect()
}

fn prop(expression: &str, declarations: &str) -> FileFacts {
    analyze(&format!(
        "---\nimport View from './View';\n{declarations}\n---\n<View client:load value={{{expression}}} />"
    ))
    .unwrap()
}

#[test]
fn literal_values_and_static_containers_have_method_specific_support() {
    let facts = analyze(
        r#"---
import View from './View';
---
<script define:vars={{ nil: null, text: '</script>😀', yes: true, no: false,
    number: -1.25, positive: +2, zero: -0, template: `plain`,
    array: [null, 'a', true, 1, { nested: [] }], object: { a: [1], ['b']: {} } }}></script>
<View client:only="react" slot="main" class:list={['a']} set:text="text"
    nil={null} text="a&amp;b" flag number={1.2e3} array={[]} object={{}} />"#,
    )
    .unwrap();
    assert_eq!(facts.boundaries.len(), 2);
    let script = &facts.boundaries[0];
    assert_eq!(script.kind, BoundaryKind::ScriptDefineVars);
    assert_eq!(script.method, TransferMethod::JsonScript);
    assert_eq!(script.evaluation, ScopeDomain::Server);
    assert_eq!(script.destination, Some(ScopeDomain::InjectedScript));
    assert_eq!(statuses(&facts, 0), vec![S::Supported; 10]);
    let component = &facts.boundaries[1];
    assert_eq!(component.kind, BoundaryKind::ClientProps);
    assert_eq!(component.method, TransferMethod::AstroProps);
    assert_eq!(component.destination, Some(ScopeDomain::BrowserModule));
    assert_eq!(
        component
            .inputs
            .iter()
            .map(|i| i.name.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("nil"),
            Some("text"),
            Some("flag"),
            Some("number"),
            Some("array"),
            Some("object")
        ]
    );
    assert_eq!(component.inputs[1].value.kind, V::String("a&b".into()));
    assert_eq!(component.inputs[2].value.kind, V::Boolean(true));
    assert_eq!(statuses(&facts, 1), vec![S::Supported; 6]);
    for index in 0..2 {
        let assumptions = conditions(&facts, index);
        assert!(assumptions.contains(&C::DataDisclosure));
        assert!(assumptions.contains(&C::CodeMovementSafety));
        assert_eq!(assumptions.contains(&C::RendererAvailability), index == 1);
    }
}

#[test]
fn bigint_and_functions_do_not_silently_use_json_value_loss() {
    let facts = analyze(
        r#"---
import View from './View';
const large = 9007199254740993n;
const alias = large;
---
<script define:vars={{ big: 1n, negative: -0xfn, alias, nested: [1n], fn: () => 1 }}></script>
<View client:load big={1n} negative={-0xfn} alias={alias} nested={[1n]}
    arrow={() => 1} fn={function () {}} generator={function* () {}}
    method={{ run() {} }} mixed={[() => 1, missing]} />"#,
    )
    .unwrap();
    assert_eq!(statuses(&facts, 0), vec![S::Unsupported; 5]);
    assert_eq!(
        statuses(&facts, 1),
        [
            S::Supported,
            S::Supported,
            S::Supported,
            S::Supported,
            S::Unsupported,
            S::Unsupported,
            S::Unsupported,
            S::Unsupported,
            S::Unsupported
        ]
    );
    let script = facts.assess_transfer(BoundaryId(0)).unwrap();
    assert_eq!(script.inputs[0].reasons[0].kind, R::BigIntInJson);
    let component = facts.assess_transfer(BoundaryId(1)).unwrap();
    assert_eq!(component.inputs[0].reasons[0].kind, R::BigIntEncoding);
    assert!(
        component.inputs[7]
            .reasons
            .iter()
            .any(|r| r.kind == R::FunctionValue)
    );
    assert_eq!(facts.boundaries[1].inputs[2].value.origins.len(), 2);
}

#[test]
fn insufficient_value_evidence_stays_unknown() {
    let declarations = "let mutable = 1; const object = { a: 1 }; object.a = 2; const array = [1]; const alias = object; const fn = () => 1; const { field } = object;";
    let cases = [
        ("mutable", U::MutableBinding),
        ("object", U::AggregateBinding),
        ("array", U::AggregateBinding),
        ("alias", U::AggregateBinding),
        ("fn", U::NonPrimitiveBinding),
        ("field", U::UntrackedBinding),
        ("missing", U::ExternalReference),
        ("Astro", U::ExternalReference),
        ("read()", U::CallResult),
        ("new Date()", U::CallResult),
        ("{ ...object }", U::Spread),
        ("{ [mutable]: 1 }", U::DynamicKey),
        ("{ get value() { return 1 } }", U::Accessor),
        ("{ __proto__: null, a: 1 }", U::NonPlainObject),
        ("{ a: 1, a: 2 }", U::DuplicateKey),
        ("{ a: 1, ['a']: 2 }", U::DuplicateKey),
        ("{ 1e21: 1, '1e+21': 2 }", U::DuplicateKey),
        ("{ [1e-7]: 1, '1e-7': 2 }", U::DuplicateKey),
        ("/pattern/g", U::UnsupportedExpression),
        ("1 + 2", U::UnsupportedExpression),
        ("`dynamic${mutable}`", U::UnsupportedExpression),
        ("Infinity", U::ExternalReference),
        ("NaN", U::ExternalReference),
        ("undefined", U::ExternalReference),
        ("1e400", U::NonFiniteNumber),
    ];
    for (expression, reason) in cases {
        let facts = prop(expression, declarations);
        assert_eq!(statuses(&facts, 0), [S::Unknown], "{expression}");
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind,
            V::Unknown(reason),
            "{expression}"
        );
        let result = facts.assess_transfer(BoundaryId(0)).unwrap();
        assert!(
            result.inputs[0]
                .reasons
                .iter()
                .any(|r| r.kind == R::UnknownValue(reason)),
            "{expression}"
        );
    }
    for (expression, reason) in [("[...array]", U::Spread), ("[, 1]", U::ArrayHole)] {
        let facts = prop(expression, declarations);
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
        assert!(
            facts.assess_transfer(BoundaryId(0)).unwrap().inputs[0]
                .reasons
                .iter()
                .any(|r| r.kind == R::UnknownValue(reason))
        );
    }
}

#[test]
fn imported_values_and_calls_are_not_followed_as_constants() {
    let facts = prop("remote", "import { remote } from './values';");
    assert_eq!(statuses(&facts, 0), [S::Unknown]);
    assert_eq!(
        facts.boundaries[0].inputs[0].value.kind,
        V::Unknown(U::ExternalReference)
    );
    for initializer in ["getValue()", "Object.freeze({ a: 1 })", "new Map()"] {
        let facts = prop("value", &format!("const value = {initializer};"));
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::CallResult)
        );
        assert_eq!(facts.boundaries[0].inputs[0].value.origins.len(), 1);
    }
}

#[test]
fn constants_aliases_and_shadowing_keep_server_and_browser_identity() {
    let source = r#"---
import View from './View';
const base = 'server';
const same = base;
---
<script define:vars={{ same }}>same; ((same) => same)(same);</script>
<script define:vars={{ same }}>same;</script>
<script>const same = { browser: true }; same;</script>
<View client:visible value={same} />
{[1].map((same) => <View client:idle value={same} />)}
{(() => { const same = 2; return <View client:load value={same} />; })()}"#;
    let facts = analyze(source).unwrap();
    assert_eq!(facts.boundaries.len(), 5);
    for index in [0, 1, 2] {
        assert_eq!(statuses(&facts, index), [S::Supported]);
        assert_eq!(
            facts.boundaries[index].inputs[0].value.kind,
            V::String("server".into())
        );
        assert_eq!(facts.boundaries[index].inputs[0].value.origins.len(), 2);
    }
    assert_eq!(statuses(&facts, 3), [S::Unknown]);
    assert_eq!(statuses(&facts, 4), [S::Supported]);
    assert_eq!(
        facts.boundaries[4].inputs[0].value.kind,
        V::FiniteNumber("2".into())
    );
    let server = facts.boundaries[0].inputs[0].value.origins[0].declaration;
    assert_eq!(
        facts.scopes[facts.declarations[server.0].scope.0].domain,
        ScopeDomain::Server
    );
    for script in &facts.scripts[..2] {
        let injected = script.injected[0];
        assert_ne!(injected, server);
        assert!(
            facts
                .references_to(injected)
                .iter()
                .all(|r| { facts.scopes[r.scope.0].domain == ScopeDomain::InjectedScript })
        );
    }
    assert_ne!(facts.scripts[0].injected, facts.scripts[1].injected);
    check_boundaries(&facts, source);
}

#[test]
fn cyclic_aliases_forward_initialization_and_analysis_limits_are_unknown() {
    for declarations in ["const a = b; const b = a;", "const a = a;"] {
        let facts = prop("a", declarations);
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::CyclicAlias)
        );
    }
    let forward = prop("a", "const a = b; const b = 1;");
    assert_eq!(statuses(&forward, 0), [S::Unknown]);
    assert_eq!(
        forward.boundaries[0].inputs[0].value.kind,
        V::Unknown(U::UncertainInitialization)
    );
    let mut declarations = String::from("const c0 = 1;\n");
    for index in 1..140 {
        declarations.push_str(&format!("const c{index} = c{};\n", index - 1));
    }
    let limited = prop("c139", &declarations);
    assert_eq!(statuses(&limited, 0), [S::Unknown]);
    assert_eq!(
        limited.boundaries[0].inputs[0].value.kind,
        V::Unknown(U::AnalysisLimit)
    );
}

#[test]
fn unknown_members_keep_static_aggregates_unknown() {
    for expression in [
        "[1, missing]",
        "{ known: [1], uncertain: missing }",
        "[{ a: 1 }, [...missing]]",
    ] {
        let facts = prop(expression, "");
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
    }
    // A const aggregate is mutable through its properties, even inside a fresh literal.
    let facts = prop("[object]", "const object = { a: 1 }; object.a = () => 1;");
    assert_eq!(statuses(&facts, 0), [S::Unknown]);
}

#[test]
fn known_client_directives_create_boundaries_but_custom_directives_do_not() {
    let source = r#"---
import View from './View';
const Dynamic = View;
---
<View client:load value={1} />
<View client:idle value={1} />
<View client:visible value={1} />
<View client:media="(width > 600px)" value={1} />
<View client:only="react" value={1} />
<View client:custom value={1n} />
<View value={1} />
<div client:load value={1} />
<style define:vars={{ color: 'red' }}>p { color: var(--color) }</style>
<Dynamic client:load value={1} />
<Astro.self client:load value={1} />"#;
    let facts = analyze(source).unwrap();
    assert_eq!(facts.boundaries.len(), 7);
    for index in 0..7 {
        assert_eq!(statuses(&facts, index), [S::Supported]);
        assert!(conditions(&facts, index).contains(&C::RendererAvailability));
    }
    assert!(
        conditions(&facts, 5).contains(&C::DirectiveValidation(atlas::ValidationStatus::Invalid))
    );
    assert!(
        facts
            .boundaries
            .iter()
            .all(|b| b.kind == BoundaryKind::ClientProps)
    );
    check_boundaries(&facts, source);
}

#[test]
fn invalid_directives_preserve_validation_and_do_not_establish_a_handoff() {
    let facts = analyze(
        r#"---
import View from './View';
---
<View client:media={23} value={1} />
<View client:load client:custom value={1} />
<script define:vars={1}></script>
<script define:vars></script>
<script define:vars={{ '123key': 1 }}></script>"#,
    )
    .unwrap();
    assert_eq!(facts.boundaries.len(), 5);
    assert_eq!(statuses(&facts, 0), [S::Supported]);
    assert_eq!(
        facts.boundaries[0].directives[0].status,
        atlas::ValidationStatus::Invalid
    );
    assert!(
        conditions(&facts, 0).contains(&C::DirectiveValidation(atlas::ValidationStatus::Invalid))
    );
    assert!(conditions(&facts, 1).contains(&C::DuplicateDirective));
    assert_eq!(statuses(&facts, 2), [S::Unknown]);
    assert!(
        conditions(&facts, 2).contains(&C::DirectiveValidation(atlas::ValidationStatus::Invalid))
    );
    assert_eq!(statuses(&facts, 3), [S::Unknown]);
    assert!(conditions(&facts, 4).contains(&C::InvalidInjectedName));
    for boundary in &facts.boundaries {
        assert!(
            boundary
                .directives
                .iter()
                .all(|d| facts.elements[boundary.element.0].directives.contains(d))
        );
    }
}

#[test]
fn duplicate_inputs_spreads_and_dynamic_keys_preserve_handoff_uncertainty() {
    let source = r#"---
import View from './View';
const key = 'dynamic';
const others = {};
---
<script define:vars={{ x: 1, x: () => 1, [key]: 1, ...others }}></script>
<View client:load x={1} x={2} {...others} />"#;
    let facts = analyze(source).unwrap();
    assert_eq!(
        statuses(&facts, 0),
        [S::Supported, S::Unsupported, S::Unknown, S::Unknown]
    );
    assert_eq!(
        statuses(&facts, 1),
        [S::Supported, S::Supported, S::Unknown]
    );
    for index in 0..2 {
        let result = facts.assess_transfer(BoundaryId(index)).unwrap();
        assert!(
            result
                .assumptions
                .iter()
                .any(|c| c.kind == C::DuplicateInput("x".into()))
        );
        assert!(
            result
                .assumptions
                .iter()
                .any(|c| c.kind == C::DynamicInputSet)
        );
    }
    assert_eq!(
        facts.boundaries[0].inputs[2].value.kind,
        V::Unknown(U::DynamicKey)
    );
    assert_eq!(
        facts.boundaries[0].inputs[3].value.kind,
        V::Unknown(U::Spread)
    );
    check_boundaries(&facts, source);
}

#[test]
fn non_enumerable_define_vars_input_sets_are_unknown() {
    for attribute in ["{values}", "{read()}", "\"literal\"", "{null}", "{[1]}"] {
        let source = format!(
            "---\nconst values = {{ a: 1 }};\n---\n<script define:vars={attribute}></script>"
        );
        let facts = analyze(&source).unwrap();
        assert_eq!(statuses(&facts, 0), [S::Unknown], "{attribute}");
        assert!(conditions(&facts, 0).contains(&C::DynamicInputSet));
        assert!(facts.boundaries[0].inputs[0].name.is_none());
    }
    let empty = analyze("<script define:vars={{}}></script>").unwrap();
    assert!(statuses(&empty, 0).is_empty());
    let parenthesized = analyze("<script define:vars={({ a: 1 })}></script>").unwrap();
    assert_eq!(statuses(&parenthesized, 0), [S::Supported]);
}

#[test]
fn unanalyzed_script_modes_are_explicit_conditions() {
    let facts = analyze(
        r#"<script define:vars={{ a: 1 }} type="application/json">not JavaScript</script>
<script define:vars={{ a: 1 }} src="./external.js">not JavaScript</script>
<script define:vars={{ a: 1 }} type={Astro.props.type}>not JavaScript</script>
<script define:vars={{ a: 1 }} type="module">a;</script>"#,
    )
    .unwrap();
    for (index, mode) in [ScriptMode::Data, ScriptMode::External, ScriptMode::Dynamic]
        .into_iter()
        .enumerate()
    {
        assert_eq!(statuses(&facts, index), [S::Supported]);
        assert_eq!(facts.boundaries[index].destination, None);
        assert!(conditions(&facts, index).contains(&C::UnanalyzedScript(mode)));
    }
    assert_eq!(
        facts.boundaries[3].destination,
        Some(ScopeDomain::BrowserModule)
    );
    assert!(
        !conditions(&facts, 3)
            .iter()
            .any(|c| matches!(c, C::UnanalyzedScript(_)))
    );
}

#[test]
fn script_attribute_spreads_and_duplicates_leave_execution_mode_unconfirmed() {
    let source = r#"<script define:vars={{ a: 1 }} {...Astro.props}>a;</script>
<script define:vars={{ a: 1 }} type="text/javascript" type="module">a;</script>"#;
    let facts = analyze(source).unwrap();
    for index in 0..2 {
        assert_eq!(statuses(&facts, index), [S::Supported]);
        assert_eq!(facts.boundaries[index].destination, None);
        assert!(conditions(&facts, index).contains(&C::UncertainScriptAttributes));
    }
}

#[test]
fn unicode_crlf_raw_text_nested_origins_and_reference_ids_survive_parser_release() {
    let source = String::from(
        "---\r\nimport View from './View';\r\nconst 名前 = '😀';\r\nconst 別名 = 名前;\r\n---\r\n<script define:vars={{ 名前: 別名 }}>名前;</script>\r\n<View client:load text={別名} nested={{ value: [別名] }} />\r\n",
    );
    let facts = analyze(&source).unwrap();
    let before = facts.clone();
    check_boundaries(&facts, &source);
    assert_eq!(facts.boundaries[1].inputs[0].value.source.raw, "別名");
    let value = &facts.boundaries[1].inputs[0].value;
    assert_eq!(value.origins.len(), 2);
    assert_eq!(value.origins[0].initializer.raw, "名前");
    assert_eq!(value.origins[1].initializer.raw, "'😀'");
    let assessment = facts.assess_transfer(BoundaryId(1)).unwrap();
    for (index, input) in assessment.inputs.iter().enumerate() {
        assert_eq!(input.input, index);
        assert_eq!(input.status, S::Supported);
        for reason in &input.reasons {
            assert!(
                source
                    .get(reason.span.start as usize..reason.span.end as usize)
                    .is_some()
            );
        }
        assert!(
            input
                .reasons
                .iter()
                .any(|r| r.kind == R::ConstantInitializer)
        );
    }
    assert_eq!(facts, before);
    drop(source);
    assert_eq!(facts.assess_transfer(BoundaryId(1)).unwrap(), assessment);
    drop(facts);
    assert_eq!(assessment.inputs.len(), 2);
}

#[test]
fn invalid_boundary_ids_return_none_and_queries_are_read_only() {
    for facts in [FileFacts::default(), analyze("").unwrap(), prop("1", "")] {
        let before = facts.clone();
        assert!(
            facts
                .assess_transfer(BoundaryId(facts.boundaries.len()))
                .is_none()
        );
        assert!(facts.assess_transfer(BoundaryId(usize::MAX)).is_none());
        for boundary in &facts.boundaries {
            assert_eq!(
                facts.assess_transfer(boundary.id),
                facts.assess_transfer(boundary.id)
            );
        }
        assert_eq!(facts, before);
    }
}

fn raw<'s>(source: &'s str, span: &std::ops::Range<u32>) -> &'s str {
    &source[span.start as usize..span.end as usize]
}

fn check_source(source: &str, value: &SourceText) {
    assert_eq!(raw(source, &value.span), value.raw);
}

fn check_reference(facts: &FileFacts, id: ReferenceId) {
    let reference = &facts.references[id.0];
    assert_eq!(reference.id, id);
    assert_eq!(facts.reference_at(reference.span.start), Some(reference));
    assert_eq!(facts.scopes[reference.scope.0].domain, ScopeDomain::Server);
}

fn check_value(facts: &FileFacts, source: &str, value: &TransferValue) {
    check_source(source, &value.source);
    for origin in &value.origins {
        check_reference(facts, origin.reference);
        let Resolution::Resolved(binding) = &facts.references[origin.reference.0].resolution else {
            panic!("constant origin must resolve")
        };
        assert_eq!(binding.id, origin.declaration);
        assert_eq!(binding, &facts.declarations[origin.declaration.0]);
        check_source(source, &origin.initializer);
    }
    match &value.kind {
        V::Array(values) => values.iter().for_each(|v| check_value(facts, source, v)),
        V::Object(properties) => properties
            .iter()
            .for_each(|(_, v)| check_value(facts, source, v)),
        _ => {}
    }
}

fn check_boundaries(facts: &FileFacts, source: &str) {
    for (index, boundary) in facts.boundaries.iter().enumerate() {
        assert_eq!(boundary.id, BoundaryId(index));
        check_boundary(facts, source, boundary);
    }
}

fn check_boundary(facts: &FileFacts, source: &str, boundary: &BoundaryFact) {
    let element = &facts.elements[boundary.element.0];
    assert_eq!(element.id, boundary.element);
    assert_eq!(element.opening_span, boundary.span);
    for input in &boundary.inputs {
        if let Some(span) = &input.key_span {
            assert!(!raw(source, span).is_empty());
        }
        for id in &input.references {
            check_reference(facts, *id);
            let reference = &facts.references[id.0];
            assert!(
                input.value.source.span.start <= reference.span.start
                    && reference.span.end <= input.value.source.span.end
            );
        }
        check_value(facts, source, &input.value);
    }
    for condition in &boundary.conditions {
        assert!(!raw(source, &condition.span).is_empty());
    }
}

#[cfg(feature = "serde")]
#[test]
fn rust_only_boundary_facts_leave_existing_json_and_report_schema_unchanged() {
    let mut facts = prop("[1n, { value: 2 }]", "");
    let json = serde_json::to_value(&facts).unwrap();
    assert!(!facts.boundaries.is_empty());
    assert!(json.get("boundaries").is_none());
    facts.boundaries.clear();
    assert_eq!(serde_json::to_value(&facts).unwrap(), json);
    let project = atlas::analyze_project([
        (
            "Page.astro",
            "---\nimport View from './View.astro';\n---\n<View client:load value={1n} />",
        ),
        ("View.astro", "<slot />"),
    ]);
    let report = project.report();
    assert_eq!(report.schema_version, 1);
    assert_eq!(atlas::REPORT_SCHEMA_VERSION, 1);
    let report_json = serde_json::to_value(&report).unwrap();
    assert_eq!(report_json.as_object().unwrap().len(), 3);
    for file in report_json["files"].as_array().unwrap() {
        assert_eq!(file.as_object().unwrap().len(), 6);
        assert!(file.get("boundaries").is_none());
    }
}

#[cfg(feature = "incremental")]
#[test]
fn cached_and_retained_assessments_keep_their_owned_values_after_updates() {
    use std::sync::Arc;
    let source = "---\r\nconst 名前 = '😀';\r\nconst alias = 名前;\r\n---\r\n<script define:vars={{ text: alias }}></script>";
    let updated_source = "<script define:vars={{ text: 1n }}></script>";
    let (first, updated, assessment) = {
        let mut db = atlas::AnalysisDatabase::new();
        db.set_source("Page.astro", source);
        let first = db.analyze_file("Page.astro").unwrap();
        assert!(Arc::ptr_eq(&first, &db.analyze_file("Page.astro").unwrap()));
        let facts = first.as_ref().as_ref().unwrap();
        check_boundaries(facts, source);
        let assessment = facts.assess_transfer(BoundaryId(0)).unwrap();
        assert_eq!(assessment.inputs[0].status, S::Supported);
        db.set_source("Page.astro", updated_source);
        let updated = db.analyze_file("Page.astro").unwrap();
        assert!(!Arc::ptr_eq(&first, &updated));
        (first, updated, assessment)
    };
    let facts = first.as_ref().as_ref().unwrap();
    assert_eq!(facts.assess_transfer(BoundaryId(0)).unwrap(), assessment);
    let updated = updated.as_ref().as_ref().unwrap();
    assert_eq!(updated, &analyze(updated_source).unwrap());
    assert_eq!(statuses(updated, 0), [S::Unsupported]);
    check_boundaries(facts, source);
}
