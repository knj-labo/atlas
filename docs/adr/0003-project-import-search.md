# ADR 0003: Project import resolution and file-usage search

Status: Accepted

Date: 2026-10-16

## Context

File-local queries can identify an imported binding and its lexical uses, but do
not identify the imported file or other files that use it. Agents need those
relationships while retaining the source version corresponding to every span.
Adding a previously missing target, removing a file, or changing resolver settings
must invalidate earlier import decisions, even if the importer text is unchanged.

This stage searches direct file dependencies and their identifier uses. Exported
declaration identity and re-export traversal require an export model and are not
part of this stage. Standalone JavaScript/TypeScript and type engines remain later
stages. An imported namespace root counts as a module use without asserting that
any particular member exists.

## Decision

Expose project facts, module resolvers, and uncached `analyze_project` and
`analyze_project_with_resolver` functions with default features. Extend the
optional `incremental` API with `remove_file`, `set_module_resolver`, and
`AnalysisDatabase::analyze_project` for callers that want caching. No default
runtime dependency is added.

### Uncached analysis and shared indexing

Uncached calls accept an iterator of path/source pairs. Paths convert to `PathBuf`
and sources to `Arc<str>`. Duplicate keys under `PathBuf` equality select the last
source before analysis. Every call analyzes those final sources and resolves
static imports anew; later snapshot queries perform no parsing or resolution.
The custom resolver is borrowed during construction and is not retained.

Both uncached entry points and the optional database use the same private
snapshot builder. The builder accepts exact sources and their analyses, so the
query layer has no Salsa dependency. File syntax errors remain available as
`ProjectFile.analysis` and do not fail the complete project call.

`ProjectSnapshot::revision()` returns `Option<ProjectRevision>`: uncached results
have no database revision (`None`), while database results return `Some` with
source-set and resolver-setting counters.

### Source and result ownership

`ProjectSnapshot` owns an ordered map of supplied keys to `ProjectFile` records.
Each contains the exact source as `Arc<str>` and its corresponding
`Arc<Result<FileFacts, AnalysisError>>`. With caching enabled, the private Salsa
input also stores `Arc<str>`, so snapshots share immutable buffers rather than
copying source text.
File facts and IDs are unchanged. An ID must be paired with its file and result;
project APIs borrow the canonical records from their retained analyses.

Failed files remain visible in the snapshot but supply no outgoing dependencies
or identifier references. Resolving a module to a supplied key does not imply
that its source parsed successfully or that its named exports exist.

### Resolution boundary

`ModuleResolver` is a `Send + Sync` adapter with an immutable request containing
the importer key, original `Import`, and scope domain. `ProjectSources` supplies
the complete supplied source snapshot in path order. Type information is not
part of this interface. Implementations must be deterministic for immutable
settings, the request, and that snapshot. Changing external inputs must be
represented by replacing the resolver or updating supplied source buffers.

`ModuleResolution` distinguishes `Resolved`, `Unresolved`, `Unsupported`, and
`Ambiguous`. A resolved target must be an exact supplied key; the snapshot builder
converts absent keys to `Unresolved`. This avoids indexing nonexistent files when
an adapter returns an outdated configured target.

Custom `Ambiguous` candidates are retained as reported for inspection; target-key
validation applies to `Resolved` results, and ambiguous candidates create no edges.

The default `RelativeResolver` is a virtual-file policy, not a Node, TypeScript,
or Vite resolver. It matches explicit relative paths after lexical normalization
of dot and parent components and collapsing repeated separators. It returns the
original key and reports ambiguity when distinct keys normalize to the same
location. Input key identity still follows `PathBuf` equality, without
canonicalization, symlink lookup, filesystem
access, or case folding. It does not infer extensions or directory indexes, and
does not handle packages, aliases, URLs, absolute specifiers, query/hash suffixes,
backslash paths, or directory forms ending in `/`, `/.`, or `/..`. Callers can
implement another policy through the adapter without changing the query API.

[TypeScript PR #64299](https://github.com/microsoft/TypeScript/pull/64299) separates
module resolution from program construction and supports snapshot-aware resolver
overrides. That separation informs this boundary; Atlas does not execute or depend
on TypeScript. A later adapter can translate external module decisions into these
file keys and resolution states independently of a type-analysis adapter.

### Indexes and query semantics

Build static-import dependencies from each successful `FileFacts`. Match import
binding spans to canonical declaration IDs, then index only references whose
lexical resolution is `Resolved` to one of those IDs. The local resolution remains
unchanged; it is distinct from module resolution. Equal names, scope shadowing,
builtins, unresolved identifiers, and runtime candidates never create false edges.

`imports_from` returns all static import declarations, including unused and
side-effect imports. `imports_to` returns direct declarations targeting a file.
`references_to_file` returns definite lexical uses of imports targeting that file.
`reference_at` returns a local reference and its originating import when present,
including unresolved module decisions. Results are ordered by path and source
position. Cycles are ordinary direct edges; the builder performs no graph traversal.
Re-exports, dynamic imports, script `src`, CSS dependencies, and transitive searches
are not indexed.

### Invalidation and lifetime

With the optional `incremental` feature, each database maintains separate monotonic
source-set and resolver revision counters. They describe one database's revisions
and are not globally unique IDs.
An actual source change, registration, or successful removal advances the source
revision. Every resolver replacement advances the resolver revision. Identical
source updates and removal of an absent key are no-ops.

A `OnceLock<Arc<ProjectSnapshot>>` caches the complete project result. Updates
clear it through the exclusive database reference; the next project request
rebuilds all resolution and reverse indexes from current files and settings.
This conservative invalidation includes negative lookups and dependencies on any
source examined by a custom resolver. Salsa continues to memoize file analysis
independently, so unchanged files reuse their results across index rebuilds.

Removing a key excludes it from subsequent file and project queries; re-adding it
creates a new private input. Internal Salsa allocations are retained until the
database is dropped. Previously returned snapshots and file results remain valid,
with their own source text and decisions, until their last owners release them.

## Alternatives and consequences

Fine-grained resolver queries could avoid rebuilding unrelated edges, but require
tracking missing-file lookups, arbitrary resolver reads, and configuration inputs.
Whole-project invalidation provides explicit correctness boundaries for this first
adapter API. No project-scale performance claim is made.

Embedding cross-file resolutions into `FileFacts` would couple local facts and IDs
to project settings and invalidate existing single-file caches. A separate snapshot
preserves the existing model, serialization, and snapshot baselines, while storing
reverse indexes and shared references to analysis results.

## Validation

Default-feature [project tests](../../tests/project.rs) cover canonical borrowing,
relative path ambiguity, source ordering, cycles, syntax errors, aliases, types,
namespace roots, Unicode/CRLF, script domains, input ownership, duplicate keys,
and uncached resolver execution.
[Incremental project tests](../../tests/project_incremental.rs) exercise resolver
replacement, file addition/removal/re-addition, source updates, negative lookup
invalidation, and retained snapshots. They also compare cached and uncached
results across all query methods.
Existing file-level memoization tests, fixture snapshots, compiler comparisons,
feature combinations, doctests, and Clippy remain required.
