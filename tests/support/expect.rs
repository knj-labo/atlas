use crate::*;
use serde_json::Value;

fn binding_in<'a>(f: &'a FileFacts, script: usize, name: &str) -> &'a Binding {
    let s = &f.scripts[script];
    f.declarations
        .iter()
        .find(|b| b.name == name && s.span.start <= b.span.start && b.span.end <= s.span.end)
        .unwrap()
}
fn resolved(r: &Reference, b: &Binding) {
    assert!(
        matches!(&r.resolution,Resolution::Resolved(target) if target.id==b.id),
        "expected identity {:?}, got {:?}",
        b.id,
        r.resolution
    );
}
pub fn check(case: &Value, result: &Result<FileFacts, AnalysisError>) {
    assert_eq!(
        result.is_err(),
        case["atlas"]["error"].as_bool().unwrap(),
        "{}: {result:?}",
        case["id"]
    );
    let check = case["atlas"]["check"].as_str().unwrap_or("");
    let f = match result {
        Ok(f) => f,
        Err(e) => {
            assert!(!e.diagnostics.is_empty());
            let phase = match check {
                "parser-error" => Some(DiagnosticPhase::Parser),
                "css-error" => Some(DiagnosticPhase::Css),
                "semantic-error" => Some(DiagnosticPhase::Semantic),
                _ => None,
            };
            if let Some(phase) = phase {
                assert!(
                    e.diagnostics
                        .iter()
                        .all(|d| d.phase == phase && d.severity == Severity::Error)
                );
            }
            return;
        }
    };
    let modes: Vec<_> = f.scripts.iter().map(|s| format!("{:?}", s.mode)).collect();
    assert_eq!(
        serde_json::json!(modes),
        case["atlas"]["modes"],
        "{}: modes",
        case["id"]
    );
    match check {
        "css-continuation" => {
            let css = f.styles[0].css.as_ref().unwrap();
            assert_eq!(css.declarations[0].name, "--tone");
            assert_eq!(css.definitions[0].name, "--tone");
            let r = &css.references[0];
            assert_eq!(r.name, "--tone");
            assert_eq!(
                r.candidates,
                [CssCandidate {
                    style: f.styles[0].id,
                    definition: 0
                }]
            );
            let attribute = &f.elements[0].attributes[0];
            assert_eq!(
                attribute.literal,
                Some(Literal::String("--tone:red;color:var(--tone)".into()))
            );
            assert_eq!(
                css.declarations[0].name_span.end - css.declarations[0].name_span.start,
                10
            );
        }
        "injected-whitespace" => {
            assert!(f.diagnostics.is_empty());
            assert_eq!(f.scripts[0].injected.len(), 2);
            let source = super::registry::source(case);
            for (name, key, value) in [("Name", "\u{85}name", "1"), ("name", "\u{feff}name", "2")] {
                let b = binding_in(f, 0, name);
                assert_eq!(b.kind, DeclarationKind::Injected);
                let origin = b.injection.as_ref().unwrap();
                assert_eq!(origin.value.raw, value);
                assert_eq!(
                    &source[origin.key_span.start as usize..origin.key_span.end as usize],
                    format!("\"{key}\"")
                );
                resolved(
                    f.scripts[0]
                        .references
                        .iter()
                        .find(|r| r.name == name)
                        .unwrap(),
                    b,
                );
            }
        }
        "processed-src" => {
            let s = &f.scripts[0];
            assert!(s.bundled);
            assert_eq!(s.language, ScriptLanguage::TypeScript);
            let b = binding_in(f, 0, "x");
            assert_eq!(b.kind, DeclarationKind::Variable);
            assert_eq!(s.references.len(), 1);
            resolved(&s.references[0], b);
        }
        "external-src" => {
            assert!(f.scripts[0].bundled);
            assert!(f.scripts[0].references.is_empty());
            assert!(!f.declarations.iter().any(|b| b.name == "x"));
        }
        "isolation" => {
            let b = binding_in(f, 0, "hidden");
            assert_eq!(f.scopes[b.scope.0].domain, ScopeDomain::InjectedScript);
            resolved(
                f.scripts[0]
                    .references
                    .iter()
                    .find(|r| r.name == "hidden")
                    .unwrap(),
                b,
            );
            assert_eq!(
                f.scripts[1].references[0].resolution,
                Resolution::Unresolved
            );
        }
        "reader" => {
            let b = binding_in(f, 0, "shared");
            assert_eq!(f.scopes[b.scope.0].domain, ScopeDomain::BrowserClassic);
            let r = &f.scripts[1].references[0];
            assert!(
                matches!(&r.resolution,Resolution::RuntimeDependent{candidates,..} if candidates==&[b.id]),
                "expected supplier identity, got {:?}",
                r.resolution
            );
        }
        "reader-local" => {
            let b = binding_in(f, 1, "shared");
            assert_eq!(b.kind, DeclarationKind::Injected);
            assert_ne!(b.id, binding_in(f, 0, "shared").id);
            resolved(&f.scripts[1].references[0], b);
        }
        "empty-return" => {
            assert!(
                f.scopes
                    .iter()
                    .any(|s| s.domain == ScopeDomain::InjectedScript)
            );
        }
        "parent" | "inline-style" => {
            if check == "parent" {
                assert_eq!(f.scripts[0].mode, ScriptMode::External);
                assert_eq!(f.scripts[0].language, ScriptLanguage::JavaScript);
                assert!(!f.scripts[0].bundled);
            }
            let s = &f.styles[0];
            assert_eq!(s.mode, StyleMode::Inline);
            assert!(s.injected.is_empty());
            assert!(
                s.css
                    .as_ref()
                    .unwrap()
                    .definitions
                    .iter()
                    .all(|d| d.kind != CssDefinitionKind::Injected)
            );
            assert!(s.css.as_ref().unwrap().references[0].candidates.is_empty());
            assert!(
                f.elements[s.element.0]
                    .attributes
                    .iter()
                    .any(|a| a.name.as_deref() == Some("define:vars"))
            );
        }
        "effective-style" => {
            let s = &f.styles[0];
            assert_eq!(s.injected.len(), 1);
            let css = s.css.as_ref().unwrap();
            let c = &css.references[0].candidates[0];
            assert_eq!(
                f.styles[c.style.0].css.as_ref().unwrap().definitions[c.definition].kind,
                CssDefinitionKind::Injected
            );
        }
        "priority" | "slots" => {
            let css = f.styles[0].css.as_ref().unwrap();
            let refs: Vec<_> = css
                .references
                .iter()
                .filter(|r| r.kind == CssReferenceKind::Animation)
                .collect();
            let expected = if check == "priority" {
                vec!["spin", "important", "none"]
            } else {
                vec![
                    "linear",
                    "reverse",
                    "both",
                    "paused",
                    "infinite",
                    "ease",
                    "alternate",
                    "forwards",
                    "running",
                    "linear",
                    "linear",
                    "--timeline",
                    "none",
                ]
            };
            assert_eq!(
                refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                serde_json::json!(
                    refs.iter()
                        .map(|r| [r.span.start, r.span.end])
                        .collect::<Vec<_>>()
                ),
                case["atlas"]["animationSpans"],
                "animation token positions"
            );
            for r in refs {
                if r.name == "none" && check == "slots" {
                    assert!(r.candidates.is_empty());
                    continue;
                }
                assert_eq!(r.candidates.len(), 1);
                let c = &r.candidates[0];
                let d = &f.styles[c.style.0].css.as_ref().unwrap().definitions[c.definition];
                assert_eq!(d.kind, CssDefinitionKind::Keyframes);
                assert_eq!(d.name, r.name);
            }
            if check == "priority" {
                assert!(css.declarations.iter().all(|d| d.important));
            }
        }
        "animation-vars" => {
            let css = f.styles[0].css.as_ref().unwrap();
            let refs: Vec<_> = css
                .references
                .iter()
                .filter(|r| r.kind == CssReferenceKind::Animation)
                .collect();
            assert_eq!(
                refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
                ["spin", "bounce", "linear", "linear", "spin", "none", "spin"]
            );
            assert_eq!(
                serde_json::json!(
                    refs.iter()
                        .map(|r| [r.span.start, r.span.end])
                        .collect::<Vec<_>>()
                ),
                case["atlas"]["animationSpans"]
            );
            for r in refs {
                assert!(r.runtime_dependent);
                assert_eq!(r.candidates.len(), 1);
                let c = &r.candidates[0];
                let d = &f.styles[c.style.0].css.as_ref().unwrap().definitions[c.definition];
                assert_eq!(d.name, r.name);
                assert_eq!(d.kind, CssDefinitionKind::Keyframes);
            }
            assert_eq!(
                css.references
                    .iter()
                    .filter(|r| r.kind == CssReferenceKind::CustomProperty)
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>(),
                ["--duration", "--ease", "--duration", "--name", "--ease"]
            );
        }
        "html-close-comment" => {
            assert!(f.diagnostics.is_empty());
            assert_eq!(f.scripts[0].injected.len(), 1);
            assert_eq!(f.scripts[0].references.len(), 2);
            for r in &f.scripts[0].references {
                let b = binding_in(f, 0, &r.name);
                resolved(r, b);
                assert_eq!(f.scopes[b.scope.0].domain, ScopeDomain::InjectedScript);
            }
        }
        "keyword-sloppy" | "keyword-prefix-strict" => {
            assert!(
                f.diagnostics.is_empty(),
                "unexpected keyword diagnostics: {:?}",
                f.diagnostics
            );
            for r in &f.scripts[0].references {
                let b = binding_in(f, 0, &r.name);
                assert_eq!(b.kind, DeclarationKind::Injected);
                resolved(r, b);
            }
            assert_eq!(
                f.scripts[0].injected.len(),
                if check == "keyword-sloppy" { 3 } else { 1 }
            );
        }
        "keyword-invalid" => {
            let expected = if f.scripts[0].mode == ScriptMode::InlineModule {
                3
            } else {
                1
            };
            assert_eq!(f.diagnostics.len(), expected);
            assert!(
                f.diagnostics
                    .iter()
                    .all(|d| d.code.as_deref() == Some("astro/invalid-injected-name")
                        && d.severity == Severity::Error)
            );
            assert!(f.scripts[0].injected.is_empty());
        }
        _ => {}
    }
}
