# atlas

Owned semantic facts and read-only queries for Astro and ESM JavaScript/TypeScript.
Call `analyze(source)` for Astro, `analyze_file(path, source)` for path-selected
syntax, or `analyze_project(sources)` for cross-file definition queries over
caller-supplied paths and source text. These APIs perform no file I/O.

```rust
let source = "import { value } from './value.ts'; value;";
let project = atlas::analyze_project([
    ("value.ts", "export const value = 1;"),
    ("use.ts", source),
]);
let definition = project.definition_at("use.ts", source.rfind("value").unwrap() as u32);
assert!(matches!(definition, Some(atlas::DefinitionResolution::Resolved(_))));
```

The optional `incremental` feature caches file analyses and project snapshots.
The `serde` feature exposes an encoding-independent `ProjectReport` with schema
version 1. The report example reads a directory and defaults to TOON:

```sh
cargo run --locked --features serde --example atlas-report -- ./src
cargo run --locked --features serde --example atlas-report -- ./src --format json
```

Supported extensions are `.astro`, `.js`, `.mjs`, `.jsx`, `.ts`, `.mts`, `.tsx`,
`.d.ts` and `.d.mts`. CommonJS, `.cjs`, `.cts` and `.d.cts` are outside the contract.
The directory example skips symlinks and generated/dependency directories.
See the [specification](docs/specification.md), [testing](docs/testing.md), and
[ESM/definition/report ADR](docs/adr/0006-esm-definitions-and-toon.md).

Merged declarations are available through a symbol; definition queries select a
representative for the reference's meaning (including type-position `typeof`):

```rust
let facts = atlas::analyze_file("types.ts",
    "interface Item { a: string } interface Item { b: number }").unwrap();
let binding = facts.declarations_named("Item")[0];
let symbol = &facts.symbols[binding.symbol.0];
assert_eq!(symbol.declarations.len(), 2);
assert_eq!(symbol.declaration(&facts, atlas::Meaning::Type).unwrap().id, binding.id);
```

Custom resolvers use `request.source`, `span` and `source_span`, and inspect the
original fact through `request.dependency` (`ModuleDependency::Import` or
`ModuleDependency::Export`). The request ranges cover the complete statement and
quoted module literal respectively. See [ADR 0007](docs/adr/0007-symbols-and-resolution-index.md).
