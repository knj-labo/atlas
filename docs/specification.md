# Analysis specification coverage

The release target is Astro **7.0.6**, with the published **@astrojs/compiler-rs 0.3.0** package as a reproducible syntax/transform oracle. This matrix defines the analysis contract; it does not claim to implement the Astro runtime or every project-level feature. Every supported row names a Rust test or a pinned compiler fixture. Results below were measured, not inferred solely from upstream code.

## Status and error contract

- **Supported**: owned facts and original UTF-8 byte spans are tested.
- **Dynamic**: expressions and references are preserved; evaluation is not attempted.
- **Project required**: renderer/import/configuration information is absent from a single source.
- **Out of scope**: compilation, type checking, rendering, runtime APIs, and CSS cascade evaluation.

`analyze` returns `Err` for parser, semantic syntax, or standard-CSS syntax errors. Parsing errors gate semantic analysis. Astro validity diagnostics are in successful facts, with phase, severity, code and owned labels. Undefined identifiers are references, not syntax errors. A successful result may have Astro error diagnostics. Known directives receive Valid, Invalid, Dynamic or RequiresProjectContext status. Custom directives remain UnknownDirective; this is not an assertion that a configured project rejects them.

IDs index their containing arrays, and do not correspond to Oxc's numeric IDs. Declaration and scope IDs are local to one analysis result, not stable across source revisions; CSS rule/declaration IDs are style-local. References are ordered by source position. Elements retain tree traversal order and parent IDs; styles retain element/attribute traversal order. Raw source text and spans always use the original input, including decoded CSS attributes.

## Coverage matrix

All rows marked Supported below passed. Rust tests are in [spec.rs](../tests/spec.rs), unless another suite is named. Compiler fixtures are in [tests/spec](../tests/spec).

| Feature | Expected and measured result | Status / boundary | Reproduction |
| --- | --- | --- | --- |
| Frontmatter, empty files, no frontmatter | Server scopes and owned declarations/references; empty files accepted | Supported; JS/TS grammar delegated to pinned Oxc | `all_references_scopes_types_and_declarations_are_owned`; phase-1 empty-file test; `empty`, `types-await` fixtures |
| Default, aliased named, namespace, side-effect and type imports | Import source and form, local binding, type-only marker; shadowing resolves to different IDs | Supported; imported files not opened | `file_facts::imports_include_aliases_types_namespace_and_side_effects`; phase-1 import test |
| Variables, destructuring, functions, parameters, classes and TS declarations | Scope parents, declaration identities, Value/Type/ValueAndType references | Supported; no TS type inference | `all_references_scopes_types_and_declarations_are_owned`; `receivers_type_scopes_and_module_expression_attributes_are_explicit` |
| Frontmatter exports | Top-level exports including `export * as` and `export *` (named `*`): name, span, local declaration, re-export source and imported specifier, type-only marker from Oxc symbol meaning; role Astro (value `getStaticPaths`, `prerender`, `partial`), Props (type `Props`) or Other; the implicit default component export is not listed | Supported; client script exports excluded; re-exports resolved at project level | `tests/exports.rs` |
| Component names, members and closing tags | One component use per element; member root resolves to declaration; opening and closing spans retained | Supported; property values not evaluated | phase-1 member/import/shadowing tests; `members`, `components` fixtures |
| HTML, custom elements, fragments, attributes, spreads, duplicates | Every element and attribute retained with spans, values, literals, parent links | Supported; attribute spread contents are Dynamic | `every_template_element_attributes_spreads_fragments_and_slots_are_kept`; `attributes`, `html` fixtures |
| Expressions, arrays, objects, IIFEs, async/await | References retained in nested expressions; nested scripts discovered independently | Supported; branch conditions not evaluated | `script_discovery_covers_array_object_and_iife_return_expressions`; `expressions`, `types-await` fixtures |
| `this`, `super`, dynamic tags | Receiver references are RuntimeDependent; variable-backed components retain lexical root identity | Dynamic; receiver object and tag value not evaluated | `receivers_type_scopes_and_module_expression_attributes_are_explicit`; `dynamic_tags_cannot_be_used_for_client_hydration` |
| `Astro`, `Astro.self`, `Fragment` | Unshadowed builtins recorded, static API member paths retained; local Fragment shadows builtin | Supported; API behavior and TS signatures out of scope | `astro_self_and_fragment_are_builtins_but_local_fragment_can_shadow`; `astro_members_are_recorded_and_unknown_members_are_diagnosed`; `api` fixture |
| Astro props, params, url, request, response, cookies, locals, slots, session, cache, csp, actions, locale and route APIs | Root reference and static member paths recorded; unknown first member warned | Supported facts; context and values Project required | `api`, `types-await` fixtures; Astro-member Rust test |
| Named/default slots, fallback, Fragment slot assignments | Slot attributes, dynamic names and fallback text retained | Supported facts; receiving component and slot rendering Project required | `every_template_element_attributes_spreads_fragments_and_slots_are_kept`; `fragments-slots` fixture |
| `class:list`, `set:html`, `set:text` | Literal form, source and references retained; dynamic expressions not executed | Supported facts / Dynamic values; DOM/serialization out of scope | `attributes`, `set-directives` fixtures |
| `client:load`, `idle`, `visible`, `media`, `only` | Component requirement, dynamic-tag restriction and static media string checked; directive values retained | Supported basic diagnostics; renderer, component kind and option schemas Project required | `static_directive_violations_are_diagnostics_and_dynamic_values_are_kept`; `components`, `attributes`, `unicode-crlf` fixtures |
| Custom `client:*` and other namespace attributes | Preserved, classified RequiresProjectContext or UnknownDirective | Project required; no registration lookup | same directive test; `attributes`, `unknown-components` fixtures |
| `server:defer` | Component target checked; attributes and refs preserved | Supported basic diagnostics; server islands and adapter Project required | directive Rust test; `server-transitions` fixture |
| `transition:name`, `animate`, `persist`, `persist-props` | Names, literal/custom animation values and expressions retained; static name type checked | Supported basic facts; navigation/persistence runtime out of scope | `attributes`, `server-transitions` fixtures |
| `is:raw`, comments, strings, raw script data | Raw text never becomes fake component uses or JS refs | Supported | `raw_children_comments_and_strings_never_create_uses_or_references`; `raw`, `data` fixtures |
| Processed scripts and external src | Independent TS/module domain and bundling eligibility; external src preserved without fetching it | Supported; external file Project required | `processed_inline_module_classic_external_data_and_dynamic_modes`; `processed`, `external` fixtures |
| `type="module"` and spread attributes | Literal type alone is eligible for bundling in 0.3.0; expression type is inline JS module | Supported, release-specific | `module-hoisted`, `module-expression`, `module-spread`, `external-module` fixtures |
| `is:inline`, other attrs, classic JS and inline module JS | Separate JS parses; invalid browser TS/imports produce errors | Supported; globals not equated with server bindings | script-mode Rust test; `classic`, `inline-module`, `defer`, `invalid-browser-ts` fixtures |
| `svg`, `noscript`, `template` boundaries | Script bodies use browser JS; literal external sources remain External/JavaScript; styles stay inline | Supported | `scripts_in_non_hoistable_context_are_browser_javascript`; `nonhoistable` fixture |
| Data and dynamic script type | Data body raw; unresolved dynamic type marked Unknown, not parsed as JS | Supported descriptor / Dynamic behavior | script-mode Rust test; `data`, `dynamic-type` fixtures |
| Classic-script globals | Another script's matching root declarations are runtime candidates; no definite cross-script binding | Dynamic | `inline_classic_globals_are_candidates_not_definite_lexical_bindings` |
| Script `define:vars` aliases, shorthand, computed literal keys, normalization | New injected IDs and key/value/origin-reference spans; nested parameters shadow injection | Supported static keys; dynamic keys/spreads unknown | `define_vars_aliases_keys_and_nested_shadowing_have_distinct_identity`; phase-1 define-vars test; `injection` fixture |
| Injection redeclarations, duplicate and invalid keys | Same-scope collisions produce semantic errors; duplicate object keys use final value; invalid normalized keys produce Astro diagnostics | Supported static keys; expression serialization not executed | `define_vars_static_collision_is_an_error_and_dynamic_keys_remain_unknown`; `define_vars_duplicate_and_computed_literal_keys_follow_object_semantics` |
| Explicit server/browser value transfer | Rust-only owned boundaries for script `define:vars` and known client component props; per-input method support, reasons, source evidence and unverified assumptions | Conservative static values and primitive const aliases; runtime execution, renderer availability, disclosure and optimization safety require separate checks | `tests/transfer.rs`; [ADR 0005](adr/0005-semantic-boundaries-and-transfer.md) |
| Scoped/global/inline styles, language attrs and style attributes | Mode, original content and language retained; standard CSS parsed | Supported | `style_modes_preprocessors_and_dynamic_style_values_are_explicit`; `styles`, `inline-css` fixtures |
| SCSS, Sass, Less, dynamic lang, dynamic style objects | Original expression/language retained, no false standard-CSS syntax error | Dynamic / out of scope for preprocessor execution | style-mode/receiver Rust tests; `styles` fixture |
| Style `define:vars` | Effective extracted styles retain custom-property injection names, original key/value and server refs; inline/non-hoistable styles retain attributes without injected definitions | Supported static keys; dynamic injection flagged | `css_custom_properties_registrations_keyframes_urls_fallbacks_and_injection`; `style-injection` fixture |
| CSS style, keyframe and at-rules | Parent rules, selectors, nesting, conditions and original preludes retained | Supported; selector matching/cascade out of scope | `css_rules_selectors_duplicate_declarations_important_conditions_and_nesting`; `css-nesting` fixture |
| CSS declarations | Source order, duplicate names and `!important` retained without optimization | Supported; no minification/code generation | same CSS-rules test |
| CSS custom properties, `@property`, `var()` and fallback | Definitions and reference candidates, nested and empty fallback retained; comments/strings ignored | Supported candidates, always runtime-dependent | CSS custom-property and comment/string tests |
| CSS keyframes and animation values | Keyframe definitions, step rules and animation-name candidates retained, including quoted/repeated keywords and conservative candidates in variable-containing values | Supported; runtime animation value not selected | `animation_names_with_keyword_spellings_and_keyframe_steps_are_kept` |
| CSS `@import` and `url()` | Dependency spelling and original spans retained | Supported facts; external resources not fetched | CSS custom-property dependency test |
| CSS token escapes and conditions | Decoded identifiers match definition candidates; raw escaped spans retained | Supported | `css_escaped_names_conditions_and_cross_style_candidates_keep_raw_positions` |
| Unicode, emoji, CRLF, entities and JS escapes | All tested spans reconstruct original input, including CSS syntax labels | Supported; no source rewriting | CSS span/error tests; phase-1/phase-2 spans; `unicode-crlf`, `inline-css` fixtures |
| Malformed input vs unresolved identifiers | Syntax Err, semantic gated; undefined tag remains Ok/Unresolved | Supported | phase-1 malformed/undefined tests; `invalid-*` fixtures |
| Routing, collections, middleware, integrations, custom renderers/directives, actions, adapters | Expressions/imports remain available as facts | Project required; no project graph or config execution | Contract boundary; no runtime implementation |
| File-level incremental analysis | Optional `AnalysisDatabase` manages caller-supplied paths and text; unchanged files reuse owned results and syntax errors | Supported with `incremental`; no filesystem access or import resolution | `incremental` unit tests; `tests/incremental.rs` |
| External ESM JS/TS | Path-selected grammar, Module scopes, JSX and type/value references, named/default/type exports; anonymous defaults retain the full export range | Supported; no Astro builtins/roles/validation, CommonJS or type inference | `tests/external_sources.rs` |
| Project component uses and unused exports | `component_uses_of` returns template uses with their elements; re-export APIs expose saved edges; `unused_exports` recognizes default/named/namespace imports and re-exports, with default excluded from stars | Supported candidates; re-export resolvers receive the actual domain; unresolved modules and unregistered consumers are not seen | `tests/project.rs`; `tests/definitions.rs` |
| Cross-file definitions | Borrowed local/export/file targets through named/default imports, aliases and re-exports; explicit priority, deduplicated targets, ambiguity and reasoned failure | Supported; no member resolution or type inference | `tests/definitions.rs`; `tests/project_incremental.rs` |
| Project report (`serde`) | Shared Serde summary with diagnostics, exports, imports, imported references and definitions, component uses, unused-export keys; schema 1 and `path#start-end` keys | Supported; example defaults to TOON, with JSON available | `tests/report.rs`; `examples/atlas_report.rs`; [ADR 0006](adr/0006-esm-definitions-and-toon.md) |
| Project import and file-usage search | Immutable source/analysis snapshots, explicit module resolution states, direct importers and definite lexical uses, versioned invalidation on source/file/resolver changes | Supported with default features; `incremental` optionally caches results; default resolver matches explicit relative virtual paths | `tests/project.rs`; [ADR 0003](adr/0003-project-import-search.md) |
| TS checking, rendering, runtime APIs, browser DOM, CSS cascade, preprocessors, CLI | Not implemented | Out of scope | Contract boundary |

## File-local search API

`analyze_file(path, source)` selects Astro or external ESM syntax without reading
the path. Supported external extensions are `.js`, `.mjs`, `.jsx`, `.ts`, `.mts`,
`.tsx`, `.d.ts` and `.d.mts`. Grammar comes from the pinned Oxc
`SourceType::from_path`, including its JSX and declaration-file rules. External
scopes have `ScopeDomain::Module`; no server/browser environment is inferred.
Astro builtins, special export roles, template validation and transfer boundaries
apply only to Astro sources. External default exports reference a local ID when
available, otherwise their whole export span. Explicit `.cjs`, `.cts` and
`.d.cts` return `atlas/unsupported-source-type`; CommonJS semantics are not
implemented. Other virtual keys retain Astro syntax. `analyze(source)` stays
Astro-only. Parser/semantic failures use the common `Source analysis failed`
display prefix and retain their original diagnostics/spans.

`FileFacts::declarations_named(name)` returns borrowed `Binding` records with an exact, case-sensitive name match. Each lexical declaration keeps its own ID and scope; an import is found under its local alias. `FileFacts::reference_at(offset)` returns a borrowed `Reference` when `span.start <= offset < span.end`, retaining its complete resolution state. Offsets use original-source UTF-8 bytes, including bytes within a multibyte character; end-of-file and out-of-range positions return `None`. Only recorded reference spans match: a component member's root is a reference in both opening and closing tags, while its property names are not.

`FileFacts::references_to(id)` returns borrowed references whose `Resolution::Resolved` binding has that declaration ID. Value and type references are included. Unresolved identifiers, builtins, and runtime-dependent candidates do not count as definite uses. Missing names or unknown declaration IDs yield empty vectors. Multiple results are ordered by `(span.start, span.end, id)` without reordering stored records or changing IDs.

The methods cover frontmatter, template, and eligible browser-script records already in `FileFacts`. CSS candidates and cross-file lookup are outside this search API. Queries perform no analysis or I/O and introduce no new error type. IDs must come from the same result; a numeric ID reused by another result cannot be distinguished. Tests are in [queries.rs](../tests/queries.rs), with cached-result and update coverage in [incremental.rs](../tests/incremental.rs).

## Boundary transfer API

`FileFacts.boundaries` records explicit handoffs independently of lexical scopes.
`BoundaryFact.evaluation` identifies where inputs are evaluated, and
`destination` identifies a browser domain when its mode is established.
`FileFacts::assess_transfer(id)` returns an owned `TransferAssessment`, or `None`
for an invalid result-local ID. It uses collected value facts without parsing,
I/O, or source execution. Inputs retain their names, original source text/spans,
reference IDs, and primitive constant origins. Assessment reasons identify
support or missing evidence at original UTF-8 byte positions.

Script `define:vars` uses `JsonScript`; known client component directives use
`AstroProps`. Directive attributes and `slot` are excluded from props. Primitive
literals, simple same-file primitive const aliases, and static arrays/plain
objects are supported. BigInt is supported only by `AstroProps`; direct function
values are unsupported by both methods. Identifier-backed arrays/objects,
mutable bindings, calls, imports, dynamic keys, spreads, and unsupported
expressions stay unknown. This classification does not infer object immutability
from `const` or evaluate arbitrary expressions.

`Supported` concerns the chosen value format. The result's shared `assumptions`
also apply to every input: directive validity, duplicate/dynamic input sets,
unconfirmed script execution, renderer availability, data disclosure, and code
movement require independent decisions. Syntactic duplicate inputs are retained
without selecting an effective value. Custom client directives alone have no
inferred boundary method. Empty input sets have empty assessment input lists.
The API is initially Rust-only: new facts are excluded from existing FileFacts
JSON, and ProjectReport schema version 1 is unchanged. Details and compiler
applications are in [ADR 0005](adr/0005-semantic-boundaries-and-transfer.md).

## Incremental API

Enable the `incremental` Cargo feature to use `AnalysisDatabase::new()` or `Default`. `set_source(path, text)` registers or replaces complete source text without analyzing it. `analyze_file(path)` returns `None` for an unregistered key and otherwise returns an `Arc<Result<FileFacts, AnalysisError>>`. Each Salsa input retains its path and uses the same grammar selection as `analyze_file(path, source)`. It reuses successful facts and syntax errors until the file changes; the next request analyzes its latest full text. Identical text updates are skipped before changing the input revision. Updating one file does not rerun another file's analysis.

Paths follow `PathBuf` equality/hashing without canonicalization or filesystem I/O. Returned results remain usable across updates and database destruction; IDs belong to each result, not to a stable cross-revision namespace. `remove_file(path)` unregisters a path and returns whether it existed. Internal Salsa inputs remain allocated until the database is dropped, and caller-held `Arc` values retain earlier results independently. There is no reset, persistent cache, or parallel query API. See [ADR 0002](adr/0002-incremental-analysis.md) for the original file-level decision and [ADR 0003](adr/0003-project-import-search.md) for removal and project snapshots.

## Project import search

`analyze_project(sources)` returns an owned `ProjectSnapshot` using the default `RelativeResolver`; `analyze_project_with_resolver(sources, &resolver)` borrows a custom resolver only for that call. Both are available with default features and analyze anew on every call without a cache or filesystem access. Input is an iterator of pairs with paths implementing `Into<PathBuf>` and sources implementing `Into<Arc<str>>`. Paths follow `PathBuf` equality; the last source for a duplicate key wins before analysis. With the optional `incremental` feature, `AnalysisDatabase::analyze_project()` returns a cached `Arc<ProjectSnapshot>` over registered Astro/ESM buffers. Both entry points share path-selected grammar, index construction and query semantics. Each `ProjectFile` retains an `Arc<str>` source and its `Arc<Result<FileFacts, AnalysisError>>`; file-local IDs keep their original meaning. `files()` and `file(path)` expose these results, including errors. Failed importers contribute no outgoing imports or identifier uses. Target existence and target syntax validity are separate: an import can resolve to a supplied file whose analysis failed.

| Snapshot method | Contract |
| --- | --- |
| `outgoing_imports(path)` | Static import declarations contained in a successful file at `path`, including side-effect, unused, and unresolved imports, with their execution domain and `ModuleResolution`. |
| `incoming_imports(path)` | Direct import declarations whose module resolved to the exact supplied target key. |
| `references_to_file(path)` | Value/type references definitely bound to an import of that target file, including aliases and namespace roots. |
| `reference_at(path, offset)` | A canonical local reference and its originating import, if definitely bound to one. Module resolution uncertainty does not replace lexical `Resolution`. |
| `definition_at(path, offset)` | A saved reference's declaration/export/file target, ambiguous candidates, or an unresolved reason; `None` without a recorded reference. |
| `revision()` | `None` for uncached analysis; `Some(ProjectRevision)` with database-local source/resolver counters for cached analysis. They are not identities across databases. |

Results borrow canonical file records and are ordered by path, then original-source span. Shadowed locals, unresolved identifiers, builtins, and runtime-dependent candidate bindings do not become imported-file uses. An unknown path produces no matches. Resolving a named import's module alone does not validate that export; `definition_at` separately follows saved import/re-export records. Resolvers receive re-exports as binding-free imports in their actual Module or Server domain. Dynamic imports, script `src`, and CSS dependencies are outside this stage.

The default `RelativeResolver` matches explicit `./` and `../` specifiers using lexical dot/parent normalization and collapsing repeated separators over supplied virtual paths. It returns the original supplied key; multiple keys normalizing to the same target produce `Ambiguous`. Missing paths are `Unresolved`. Bare specifiers, absolute paths, URLs, backslashes, directory forms ending in `/`, `/.`, or `/..`, and query/hash suffixes are `Unsupported`; extensions and directory indexes are not inferred. There is no filesystem, symlink, or case-folding lookup. `ModuleResolver` implementations can use the original import, execution domain, and immutable `ProjectSources`, including source contents, to supply other policies. A resolver's `Resolved` result for an absent exact key is converted to `Unresolved`. Custom `Ambiguous` candidates are retained as reported for inspection; they are not validated or indexed as resolved imports.

With `AnalysisDatabase`, any source addition, edit, removal, or call to `set_module_resolver` invalidates the complete project index on demand, including negative lookups. Identical source updates and removal of an unknown key preserve it. File parsing remains memoized separately; configuration changes do not reparse unchanged files. Resolver implementations must use immutable settings and the supplied snapshot, and callers must replace them whenever their settings or external inputs change. Earlier project results retain their sources, facts, and resolution decisions across updates and database destruction.

## Cross-file definition API

`definition_at` returns `Option<DefinitionResolution<'_>>`. It searches recorded
reference spans with the same UTF-8 half-open contract as `reference_at` and
traverses only the snapshot's saved analyses and module resolutions. It performs
no parsing, resolver execution, I/O or source evaluation. Local references return
canonical `Binding` records. Imported references follow named/default imports,
aliases, imported-name exports, named re-exports and `export *` chains. Explicit
exports take precedence, stars exclude default, and repeated routes to one
declaration are merged. Multiple distinct targets are `Ambiguous` in path/span
order. Active `(path, export name)` pairs detect cycles; a star cycle can still
reach an explicit definition, while an otherwise cyclic lookup returns `Cycle`.
A failed or unopened star branch prevents an assumed unique definition.

`DefinitionKind` contains a borrowed declaration, anonymous/expression export,
Astro component file, or module file for a namespace root. File targets cover
the original source's complete span. Namespace member names and dynamic property
strings are not references and yield `None`; the namespace root resolves to its
module. The query does not infer member values or types. `DefinitionUnresolved`
distinguishes unresolved/unsupported/ambiguous module resolution, unsupported
source type, failed analysis, missing export and cycle; local unresolved,
builtin and runtime-dependent references have separate reasons. Retained
snapshots keep their original IDs, declarations and source buffers after
updates, deletion, re-registration and database destruction.

## Project report and directory example

With `serde`, `ProjectReport` is a shared model for any Serde encoding, retaining
`schema_version: 1`. Each file's `references` lists only references bound to
imports. Each record has a reference `key`, `name`, reference `kind`, originating
`import` key and `definition` (`Resolved`, `Ambiguous` candidates or `Unresolved`
reason). Targets carry a `key` and kind; declaration/export keys use their saved
spans and component/module keys cover the entire file. Other report fields keep
diagnostics, exports, imports/module resolution, component uses and unused
exports. No migration layer is required for this additive field.

```sh
cargo run --locked --features serde --example atlas-report -- ./src
cargo run --locked --features serde --example atlas-report -- ./src --format toon
cargo run --locked --features serde --example atlas-report -- ./src --format json
```

The example's arguments are `atlas-report <dir> [--format toon|json]`; TOON is
the default. It uses the default settings of dev-only `toon-format = "=0.7.0"`
with default features disabled, without adding a normal dependency or library
feature. Invalid arguments and I/O/encoding/output errors print to stderr and
exit nonzero. It collects the supported extensions recursively, skips `.git`,
`node_modules`, `target`, `dist`, `.astro` directories and never follows symlinks.
CommonJS files are not collected. The library APIs themselves do no directory
collection or I/O. See [ADR 0006](adr/0006-esm-definitions-and-toon.md).

## Fixed-release differences

1. compiler-rs 0.3.0 emits three `renderScript` calls for scripts in array/object/IIFE return expressions, but its extracted `scripts` array is empty. The named `nested-script-extraction` manifest case asserts this exact official output and atlas's three independent script facts. It is not ignored. `Script.bundled` expresses eligibility, not proof of successful upstream extraction.
2. `type="module"` alone is processed/hoisted by this release. `type={"module"}` and `type="module" src="..."` are not. Documentation's general extra-attribute rule is therefore insufficient for the pinned release.
3. HTML5 unquoted values include `/` in `<input value=4/>`, so its value is `4/`. `<Comp value= />` has a `/` value followed by an unclosed component, with an EOF diagnostic. Atlas follows the official package and upstream implementation. Two stale Oxc test expectations were updated, rather than changing this grammar. See [HTML tokenization](https://html.spec.whatwg.org/multipage/parsing.html#attribute-value-(unquoted)-state).
4. The official transform accepts dynamic slot names and several invalid directive values without diagnostics. Atlas preserves dynamic names and applies the documented basic static target/value checks; it does not attempt to match compiler omissions in diagnostic counts.

The shared catalog, owned-result snapshots, regression assertions, and compiler/JavaScript grammar evidence are described in [testing.md](testing.md). The catalog includes all AST semantic fixtures as well as the specification and regression fixtures. Bare and expression-valued bundled `src` bodies are processed; only literal bundled sources are external.

Classic `define:vars` scripts are isolated even when no bindings can be extracted, and can still read runtime candidates supplied by ordinary classic scripts. Their generated declarations precede the body, so a later `"use strict"` string is not a directive. Empty injections preserve effective directive prologues. A newline before the original body preserves classic HTML-close line comments. Module bindings are validated with module grammar.

The native compiler's Astro parse rejects a top-level `return` and an opening-line `-->` comment in the two classic wrapper fixtures. Atlas models the generated IIFE grammar, which Node's syntax-only checks accept. Conversely, the compiler accepts the empty-injection strict-body fixture whose generated classic wrapper has a reserved `yield` identifier. These parser/grammar results are asserted separately in the manifest; no runtime execution or dependency patch is involved.

## Sources and reproducibility

- [Astro 7.0.6 source](https://github.com/withastro/astro/tree/dbe9144e55ccbe4cccac53abef60d8a41509178f), including public attribute types and `defineScriptVars` normalization.
- [Astro syntax](https://docs.astro.build/en/basics/astro-syntax/), [directives](https://docs.astro.build/en/reference/directives-reference/), [scripts](https://docs.astro.build/en/guides/client-side-scripts/), [styling](https://docs.astro.build/en/guides/styling/). These are supporting current documentation; release-specific behavior is checked using the pinned package.
- [Lightning CSS](https://github.com/parcel-bundler/lightningcss) parses typed stylesheet/declaration values for syntax validation. cssparser supplies lossless tokens for ordered facts and source maps. Unknown property values remain raw rather than being classified as computed CSS.
- `npm ci --ignore-scripts && npm run test:conformance` builds only the test serializer and checks 86 registered sources. Node is not a library runtime dependency. `cargo test --locked --all-targets` runs the owned semantic/CSS assertions without Node.
