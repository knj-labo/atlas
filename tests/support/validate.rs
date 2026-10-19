use super::render::outcome;
use crate::*;
use serde_json::Value;

fn span(source: &str, range: &std::ops::Range<u32>) {
    assert!(
        range.start <= range.end && range.end as usize <= source.len(),
        "invalid span {range:?}"
    );
    assert!(
        source.is_char_boundary(range.start as usize)
            && source.is_char_boundary(range.end as usize),
        "non-UTF-8 span {range:?}"
    );
}
fn spans(source: &str, value: &Value) {
    match value {
        Value::Array(values) => {
            for v in values {
                spans(source, v);
            }
        }
        Value::Object(fields) => {
            for (key, v) in fields {
                if key.ends_with("span") && v.is_array() {
                    let a = v.as_array().unwrap();
                    let r = u32::try_from(a[0].as_u64().unwrap()).unwrap()
                        ..u32::try_from(a[1].as_u64().unwrap()).unwrap();
                    span(source, &r);
                }
                spans(source, v);
            }
            if let (Some(raw), Some(r)) = (fields.get("raw"), fields.get("span")) {
                assert_eq!(
                    raw.as_str().unwrap(),
                    &source[r[0].as_u64().unwrap() as usize..r[1].as_u64().unwrap() as usize],
                    "raw source recovery"
                );
            }
        }
        _ => {}
    }
}
fn parents(values: &[Option<usize>]) {
    for i in 0..values.len() {
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(i);
        while let Some(id) = current {
            assert!(id < values.len(), "parent out of bounds");
            assert!(seen.insert(id), "parent cycle");
            current = values[id];
        }
    }
}
fn origin(f: &FileFacts, o: &InjectionOrigin) {
    for id in &o.references {
        assert!(
            id.0 < f.references.len(),
            "injection reference out of bounds"
        );
    }
}
fn reference(f: &FileFacts, r: &Reference) {
    assert!(r.scope.0 < f.scopes.len(), "reference scope out of bounds");
    match &r.resolution {
        Resolution::Resolved(b) => assert_eq!(
            f.declarations.get(b.id.0),
            Some(b),
            "resolved binding differs from canonical declaration"
        ),
        Resolution::RuntimeDependent { candidates, .. } => {
            for id in candidates {
                assert!(id.0 < f.declarations.len(), "candidate out of bounds");
            }
        }
        _ => {}
    }
}
pub fn validate(source: &str, result: &Result<FileFacts, AnalysisError>) {
    if let Ok(f) = result {
        for (i, b) in f.declarations.iter().enumerate() {
            assert_eq!(b.id.0, i, "declaration id");
            assert!(b.scope.0 < f.scopes.len(), "binding scope");
            assert!(
                f.symbols[b.symbol.0].declarations.contains(&b.id),
                "binding symbol"
            );
            if let Some(o) = &b.injection {
                origin(f, o);
            }
        }
        for (i, symbol) in f.symbols.iter().enumerate() {
            assert_eq!(symbol.id.0, i, "symbol id");
            assert!(!symbol.declarations.is_empty(), "symbol has declarations");
            let mut seen = std::collections::HashSet::new();
            for id in &symbol.declarations {
                assert!(seen.insert(id), "duplicate physical declaration");
                assert_eq!(f.declarations[id.0].symbol, symbol.id, "declaration symbol");
            }
        }
        for import in &f.imports {
            for binding in &import.bindings {
                let declaration = &f.declarations[binding.declaration.0];
                assert_eq!(declaration.span, binding.span, "physical import span");
                assert_eq!(
                    declaration.import.as_ref(),
                    Some(&binding.target),
                    "physical import target"
                );
            }
        }
        for export in &f.exports {
            span(source, &export.declaration_span);
            assert!(
                export.declaration_span.start <= export.span.start
                    && export.span.end <= export.declaration_span.end
            );
            if let Some(source_span) = &export.source_span {
                span(source, source_span);
                assert!(
                    export.declaration_span.start <= source_span.start
                        && source_span.end <= export.declaration_span.end
                );
            }
        }
        for (i, s) in f.scopes.iter().enumerate() {
            assert_eq!(s.id.0, i, "scope id");
        }
        parents(
            &f.scopes
                .iter()
                .map(|s| s.parent.map(|p| p.0))
                .collect::<Vec<_>>(),
        );
        for (i, r) in f.references.iter().enumerate() {
            assert_eq!(r.id.0, i, "reference id");
            reference(f, r);
        }
        for (i, e) in f.elements.iter().enumerate() {
            assert_eq!(e.id.0, i, "element id");
            if let Some(id) = e.reference {
                assert!(id.0 < f.references.len(), "element reference");
            }
        }
        parents(
            &f.elements
                .iter()
                .map(|e| e.parent.map(|p| p.0))
                .collect::<Vec<_>>(),
        );
        for c in &f.components {
            let e = &f.elements[c.element.0];
            assert_eq!(
                e.reference,
                Some(c.reference.id),
                "component element reference"
            );
            assert_eq!(
                f.references.get(c.reference.id.0),
                Some(&c.reference),
                "embedded component reference"
            );
        }
        for s in &f.scripts {
            assert!(s.element.0 < f.elements.len(), "script element");
            for r in &s.references {
                assert_eq!(
                    f.references.get(r.id.0),
                    Some(r),
                    "embedded script reference"
                );
            }
            for id in &s.injected {
                assert_eq!(f.declarations[id.0].kind, DeclarationKind::Injected);
            }
        }
        for (i, s) in f.styles.iter().enumerate() {
            assert_eq!(s.id.0, i, "style id");
            assert!(s.element.0 < f.elements.len(), "style element");
            for injection in &s.injected {
                origin(f, &injection.origin);
            }
            if let Some(css) = &s.css {
                for (i, r) in css.rules.iter().enumerate() {
                    assert_eq!(r.id.0, i, "CSS rule id");
                }
                parents(
                    &css.rules
                        .iter()
                        .map(|r| r.parent.map(|p| p.0))
                        .collect::<Vec<_>>(),
                );
                for (i, d) in css.declarations.iter().enumerate() {
                    assert_eq!(d.id.0, i, "CSS declaration id");
                    if let Some(id) = d.rule {
                        assert!(id.0 < css.rules.len(), "CSS declaration rule");
                    }
                }
                for d in &css.definitions {
                    if let Some(id) = d.declaration {
                        assert!(id.0 < css.declarations.len(), "CSS definition declaration");
                    }
                }
                for r in &css.references {
                    for c in &r.candidates {
                        let d = &f.styles[c.style.0]
                            .css
                            .as_ref()
                            .expect("candidate style CSS")
                            .definitions[c.definition];
                        assert_eq!(d.name, r.name, "CSS candidate name");
                        assert_eq!(
                            d.kind == CssDefinitionKind::Keyframes,
                            r.kind == CssReferenceKind::Animation,
                            "CSS candidate kind"
                        );
                    }
                }
            }
        }
    }
    spans(source, &outcome(result));
}
pub fn check_invalid_graphs() {
    let source = "<script is:inline>const x=1;x;</script>";
    let base = analyze(source).unwrap();
    let mut variants = Vec::new();
    let mut f = base.clone();
    f.declarations[0].id = DeclarationId(999);
    variants.push(f);
    let mut f = base.clone();
    f.scopes[0].parent = Some(ScopeId(0));
    variants.push(f);
    let mut f = base.clone();
    f.elements[0].parent = Some(ElementId(0));
    variants.push(f);
    let mut f = base.clone();
    f.scopes[0].parent = Some(ScopeId(999));
    variants.push(f);
    let mut f = base.clone();
    f.references[0].scope = ScopeId(999);
    variants.push(f);
    let mut f = base.clone();
    f.scripts[0].references[0].name = "wrong".into();
    variants.push(f);
    let mut f = base.clone();
    if let Resolution::Resolved(b) = &mut f.references[0].resolution {
        b.name = "wrong".into();
    }
    variants.push(f);
    let mut f = base;
    f.scripts[0].content.raw = "wrong".into();
    variants.push(f);
    for f in variants {
        assert!(
            std::panic::catch_unwind(|| validate(source, &Ok(f))).is_err(),
            "invalid graph must be rejected"
        );
    }
    let css_source = "<style>@keyframes spin{}.a{animation-name:spin}</style>";
    let css = analyze(css_source).unwrap();
    validate(css_source, &Ok(css.clone()));
    let mut f = css.clone();
    f.styles[0].css.as_mut().unwrap().references[0].candidates[0].definition = 999;
    assert!(std::panic::catch_unwind(|| validate(css_source, &Ok(f))).is_err());
    let mut f = css;
    f.styles[0].css.as_mut().unwrap().rules[0].parent = Some(CssRuleId(0));
    assert!(std::panic::catch_unwind(|| validate(css_source, &Ok(f))).is_err());
    assert!(std::panic::catch_unwind(|| spans("😀", &serde_json::json!({"span":[1,4]}))).is_err());
}
