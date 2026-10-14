# ADR 0001: Owned facts from single-file Astro analysis

Status: Accepted

Date: 2026-10-14

## Context

Atlas extracts source-positioned facts from one Astro file: imports, declarations, scopes, references, template elements, scripts, styles, built-in uses, and diagnostics. Consumers need declaration identity and recoverable source text without retaining a parser allocator or borrowing an AST. A component tag must resolve to the declaration selected by lexical scope, including shadowing, rather than to any declaration with the same name.

An Astro file combines server-side frontmatter and template expressions with browser scripts and CSS. Those languages have different parsing rules and scope domains. Script attributes and parent elements affect extraction eligibility, language, and `define:vars` behavior. CSS in a static style attribute may contain HTML entities or JavaScript string escapes, so decoded CSS positions differ from positions in the Astro source.

## Decision

### Public contract and ownership

Expose `analyze(&str) -> Result<FileFacts, AnalysisError>`. Successful facts and error diagnostics own their strings and positions. The input, Oxc AST, semantic data, and allocators may be released after the call. Public IDs identify records within the containing result; Oxc IDs and borrowed AST nodes remain internal. Every public span is a UTF-8 byte half-open interval `[start, end)` in the original Astro source. `SourceText.raw` is the corresponding original slice.

Astro and script parser failures, semantic syntax diagnostics, and standard CSS syntax diagnostics return `AnalysisError` without partial facts. Astro parsing must succeed before server semantic analysis starts; each script must parse successfully before its own semantic analysis. A syntactically valid unresolved reference remains a fact. Static Astro convention violations accompany `Ok(FileFacts)`, which can contain diagnostics with `Severity::Error`.

### Oxc parsing and semantic projection

Use the pinned Astro-capable Oxc fork for parsing and semantic analysis. The initial Astro parse disables `parse_astro_script_content`, retaining script attributes and body spans while deferring browser grammar selection. Server semantics cover frontmatter and template expressions. The shared `semantic::project` function converts Oxc imports, symbols, scopes, references, and built-in uses into owned records. Resolved references follow Oxc `SymbolId` identity and receive an Atlas `DeclarationId`; equal names do not merge declarations.

Discover template elements, attributes, component uses, script inputs, style inputs, and injection origins through the Astro AST visitor. Analyze each eligible script body separately: processed scripts use TSX/module grammar, inline modules use JavaScript module grammar, and inline classic scripts use JavaScript script grammar. External, data, and dynamically typed scripts retain metadata and source without parsing their bodies. Padding preserves original body byte offsets.

Keep `Server`, `BrowserModule`, `BrowserClassic`, and `InjectedScript` scope domains explicit. Static `define:vars` keys create distinct script-local declarations after name conversion and grammar validation. Their origins retain the original key span, value text, and references to server declarations. Classic scripts with `define:vars` use the injected domain even when no static bindings can be extracted. Computed keys, spreads, and possible globals supplied by other classic scripts use `RuntimeDependent` resolutions and declaration candidates where available. They do not establish definite cross-script lexical bindings.

### CSS semantics and original-source positions

Use Lightning CSS 1.0.0-alpha.72 to validate standard stylesheets and style attributes. During collection, use its typed property parsing to identify animation names and distinguish shorthand slots. Unparsed dynamic values retain conservative candidates.

Use cssparser 0.37.0 rule/declaration callbacks and tokens to collect source order, duplicate declarations, rule nesting, conditions, `!important`, selectors, custom-property references and fallbacks, keyframes, and URL dependencies. Token byte positions identify exact source ranges. CSS references retain matching definition candidates across styles in the file; they do not claim a winning cascade value.

`CssSource` maps each decoded CSS byte to its original Astro byte range. Style bodies use a direct map. Static style attributes decode HTML entities or JavaScript string escapes, including line continuations, while retaining their origins. Collected spans and CSS diagnostic locations are translated through this map, and raw text is sliced from the original source. CSS escapes are interpreted by cssparser while their original spelling remains recoverable. Dynamic style expressions and non-CSS languages retain metadata and source; Atlas does not evaluate expressions or run preprocessors.

### Responsibilities and current modules

Template, script, and style are logical responsibilities, not three separate public APIs or a requirement for one module per responsibility.

| Responsibility | Current modules |
| --- | --- |
| Entry point and sequencing | `lib.rs` exports the API and fact types. `analyze.rs` orchestrates parsing, server semantics, template discovery, script analysis, style injection and CSS analysis, Astro validation, and final ordering. It also converts Oxc diagnostics and remaps reference IDs after sorting. |
| Template | `markup.rs` visits elements, classifies components, retains attributes/text/directives, selects script/style modes and extraction eligibility, and discovers `define:vars` origins. `validate.rs` checks static Astro conventions and updates directive status. |
| Script | `scripts.rs` selects body grammar, creates and validates injection declarations, performs independent semantic analysis, and records possible classic-script globals. It consumes inputs classified by `markup.rs`. |
| Style | `css_source.rs` decodes CSS inputs and maps positions to original source. `css.rs` validates and collects standard CSS facts and links definition candidates. `markup.rs` supplies style metadata and effective injection seeds; `analyze.rs` converts those seeds into CSS injections. |
| Shared owned model | `facts.rs` defines the public records, IDs, scope domains, resolutions, source text, and diagnostics used by every responsibility. |
| Shared syntax and semantics | `syntax.rs` converts spans, attributes, literals, and static keys and builds diagnostics. `semantic.rs` projects Oxc semantics for both the server and independently parsed scripts into the common owned model. |

The official `@astrojs/compiler-rs` 0.3.0 package supplies test evidence for parsing, template positions, and extraction behavior. Node and the compiler package belong to the test harness; the Rust analysis API uses Oxc, Lightning CSS, cssparser, and HTML entity decoding at runtime. Compiler comparison does not establish lexical declaration identity or CSS resolution.

## Alternatives

| Alternative | Assessment |
| --- | --- |
| Publish a borrowed Oxc AST and semantic result | Exposes allocator lifetimes, Oxc types, and dependency IDs to consumers. An owned projection gives a result boundary that can outlive parser data, at the cost of copying strings and records. |
| Implement Astro, JavaScript/TypeScript, and CSS parsing and name resolution ourselves | Requires maintaining language grammar, lexical scope, escape rules, and typed CSS value behavior. Reuse Oxc and CSS parsers; implement the Astro-specific classification, projection, injection, and source mapping in Atlas. |
| Use only Lightning CSS to obtain CSS facts and positions | Its parsed representation provides syntax and typed values, but does not provide all original token spans and declaration ordering needed by this contract. cssparser supplies token boundaries and ordered collection; `CssSource` handles decoded attributes for both parsers. |

## Consequences

Consumers receive owned facts with declaration identity, explicit scope domains, and recoverable original-source spans. Parser details and temporary lifetimes are contained within analysis. Final reference sorting remaps links in elements, components, scripts, and injection origins so that IDs remain consistent within the result.

Ownership allocates strings and records. Browser programs receive separate parse/semantic passes, and standard CSS is parsed for validation and again for ordered collection. Atlas must maintain the Oxc fork option, coordinate the pinned parser versions, and test decoding and source mapping boundaries.

The contract is limited to one file. Import targets are recorded without analyzing imported implementations. Project-wide globals, renderer availability, custom directive registration, routing, content collections, actions, and adapters require additional project information. Atlas performs neither TypeScript type checking nor HTML generation, browser execution, CSS cascade/inheritance evaluation, final animation selection, or preprocessor execution. Dynamic values retain source and uncertainty. Callers must inspect successful-result diagnostics as well as handle `AnalysisError`.

## References

- [Public facts](../../src/facts.rs), [analysis entry point](../../src/analyze.rs), and [shared semantic projection](../../src/semantic.rs).
- [Template discovery](../../src/markup.rs), [script analysis](../../src/scripts.rs), [CSS collection](../../src/css.rs), and [CSS source mapping](../../src/css_source.rs).
- [Specification coverage and limits](../specification.md), [fixture and verification strategy](../testing.md), and [Oxc fork changes and evidence](../oxc-patch.md).
- [Pinned dependencies](../../Cargo.toml) and [test-only compiler package](../../package.json).
- [Lightning CSS stylesheet parsing](https://docs.rs/lightningcss/1.0.0-alpha.72/lightningcss/stylesheet/struct.StyleSheet.html) and [typed property parsing](https://docs.rs/lightningcss/1.0.0-alpha.72/lightningcss/properties/enum.Property.html).
- [cssparser token positions](https://docs.rs/cssparser/0.37.0/cssparser/struct.Parser.html) and [ordered rule/declaration parsing](https://docs.rs/cssparser/0.37.0/cssparser/struct.RuleBodyParser.html).
