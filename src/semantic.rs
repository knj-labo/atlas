use crate::{facts::*, syntax::range};
use oxc_ast::{AstKind, ast};
use oxc_semantic::{ScopeFlags, Semantic, SymbolId as OxcSymbolId};
use oxc_span::GetSpan;
use oxc_syntax::{node::NodeId, symbol::SymbolFlags};
use std::collections::HashMap;

pub(crate) struct Projection {
    pub symbols: HashMap<OxcSymbolId, SymbolId>,
    pub declaration_nodes: HashMap<DeclarationId, NodeId>,
    pub references: HashMap<(u32, u32), Reference>,
}

pub(crate) fn project(
    semantic: &Semantic<'_>,
    facts: &mut FileFacts,
    domain: ScopeDomain,
    root_span: std::ops::Range<u32>,
    injections: &HashMap<(u32, u32), InjectionOrigin>,
) -> Projection {
    let import_start = facts.imports.len();
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
                imports.insert((local.span.start, local.span.end), target.clone());
                bindings.push(ImportBinding {
                    declaration: DeclarationId(usize::MAX),
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
    let mut declaration_nodes = HashMap::new();
    let mut physical = HashMap::new();
    // Allocate the old representative IDs first, then append every additional
    // physical declaration. Browser projections use the same file-local spaces.
    for id in semantic.scoping().symbol_ids() {
        let symbol = SymbolId(facts.symbols.len());
        symbols.insert(id, symbol);
        let span = semantic.scoping().symbol_span(id);
        let node = semantic.scoping().symbol_declaration(id);
        let redeclaration = semantic
            .scoping()
            .symbol_redeclarations(id)
            .iter()
            .find(|r| r.span == span && r.declaration == node);
        let flags = redeclaration.map_or_else(|| semantic.scoping().symbol_flags(id), |r| r.flags);
        let binding = project_binding(
            semantic,
            symbol,
            id,
            span,
            node,
            flags,
            DeclarationId(facts.declarations.len()),
            &scopes,
            &imports,
            injections,
        );
        physical.insert((span.start, span.end), binding.id);
        declaration_nodes.insert(binding.id, node);
        facts.symbols.push(Symbol {
            id: symbol,
            declarations: vec![binding.id],
        });
        facts.declarations.push(binding);
    }
    for id in semantic.scoping().symbol_ids() {
        let symbol = symbols[&id];
        for r in semantic.scoping().symbol_redeclarations(id) {
            if physical.contains_key(&(r.span.start, r.span.end)) {
                continue;
            }
            let binding = project_binding(
                semantic,
                symbol,
                id,
                r.span,
                r.declaration,
                r.flags,
                DeclarationId(facts.declarations.len()),
                &scopes,
                &imports,
                injections,
            );
            physical.insert((r.span.start, r.span.end), binding.id);
            declaration_nodes.insert(binding.id, r.declaration);
            facts.symbols[symbol.0].declarations.push(binding.id);
            facts.declarations.push(binding);
        }
    }
    for import in &mut facts.imports[import_start..] {
        for binding in &mut import.bindings {
            binding.declaration = physical[&(binding.span.start, binding.span.end)];
        }
    }
    if matches!(domain, ScopeDomain::Server | ScopeDomain::Module) {
        if domain == ScopeDomain::Module {
            implicit_declaration_exports(semantic, facts, &physical);
        }
        for node in semantic.nodes().iter() {
            if !matches!(semantic.nodes().parent_kind(node.id()), AstKind::Program(_)) {
                continue;
            }
            if let AstKind::ExportDefaultDeclaration(export) = node.kind() {
                if domain != ScopeDomain::Module {
                    continue;
                }
                let (target, anonymous_type) = match &export.declaration {
                    ast::ExportDefaultDeclarationKind::FunctionDeclaration(f) => (
                        f.id.as_ref().map(|id| {
                            ExportLocal::Declaration(physical[&(id.span.start, id.span.end)])
                        }),
                        false,
                    ),
                    ast::ExportDefaultDeclarationKind::ClassDeclaration(c) => (
                        c.id.as_ref().map(|id| {
                            ExportLocal::Declaration(physical[&(id.span.start, id.span.end)])
                        }),
                        false,
                    ),
                    ast::ExportDefaultDeclarationKind::TSInterfaceDeclaration(t) => (
                        Some(ExportLocal::Declaration(
                            physical[&(t.id.span.start, t.id.span.end)],
                        )),
                        true,
                    ),
                    kind => (
                        kind.as_expression().and_then(|e| {
                            if let ast::Expression::Identifier(id) = e.get_inner_expression() {
                                id.reference_id
                                    .get()
                                    .and_then(|r| semantic.scoping().get_reference(r).symbol_id())
                                    .map(|id| ExportLocal::Symbol(symbols[&id]))
                            } else {
                                None
                            }
                        }),
                        false,
                    ),
                };
                push_export(
                    facts,
                    domain,
                    "default",
                    range(export.span),
                    range(export.span),
                    None,
                    None,
                    None,
                    target,
                    anonymous_type,
                    target.is_none(),
                );
                continue;
            }
            let (export_kind, source) = match node.kind() {
                AstKind::ExportNamedDeclaration(export) => {
                    (export.export_kind, export.source.as_ref())
                }
                AstKind::ExportAllDeclaration(export) => (export.export_kind, Some(&export.source)),
                _ => continue,
            };
            let type_export = export_kind == ast::ImportOrExportKind::Type;
            let mut push = |name: &str, span, target, imported, type_only| {
                push_export(
                    facts,
                    domain,
                    name,
                    range(span),
                    range(node.kind().span()),
                    source.map(|s| s.value.to_string()),
                    source.map(|s| range(s.span)),
                    imported,
                    target,
                    type_only,
                    false,
                );
            };
            match node.kind() {
                AstKind::ExportAllDeclaration(export) => match &export.exported {
                    Some(exported) => push(
                        &exported.name(),
                        exported.span(),
                        None,
                        Some(ImportSpecifier::Namespace),
                        type_export,
                    ),
                    None => push("*", export.span, None, None, type_export),
                },
                AstKind::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(ast::Declaration::VariableDeclaration(variables)) => {
                        for declarator in &variables.declarations {
                            for id in declarator.id.get_binding_identifiers() {
                                push(
                                    &id.name,
                                    id.span,
                                    Some(ExportLocal::Declaration(
                                        physical[&(id.span.start, id.span.end)],
                                    )),
                                    None,
                                    false,
                                );
                            }
                        }
                    }
                    Some(declaration) => {
                        if let Some(id) = declaration.id() {
                            push(
                                &id.name,
                                id.span,
                                Some(ExportLocal::Declaration(
                                    physical[&(id.span.start, id.span.end)],
                                )),
                                None,
                                false,
                            );
                        }
                    }
                    None => {
                        for specifier in &export.specifiers {
                            let target = match &specifier.local {
                                ast::ModuleExportName::IdentifierReference(r)
                                    if source.is_none() =>
                                {
                                    // A type-only export can expose a runtime
                                    // binding for later `typeof` queries even
                                    // when Oxc's Type reference is unresolved.
                                    semantic
                                        .scoping()
                                        .get_root_binding(r.name)
                                        .map(|id| ExportLocal::Symbol(symbols[&id]))
                                }
                                _ => None,
                            };
                            push(
                                &specifier.exported.name(),
                                specifier.exported.span(),
                                target,
                                source.map(|_| match specifier.local.name().as_str() {
                                    "default" => ImportSpecifier::Default,
                                    name => ImportSpecifier::Named(name.into()),
                                }),
                                type_export
                                    || specifier.export_kind == ast::ImportOrExportKind::Type,
                            );
                        }
                    }
                },
                _ => unreachable!(),
            }
        }
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
            let meaning = reference_meaning(semantic, node.id(), flags);
            let resolution = match actual.symbol_id() {
                Some(id) => facts.symbols[symbols[&id].0]
                    .declaration(facts, meaning)
                    .cloned()
                    .map_or(Resolution::Unresolved, Resolution::Resolved),
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
                meaning,
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
            meaning: Meaning::Value,
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
        declaration_nodes,
        references,
    }
}

#[allow(clippy::too_many_arguments)]
fn project_binding(
    semantic: &Semantic<'_>,
    symbol: SymbolId,
    oxc_symbol: OxcSymbolId,
    span: oxc_span::Span,
    node_id: NodeId,
    flags: SymbolFlags,
    id: DeclarationId,
    scopes: &HashMap<oxc_syntax::scope::ScopeId, ScopeId>,
    imports: &HashMap<(u32, u32), ImportTarget>,
    injections: &HashMap<(u32, u32), InjectionOrigin>,
) -> Binding {
    let node = semantic.nodes().kind(node_id);
    let import = imports.get(&(span.start, span.end)).cloned();
    let injection = injections.get(&(span.start, span.end)).cloned();
    let kind = if injection.is_some() {
        DeclarationKind::Injected
    } else if import.is_some() {
        DeclarationKind::Import
    } else {
        match node {
            AstKind::Function(_) => DeclarationKind::Function,
            AstKind::Class(_) => DeclarationKind::Class,
            AstKind::FormalParameter(_) | AstKind::CatchParameter(_) => DeclarationKind::Parameter,
            AstKind::VariableDeclarator(_) => DeclarationKind::Variable,
            AstKind::TSInterfaceDeclaration(_)
            | AstKind::TSTypeAliasDeclaration(_)
            | AstKind::TSTypeParameter(_) => DeclarationKind::Type,
            _ if flags.is_type() && !flags.is_value() => DeclarationKind::Type,
            _ => DeclarationKind::Other,
        }
    };
    let meanings = [
        Meaning::Value,
        Meaning::Type,
        Meaning::Namespace,
        Meaning::ValueAsType,
    ]
    .into_iter()
    .filter(|meaning| match meaning {
        Meaning::Value => import.as_ref().map_or(flags.is_value(), |i| !i.type_only),
        Meaning::Type => import.is_some() || flags.intersects(SymbolFlags::Type),
        Meaning::Namespace => import.is_some() || flags.intersects(SymbolFlags::Namespace),
        Meaning::ValueAsType => import.is_some() || flags.is_value(),
    })
    .collect();
    Binding {
        id,
        symbol,
        scope: scopes[&semantic.scoping().symbol_scope_id(oxc_symbol)],
        name: semantic.scoping().symbol_name(oxc_symbol).into(),
        span: range(span),
        kind,
        import,
        injection,
        meanings,
        function_implementation: matches!(node, AstKind::Function(f) if f.body.is_some()),
    }
}

fn reference_meaning(
    semantic: &Semantic<'_>,
    id: NodeId,
    flags: oxc_syntax::reference::ReferenceFlags,
) -> Meaning {
    // Oxc normalizes ValueAsType to Type after resolution. Only the expression
    // name of a type query (not its type arguments) has value-as-type meaning.
    let mut child = id;
    let mut qualified = false;
    loop {
        let parent = semantic.nodes().parent_id(child);
        match semantic.nodes().kind(parent) {
            AstKind::TSQualifiedName(_) => {
                qualified = true;
                child = parent;
            }
            AstKind::TSTypeQuery(query)
                if query
                    .expr_name
                    .span()
                    .contains_inclusive(semantic.nodes().kind(id).span()) =>
            {
                return Meaning::ValueAsType;
            }
            _ => break,
        }
    }
    if flags.is_value_as_type() {
        Meaning::ValueAsType
    } else if qualified || flags.is_namespace() {
        Meaning::Namespace
    } else if flags.is_type() && !flags.is_value() {
        Meaning::Type
    } else {
        Meaning::Value
    }
}

#[allow(clippy::too_many_arguments)]
fn push_export(
    facts: &mut FileFacts,
    domain: ScopeDomain,
    name: &str,
    span: std::ops::Range<u32>,
    declaration_span: std::ops::Range<u32>,
    source: Option<String>,
    source_span: Option<std::ops::Range<u32>>,
    imported: Option<ImportSpecifier>,
    target: Option<ExportLocal>,
    type_export: bool,
    default_expression: bool,
) {
    let local = target.map(|target| match target {
        ExportLocal::Declaration(id) => id,
        ExportLocal::Symbol(id) => facts.symbols[id.0].declarations[0],
    });
    let mut meanings: Vec<_> = match target {
        Some(ExportLocal::Declaration(id)) => facts.declarations[id.0].meanings.clone(),
        Some(ExportLocal::Symbol(id)) => [
            Meaning::Value,
            Meaning::Type,
            Meaning::Namespace,
            Meaning::ValueAsType,
        ]
        .into_iter()
        .filter(|m| facts.symbols[id.0].declaration(facts, *m).is_some())
        .collect(),
        None => vec![
            Meaning::Value,
            Meaning::Type,
            Meaning::Namespace,
            Meaning::ValueAsType,
        ],
    };
    if type_export {
        meanings.retain(|m| *m != Meaning::Value);
    }
    let type_only = !meanings.contains(&Meaning::Value);
    facts.exports.push(Export {
        name: name.into(),
        span,
        local,
        source,
        source_span,
        declaration_span,
        imported,
        type_only,
        target,
        meanings,
        default_expression,
        role: match name {
            "getStaticPaths" | "prerender" | "partial"
                if !type_only && domain == ScopeDomain::Server =>
            {
                ExportRole::Astro
            }
            "Props" if type_only && domain == ScopeDomain::Server => ExportRole::Props,
            _ => ExportRole::Other,
        },
    });
}

fn implicit_declaration_exports(
    semantic: &Semantic<'_>,
    facts: &mut FileFacts,
    physical: &HashMap<(u32, u32), DeclarationId>,
) {
    let source_type = semantic.source_type();
    if !source_type.is_typescript_definition() || !source_type.is_module() {
        return;
    }
    let program = semantic.nodes().program();
    if program.body.iter().any(|statement| match statement {
        ast::Statement::ExportNamedDeclaration(export) => export.declaration.is_none(),
        ast::Statement::ExportAllDeclaration(_) | ast::Statement::TSExportAssignment(_) => true,
        ast::Statement::ExportDefaultDeclaration(export) => {
            export.declaration.as_expression().is_some()
        }
        _ => false,
    }) {
        return;
    }
    for statement in &program.body {
        let mut push = |id: &ast::BindingIdentifier<'_>| {
            push_export(
                facts,
                ScopeDomain::Module,
                &id.name,
                range(id.span),
                range(statement.span()),
                None,
                None,
                None,
                Some(ExportLocal::Declaration(
                    physical[&(id.span.start, id.span.end)],
                )),
                false,
                false,
            );
        };
        match statement.as_declaration() {
            Some(ast::Declaration::VariableDeclaration(variables)) => {
                for declarator in &variables.declarations {
                    for id in declarator.id.get_binding_identifiers() {
                        push(id);
                    }
                }
            }
            Some(ast::Declaration::TSImportEqualsDeclaration(_)) => {}
            Some(declaration) => {
                if let Some(id) = declaration.id() {
                    push(id);
                }
            }
            None => {}
        }
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
