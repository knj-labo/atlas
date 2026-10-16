# atlas

Rust analysis library for agents inspecting declarations, uses, and dependencies
before changing code. The current API analyzes one Astro file, including its
frontmatter, template expressions, browser scripts, and CSS facts.

```rust
let source = "---\nimport Button from './Button.astro';\n---\n<Button />";
let facts = atlas::analyze(source).unwrap();
let button = facts.declarations_named("Button")[0];
let uses = facts.references_to(button.id);
let offset = source.find("<Button").unwrap() as u32 + 1;
let reference = facts.reference_at(offset).unwrap();
assert_eq!(reference, uses[0]);
```

| `FileFacts` method | Result |
| --- | --- |
| `declarations_named(&str) -> Vec<&Binding>` | All exact, case-sensitive name matches, including distinct scopes and local import aliases, in source order. |
| `reference_at(u32) -> Option<&Reference>` | The recorded reference containing an original-source UTF-8 byte offset in its half-open span, with its existing `Resolution`. |
| `references_to(DeclarationId) -> Vec<&Reference>` | Definite value and type uses of a declaration in the same result, in source order. |

Queries borrow the existing records without changing them. Unknown names and IDs
return empty vectors; positions outside recorded reference spans return `None`.
Unresolved, builtin, and runtime-dependent references are available through
`reference_at` but are not definite declaration uses. CSS candidates are separate
facts and are not searched by these methods. Syntax failures use the existing
`AnalysisError`; successful facts may still contain Astro validity diagnostics.

IDs are local to one analysis result. After a source update, obtain IDs from the
new result. Numeric IDs can be reused; queries cannot recognize IDs from an older
result or another file. With the optional `incremental` feature,
`AnalysisDatabase` caches analyses of caller-supplied paths and text. The caller
handles file access and change notifications.

Enable `incremental` for project-wide import lookup over registered Astro buffers:

```rust
let mut db = atlas::AnalysisDatabase::new();
db.set_source("Button.astro", "<button />");
db.set_source("Page.astro", "---\nimport Button from './Button.astro';\n---\n<Button />");
let project = db.analyze_project();
assert_eq!(project.imports_to("Button.astro").len(), 1);
assert_eq!(project.references_to_file("Button.astro").len(), 1);
```

`ProjectSnapshot` retains the exact sources and analyses for its revision.
`imports_from(path)` lists dependencies and their `ModuleResolution`;
`imports_to(path)` lists direct importers, including unused and side-effect
imports. `references_to_file(path)` lists definite lexical uses of those imports,
and `reference_at(path, byte)` combines a local reference with its originating
import. Results borrow existing facts and are ordered by path, then source position.
Inspect `file(path).analysis` for syntax errors; failed importers have no indexed
outgoing imports. A module may resolve to a registered file with an analysis error.

The default `RelativeResolver` matches explicit relative paths against registered
virtual files, normalizing dot/parent components and repeated separators without
filesystem I/O. It preserves the exact registered keys and reports ambiguous matches. Extension
inference, directory indexes, packages, aliases, and query/hash suffixes are not
implemented by this resolver. Directory forms ending in `/`, `/.`, or `/..` are
also unsupported. Use `set_module_resolver` with a `ModuleResolver`
implementation for other policies. Replace the resolver when its settings change.

Source changes, file registration/removal through `remove_file`, and resolver
replacement invalidate the project snapshot. Unchanged file analyses are reused;
previous snapshots remain valid. `revision()` identifies source and resolver
versions within one database. This stage indexes files, not exported declarations:
named-export validation, re-exports, dynamic imports, and transitive traversal are
not implemented. See [ADR 0003](docs/adr/0003-project-import-search.md).

Planned stages are standalone JavaScript, then standalone TypeScript and external
type analysis. Module resolution has its own adapter interface; no TypeScript
runtime dependency is required. CLI and MCP integrations will consume the Rust API.

See the [specification](docs/specification.md) for current coverage and boundaries,
and [testing guide](docs/testing.md) for verification commands.
