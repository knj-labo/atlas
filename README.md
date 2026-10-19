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
