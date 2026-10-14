#[path = "expect.rs"]
mod expect;
#[path = "registry.rs"]
mod registry;
#[path = "render.rs"]
mod render;
#[path = "validate.rs"]
mod validate;

#[test]
fn fixture_snapshots() {
    let cases = registry::cases();
    let mut analyzed = Vec::new();
    let mut incorrect = Vec::new();
    for case in &cases {
        let source = registry::source(case);
        let result = crate::analyze(&source);
        if std::panic::catch_unwind(|| {
            expect::check(case, &result);
            validate::validate(&source, &result);
        })
        .is_err()
        {
            incorrect.push(case["id"].as_str().unwrap());
        }
        analyzed.push((case, result));
    }
    assert!(
        incorrect.is_empty(),
        "correctness assertions failed: {incorrect:?}"
    );
    let mut failures = Vec::new();
    for (case, result) in analyzed {
        let id = case["id"].as_str().unwrap();
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/snapshots")
                .join(case["snapshot_group"].as_str().unwrap()),
        );
        settings.set_prepend_module_to_snapshot(false);
        let _guard = settings.bind_to_scope();
        if std::panic::catch_unwind(|| {
            insta::assert_snapshot!(
                format!("fixture_snapshots__{id}"),
                render::snapshot(&result)
            )
        })
        .is_err()
        {
            failures.push(id);
        }
    }
    assert!(
        failures.is_empty(),
        "snapshots require review: {failures:?}"
    );
}
fn regression(checks: &[&str]) {
    let cases = registry::cases();
    for check in checks {
        assert!(
            cases
                .iter()
                .any(|c| c["atlas"]["check"].as_str() == Some(check)),
            "missing regression family: {check}"
        );
    }
    for case in cases
        .into_iter()
        .filter(|c| checks.contains(&c["atlas"]["check"].as_str().unwrap_or("")))
    {
        let source = registry::source(&case);
        let result = crate::analyze(&source);
        expect::check(&case, &result);
        validate::validate(&source, &result);
    }
}
#[test]
fn bundled_src_bodies() {
    regression(&["processed-src", "external-src", "parser-error"]);
}
#[test]
fn injected_isolation() {
    regression(&["isolation", "empty-return"]);
}
#[test]
fn injected_global_readers() {
    regression(&["reader", "reader-local"]);
}
#[test]
fn inline_style_injection() {
    regression(&["inline-style", "effective-style"]);
}
#[test]
fn animation_priority() {
    regression(&["priority"]);
}
#[test]
fn animation_slots() {
    regression(&["slots"]);
}
#[test]
fn injected_keyword_grammar() {
    regression(&["keyword-sloppy", "keyword-prefix-strict", "keyword-invalid"]);
}
#[test]
fn external_parent_language() {
    regression(&["parent"]);
}
#[test]
fn invalid_graphs_are_rejected() {
    validate::check_invalid_graphs();
}
#[test]
fn fixture_registry_rejects_invalid_catalogs() {
    let base = serde_json::Value::Array(registry::cases());
    let inventory = registry::inventory();
    let mut cases = base.clone();
    let duplicate = cases[0].clone();
    cases.as_array_mut().unwrap().push(duplicate);
    assert!(registry::validate(&cases, &inventory).is_err());
    let mut cases = base.clone();
    cases[1]["file"] = cases[0]["file"].clone();
    assert!(registry::validate(&cases, &inventory).is_err());
    let mut cases = base.clone();
    cases[0]["file"] = serde_json::json!("missing.astro");
    assert!(registry::validate(&cases, &inventory).is_err());
    let mut cases = base;
    cases.as_array_mut().unwrap().pop();
    assert!(registry::validate(&cases, &inventory).is_err());
}

#[test]
fn animation_dynamic_values() {
    regression(&["animation-vars"]);
}
#[test]
fn classic_html_close_comment() {
    regression(&["html-close-comment"]);
}

#[test]
fn css_line_continuations() {
    regression(&["css-continuation"]);
}
#[test]
fn injected_ecmascript_whitespace() {
    regression(&["injected-whitespace"]);
}

#[test]
fn css_parser_boundaries() {
    use crate::{CssCandidate, CssDependencyKind, CssRuleKind, analyze};
    let source = r#"<style>@charset "UTF-8";@import "base.css";.a{--payload:{x:var(--tone);y:"}"};color:red;color:blue!important;a:hover{color:var(--tone)}@media(width>1px){& .b{--tone:green}}}</style>"#;
    let facts = analyze(source).unwrap();
    let css = facts.styles[0].css.as_ref().unwrap();
    assert_eq!(
        css.rules.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(),
        [
            CssRuleKind::AtRule("charset".into()),
            CssRuleKind::AtRule("import".into()),
            CssRuleKind::Style,
            CssRuleKind::Style,
            CssRuleKind::Media,
            CssRuleKind::Style
        ]
    );
    assert_eq!(
        css.declarations
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        ["--payload", "color", "color", "color", "--tone"]
    );
    assert_eq!(css.declarations[0].value.raw, r#"{x:var(--tone);y:"}"}"#);
    assert_eq!(css.declarations[1].value.raw, "red");
    assert_eq!(css.declarations[2].value.raw, "blue");
    assert!(css.declarations[2].important);
    assert_eq!(css.rules[3].parent, Some(css.rules[2].id));
    assert_eq!(css.rules[5].parent, Some(css.rules[4].id));
    assert_eq!(css.rules[5].conditions, [css.rules[4].prelude.clone()]);
    assert_eq!(css.dependencies[0].kind, CssDependencyKind::Import);
    assert_eq!(css.dependencies[0].value, "base.css");
    assert_eq!(css.references.len(), 2);
    for reference in &css.references {
        assert_eq!(
            reference.candidates,
            [CssCandidate {
                style: facts.styles[0].id,
                definition: 1
            }]
        );
    }
    validate::validate(source, &Ok(facts));
}
#[test]
fn css_decoded_names_preserve_original_source() {
    use crate::{CssCandidate, analyze};
    for (source, name) in [
        (
            r#"<div style={"--to\u2028ne:red;color:var(--to\u2028ne)"}/>"#,
            "--to\u{2028}ne",
        ),
        (
            r#"<div style={"--to\u2029ne:red;color:var(--to\u2029ne)"}/>"#,
            "--to\u{2029}ne",
        ),
        (
            r#"<div style="--to&#110;e:red;color:var(--tone)"/>"#,
            "--tone",
        ),
        (
            r#"<div style={"--to\x6ee:red;color:var(--tone)"}/>"#,
            "--tone",
        ),
        (
            r#"<div style={"--to\u{6e}e:red;color:var(--tone)"}/>"#,
            "--tone",
        ),
        (
            r#"<div style="--\74 one:red;color:var(--tone)"/>"#,
            "--tone",
        ),
    ] {
        let facts = analyze(source).unwrap();
        let css = facts.styles[0].css.as_ref().unwrap();
        assert_eq!(css.definitions[0].name, name);
        assert_eq!(css.references[0].name, name);
        assert_eq!(
            css.references[0].candidates,
            [CssCandidate {
                style: facts.styles[0].id,
                definition: 0
            }]
        );
        validate::validate(source, &Ok(facts));
    }
}
