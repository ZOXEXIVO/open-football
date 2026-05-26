# LLM Implementation Prompt: Realistic Club Board System

You are working in the Rust repo `open-football`, an analog of Football Manager. Improve the club/board simulation so it behaves like a real football club board: ownership, directors, finances, season expectations, supporter pressure, staff governance, transfer approvals, manager trust, facilities, and long-term strategy must all interact with the existing simulation.

## Current Code Map

Read these files first:

- `src/core/src/club/board/board.rs` - board state, season targets, transfer review, performance evaluation, sacking logic.
- `src/core/src/club/board/context.rs` - aggregated club data passed into board simulation.
- `src/core/src/club/board/result.rs` - effects applied after board simulation.
- `src/core/src/club/board/manager_market.rs` - manager search, shortlist, appointment, poaching.
- `src/core/src/club/club/mod.rs` - builds `BoardContext`, runs finance/team/board/academy simulation, syncs season budgets.
- `src/core/src/club/finance/*` - finance history, balance, sponsorship.
- `src/core/src/club/facilities.rs` - club facilities.
- `src/core/src/club/team/reputation/*` - reputation and achievements.
- `src/core/src/transfers/pipeline/*` - recruitment plan, shortlists, meetings, negotiation pipeline.
- `src/core/src/club/team/squad_life/*` and `src/core/src/club/player/happiness/*` - dressing-room/player mood systems that board decisions should eventually influence.

Keep changes idiomatic to this codebase: small domain structs, explicit result structs, deterministic calculations, and tests beside the module where possible.

## Product Goal

Create a board system that can produce believable stories without scripting:

- A patient academy-focused club tolerates a mid-table season if young players improve and wages are controlled.
- A reckless rich owner raises expectations, injects transfer funds, and sacks quickly after bad runs.
- A conservative board blocks expensive low-value transfers, sells players under FFP pressure, and rewards wage discipline.
- A club with strong fan pressure reacts harder to derby defeats, relegation danger, and unpopular sales.
- A director of football can support or conflict with the manager depending on recruitment vision and staff quality.
- The board should make visible decisions that the web UI can surface later: warnings, backing, budget changes, facility approvals, transfer veto reasons, takeover rumours, and manager objectives.

## Implementation Plan

### 1. Expand Board Domain Model

In `board.rs`, split board personality into persistent submodels:

- `OwnershipModel`
  - `ownership_type`: `MemberOwned`, `LocalBusiness`, `Consortium`, `StateBacked`, `PrivateEquity`, `FamilyOwned`.
  - `wealth`: 0-100.
  - `ambition`: already exists conceptually; keep or migrate.
  - `patience`: already exists conceptually; keep or migrate.
  - `interference`: 0-100, affects forced signings, pressure, and manager autonomy.
  - `risk_tolerance`: 0-100, affects debt, wage ratio, transfer budget.
  - `exit_pressure`: 0-100, raises takeover/sale behaviour.

- `BoardStrategy`
  - Current `ClubVision` fields.
  - Add `preferred_squad_profile`: youth, prime-age, stars, domestic, resale-value.
  - Add `infrastructure_priority`: training, youth, stadium, commercial, none.
  - Add `manager_autonomy`: low/medium/high.
  - Add `review_frequency`: monthly, quarterly, season-end-only.

- `BoardPressure`
  - `supporter_pressure`: 0-100.
  - `media_pressure`: 0-100.
  - `dressing_room_pressure`: 0-100.
  - `financial_pressure`: 0-100.
  - `regulatory_pressure`: 0-100.

- `BoardPromise`
  - `promise_type`: transfer budget, facility improvement, youth minutes, continental qualification, survival, title challenge.
  - `created_at`, `due_date`, `status`.
  - `trust_delta_on_success`, `trust_delta_on_failure`.

Do not add unused fields only. Every new field must influence at least one calculation or result.

### 2. Improve BoardContext

Extend `BoardContext` with the data a real board needs:

- `league_tier`
- `points_per_match`
- `goal_difference`
- `recent_goal_difference`
- `distance_to_target_position`
- `distance_to_relegation`
- `distance_to_europe_or_playoff`
- `attendance_ratio`
- `supporter_mood`
- `wage_budget_usage`
- `transfer_budget_usage`
- `debt_ratio`
- `profit_loss_12m`
- `academy_graduates_this_season`
- `u21_minutes_share`
- `injury_crisis_score`
- `manager_contract_months_left`
- `key_player_unrest_count`

Build these in `Club::build_board_context` or nearby helper methods. If some values are not currently available, add neutral defaults and clear TODO comments only where the data source is genuinely missing.

### 3. Replace Flat Confidence With Component Scoring

Change monthly evaluation from one linear confidence change into a component score:

- Sporting score
  - League position vs expected position.
  - Points per match vs target.
  - Recent form.
  - Goal difference trend.
  - Cup/continental progress when available.
  - Season phase weighting: early season is softer, run-in is harsher.

- Financial score
  - Balance.
  - FFP status.
  - Wage budget usage.
  - Profit/loss trend.
  - Transfer spending discipline.

- Squad building score
  - Squad size.
  - Age profile vs vision.
  - Homegrown/youth usage vs vision.
  - Ability gap vs league expectation.
  - Injury crisis should soften blame.

- Strategy score
  - Tactical style fit.
  - Transfer policy fit.
  - Promise fulfilment.
  - Facility/academy progress.

Store the latest component scores on `ClubBoard` so UI and tests can inspect why the board is happy or angry.

### 4. Create Board Decisions

Add a `BoardDecision` enum and expose decisions through `BoardResult`:

- `IssueManagerBacking`
- `IssueFormalWarning`
- `HoldCrisisMeeting`
- `SackManager`
- `IncreaseTransferBudget { amount, reason }`
- `CutTransferBudget { amount, reason }`
- `AdjustWageBudget { amount, reason }`
- `ApproveFacilityUpgrade { facility, cost }`
- `RejectFacilityUpgrade { facility, reason }`
- `DemandPlayerSale { reason }`
- `BlockTransfer { player_id, reason }`
- `ApproveTransferException { player_id, reason }`
- `StartTakeoverRumour`
- `CompleteTakeover`

Keep backward-compatible booleans in `BoardResult` initially if other code uses them, but drive them from the new decision list.

### 5. Make Transfer Governance Realistic

Extend `BoardTransferProposal` with:

- wage impact
- agent fee
- contract length
- resale projection
- personality/professionalism risk if available
- homegrown/domestic fit
- injury risk
- commercial value
- manager priority

Update `review_transfer_proposal`:

- Conservative/austerity boards veto deals that push wage usage above target.
- Ambitious/reckless boards allow exceptions for elite players or critical gaps.
- Youth-focused boards prefer high potential, age <= 23, resale value.
- Private-equity style owners prefer resale and wage control.
- State-backed owners tolerate negative short-term cash but expect trophies.
- Member-owned/local boards punish unpopular sales and reckless debt.

Add tests for each ownership archetype.

### 6. Facilities and Infrastructure

Add a yearly facility review:

- Trigger at season start or season end.
- Consider balance, annual profit, FFP, facilities level, academy output, board strategy.
- Produce facility decisions:
  - training ground upgrade
  - youth facility upgrade
  - academy recruitment upgrade
  - stadium expansion if attendance ratio is high
  - reject because of debt/FFP/low priority

Apply decisions in `BoardResult::process` by modifying `club.facilities` and `club.finance.balance` with clear cost formulas.

### 7. Manager Relationship

Replace simple `manager_loyalty` drift with relationship factors:

- `trust_results`
- `trust_finances`
- `trust_squad_building`
- `trust_communication`
- `style_alignment`

Use these to determine:

- board meetings
- promises
- renewals
- sacking threshold
- transfer autonomy
- whether director of football overrides manager choices

Sack only when the combination of results, timing, pressure, and patience justifies it. Keep early-season protection unless the club is in extreme crisis.

### 8. Supporters and Media

Add supporter/media pressure as board inputs. Do not build a whole UI yet; just model the numbers.

Important events:

- derby win/loss
- relegation zone
- promotion race
- selling a fan favourite
- signing a star
- long winless run
- humiliating cup exit
- youth prospect breakthrough

Board pressure should influence confidence and meetings, but not fully control decisions. Rich reckless owners may ignore supporters; member-owned clubs should react strongly.

### 9. Takeovers and Ownership Changes

Add rare ownership events:

- takeover rumour when finances are poor, exit pressure is high, or club reputation rises rapidly.
- takeover completion changes ownership model, wealth, ambition, patience, strategy.
- failed takeover creates instability and short-term budget freeze.

Keep this deterministic enough for tests. Use seeded/random utilities already present in the repo if randomness is needed.

### 10. Tests Required

Add unit tests for:

- Early season bad form does not sack manager.
- Run-in underperformance can trigger crisis/sacking.
- FFP breach cuts budgets and increases financial pressure.
- Reckless owner increases budgets but lowers patience.
- Conservative owner blocks wage-heavy transfer.
- Youth-focused board accepts weaker young player but blocks old depth signing.
- Facility upgrade happens only when finances and strategy support it.
- Manager renewal is offered after sustained high trust.
- Takeover changes board personality and resets strategy.

Also run:

```bash
cargo test -p core club::board
```

If changes touch transfer pipeline:

```bash
cargo test -p core transfers::pipeline
```

If changes touch country/simulator result processing:

```bash
cargo test -p core country::result
```

## Acceptance Criteria

- Board behavior is explainable through stored component scores and decision reasons.
- New board fields are used by real calculations.
- Budgets and decisions are applied through `BoardResult::process`, not left as inert state.
- No broad refactor of unrelated staff/player modules.
- Existing board and manager-market tests pass.
- New tests cover at least four distinct board archetypes.
- The simulation can generate different board stories for different clubs without hard-coded club names.

