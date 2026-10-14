use crate::{facts::*, markup::ScriptInput, semantic::project, syntax::diagnostic};
use oxc_allocator::Allocator;
use oxc_ast::{AstBuilder, ast};
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::{SourceType, Span};
use oxc_syntax::{identifier::is_white_space, line_terminator::is_line_terminator};
use std::collections::HashMap;

pub(crate) fn analyze_scripts(
    source: &str,
    facts: &mut FileFacts,
    inputs: Vec<ScriptInput>,
) -> Result<(), AnalysisError> {
    for input in inputs {
        let script = &facts.scripts[input.index];
        if matches!(
            script.mode,
            ScriptMode::External | ScriptMode::Data | ScriptMode::Dynamic
        ) {
            continue;
        }
        let allocator = Allocator::default();
        let content_span = script.content.span.clone();
        let mode = script.mode;
        let source_type = match mode {
            ScriptMode::Processed => SourceType::tsx(),
            ScriptMode::InlineModule => SourceType::mjs(),
            _ => SourceType::script(),
        };
        let suppress_directives =
            input.has_define_vars && !input.seeds.is_empty() && mode == ScriptMode::InlineClassic;
        let mut padding = " ".repeat(content_span.start as usize);
        if suppress_directives {
            padding.replace_range(padding.len() - 3.., "0;\n");
        }
        let padded = allocator.alloc_str(&format!(
            "{}{}",
            padding,
            &source[content_span.start as usize..content_span.end as usize]
        ));
        let mut parsed = Parser::new(&allocator, padded, source_type)
            .with_options(ParseOptions {
                parse_regular_expression: true,
                allow_return_outside_function: input.has_define_vars
                    && mode == ScriptMode::InlineClassic,
                ..ParseOptions::default()
            })
            .parse();
        if parsed.panicked || !parsed.errors.is_empty() {
            return Err(AnalysisError {
                diagnostics: crate::analyze::to_diagnostics(
                    &parsed.errors,
                    DiagnosticPhase::Parser,
                ),
                parser_panicked: parsed.panicked,
            });
        }
        if suppress_directives {
            parsed.program.body.remove(0);
        }
        let ast = AstBuilder::new(&allocator);
        let mut injections = HashMap::new();
        let mut statements = ast.vec();
        for seed in &input.seeds {
            let key = seed
                .name
                .trim_matches(|ch| is_white_space(ch) || is_line_terminator(ch));
            let mut name = String::new();
            let mut previous_word = false;
            for (index, ch) in key.char_indices() {
                let word = ch.is_ascii_alphanumeric() || ch == '_';
                if word {
                    name.push(if index != 0 && !previous_word {
                        ch.to_ascii_uppercase()
                    } else {
                        ch
                    });
                }
                previous_word = word;
            }
            let validation_allocator = Allocator::default();
            let code = format!("const {name} = null;");
            let validation = Parser::new(&validation_allocator, &code, source_type).parse();
            let valid = !validation.panicked
                && validation.errors.is_empty()
                && SemanticBuilder::new()
                    .with_check_syntax_error(true)
                    .build(&validation.program)
                    .errors
                    .is_empty();
            if !valid {
                facts.diagnostics.push(diagnostic(
                    DiagnosticPhase::Astro,
                    Severity::Error,
                    "astro/invalid-injected-name",
                    format!(
                        "define:vars key {:?} produces an invalid JavaScript identifier",
                        seed.name
                    ),
                    seed.origin.key_span.clone(),
                ));
                continue;
            }
            let span = Span::new(seed.origin.key_span.start, seed.origin.key_span.end);
            let binding = ast.binding_pattern_binding_identifier(span, allocator.alloc_str(&name));
            let declarator = ast.variable_declarator(
                span,
                ast::VariableDeclarationKind::Const,
                binding,
                None::<ast::TSTypeAnnotation>,
                Some(ast.expression_null_literal(span)),
                false,
            );
            statements.push(ast::Statement::VariableDeclaration(
                ast.alloc_variable_declaration(
                    span,
                    ast::VariableDeclarationKind::Const,
                    ast.vec1(declarator),
                    false,
                ),
            ));
            injections.insert((span.start, span.end), seed.origin.clone());
        }
        statements.append(&mut parsed.program.body);
        parsed.program.body = statements;
        let result = SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        if !result.errors.is_empty() {
            return Err(AnalysisError {
                diagnostics: crate::analyze::to_diagnostics(
                    &result.errors,
                    DiagnosticPhase::Semantic,
                ),
                parser_panicked: false,
            });
        }
        let domain = match mode {
            ScriptMode::Processed | ScriptMode::InlineModule => ScopeDomain::BrowserModule,
            _ if input.has_define_vars => ScopeDomain::InjectedScript,
            _ => ScopeDomain::BrowserClassic,
        };
        let projection = project(&result.semantic, facts, domain, content_span, &injections);
        let script = &mut facts.scripts[input.index];
        script.injected = projection
            .symbols
            .values()
            .filter(|b| b.kind == DeclarationKind::Injected)
            .map(|b| b.id)
            .collect();
        script.injected.sort();
        script.references = projection.references.values().cloned().collect();
        if script.dynamic_injection {
            for reference in &mut script.references {
                if reference.resolution == Resolution::Unresolved {
                    reference.resolution = Resolution::RuntimeDependent {
                        reason: "define:vars has computed keys or spreads".into(),
                        candidates: Vec::new(),
                    };
                    facts.references[reference.id.0] = reference.clone();
                }
            }
        }
        script.references.sort_by_key(|r| r.span.start);
    }
    let globals: Vec<_> = facts
        .declarations
        .iter()
        .filter(|b| {
            let s = &facts.scopes[b.scope.0];
            s.domain == ScopeDomain::BrowserClassic && s.parent.is_none()
        })
        .cloned()
        .collect();
    for script in &mut facts.scripts {
        if script.mode != ScriptMode::InlineClassic {
            continue;
        }
        for reference in &mut script.references {
            if reference.resolution != Resolution::Unresolved
                && !(script.dynamic_injection
                    && matches!(reference.resolution, Resolution::RuntimeDependent { .. }))
            {
                continue;
            }
            let candidates = globals
                .iter()
                .filter(|b| {
                    b.name == reference.name
                        && !(script.span.start <= b.span.start && b.span.end <= script.span.end)
                })
                .map(|b| b.id)
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                reference.resolution = Resolution::RuntimeDependent {
                    reason: if script.dynamic_injection { "define:vars has computed keys or spreads; another classic script may supply a browser global" } else { "another classic script may supply a browser global" }.into(),
                    candidates,
                };
                facts.references[reference.id.0] = reference.clone();
            }
        }
    }
    Ok(())
}
