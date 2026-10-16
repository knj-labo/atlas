# ADR 0002: Optional file-level incremental analysis

Status: Accepted

Extended by [ADR 0003](0003-project-import-search.md) for file removal, shared source
buffers, and project import snapshots. The original decision below describes the
initial file-level API.

Date: 2026-10-14

## Context

The single-file `analyze` API produces owned facts and diagnostics, independent of Oxc allocators and AST lifetimes. Consumers that repeatedly analyze multiple source buffers need to reuse unchanged results and obtain fresh results after a file's text changes. They also need to retain earlier results while updating their source buffers.

This stage adds in-memory, file-level memoization. Callers provide path keys and complete UTF-8 source strings. Import resolution, filesystem access, file watching, query boundaries within a file, and persistent caching remain separate concerns.

## Decision

### Optional dependency and public boundary

Add an opt-in `incremental` feature to the existing atlas crate. Pin Salsa to `=0.28.5`, disable its default features, and enable only `macros` and `inventory`. Salsa's minimum Rust version is 1.88; atlas retains Rust 1.91.1. Default atlas builds do not include Salsa or the new API.

Export `AnalysisDatabase` at the crate root when the feature is enabled. Its public API contains Atlas and standard-library types; Salsa databases, inputs, queries, revisions, and IDs remain private. `new()` and `Default` create an empty database.

| Method | Contract |
| --- | --- |
| `set_source(&mut self, path: impl Into<PathBuf>, source: impl Into<String>)` | Register a new path or replace its complete source text. It performs no analysis. Identical text is detected before calling the Salsa setter. |
| `analyze_file(&self, path: impl AsRef<Path>) -> Option<Arc<Result<FileFacts, AnalysisError>>>` | Return `None` for an unregistered path. Otherwise, obtain the current memoized analysis result and clone its `Arc`. Both successful facts and syntax errors are cached. |

Path identity follows `PathBuf` equality and hashing. Atlas performs no canonicalization, absolute-path conversion, or filesystem lookup. Callers choose consistent keys; unresolved `..` components remain part of the key. Different path keys receive independent inputs even when their source text is equal.

### Inputs, queries, and updates

The wrapper holds a path-to-input map outside Salsa. Each registered path owns one private `SourceFile` input containing its text. Updates preserve that input's identity. Source equality is checked before invoking a setter because Salsa setters mark a field as changed even when its value is equal.

A private `analyze_input` tracked query depends only on that input's text and invokes the existing `analyze` function. It memoizes an `Arc<Result<FileFacts, AnalysisError>>` and uses `returns(clone)` to return shared ownership. Existing fact and diagnostic equality supplies Salsa's result comparison; no Salsa traits or lifetimes are added to the public fact model.

Registering or updating text is lazy. The first request for a file performs its analysis. Repeated requests reuse its result. Updating one input causes its next request to analyze the whole file, while unchanged inputs reuse their results. Multiple updates before a request analyze only the latest text. Oxc ASTs and CSS parser data remain temporary values inside `analyze`, rather than database values.

The syntax/diagnostic contract is unchanged: parser, semantic syntax, and standard CSS syntax failures are cached `Err(AnalysisError)` values. Astro convention diagnostics, including error severity, remain inside cached `Ok(FileFacts)` values. Unknown paths are distinct from syntax errors through the outer `Option`.

### Ownership and lifetime

Returned `Arc` values can outlive source updates and database destruction. Updating a file produces its current result without mutating results already held by a caller. Facts IDs remain local to a particular result; this API does not establish stable declaration or reference IDs across revisions.

The initial API provides registration, updates, and analysis only. Salsa inputs and current memos are retained until the database is dropped. Previously returned results remain allocated while callers retain their `Arc` values. File removal, reset, eviction controls, database snapshots, and parallel query orchestration are outside this stage.

## Alternatives

| Alternative | Assessment |
| --- | --- |
| A manually invalidated `HashMap` cache | Sufficient for one query and fewer dependencies. Salsa supplies dependency tracking, lazy validation, and result comparison that can also support future derived queries. |
| Public Salsa inputs and queries | Allows consumers to compose Salsa queries directly, but exposes version-specific database types and lifetimes. The selected Atlas wrapper keeps that dependency inside the feature's implementation. |
| Enable Salsa by default or add a separate crate | A default dependency affects every consumer. A separate crate adds workspace and release coordination. An optional feature keeps this initial API alongside the existing owned model and test harness. |
| Cache parser ASTs or split the analysis pipeline into queries | Requires new ownership boundaries around borrowed parser data and finer dependency contracts. File-level memoization reuses the existing complete analysis contract. |

## Consequences

Consumers can update buffers and request analyses without rerunning unchanged files or copying their entire result graphs. Existing single-file consumers retain the default dependency set. The new API caches errors as well as facts and preserves the source-positioned owned model.

An edit still reparses the complete changed file. Registered inputs and cached results consume memory for the lifetime of the database, and retained old `Arc` results extend their own lifetimes. Atlas maintains an additional pinned dependency, feature combinations, and deterministic reuse tests. This stage makes no benchmark speedup claim.

## References

- [Single-file ownership decision](0001-single-file-analysis.md), [incremental implementation](../../src/incremental.rs), and [public exports](../../src/lib.rs).
- [Pinned dependencies and feature definitions](../../Cargo.toml), [verification strategy](../testing.md), and [specification coverage](../specification.md).
- [Salsa 0.28.5 manifest](https://github.com/salsa-rs/salsa/blob/salsa-v0.28.5/Cargo.toml).
- [Salsa inputs, tracked functions, return modes, and lifetimes](https://docs.rs/salsa/0.28.5/salsa/) and [tracked query options](https://docs.rs/salsa/0.28.5/salsa/attr.tracked.html).
- [Salsa's execution-event test example](https://github.com/salsa-rs/salsa/blob/salsa-v0.28.5/examples/calc/db.rs).
