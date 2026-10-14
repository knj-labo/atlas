use crate::{facts::*, syntax::range};
use oxc_ast::{AstKind, ast};
use oxc_semantic::{ScopeFlags, Semantic, SymbolId};
use oxc_span::GetSpan;
use std::collections::HashMap;

pub(crate) struct Projection {
    pub symbols: HashMap<SymbolId, Binding>,
    pub references: HashMap<(u32, u32), Reference>,
}

pub(crate) fn project(
    semantic: &Semantic<'_>,
    facts: &mut FileFacts,
    domain: ScopeDomain,
    root_span: std::ops::Range<u32>,
    injections: &HashMap<(u32, u32), InjectionOrigin>,
) -> Projection {
    let mut imports = HashMap::new();
    for node in semantic.nodes().iter() {
        if let AstKind::ImportDeclaration(declaration) = node.kind() {
            let mut bindings = Vec::new();
            for specifier in declaration.specifiers.iter().flatten() {
                let (local, specifier, type_only) = match specifier {
                    ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        (&s.local, ImportSpecifier::Default, false)
                    }
                    ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        (&s.local, ImportSpecifier::Namespace, false)
                    }
                    ast::ImportDeclarationSpecifier::ImportSpecifier(s) => (
                        &s.local,
                        ImportSpecifier::Named(s.imported.name().to_string()),
                        s.import_kind == ast::ImportOrExportKind::Type,
                    ),
                };
                let target = ImportTarget {
                    source: declaration.source.value.to_string(),
                    specifier,
                    type_only: type_only
                        || declaration.import_kind == ast::ImportOrExportKind::Type,
                };
                if let Some(id) = local.symbol_id.get() {
                    imports.insert(id, target.clone());
                }
                bindings.push(ImportBinding {
                    name: local.name.to_string(),
                    span: range(local.span),
                    target,
                });
            }
            facts.imports.push(Import {
                span: range(declaration.span),
                source: declaration.source.value.to_string(),
                source_span: range(declaration.source.span),
                bindings,
            });
        }
    }
    let mut scopes = HashMap::new();
    let scope_ids: Vec<_> = semantic
        .scoping()
        .scope_descendants_from_root()
        .filter(|id| {
            !matches!(
                semantic
                    .nodes()
                    .get_node(semantic.scoping().get_node_id(*id))
                    .kind(),
                AstKind::AstroScript(_)
            )
        })
        .collect();
    for id in &scope_ids {
        let node = semantic
            .nodes()
            .get_node(semantic.scoping().get_node_id(*id));
        let flags = semantic.scoping().scope_flags(*id);
        let own = ScopeId(facts.scopes.len());
        scopes.insert(*id, own);
        let is_root = semantic.scoping().scope_parent_id(*id).is_none();
        let kind = if is_root {
            ScopeKind::Root
        } else if flags.contains(ScopeFlags::Function) {
            ScopeKind::Function
        } else if flags.intersects(ScopeFlags::TsConditional | ScopeFlags::TsModuleBlock)
            || matches!(
                node.kind(),
                AstKind::TSTypeAliasDeclaration(_) | AstKind::TSInterfaceDeclaration(_)
            )
        {
            ScopeKind::Type
        } else if matches!(node.kind(), AstKind::Class(_)) {
            ScopeKind::Class
        } else {
            ScopeKind::Block
        };
        facts.scopes.push(Scope {
            id: own,
            parent: None,
            span: if is_root {
                root_span.clone()
            } else {
                range(node.kind().span())
            },
            domain,
            kind,
        });
    }
    for id in &scope_ids {
        facts.scopes[scopes[id].0].parent = semantic
            .scoping()
            .scope_parent_id(*id)
            .and_then(|parent| scopes.get(&parent).copied());
    }
    let mut symbols = HashMap::new();
    for id in semantic.scoping().symbol_ids() {
        let span = semantic.scoping().symbol_span(id);
        let scope = scopes[&semantic.scoping().symbol_scope_id(id)];
        let flags = semantic.scoping().symbol_flags(id);
        let node = semantic.symbol_declaration(id).kind();
        let injection = injections.get(&(span.start, span.end)).cloned();
        let kind = if injection.is_some() {
            DeclarationKind::Injected
        } else if flags.is_import() {
            DeclarationKind::Import
        } else if flags.is_type() && !flags.is_value() {
            DeclarationKind::Type
        } else {
            match node {
                AstKind::Function(_) => DeclarationKind::Function,
                AstKind::Class(_) => DeclarationKind::Class,
                AstKind::FormalParameter(_) | AstKind::CatchParameter(_) => {
                    DeclarationKind::Parameter
                }
                AstKind::VariableDeclarator(_) => DeclarationKind::Variable,
                _ => DeclarationKind::Other,
            }
        };
        let binding = Binding {
            id: DeclarationId(facts.declarations.len()),
            scope,
            name: semantic.scoping().symbol_name(id).into(),
            span: range(span),
            kind,
            import: imports.get(&id).cloned(),
            injection,
        };
        facts.declarations.push(binding.clone());
        symbols.insert(id, binding);
    }
    let mut references = HashMap::new();
    for node in semantic.nodes().iter() {
        if let AstKind::IdentifierReference(identifier) = node.kind() {
            let actual = semantic
                .scoping()
                .get_reference(identifier.reference_id.get().expect("analyzed reference"));
            let flags = actual.flags();
            let kind = match (
                flags.is_value() || flags.is_value_as_type(),
                flags.is_type(),
            ) {
                (true, true) => ReferenceKind::ValueAndType,
                (false, true) => ReferenceKind::Type,
                _ => ReferenceKind::Value,
            };
            let resolution = match actual.symbol_id() {
                Some(id) => Resolution::Resolved(symbols[&id].clone()),
                None if domain == ScopeDomain::Server && identifier.name == "Astro" => {
                    Resolution::Builtin(Builtin::Astro)
                }
                None if domain == ScopeDomain::Server && identifier.name == "Fragment" => {
                    Resolution::Builtin(Builtin::Fragment)
                }
                None => Resolution::Unresolved,
            };
            let reference = Reference {
                id: ReferenceId(facts.references.len()),
                name: identifier.name.to_string(),
                span: range(identifier.span),
                scope: scopes[&node.scope_id()],
                kind,
                resolution,
            };
            if let Resolution::Builtin(builtin) = reference.resolution {
                facts.builtin_uses.push(BuiltinUse {
                    builtin,
                    member: None,
                    span: reference.span.clone(),
                });
            }
            references.insert(
                (identifier.span.start, identifier.span.end),
                reference.clone(),
            );
            facts.references.push(reference);
        }
    }
    for node in semantic.nodes().iter() {
        let (name, span) = match node.kind() {
            AstKind::ThisExpression(receiver) => ("this", receiver.span),
            AstKind::Super(receiver) => ("super", receiver.span),
            _ => continue,
        };
        let reference = Reference {
            id: ReferenceId(facts.references.len()),
            name: name.into(),
            span: range(span),
            scope: scopes[&node.scope_id()],
            kind: ReferenceKind::Receiver,
            resolution: Resolution::RuntimeDependent {
                reason: "receiver depends on the calling context".into(),
                candidates: Vec::new(),
            },
        };
        references.insert((span.start, span.end), reference.clone());
        facts.references.push(reference);
    }
    if domain == ScopeDomain::Server {
        for node in semantic.nodes().iter() {
            if let AstKind::StaticMemberExpression(member) = node.kind()
                && let Some((root, path)) = access_path(&member.object)
                && matches!(
                    references
                        .get(&(root.span.start, root.span.end))
                        .map(|r| &r.resolution),
                    Some(Resolution::Builtin(Builtin::Astro))
                )
            {
                let path = if path.is_empty() {
                    member.property.name.to_string()
                } else {
                    format!("{path}.{}", member.property.name)
                };
                facts.builtin_uses.push(BuiltinUse {
                    builtin: Builtin::Astro,
                    member: Some(path),
                    span: range(member.span),
                });
            }
        }
    }
    Projection {
        symbols,
        references,
    }
}
fn access_path<'a, 'b>(
    expression: &'b ast::Expression<'a>,
) -> Option<(&'b ast::IdentifierReference<'a>, String)> {
    match expression {
        ast::Expression::Identifier(id) => Some((id, String::new())),
        ast::Expression::StaticMemberExpression(member) => {
            let (id, path) = access_path(&member.object)?;
            Some((
                id,
                if path.is_empty() {
                    member.property.name.to_string()
                } else {
                    format!("{path}.{}", member.property.name)
                },
            ))
        }
        _ => None,
    }
}
