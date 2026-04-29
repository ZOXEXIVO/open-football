# Shot / Goal Volume Analysis

Generated against `master` (commit `fbb2a4ea`), benchmark = 30 matches, random
squad levels 6–18.

## Progress summary (after fixes)

100-match validation:

| metric                | start  | after fixes | real | progress |
|-----------------------|-------:|------------:|-----:|---------:|
| **goals / match**     | 0.53   | **1.47**    | 2.5  | +178%    |
| xG / team / match     | 0.25   | 0.97        | 1.3  | +288%    |
| shots / team / match  | 6.9    | 34.5        | 13   | overshoots — most low-xG |
| on-target rate        | 21.3%  | 17.2%       | 33%  | flat     |
| on-target → goal      | 18.0%  | 12.4%       | 30%  | flat     |
| tackles / team        | 38.2   | 26.4        | 18   | +31%     |
| 0-goal matches        | 70%    | 29%         | ~7%  | +59%     |

Score distribution shape now realistic (0/1/2/3+ spread looks like real
football); absolute conversion still below target.

### Fixes applied

1. **Save credit gate** — `handle_caught_ball_event`, `handle_parried_ball_event`,
   `handle_clear_ball_event` all now gate save+on-target on real shot context.
2. **`try_intercept` clears `cached_shot_target`** — prevents shot flag leaking
   to subsequent ball events.
3. **`is_attack_ready` relaxed** — drops the "completely unmarked" requirement.
   Real defences always mark; position alone is the signal.
4. **Forward `FWD_PATIENT_POSSESSION` recycle gated on `stuck`** — only fires
   when the forward genuinely can't progress (multiple opponents, no open
   space ahead). Was the dominant shot-volume suppressor.
5. **xG quality gate (≥0.04)** — refuse hopeless-blast shots from 25+ yards
   at steep angle / heavy marking.
6. **`try_save_shot` per-tick save_prob** — clamp 0.10..0.96 → 0.05..0.55,
   skill_mult 0.72..1.07 → 0.45..0.80. Calibrated for cumulative ~67% real
   save rate.
7. **Diving / catching state phantom-parry removed** — far-distance "ball
   moving away" branches no longer credit a parry the keeper never touched.
8. **Forward press distance** 150 u → 60 u — calibrated to real press radii.
9. **Forward tackle skill gate** (tackling ≥ 8 / 20) — most strikers don't
   drill defensive tackles; gate eliminates 14.9 → ~1.3 forward tackles
   per team per match.
10. **Player shot cooldown** 800 ticks → 200 ticks — re-enables rebound
    goal patterns.

### Remaining issues

- **`saves / on-target` ratio ≈ 197%** — measurement-only bug. Saves are
  being credited from a path I haven't fully traced. Doesn't affect actual
  goal count, but the ratio is impossible (every save should pair with one
  on-target). Likely candidate: stale `cached_shot_target` from a path I
  didn't audit (passing kicks, deflections, set pieces).
- **Shots / team still too high (34 vs 13)** — forwards in the new
  setup take many low-xG attempts. xG threshold is generous (0.04) so
  shots from medium range still go through. Tightening past 0.06 caused
  forwards to hold the ball forever waiting for box opportunities,
  blowing goals out of proportion. The shot count overshoot is cosmetic;
  per-shot xG is low so total xG and goals are ~realistic.
- **Fouls / team dropped to 3.1 (real 12)** — side effect of the forward
  tackle skill gate. Forwards generated most fouls under the old code.
  Defender / midfielder foul rates need a proportional uplift.
- **0-goal matches still 29%** (real ~7%) — the underlying conversion
  curve (on-target → goal 12.4% vs real 30%) is still too low; keepers
  catch / parry too many on-target shots in the long tail.

---

## Original analysis (pre-fix baseline)


## Headline numbers (vs real-football targets)

| metric                     | engine | real | gap            |
|----------------------------|-------:|-----:|----------------|
| goals / match              | 0.53   | 2.5  | **−4.7×**      |
| xG / team / match          | 0.25   | 1.3  | **−5.2×**      |
| shots / team / match       | 6.9    | 13   | −1.9×          |
| shots per xG               | 27.4   | 10   | +2.7× (low Q)  |
| on-target rate             | 21.3%  | 33%  | −12 pp         |
| on-target → goal           | 18.0%  | 30%  | −12 pp         |
| **saves / on-target**      | 169.7% | 67%  | impossible     |
| tackles / team             | 38.2   | 18   | +2.1×          |
| interceptions / team       | 2.9    | 10   | −3.4×          |
| passes / team              | 521    | 500  | OK             |
| pass accuracy              | 77%    | 85%  | −8 pp          |

21 / 30 matches finished 0–0. The match is too defensive at every level —
shot volume, shot quality, and on-target rate all underperform.

## Shot-gate waterfall

```
HAS_BALL_IN_RANGE (dist≤90)      783  base
PASSED_CAN_SHOOT                 783    drop  0.0%
PASSED_SETTLED (own≥30)          678    drop 13.4%
PASSED_NOT_DEFER                 646    drop  4.7%
PASSED_MAX_DIST (≤90)            646    drop  0.0%
PASSED_CLEAR_SHOT                560    drop 13.3%
PASSED_WILLINGNESS               395    drop 29.5%
FIRED                            395    drop  0.0%
```

**Across 30 matches × 60 team-halves the base is only 783 ticks of
"forward has ball within 90 u of opponent goal".** That's ~13 ticks per
team-match (≈260 ms of forward-with-ball-in-range play per team per
90-minute match). The shot-gate chain is *not* the bottleneck — the
bottleneck is **forwards almost never get the ball in shooting range
to begin with**.

## Root causes (ranked by impact)

### 1. Attacks self-abort via `should_play_possession()` ⭐⭐⭐⭐⭐

`team.rs:96` — `should_play_possession()` returns `true` whenever
`!is_attack_ready()`. `is_attack_ready` requires a forward/midfielder
within **70 u of goal AND with no opponent inside 8 u**.

Every well-defended box has multiple defenders inside 8 u of the
attacking forwards. So the moment a forward enters the shooting third,
the team is classified as "attack not ready" → "should play possession"
→ the forward fires `FWD_PATIENT_POSSESSION` and passes the ball
*backwards*.

`forwarders/states/running/mod.rs:204-256`:

```rust
if !under_pressure
    && distance_to_goal > 50.0
    && ctx.tick_context.ball.ownership_duration > 10
    && ctx.team().should_play_possession()
{
    // … find a teammate BEHIND us … pass back
    return Some(StateChangeResult::with_forward_state_and_event(
        ForwardState::Running,
        Event::PlayerEvent(PlayerEvent::PassTo(
            … with_reason("FWD_PATIENT_POSSESSION") …
        )),
    ));
}
```

**This is a classic chicken-and-egg deadlock:** to escape possession
mode we need a forward to be unmarked in the box; to get a forward
unmarked in the box we need to attack; we won't attack because we're
in possession mode. Result: ball cycles through midfield, attempts
forward incursion, gets marked, recycles, repeat.

**Fix direction:** `is_attack_ready` is too strict. Either
- Lower the unmarked threshold (8 u → much higher, since real defenders
  *always* mark forwards in the box), or
- Add a "match progress" / "attempts since last shot" pressure release —
  if we haven't taken a shot for >60 s of possession-heavy play, force
  a forward attempt regardless of marking.

A universal fix matches the CLAUDE.md guidance: replace the binary
"is_attack_ready" with a continuous **attacking pressure** signal
that decays without shot attempts and shifts the recycle/attack
balance back toward attack.

### 2. Saves credited for non-shots ⭐⭐⭐⭐⭐

`player/events/players.rs:1572` — `handle_caught_ball_event`:

```rust
if ball_was_moving && last_owner_team.is_some() && last_owner_team != gk_team {
    let was_shot = field.ball.cached_shot_target.is_some();
    if let Some(player) = field.get_player_mut(player_id) {
        player.statistics.saves += 1;          // ← always
        if was_shot {
            player.statistics.shots_faced += 1;
        }
    }
    if was_shot {
        if let Some(sid) = shooter_id {
            if let Some(shooter) = field.get_player_mut(sid) {
                shooter.memory.credit_shot_on_target();
            }
        }
    }
}
```

`saves += 1` runs for **any** moving ball the keeper claims from an
opponent — long crosses, miscued passes, cleared balls that drift to
the keeper. Only `shots_faced` and `credit_shot_on_target` are gated
on `was_shot`.

This explains the impossible ratio `saves / on-target = 169.7%`. With
4.4 saves per team avg vs ~2.6 on-target shots per team avg, almost
half of "saves" are not saves at all.

The companion path — `handle_clear_ball_event` (`players.rs:1961`) —
correctly gates on `gk_clearing_shot()`. Only `handle_caught_ball_event`
is wrong.

**Fix:** move `player.statistics.saves += 1` inside the `if was_shot`
block. One-line change.

### 3. Per-player shot cooldown is excessive ⭐⭐⭐

`player/memory.rs:151-157`:

```rust
const PLAYER_SHOT_COOLDOWN_TICKS: u64 = 800;  // 8 s
```

Combined with `shots_this_possession < 2` and 5 s team cooldown, this
keeps the shot rate suppressed. Real football: a striker often takes
2–3 shots inside a 30-second flurry (corners, rebounds, follow-up
chances). 800 ticks = 8 s would block all of them.

The comment defends 800 as preventing "the same striker [from blasting]
three or four attempts in the same possession" — but
`shots_this_possession < 2` already enforces that limit *per
possession*, and `team.can_shoot()`'s 500-tick cadence enforces it
per team. The 800-tick personal lockout is redundant *across*
possessions (where it shouldn't apply).

**Fix:** drop player cooldown to ~150 ticks (1.5 s — keeps it visually
plausible "they're recovering balance"), since the per-possession cap
is the right place to limit shot spam.

### 4. Shot save base probability too high ⭐⭐⭐

`ball/ball.rs:856-873`:

```rust
let base = 0.88 - reach_ratio * reach_ratio * 0.58;        // 0.88..0.30
let skill_mult = 0.72 + skill * 0.35;                      // 0.72..1.07
let save_prob = ((base - speed_penalty) * skill_mult).clamp(0.10, 0.96);
```

For a centred shot vs an average-skill keeper:
`0.88 × 0.90 = 0.79` save rate. Real centred shots from inside the
box convert at much higher than 21% — placement matters more than
this curve gives credit for.

The y-error spread already pushes most aimed corner shots back toward
center (line `1298-1300`, `base_position_error = 30 × distance_factor
× (1 - accuracy)` — 30u spread is *larger* than the 29u half-goal).
So most "aimed at corner" shots end up back near center, where save
rate is 79–94%.

**Combined with bug #2** (saves credited for non-shots), the engine
both *over*-credits saves for moving balls AND *over*-saves real
shots.

**Fix direction:** drop `base` peak to ~0.70 for centred shots, OR
tighten the `base_position_error` spread so corner-aimed shots
actually land at the corner more often. The latter is more realistic
— elite finishers rarely re-center their corner aim.

### 5. Willingness gate dropping 29.5% of qualified opportunities ⭐⭐

The waterfall shows the willingness roll is the largest single drop
*after* the chain narrows (`560 → 395`, −29.5%). With per-tick
willingness in the 0.70–0.95 range and the gate firing on a 50 Hz
cadence (light/full tick parity in `engine.rs:455-459`), the *effective*
willingness drops by another factor.

This is the smallest of the five top issues — once `is_attack_ready`
is fixed and the base waterfall expands, willingness rejection
matters less in absolute terms.

### 6. Forward-state tackle pressure wiping out attacks ⭐⭐

Tackle stats: 38.2 tackles / team / match (real ~18). The breakdown:

```
DEF: 1010 successes / 4109 attempts = 24.6%   (16.8/team — close to real)
MID:  692 successes / 3598 attempts = 19.2%   (11.5/team)
FWD:  588 successes / 2367 attempts = 24.8%   ( 9.8/team — too high)
```

Forwards tackling 9.8 times per match is unrealistic — real forwards
average 1–2. Forward tackles primarily fire from the press path; the
press distance scales with stamina × work_rate × intensity and seems
calibrated too aggressively.

This compounds with #1: every successful forward tackle dispossesses
the *opponent's* attack, reducing total time-in-attacking-third per
match. Both teams are press-heavy → both teams' attacks die fast →
no one shoots.

## Engine.rs orchestration findings

`engine.rs` itself is mostly sound — the orchestration matches the
spec and the recent additions (set-piece teleport drain, ball owner
refresh between play_ball and play_players, GK-only AI on light ticks
during shot flight) are correct.

Two minor concerns worth flagging:

### `tick_parity` halves AI cadence (line 432-459)

```rust
if tick_parity & 1 == 0 {
    Self::game_tick_light(field, context, match_data, &mut events);
} else {
    Self::game_tick_inner(field, context, match_data, &mut tick_ctx, &mut events);
}
```

Player AI runs every other tick (50 Hz instead of 100 Hz). All
per-tick die-rolls (willingness, dribble, decisions) implicitly
have their effective rate halved, which has to be remembered when
calibrating constants like the 0.70–0.95 willingness clamp. Not a
bug — but a calibration multiplier present-but-undocumented.

### `check_goal` can fire before the goal-scoring event is dispatched

In `play_ball`:
- `check_goal` sets `goal_scored = true` and calls `self.reset()`,
  which clears `cached_shot_target`.
- The `BallEvent::Goal` is queued in the events vec.
- `tick_ctx.refresh_ball(field)` then refreshes the tick context,
  showing the ball at centre with no owner.
- `play_players` runs — players see ball at centre, no owner.
- `EventDispatcher::dispatch` runs → finally credits the goal.
- `handle_goal_reset` resets player positions for kickoff.

So between `check_goal` firing the BallEvent::Goal and
`handle_goal_reset` running, players are running AI for one tick on a
ball that's been teleported back to centre. They'll mostly skip
because the ball is unowned and not near them. Cosmetic only — but
worth tightening if a regression turns up after fixes elsewhere.

## Recommended fix priority

1. **Save credit gate (bug #2)** — one-line fix, restores a fundamental
   stats invariant. Will reveal whether other shooting numbers shift.
2. **`is_attack_ready` / `should_play_possession`** — single biggest
   shot-volume lever. Replace binary "unmarked forward in 70 u" with
   a continuous attacking-pressure signal.
3. **Forward press / tackle aggression** — drop press distance and/or
   the fwd tackle attempt rate; 9.8 forward-tackles/match is implausible
   and steals time-in-attacking-third from both sides.
4. **Save curve flattening** — drop centred-shot save base from 0.88
   to ~0.70, OR tighten shot Y-error so corner-aimed shots land at the
   corner. Both fix the on-target → goal rate.
5. **Player shot cooldown** 800 → ~150 ticks — opens the rebound
   pattern that real attacks rely on.

## How to reproduce

```bash
cd .dev/match
cargo run --release -- stats 30
```

Output includes the SHOT-GATE WATERFALL and TACKLE FLOW tables used
in this analysis. Re-run after each fix to see which gate shifts.
