# Fixture and snapshot verification

The catalog in [tests/spec/manifest.json](../tests/spec/manifest.json) registers all **86** Astro files in `tests/fixtures/` and `tests/spec/`: 16 AST semantic fixtures, 38 original specification fixtures, and 32 regression fixtures. Both Rust and Node reject duplicate IDs or paths, missing files, and unregistered Astro files, including files in nested directories. Both validate the snapshot group. Inline Rust test inputs are independent unit scenarios and do not require catalog entries.

Each entry has a stable snapshot `id`, a file path relative to `tests/spec/`, a `snapshot_group`, an `atlas` expectation, and a separate `compiler` expectation. Atlas expectations select explicit semantic assertions and expected success/error outcomes and script modes. Compiler expectations record parse outcomes, extraction counts or exact script types/content, generated IIFEs/helpers, and selected syntax-only JavaScript checks. Compiler output does not establish declaration identity, global suppliers, or CSS candidates.

Snapshots are grouped by analysis responsibility:

```text
tests/snapshots/
  template/  # components, scopes, template attributes, diagnostics (30)
  script/    # script modes, isolation, injection, browser globals (43)
  style/     # style rules, declarations, references, source spans (13)
```

A fixture has one group in the shared manifest. Mixed inputs use the responsibility of the main assertion. Cases reproducing previous defects share these directories with the other cases. Moving the files preserves all 86 snapshot bodies and names.

## Assertions and representation

[snapshots.rs](../tests/support/snapshots.rs) runs correctness assertions and graph/source validation for the entire catalog before any snapshot assertions. Twelve named regression tests independently check the functional defects. Registry and invalid-graph tests verify that the harness rejects malformed catalogs and facts. Existing AST, owned-facts, and specification tests remain active.

The [test-only renderer](../tests/support/render.rs) uses `serde_json::to_value` with test-only Serialize derives and does not require the optional public `serde` feature. A compact JSON transformation adds ID labels and reduces embedded records; the span validator inspects the unreduced serialized facts. Each `.snap` contains compact JSON for `Result<FileFacts, AnalysisError>`. Ranges are `[start,end]` UTF-8 half-open intervals. Declaration, scope, reference, element, and style IDs use `d`, `s`, `r`, `e`, and `style` labels with their original array indices. Same-name declarations remain distinct. Resolved bindings and embedded component/script references become ID links to canonical records. CSS rule/declaration IDs are local to their containing style; candidates retain both the target style and definition index.

Snapshots retain import targets, all template attributes/directives, scope parents/domains/kinds, reference kinds and targets, runtime candidates and reasons, injection keys/values/origin references, script/style metadata, CSS selectors/rules/conditions/declarations/definitions/references/fallbacks/dependencies, and diagnostics. Both `Ok` diagnostics and `Err` diagnostics include phase, severity, nullable code, message, label spans/messages; errors also retain `parser_panicked`.

Before reducing embedded records to links, [validation](../tests/support/validate.rs) checks original ID namespaces, canonical binding/reference equality, injection links, element/scope/CSS parent bounds and cycles, and CSS candidate kinds/names. Every span must be bounded and lie on UTF-8 boundaries. Raw source text must equal its original slice. Injection keys may lie outside their script-body scope span because the keys occur in opening attributes.

## Regression expectations and results

Each functional row was reproduced with failing assertions before its fix and passes afterward. Snapshots were generated only after all correctness assertions passed.

| Test | Fixtures / variants | Expected and measured result |
| --- | --- | --- |
| `bundled_src_bodies` | bare, expression, expression-string, literal `src`; malformed expression-src body | Bundle-eligible nonliteral sources retain body declarations/references and report syntax errors. Literal source stays external. Binding identity is checked. |
| `injected_isolation` | empty, entirely dynamic, static `define:vars`; empty wrapper with return | Wrapper-local declarations belong to InjectedScript and supply no classic globals, even with zero extracted bindings. |
| `injected_global_readers` | empty/dynamic/static readers; injected local shadows supplier | Wrapped classic readers retain the supplying classic declaration ID as a runtime candidate; a local injection resolves to its distinct ID. |
| `inline_style_injection` | inline and effective extracted style | Inline style retains `define:vars` attributes but has no fake injected definition/candidate; extracted style has the injected candidate. |
| `animation_priority` | priority suffix, actual keyframes `important`, quoted `none` | Only actual animation-name tokens have references; priority words never become names. Exact spans and candidate links are checked. |
| `animation_slots` | nine repeated keyword spellings across commas, functions/time/count slots, quoted names, shared name/timeline spelling | Lightning CSS determines occupied slots; the correct name token and its exact span survive. |
| `injected_keyword_grammar` | sloppy `yield`/`await`/`implements`, declaration prefix before strict string, module and reserved bindings | Sloppy contextual names are injected with distinct identities; invalid module/reserved names receive Astro diagnostics. Effective strict-body failure is separately snapshotted. |
| `external_parent_language` | template, svg, noscript | External source remains External with JavaScript language and bundled=false. Nested styles have no effective injection. |
| `animation_dynamic_values` | variable duration/easing/name, comma lists, repeated keywords, quoted name, priority | Unparsed values retain conservative runtime-dependent animation references and custom-property links. Ambiguous repeated keywords can retain multiple possible name tokens. |
| `classic_html_close_comment` | static injection followed immediately by `-->`, Unicode comment | Synthetic padding ends with a newline; classic parsing succeeds and original declaration/reference byte spans remain intact. |

Twelve fixtures snapshot `Err` results, covering parser, semantic, and CSS phases, including Unicode/CRLF CSS diagnostics. Successful fixtures also cover Astro Error/Information diagnostics. The existing named `nested-script-extraction` discrepancy stays asserted: compiler-rs emits three render calls with zero extracted scripts; atlas retains three script facts.

Compiler evidence and Node grammar checks remain separate. The pinned compiler rejects the wrapper `return` and HTML-close-comment inputs before transformation; syntax-only classic IIFE checks accept them. It accepts the empty-injection strict-body input, but Node and atlas reject its effective reserved keyword. Module helper calls do not produce classic IIFEs. No analyzed code is executed.

## Refactor verification and source references

The two new failures were reproduced before their fixes: a backslash followed by an actual U+2028/U+2029 produced `--to<separator>ne` instead of `--tone`; NEL/BOM keys resolved to declarations with the wrong injection origins. The `css_line_continuations` and `injected_ecmascript_whitespace` tests now check names, target IDs, key/value origins, UTF-8 spans, and raw source recovery. The three registered fixtures have accepted snapshots and independent compiler extraction/helper evidence.

`css_parser_boundaries` checks charset/import rules, nested selectors and conditions, duplicate declarations, `!important`, and custom-property block values containing semicolons and braces. `css_decoded_names_preserve_original_source` distinguishes literal Unicode escapes from line continuations and checks HTML entities, JavaScript hex/braced Unicode escapes, and CSS escapes. All 83 pre-refactor snapshot bodies are unchanged. The 16 AST semantic fixtures use their file names as IDs and matching snapshot filenames. Default and serde-only test runs cover 73 Rust tests, 86 snapshots, and two doctests; compiler comparisons cover 86 cases, plus four registry rejection tests.

## Incremental verification

The optional `incremental` feature has six unit tests in [incremental.rs](../src/incremental.rs) and four public-API tests in [tests/incremental.rs](../tests/incremental.rs). Unit tests install a test-only Salsa `WillExecute` event callback. They count query executions to verify lazy registration, memo reuse, identical-text no-ops, isolation between files, latest-text analysis after multiple pending updates, independent inputs for equal text at different paths, and caching/recovery for parser, semantic, and CSS syntax errors. Pointer identity checks additionally verify shared `Arc` results; execution events establish that the query did not rerun.

Public-API tests compare all 86 registered fixtures with standalone `analyze`, check repeated shared results, retain results across updates and database destruction, and verify independent databases, unknown paths, and lexical `PathBuf` keys that do not require files to exist. Fixture parity covers both successful facts and syntax errors without new snapshots or compiler baselines. Including the file-local and project query suites, incremental-only and all-feature runs cover 94 Rust tests, the existing 86 snapshots, the 86 fixture parity comparisons, and five doctests.

CI tests all four feature combinations: default, serde only, incremental only, and both features. It checks that the default normal dependency tree contains no Salsa packages and runs doctests with default and all features. Rust 1.91.1 remains pinned; Salsa 0.28.5 enables only `macros` and `inventory`.

Implementation references:

- [Oxc AST helpers](https://github.com/jp-knj/oxc-atlas/blob/8ef5b6a720bab87987dbc33f569af9d784c06850/crates/oxc_ast/src/ast_impl/js.rs) supply parenthesis removal and static property names; supported dynamic/static key categories are unchanged. JSX root lookup retains `this` receivers.
- [Oxc string escapes](https://github.com/jp-knj/oxc-atlas/blob/8ef5b6a720bab87987dbc33f569af9d784c06850/crates/oxc_parser/src/lexer/unicode.rs) define line continuations. The lexer function is private, so CssSource retains its original-byte map and follows those escape rules.
- [Oxc whitespace predicates](https://github.com/jp-knj/oxc-atlas/blob/8ef5b6a720bab87987dbc33f569af9d784c06850/crates/oxc_syntax/src/identifier.rs) and line-terminator predicates implement ECMAScript trimming for [Astro 7.0.6 toIdent](https://github.com/withastro/astro/blob/dbe9144e55ccbe4cccac53abef60d8a41509178f/packages/astro/src/runtime/server/render/util.ts). Script grammar validation still uses Oxc.
- [compiler-rs scanner](https://github.com/withastro/compiler-rs/blob/main/crates/astro_codegen/src/scanner.rs) and style printer are eligibility references; the pinned 0.3.0 native package supplies reproducible compiler evidence. Script/style parent eligibility is shared within atlas.
- [cssparser 0.37.0 rule APIs](https://docs.rs/cssparser/0.37.0/cssparser/struct.RuleBodyParser.html) delimit rules and declarations through trait callbacks. The collector uses cssparser tokens directly with source spans. Lightning CSS 1.0.0-alpha.72 continues syntax validation and typed animation parsing; atlas preserves conservative names in Unparsed values. A leading charset rule is collected before StyleSheetParser, which otherwise discards it.

## Reproduction and review

[queries.rs](../tests/queries.rs) checks exact declaration names and import aliases,
lexical shadowing, source ordering across template and script domains, borrowed
record identity, explicit resolution states, injected bindings, type references,
Unicode/CRLF byte boundaries, and empty results outside the query contract.
[incremental.rs](../tests/incremental.rs) also compares queries on cached and direct
analyses and checks retained results after source updates and database destruction.
These inline scenarios do not add fixtures or change snapshot baselines.

[project.rs](../tests/project.rs) covers direct imports and reverse file-use
indexes, aliases/type imports/namespaces, side-effect and unused imports,
server/browser separation, uncertain resolutions, deterministic ordering, cycles,
Unicode/CRLF positions, relative virtual path normalization and ambiguity, missing
and unsupported targets, file addition/removal/re-addition, source and resolver
changes, retained snapshots, analysis errors, and reuse of unchanged file results.
The compiler comparison remains evidence for single-file syntax and positions;
it is not a module-resolution oracle for the virtual project API.

```sh
cargo fmt --all -- --check
INSTA_UPDATE=no cargo test --locked --all-targets
INSTA_UPDATE=no cargo test --locked --all-targets --features serde
INSTA_UPDATE=no cargo test --locked --all-targets --features incremental
INSTA_UPDATE=no cargo test --locked --all-targets --all-features
cargo test --locked --doc
cargo test --locked --doc --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
npm ci --ignore-scripts
npm run test:conformance
```

Inspect the default dependency boundary with `cargo tree --locked --edges normal --prefix none`; no `salsa`, `salsa-macros`, or `salsa-macro-rules` package should appear. `cargo tree --locked --features incremental --edges normal --prefix none` includes the pinned Salsa packages.

To isolate correctness assertions from snapshots:

```sh
cargo test --locked --lib -- --skip fixture_snapshots
```

For intentional baseline changes, generate pending snapshots, inspect them, and review:

```sh
INSTA_UPDATE=new cargo test --locked --lib
cargo insta pending-snapshots
cargo insta review
```

The generation command fails while new snapshots await acceptance. Review `.snap.new` against the old `.snap`, fixture source, and explicit expectations. A scoped noninteractive alternative from [Oxc's Insta skill](https://github.com/oxc-project/oxc/blob/main/.agents/skills/insta-snapshots/SKILL.md) is `cargo insta accept --snapshot tests/snapshots/GROUP/fixture_snapshots__CASE_ID.snap` after that review. Accept only reviewed task snapshots, then rerun with `INSTA_UPDATE=no`. Commit accepted `.snap` files; ignored `.snap.new` files never satisfy snapshot assertions. CI disables snapshot writes for all checks.

The Rust toolchain, Oxc revision, and compiler version retain their existing pins. Cargo.lock includes Insta 1.48.0 and adds direct oxc_syntax usage at the same pinned Oxc revision. Browser execution, project-wide globals, dynamic expression evaluation, CSS cascade/inheritance, and final animation selection remain outside the contract.
