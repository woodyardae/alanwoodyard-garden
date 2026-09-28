# Bosun Garden Compiler

[![Namespace](https://img.shields.io/badge/namespace-Bosun--PKM--Tools-1B4F72)](https://alanwoodyard.com)
[![Stack](https://img.shields.io/badge/stack-Rust_2021_%2B_yaml--rust2-1A5276)](https://alanwoodyard.com)
[![IP](https://img.shields.io/badge/IP-proprietary-7B241C)](https://alanwoodyard.com)

[Alan Woodyard](https://alanwoodyard.com)

## System Role

Bosun Garden Compiler turns a folder of Markdown notes with YAML frontmatter into a compiled digital-garden vault: pages, links, and tenant slugs. It is the batch compiler Bosun Studio links in process when the studio needs a garden build without shelling out to a hosted publisher.

Fleet entity: `bosun-garden`. Formal name: Bosun Garden Compiler. Tagline: high-performance Rust static compiler for digital garden notes and CommonMark taxonomies. Visibility `internal`. IP status `proprietary`. No `primary_remote` is registered, and this directory is not a git repository.

Harvest maturity: working core (2,940 implementation LOC, 552 test LOC).

## Audited Architecture & Runtime

Cargo workspace:

| Crate | Path | Role |
| --- | --- | --- |
| `bosun-garden-core` | `crates/bosun-garden-core` | Notes, links, graph, publish model |
| `bosun-garden-compiler` | `crates/bosun-garden-compiler` | YAML frontmatter via `yaml-rust2` |
| `bosun-garden-cli` | `apps/bosun-garden-cli` | Binary `bosun-garden` |

Imported libraries: `yaml-rust2`, `clap`, `marlinspike-ast`, `rayon`, `serde_json`, `thiserror`. Frontmatter parsing is YAML. Note structure reuses the Marlinspike AST rather than a second Markdown parser. `rayon` parallelizes the vault walk.

Exported items harvested from the crates: `compile_vault`, `CompiledPage`, `CompiledVault`, `CompileError`, `list_tenant_slugs`, `normalize_origin`, plus modules `emit`, `emitter`, `frontmatter`, `walk`, `graph`, `links`, `note`, `publish`.

Bosun Studio depends on `bosun_garden_core` and `bosun_garden_compiler` in process.

## CLI / API Surface

```text
bosun-garden build
```

Clap command name: `bosun-garden`. Harvested subcommand: `build`. No HTTP routes and no JSON-RPC methods were harvested. The programmatic API is `compile_vault` and the compiled-page types above.

## Operational Boundaries

Proprietary internal compiler. It reads a local vault and writes a compiled tree. There is no network client in the harvested import set.

This directory has no git remote. Do not assume a GitHub publish path until an origin is attached. Consumers today are in-process (Bosun Studio) or the local `bosun-garden` binary. Tenant slug rules (`list_tenant_slugs`, `normalize_origin`) are part of the compiler contract. They are not a license to publish another org's garden.
