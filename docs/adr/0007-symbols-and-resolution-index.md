# ADR 0007: Physical declarations and snapshot resolution indexes

Status: Accepted

Date: 2026-10-19

## Context

Oxc symbols can represent a type-only import and a same-name runtime constant,
multiple interface declarations, or function signatures and an implementation.
Treating a symbol as one declaration makes results depend on declaration order,
can misattribute local value uses to an import, and can select the wrong constant
initializer. Matching imports against that representative's span can omit their
edges. Root-local traversal also repeats export scans across consumers and cannot
share partially explored cycles safely.

## Decision

- Separate `SymbolId`/`Symbol` from physical `DeclarationId`/`Binding` records.
  Import every Oxc redeclaration with its own span, kind, meanings and AST node
  during collection. Keep representative IDs before appended declarations across
  all server/browser domains. A binding points to its symbol; a symbol exposes
  every declaration and selects a representative for a requested `Meaning`.
- Preserve Value, Type, Namespace and ValueAsType. Recover `typeof` and qualified
  type-name context from the AST after Oxc normalizes flags. Value and ValueAsType
  prefer real value declarations over aliases. Functions prefer an implementation,
  then the first signature; other matches use source order. Valid merges stay
  resolved and do not require a new public merged-result variant.
- Direct exports preserve physical declarations and their meanings; export lists
  expose symbols. Meaning selection happens after explicit-name priority. A
  mismatch returns UnresolvedReference without retrying stars. Type-only paths
  reject runtime Value but permit appropriate ValueAsType queries.
- Import bindings carry their physical declaration ID. Module requests borrow
  the original Import or Export using `ModuleDependency` and expose the decoded
  source, whole-statement range, literal range and execution domain. Remove
  synthetic binding-free imports for re-exports. Keep Export.span/report keys.
- Build export-name, star, export-edge and used-export-name indexes once per
  snapshot. Namespace/star use flags make unused-export membership independent
  of incoming consumer count. Component usage is also indexed once per snapshot.
- Share completed definition results across definition, component and report
  queries in a Mutex-protected cache keyed by file ID, export name or SymbolId,
  and Meaning. Cache only owned IDs and completed immutable Arc results.
  Convert them to borrowed records after unlocking.
- Expand only uncomputed reachable nodes. Use iterative Tarjan for SCCs and
  dependency-first evaluation of the condensation DAG. Deduplicate candidate
  IDs with sets and share unchanged results through simple alias chains.
  No partial cycle result escapes an individual computation.
- Keep precedence: multiple candidates, blocking failure, one candidate, cycle,
  missing export. Within each failure category, choose the smallest tuple of
  failing-node path, node name and reason variant name, then reason payload.
  Cycles use the smallest participating path/name. The specification documents
  this deterministic tie-break independently of traversal or query order.
- Read a constant initializer from the selected physical declaration's AST node.
  Use the same declaration mapping for references, exports and browser scripts.
- Keep new metadata outside Serde, retain report schema 1, and add no dependency,
  feature, type inference or CommonJS support. Each new snapshot owns a fresh
  cache; retained snapshots and concurrent queries preserve their original facts.

## Consequences

Public Rust fact and resolver request APIs gain symbol and dependency metadata.
Callers can inspect every merged declaration while definition queries retain one
representative location appropriate to their reference. Custom resolvers receive
accurate original syntax and ranges, including type-only and namespace forms.
Existing JSON/TOON shapes and source-position keys remain compatible.

Cache memory grows with distinct queried file/name/meaning nodes. A mutex
serializes first-time graph expansion; completed results are immutable and shared.
No query reparses, invokes the module resolver, executes source or performs I/O.
Operation-count tests cover many names/consumers, shared diamonds and long chains;
wall-clock measurements are supplementary, not CI thresholds.

## References

- [Specification](../specification.md#cross-file-definition-api)
- [Declaration and dependency regressions](../../tests/symbols.rs)
- [SCC and work-count tests](../../src/definition.rs)
- [Existing graph regressions](../../tests/definition_graph_regressions.rs)
- [ADR 0006](0006-esm-definitions-and-toon.md)
