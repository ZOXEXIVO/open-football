# LLM Prompt: Fix and Improve Player Match Rating Realism

You are working in the `open-football` Rust codebase. The current player match rating model produces unrealistic season averages. Example bug: a young prospect has 9 appearances, 2 goals, and an average rating of 8.2. That is too high for real-life football unless the player is delivering repeated elite performances. Fix the rating model so match and season ratings look realistic across positions, minutes, ability, and sample size.

## Files to inspect first

- `src/core/src/match/engine/rating.rs`
- `src/core/src/match/engine/result.rs`
- `src/core/src/match/engine/engine.rs`
- `src/core/src/league/result/match_events.rs`
- `src/core/src/club/player/events/match_play.rs`
- `src/core/src/club/player/statistics/types.rs`
- `src/core/src/club/player/development/modifiers.rs`
- Existing tests in `src/core/src/match/engine/rating.rs`

## Current behavior summary

`rating.rs` computes raw match rating from `PlayerMatchEndStats`:

```text
rating = 6.0
  + sum(position_weight * component)
  + result / clean sheet / conceded / discipline / error context
  clamped to 1.0..10.0
```

The match engine calls:

```rust
RatingContext::new(&stats, player_team_goals, opponent_goals).calculate()
```

Then `match_events.rs` converts raw `stats.match_rating` to `effective_rating` by applying settlement, chemistry, consistency variance, temperament, and important-match modifiers. Finally, `match_play.rs` stores `PlayerStatistics.average_rating` as a plain per-appearance arithmetic average:

```rust
(prev * (games - 1) + o.effective_rating) / games
```

This means a small sample can stay inflated, because there is no minutes weighting, no sample-size regression, no competition/ability expectation, and no cap on repeated elite outcomes for low-volume production.

## Main problems to solve

1. Goals and assists are too powerful for sparse samples.
   - Current single-goal scoring component is about `sat(1, 1.6) * 2.6 = 1.21`, plus clinical and decisive bonuses, plus shooting and result.
   - A two-goal match can easily hit 8+.
   - That is okay for one match, but 2 goals over 9 games should not support 8.2 average by itself.

2. Season average is simple appearance-weighted, not minutes-weighted.
   - A 10-minute goal cameo counts the same as a 90-minute full match in season average.
   - This is not realistic.

3. Exceptional events bypass minute confidence entirely.
   - In `rating.rs`, if `stats.goals > 0`, all weighted impact bypasses minute damping.
   - This is too broad: a late goal should keep goal credit, but unrelated routine actions should still be minute-damped.

4. Rating has no expectation baseline.
   - A low-ability young player can receive elite ratings if simulated event counters are favorable.
   - Realistic rating should be primarily event-based, but adjusted toward a reasonable expected band based on ability, role, minutes, league/club context, and sample size.

5. Positive components stack too easily.
   - Goal + xG + shots on target + key passes + progression + result can all add in the same match.
   - Saturation exists per component, but not across the whole positive attacking contribution.

6. Personality variance can inflate the stored average.
   - `match_events.rs` can add up to `+0.6` for low consistency and `+0.15` for important matches.
   - Variance is deterministic by date and player, but stored season average has no regression toward neutral.

## Design target

Keep the rating useful and expressive, but make distribution realistic:

- Typical starter: 6.3 to 6.9
- Good match: 7.0 to 7.4
- Very good match: 7.5 to 7.9
- Elite match: 8.0 to 8.5, uncommon
- 9.0+: rare, requires multi-goal or decisive all-around performance
- Season average over 8.0: very rare and should require high minutes plus repeated elite output
- Young prospect with 9 apps and 2 goals: normally around 6.7 to 7.3, maybe 7.4 to 7.6 if the matches were genuinely strong; not 8.2

## Required implementation approach

Make a focused change. Do not rewrite unrelated match engine code. Prefer small helpers and explicit constants in or near `rating.rs`, plus necessary season-average changes in `match_play.rs` or `PlayerStatistics`.

### A. Split raw event impact into normal and exceptional parts

Current logic:

```rust
let confidence_applied = if self.exceptional {
    weighted_impact
} else {
    weighted_impact * self.confidence
};
```

Replace this with narrower handling:

- Always apply minute confidence to routine components.
- Give direct goal/error/red-card/own-goal event deltas their own minute policy.
- For goals, use a partial cameo uplift rather than bypassing all damping:

```text
event_minutes_factor(minutes) = 0.70 + 0.30 * minute_confidence(minutes)
```

This lets a 5-minute winner rate high, but prevents a cameo from carrying full credit for all small components.

### B. Retune component coefficients

Use these as initial coefficients:

```text
BASE_RATING = 6.00
RATING_MIN = 1.00
RATING_MAX = 10.00

single_goal_base = sat(goals, 1.7) * 2.05
clinical_goal_bonus = sat(max(goals - xg, 0), 1.0) * 0.15
decisive_goal_bonus = 0.08 if team won else 0.0

assist_component = sat(assists, 1.6) * 1.10
key_pass_component = sat(key_passes, 3.5) * 0.42
box_entry_component = sat(passes_into_box + carries_into_box, 5.0) * 0.30
cross_credit = sat(crosses_completed, 3.5) * 0.16
cross_penalty = sat(failed_crosses, 5.0) * 0.18
xg_buildup = sat(xg_buildup, 1.2) * 0.28
lane_creation = sat(half_space_passes_into_box + central_passes_into_box + switches_of_play, 7.0) * 0.18
final_third_progression_creation = sat(progressive_passes_into_final_third + progressive_carries_into_final_third, 7.0) * 0.12

xg_value = sat(xg, 1.8) * 0.28
shots_on_target_value = sat(shots_on_target, 2.5) * 0.22
wasted_xg_penalty = if goals == 0 && xg > 0.6 then sat(xg - 0.6, 1.2) * -0.55
shot_accuracy = signed_sat(accuracy - 0.40, 0.30) * 0.08
shot_spam_penalty = if shots_total >= 5 && xg_per_shot < 0.08 then sat(shots_total - 4, 4.0) * -0.35

progressive_passes = sat(progressive_passes, 6.0) * 0.32
progressive_carries = sat(progressive_carries, 5.0) * 0.28
carry_distance = sat(carry_distance / 1000.0, 1.8) * 0.14
successful_dribbles = sat(successful_dribbles, 3.5) * role_dribble_weight
role_dribble_weight: forward/midfielder 0.30, defender/goalkeeper 0.18
failed_dribble_penalty: forward 0.18, others 0.24

retention_baseline_pct = 0.74
retention = signed_sat(pass_pct - 0.74, 0.18) * sat(passes_attempted, 45.0) * 0.46
```

Defensive and goalkeeper coefficients can remain closer to current values, but reduce positive stacking slightly:

```text
tackles = sat(effective_tackles, 4.5) * 0.48
interceptions = sat(interceptions, 4.5) * 0.48
blocks = sat(blocks, 2.8) * 0.38
clearances = sat(clearances, 5.5) * 0.28
successful_pressures = sat(successful_pressures, 4.5) * 0.24
raw_pressure_volume = sat(raw_pressures, 10.0) * 0.08
danger_zone_bonus = sat(danger_actions, 5.5) * 0.38
```

Goalkeeper:

```text
saves = sat(saves, 2.8) * 1.35
save_pct_above_baseline max bonus = 0.80
xg_prevented = sat(xg_prevented, 1.5) * 0.90
workload = sat(max(shots_faced - 2, 0), 6.0) * 0.35
quiet_clean_sheet = 0.12
```

### C. Add cross-component positive dampening

After summing all weighted positive components, damp excessive total upside:

```rust
fn compress_positive_delta(delta: f32) -> f32 {
    if delta <= 1.6 {
        delta
    } else {
        1.6 + (delta - 1.6) * 0.55
    }
}
```

Alternative smoother formula:

```rust
fn compress_positive_delta(delta: f32) -> f32 {
    if delta <= 0.0 {
        delta
    } else {
        3.2 * (1.0 - (-delta / 3.2).exp())
    }
}
```

Use this only on positive routine event delta before always-on negative events. Do not compress errors, red cards, own goals, or conceded penalties.

### D. Add realistic match-level soft caps by contribution profile

Avoid hard caps for all players, but prevent anonymous players from drifting into elite ratings.

Use contribution gates:

```text
major_goal_contrib = goals + assists
shot_or_chance_volume = shots_total + key_passes + passes_into_box + successful_dribbles
defensive_volume = tackles + interceptions + blocks + clearances + successful_pressures
gk_volume = saves + command_actions
```

Suggested soft cap after all positive context, before negative events:

```text
if minutes >= 60 and no goals/assists and total meaningful volume low:
  soft cap around 7.1
if minutes < 30 and no goals/assists/errors/red:
  soft cap around 6.7
if one goal only and low all-around volume:
  soft cap around 7.6
if two goals or goal+assist:
  no cap below 8.3
if hat trick:
  no cap below 9.0
```

Implement soft caps as compression, not hard clamp:

```rust
fn soft_cap(value: f32, cap: f32, slope_after: f32) -> f32 {
    if value <= cap {
        value
    } else {
        cap + (value - cap) * slope_after
    }
}
```

Use `slope_after = 0.25` for low-involvement caps and `0.45` for one-goal caps.

### E. Make stored season average minutes-weighted or reliability-adjusted

Do not let a 10-minute cameo count like a full 90. `PlayerStatistics` currently lacks minutes, so choose one:

Option 1, preferred: add rating points and rating weight fields.

```rust
pub rating_points: f32,
pub rating_weight: f32,
```

For each match:

```text
minutes_weight = clamp(minutes_played / 90.0, 0.20, 1.00)
if starter: min weight 0.65
if substitute: min weight 0.20
rating_points += effective_rating * minutes_weight
rating_weight += minutes_weight
average_rating = rating_points / rating_weight
```

Migration/backward compatibility:

- Existing default stats have zero points/weight.
- When merging old stats, if `rating_weight == 0` and `average_rating > 0`, synthesize weight from games.
- Keep display formatting unchanged.

Option 2, smaller change: keep simple average but adjust the input rating before storing:

```text
stored_rating = 6.0 + (effective_rating - 6.0) * minutes_weight
minutes_weight = clamp(minutes_played / 75.0, 0.35, 1.00)
```

This is less correct but avoids schema changes.

### F. Add sample-size regression for displayed/stored season average

For season stats, small samples should regress toward positional neutral:

```text
neutral_by_position:
  GK 6.65
  DEF 6.55
  MID 6.60
  FWD 6.55

reliability_games = 12.0
reliability = effective_full_match_equivalent / (effective_full_match_equivalent + reliability_games)
regressed_avg = neutral + (raw_avg - neutral) * reliability
```

Use this for display-facing `average_rating` or for downstream systems that currently overreact to small samples. If changing stored `average_rating` is too invasive, add a helper like:

```rust
pub fn realistic_average_rating(&self, position: PlayerFieldPositionGroup) -> f32
```

Then update places that rank/select based on rating to use the realistic value where appropriate.

For the reported bug:

```text
raw_avg = 8.20
neutral = 6.55
effective_games = 9.0
reliability = 9 / (9 + 12) = 0.429
regressed = 6.55 + (8.20 - 6.55) * 0.429 = 7.26
```

This is much more realistic for 9 apps and 2 goals.

### G. Keep match awards using raw match rating

Do not over-regress single-match awards. `player_of_the_match` and team/player of the week can still use raw or lightly adjusted match ratings, because they are about individual match output. Season awards, squad selection, development, scouting, and contract logic should use the reliability-adjusted average if they currently consume `statistics.average_rating`.

### H. Tests to add

Add tests in `rating.rs`:

1. `one_goal_low_volume_forward_does_not_exceed_7_7`
   - 90 minutes, 1 goal, 1 shot on target, low passes, no key passes.
   - Expected: `7.0 <= rating <= 7.7`.

2. `two_goals_can_reach_eight_but_not_nine_without_all_round_volume`
   - 90 minutes, 2 goals, 2 shots on target, low creation.
   - Expected: `8.0 <= rating <= 8.7`.

3. `creative_no_goal_forward_can_clear_seven_but_not_elite`
   - Similar existing test, but assert `< 7.8`.

4. `late_goal_cameo_rates_high_but_not_full_match_elite_by_default`
   - 5 minutes, 1 winner.
   - Expected: `7.1 <= rating <= 7.8`.

5. `anonymous_clean_sheet_defender_stays_below_7`
   - 90 minutes, clean sheet, low events.
   - Expected: `< 7.0`.

Add tests around season average in `match_play.rs` or `statistics/types.rs`:

1. `short_cameo_rating_has_lower_average_weight_than_starter`
2. `nine_games_two_goals_regresses_below_elite_average`
   - Simulate nine stored ratings or raw average 8.2.
   - Expected realistic/regressed average around `7.2..7.4`, definitely `< 7.6`.

## Acceptance criteria

- Existing tests pass after updating thresholds that encoded unrealistic behavior.
- A single good event no longer creates repeated 8+ season averages.
- 9 apps and 2 goals should not produce 8.2 average unless the player also has repeated assists, high xG, key passes, or defensive/GK elite actions.
- Short substitute appearances have less effect on season average than 90-minute starts.
- 8.0+ season averages are rare after sample-size regression and require a meaningful sample.
- Rating remains deterministic and cheap to compute.

## Important implementation notes

- Keep `RatingContext::new(&stats, team_goals, opponent_goals).calculate()` API unless a broader refactor is clearly necessary.
- Do not use random runtime variance inside `rating.rs`; deterministic variance already exists in `match_events.rs`.
- Keep negative events strong. Errors to goal, red cards, own goals, and GK failed claims must remain decisive.
- Prefer constants with names over magic numbers.
- Be careful with `PlayerStatistics::merge_from`: any added rating-weight fields must merge correctly.
- Do not break serialization/default construction if this project serializes save data.
- Run relevant tests:

```bash
cargo test -p core rating
cargo test -p core player::statistics
cargo test -p core match_play
```

If package names differ, inspect `Cargo.toml` and run the equivalent focused tests.
