use crate::facts::*;
use oxc_ast::ast;
use oxc_span::{GetSpan, Span};

pub(crate) fn range(span: Span) -> std::ops::Range<u32> {
    span.start..span.end
}
pub(crate) fn source_text(source: &str, span: Span) -> SourceText {
    SourceText {
        span: range(span),
        raw: span.source_text(source).to_owned(),
    }
}
pub(crate) fn attribute_name(name: &ast::JSXAttributeName<'_>) -> String {
    match name {
        ast::JSXAttributeName::Identifier(n) => n.name.to_string(),
        ast::JSXAttributeName::NamespacedName(n) => format!("{}:{}", n.namespace.name, n.name.name),
    }
}
pub(crate) fn attributes(source: &str, items: &[ast::JSXAttributeItem<'_>]) -> Vec<Attribute> {
    items
        .iter()
        .map(|item| match item {
            ast::JSXAttributeItem::Attribute(attr) => Attribute {
                name: Some(attribute_name(&attr.name)),
                span: range(attr.span),
                name_span: Some(range(attr.name.span())),
                value: attr.value.as_ref().map(|value| DirectiveValue {
                    raw: value.span().source_text(source).to_owned(),
                    span: range(value.span()),
                    kind: match value {
                        ast::JSXAttributeValue::StringLiteral(_) => DirectiveValueKind::String,
                        ast::JSXAttributeValue::ExpressionContainer(_) => {
                            DirectiveValueKind::Expression
                        }
                        ast::JSXAttributeValue::Element(_) => DirectiveValueKind::Element,
                        ast::JSXAttributeValue::Fragment(_) => DirectiveValueKind::Fragment,
                    },
                }),
                literal: match &attr.value {
                    None => Some(Literal::Boolean(true)),
                    Some(ast::JSXAttributeValue::StringLiteral(s)) => Some(Literal::String(
                        html_escape::decode_html_entities(&s.value).into_owned(),
                    )),
                    Some(ast::JSXAttributeValue::ExpressionContainer(c)) => {
                        c.expression.as_expression().and_then(literal)
                    }
                    _ => None,
                },
                spread: false,
            },
            ast::JSXAttributeItem::SpreadAttribute(spread) => Attribute {
                name: None,
                span: range(spread.span),
                name_span: None,
                value: Some(DirectiveValue {
                    raw: spread.span.source_text(source).into(),
                    span: range(spread.span),
                    kind: DirectiveValueKind::Expression,
                }),
                literal: None,
                spread: true,
            },
        })
        .collect()
}
pub(crate) fn literal(expression: &ast::Expression<'_>) -> Option<Literal> {
    match expression.without_parentheses() {
        ast::Expression::StringLiteral(s) => Some(Literal::String(s.value.to_string())),
        ast::Expression::BooleanLiteral(b) => Some(Literal::Boolean(b.value)),
        ast::Expression::NullLiteral(_) => Some(Literal::Null),
        ast::Expression::NumericLiteral(n) => Some(Literal::Number(n.value.to_string())),
        ast::Expression::TemplateLiteral(t) if t.expressions.is_empty() => Some(Literal::String(
            t.quasis
                .iter()
                .map(|q| {
                    q.value
                        .cooked
                        .as_ref()
                        .map_or(q.value.raw.as_str(), |s| s.as_str())
                })
                .collect(),
        )),
        ast::Expression::ArrayExpression(a) => Some(Literal::Array(
            a.elements
                .iter()
                .map(|e| e.as_expression().and_then(literal))
                .collect::<Option<Vec<_>>>()?,
        )),
        ast::Expression::ObjectExpression(o) => Some(Literal::Object(
            o.properties
                .iter()
                .map(|p| match p {
                    ast::ObjectPropertyKind::ObjectProperty(p) if !p.method => {
                        Some((property_key(p)?, literal(&p.value)?))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => None,
    }
}
pub(crate) fn property_key(property: &ast::ObjectProperty<'_>) -> Option<String> {
    if property.computed {
        return match property.key.as_expression().and_then(literal) {
            Some(Literal::String(s) | Literal::Number(s)) => Some(s),
            _ => None,
        };
    }
    match &property.key {
        ast::PropertyKey::StaticIdentifier(_)
        | ast::PropertyKey::StringLiteral(_)
        | ast::PropertyKey::NumericLiteral(_) => {
            property.key.static_name().map(|name| name.into_owned())
        }
        _ => None,
    }
}
pub(crate) fn member_root(object: &ast::JSXMemberExpressionObject<'_>) -> Option<Span> {
    match object {
        ast::JSXMemberExpressionObject::IdentifierReference(reference) => Some(reference.span),
        ast::JSXMemberExpressionObject::ThisExpression(receiver) => Some(receiver.span),
        ast::JSXMemberExpressionObject::MemberExpression(member) => member_root(&member.object),
    }
}
pub(crate) fn diagnostic(
    phase: DiagnosticPhase,
    severity: Severity,
    code: &str,
    message: impl Into<String>,
    span: std::ops::Range<u32>,
) -> Diagnostic {
    Diagnostic {
        phase,
        severity,
        code: Some(code.into()),
        message: message.into(),
        labels: vec![DiagnosticLabel {
            span,
            message: None,
        }],
    }
}
