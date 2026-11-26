# Agent guide

## What a build checks

`.cargo/config.toml` makes clippy the compiler for workspace crates, so every
`cargo build`, `check`, `run` and `test` fails on a production function over
100 lines and on any rustc or default clippy warning, in every profile and
feature set. Measured on a one-line `core` edit under `--profile fast`, this
adds no time that rises above the noise (about 64 s with or without it).

Every crate root forbids the line cap outside test builds, so no `allow` or
`expect` can lift it. A new crate takes `[lints] workspace = true` and:

```rust
#![cfg_attr(not(test), forbid(clippy::too_many_lines))]
#![cfg_attr(test, allow(clippy::too_many_lines))]
```

Set any other lint aside for one item, with `expect` and a reason.

Module-level functions and legacy directory names are not build errors; the
architecture test catches them.

## Module layout and naming

These rules are normative for every crate in the repository. The architecture
test (`cargo test -p core --test function_structure`) enforces the
directory-name rules and the ban on module-level functions.

- A domain module is named after the domain (`club`, `country`, `league`,
  `transfers`, …). Its aggregate lives at the module root or in `model/`;
  `builder.rs` and `context.rs` are its siblings.
- `core/` and `local/` are not module names. Name the responsibility:
  `model/` for the aggregate and its data, `process/` for cross-domain
  orchestration and tick phases, `ticks/` for per-tick processors, and
  `result/` for result state plus the code that drains it.
- A directory and the type it owns share a name: `CountryTicks` lives in
  `country/process/ticks`, `CountryResult` in `country/result`, `League`
  in `league/model`, `Player` in `club/player/model`, `Club` in `club`.
- `result/` never owns domain behaviour or tick orchestration. It owns the
  result types and their application path.
- Do not create catch-all `misc`, `helpers`, or domain-level `utils`
  modules. Name the concern (`standings`, `awards`, `scouting`, `growth`).
  `shared/` is only for code genuinely shared by sibling modules; crate-level
  `shared` and `utils` namespaces are for cross-domain primitives only.
- `common/` is not a module name. Use `shared/` when the code really is shared
  by the sibling modules of one domain, or name the concrete concern.
- Process phases live in their own file named after the phase, not in a
  shared `mod.rs`. A phase that grows a family becomes a directory under
  `process/`.
- Tests live beside the code they test (`#[cfg(test)] mod tests` or
  `tests.rs`); production naming rules have no test-only exceptions.

Canonical country tree:

```text
country/
├── builder.rs
├── context.rs
├── country.rs
├── process/
│   ├── awards.rs
│   ├── clubs.rs
│   ├── league.rs
│   ├── market.rs
│   ├── period.rs
│   ├── results.rs
│   └── ticks/
│       ├── media.rs
│       ├── preseason.rs
│       ├── registration.rs
│       ├── reputation.rs
│       └── statistics.rs
├── result/
├── economy/
├── media/
├── national/
└── rules/
```

The architecture test rejects `core`, `local`, and `common` directories in
every crate source tree, so new modules cannot reintroduce the legacy names.
