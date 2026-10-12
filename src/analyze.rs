use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast};
use oxc_ast_visit::Visit;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder, SemanticBuilderAstroExt, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};

use crate::facts::*;

/// Analyze one Astro file and release its AST before returning owned facts.
///
/// Parser or semantic syntax diagnostics produce an error without partial facts.
/// Unresolved identifiers remain successful facts with `Resolution::Unresolved`.
/// Imports are described syntactically; modules and runtime values are not evaluated.
pub fn analyze(source: &str) -> Result<FileFacts, AnalysisError> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::astro()).parse_astro();
    if parsed.panicked || !parsed.errors.is_empty() {
        let mut diagnostics = own_diagnostics(&parsed.errors, DiagnosticPhase::Parser);
        if diagnostics.is_empty() {
            diagnostics.push(Diagnostic {
                phase: DiagnosticPhase::Parser,
                message: "Astro parser terminated before producing a complete AST".into(),
                labels: Vec::new(),
            });
        }
        return Err(AnalysisError {
            diagnostics,
            parser_panicked: parsed.panicked,
        });
    }

    // The pinned parser always supplies a frontmatter Program, synthesizing an
    // empty one for inputs without frontmatter. All allocations remain local.
    let comments = &parsed
        .root
        .frontmatter
        .as_ref()
        .expect("parser supplies frontmatter")
        .program
        .comments;
    let built = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build_astro(&parsed.root, source, comments);
    if !built.errors.is_empty() {
        return Err(AnalysisError {
            diagnostics: own_diagnostics(&built.errors, DiagnosticPhase::Semantic),
            parser_panicked: false,
        });
    }

    // Join imports to references by SymbolId while the semantic model is alive.
    // Project only declaration spans and owned strings into the public result.
    let mut imports = ImportCollector::default();
    imports.visit_astro_root(&parsed.root);
    let mut collector = FactCollector {
        source,
        semantic: &built.semantic,
        import_targets: &imports.targets,
        facts: FileFacts {
            imports: imports.imports,
            ..FileFacts::default()
        },
        in_frontmatter: false,
        script_stack: Vec::new(),
    };
    collector.visit_astro_root(&parsed.root);
    collector
        .facts
        .imports
        .sort_by_key(|import| import.span.start);
    collector
        .facts
        .components
        .sort_by_key(|component| component.span.start);
    collector
        .facts
        .scripts
        .sort_by_key(|script| script.span.start);
    for script in &mut collector.facts.scripts {
        script
            .references
            .sort_by_key(|reference| reference.span.start);
    }
    Ok(collector.facts)
}

fn own_diagnostics(errors: &[OxcDiagnostic], phase: DiagnosticPhase) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|error| Diagnostic {
            phase,
            message: error.message.to_string(),
            labels: error
                .labels
                .iter()
                .flatten()
                .map(|label| {
                    let start = u32::try_from(label.offset()).expect("Oxc source offsets fit u32");
                    let length = u32::try_from(label.len()).expect("Oxc spans fit u32");
                    DiagnosticLabel {
                        span: start..start + length,
                        message: label.label().map(str::to_owned),
                    }
                })
                .collect(),
        })
        .collect()
}

fn range(span: Span) -> std::ops::Range<u32> {
    span.start..span.end
}

#[derive(Default)]
struct ImportCollector {
    imports: Vec<Import>,
    targets: HashMap<SymbolId, ImportTarget>,
}

impl<'a> Visit<'a> for ImportCollector {
    fn visit_import_declaration(&mut self, declaration: &ast::ImportDeclaration<'a>) {
        let mut bindings = Vec::new();
        for specifier in declaration.specifiers.iter().flatten() {
            let (local, specifier, type_only) = match specifier {
                ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    (&specifier.local, ImportSpecifier::Default, false)
                }
                ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    (&specifier.local, ImportSpecifier::Namespace, false)
                }
                ast::ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                    &specifier.local,
                    ImportSpecifier::Named(specifier.imported.name().to_string()),
                    specifier.import_kind == ast::ImportOrExportKind::Type,
                ),
            };
            let target = ImportTarget {
                source: declaration.source.value.to_string(),
                specifier,
                type_only: type_only || declaration.import_kind == ast::ImportOrExportKind::Type,
            };
            if let Some(symbol_id) = local.symbol_id.get() {
                self.targets.insert(symbol_id, target.clone());
            }
            bindings.push(ImportBinding {
                name: local.name.to_string(),
                span: range(local.span),
                target,
            });
        }
        self.imports.push(Import {
            span: range(declaration.span),
            source: declaration.source.value.to_string(),
            source_span: range(declaration.source.span),
            bindings,
        });
    }
}

struct FactCollector<'s, 'm, 'a> {
    source: &'s str,
    semantic: &'m Semantic<'a>,
    import_targets: &'m HashMap<SymbolId, ImportTarget>,
    facts: FileFacts,
    in_frontmatter: bool,
    script_stack: Vec<usize>,
}

impl FactCollector<'_, '_, '_> {
    fn reference(&self, reference: &ast::IdentifierReference<'_>) -> Reference {
        let reference_id = reference
            .reference_id
            .get()
            .expect("semantic analysis assigns reference IDs");
        let resolution = match self
            .semantic
            .scoping()
            .get_reference(reference_id)
            .symbol_id()
        {
            Some(symbol_id) => Resolution::Resolved(Binding {
                name: self.semantic.scoping().symbol_name(symbol_id).to_owned(),
                span: range(self.semantic.scoping().symbol_span(symbol_id)),
                import: self.import_targets.get(&symbol_id).cloned(),
            }),
            None => Resolution::Unresolved,
        };
        Reference {
            name: reference.name.to_string(),
            span: range(reference.span),
            resolution,
        }
    }
}

impl<'a> Visit<'a> for FactCollector<'_, '_, 'a> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::AstroFrontmatter(_) => self.in_frontmatter = true,
            AstKind::AstroScript(script) => {
                self.script_stack.push(self.facts.scripts.len());
                self.facts.scripts.push(Script {
                    span: range(script.span),
                    references: Vec::new(),
                });
            }
            AstKind::IdentifierReference(reference) if !self.script_stack.is_empty() => {
                let reference = self.reference(reference);
                self.facts.scripts[*self.script_stack.last().unwrap()]
                    .references
                    .push(reference);
            }
            AstKind::JSXElement(element)
                if !self.in_frontmatter && self.script_stack.is_empty() =>
            {
                let root = match &element.opening_element.name {
                    ast::JSXElementName::IdentifierReference(reference) => Some(&**reference),
                    ast::JSXElementName::MemberExpression(member) => member_root(&member.object),
                    _ => None,
                };
                if let Some(reference) = root {
                    self.facts.components.push(ComponentUse {
                        name: element
                            .opening_element
                            .name
                            .span()
                            .source_text(self.source)
                            .to_owned(),
                        span: range(element.span),
                        name_span: range(element.opening_element.name.span()),
                        closing_name_span: element
                            .closing_element
                            .as_ref()
                            .map(|closing| range(closing.name.span())),
                        reference: self.reference(reference),
                        directives: element
                            .opening_element
                            .attributes
                            .iter()
                            .filter_map(|attribute| match attribute {
                                ast::JSXAttributeItem::Attribute(attribute) => {
                                    let name = match &attribute.name {
                                        ast::JSXAttributeName::Identifier(name) => {
                                            name.name.to_string()
                                        }
                                        ast::JSXAttributeName::NamespacedName(name) => {
                                            format!("{}:{}", name.namespace.name, name.name.name)
                                        }
                                    };
                                    if !name.contains(':') {
                                        return None;
                                    }
                                    let value = attribute.value.as_ref().map(|value| {
                                        let kind = match value {
                                            ast::JSXAttributeValue::StringLiteral(_) => {
                                                DirectiveValueKind::String
                                            }
                                            ast::JSXAttributeValue::ExpressionContainer(_) => {
                                                DirectiveValueKind::Expression
                                            }
                                            ast::JSXAttributeValue::Element(_) => {
                                                DirectiveValueKind::Element
                                            }
                                            ast::JSXAttributeValue::Fragment(_) => {
                                                DirectiveValueKind::Fragment
                                            }
                                        };
                                        let span = value.span();
                                        DirectiveValue {
                                            raw: span.source_text(self.source).to_owned(),
                                            span: range(span),
                                            kind,
                                        }
                                    });
                                    Some(Directive {
                                        name,
                                        span: range(attribute.span),
                                        name_span: range(attribute.name.span()),
                                        value,
                                    })
                                }
                                ast::JSXAttributeItem::SpreadAttribute(_) => None,
                            })
                            .collect(),
                    });
                }
            }
            _ => {}
        }
    }

    fn leave_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::AstroFrontmatter(_) => self.in_frontmatter = false,
            AstKind::AstroScript(_) => {
                self.script_stack.pop();
            }
            _ => {}
        }
    }
}

fn member_root<'a, 'b>(
    object: &'b ast::JSXMemberExpressionObject<'a>,
) -> Option<&'b ast::IdentifierReference<'a>> {
    match object {
        ast::JSXMemberExpressionObject::IdentifierReference(reference) => Some(reference),
        ast::JSXMemberExpressionObject::MemberExpression(member) => member_root(&member.object),
        ast::JSXMemberExpressionObject::ThisExpression(_) => None,
    }
}
