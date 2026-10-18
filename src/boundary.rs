use crate::{facts::*, semantic::Projection, syntax::*, transfer::*};
use oxc_ast::{AstKind, ast};
use oxc_ast_visit::Visit;
use oxc_semantic::Semantic;
use oxc_span::{GetSpan, Span};
use oxc_syntax::{number::ToJsString, operator::UnaryOperator};
use std::collections::{HashMap, HashSet};

pub(crate) fn collect<'a>(
    source: &str,
    root: &ast::AstroRoot<'a>,
    semantic: &Semantic<'a>,
    projection: &Projection,
    facts: &mut FileFacts,
) {
    let mut collector = Collector {
        values: Values {
            source,
            semantic,
            projection,
        },
        elements: facts
            .elements
            .iter()
            .map(|e| ((e.span.start, e.span.end), e.id))
            .collect(),
        facts,
        boundaries: Vec::new(),
    };
    collector.visit_astro_root(root);
    facts.boundaries = collector.boundaries;
}

struct Collector<'s, 'a, 'f> {
    values: Values<'s, 'a, 'f>,
    elements: HashMap<(u32, u32), ElementId>,
    facts: &'f FileFacts,
    boundaries: Vec<BoundaryFact>,
}

impl<'a> Visit<'a> for Collector<'_, 'a, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        let AstKind::JSXElement(node) = kind else {
            return;
        };
        let Some(id) = self.elements.get(&(node.span.start, node.span.end)) else {
            return;
        };
        let element = &self.facts.elements[id.0];
        let script = element.kind == ElementKind::Script;
        let directives: Vec<_> = element
            .directives
            .iter()
            .filter(|d| {
                if script {
                    d.name == "define:vars"
                } else {
                    known_client(&d.name)
                }
            })
            .cloned()
            .collect();
        if directives.is_empty()
            || (!script
                && !matches!(
                    element.kind,
                    ElementKind::Component | ElementKind::Dynamic | ElementKind::SelfComponent
                ))
        {
            return;
        }
        let mut boundary = BoundaryFact {
            id: BoundaryId(self.boundaries.len()),
            element: element.id,
            kind: if script {
                BoundaryKind::ScriptDefineVars
            } else {
                BoundaryKind::ClientProps
            },
            method: if script {
                TransferMethod::JsonScript
            } else {
                TransferMethod::AstroProps
            },
            span: element.opening_span.clone(),
            evaluation: ScopeDomain::Server,
            destination: Some(ScopeDomain::BrowserModule),
            directives,
            inputs: Vec::new(),
            conditions: Vec::new(),
        };
        for directive in &boundary.directives {
            if directive.status != ValidationStatus::Valid {
                boundary.conditions.push(TransferCondition {
                    kind: TransferConditionKind::DirectiveValidation(directive.status),
                    span: directive.span.clone(),
                });
            }
        }
        // Multiple hydration directives (including a custom one) leave the
        // actual handoff uncertain. Never infer the custom directive's contract.
        let directive_count = element
            .directives
            .iter()
            .filter(|d| {
                if script {
                    d.name == "define:vars"
                } else {
                    d.name.starts_with("client:")
                }
            })
            .count();
        if directive_count > 1 {
            boundary.conditions.push(TransferCondition {
                kind: TransferConditionKind::DuplicateDirective,
                span: boundary.span.clone(),
            });
        }
        if script {
            self.script_inputs(node, &mut boundary);
            let script = self
                .facts
                .scripts
                .iter()
                .find(|s| s.element == element.id)
                .unwrap();
            boundary.destination = match script.mode {
                ScriptMode::Processed | ScriptMode::InlineModule => {
                    Some(ScopeDomain::BrowserModule)
                }
                ScriptMode::InlineClassic => Some(ScopeDomain::InjectedScript),
                _ => {
                    boundary.conditions.push(TransferCondition {
                        kind: TransferConditionKind::UnanalyzedScript(script.mode),
                        span: script.span.clone(),
                    });
                    None
                }
            };
            let mut attribute_names = HashSet::new();
            for attribute in &element.attributes {
                if attribute.spread
                    || attribute
                        .name
                        .as_ref()
                        .is_some_and(|name| !attribute_names.insert(name))
                {
                    boundary.destination = None;
                    boundary.conditions.push(TransferCondition {
                        kind: TransferConditionKind::UncertainScriptAttributes,
                        span: attribute.span.clone(),
                    });
                }
            }
            for diagnostic in &self.facts.diagnostics {
                if diagnostic.code.as_deref() == Some("astro/invalid-injected-name") {
                    for label in &diagnostic.labels {
                        if element.opening_span.start <= label.span.start
                            && label.span.end <= element.opening_span.end
                        {
                            boundary.conditions.push(TransferCondition {
                                kind: TransferConditionKind::InvalidInjectedName,
                                span: label.span.clone(),
                            });
                        }
                    }
                }
            }
        } else {
            for attribute in &node.opening_element.attributes {
                match attribute {
                    ast::JSXAttributeItem::Attribute(attribute) => {
                        let name = attribute_name(&attribute.name);
                        if name.contains(':') || name == "slot" {
                            continue;
                        }
                        let value = self.values.attribute(attribute);
                        boundary.inputs.push(self.input(
                            Some(name),
                            Some(attribute.name.span()),
                            value,
                        ));
                    }
                    ast::JSXAttributeItem::SpreadAttribute(spread) => {
                        boundary.inputs.push(
                            self.input(
                                None,
                                None,
                                self.values
                                    .unknown(spread.span, TransferUnknownReason::Spread),
                            ),
                        );
                    }
                }
            }
        }
        let mut names = HashSet::new();
        for input in &boundary.inputs {
            if let Some(name) = &input.name {
                if !names.insert(name) {
                    boundary.conditions.push(TransferCondition {
                        kind: TransferConditionKind::DuplicateInput(name.clone()),
                        span: input
                            .key_span
                            .clone()
                            .unwrap_or_else(|| input.value.source.span.clone()),
                    });
                }
            } else {
                boundary.conditions.push(TransferCondition {
                    kind: TransferConditionKind::DynamicInputSet,
                    span: input.value.source.span.clone(),
                });
            }
        }
        self.boundaries.push(boundary);
    }
}

fn known_client(name: &str) -> bool {
    matches!(
        name,
        "client:load" | "client:idle" | "client:visible" | "client:media" | "client:only"
    )
}

// JS property names use Number::toString, whose exponent thresholds and
// shortest decimal representation differ from Rust's f64 Display. Keep the
// legacy Literal projection unchanged while detecting duplicates accurately.
fn transfer_key(property: &ast::ObjectProperty<'_>) -> Option<String> {
    if property.computed {
        if let Some(ast::Expression::NumericLiteral(n)) = property
            .key
            .as_expression()
            .map(ast::Expression::without_parentheses)
        {
            return Some(n.value.to_js_string());
        }
    } else if let ast::PropertyKey::NumericLiteral(n) = &property.key {
        return Some(n.value.to_js_string());
    }
    property_key(property)
}

impl Collector<'_, '_, '_> {
    fn input(
        &self,
        name: Option<String>,
        key: Option<Span>,
        value: TransferValue,
    ) -> BoundaryInput {
        let span = &value.source.span;
        let mut references: Vec<_> = self
            .values
            .projection
            .references
            .values()
            .filter(|r| span.start <= r.span.start && r.span.end <= span.end)
            .collect();
        references.sort_by_key(|r| (r.span.start, r.span.end, r.id));
        BoundaryInput {
            name,
            key_span: key.map(range),
            references: references.iter().map(|r| r.id).collect(),
            value,
        }
    }

    fn script_inputs(&self, node: &ast::JSXElement<'_>, boundary: &mut BoundaryFact) {
        for attribute in &node.opening_element.attributes {
            let ast::JSXAttributeItem::Attribute(attribute) = attribute else {
                continue;
            };
            if attribute_name(&attribute.name) != "define:vars" {
                continue;
            }
            let expression = match &attribute.value {
                Some(ast::JSXAttributeValue::ExpressionContainer(c)) => {
                    c.expression.as_expression()
                }
                _ => None,
            };
            if let Some(ast::Expression::ObjectExpression(object)) =
                expression.map(ast::Expression::without_parentheses)
            {
                for property in &object.properties {
                    let (name, key, value) = match property {
                        ast::ObjectPropertyKind::ObjectProperty(p) => {
                            let name = transfer_key(p);
                            let value = if name.is_none() {
                                self.values
                                    .unknown(p.span, TransferUnknownReason::DynamicKey)
                            } else if name.as_deref() == Some("__proto__")
                                && !p.computed
                                && !p.shorthand
                            {
                                self.values
                                    .unknown(p.span, TransferUnknownReason::NonPlainObject)
                            } else if p.kind != ast::PropertyKind::Init {
                                self.values.unknown(p.span, TransferUnknownReason::Accessor)
                            } else {
                                self.values.expression(&p.value, &mut HashSet::new(), 0)
                            };
                            (name, Some(p.key.span()), value)
                        }
                        ast::ObjectPropertyKind::SpreadProperty(p) => (
                            None,
                            None,
                            self.values.unknown(p.span, TransferUnknownReason::Spread),
                        ),
                    };
                    boundary.inputs.push(self.input(name, key, value));
                }
            } else {
                let span = attribute
                    .value
                    .as_ref()
                    .map_or(attribute.span, GetSpan::span);
                // The expression denotes the input object, not a single value.
                // Even a supported primitive cannot establish its entries.
                boundary.inputs.push(self.input(
                    None,
                    None,
                    self.values.unknown(span, TransferUnknownReason::InputSet),
                ));
            }
        }
    }
}

struct Values<'s, 'a, 'f> {
    source: &'s str,
    semantic: &'f Semantic<'a>,
    projection: &'f Projection,
}

impl Values<'_, '_, '_> {
    fn value(&self, span: Span, kind: TransferValueKind) -> TransferValue {
        TransferValue {
            source: source_text(self.source, span),
            kind,
            origins: Vec::new(),
        }
    }

    fn unknown(&self, span: Span, reason: TransferUnknownReason) -> TransferValue {
        self.value(span, TransferValueKind::Unknown(reason))
    }

    fn attribute(&self, attribute: &ast::JSXAttribute<'_>) -> TransferValue {
        match &attribute.value {
            None => self.value(attribute.span, TransferValueKind::Boolean(true)),
            Some(ast::JSXAttributeValue::StringLiteral(s)) => self.value(
                s.span,
                TransferValueKind::String(html_escape::decode_html_entities(&s.value).into_owned()),
            ),
            Some(ast::JSXAttributeValue::ExpressionContainer(c)) => {
                c.expression.as_expression().map_or_else(
                    || self.unknown(c.span, TransferUnknownReason::UnsupportedExpression),
                    |e| self.expression(e, &mut HashSet::new(), 0),
                )
            }
            Some(value) => self.unknown(value.span(), TransferUnknownReason::UnsupportedExpression),
        }
    }

    fn expression(
        &self,
        expression: &ast::Expression<'_>,
        active: &mut HashSet<DeclarationId>,
        depth: usize,
    ) -> TransferValue {
        use TransferUnknownReason as U;
        use TransferValueKind as V;
        let span = expression.span();
        if depth >= 128 {
            return self.unknown(span, U::AnalysisLimit);
        }
        let kind = match expression.without_parentheses() {
            ast::Expression::NullLiteral(_) => V::Null,
            ast::Expression::StringLiteral(s) => V::String(s.value.to_string()),
            ast::Expression::BooleanLiteral(b) => V::Boolean(b.value),
            ast::Expression::NumericLiteral(n) if n.value.is_finite() => {
                V::FiniteNumber(n.value.to_string())
            }
            ast::Expression::NumericLiteral(_) => V::Unknown(U::NonFiniteNumber),
            ast::Expression::BigIntLiteral(b) => V::BigInt(b.value.to_string()),
            ast::Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                let Some(Literal::String(value)) = literal(expression) else {
                    unreachable!()
                };
                V::String(value)
            }
            ast::Expression::UnaryExpression(u) => {
                match (u.operator, u.argument.without_parentheses()) {
                    (
                        UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus,
                        ast::Expression::NumericLiteral(n),
                    ) if n.value.is_finite() => {
                        V::FiniteNumber(if u.operator == UnaryOperator::UnaryNegation {
                            format!("-{}", n.value)
                        } else {
                            n.value.to_string()
                        })
                    }
                    (UnaryOperator::UnaryNegation, ast::Expression::BigIntLiteral(b)) => {
                        V::BigInt(format!("-{}", b.value))
                    }
                    _ => V::Unknown(U::UnsupportedExpression),
                }
            }
            ast::Expression::FunctionExpression(_)
            | ast::Expression::ArrowFunctionExpression(_) => V::Function,
            ast::Expression::Identifier(identifier) => {
                let mut value = self.identifier(identifier, active, depth + 1);
                value.source = source_text(self.source, span);
                return value;
            }
            ast::Expression::ArrayExpression(array) => V::Array(
                array
                    .elements
                    .iter()
                    .map(|element| match element {
                        ast::ArrayExpressionElement::SpreadElement(s) => {
                            self.unknown(s.span, U::Spread)
                        }
                        ast::ArrayExpressionElement::Elision(s) => {
                            self.unknown(s.span, U::ArrayHole)
                        }
                        _ => self.expression(
                            element.as_expression().expect("array expression"),
                            active,
                            depth + 1,
                        ),
                    })
                    .collect(),
            ),
            ast::Expression::ObjectExpression(object) => {
                let mut properties = Vec::new();
                let mut names = HashSet::new();
                for property in &object.properties {
                    let ast::ObjectPropertyKind::ObjectProperty(p) = property else {
                        return self.unknown(span, U::Spread);
                    };
                    let Some(name) = transfer_key(p) else {
                        return self.unknown(span, U::DynamicKey);
                    };
                    if !names.insert(name.clone()) {
                        return self.unknown(span, U::DuplicateKey);
                    }
                    if name == "__proto__" && !p.computed && !p.shorthand {
                        return self.unknown(span, U::NonPlainObject);
                    }
                    if p.kind != ast::PropertyKind::Init {
                        return self.unknown(span, U::Accessor);
                    }
                    properties.push((name, self.expression(&p.value, active, depth + 1)));
                }
                V::Object(properties)
            }
            ast::Expression::CallExpression(_) | ast::Expression::NewExpression(_) => {
                V::Unknown(U::CallResult)
            }
            _ => V::Unknown(U::UnsupportedExpression),
        };
        self.value(span, kind)
    }

    fn identifier(
        &self,
        identifier: &ast::IdentifierReference<'_>,
        active: &mut HashSet<DeclarationId>,
        depth: usize,
    ) -> TransferValue {
        use TransferUnknownReason as U;
        use TransferValueKind as V;
        let span = identifier.span;
        let Some(reference) = self.projection.references.get(&(span.start, span.end)) else {
            return self.unknown(span, U::ExternalReference);
        };
        let Resolution::Resolved(binding) = &reference.resolution else {
            return self.unknown(span, U::ExternalReference);
        };
        if binding.import.is_some() {
            return self.unknown(span, U::ExternalReference);
        }
        let symbol = identifier
            .reference_id
            .get()
            .and_then(|r| self.semantic.scoping().get_reference(r).symbol_id())
            .expect("resolved identifier");
        let AstKind::VariableDeclarator(declarator) =
            self.semantic.symbol_declaration(symbol).kind()
        else {
            return self.unknown(span, U::UntrackedBinding);
        };
        if declarator.kind != ast::VariableDeclarationKind::Const
            || self
                .semantic
                .scoping()
                .get_resolved_references(symbol)
                .any(|r| r.is_write())
        {
            return self.unknown(span, U::MutableBinding);
        }
        if !matches!(&declarator.id, ast::BindingPattern::BindingIdentifier(_)) {
            return self.unknown(span, U::UntrackedBinding);
        }
        let Some(initializer) = &declarator.init else {
            return self.unknown(span, U::UncertainInitialization);
        };
        if !active.insert(binding.id) {
            return self.unknown(span, U::CyclicAlias);
        }
        let mut value = match initializer.without_parentheses() {
            ast::Expression::ArrayExpression(_) | ast::Expression::ObjectExpression(_) => {
                self.unknown(span, U::AggregateBinding)
            }
            ast::Expression::FunctionExpression(_)
            | ast::Expression::ArrowFunctionExpression(_) => {
                self.unknown(span, U::NonPrimitiveBinding)
            }
            _ => self.expression(initializer, active, depth),
        };
        active.remove(&binding.id);
        // Forward references can be in the temporal dead zone. Lexical name
        // resolution alone does not prove that initialization has occurred.
        if initializer.span().end > span.start && !matches!(value.kind, V::Unknown(U::CyclicAlias))
        {
            value.kind = V::Unknown(U::UncertainInitialization);
        }
        value.origins.insert(
            0,
            ConstantOrigin {
                declaration: binding.id,
                reference: reference.id,
                initializer: source_text(self.source, initializer.span()),
            },
        );
        value.source = source_text(self.source, span);
        value
    }
}

pub(crate) fn remap_references(
    boundaries: &mut [BoundaryFact],
    ids: &HashMap<ReferenceId, ReferenceId>,
) {
    fn remap_value(value: &mut TransferValue, ids: &HashMap<ReferenceId, ReferenceId>) {
        for origin in &mut value.origins {
            origin.reference = ids[&origin.reference]
        }
        match &mut value.kind {
            TransferValueKind::Array(values) => values.iter_mut().for_each(|v| remap_value(v, ids)),
            TransferValueKind::Object(properties) => {
                properties.iter_mut().for_each(|(_, v)| remap_value(v, ids))
            }
            _ => {}
        }
    }
    for boundary in boundaries {
        for input in &mut boundary.inputs {
            for reference in &mut input.references {
                *reference = ids[reference]
            }
            remap_value(&mut input.value, ids);
        }
    }
}
