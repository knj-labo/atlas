# ADR 0005: Shared semantic facts and boundary transfer assessment

Status: Accepted

Date: 2026-10-18

## Context

Compiler tools need to ask where an Astro expression is evaluated, which
declaration a reference denotes, and whether a value can cross an explicit
server/browser boundary. A lexical scope describes name lookup; it does not
itself describe execution or transfer. Re-parsing expressions in each consumer
would duplicate semantic work and lose the connection to original-source
evidence, especially for injected script bindings and shadowed names.

The first consumer is a compiler checking script injections and hydrated
component props. A successful value-format check must not imply that a renderer
exists, that publishing the data is permitted, or that code can be relocated.

## Decision

`FileFacts.boundaries` contains owned `BoundaryFact` records. Each has a
result-local `BoundaryId`, its target `ElementId`, original opening-tag span,
boundary kind, transfer method, evaluation and destination domains, copies of
existing directive validation results, syntactic inputs, and handoff conditions.
Lexical scopes and resolutions continue to use existing `ScopeId`,
`DeclarationId`, and `ReferenceId` records. The boundary's evaluation domain is
`Server`; script destination domains reflect the existing script classification
and component props target `BrowserModule`. A destination is absent when script
execution is unconfirmed.

Two boundaries are collected:

- Literal `<script define:vars>` uses `JsonScript`.
- Components with `client:load`, `client:idle`, `client:visible`, `client:media`,
  or `client:only` use `AstroProps`. Directives and `slot` are excluded from props.
  Dynamic/self component targets retain their existing validity states.

Custom client directives alone do not create a boundary: their transfer method
is not inferred. If a known directive occurs with another client directive, the
known method is assessed with a multiple-directive condition. Styles, HTML
output, external JS/TS, code generation, and changes to execution placement are
deferred.

### Pinned transfer contracts

The reference release is Astro 7.0.6, commit
`dbe9144e55ccbe4cccac53abef60d8a41509178f`:

- [stringifyForScript in escape.ts](https://github.com/withastro/astro/blob/dbe9144e55ccbe4cccac53abef60d8a41509178f/packages/astro/src/runtime/server/escape.ts)
  JSON-encodes a value and escapes less-than characters for script embedding.
  [defineScriptVars](https://github.com/withastro/astro/blob/dbe9144e55ccbe4cccac53abef60d8a41509178f/packages/astro/src/runtime/server/render/util.ts)
  invokes it for each input entry.
- [serializeProps in serialize.ts](https://github.com/withastro/astro/blob/dbe9144e55ccbe4cccac53abef60d8a41509178f/packages/astro/src/runtime/server/serialize.ts)
  recursively produces tagged values before JSON encoding and has a BigInt
  tag. Its cyclic-object detection does not establish anything about an
  arbitrary source expression without value analysis.

Atlas records facts about source values and implements conservative rules over
those facts. It does not reimplement either serializer or execute source code.

### Collection and assessment

While Oxc's AST and semantic information are alive, the collector classifies
values and follows same-file simple `const` bindings and aliases to primitives.
Each followed alias records the declaration ID, reference ID, and initializer
source text. Resolved declaration identity, rather than name matching, handles
shadowing and separates server references from browser and injected bindings.
All new reference IDs participate in the existing source-order renumbering.
Facts retain raw text and UTF-8 byte spans, including Unicode and CRLF input.

`FileFacts::assess_transfer(BoundaryId) -> Option<TransferAssessment>` needs only
owned records. An invalid ID returns `None`. Each input result has its input
index, `Supported`, `Unsupported`, or `Unknown` status, and typed reasons with
original-source spans. The result also carries shared unverified assumptions.
The query does no parsing, I/O, or expression evaluation and preserves the facts.

| Collected value | JsonScript | AstroProps |
| --- | --- | --- |
| Null, string, boolean, finite numeric literal | Supported | Supported |
| Static array/plain object with supported members | Supported | Supported |
| Same-file simple const/alias to a supported primitive | Supported | Supported |
| BigInt literal, including a negative literal or primitive alias | Unsupported | Supported |
| Direct function/arrow/method value | Unsupported | Unsupported |
| Mutable binding, identifier-backed array/object | Unknown | Unknown |
| Calls, imports, unresolved references, unsupported expressions | Unknown | Unknown |
| Dynamic keys, spreads, getters/setters, prototype-setting objects | Unknown | Unknown |
| Array holes, duplicate keys inside a value object, nonfinite numeric literals | Unknown | Unknown |

Function rejection describes loss of the function value; JSON can omit a
function or substitute null without throwing. An aggregate with a definitely
unsupported member is unsupported. Otherwise, an unknown member keeps it unknown.
`const` alone does not establish object immutability, so identifier-backed
aggregates are not followed. Destructuring and function bindings are untracked;
a const function initializer is not followed as a primitive alias. Forward
initialization, alias cycles, and a collection depth limit of 128 produce unknown
results. There is no general constant evaluator or control-flow proof.

Inputs are syntactic entries, not a computed effective prop set. Duplicate input
names are preserved with a `DuplicateInput` condition. Spreads and dynamic keys
produce unknown inputs and a `DynamicInputSet` condition; their possible overrides
are not selected. A non-enumerable `define:vars` expression represents an unknown
input set, even if the expression itself is a literal primitive. Numeric property
names use ECMAScript number formatting to detect equivalent keys consistently.

Directive statuses are retained unchanged. A known primitive can have format
support while its boundary has invalid or dynamic directive conditions. Data,
external, and dynamic script modes retain their original classifications and an
`UnanalyzedScript` condition. Spreads or duplicate script attributes leave the
destination unconfirmed. Invalid injected identifiers are also recorded.

`Supported` answers only whether the collected value is handled by the selected
format under the pinned standard runtime contract. It is not a proof of complete
serializability for arbitrary expressions, boundary execution, or effective
argument selection. Every assessment records data-disclosure and code-movement
assumptions; client props additionally require renderer availability. Callers
must inspect these conditions independently.

### Rust API and compatibility

The initial API is Rust-only. `FileFacts.boundaries` is excluded from its
existing serde representation, and the new value and assessment types do not
implement serialization. `ProjectReport` and `REPORT_SCHEMA_VERSION = 1` retain
their current shape. No new dependency or Oxc revision is introduced; the existing
`oxc_syntax` dependency explicitly enables its already-transitive
`to_js_string` feature for numeric key formatting.

Owned facts and assessments outlive the parser, source buffers, database
updates, and database destruction. IDs belong to the containing result, so a
consumer must obtain them again after a source revision. This is the same
contract as declaration and reference queries.

## Compiler applications

A compiler can enumerate boundaries before emitting a handoff, call
`assess_transfer`, and attach a format-specific diagnostic to an unsupported
input. For example, the same `1n` value is rejected for script injection and
accepted by the props format. Unknown inputs require a runtime path or further
analysis; a compiler must not promote them to supported. Renderer checks and
directive diagnostics remain independent, including for `client:only`.

Removing an unused injected value is a possible later application. The existing
injected declaration and origin reference IDs can relate the browser binding to
the server expression. This assessment alone does not authorize removal:
the compiler also needs complete use analysis, including nested closures and
dynamic injection/global lookup, plus side-effect and evaluation-order analysis
of both the value expression and the input object. Removing an unused
`loadData()` initializer, a computed key, a spread, or a getter can change program
behavior. Moving code across the boundary requires additional capture, runtime,
and execution-context analysis. No such optimization is implemented here.

## Verification

[tests/transfer.rs](../../tests/transfer.rs) checks both methods, primitive
constant provenance, aliases, shadowing, same-named server/browser variables,
multiple scripts, `client:only`, excluded directives/slot, unsupported functions,
BigInt differences, mutable aggregates, imports, calls, spread/dynamic keys,
cycles, invalid directives, duplicate inputs, script mode conditions, invalid
IDs, and Unicode/CRLF source recovery. It also checks recursive origin/reference
renumbering, parser/source independence, cached/retained results, and unchanged
JSON/report schema with the relevant features enabled.

Regression cases run both transfer methods over the same value tables, check
alias/container depth limits immediately before and at the limit, and compare
reason and condition spans to exact source intervals. A fixed-seed grammar
generates 128 JSON-safe values with maximum depth 4, checking parentheses,
primitive aliases, LF/CRLF, and unrelated browser scripts against explicit
fixture values and server declaration identity. Incremental tests retain each
revision's IDs and constant origins across updates, removal, re-registration,
and database destruction.

`npm run test:transfer` compares Atlas with the real `stringifyForScript` and
`serializeProps` implementations from the exact `astro@7.0.6` development
dependency. A test-only `transfer-dump` example reads Astro source from stdin
and emits boundary methods, input names, and statuses without extending the
public serialization contract. Runtime values come from predefined fixture
factories; analyzed expressions are never evaluated. Expectations cover JSON
restoration and script escaping, explicit props tags and payloads, negative zero,
BigInt, and function omission/substitution. Successful runtime serialization of
const aggregates or call results still requires Atlas to report `Unknown`.

The existing four feature combinations, doctests, Clippy checks, owned-facts
snapshots, and pinned compiler comparisons continue to apply. Compiler comparisons
verify the existing syntax/extraction contract; they are not an oracle for the
new transfer classification. Transfer rules have separate pinned runtime
comparisons and focused Rust assertions. Both comparison suites run in CI.
