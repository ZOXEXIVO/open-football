# LLM Prompt: Polish the New Player Rating Implementation

You are working in the `open-football` Rust codebase after an initial player-rating realism pass. The implementation now compiles and has focused tests, but it needs a polishing pass before the bug is truly fixed end-to-end.

Original bug: a young prospect with 9 appearances and 2 goals showed an average rating of 8.2. The new implementation added minutes-weighted raw averages and `PlayerStatistics::realistic_average_rating(position)`, but many UI and gameplay systems still read `statistics.average_rating` directly. That means the visible and behavioral bug may still survive even though the helper test passes.

## Files to inspect first

- `src/core/src/match/engine/rating.rs`
- `src/core/src/club/player/statistics/types.rs`
- `src/core/src/club/player/events/match_play.rs`
- `src/web/src/player/get/mod.rs`
- `src/web/src/player/history/mod.rs`
- `src/web/src/teams/stats/mod.rs`
- `src/web/src/teams/get/mod.rs`
- `src/core/src/match/squad/selection/scoring.rs`
- `src/core/src/match/squad/selection/omissions.rs`
- `src/core/src/club/player/development/tick.rs`
- `src/core/src/transfers/pipeline/scouting.rs`
- `src/core/src/transfers/pipeline/helpers.rs`
- `src/core/src/transfers/pipeline/recommendations.rs`
- `src/core/src/country/national/callup.rs`
- `src/core/src/club/player/calculators/calculator.rs`
- `src/core/src/simulator/awards/season.rs`
- `src/core/src/league/awards/season_awards.rs`

Use `rg -n "statistics\\.average_rating|average_rating_str\\(|combined_rating_str\\(|realistic_average_rating" src` to find remaining raw-rating consumers.

## Current implementation status

The current changes appear to do these things correctly:

- `rating.rs` no longer lets any goal bypass all minute damping.
- Routine positive components are damped by minute confidence.
- Goal events use a partial `event_minutes_factor`.
- Positive components are compressed and contribution soft caps were added.
- `PlayerStatistics` now has:

```rust
pub rating_points: f32,
pub rating_weight: f32,
pub fn weighted_average_rating(&self) -> f32
pub fn realistic_average_rating(&self, position: PlayerFieldPositionGroup) -> f32
```

- `Player::record_match_stats` now calls `record_match_rating`.
- Focused tests pass:

```bash
cargo test -p core rating
cargo test -p core statistics::types
cargo test -p core match_play
cargo test -p database --no-run
cargo test -p web --no-run
```

## Main remaining problems

### 1. `realistic_average_rating` is not integrated

Most of the app still reads `statistics.average_rating`, which is now a minutes-weighted raw average, not the regressed realistic value.

Examples found:

- Web player overview: `src/web/src/player/get/mod.rs`
- Player history: `src/web/src/player/history/mod.rs`
- Team stats: `src/web/src/teams/stats/mod.rs`
- Team squad list: `src/web/src/teams/get/mod.rs`
- Squad selection: `src/core/src/match/squad/selection/scoring.rs`
- Development: `src/core/src/club/player/development/tick.rs`
- Scouting/transfer logic: `src/core/src/transfers/pipeline/*.rs`
- National callups: `src/core/src/country/national/callup.rs`
- Contract/value calculators: `src/core/src/club/player/calculators/calculator.rs`
- Season awards: `src/core/src/simulator/awards/season.rs` and `src/core/src/league/awards/season_awards.rs`

If the UI displays `average_rating_str()` or `statistics.average_rating`, the reported player can still show `8.20` after 9 full matches. The helper test only proves `realistic_average_rating()` would return around `7.26`; it does not prove any real screen or decision path uses it.

### 2. Clarify raw vs realistic rating semantics

Decide and document the model:

- `weighted_average_rating()` = raw minutes-weighted form.
- `realistic_average_rating(position)` = display/season/decision average with sample-size regression.
- `match_rating` = single-match raw output, used for player of the match, match events, and weekly awards.

Then apply it consistently:

- Use raw match rating for single-match awards and immediate match reactions.
- Use realistic/regressed average for season display, scouting, development, squad selection, national calls, transfer valuation, contracts, and staff perception.
- If a view intentionally displays raw weighted rating, label/structure it clearly in code so it is not mistaken for the realistic season average.

### 3. `combined_rating_str` still uses game-count arithmetic

`PlayerStatistics::combined_rating_str(&self, other)` still combines:

```rust
self.average_rating * games_a + other.average_rating * games_b
```

This ignores the new rating ledger and cameo weighting. Update it to combine `rating_points/rating_weight`, with legacy fallback via the same synthesized ledger logic used by `merge_from`.

Suggested helper:

```rust
pub fn combined_weighted_average_rating(&self, other: &PlayerStatistics) -> f32
```

Then `combined_rating_str` should call that helper. If the combined rating is used for display and a position is available, add:

```rust
pub fn combined_realistic_average_rating(
    &self,
    other: &PlayerStatistics,
    position: PlayerFieldPositionGroup,
) -> f32
```

### 4. `record_match_rating` should validate input

Current implementation records any `effective_rating` and assigns at least `0.20` weight for substitutes. Add a guard:

```rust
if !(1.0..=10.0).contains(&effective_rating) || minutes_played == 0 {
    return;
}
```

This prevents accidental unused-sub or bad-data paths from contaminating the rating ledger.

### 5. Tests mostly prove helpers, not integration

Add tests that fail if real consumers keep using raw `average_rating`.

At minimum:

- A display/view-model test where a player with 9 starts at raw 8.2 is rendered below 7.6.
- A squad-selection/scouting/development test that verifies the performance factor uses realistic average for small samples.
- A combined-rating test proving cameo-heavy friendly/official stats combine via rating weights, not games.
- A `record_match_rating` test for zero minutes / invalid rating ignored.

Do not just add more helper tests; test at least one real call site.

## Recommended polish steps

### Step A: Add explicit helpers to `PlayerStatistics`

Keep the current helpers, then add:

```rust
pub fn average_rating_raw(&self) -> f32 {
    self.weighted_average_rating()
}

pub fn average_rating_realistic(&self, position: PlayerFieldPositionGroup) -> f32 {
    self.realistic_average_rating(position)
}

pub fn display_average_rating(&self, position: PlayerFieldPositionGroup) -> String {
    Self::format_rating(self.average_rating_realistic(position))
}
```

The naming should make it hard for future code to accidentally use the raw field.

### Step B: Update display code first

The reported bug is visible rating, so prioritize UI display paths:

- `src/web/src/player/get/mod.rs`
- `src/web/src/player/history/mod.rs`
- `src/web/src/teams/stats/mod.rs`
- `src/web/src/teams/get/mod.rs`
- `src/core/src/club/player/core/display.rs`

Use player position where available:

```rust
let avg = player.statistics.realistic_average_rating(player.position().group());
```

Adjust for the actual position API in this codebase.

For historical rows where player position may not be stored, either:

- pass the current player position into row mapping, or
- use a neutral default of midfielder `6.60`, but prefer passing position.

### Step C: Update decision systems selectively

Replace raw `statistics.average_rating` with realistic average in systems that should not overreact to small samples:

- Development multiplier in `src/core/src/club/player/development/tick.rs`
- Squad selection scoring and omissions
- Scouting/recommendation/loan-market helpers
- National callup candidate construction
- Contract/player value calculators
- Staff perception / morale long-form checks
- Team satisfaction based on squad form

Keep raw match ratings for:

- `player_of_the_match`
- match events immediately after one fixture
- player/team of the week
- any code using `PlayerMatchEndStats.match_rating` directly for one-match recognition

### Step D: Revisit season awards

Season awards aggregators often compute ratings from per-match `stats.match_rating` and may be okay if they also enforce meaningful match/minute thresholds. Check:

- `src/core/src/league/awards/season_awards.rs`
- `src/core/src/simulator/awards/season.rs`
- `src/core/src/simulator/awards/team_of_year.rs`

If they allow a small number of high ratings to win season awards, apply:

```text
minimum effective appearances >= 15
or use reliability adjusted aggregate:
regressed = neutral + (raw_avg - neutral) * effective_matches / (effective_matches + 12)
```

Do not regress weekly awards; those are intentionally short-window.

### Step E: Backward compatibility and save data

Check whether `PlayerStatistics` is serialized anywhere. If serde is used on this struct or parent structs, added fields should have defaults:

```rust
#[serde(default)]
pub rating_points: f32,
#[serde(default)]
pub rating_weight: f32,
```

If no serde is used, document that compile checks are enough.

### Step F: Tighten comments and tests

Some comments say `average_rating` is kept in sync for legacy readers. This can be misleading because it is still raw weighted, not realistic. Update docs to say:

```text
average_rating stores the raw minutes-weighted average for compatibility.
Use realistic_average_rating(position) for display and season-level decisions.
```

## Acceptance criteria

- The visible player/team rating screens no longer display raw 8.2 for 9 apps unless a deliberate raw-rating field is shown.
- The reported case (`9 apps, 2 goals, raw 8.2`) displays/evaluates around `7.2..7.4`, definitely below `7.6`.
- Short substitute appearances have less effect than starts in both raw weighted and combined ratings.
- Development/scouting/selection no longer reward a small-sample 8.2 as if it were a proven elite season.
- Single-match awards and match reactions still use raw match rating.
- All focused tests pass:

```bash
cargo test -p core rating
cargo test -p core statistics::types
cargo test -p core match_play
cargo test -p database --no-run
cargo test -p web --no-run
```

Run broader tests if touching awards/scouting/selection:

```bash
cargo test -p core squad::selection
cargo test -p core scouting
cargo test -p core awards
```
