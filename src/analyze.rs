use crate::{facts::*, markup::Markup, semantic::project};
use oxc_allocator::Allocator;
use oxc_ast_visit::Visit;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::{SemanticBuilder, SemanticBuilderAstroExt};
use oxc_span::SourceType;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn is_commonjs(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("cjs" | "cts")
    )
}

/// Analyze caller-supplied Astro or ESM JavaScript/TypeScript source, without I/O.
///
/// `.js`, `.mjs`, `.jsx`, `.ts`, `.mts`, `.tsx`, `.d.ts` and `.d.mts` use
/// Oxc's path-based grammar selection. External scopes have domain
/// [`ScopeDomain::Module`], without Astro builtins, validation or export roles.
/// `.cjs` and `.cts` (including `.d.cts`) return an unsupported-source diagnostic.
/// Other virtual path keys retain the Astro grammar of [`analyze`].
pub fn analyze_file(path: impl AsRef<Path>, source: &str) -> Result<FileFacts, AnalysisError> {
    let path = path.as_ref();
    if is_commonjs(path) {
        return Err(AnalysisError {
            diagnostics: vec![Diagnostic {
                phase: DiagnosticPhase::Parser,
                severity: Severity::Error,
                code: Some("atlas/unsupported-source-type".into()),
                message: format!("Unsupported CommonJS source: {}", path.display()),
                labels: Vec::new(),
            }],
            parser_panicked: false,
        });
    }
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("js" | "mjs" | "jsx" | "ts" | "mts" | "tsx")
    ) {
        return analyze(source);
    }
    let source_type = SourceType::from_path(path).map_err(|error| AnalysisError {
        diagnostics: vec![Diagnostic {
            phase: DiagnosticPhase::Parser,
            severity: Severity::Error,
            code: Some("atlas/unsupported-source-type".into()),
            message: format!("Unsupported source path {}: {error}", path.display()),
            labels: Vec::new(),
        }],
        parser_panicked: false,
    })?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse();
    if parsed.panicked || !parsed.errors.is_empty() {
        let mut diagnostics = to_diagnostics(&parsed.errors, DiagnosticPhase::Parser);
        if diagnostics.is_empty() {
            diagnostics.push(Diagnostic {
                phase: DiagnosticPhase::Parser,
                severity: Severity::Error,
                code: None,
                message: "Source parser terminated early".into(),
                labels: Vec::new(),
            });
        }
        return Err(AnalysisError {
            diagnostics,
            parser_panicked: parsed.panicked,
        });
    }
    let result = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if !result.errors.is_empty() {
        return Err(AnalysisError {
            diagnostics: to_diagnostics(&result.errors, DiagnosticPhase::Semantic),
            parser_panicked: false,
        });
    }
    let mut facts = FileFacts::default();
    project(
        &result.semantic,
        &mut facts,
        ScopeDomain::Module,
        0..source.len().try_into().expect("Oxc source length"),
        &HashMap::new(),
    );
    canonicalize(&mut facts);
    Ok(facts)
}

/// Analyze Astro 7.0.6 syntax and return source-positioned owned facts.
/// Syntax errors produce `Err`; Astro-specific validity diagnostics accompany
/// successfully parsed facts. Dynamic values are preserved rather than executed.
pub fn analyze(source: &str) -> Result<FileFacts, AnalysisError> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::astro())
        .with_options(ParseOptions {
            parse_astro_script_content: false,
            parse_regular_expression: true,
            ..ParseOptions::default()
        })
        .parse_astro();
    if parsed.panicked || !parsed.errors.is_empty() {
        let mut diagnostics = to_diagnostics(&parsed.errors, DiagnosticPhase::Parser);
        if diagnostics.is_empty() {
            diagnostics.push(Diagnostic {
                phase: DiagnosticPhase::Parser,
                severity: Severity::Error,
                code: None,
                message: "Astro parser terminated early".into(),
                labels: Vec::new(),
            });
        }
        return Err(AnalysisError {
            diagnostics,
            parser_panicked: parsed.panicked,
        });
    }
    let comments = &parsed
        .root
        .frontmatter
        .as_ref()
        .expect("synthetic frontmatter")
        .program
        .comments;
    let result = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build_astro(&parsed.root, source, comments);
    if !result.errors.is_empty() {
        return Err(AnalysisError {
            diagnostics: to_diagnostics(&result.errors, DiagnosticPhase::Semantic),
            parser_panicked: false,
        });
    }
    let mut facts = FileFacts::default();
    let projection = project(
        &result.semantic,
        &mut facts,
        ScopeDomain::Server,
        0..source.len().try_into().expect("Oxc source length"),
        &HashMap::new(),
    );
    let mut markup = Markup {
        source,
        projection: &projection,
        facts: &mut facts,
        scripts: Vec::new(),
        style_seeds: Vec::new(),
        in_frontmatter: false,
        stack: Vec::new(),
    };
    markup.visit_astro_root(&parsed.root);
    let scripts = std::mem::take(&mut markup.scripts);
    let style_seeds = std::mem::take(&mut markup.style_seeds);
    crate::scripts::analyze_scripts(source, &mut facts, scripts)?;
    for (style, seeds) in style_seeds {
        facts.styles[style].injected = seeds
            .into_iter()
            .map(|seed| CssInjection {
                name: format!("--{}", seed.name),
                origin: seed.origin,
            })
            .collect();
    }
    crate::css::analyze_styles(source, &mut facts)?;
    crate::validate::validate(&mut facts);
    crate::boundary::collect(
        source,
        &parsed.root,
        &result.semantic,
        &projection,
        &mut facts,
    );
    canonicalize(&mut facts);
    Ok(facts)
}
pub(crate) fn to_diagnostics(errors: &[OxcDiagnostic], phase: DiagnosticPhase) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| Diagnostic {
            phase,
            severity: Severity::Error,
            code: None,
            message: error.message.to_string(),
            labels: error
                .labels
                .iter()
                .flatten()
                .map(|label| {
                    let start = u32::try_from(label.offset()).expect("Oxc offset");
                    DiagnosticLabel {
                        span: start..start + u32::try_from(label.len()).expect("Oxc length"),
                        message: label.label().map(str::to_owned),
                    }
                })
                .collect(),
        })
        .collect()
}
fn canonicalize(facts: &mut FileFacts) {
    canonicalize_declarations(facts);
    facts.imports.sort_by_key(|i| i.span.start);
    facts.exports.sort_by_key(|e| (e.span.start, e.span.end));
    facts.components.sort_by_key(|c| c.span.start);
    facts.references.sort_by_key(|r| (r.span.start, r.span.end));
    let ids: HashMap<_, _> = facts
        .references
        .iter()
        .enumerate()
        .map(|(index, r)| (r.id, ReferenceId(index)))
        .collect();
    for r in &mut facts.references {
        r.id = ids[&r.id];
        remap_resolution(&mut r.resolution, &ids);
    }
    for e in &mut facts.elements {
        e.reference = e.reference.map(|id| ids[&id]);
    }
    for c in &mut facts.components {
        c.reference.id = ids[&c.reference.id];
        remap_resolution(&mut c.reference.resolution, &ids);
    }
    for s in &mut facts.scripts {
        for r in &mut s.references {
            r.id = ids[&r.id];
            remap_resolution(&mut r.resolution, &ids);
        }
    }
    for b in &mut facts.declarations {
        if let Some(origin) = &mut b.injection {
            for id in &mut origin.references {
                *id = ids[id];
            }
        }
    }
    for s in &mut facts.styles {
        for injected in &mut s.injected {
            for id in &mut injected.origin.references {
                *id = ids[id];
            }
        }
    }
    crate::boundary::remap_references(&mut facts.boundaries, &ids);
    facts.scripts.sort_by_key(|s| s.span.start);
    facts
        .builtin_uses
        .sort_by_key(|u| (u.span.start, u.span.end));
    facts
        .diagnostics
        .sort_by_key(|d| d.labels.first().map_or(0, |l| l.span.start));
}

fn canonicalize_declarations(facts: &mut FileFacts) {
    if facts.symbols.len() == facts.declarations.len() {
        return;
    }
    let primary: std::collections::HashSet<_> =
        facts.symbols.iter().map(|s| s.declarations[0]).collect();
    // Keep the representative ID order from every server/browser projection,
    // then allocate the newly exposed redeclarations at the end of the file.
    facts
        .declarations
        .sort_by_key(|b| (!primary.contains(&b.id), b.id));
    let ids: HashMap<_, _> = facts
        .declarations
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, DeclarationId(i)))
        .collect();
    for binding in &mut facts.declarations {
        binding.id = ids[&binding.id];
    }
    for symbol in &mut facts.symbols {
        for id in &mut symbol.declarations {
            *id = ids[id];
        }
    }
    for import in &mut facts.imports {
        for binding in &mut import.bindings {
            binding.declaration = ids[&binding.declaration];
        }
    }
    for export in &mut facts.exports {
        export.local = export.local.map(|id| ids[&id]);
        if let Some(ExportLocal::Declaration(id)) = &mut export.target {
            *id = ids[id];
        }
    }
    fn resolution(value: &mut Resolution, ids: &HashMap<DeclarationId, DeclarationId>) {
        match value {
            Resolution::Resolved(binding) => binding.id = ids[&binding.id],
            Resolution::RuntimeDependent { candidates, .. } => {
                for id in candidates {
                    *id = ids[id];
                }
            }
            _ => {}
        }
    }
    for reference in &mut facts.references {
        resolution(&mut reference.resolution, &ids);
    }
    for component in &mut facts.components {
        resolution(&mut component.reference.resolution, &ids);
    }
    for script in &mut facts.scripts {
        for id in &mut script.injected {
            *id = ids[id];
        }
        for reference in &mut script.references {
            resolution(&mut reference.resolution, &ids);
        }
    }
    fn remap_value(value: &mut crate::TransferValue, ids: &HashMap<DeclarationId, DeclarationId>) {
        for origin in &mut value.origins {
            origin.declaration = ids[&origin.declaration];
        }
        match &mut value.kind {
            crate::TransferValueKind::Array(values) => {
                for item in values {
                    remap_value(item, ids);
                }
            }
            crate::TransferValueKind::Object(properties) => {
                for (_, item) in properties {
                    remap_value(item, ids);
                }
            }
            _ => {}
        }
    }
    for boundary in &mut facts.boundaries {
        for input in &mut boundary.inputs {
            remap_value(&mut input.value, &ids);
        }
    }
}
fn remap_resolution(resolution: &mut Resolution, ids: &HashMap<ReferenceId, ReferenceId>) {
    if let Resolution::Resolved(b) = resolution
        && let Some(origin) = &mut b.injection
    {
        for id in &mut origin.references {
            *id = ids[id];
        }
    }
}
