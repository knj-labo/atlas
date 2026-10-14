use crate::{facts::*, semantic::Projection, syntax::*};
use oxc_ast::{AstKind, ast};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};

#[derive(Clone)]
pub(crate) struct Seed {
    pub name: String,
    pub origin: InjectionOrigin,
}
pub(crate) struct ScriptInput {
    pub index: usize,
    pub seeds: Vec<Seed>,
    pub has_define_vars: bool,
}

pub(crate) struct Markup<'s, 'f> {
    pub source: &'s str,
    pub projection: &'f Projection,
    pub facts: &'f mut FileFacts,
    pub scripts: Vec<ScriptInput>,
    pub style_seeds: Vec<(usize, Vec<Seed>)>,
    pub in_frontmatter: bool,
    pub stack: Vec<ElementId>,
}
impl<'a> Visit<'a> for Markup<'_, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::AstroFrontmatter(_) => self.in_frontmatter = true,
            AstKind::JSXElement(element) if !self.in_frontmatter => self.element(element),
            AstKind::JSXFragment(fragment) if !self.in_frontmatter => {
                let id = ElementId(self.facts.elements.len());
                self.facts.elements.push(Element {
                    id,
                    parent: self.stack.last().copied(),
                    kind: ElementKind::Fragment,
                    name: String::new(),
                    span: range(fragment.span),
                    opening_span: range(fragment.opening_fragment.span),
                    name_span: None,
                    closing_span: Some(range(fragment.closing_fragment.span)),
                    reference: None,
                    attributes: Vec::new(),
                    directives: Vec::new(),
                    text: Vec::new(),
                    slot: None,
                });
                self.stack.push(id);
            }
            AstKind::JSXText(text) if !self.in_frontmatter => {
                if let Some(id) = self.stack.last() {
                    self.facts.elements[id.0]
                        .text
                        .push(source_text(self.source, text.span));
                }
            }
            _ => {}
        }
    }
    fn leave_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::AstroFrontmatter(_) => self.in_frontmatter = false,
            AstKind::JSXElement(_) | AstKind::JSXFragment(_) if !self.in_frontmatter => {
                self.stack.pop();
            }
            _ => {}
        }
    }
}
impl Markup<'_, '_> {
    fn element(&mut self, element: &ast::JSXElement<'_>) {
        let id = ElementId(self.facts.elements.len());
        let name = element
            .opening_element
            .name
            .span()
            .source_text(self.source)
            .to_owned();
        let root_span = element
            .opening_element
            .name
            .get_identifier()
            .map(|r| r.span)
            .or_else(|| match &element.opening_element.name {
                ast::JSXElementName::ThisExpression(receiver) => Some(receiver.span),
                ast::JSXElementName::MemberExpression(member) => member_root(&member.object),
                _ => None,
            });
        let root =
            root_span.map(|span| self.projection.references[&(span.start, span.end)].clone());
        let kind = match name.as_str() {
            "script" => ElementKind::Script,
            "style" => ElementKind::Style,
            "slot" => ElementKind::Slot,
            "Fragment"
                if matches!(
                    root.as_ref().map(|r| &r.resolution),
                    Some(Resolution::Builtin(Builtin::Fragment))
                ) =>
            {
                ElementKind::Fragment
            }
            "Astro.self"
                if matches!(
                    root.as_ref().map(|r| &r.resolution),
                    Some(Resolution::Builtin(Builtin::Astro))
                ) =>
            {
                ElementKind::SelfComponent
            }
            _ if root.is_some() => {
                if matches!(
                    root.as_ref().map(|r| &r.resolution),
                    Some(
                        Resolution::Resolved(Binding { import: None, .. })
                            | Resolution::RuntimeDependent { .. }
                    )
                ) {
                    ElementKind::Dynamic
                } else {
                    ElementKind::Component
                }
            }
            _ if name.contains('-') => ElementKind::CustomElement,
            _ => ElementKind::Html,
        };
        let attrs = attributes(self.source, &element.opening_element.attributes);
        let directives = attrs
            .iter()
            .filter_map(|a| {
                let name = a.name.as_ref()?;
                name.contains(':').then(|| Directive {
                    name: name.clone(),
                    span: a.span.clone(),
                    name_span: a.name_span.clone().unwrap(),
                    value: a.value.clone(),
                    status: ValidationStatus::Valid,
                })
            })
            .collect::<Vec<_>>();
        let slot = attrs
            .iter()
            .find(|a| a.name.as_deref() == Some("slot"))
            .map(|a| SlotAssignment {
                name: match &a.literal {
                    Some(Literal::String(s)) => Some(s.clone()),
                    _ => None,
                },
                span: a.span.clone(),
                dynamic: !matches!(a.literal, Some(Literal::String(_))),
            });
        self.facts.elements.push(Element {
            id,
            parent: self.stack.last().copied(),
            kind,
            name: name.clone(),
            span: range(element.span),
            opening_span: range(element.opening_element.span),
            name_span: Some(range(element.opening_element.name.span())),
            closing_span: element.closing_element.as_ref().map(|c| range(c.span)),
            reference: root.as_ref().map(|r| r.id),
            attributes: attrs.clone(),
            directives: directives.clone(),
            text: Vec::new(),
            slot,
        });
        if kind == ElementKind::SelfComponent {
            self.facts.builtin_uses.push(BuiltinUse {
                builtin: Builtin::AstroSelf,
                member: Some("self".into()),
                span: range(element.opening_element.name.span()),
            });
        }
        if let Some(reference) = root
            && kind != ElementKind::Fragment
        {
            self.facts.components.push(ComponentUse {
                element: id,
                name,
                span: range(element.span),
                name_span: range(element.opening_element.name.span()),
                closing_name_span: element
                    .closing_element
                    .as_ref()
                    .map(|c| range(c.name.span())),
                reference,
                directives,
            });
        }
        let content_start = element.opening_element.span.end;
        let content_end = element
            .closing_element
            .as_ref()
            .map_or(element.span.end, |c| c.span.start);
        let content = source_text(
            self.source,
            Span::new(content_start.min(content_end), content_end),
        );
        let (seeds, dynamic) = define_vars(
            self.source,
            &element.opening_element.attributes,
            &self.facts.references,
        );
        let non_hoistable = self.stack.iter().any(|id| {
            matches!(
                self.facts.elements[id.0].name.as_str(),
                "svg" | "noscript" | "template"
            )
        });
        if kind == ElementKind::Script {
            let eligible = script_is_bundled(&attrs);
            let bundled = !non_hoistable && eligible;
            let (mut mode, mut language) = script_mode(&attrs, eligible);
            if non_hoistable {
                if mode == ScriptMode::Processed {
                    mode = if has(&attrs, "src") {
                        ScriptMode::External
                    } else if attr_string(&attrs, "type") == Some("module") {
                        ScriptMode::InlineModule
                    } else {
                        ScriptMode::InlineClassic
                    };
                }
                if matches!(
                    mode,
                    ScriptMode::External | ScriptMode::InlineClassic | ScriptMode::InlineModule
                ) {
                    language = ScriptLanguage::JavaScript;
                }
            }
            let index = self.facts.scripts.len();
            self.facts.scripts.push(Script {
                element: id,
                span: range(element.span),
                content,
                mode,
                bundled,
                language,
                source: attrs
                    .iter()
                    .find(|a| a.name.as_deref() == Some("src"))
                    .and_then(|a| a.value.clone()),
                references: Vec::new(),
                injected: Vec::new(),
                dynamic_injection: dynamic,
            });
            self.scripts.push(ScriptInput {
                index,
                seeds,
                has_define_vars: has(&attrs, "define:vars"),
            });
        } else if kind == ElementKind::Style {
            let index = self.facts.styles.len();
            let mode = if non_hoistable
                || has(&attrs, "is:inline")
                || has(&attrs, "set:html")
                || has(&attrs, "set:text")
            {
                StyleMode::Inline
            } else if has(&attrs, "is:global") {
                StyleMode::Global
            } else {
                StyleMode::Scoped
            };
            let effective_injection = mode != StyleMode::Inline;
            let dynamic_language = has(&attrs, "lang") && attr_string(&attrs, "lang").is_none();
            let language = attr_string(&attrs, "lang")
                .unwrap_or(if dynamic_language { "unknown" } else { "css" })
                .to_owned();
            let mode = if dynamic_language {
                StyleMode::Dynamic
            } else {
                mode
            };
            self.facts.styles.push(Style {
                id: StyleId(index),
                element: id,
                span: range(element.span),
                content,
                mode,
                language,
                inline_attribute: false,
                dynamic_injection: dynamic,
                css: None,
                injected: Vec::new(),
            });
            if effective_injection {
                self.style_seeds.push((index, seeds));
            }
        }
        for attribute in &attrs {
            if attribute.name.as_deref() == Some("style")
                && let Some(value) = &attribute.value
            {
                let mut content_span = Span::new(value.span.start, value.span.end);
                if let Some(original) =
                    element
                        .opening_element
                        .attributes
                        .iter()
                        .find_map(|a| match a {
                            ast::JSXAttributeItem::Attribute(a)
                                if range(a.span) == attribute.span =>
                            {
                                Some(a)
                            }
                            _ => None,
                        })
                    && let Some(ast::JSXAttributeValue::ExpressionContainer(c)) = &original.value
                    && let Some(e) = c.expression.as_expression()
                    && let Some(span) = (match e.without_parentheses() {
                        ast::Expression::StringLiteral(s) => Some(s.span),
                        ast::Expression::TemplateLiteral(t) if t.expressions.is_empty() => {
                            Some(t.span)
                        }
                        _ => None,
                    })
                {
                    content_span = span;
                }
                let index = self.facts.styles.len();
                self.facts.styles.push(Style {
                    id: StyleId(index),
                    element: id,
                    span: attribute.span.clone(),
                    content: source_text(self.source, content_span),
                    mode: if matches!(attribute.literal, Some(Literal::String(_))) {
                        StyleMode::Inline
                    } else {
                        StyleMode::Dynamic
                    },
                    language: "css".into(),
                    inline_attribute: true,
                    dynamic_injection: false,
                    css: None,
                    injected: Vec::new(),
                });
            }
        }
        self.stack.push(id);
    }
}
pub(crate) fn has(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.name.as_deref() == Some(name))
}
pub(crate) fn attr_string<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    match &attrs
        .iter()
        .find(|a| a.name.as_deref() == Some(name))?
        .literal
    {
        Some(Literal::String(s)) => Some(s),
        _ => None,
    }
}
fn script_is_bundled(attrs: &[Attribute]) -> bool {
    let mut module = false;
    let mut src = false;
    let mut other = false;
    for a in attrs.iter().filter(|a| !a.spread) {
        match a.name.as_deref() {
            Some("is:inline" | "define:vars") => return false,
            Some("src") => src = true,
            Some("type")
                if a.value
                    .as_ref()
                    .is_some_and(|v| v.kind == DirectiveValueKind::String)
                    && matches!(&a.literal, Some(Literal::String(s)) if s == "module") =>
            {
                module = true
            }
            _ => other = true,
        }
    }
    if src {
        !module && !other
    } else {
        attrs.is_empty() || (module && !other)
    }
}
fn script_mode(attrs: &[Attribute], bundled: bool) -> (ScriptMode, ScriptLanguage) {
    if has(attrs, "src")
        && (!bundled
            || attrs.iter().any(|a| {
                a.name.as_deref() == Some("src")
                    && a.value
                        .as_ref()
                        .is_some_and(|v| v.kind == DirectiveValueKind::String)
            }))
    {
        return (
            ScriptMode::External,
            if bundled {
                ScriptLanguage::TypeScript
            } else {
                ScriptLanguage::JavaScript
            },
        );
    }
    if bundled {
        return (ScriptMode::Processed, ScriptLanguage::TypeScript);
    }
    if has(attrs, "type") && attr_string(attrs, "type").is_none() {
        return (ScriptMode::Dynamic, ScriptLanguage::Unknown);
    }
    if let Some(t) = attr_string(attrs, "type") {
        if t == "module" {
            return (ScriptMode::InlineModule, ScriptLanguage::JavaScript);
        }
        if !matches!(
            t,
            "" | "text/javascript"
                | "application/javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "application/x-javascript"
        ) {
            return (ScriptMode::Data, ScriptLanguage::Raw);
        }
    }
    (ScriptMode::InlineClassic, ScriptLanguage::JavaScript)
}

pub(crate) fn define_vars(
    source: &str,
    attrs: &[ast::JSXAttributeItem<'_>],
    references: &[Reference],
) -> (Vec<Seed>, bool) {
    let Some(attribute) = attrs.iter().find_map(|a| match a {
        ast::JSXAttributeItem::Attribute(a) if attribute_name(&a.name) == "define:vars" => Some(a),
        _ => None,
    }) else {
        return (Vec::new(), false);
    };
    let Some(ast::JSXAttributeValue::ExpressionContainer(container)) = &attribute.value else {
        return (Vec::new(), true);
    };
    let Some(ast::Expression::ObjectExpression(object)) = container.expression.as_expression()
    else {
        return (Vec::new(), true);
    };
    let mut result: Vec<Seed> = Vec::new();
    let mut dynamic = false;
    for property in &object.properties {
        if let ast::ObjectPropertyKind::ObjectProperty(property) = property {
            if let Some(name) = property_key(property) {
                let span = property.value.span();
                let origin = InjectionOrigin {
                    key_span: range(property.key.span()),
                    value: source_text(source, span),
                    references: references
                        .iter()
                        .filter(|r| span.start <= r.span.start && r.span.end <= span.end)
                        .map(|r| r.id)
                        .collect(),
                };
                let seed = Seed {
                    name: name.clone(),
                    origin,
                };
                if let Some(existing) = result.iter_mut().find(|s| s.name == name) {
                    *existing = seed;
                } else {
                    result.push(seed);
                }
            } else {
                dynamic = true;
            }
        } else {
            dynamic = true;
        }
    }
    (result, dynamic)
}
