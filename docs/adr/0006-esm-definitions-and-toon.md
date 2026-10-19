# ADR 0006: ESM analysis, cross-file definitions and TOON reports

Status: Accepted

Date: 2026-10-19

## Context

Astro frontmatter and browser scripts import declarations from external modules.
File-level import indexes alone do not identify those declarations, distinguish
ambiguous barrel exports, or explain failed definition lookups. Report consumers
need these results in both JSON and TOON without making the library depend on an
output encoding.

## Decision

- Add `analyze_file(path, source)` without filesystem access. Keep `analyze(source)`
  Astro-only. `.js`, `.mjs`, `.jsx`, `.ts`, `.mts`, `.tsx`, `.d.ts` and `.d.mts`
  use the pinned Oxc `SourceType::from_path`. Other virtual keys retain Astro
  syntax. Explicit `.cjs`, `.cts` and `.d.cts` inputs return
  `atlas/unsupported-source-type`. CommonJS semantics are outside this decision.
- External scopes use `ScopeDomain::Module`; runtime environment is unspecified.
  Collect lexical declarations, JSX/value/type references and ESM exports, with
  no Astro builtins, export roles, template validation or transfer boundaries.
  Default exports retain a local declaration ID when available; anonymous and
  expression defaults retain the complete export range. Browser script exports
  remain separate from the containing Astro component's exports.
- Declaration modules implicitly export top-level local declarations unless an
  export list, star or assignment defines their export surface. Global `.d.ts`
  scripts and ordinary `.ts` files do not gain implicit exports. Preserve the
  syntactic distinction between default expressions and unresolved local export
  specifiers as Rust-only metadata; an unresolved alias stays unresolved.
- Use the same path-selected analysis in projects and Salsa. Store the original
  path in each Salsa input and pass the actual domain to re-export resolvers.
  Do not add extension guessing, package configuration or filesystem resolution.
- Add `ProjectSnapshot::definition_at(path, offset)` over saved facts and module
  resolutions. No recorded reference means `None`. Other results are resolved,
  ambiguous, or unresolved with a reason. Targets borrow canonical declarations,
  exports or files from that snapshot, preserving IDs and original source spans.
- Follow local bindings, default/named imports, aliases, imported-name exports,
  named re-exports and star chains. Explicit exports take priority, stars exclude
  default, and identical target identities are merged. Track active `(path,
  export name)` pairs to terminate cycles. A star cycle can still reach an
  explicit declaration; an otherwise cyclic lookup returns `Cycle`. Failed or
  unopened star branches cannot establish a unique target. Namespace roots
  return modules; properties, dynamic members and type inference are excluded.
- Each definition root accumulates reachable targets/failures while visiting each
  `(path, name)` once. Active visits detect cycles; completed visits add nothing.
  A report batch reuses only finished binding/export root results, keeping SCC
  traversal state separate. Recorded references are wrapped directly without
  repeated source-offset searches.
- Attribute component uses through the same definition resolver, including local
  imported-name/default re-exports, explicit priority and ambiguity. Namespace
  template members follow static export paths; ordinary object members are not
  inferred. Direct Astro namespace imports retain single-member file candidates.
- Distinguish unresolved, unsupported and ambiguous module resolution,
  unsupported source types, failed target analysis, missing exports and cycles.
  Builtins, unresolved lexical references and runtime-dependent references retain
  separate reasons. `unused_exports` recognizes default imports and excludes
  defaults from `export *`; namespace imports include defaults.
- Keep `ProjectReport` an encoding-independent Serde model and retain
  `schema_version: 1`. Add per-file imported-reference records linking the source
  reference and import keys to definition keys, ambiguous candidates or an
  unresolved reason. Keys remain `path#start-end`; module/component targets cover
  the entire file. No migration layer or transfer report is introduced.
- Extend the report example to `atlas-report <dir> [--format toon|json]`, defaulting
  to TOON with its default encoding settings. Pin `toon-format = "=0.7.0"` with
  `default-features = false` as a dev dependency only. Invalid arguments, file I/O,
  encoding and output failures return stderr and a nonzero exit status.
- Collect supported extensions recursively while skipping `.git`, `node_modules`,
  `target`, `dist`, `.astro`, CommonJS files and symlinks. The library still performs
  no I/O. Recursively sort test snapshot keys because TOON enables
  `serde_json/preserve_order`; preserve all existing Astro fixture bodies.

## Consequences

External modules participate in uncached and incremental queries with identical
grammar and resolution rules. A newer snapshot reflects additions, edits,
removals and resolver replacement; retained snapshots keep their own sources,
facts and resolution decisions. Definition lookup traverses those saved records
without parsing or executing a resolver again.

The report example adds TOON without changing normal library dependencies or
features. Strict-decoding tests compare the encoded report to its JSON value,
including empty arrays, nulls, enums, Japanese text and paths/diagnostics with
newlines and delimiters. Existing compiler and transfer comparisons remain Astro
syntax/runtime evidence; they do not determine ESM export identity.

Cross-file constant evaluation for transfer assessment, HTML/style transfer,
code rewriting, CommonJS, dynamic member resolution, package settings and type
inference remain future work. This decision extends ADRs 0002–0004 and leaves
the transfer contract in ADR 0005 unchanged.

## References

- [Pinned Oxc grammar selection](https://github.com/jp-knj/oxc-atlas/blob/8ef5b6a720bab87987dbc33f569af9d784c06850/crates/oxc_span/src/source_type.rs)
- [TOON Rust 0.7.0](https://docs.rs/crate/toon-format/0.7.0)
- [External grammar tests](../../tests/external_sources.rs)
- [Definition queries](../../tests/definitions.rs)
- [Shared graph regressions](../../tests/definition_graph_regressions.rs)
- [Component attribution](../../tests/component_definitions.rs)
- [Incremental retention](../../tests/project_incremental.rs)
- [Report round trip](../../tests/report.rs)
