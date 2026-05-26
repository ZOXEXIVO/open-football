# LLM Polishing Prompt: Club Board Logic Realism Pass

You are working in `open-football`, a Rust football-management simulator. The club board system has already been expanded with ownership, pressure, promises, relationship trust, component scoring, infrastructure reviews, board decisions, and takeover logic. Your task is not to redesign it from scratch. Your task is to polish, calibrate, and integrate the existing implementation so it behaves consistently in long simulations and is explainable like a real Football Manager-style board.

## Read First

Start with these files:

- `src/core/src/club/board/board.rs`
- `src/core/src/club/board/context.rs`
- `src/core/src/club/board/result.rs`
- `src/core/src/club/board/decision/mod.rs`
- `src/core/src/club/board/scoring/mod.rs`
- `src/core/src/club/board/ownership/mod.rs`
- `src/core/src/club/board/pressure/mod.rs`
- `src/core/src/club/board/relationship/mod.rs`
- `src/core/src/club/board/promise/mod.rs`
- `src/core/src/club/board/infrastructure/mod.rs`
- `src/core/src/club/board/takeover/mod.rs`
- `src/core/src/club/club/mod.rs`
- `src/core/src/transfers/pipeline/shortlists.rs`

Keep the current architecture. Make focused, test-backed improvements.

## Current Strengths

The board system now has the right shape:

- Ownership archetypes are persistent and affect budgets, autonomy, transfer governance, facilities, pressure, and takeover eligibility.
- `BoardContext` carries richer inputs for scoring and infrastructure decisions.
- Component scoring separates sporting, financial, squad-building, and strategy reasons.
- `BoardResult.decisions` exposes machine-readable decisions for UI/news.
- Infrastructure review and takeover watch exist.
- Manager relationship has separate trust facets.
- Transfer review can use both scouting dossier and economic dossier.

## Main Polishing Problems To Fix

### 1. Remove Double Budget Effects

`BoardResult::process` still applies legacy mood effects:

- Poor mood cuts transfer budget by 25%.
- Excellent mood adds 20%.

The new `BoardDecision` system can also emit `CutTransferBudget` and `IncreaseTransferBudget`. This can double-apply budget changes in the same tick.

Fix this by making decisions the single source of truth:

- Either convert legacy `cut_transfer_budget` / `bonus_transfer_funds` into `BoardDecision` entries in `evaluate_performance`, or keep legacy fields only as UI compatibility flags but do not apply them separately.
- Add regression tests that a Poor + FFP breach month applies exactly one intended cut, not both percentage and amount cuts.
- Add regression tests that Excellent + owner injection does not stack uncontrolled percentage and fixed increases unless explicitly intended.

### 2. Make Takeovers Deterministic Under Simulation Seeds

`tick_takeover` uses `IntegerUtils::random(0, 100)`. That may be globally random and difficult to replay. The takeover module itself is deterministic if passed a `roll`.

Polish goal:

- Source takeover rolls from an existing deterministic simulation RNG if available.
- If no seeded RNG exists in `GlobalContext`, derive a stable roll from club id + date + takeover months in status.
- Avoid nondeterministic board outcomes in tests and saves.
- Add tests proving identical club/date/state produces identical takeover decisions.

### 3. Finish Promise Lifecycle

`PromiseLedger` exists, but promises are mostly passive. Build realistic creation and fulfilment flows.

Add or wire:

- Create `TransferBudget` promise when the board approves a transfer-budget increase after a manager warning or takeover.
- Create `FacilityImprovement` promise when the board rejects a requested upgrade but commits to revisit it next season.
- Create `YouthMinutes` promise for youth-focused/member-owned boards when appointing/backing a manager.
- Create `Survival`, `ContinentalQualification`, or `TitleChallenge` promise from season targets/long-term goal.
- Fulfil promises when corresponding decisions happen or season outcomes are met.

Requirements:

- Promises should not duplicate endlessly; use `has_active`.
- Add tests for promise creation, fulfilment, overdue breakage, and trust impact.

### 4. Replace Proxy Context Defaults With Real Data Or Explicit Neutrality

Several `BoardContext` fields are useful but may still be proxies/defaults:

- `transfer_budget_usage`
- `academy_graduates_this_season`
- `u21_minutes_share`
- `injury_crisis_score`
- `key_player_unrest_count`
- `manager_contract_months_left`
- `distance_to_relegation`
- `distance_to_europe_or_playoff`
- `points_per_match`
- `goal_difference`

For each field:

- Trace whether `Club::build_board_context` populates it from real data.
- If real data exists elsewhere, wire it.
- If not, keep it neutral and add one concise TODO with the exact missing source.
- Avoid fake precision. A bad proxy is worse than a neutral default.

Add tests around `build_board_context` for fields that can be derived locally from match history, contracts, injuries, and finances.

### 5. Calibrate Component Scores

Run through `BoardComponentScores::evaluate` and make the ranges coherent:

- A normal mid-table team meeting expectations should produce near-zero confidence delta.
- A title challenger in 7th halfway through the season should get a warning but not instant sacking.
- A relegation candidate in 15th should not be punished like a big club in 15th.
- FFP breach should hurt finances strongly, but not make sporting performance irrelevant.
- Injury crisis should soften sporting/squad blame, not reward bad squad building.

Add table-driven tests for archetypes:

- elite state-backed title challenger
- mid-table family-owned club
- private-equity selling club
- member-owned youth club
- relegation survivor

Each test should assert component signs and confidence movement, not exact fragile scores unless necessary.

### 6. Improve Season Target Calculation

The current expected position is mostly reputation-to-table-position. Polish it with:

- league tier
- promoted/relegated status if available
- previous season finish if available
- wage rank or squad ability rank if available
- ownership ambition
- long-term goal

Do not hard-code clubs. Use available data. If previous-season finish or wage rank is unavailable, document the missing source and use a conservative fallback.

Add tests:

- low-rep promoted side expects survival, not mid-table.
- rich state-backed owner raises expected position.
- conservative small club does not demand impossible finish.

### 7. Make Ownership Bootstrap Stable And Save-Friendly

`bootstrap_personality` derives ownership once from context and club id. Check persistence semantics:

- If the save/load system serializes `ClubBoard`, ensure `personality_initialized` and `ownership` persist.
- If not, make derivation idempotent and stable.
- Avoid re-randomizing personality after every load.

Also check takeover completion:

- `apply_takeover_completion` currently forces Stars + WinLeague + Low autonomy. That is too narrow.
- Map post-takeover owner type to varied strategy:
  - State-backed: stars, ambitious, trophies.
  - Private equity: resale value, wage control, facility/commercial priority.
  - Consortium: prime-age/balanced, top-half or continental goal.

Add tests for each post-takeover archetype.

### 8. Make Board Decisions Auditable

`BoardDecision` should be good enough for UI/news/debugging.

Polish:

- Ensure every decision has a reason where useful.
- Add amount/cost signs and units consistently.
- Add helper methods:
  - `decision.kind()`
  - `decision.reason() -> Option<DecisionReason>`
  - `decision.is_actionable()`
  - `decision.is_public_newsworthy()`
- Ensure `BoardResult` emits matching decision entries for legacy meeting/sacking fields.

Tests:

- every decision variant has a stable `kind`.
- actionable decisions are exactly the ones `BoardResult::process` mutates.

### 9. Connect Transfer Economics More Deeply

`BoardTransferEconomics` exists, but make sure transfer pipeline call sites actually populate it.

Check `src/core/src/transfers/pipeline/shortlists.rs` and related recruitment/negotiation modules.

Polish:

- wage impact from proposed salary, not guessed.
- wage headroom from board season target/current annual wages.
- resale projection from age, ability, potential, contract length.
- homegrown/domestic fit from country/registration data if available.
- injury risk from player injury model if available.
- commercial value from reputation/star status if available.

Keep missing fields neutral instead of invented.

Add tests:

- conservative board vetoes wage-headroom breach.
- private-equity board flags poor resale.
- state-backed board allows elite exception.
- member-owned board values homegrown fit.

### 10. Avoid Overactive Infrastructure Upgrades

Facility review runs at season start and may approve upgrades aggressively for wealthy owners.

Polish:

- Add cooldown per facility or per board so a club cannot upgrade every single season unrealistically.
- Do not approve stadium expansion only from `attendance_ratio`; require sustained high demand or explicit priority.
- Ensure `BoardFacility::Stadium` does not silently do nothing if the club has no stadium model. If stadium capacity is not modeled, emit a news-only decision or add TODO and avoid debiting money for an invisible upgrade.
- Facility upgrade cost should scale by country price level and club level.

Tests:

- no duplicate facility upgrade while cooldown active.
- stadium decision does not debit money unless a real stadium state changes.
- FFP breach blocks capex.

### 11. Long Simulation Invariants

Add tests or a small deterministic simulation smoke test for 12-36 months:

- confidence stays within 0..100.
- pressure stays within 0..100.
- relationship facets stay within 0..100.
- budgets never become negative from board processing.
- no club repeatedly sacks managers every month after a reset.
- takeover rumour eventually resolves.
- promise ledger does not grow unbounded.

Prefer targeted tests over broad slow tests, but include at least one board-only multi-month progression test.

## Implementation Constraints

- Keep public APIs backwards compatible unless all call sites are updated.
- Do not add fields that are never read.
- Do not use hard-coded club names.
- Prefer deterministic calculations over unseeded randomness.
- Keep scoring explainable: comments should explain why a term exists, not narrate obvious code.
- Avoid broad refactors outside `club/board`, `club/club/mod.rs`, and transfer pipeline wiring unless necessary.

## Commands To Run

Run after changes:

```bash
cargo test -p core club::board
```

If transfer economics wiring is touched:

```bash
cargo test -p core transfers::pipeline
cargo test -p core club::transfers
```

If `Club::build_board_context` or result processing changes:

```bash
cargo test -p core club::result
cargo test -p core country::result
```

Before finishing, summarize:

- which polishing problems were fixed,
- which context fields still use neutral defaults,
- which decisions mutate club state,
- and which realism gaps remain.

