# Project-specific instructions for Claude

## Imports / paths
- Do use explicit ns for types. Use 'use' at file header

## Architecture — SOLID, no cross-cutting hacks
- Each module owns its own domain. External code must NOT reach into
  another module's internals (no `player.happiness.add_event(...)` from
  transfer / match-result / behaviour code, no `player.statuses.add(...)`
  from the transfer pipeline, etc.).
- Side effects of "something happened to X" belong on X as `on_*` methods
  (`Player::on_match_played`, `Player::complete_transfer`,
  `Player::on_match_exertion`, …). The caller states the fact; the owner
  decides the reactions.
- Prefer event-driven / natural flow over injecting side effects at the
  call site. If a transfer execution function is the one pushing morale
  events and role-fit checks, that's a hack — let the Player react in
  its own processing tick by staging a small marker (e.g.
  `pending_signing`) and consuming it during the next simulate pass.
- New cross-domain logic goes behind a dedicated method on the owning
  type; the pipeline just dispatches. Transfer / match / behaviour
  modules should read as thin orchestration.

## Universal logic, no special cases
- Solutions must cover all cases with the same mechanism. If you catch
  yourself writing `if is_relegated { … } else if is_promoted { … }`, step
  back — there's usually a single continuous signal (league position,
  rep gap, ambition delta, …) that covers every branch when parameterised
  correctly.
- Don't ignore a category "because it's temporary" (loans, youth moves,
  friendlies). Dampen the magnitude if needed; never hard-gate a whole
  feature off for one kind of entity.

## Code style
- No global / free functions in domain code. Every helper, algorithm, or
  orchestrator must live as a method on a logical struct (e.g.
  `PlayerOfTheWeekSelector::aggregate(...)`, `WeeklyAwardsTick::run(...)`),
  not as a bare `fn` at module scope. Group functions by purpose under a
  named type so callers pull a single namespace and the unit test surface
  reads as one cluster. Test-only helpers inside `#[cfg(test)]` are
  exempt.
- No unnecessary comments. Explain the WHY when non-obvious — never the
  WHAT (identifiers do that).
- No multi-paragraph docstrings, no comment blocks narrating the current
  PR / task / fix ("added for Y flow", "fixes issue #123") — those belong
  in the commit message.
- Prefer editing existing files over creating new ones.
- Don't add error handling, fallbacks, or validation for scenarios that
  can't happen. Trust internal code and framework guarantees. Validate
  only at genuine system boundaries.
- Don't add backwards-compatibility shims, renamed `_vars`, `// removed`
  tombstone comments, or re-exports for deleted items. Delete cleanly.
- No speculative abstractions. Three similar lines beat a premature
  helper. Refactor when the third or fourth caller actually arrives.

## Shell tooling
- Use `rg` for searching, `sed` for reading line ranges and mechanical
  edits, and `jq` for querying JSON (locale files, API payloads, the
  gunzipped database) — in the Bash tool, not PowerShell.
- Prefer them over writing ad-hoc Python / PowerShell scripts or
  building a Rust harness for a one-off lookup.

## Localization
- Adding a key to `src/web/assets/i18n/en.json` means adding it to
  every other locale file (`de`, `es`, `fr`, `ja`, `pt`, `ru`, `tr`,
  `zh`) in the same change. No locale may lag behind.