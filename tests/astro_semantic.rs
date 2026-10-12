use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_ast_visit::Visit;
use oxc_parser::{AstroParserReturn, Parser};
use oxc_semantic::{Semantic, SemanticBuilder, SemanticBuilderAstroExt, SymbolId};
use oxc_span::{GetSpan, SourceType, Span};

// Only borrowed upstream AST nodes: this is a test visitor, not an atlas facts model.
#[derive(Default)]
struct Nodes<'a> {
    bindings: Vec<&'a BindingIdentifier<'a>>,
    references: Vec<&'a IdentifierReference<'a>>,
    elements: Vec<&'a JSXElement<'a>>,
    scripts: Vec<&'a AstroScript<'a>>,
    comments: Vec<&'a AstroComment<'a>>,
    strings: Vec<&'a StringLiteral<'a>>,
}

impl<'a> Visit<'a> for Nodes<'a> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        match kind {
            AstKind::BindingIdentifier(node) => self.bindings.push(node),
            AstKind::IdentifierReference(node) => self.references.push(node),
            AstKind::JSXElement(node) => self.elements.push(node),
            AstKind::AstroScript(node) => self.scripts.push(node),
            AstKind::AstroComment(node) => self.comments.push(node),
            AstKind::StringLiteral(node) => self.strings.push(node),
            _ => {}
        }
    }
}

impl<'a> Nodes<'a> {
    fn collect(root: &'a AstroRoot<'a>) -> Self {
        let mut nodes = Self::default();
        nodes.visit_astro_root(root);
        nodes
    }

    fn binding(&self, span: Span) -> &'a BindingIdentifier<'a> {
        let found: Vec<_> = self
            .bindings
            .iter()
            .filter(|node| node.span == span)
            .collect();
        assert_eq!(found.len(), 1, "binding at {span:?}");
        found[0]
    }

    fn reference(&self, span: Span) -> &'a IdentifierReference<'a> {
        let found: Vec<_> = self
            .references
            .iter()
            .filter(|node| node.span == span)
            .collect();
        assert_eq!(found.len(), 1, "reference at {span:?}");
        found[0]
    }

    fn components(&self) -> Vec<&'a JSXElement<'a>> {
        self.elements
            .iter()
            .copied()
            .filter(|element| {
                matches!(
                    element.opening_element.name,
                    JSXElementName::IdentifierReference(_) | JSXElementName::MemberExpression(_)
                )
            })
            .collect()
    }
}

fn parse<'a>(allocator: &'a Allocator, source: &'a str) -> AstroParserReturn<'a> {
    Parser::new(allocator, source, SourceType::astro()).parse_astro()
}

// This gate is shared by valid-input and invalid-input tests. Script parsing is
// already part of parse_astro(); do not invoke parse_astro_scripts a second time.
fn analyze<'a>(parsed: &'a AstroParserReturn<'a>, source: &'a str) -> Option<Semantic<'a>> {
    if parsed.panicked || !parsed.errors.is_empty() {
        return None;
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
    assert!(
        result.errors.is_empty(),
        "semantic syntax diagnostics: {:?}",
        result.errors
    );
    Some(result.semantic)
}

fn valid_semantic<'a>(parsed: &'a AstroParserReturn<'a>, source: &'a str) -> Semantic<'a> {
    assert!(
        !parsed.panicked,
        "parser terminated early: {:?}",
        parsed.errors
    );
    assert!(
        parsed.errors.is_empty(),
        "parser syntax diagnostics: {:?}",
        parsed.errors
    );
    analyze(parsed, source).expect("valid input must be analyzed")
}

fn slice(source: &str, span: Span) -> &str {
    source
        .get(span.start as usize..span.end as usize)
        .expect("UTF-8 half-open byte span")
}

// Text locates an unambiguous fixture position only. Resolution below always
// compares the reference's SymbolId with the BindingIdentifier's SymbolId.
fn location(source: &str, context: &str, token: &str) -> Span {
    assert_eq!(
        source.matches(context).count(),
        1,
        "unique context: {context}"
    );
    assert_eq!(context.matches(token).count(), 1, "unique token: {token}");
    let start = source.find(context).unwrap() + context.find(token).unwrap();
    Span::new(
        start.try_into().unwrap(),
        (start + token.len()).try_into().unwrap(),
    )
}

fn symbol(binding: &BindingIdentifier<'_>) -> SymbolId {
    binding.symbol_id.get().expect("declaration has a SymbolId")
}

fn assert_resolves(
    semantic: &Semantic<'_>,
    reference: &IdentifierReference<'_>,
    binding: &BindingIdentifier<'_>,
) {
    let reference_id = reference
        .reference_id
        .get()
        .expect("reference has a ReferenceId");
    let actual = semantic.scoping().get_reference(reference_id);
    assert_eq!(
        actual.symbol_id(),
        Some(symbol(binding)),
        "reference at {:?}",
        reference.span
    );
    assert_eq!(
        semantic.scoping().symbol_span(symbol(binding)),
        binding.span
    );
    assert_eq!(semantic.reference_span(actual), reference.span);
    assert!(
        semantic
            .symbol_references(symbol(binding))
            .any(|item| item.node_id() == actual.node_id())
    );
    let declaration_span = semantic.symbol_declaration(symbol(binding)).kind().span();
    assert!(
        declaration_span.start <= binding.span.start && binding.span.end <= declaration_span.end
    );
}

fn assert_unresolved(semantic: &Semantic<'_>, reference: &IdentifierReference<'_>) {
    let id = reference
        .reference_id
        .get()
        .expect("unresolved reference still has a ReferenceId");
    assert_eq!(
        semantic.scoping().get_reference(id).symbol_id(),
        None,
        "reference at {:?}",
        reference.span
    );
    assert!(
        semantic
            .scoping()
            .root_unresolved_references_ids()
            .flatten()
            .any(|item| item == id)
    );
}

fn tag_reference<'a>(name: &'a JSXElementName<'a>) -> &'a IdentifierReference<'a> {
    match name {
        JSXElementName::IdentifierReference(reference) => reference,
        _ => panic!("expected a simple component tag"),
    }
}

fn attributes<'a>(element: &'a JSXElement<'a>) -> Vec<&'a JSXAttribute<'a>> {
    element
        .opening_element
        .attributes
        .iter()
        .map(|item| match item {
            JSXAttributeItem::Attribute(attribute) => attribute.as_ref(),
            JSXAttributeItem::SpreadAttribute(_) => panic!("fixture has no spread attributes"),
        })
        .collect()
}

#[test]
fn counter_import_parameter_and_script() {
    let source = include_str!("fixtures/counter.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    let imported = nodes.binding(location(source, "import Counter from", "Counter"));
    let parameter = nodes.binding(location(source, "(Counter) =>", "Counter"));
    assert_ne!(symbol(imported), symbol(parameter));
    let components = nodes.components();
    assert_eq!(components.len(), 2);
    assert_resolves(
        &semantic,
        tag_reference(&components[0].opening_element.name),
        imported,
    );
    assert_resolves(
        &semantic,
        tag_reference(&components[1].opening_element.name),
        parameter,
    );
    assert_eq!(attributes(components[0]).len(), 1);
    assert_eq!(
        slice(source, attributes(components[0])[0].name.span()),
        "client:visible"
    );
    assert!(attributes(components[1]).is_empty());
    assert_eq!(nodes.scripts.len(), 1);
    assert_unresolved(
        &semantic,
        nodes.reference(location(source, "console.log(Counter)", "Counter")),
    );
    assert_unresolved(
        &semantic,
        nodes.reference(location(source, "Astro.props", "Astro")),
    );
    // Only the first tag uses the import; map() and <script> must not add references.
    assert_eq!(semantic.symbol_references(symbol(imported)).count(), 1);
}

#[test]
fn imports_block_and_nested_parameters_have_distinct_symbols() {
    let source = include_str!("fixtures/shadowing.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    let imported = nodes.binding(location(source, "import Counter from", "Counter"));
    let named = nodes.binding(location(source, "as NamedCounter", "NamedCounter"));
    let local = nodes.binding(location(source, "const Counter =", "Counter"));
    let outer = nodes.binding(location(source, "(Counter) => <section>", "Counter"));
    let inner_parameter = location(source, "(Counter) => <Counter data-inner", "(Counter)");
    let inner = nodes.binding(Span::new(
        inner_parameter.start + 1,
        inner_parameter.end - 1,
    ));
    let declarations = [imported, named, local, outer, inner];
    for (index, left) in declarations.iter().enumerate() {
        for right in &declarations[index + 1..] {
            assert_ne!(symbol(left), symbol(right));
        }
    }
    assert!(
        semantic
            .scoping()
            .symbol_flags(symbol(imported))
            .is_import()
    );
    assert!(semantic.scoping().symbol_flags(symbol(named)).is_import());
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "= NamedCounter;", "NamedCounter")),
        named,
    );
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "  Counter;", "Counter")),
        local,
    );
    let components = nodes.components();
    assert_eq!(components.len(), 6);
    for (element, declaration) in components
        .iter()
        .zip([imported, named, outer, inner, outer, imported])
    {
        assert_resolves(
            &semantic,
            tag_reference(&element.opening_element.name),
            declaration,
        );
    }
}

#[test]
fn member_tag_resolves_only_the_namespace_object() {
    let source = include_str!("fixtures/member.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    let imported = nodes.binding(location(source, "* as UI", "UI"));
    let components = nodes.components();
    assert_eq!(components.len(), 1);
    let element = components[0];
    for name in [
        &element.opening_element.name,
        &element.closing_element.as_ref().unwrap().name,
    ] {
        let JSXElementName::MemberExpression(member) = name else {
            panic!("member tag")
        };
        let JSXMemberExpressionObject::IdentifierReference(object) = &member.object else {
            panic!("namespace reference")
        };
        assert_resolves(&semantic, object, imported);
        assert_eq!(slice(source, member.property.span), "Counter");
        assert!(
            !nodes
                .references
                .iter()
                .any(|node| node.span == member.property.span)
        );
    }
    assert_eq!(nodes.references.len(), 2);
}

#[test]
fn scripts_resolve_locals_and_isolate_other_scopes() {
    let source = include_str!("fixtures/scripts.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    assert_eq!(nodes.scripts.len(), 2);
    assert!(
        nodes
            .scripts
            .iter()
            .all(|script| !script.program.body.is_empty())
    );
    let first = nodes.binding(location(source, "const shared = 1", "shared"));
    let second = nodes.binding(location(source, "const shared = 2", "shared"));
    let parameter = nodes.binding(location(source, "(TemplateOnly) =>", "TemplateOnly"));
    let imported = nodes.binding(location(source, "import Counter from", "Counter"));
    let scopes =
        [first, second, parameter, imported].map(|binding| semantic.symbol_scope(symbol(binding)));
    for (index, scope) in scopes.iter().enumerate() {
        for other in &scopes[index + 1..] {
            assert_ne!(scope, other);
        }
    }
    assert_eq!(nodes.scripts[0].program.scope_id.get(), Some(scopes[0]));
    assert_eq!(nodes.scripts[1].program.scope_id.get(), Some(scopes[1]));
    for (context, binding) in [
        ("firstOnly = shared;", first),
        ("    shared;", first),
        ("return [shared,", first),
        ("secondOnly = shared;", second),
        ("\n  shared;", second),
    ] {
        assert_resolves(
            &semantic,
            nodes.reference(location(source, context, "shared")),
            binding,
        );
    }
    let nested = nodes.binding(location(source, "function nested()", "nested"));
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "    nested();", "nested")),
        nested,
    );
    for (context, name) in [
        ("    Counter;", "Counter"),
        ("    serverOnly;", "serverOnly"),
        ("    TemplateOnly;", "TemplateOnly"),
        ("    secondOnly;", "secondOnly"),
        ("shared, Counter,", "Counter"),
        ("Counter, TemplateOnly,", "TemplateOnly"),
        ("TemplateOnly, secondOnly]", "secondOnly"),
        ("  firstOnly;", "firstOnly"),
        ("\n  Counter;", "Counter"),
        ("{firstOnly}", "firstOnly"),
        ("{secondOnly}", "secondOnly"),
    ] {
        assert_unresolved(&semantic, nodes.reference(location(source, context, name)));
    }
    let components = nodes.components();
    assert_eq!(components.len(), 2);
    assert_resolves(
        &semantic,
        tag_reference(&components[0].opening_element.name),
        imported,
    );
    assert_resolves(
        &semantic,
        tag_reference(&components[1].opening_element.name),
        parameter,
    );
}

// is:inline scripts are still browser JavaScript; Astro only skips bundling them.
#[test]
fn inline_script_is_parsed_and_isolated_from_frontmatter() {
    let source = include_str!("fixtures/inline.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    assert_eq!(nodes.scripts.len(), 1);
    let local = nodes.binding(location(source, "const inlineOnly", "inlineOnly"));
    assert_eq!(
        nodes.scripts[0].program.scope_id.get(),
        Some(semantic.symbol_scope(symbol(local)))
    );
    assert_resolves(
        &semantic,
        nodes.reference(location(source, " inlineOnly;", "inlineOnly")),
        local,
    );
}

// define:vars injects frontmatter values into the script at runtime.
#[test]
#[ignore = "upstream: build_astro isolates define:vars script bodies from frontmatter"]
fn define_vars_script_sees_frontmatter_values() {
    let source = include_str!("fixtures/define-vars.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    let title = nodes.binding(location(source, "const title", "title"));
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "{{ title }}", "title")),
        title,
    );
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "log(title)", "title")),
        title,
    );
}

#[test]
fn non_javascript_script_type_preserves_raw_text() {
    let source = include_str!("fixtures/raw-script.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let _semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    assert!(nodes.scripts.is_empty());
    assert!(nodes.bindings.is_empty());
    assert!(nodes.references.is_empty());
    assert!(nodes.components().is_empty());
    let [JSXChild::Text(body)] = nodes.elements[0].children.as_slice() else {
        panic!("raw script text")
    };
    assert_eq!(
        slice(source, body.span),
        r#"{"component":"<Counter />", "value": }"#
    );
}

#[test]
fn uses_tags_and_directives_remain_distinct_ast_nodes() {
    let source = include_str!("fixtures/usage.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    let imported = nodes.binding(location(source, "import Counter from", "Counter"));
    let components = nodes.components();
    assert_eq!(components.len(), 3);
    assert_eq!(nodes.elements.len(), 5); // div, counter, and three Counter elements
    assert_eq!(nodes.comments.len(), 1);
    assert!(
        nodes
            .strings
            .iter()
            .any(|string| string.value == "<Counter />")
    );
    assert!(!parsed.body_comments.is_empty());
    let mut tag_ids = Vec::new();
    for element in &components {
        let reference = tag_reference(&element.opening_element.name);
        assert_resolves(&semantic, reference, imported);
        tag_ids.push(reference.reference_id.get().unwrap());
        if let Some(closing) = &element.closing_element {
            let closing_reference = tag_reference(&closing.name);
            assert_resolves(&semantic, closing_reference, imported);
            assert_ne!(reference.span, closing_reference.span);
            tag_ids.push(closing_reference.reference_id.get().unwrap());
        }
    }
    assert_eq!(tag_ids.len(), 4); // 3 elements; the paired element has 2 tag-name references
    for (index, id) in tag_ids.iter().enumerate() {
        assert!(!tag_ids[index + 1..].contains(id));
    }
    assert_eq!(semantic.symbol_references(symbol(imported)).count(), 4);
    let first_attributes = attributes(components[0]);
    assert_eq!(
        first_attributes
            .iter()
            .map(|attribute| slice(source, attribute.name.span()))
            .collect::<Vec<_>>(),
        [
            "client:visible",
            "client:idle",
            "client:visible",
            "custom:flag"
        ]
    );
    assert_ne!(first_attributes[0].span, first_attributes[2].span);
    assert!(
        first_attributes[..3]
            .iter()
            .all(|attribute| attribute.value.is_none())
    );
    assert_eq!(
        slice(source, first_attributes[3].value.as_ref().unwrap().span()),
        "\"yes\""
    );
    let second_attributes = attributes(components[1]);
    assert_eq!(second_attributes.len(), 1);
    assert_eq!(
        slice(source, second_attributes[0].name.span()),
        "client:only"
    );
    assert_eq!(
        slice(source, second_attributes[0].value.as_ref().unwrap().span()),
        "\"react\""
    );
    assert!(attributes(components[2]).is_empty());
    assert!(components[0].closing_element.is_some());
    assert!(
        components[1..]
            .iter()
            .all(|element| element.closing_element.is_none())
    );
}

#[test]
fn spans_are_utf8_half_open_offsets_in_the_original_crlf_source() {
    // Read the fixture as bytes to make preservation of its CRLF bytes explicit.
    let source = std::str::from_utf8(include_bytes!("fixtures/positions.astro")).unwrap();
    assert!(source.contains("\r\n"));
    assert_eq!(
        source.bytes().filter(|byte| *byte == b'\n').count(),
        source.matches("\r\n").count()
    );
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    assert_eq!(slice(source, parsed.root.span), source);
    let imported = nodes.binding(location(source, "import Counter from", "Counter"));
    assert_eq!(slice(source, imported.span), "Counter");
    let components = nodes.components();
    assert_eq!(components.len(), 1);
    let element = components[0];
    assert_eq!(
        slice(source, element.span),
        "<Counter title=\"和文 😀\" client:media=\"(幅: 1px)\">{caption}</Counter>"
    );
    assert_eq!(
        slice(source, element.opening_element.name.span()),
        "Counter"
    );
    assert_eq!(
        slice(
            source,
            element.closing_element.as_ref().unwrap().name.span()
        ),
        "Counter"
    );
    assert_resolves(
        &semantic,
        tag_reference(&element.opening_element.name),
        imported,
    );
    assert_resolves(
        &semantic,
        tag_reference(&element.closing_element.as_ref().unwrap().name),
        imported,
    );
    let attrs = attributes(element);
    assert_eq!(slice(source, attrs[0].name.span()), "title");
    assert_eq!(slice(source, attrs[0].span), "title=\"和文 😀\"");
    assert_eq!(
        slice(source, attrs[0].value.as_ref().unwrap().span()),
        "\"和文 😀\""
    );
    assert_eq!(slice(source, attrs[1].name.span()), "client:media");
    assert_eq!(
        slice(source, attrs[1].value.as_ref().unwrap().span()),
        "\"(幅: 1px)\""
    );
    let caption = nodes.binding(location(source, "const caption =", "caption"));
    assert_resolves(
        &semantic,
        nodes.reference(location(source, "{caption}", "caption")),
        caption,
    );
    assert!(source[..element.span.start as usize].chars().count() < element.span.start as usize);
}

#[test]
fn empty_file_and_template_without_frontmatter_are_analyzable() {
    for source in [
        include_str!("fixtures/empty.astro"),
        include_str!("fixtures/no-frontmatter.astro"),
    ] {
        let allocator = Allocator::default();
        let parsed = parse(&allocator, source);
        let semantic = valid_semantic(&parsed, source);
        let frontmatter = parsed
            .root
            .frontmatter
            .as_ref()
            .expect("synthetic frontmatter");
        assert_eq!(frontmatter.span, Span::new(0, 0));
        assert!(frontmatter.program.body.is_empty());
        assert!(frontmatter.program.scope_id.get().is_some());
        assert_eq!(slice(source, parsed.root.span), source);
        assert_eq!(
            semantic
                .scoping()
                .root_unresolved_references_ids()
                .flatten()
                .count(),
            0
        );
        assert_eq!(
            Nodes::collect(&parsed.root).elements.len(),
            usize::from(!source.is_empty())
        );
    }
}

#[test]
fn malformed_input_is_diagnosed_and_never_semantically_analyzed() {
    for (name, source) in [
        (
            "frontmatter",
            include_str!("fixtures/invalid-frontmatter.astro"),
        ),
        ("template", include_str!("fixtures/invalid-template.astro")),
        ("script", include_str!("fixtures/invalid-script.astro")),
    ] {
        let allocator = Allocator::default();
        let parsed = parse(&allocator, source);
        assert!(
            !parsed.errors.is_empty(),
            "{name}: expected parser diagnostics"
        );
        assert!(
            analyze(&parsed, source).is_none(),
            "{name}: must skip semantic analysis"
        );
        let nodes = Nodes::collect(&parsed.root);
        assert!(
            nodes
                .bindings
                .iter()
                .all(|node| node.symbol_id.get().is_none())
        );
        assert!(
            nodes
                .references
                .iter()
                .all(|node| node.reference_id.get().is_none())
        );
        for error in &parsed.errors {
            assert!(
                !error.message.is_empty(),
                "{name}: diagnostic must explain the syntax error"
            );
            eprintln!("{name}: panicked={}, {error:?}", parsed.panicked);
        }
    }
}

#[test]
fn valid_undefined_component_is_an_unresolved_reference_not_a_syntax_error() {
    let source = include_str!("fixtures/undefined.astro");
    let allocator = Allocator::default();
    let parsed = parse(&allocator, source);
    let semantic = valid_semantic(&parsed, source);
    let nodes = Nodes::collect(&parsed.root);
    assert!(nodes.bindings.is_empty());
    assert_eq!(nodes.components().len(), 1);
    assert_eq!(nodes.references.len(), 1);
    assert_unresolved(
        &semantic,
        tag_reference(&nodes.components()[0].opening_element.name),
    );
}
