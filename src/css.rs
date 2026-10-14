use crate::{css_source::CssSource, facts::*, syntax::diagnostic};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use lightningcss::{
    properties::{Property, PropertyId, animation::AnimationName},
    stylesheet::{ParserFlags, ParserOptions, StyleAttribute, StyleSheet},
};

#[derive(Clone, Debug)]
struct Lex<'i> {
    token: Token<'i>,
    start: usize,
    end: usize,
    children: Vec<Lex<'i>>,
}
impl Lex<'_> {
    fn significant(&self) -> bool {
        !matches!(self.token, Token::WhiteSpace(_) | Token::Comment(_))
    }
}
fn tokenize<'i>(parser: &mut Parser<'i, '_>, offset: usize) -> Vec<Lex<'i>> {
    let mut result = Vec::new();
    loop {
        let start = offset + parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        let children = if matches!(
            token,
            Token::Function(_)
                | Token::CurlyBracketBlock
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
        ) {
            parser
                .parse_nested_block(|nested| Ok::<_, ParseError<'i, ()>>(tokenize(nested, offset)))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        result.push(Lex {
            token,
            start,
            end: offset + parser.position().byte_index(),
            children,
        });
    }
    result
}

pub(crate) fn analyze_styles(source: &str, facts: &mut FileFacts) -> Result<(), AnalysisError> {
    for style in &mut facts.styles {
        if style.language != "css" || style.mode == StyleMode::Dynamic {
            continue;
        }
        let input = if style.inline_attribute {
            let attribute = facts.elements[style.element.0]
                .attributes
                .iter()
                .find(|a| a.span == style.span)
                .expect("style attribute");
            let javascript = attribute
                .value
                .as_ref()
                .is_some_and(|v| v.kind == DirectiveValueKind::Expression);
            CssSource::attribute(&style.content.raw, style.content.span.start, javascript)
        } else {
            CssSource::direct(&style.content.raw, style.content.span.start)
        };
        let options = ParserOptions {
            flags: ParserFlags::NESTING,
            ..ParserOptions::default()
        };
        let error = if style.inline_attribute {
            StyleAttribute::parse(&input.code, options).err()
        } else {
            StyleSheet::parse(&input.code, options).err()
        };
        if let Some(error) = error {
            let start = error
                .loc
                .as_ref()
                .map_or(0, |loc| input.location(loc.line, loc.column));
            let end = (start
                + input
                    .code
                    .get(start..)
                    .and_then(|s| s.chars().next())
                    .map_or(0, char::len_utf8))
            .min(input.code.len());
            return Err(AnalysisError {
                diagnostics: vec![diagnostic(
                    DiagnosticPhase::Css,
                    Severity::Error,
                    "css/syntax",
                    error.to_string(),
                    input.range(start, end),
                )],
                parser_panicked: false,
            });
        }
        let mut parser_input = ParserInput::new(&input.code);
        let mut parser = Parser::new(&mut parser_input);
        let mut collector = Collector {
            source,
            input: &input,
            css: CssFacts::default(),
        };
        let mut rules = Rules {
            collector: &mut collector,
            parent: None,
            conditions: Vec::new(),
            declarations: style.inline_attribute,
        };
        let error = if style.inline_attribute {
            RuleBodyParser::new(&mut parser, &mut rules).find_map(Result::err)
        } else {
            let state = parser.state();
            let charset = matches!(parser.next(), Ok(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("charset"));
            parser.reset(&state);
            let first = if charset {
                RuleBodyParser::new(&mut parser, &mut rules)
                    .next()
                    .and_then(Result::err)
            } else {
                None
            };
            first.or_else(|| StyleSheetParser::new(&mut parser, &mut rules).find_map(Result::err))
        };
        if let Some((error, _)) = error {
            let start = input.location(error.location.line, error.location.column);
            let end = start + input.code[start..].chars().next().map_or(0, char::len_utf8);
            return Err(AnalysisError {
                diagnostics: vec![diagnostic(
                    DiagnosticPhase::Css,
                    Severity::Error,
                    "css/syntax",
                    format!("{:?}", error.kind),
                    input.range(start, end),
                )],
                parser_panicked: false,
            });
        }
        for injection in &style.injected {
            collector.css.definitions.push(CssDefinition {
                name: injection.name.clone(),
                span: injection.origin.key_span.clone(),
                kind: CssDefinitionKind::Injected,
                declaration: None,
                conditions: Vec::new(),
            });
        }
        style.css = Some(collector.css);
    }
    let definitions: Vec<_> = facts
        .styles
        .iter()
        .flat_map(|style| {
            style.css.iter().flat_map(move |css| {
                css.definitions
                    .iter()
                    .enumerate()
                    .map(move |(index, d)| (style.id, index, d.name.clone(), d.kind))
            })
        })
        .collect();
    for style in &mut facts.styles {
        if let Some(css) = &mut style.css {
            for reference in &mut css.references {
                reference.candidates = definitions
                    .iter()
                    .filter(|(_, _, name, kind)| {
                        name == &reference.name
                            && match reference.kind {
                                CssReferenceKind::CustomProperty => {
                                    *kind != CssDefinitionKind::Keyframes
                                }
                                CssReferenceKind::Animation => {
                                    *kind == CssDefinitionKind::Keyframes
                                }
                            }
                    })
                    .map(|(style, index, _, _)| CssCandidate {
                        style: *style,
                        definition: *index,
                    })
                    .collect();
            }
        }
    }
    Ok(())
}
struct Collector<'a> {
    source: &'a str,
    input: &'a CssSource,
    css: CssFacts,
}
impl Collector<'_> {
    fn rule(
        &mut self,
        prelude: &[Lex],
        parent: Option<CssRuleId>,
        conditions: &[SourceText],
    ) -> (CssRuleId, Vec<SourceText>) {
        let significant: Vec<_> = prelude.iter().filter(|t| t.significant()).collect();
        let first = significant[0];
        let last = significant.last().unwrap();
        let kind = match &first.token {
            Token::AtKeyword(name) => match name.to_ascii_lowercase().as_str() {
                "media" => CssRuleKind::Media,
                "supports" => CssRuleKind::Supports,
                "container" => CssRuleKind::Container,
                "layer" => CssRuleKind::Layer,
                "scope" => CssRuleKind::Scope,
                "keyframes" | "-webkit-keyframes" => CssRuleKind::Keyframes,
                "property" => CssRuleKind::Property,
                _ => CssRuleKind::AtRule(name.to_string()),
            },
            _ if parent.is_some_and(|id| self.css.rules[id.0].kind == CssRuleKind::Keyframes) => {
                CssRuleKind::Keyframe
            }
            _ => CssRuleKind::Style,
        };
        let id = CssRuleId(self.css.rules.len());
        let text = self.input.source_text(self.source, first.start, last.end);
        let selectors = if kind == CssRuleKind::Style {
            self.selectors(prelude)
        } else {
            Vec::new()
        };
        self.css.rules.push(CssRule {
            id,
            parent,
            span: self.input.range(first.start, first.end),
            prelude: text.clone(),
            kind: kind.clone(),
            selectors,
            conditions: conditions.to_vec(),
        });
        if matches!(kind, CssRuleKind::Keyframes | CssRuleKind::Property)
            && let Some(name) = significant.get(1)
            && let Token::Ident(value) | Token::QuotedString(value) = &name.token
        {
            self.css.definitions.push(CssDefinition {
                name: value.to_string(),
                span: self.input.range(name.start, name.end),
                kind: if kind == CssRuleKind::Keyframes {
                    CssDefinitionKind::Keyframes
                } else {
                    CssDefinitionKind::RegisteredProperty
                },
                declaration: None,
                conditions: conditions.to_vec(),
            });
        }
        let mut nested = conditions.to_vec();
        if matches!(
            kind,
            CssRuleKind::Media
                | CssRuleKind::Supports
                | CssRuleKind::Container
                | CssRuleKind::Layer
                | CssRuleKind::Scope
        ) {
            nested.push(text);
        }
        (id, nested)
    }
    fn statement(
        &mut self,
        tokens: &[Lex],
        parent: Option<CssRuleId>,
        conditions: &[SourceText],
        end: usize,
    ) {
        let significant: Vec<_> = tokens.iter().filter(|t| t.significant()).collect();
        if significant.is_empty() {
            return;
        }
        let first = significant[0];
        let last = significant.last().unwrap();
        if let Token::AtKeyword(name) = &first.token {
            let id = CssRuleId(self.css.rules.len());
            self.css.rules.push(CssRule {
                id,
                parent,
                span: self.input.range(first.start, end),
                prelude: self.input.source_text(self.source, first.start, last.end),
                kind: CssRuleKind::AtRule(name.to_string()),
                selectors: Vec::new(),
                conditions: conditions.to_vec(),
            });
            if name.eq_ignore_ascii_case("import") {
                if let Some(value) = significant.get(1) {
                    let url = match &value.token {
                        Token::QuotedString(s) | Token::UnquotedUrl(s) => Some(s.to_string()),
                        Token::Function(s) if s.eq_ignore_ascii_case("url") => {
                            value.children.iter().find_map(|t| match &t.token {
                                Token::QuotedString(s) | Token::Ident(s) => Some(s.to_string()),
                                _ => None,
                            })
                        }
                        _ => None,
                    };
                    if let Some(url) = url {
                        self.css.dependencies.push(CssDependency {
                            kind: CssDependencyKind::Import,
                            value: url,
                            span: self.input.range(value.start, value.end),
                        });
                    }
                }
                self.values(tokens, true);
                return;
            }
        }
        self.values(tokens, false);
    }
    fn declaration(
        &mut self,
        tokens: &[Lex],
        rule: Option<CssRuleId>,
        conditions: &[SourceText],
        end: usize,
    ) {
        let Some(colon) = tokens.iter().position(|t| matches!(t.token, Token::Colon)) else {
            return;
        };
        let Some(name) = tokens[..colon].iter().find(|t| t.significant()) else {
            return;
        };
        let Token::Ident(value) = &name.token else {
            return;
        };
        let values = &tokens[colon + 1..];
        let significant: Vec<_> = values.iter().filter(|t| t.significant()).collect();
        let important = significant.len() >= 2
            && matches!(&significant[significant.len()-1].token,Token::Ident(s) if s.eq_ignore_ascii_case("important"))
            && matches!(significant[significant.len() - 2].token, Token::Delim('!'));
        let value_end = if important {
            significant[significant.len() - 2].start
        } else {
            significant.last().map_or(tokens[colon].end, |t| t.end)
        };
        let value_start = significant
            .first()
            .map_or(tokens[colon].end, |t| t.start)
            .min(value_end);
        let trimmed_end = values
            .iter()
            .filter(|t| t.significant() && t.end <= value_end)
            .next_back()
            .map_or(value_start, |t| t.end);
        let id = CssDeclarationId(self.css.declarations.len());
        self.css.declarations.push(CssDeclaration {
            id,
            rule,
            span: self.input.range(name.start, end),
            name: value.to_string(),
            name_span: self.input.range(name.start, name.end),
            value: self
                .input
                .source_text(self.source, value_start, trimmed_end),
            important,
        });
        if value.starts_with("--") {
            self.css.definitions.push(CssDefinition {
                name: value.to_string(),
                span: self.input.range(name.start, name.end),
                kind: CssDefinitionKind::CustomProperty,
                declaration: Some(id),
                conditions: conditions.to_vec(),
            });
        }
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "animation-name" | "animation" | "-webkit-animation" | "-webkit-animation-name"
        ) {
            let tokens = &values[..values.partition_point(|t| t.end <= value_end)];
            for component in tokens.split(|t| matches!(t.token, Token::Comma)) {
                let significant: Vec<_> = component.iter().filter(|t| t.significant()).collect();
                let (Some(first), Some(last)) = (significant.first(), significant.last()) else {
                    continue;
                };
                let property = Property::parse_string(
                    PropertyId::from(value.as_ref()),
                    &self.input.code[first.start..last.end],
                    ParserOptions::default(),
                );
                let candidates = if matches!(property, Ok(Property::Unparsed(_))) {
                    let potential: Vec<_> = significant
                        .iter()
                        .copied()
                        .filter(|t| {
                            matches!(t.token, Token::Ident(_) | Token::QuotedString(_))
                                && Property::parse_string(
                                    PropertyId::from("animation-name"),
                                    &self.input.code[t.start..t.end],
                                    ParserOptions::default(),
                                )
                                .as_ref()
                                .ok()
                                .and_then(animation_name)
                                .is_some_and(|name| !matches!(name, AnimationName::None))
                        })
                        .collect();
                    let forced: Vec<_> = potential
                        .iter()
                        .copied()
                        .filter(|t| {
                            Property::parse_string(
                                PropertyId::from(value.as_ref()),
                                &self.input.code[t.start..t.end],
                                ParserOptions::default(),
                            )
                            .as_ref()
                            .ok()
                            .and_then(animation_name)
                            .is_some_and(|name| !matches!(name, AnimationName::None))
                        })
                        .collect();
                    if forced.is_empty() { potential } else { forced }
                } else {
                    let name = property.as_ref().ok().and_then(animation_name);
                    significant
                        .iter()
                        .copied()
                        .find(|t| {
                            let matches_name = match name {
                                Some(AnimationName::Ident(name)) => {
                                    matches!(&t.token, Token::Ident(s) if s.as_ref() == name.0.as_ref())
                                }
                                Some(AnimationName::String(name)) => {
                                    matches!(&t.token, Token::QuotedString(s) if s.as_ref() == name.as_ref())
                                }
                                _ => false,
                            };
                            matches_name
                                && Property::parse_string(
                                    PropertyId::from(value.as_ref()),
                                    &self.input.code[first.start..t.end],
                                    ParserOptions::default(),
                                )
                                .as_ref()
                                .ok()
                                .and_then(animation_name)
                                    == name
                        })
                        .into_iter()
                        .collect()
                };
                for token in candidates {
                    if let Token::Ident(name) | Token::QuotedString(name) = &token.token {
                        self.css.references.push(CssReference {
                            name: name.to_string(),
                            span: self.input.range(token.start, token.end),
                            kind: CssReferenceKind::Animation,
                            fallback: None,
                            candidates: Vec::new(),
                            runtime_dependent: true,
                        });
                    }
                }
            }
        }
        self.values(values, false);
    }
    fn values(&mut self, tokens: &[Lex], import: bool) {
        for token in tokens {
            match &token.token {
                Token::Function(name) if name.eq_ignore_ascii_case("var") => {
                    let values: Vec<_> =
                        token.children.iter().filter(|t| t.significant()).collect();
                    if let Some(first) = values.first()
                        && let Token::Ident(name) = &first.token
                    {
                        let fallback = values
                            .iter()
                            .position(|t| matches!(t.token, Token::Comma))
                            .map(|comma| {
                                let start =
                                    values.get(comma + 1).map_or(values[comma].end, |t| t.start);
                                let end = values.last().map_or(start, |t| t.end).max(start);
                                self.input.source_text(self.source, start, end)
                            });
                        self.css.references.push(CssReference {
                            name: name.to_string(),
                            span: self.input.range(first.start, first.end),
                            kind: CssReferenceKind::CustomProperty,
                            fallback,
                            candidates: Vec::new(),
                            runtime_dependent: true,
                        });
                    }
                }
                Token::UnquotedUrl(value) if !import => self.css.dependencies.push(CssDependency {
                    kind: CssDependencyKind::Url,
                    value: value.to_string(),
                    span: self.input.range(token.start, token.end),
                }),
                Token::Function(name) if name.eq_ignore_ascii_case("url") && !import => {
                    if let Some(value) = token.children.iter().find_map(|t| match &t.token {
                        Token::QuotedString(s) | Token::Ident(s) => Some(s.to_string()),
                        _ => None,
                    }) {
                        self.css.dependencies.push(CssDependency {
                            kind: CssDependencyKind::Url,
                            value,
                            span: self.input.range(token.start, token.end),
                        });
                    }
                }
                _ => {}
            }
            self.values(&token.children, import);
        }
    }
    fn selectors(&self, tokens: &[Lex]) -> Vec<CssSelector> {
        let mut result = Vec::new();
        let mut start = 0;
        for end in 0..=tokens.len() {
            if end == tokens.len() || matches!(tokens[end].token, Token::Comma) {
                let parts: Vec<_> = tokens[start..end]
                    .iter()
                    .filter(|t| t.significant())
                    .collect();
                if let (Some(first), Some(last)) = (parts.first(), parts.last()) {
                    let mut selector = CssSelector {
                        span: self.input.range(first.start, last.end),
                        raw: self
                            .input
                            .source_text(self.source, first.start, last.end)
                            .raw,
                        classes: Vec::new(),
                        ids: Vec::new(),
                        global: false,
                        nesting: false,
                    };
                    self.selector_names(&tokens[start..end], &mut selector);
                    result.push(selector);
                }
                start = end + 1;
            }
        }
        result
    }
    fn selector_names(&self, tokens: &[Lex], selector: &mut CssSelector) {
        for (index, token) in tokens.iter().enumerate() {
            match &token.token {
                Token::Delim('.') => {
                    if let Some(next) = tokens[index + 1..].iter().find(|t| t.significant())
                        && let Token::Ident(name) = &next.token
                    {
                        selector.classes.push(CssName {
                            name: name.to_string(),
                            span: self.input.range(next.start, next.end),
                        });
                    }
                }
                Token::Hash(name) | Token::IDHash(name) => selector.ids.push(CssName {
                    name: name.to_string(),
                    span: self.input.range(token.start, token.end),
                }),
                Token::Delim('&') => selector.nesting = true,
                Token::Function(name) => {
                    if name.as_ref() == "global" {
                        selector.global = true;
                    }
                    self.selector_names(&token.children, selector);
                }
                _ => {}
            }
        }
    }
}

struct AtPrelude<'i> {
    name: CowRcStr<'i>,
    name_end: usize,
    tokens: Vec<Lex<'i>>,
    end: usize,
}
struct Rules<'c, 's> {
    collector: &'c mut Collector<'s>,
    parent: Option<CssRuleId>,
    conditions: Vec<SourceText>,
    declarations: bool,
}
impl Rules<'_, '_> {
    fn block<'i>(
        &mut self,
        prelude: Vec<Lex<'i>>,
        input: &mut Parser<'i, '_>,
    ) -> Result<(), ParseError<'i, ()>> {
        let (id, conditions) = self.collector.rule(&prelude, self.parent, &self.conditions);
        let mut nested = Rules {
            collector: self.collector,
            parent: Some(id),
            conditions,
            declarations: true,
        };
        for result in RuleBodyParser::new(input, &mut nested) {
            result.map_err(|(error, _)| error)?;
        }
        let end = input.position().byte_index();
        let end = end + usize::from(self.collector.input.code.as_bytes().get(end) == Some(&b'}'));
        self.collector.css.rules[id.0].span = self.collector.input.range(prelude[0].start, end);
        self.collector.values(&prelude, false);
        Ok(())
    }
}
impl<'i> DeclarationParser<'i> for Rules<'_, '_> {
    type Declaration = ();
    type Error = ();
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        start: &ParserState,
    ) -> Result<(), ParseError<'i, ()>> {
        let values = tokenize(input, 0);
        if !name.starts_with("--")
            && values
                .iter()
                .any(|t| matches!(t.token, Token::CurlyBracketBlock))
        {
            return Err(input.new_custom_error(()));
        }
        let begin = start.position().byte_index();
        let value_start = values
            .first()
            .map_or(input.position().byte_index(), |t| t.start);
        let mut prefix_input = ParserInput::new(&self.collector.input.code[begin..value_start]);
        let mut tokens = tokenize(&mut Parser::new(&mut prefix_input), begin);
        tokens.extend(values);
        let end = input.position().byte_index();
        let end = end + usize::from(self.collector.input.code.as_bytes().get(end) == Some(&b';'));
        self.collector
            .declaration(&tokens, self.parent, &self.conditions, end);
        Ok(())
    }
}
impl<'i> AtRuleParser<'i> for Rules<'_, '_> {
    type Prelude = AtPrelude<'i>;
    type AtRule = ();
    type Error = ();
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<AtPrelude<'i>, ParseError<'i, ()>> {
        let name_end = input.position().byte_index();
        let tokens = tokenize(input, 0);
        Ok(AtPrelude {
            name,
            name_end,
            tokens,
            end: input.position().byte_index(),
        })
    }
    fn rule_without_block(
        &mut self,
        mut prelude: AtPrelude<'i>,
        start: &ParserState,
    ) -> Result<(), ()> {
        prelude.tokens.insert(
            0,
            Lex {
                token: Token::AtKeyword(prelude.name),
                start: start.position().byte_index(),
                end: prelude.name_end,
                children: Vec::new(),
            },
        );
        let end = prelude.end
            + usize::from(self.collector.input.code.as_bytes().get(prelude.end) == Some(&b';'));
        self.collector
            .statement(&prelude.tokens, self.parent, &self.conditions, end);
        Ok(())
    }
    fn parse_block<'t>(
        &mut self,
        mut prelude: AtPrelude<'i>,
        start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        prelude.tokens.insert(
            0,
            Lex {
                token: Token::AtKeyword(prelude.name),
                start: start.position().byte_index(),
                end: prelude.name_end,
                children: Vec::new(),
            },
        );
        self.block(prelude.tokens, input)
    }
}
impl<'i> QualifiedRuleParser<'i> for Rules<'_, '_> {
    type Prelude = Vec<Lex<'i>>;
    type QualifiedRule = ();
    type Error = ();
    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Vec<Lex<'i>>, ParseError<'i, ()>> {
        Ok(tokenize(input, 0))
    }
    fn parse_block<'t>(
        &mut self,
        prelude: Vec<Lex<'i>>,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        self.block(prelude, input)
    }
}
impl<'i> RuleBodyItemParser<'i, (), ()> for Rules<'_, '_> {
    fn parse_declarations(&self) -> bool {
        self.declarations
    }
    fn parse_qualified(&self) -> bool {
        true
    }
}

fn animation_name<'p, 'i>(property: &'p Property<'i>) -> Option<&'p AnimationName<'i>> {
    match property {
        Property::Animation(list, _) => list.first().map(|a| &a.name),
        Property::AnimationName(list, _) => list.first(),
        _ => None,
    }
}
