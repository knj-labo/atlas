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

const METHODS: [TransferMethod; 2] = [TransferMethod::JsonScript, TransferMethod::AstroProps];

fn handoff(expression: &str, declarations: &str, method: TransferMethod) -> String {
    let element = match method {
        TransferMethod::JsonScript => {
            format!("<script define:vars={{{{ value: {expression} }}}}>value;</script>")
        }
        TransferMethod::AstroProps => format!("<View client:load value={{{expression}}} />"),
    };
    format!("---\nimport View from './View';\n{declarations}\n---\n{element}")
}

#[test]
fn primitive_and_alias_cases_agree_for_both_methods() {
    let declarations = "const nil = null; const text = '😀'; const yes = true; const number = -12.5; const large = -0xfn; const alias = text; const twice = alias;";
    let cases = [
        ("null", V::Null, 0),
        ("'😀'", V::String("😀".into()), 0),
        ("`plain`", V::String("plain".into()), 0),
        ("false", V::Boolean(false), 0),
        ("+0x10", V::FiniteNumber("16".into()), 0),
        ("-12.5", V::FiniteNumber("-12.5".into()), 0),
        ("-0", V::FiniteNumber("-0".into()), 0),
        ("1_000", V::FiniteNumber("1000".into()), 0),
        ("nil", V::Null, 1),
        ("text", V::String("😀".into()), 1),
        ("yes", V::Boolean(true), 1),
        ("number", V::FiniteNumber("-12.5".into()), 1),
        ("twice", V::String("😀".into()), 3),
        ("large", V::BigInt("-15".into()), 1),
    ];
    for method in METHODS {
        for (expression, expected, origins) in &cases {
            let source = handoff(expression, declarations, method);
            let facts = analyze(&source).unwrap();
            let value = &facts.boundaries[0].inputs[0].value;
            assert_eq!(&value.kind, expected, "{method:?}: {expression}");
            assert_eq!(value.origins.len(), *origins, "{expression}");
            let expected_status = if *expression == "large" && method == TransferMethod::JsonScript
            {
                S::Unsupported
            } else {
                S::Supported
            };
            assert_eq!(statuses(&facts, 0), [expected_status], "{expression}");
            check_boundaries(&facts, &source);
        }
    }
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
        ("!true", U::UnsupportedExpression),
        ("+1n", U::UnsupportedExpression),
        ("`dynamic${mutable}`", U::UnsupportedExpression),
        ("Infinity", U::ExternalReference),
        ("NaN", U::ExternalReference),
        ("undefined", U::ExternalReference),
        ("1e400", U::NonFiniteNumber),
        ("<span />", U::UnsupportedExpression),
        ("<>text</>", U::UnsupportedExpression),
    ];
    for ((expression, reason), method) in cases
        .into_iter()
        .flat_map(|case| METHODS.map(|method| (case, method)))
    {
        let facts = analyze(&handoff(expression, declarations, method)).unwrap();
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
        for method in METHODS {
            let facts = analyze(&handoff(expression, declarations, method)).unwrap();
            assert_eq!(statuses(&facts, 0), [S::Unknown]);
            assert!(
                facts.assess_transfer(BoundaryId(0)).unwrap().inputs[0]
                    .reasons
                    .iter()
                    .any(|r| r.kind == R::UnknownValue(reason))
            );
        }
    }
}

#[test]
fn jsx_attribute_values_and_empty_expressions_remain_unknown() {
    for value in ["<span />", "<>text</>", "{/* empty */}"] {
        let source = format!(
            "---\r\nimport View from './View';\r\nconst 注釈 = '😀';\r\n---\r\n<View client:load value={value} />"
        );
        let facts = analyze(&source).unwrap();
        assert_eq!(statuses(&facts, 0), [S::Unknown], "{value}");
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::UnsupportedExpression)
        );
        let result = facts.assess_transfer(BoundaryId(0)).unwrap();
        assert_eq!(
            result.inputs[0]
                .reasons
                .iter()
                .map(|r| (r.kind, r.span.clone()))
                .collect::<Vec<_>>(),
            [(
                R::UnknownValue(U::UnsupportedExpression),
                span_in(&source, &format!("value={value}"), value)
            )]
        );
        check_boundaries(&facts, &source);
    }
}

#[test]
fn ambient_constants_and_written_constants_never_become_supported() {
    for (declarations, expected) in [
        ("declare const value: number;", U::UncertainInitialization),
        ("const value = 1; value = 2;", U::MutableBinding),
        ("const value = 1; value++;", U::MutableBinding),
        ("const value = 1; ({ value } = other);", U::MutableBinding),
    ] {
        for method in METHODS {
            let source = handoff("value", declarations, method);
            let facts = analyze(&source).unwrap();
            assert_eq!(statuses(&facts, 0), [S::Unknown], "{declarations}");
            assert_eq!(
                facts.boundaries[0].inputs[0].value.kind,
                V::Unknown(expected)
            );
            assert!(facts.boundaries[0].inputs[0].value.origins.is_empty());
            check_boundaries(&facts, &source);
        }
    }
}

#[test]
fn accessors_and_prototype_setters_differ_from_computed_data_keys() {
    let cases = [
        ("{ get x() { return 1; } }", S::Unknown, Some(U::Accessor)),
        ("{ set x(value) {} }", S::Unknown, Some(U::Accessor)),
        ("{ __proto__: null }", S::Unknown, Some(U::NonPlainObject)),
        ("{ '__proto__': null }", S::Unknown, Some(U::NonPlainObject)),
        ("{ ['__proto__']: null }", S::Supported, None),
        ("{ __proto__ }", S::Supported, None),
        ("{ ['x']: 1, [2]: true }", S::Supported, None),
    ];
    for method in METHODS {
        for (expression, status, reason) in cases {
            let source = handoff(expression, "const __proto__ = 1;", method);
            let facts = analyze(&source).unwrap();
            assert_eq!(statuses(&facts, 0), [status], "{method:?}: {expression}");
            if let Some(reason) = reason {
                assert_eq!(facts.boundaries[0].inputs[0].value.kind, V::Unknown(reason));
            }
            check_boundaries(&facts, &source);
        }
    }
    // These are properties of the injection object itself, rather than nested values.
    for (property, expected) in [
        ("get value() { return 1; }", V::Unknown(U::Accessor)),
        ("set value(next) {}", V::Unknown(U::Accessor)),
        ("__proto__: null", V::Unknown(U::NonPlainObject)),
        ("'__proto__': null", V::Unknown(U::NonPlainObject)),
        ("['__proto__']: null", V::Null),
        ("__proto__", V::FiniteNumber("1".into())),
        ("value() {}", V::Function),
    ] {
        let source = format!(
            "---\nconst __proto__ = 1;\n---\n<script define:vars={{{{ {property} }}}}></script>"
        );
        let facts = analyze(&source).unwrap();
        assert_eq!(facts.boundaries[0].inputs.len(), 1);
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind, expected,
            "{property}"
        );
        let status = match expected {
            V::Unknown(_) => S::Unknown,
            V::Function => S::Unsupported,
            _ => S::Supported,
        };
        assert_eq!(statuses(&facts, 0), [status], "{property}");
        check_boundaries(&facts, &source);
    }
}

#[test]
fn imported_values_and_calls_are_not_followed_as_constants() {
    for method in METHODS {
        let facts = analyze(&handoff(
            "remote",
            "import { remote } from './values';",
            method,
        ))
        .unwrap();
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
        assert_eq!(
            facts.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::ExternalReference)
        );
        for initializer in ["getValue()", "Object.freeze({ a: 1 })", "new Map()"] {
            let facts = analyze(&handoff(
                "value",
                &format!("const value = {initializer};"),
                method,
            ))
            .unwrap();
            assert_eq!(statuses(&facts, 0), [S::Unknown]);
            assert_eq!(
                facts.boundaries[0].inputs[0].value.kind,
                V::Unknown(U::CallResult)
            );
            assert_eq!(facts.boundaries[0].inputs[0].value.origins.len(), 1);
        }
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
    for method in METHODS {
        for declarations in ["const a = b; const b = a;", "const a = a;"] {
            let facts = analyze(&handoff("a", declarations, method)).unwrap();
            assert_eq!(statuses(&facts, 0), [S::Unknown]);
            assert_eq!(
                facts.boundaries[0].inputs[0].value.kind,
                V::Unknown(U::CyclicAlias)
            );
        }
        let forward = analyze(&handoff("a", "const a = b; const b = 1;", method)).unwrap();
        assert_eq!(statuses(&forward, 0), [S::Unknown]);
        assert_eq!(
            forward.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::UncertainInitialization)
        );
        let mut declarations = String::from("const c0 = 1;\n");
        for index in 1..140 {
            declarations.push_str(&format!("const c{index} = c{};\n", index - 1));
        }
        let limited = analyze(&handoff("c139", &declarations, method)).unwrap();
        assert_eq!(statuses(&limited, 0), [S::Unknown]);
        assert_eq!(
            limited.boundaries[0].inputs[0].value.kind,
            V::Unknown(U::AnalysisLimit)
        );
    }
}

#[test]
fn alias_and_container_depth_limits_have_exact_edges() {
    let mut declarations = String::from("const c0 = 1;\n");
    for index in 1..128 {
        declarations.push_str(&format!("const c{index} = c{};\n", index - 1));
    }
    for method in METHODS {
        for (name, status, expected, origins) in [
            ("c126", S::Supported, V::FiniteNumber("1".into()), 127),
            ("c127", S::Unknown, V::Unknown(U::AnalysisLimit), 128),
        ] {
            let source = handoff(name, &declarations, method);
            let facts = analyze(&source).unwrap();
            assert_eq!(statuses(&facts, 0), [status], "{method:?}: {name}");
            let value = &facts.boundaries[0].inputs[0].value;
            assert_eq!(value.kind, expected);
            assert_eq!(value.origins.len(), origins);
            for (index, origin) in value.origins.iter().enumerate() {
                assert_eq!(
                    facts.declarations[origin.declaration.0].name,
                    format!("c{}", origins - index - 1)
                );
            }
            check_boundaries(&facts, &source);
        }
        for object in [false, true] {
            for (leaf, depth, status) in [
                ("1", 127, S::Supported),
                ("1", 128, S::Unknown),
                ("c0", 126, S::Supported),
                ("c0", 127, S::Unknown),
            ] {
                let expression = if object {
                    format!("{}{}{}", "{ x: ".repeat(depth), leaf, " }".repeat(depth))
                } else {
                    format!("{}{}{}", "[".repeat(depth), leaf, "]".repeat(depth))
                };
                let source = handoff(&expression, "const c0 = 1;", method);
                let facts = analyze(&source).unwrap();
                assert_eq!(
                    statuses(&facts, 0),
                    [status],
                    "{method:?}: object={object}, {leaf}, depth={depth}"
                );
                let assessment = facts.assess_transfer(BoundaryId(0)).unwrap();
                assert_eq!(
                    assessment.inputs[0]
                        .reasons
                        .iter()
                        .filter(|r| r.kind == R::UnknownValue(U::AnalysisLimit))
                        .count(),
                    usize::from(status == S::Unknown)
                );
                let mut value = &facts.boundaries[0].inputs[0].value;
                for _ in 0..depth {
                    value = match &value.kind {
                        V::Array(values) => &values[0],
                        V::Object(properties) => &properties[0].1,
                        other => panic!("expected container, got {other:?}"),
                    };
                }
                assert_eq!(
                    value.kind,
                    if status == S::Unknown {
                        V::Unknown(U::AnalysisLimit)
                    } else {
                        V::FiniteNumber("1".into())
                    }
                );
                check_boundaries(&facts, &source);
            }
        }
    }
}

#[test]
fn unsupported_members_dominate_unknown_members_in_either_order() {
    for (expression, json, props) in [
        ("[missing, () => 1]", S::Unsupported, S::Unsupported),
        ("[() => 1, missing]", S::Unsupported, S::Unsupported),
        (
            "{ uncertain: missing, bad: [() => 1] }",
            S::Unsupported,
            S::Unsupported,
        ),
        (
            "{ bad: [() => 1], uncertain: missing }",
            S::Unsupported,
            S::Unsupported,
        ),
        ("[missing, 1n]", S::Unsupported, S::Unknown),
        ("[1n, missing]", S::Unsupported, S::Unknown),
        ("[missing, [other]]", S::Unknown, S::Unknown),
        ("{ a: missing, b: [1] }", S::Unknown, S::Unknown),
    ] {
        for (method, expected) in [
            (TransferMethod::JsonScript, json),
            (TransferMethod::AstroProps, props),
        ] {
            let source = handoff(expression, "", method);
            let facts = analyze(&source).unwrap();
            assert_eq!(statuses(&facts, 0), [expected], "{method:?}: {expression}");
            check_boundaries(&facts, &source);
        }
    }
}

// A fixture grammar of JSON-safe values, independent of the transfer rules.
#[derive(Debug)]
enum StaticValue {
    Null,
    Text,
    Boolean(bool),
    Number(i32),
    Alias,
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl StaticValue {
    fn generate(seed: &mut u32, depth: usize) -> Self {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let choice = (*seed >> 16) % if depth == 0 { 5 } else { 7 };
        match choice {
            0 => Self::Null,
            1 => Self::Text,
            2 => Self::Boolean(*seed & 1 == 0),
            3 => Self::Number((*seed % 2001) as i32 - 1000),
            4 => Self::Alias,
            5 => Self::Array((0..2).map(|_| Self::generate(seed, depth - 1)).collect()),
            _ => Self::Object(
                (0..2)
                    .map(|i| (format!("k{i}"), Self::generate(seed, depth - 1)))
                    .collect(),
            ),
        }
    }

    fn render(&self, parentheses: bool, aliases: bool) -> String {
        let expression = match self {
            Self::Null => "null".into(),
            Self::Text => "'日本語😀</script>'".into(),
            Self::Boolean(value) => value.to_string(),
            Self::Number(value) => value.to_string(),
            Self::Alias if aliases => "alias".into(),
            Self::Alias => "'😀'".into(),
            Self::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| v.render(parentheses, aliases))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Object(properties) => format!(
                "{{ {} }}",
                properties
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.render(parentheses, aliases)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        if parentheses {
            format!("({expression})")
        } else {
            expression
        }
    }

    fn check(
        &self,
        value: &TransferValue,
        facts: &FileFacts,
        source: &str,
        parentheses: bool,
        aliases: bool,
    ) {
        assert_eq!(value.source.raw, self.render(parentheses, aliases));
        if matches!(self, Self::Alias) && aliases {
            assert_eq!(value.origins.len(), 2);
            for (origin, name, initializer) in [
                (&value.origins[0], "alias", "base"),
                (&value.origins[1], "base", "'😀'"),
            ] {
                let binding = &facts.declarations[origin.declaration.0];
                assert_eq!(binding.name, name);
                assert_eq!(
                    binding.span,
                    span_in(source, &format!("const {name} = {initializer};"), name)
                );
                assert_eq!(facts.scopes[binding.scope.0].domain, ScopeDomain::Server);
                assert_eq!(
                    origin.initializer.span,
                    span_in(
                        source,
                        &format!("const {name} = {initializer};"),
                        initializer
                    )
                );
            }
            let alias_position =
                value.source.span.start + value.source.raw.find("alias").unwrap() as u32;
            assert_eq!(
                value.origins[0].reference,
                facts.reference_at(alias_position).unwrap().id
            );
            assert_eq!(
                facts.references[value.origins[1].reference.0].span,
                span_in(source, "const alias = base;", "base")
            );
        } else {
            assert!(value.origins.is_empty());
        }
        match (self, &value.kind) {
            (Self::Null, V::Null) => {}
            (Self::Text, V::String(s)) => assert_eq!(s, "日本語😀</script>"),
            (Self::Alias, V::String(s)) => assert_eq!(s, "😀"),
            (Self::Boolean(expected), V::Boolean(actual)) => assert_eq!(actual, expected),
            (Self::Number(expected), V::FiniteNumber(actual)) => {
                assert_eq!(actual, &expected.to_string())
            }
            (Self::Array(expected), V::Array(actual)) => {
                assert_eq!(actual.len(), expected.len());
                for (expected, actual) in expected.iter().zip(actual) {
                    expected.check(actual, facts, source, parentheses, aliases);
                }
            }
            (Self::Object(expected), V::Object(actual)) => {
                assert_eq!(actual.len(), expected.len());
                for ((expected_key, expected), (actual_key, actual)) in expected.iter().zip(actual)
                {
                    assert_eq!(actual_key, expected_key);
                    expected.check(actual, facts, source, parentheses, aliases);
                }
            }
            _ => panic!("fixture {self:?} does not match {:?}", value.kind),
        }
    }
}

#[test]
fn seeded_static_values_preserve_support_and_declaration_identity() {
    let mut seed = 0x51a7_c0de;
    let declarations = "const base = '😀';\nconst alias = base;";
    let scripts = "<script>const alias = 'browser'; alias;</script>\n<script is:inline>const base = false; base;</script>\n";
    for case in 0..128 {
        // The outer object guarantees an alias leaf, with maximum leaf depth 4.
        let expected = StaticValue::Object(vec![
            ("generated".into(), StaticValue::generate(&mut seed, 3)),
            ("marker".into(), StaticValue::Alias),
        ]);
        for (parentheses, aliases, newline, extra) in [
            (false, false, "\n", ""),
            (true, false, "\n", ""),
            (false, true, "\n", ""),
            (false, true, "\r\n", ""),
            (true, true, "\r\n", scripts),
        ] {
            for method in METHODS {
                let expression = expected.render(parentheses, aliases);
                let source = handoff(&expression, declarations, method)
                    .replace("---\n<", &format!("---\n{extra}<"))
                    .replace('\n', newline);
                let facts = analyze(&source).unwrap();
                assert_eq!(facts.boundaries.len(), 1, "case {case}: {source}");
                assert_eq!(
                    statuses(&facts, 0),
                    [S::Supported],
                    "case {case}: {method:?}: {source}"
                );
                expected.check(
                    &facts.boundaries[0].inputs[0].value,
                    &facts,
                    &source,
                    parentheses,
                    aliases,
                );
                check_boundaries(&facts, &source);
            }
        }
    }
}

fn span_in(source: &str, context: &str, text: &str) -> std::ops::Range<u32> {
    let start = source.find(context).expect(context) + context.find(text).expect(text);
    start as u32..(start + text.len()) as u32
}

#[test]
fn unknown_members_keep_static_aggregates_unknown() {
    for method in METHODS {
        for expression in [
            "[1, missing]",
            "{ known: [1], uncertain: missing }",
            "[{ a: 1 }, [...missing]]",
        ] {
            let facts = analyze(&handoff(expression, "", method)).unwrap();
            assert_eq!(statuses(&facts, 0), [S::Unknown]);
        }
        // A const aggregate is mutable through its properties, even inside a fresh literal.
        let facts = analyze(&handoff(
            "[object]",
            "const object = { a: 1 }; object.a = () => 1;",
            method,
        ))
        .unwrap();
        assert_eq!(statuses(&facts, 0), [S::Unknown]);
    }
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
fn reason_spans_identify_exact_nested_values_and_constant_initializers() {
    let expression = "[alias, { bad: () => 1, uncertain: missing }]";
    let declarations = "const 注釈 = '😀';\nconst base = 1n;\nconst alias = base;";
    for method in METHODS {
        let source = handoff(expression, declarations, method).replace('\n', "\r\n");
        let facts = analyze(&source).unwrap();
        let result = facts.assess_transfer(BoundaryId(0)).unwrap();
        let range = |text| span_in(&source, expression, text);
        let expected = vec![
            (R::StaticArray, range(expression)),
            (
                if method == TransferMethod::JsonScript {
                    R::BigIntInJson
                } else {
                    R::BigIntEncoding
                },
                range("alias"),
            ),
            (
                R::ConstantInitializer,
                span_in(&source, "const alias = base;", "base"),
            ),
            (
                R::ConstantInitializer,
                span_in(&source, "const base = 1n;", "1n"),
            ),
            (
                R::PlainObject,
                range("{ bad: () => 1, uncertain: missing }"),
            ),
            (R::FunctionValue, range("() => 1")),
            (R::UnknownValue(U::ExternalReference), range("missing")),
        ];
        assert_eq!(result.inputs[0].status, S::Unsupported);
        assert_eq!(
            result.inputs[0]
                .reasons
                .iter()
                .map(|r| (r.kind, r.span.clone()))
                .collect::<Vec<_>>(),
            expected
        );
        let input = &facts.boundaries[0].inputs[0];
        assert_eq!(
            input
                .references
                .iter()
                .map(|id| facts.references[id.0].span.clone())
                .collect::<Vec<_>>(),
            [range("alias"), range("missing")]
        );
        let V::Array(items) = &input.value.kind else {
            panic!("array")
        };
        for (origin, name, reference) in [
            (&items[0].origins[0], "alias", range("alias")),
            (
                &items[0].origins[1],
                "base",
                span_in(&source, "const alias = base;", "base"),
            ),
        ] {
            assert_eq!(facts.declarations[origin.declaration.0].name, name);
            assert_eq!(facts.references[origin.reference.0].span, reference);
        }
        check_boundaries(&facts, &source);
    }
}

#[test]
fn condition_spans_identify_exact_directives_attributes_and_input_keys() {
    use atlas::ValidationStatus;
    let cases = [
        (
            "<View client:media={23} client:load x={1} x={2} {...others} />",
            vec![
                (
                    C::DirectiveValidation(ValidationStatus::Invalid),
                    "client:media={23}",
                    "client:media={23}",
                ),
                (
                    C::DirectiveValidation(ValidationStatus::RequiresProjectContext),
                    "client:load",
                    "client:load",
                ),
                (
                    C::DuplicateDirective,
                    "<View client:media={23} client:load x={1} x={2} {...others} />",
                    "<View client:media={23} client:load x={1} x={2} {...others} />",
                ),
                (C::DuplicateInput("x".into()), "x={2}", "x"),
                (C::DynamicInputSet, "{...others}", "{...others}"),
            ],
        ),
        (
            "<script define:vars={1}></script>",
            vec![
                (
                    C::DirectiveValidation(ValidationStatus::Invalid),
                    "define:vars={1}",
                    "define:vars={1}",
                ),
                (C::DynamicInputSet, "{1}", "{1}"),
            ],
        ),
        (
            "<script define:vars={{ a: 1 }} type=\"application/json\">raw</script>",
            vec![(
                C::UnanalyzedScript(ScriptMode::Data),
                "<script define:vars={{ a: 1 }} type=\"application/json\">raw</script>",
                "<script define:vars={{ a: 1 }} type=\"application/json\">raw</script>",
            )],
        ),
        (
            "<script define:vars={{ a: 1 }} {...other} type=\"text/javascript\" type=\"module\">a;</script>",
            vec![
                (C::UncertainScriptAttributes, "{...other}", "{...other}"),
                (
                    C::UncertainScriptAttributes,
                    "type=\"module\"",
                    "type=\"module\"",
                ),
            ],
        ),
        (
            "<script define:vars={{ '123key': 1 }}></script>",
            vec![(C::InvalidInjectedName, "'123key'", "'123key'")],
        ),
    ];
    for (element, expected) in cases {
        let source =
            format!("---\r\nimport View from './View';\r\nconst 注釈 = '😀';\r\n---\r\n{element}");
        let facts = analyze(&source).unwrap();
        let result = facts.assess_transfer(BoundaryId(0)).unwrap();
        let opening = &element[..=element.find('>').unwrap()];
        let opening_span = span_in(&source, element, opening);
        let mut expected: Vec<_> = expected
            .into_iter()
            .map(|(kind, context, text)| (kind, span_in(&source, context, text)))
            .collect();
        if element.starts_with("<View") {
            expected.push((C::RendererAvailability, opening_span.clone()));
        }
        expected.extend([
            (C::DataDisclosure, opening_span.clone()),
            (C::CodeMovementSafety, opening_span),
        ]);
        assert_eq!(
            result
                .assumptions
                .iter()
                .map(|c| (c.kind.clone(), c.span.clone()))
                .collect::<Vec<_>>(),
            expected,
            "{element}"
        );
        check_boundaries(&facts, &source);
    }
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
        assert_eq!(file.as_object().unwrap().len(), 7);
        assert!(file["references"].is_array());
        assert!(file.get("boundaries").is_none());
    }
}

#[cfg(feature = "incremental")]
#[test]
fn cached_and_retained_assessments_keep_ids_and_origins_after_update_remove_and_readd() {
    use std::sync::Arc;
    let sources = [
        "---\r\nconst 名前 = '😀';\r\nconst alias = 名前;\r\n---\r\n<script define:vars={{ text: alias }}>text;</script>",
        "---\r\nconst unused = 0;\r\nconst 名前 = 1n;\r\nconst alias = 名前;\r\n---\r\n<script>const alias = false; alias;</script>\r\n<script define:vars={{ pre: 0 }}>pre;</script>\r\n<script define:vars={{ text: alias }}>text;</script>",
        "---\r\nconst unused = 0;\r\nconst another = 1;\r\nconst 名前 = true;\r\nconst alias = 名前;\r\n---\r\n<script define:vars={{ pre: 0 }}>pre;</script>\r\n<script define:vars={{ next: 1 }}>next;</script>\r\n<script define:vars={{ text: alias }}>text;</script>",
    ];
    let mut db = atlas::AnalysisDatabase::new();
    let mut retained = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if index == 2 {
            assert!(db.remove_file("Page.astro"));
            assert!(db.analyze_file("Page.astro").is_none());
        }
        db.set_source("Page.astro", *source);
        let current = db.analyze_file("Page.astro").unwrap();
        assert!(Arc::ptr_eq(
            &current,
            &db.analyze_file("Page.astro").unwrap()
        ));
        let facts = current.as_ref().as_ref().unwrap();
        // Each revision inserts a preceding boundary; obtain the target ID anew.
        let boundary = facts
            .boundaries
            .iter()
            .find(|b| b.inputs[0].name.as_deref() == Some("text"))
            .unwrap();
        assert_eq!(boundary.id, BoundaryId(index));
        let assessment = facts.assess_transfer(boundary.id).unwrap();
        retained.push((current, assessment));
    }
    drop(db);
    for (index, ((result, assessment), source)) in retained.iter().zip(sources).enumerate() {
        let facts = result.as_ref().as_ref().unwrap();
        assert_eq!(facts, &analyze(source).unwrap());
        assert_eq!(
            &facts.assess_transfer(assessment.boundary).unwrap(),
            assessment
        );
        let boundary = &facts.boundaries[assessment.boundary.0];
        let value = &boundary.inputs[0].value;
        let (initializer, expected, status) = match index {
            0 => ("'😀'", V::String("😀".into()), S::Supported),
            1 => ("1n", V::BigInt("1".into()), S::Unsupported),
            _ => ("true", V::Boolean(true), S::Supported),
        };
        assert_eq!(assessment.inputs[0].status, status);
        assert_eq!(value.kind, expected);
        assert_eq!(value.origins.len(), 2);
        assert_eq!(boundary.inputs[0].references, [value.origins[0].reference]);
        assert_eq!(
            facts.references[value.origins[0].reference.0].span,
            span_in(source, "text: alias", "alias")
        );
        assert_eq!(
            facts.references[value.origins[1].reference.0].span,
            span_in(source, "const alias = 名前;", "名前")
        );
        for (origin, name, init) in [
            (&value.origins[0], "alias", "名前"),
            (&value.origins[1], "名前", initializer),
        ] {
            let declaration = &facts.declarations[origin.declaration.0];
            let context = format!("const {name} = {init};");
            assert_eq!(declaration.span, span_in(source, &context, name));
            assert_eq!(origin.initializer.span, span_in(source, &context, init));
        }
        check_boundaries(facts, source);
    }
    for pair in retained.windows(2) {
        assert!(!Arc::ptr_eq(&pair[0].0, &pair[1].0));
        let previous = pair[0].0.as_ref().as_ref().unwrap();
        let current = pair[1].0.as_ref().as_ref().unwrap();
        let old_origin = &previous.boundaries[pair[0].1.boundary.0].inputs[0]
            .value
            .origins[0];
        let new_origin = &current.boundaries[pair[1].1.boundary.0].inputs[0]
            .value
            .origins[0];
        assert_ne!(old_origin.declaration, new_origin.declaration);
        // The numeric reference ID may be reused in another result. Its source
        // evidence must still belong to that result's revision.
        assert_ne!(
            previous.references[old_origin.reference.0].span,
            current.references[new_origin.reference.0].span
        );
    }
}
