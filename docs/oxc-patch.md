# Oxc dependency patch

Base: [withastro/oxc 8bb526fc0c20beb4649b223d3ac39851505caa5a](https://github.com/withastro/oxc/tree/8bb526fc0c20beb4649b223d3ac39851505caa5a). Dedicated branch: `feat/atlas-script-parsing`.

Pinned fork commit: [8ef5b6a720bab87987dbc33f569af9d784c06850](https://github.com/jp-knj/oxc-atlas/commit/8ef5b6a720bab87987dbc33f569af9d784c06850). All atlas Oxc dependencies are pinned together; no Cargo cache source is edited and no local path override is part of the delivered manifest.

## Changes

- Add `ParseOptions::parse_astro_script_content`, default true. The existing Astro parse behavior remains the default. Disabling it retains template nodes, attributes and exact script body spans without parsing the browser programs.
- Atlas disables that pass, then chooses TS/module, inline-module JS or classic JS for each body, with original byte offsets, before semantic analysis. This also permits distinct script-local `define:vars` declarations instead of resolving browser references to server symbols.
- Update the two complete ParseOptions initializers in formatter/playground so the workspace remains compatible with the added field.
- Add deferred-body tests with invalid JS, default-mode regression, leading comments, Unicode and CRLF.
- Update two stale unquoted-attribute tests to match the base commit's deliberate HTML5 grammar change and the published compiler package: `value=4/>` has value `4/`, and `<Comp value= />` fails at EOF because the component is unclosed. Production attribute grammar is unchanged. Rustfmt also updates existing nearby formatting.

No AST definitions or generated files are modified. No upstream issue or PR has been submitted. This implementation was prepared with AI assistance and reviewed against the original code, the pinned native compiler and executable regression cases.

## Verification

All commands use Rust 1.91.1. Debug/incremental artifacts were disabled for the large Oxc checks because the workspace had limited disk space.

| Check | Result |
| --- | --- |
| `cargo +1.91.1 test --locked -p oxc_parser --features astro,regular_expression` | 274 passed, zero failed/ignored |
| `cargo +1.91.1 coverage parser --filter misc` | 62 positive and 134 negative parser cases passed. Test262/Babel/TypeScript were not downloaded, so these are not full external-suite results. |
| `cargo +1.91.1 fmt --all -- --check` | Passed |
| `just fmt` with `RUSTUP_TOOLCHAIN=1.91.1` | Rust and JS formatting passed after installing the workspace Node dependencies. Optional `cargo-shear` is unavailable; that step is allowed to fail by the recipe. |
| `just allocs` with `RUSTUP_TOOLCHAIN=1.91.1` | Could not build mimalloc's allocation tool because `cmake` is unavailable. No allocation snapshots were changed. |
| `just ready` | Dependency installation succeeded; stopped at `typos`, which is unavailable. Full workspace readiness is not claimed. |

An initial command included an extra `--` after the `cargo coverage` alias, which ran additional misc tools. Its parser cases passed, while semantic transform/rebuild checks reported 13 unrelated cases. The parser-only command above corrects the invocation. Those transform discrepancies are not claimed fixed by this parser-option patch.
