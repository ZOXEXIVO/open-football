# core/architecture Specification

## Purpose

Defines the source-structure rules every crate in the repository obeys and when each is checked, so a change cannot reach a release while breaking them.

## Requirements

### Requirement: Production functions stay within one hundred lines
Every production function in every crate SHALL count at most 100 lines, as clippy's `too_many_lines` lint counts them: lines in the function body that hold code, not counting blank lines or lines that are only comments. Test code is exempt: items under `cfg(test)` and files in `tests/`, `examples/` and `benches/` directories. Every crate root SHALL forbid the lint in production builds, so that production code cannot allow or expect it, either on an item, on a module or crate-wide.

#### Scenario: A production function grows past the cap
- **WHEN** a production method's body reaches 101 counted lines
- **THEN** the build of its crate fails with an error naming the file and line, and the count against 100

#### Scenario: Blank and comment lines are not counted
- **WHEN** a production function has 100 code lines plus blank lines and comment-only lines
- **THEN** the build of its crate succeeds

#### Scenario: A long test function
- **WHEN** a function inside a `#[cfg(test)]` module has more than 100 counted lines
- **THEN** the crate still builds, and its tests compile and run

#### Scenario: Silencing the cap in production code
- **WHEN** a production item or module carries `allow` or `expect` of `clippy::too_many_lines`, or of `clippy::pedantic`, the group it belongs to
- **THEN** the build of its crate fails with an error pointing at the attribute

### Requirement: The line cap and warnings are enforced by every build
`cargo build`, `cargo check`, `cargo run` and `cargo test`, in any profile and for any package or feature selection, SHALL fail when a compiled workspace crate breaks the line cap or raises a warning. No separate command, feature flag or environment variable is needed to turn the check on.

#### Scenario: Running only a package's tests
- **WHEN** `cargo test -p web` runs on a tree where a `web` production function is 120 lines long
- **THEN** the command fails while compiling `web`, before any test runs

#### Scenario: A release build
- **WHEN** `cargo build --release` runs on a tree with a production function over the cap
- **THEN** the build fails and no binary is produced

#### Scenario: A clean tree
- **WHEN** every crate obeys the rules
- **THEN** builds print no compiler, clippy or cargo lint warnings

### Requirement: Builds are warning-free
Every crate SHALL compile with no rustc warnings and no findings from clippy's default lint groups, in default and feature builds. Any such warning SHALL fail the build of its crate. A change SHALL NOT add a crate-wide or module-wide lint suppression to production code: it sets a lint aside for one item, with a stated reason, preferring `expect`, which fails the build once it goes stale. Dependencies are not held to this rule.

#### Scenario: A new warning is introduced
- **WHEN** a change adds an unused variable, or code that clippy's default groups flag, to a production or test item
- **THEN** the build of that crate fails with the warning reported as an error

#### Scenario: A justified exception
- **WHEN** an item carries `#[expect(clippy::<lint>)]` with a reason, and the lint fires on that item
- **THEN** the build succeeds. When a later change removes what the lint fired on, the build fails on the unfulfilled expectation

#### Scenario: Warnings in dependencies
- **WHEN** a third-party dependency's code would raise warnings
- **THEN** the workspace build is unaffected

### Requirement: Production code has no module-level functions
Production code in every crate SHALL define functions only as methods or associated functions of a type, or as trait items. A free `fn` at module scope is a violation, whether the module is a file or an inline `mod` block. These are exempt:

- `main` in the root binary;
- build scripts;
- WebAssembly start functions (`#[wasm_bindgen(start)]`);
- `#[no_mangle] extern "C"` entry points;
- code under `cfg(test)`;
- files under `tests/`, `examples/`, `benches/` and `.dev/`.

The architecture test in `core` SHALL check this on every crate, and the repository SHALL carry no baseline of tolerated violations.

#### Scenario: A free helper function is added
- **WHEN** a production module declares `fn helper()` at module scope
- **THEN** `cargo test -p core` fails with an error naming the function by its file and module path

#### Scenario: Methods and test helpers are accepted
- **WHEN** a module defines functions only inside `impl` blocks, and free helper functions only inside a `#[cfg(test)]` module
- **THEN** the architecture test passes

### Requirement: Module directories use named responsibilities
No directory in any crate's source tree SHALL be named `core`, `local` or `common`. These names hold no responsibility. Code goes in `model/`, `process/`, `ticks/`, `result/`, `shared/`, or a directory named after the concern. The architecture test in `core` SHALL check this.

#### Scenario: A legacy directory name is reintroduced
- **WHEN** a directory named `common` is created anywhere under a crate's `src/`
- **THEN** `cargo test -p core` fails with an error naming the directory

### Requirement: The architecture test covers every crate
The architecture test SHALL fail when a crate manifest under `src/` belongs to a crate it does not scan.

#### Scenario: A new crate is added without an entry
- **WHEN** a new crate with a `Cargo.toml` is added under `src/` and the test has no entry for its root
- **THEN** `cargo test -p core` fails with an error naming the crate

### Requirement: Continuous integration rejects violations independently
The CI pipeline SHALL fail when any crate breaks a rule of this capability, even if a contributor builds with the local check turned off. The wasm match viewer is included: its nested build inside the web crate turns compile failures into warnings.

#### Scenario: The local wrapper was bypassed
- **WHEN** a commit with a 130-line production function is pushed after being built locally with the compiler wrapper cleared
- **THEN** the CI image build fails and nothing is deployed

#### Scenario: A viewer violation
- **WHEN** the match viewer crate gains a free function at module scope, or a production function over the cap
- **THEN** the CI image build fails, even though the web crate's nested viewer build only reports a warning
