# ADR 0004: Export model and project report

Status: Accepted

Date: 2026-10-17

## Context

Other tools already cover nearby ground: `@astrojs/compiler-rs` compiles Astro,
Fallow builds dependency graphs, and Vize/Verter cover Vue. None of them exposes
Astro-specific semantics (exports read by Astro, component props, directives,
slots) in a stable machine-readable form. Fallow and AI agents need that form.

## Decision

- `FileFacts.exports` records explicit frontmatter exports with an
  `ExportRole` (`Astro` for `getStaticPaths`/`prerender`/`partial`, `Props`,
  `Other`). The implicit default component export is not listed. Client script
  exports are excluded.
- `ProjectSnapshot::component_uses_of` returns cross-file component uses with
  their elements. `unused_exports` reports `Other`-role exports that no
  resolved named or namespace import reaches. Treat results as candidates:
  re-export consumers, unresolved modules and non-`.astro` importers are not seen.
- With the `serde` feature, `ProjectSnapshot::report` returns a versioned
  `ProjectReport` (`REPORT_SCHEMA_VERSION`). Records are keyed `path#start-end`
  instead of local IDs, so consumers can quote them across runs.
- `examples/atlas_report.rs` prints the report for a directory. A CLI stays out
  of scope.

## Consequences

Re-export traversal and standalone JS/TS analysis remain deferred. Incompatible
report changes must bump the schema version.
