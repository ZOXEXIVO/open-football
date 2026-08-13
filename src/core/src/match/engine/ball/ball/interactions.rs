//! Ball-vs-defender interactions during in-flight passes and shots:
//! interception, shot-block, and goalkeeper save. Each runs only on
//! unowned balls with `in_flight_state > 0` so routine possession
//! play isn't disturbed.

use super::{AerialReach, Ball};
use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::player::events::players::save_accounting_stats;
use crate::r#match::events::EventCollection;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

/// The physics-layer shot-stopping curve.
///
/// Kept on a struct rather than inline in [`Ball::try_save_shot`] so the
/// live path and the spread regression test read the SAME numbers. The
/// inline version was flattened to a 4.8%-wide skill band at one point
/// and nothing caught it: no test pinned the slope, and equal-level
/// harness runs can't see it (both keepers are equally good, so the
/// population save% is unchanged whatever the slope is). The gap only
/// shows when quality differs — which is the normal case on the live
/// site, and the reason youth keepers were performing like
/// internationals.
/// Why shot blocks don't happen. `blocks` reads ~0.01 per defender per
/// match against a real ~0.9, and the counter alone cannot say whether
/// the shot never reaches the check, no defender is ever in the lane, or
/// the roll simply fails. `match-logs` only.
#[cfg(feature = "match-logs")]
pub mod block_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// `try_block_shot` reached with a live shot in flight.
    pub static SHOTS_SEEN: AtomicU64 = AtomicU64::new(0);
    /// Rejected because the ball was above blocking height.
    pub static TOO_HIGH: AtomicU64 = AtomicU64::new(0);
    /// A defender was found inside the lane.
    pub static CANDIDATES: AtomicU64 = AtomicU64::new(0);
    /// The roll succeeded.
    pub static FIRED: AtomicU64 = AtomicU64::new(0);

    // ── Per-opponent rejection lanes ────────────────────────────────
    //
    // `CANDIDATES` alone says "no defender in the lane" without saying
    // WHY, and the three possible causes want opposite fixes: defenders
    // standing behind the ball is a positioning problem, defenders past
    // the lookahead is a window problem, defenders goal-side but wide is
    // a corridor-width problem. These split the rejection so the next
    // reader doesn't have to re-derive it.
    /// Opposition outfielders examined across all shot-ticks.
    pub static OPP_SEEN: AtomicU64 = AtomicU64::new(0);
    /// Rejected: level with or behind the ball along the shot line.
    pub static BEHIND_BALL: AtomicU64 = AtomicU64::new(0);
    /// Rejected: goal-side but further than `BLOCK_LOOKAHEAD` ahead.
    pub static BEYOND_LOOKAHEAD: AtomicU64 = AtomicU64::new(0);
    /// Rejected: inside the lookahead window but wider than the corridor.
    pub static OUTSIDE_CORRIDOR: AtomicU64 = AtomicU64::new(0);
    /// Sum of perpendicular distances for opponents inside the lookahead
    /// window, x100 — divided by `IN_WINDOW` it gives the mean miss
    /// distance, which is what says whether the corridor is merely too
    /// narrow or the defenders are nowhere near the line.
    pub static PERP_SUM_X100: AtomicU64 = AtomicU64::new(0);
    /// Opponents inside the lookahead window (the `PERP_SUM_X100` denom).
    pub static IN_WINDOW: AtomicU64 = AtomicU64::new(0);

    // ── At the moment of the strike ─────────────────────────────────
    //
    // The per-tick counters above sample the whole flight, which biases
    // "behind the ball" upward: a defender the ball has already passed
    // counts as behind on every remaining tick. These sample ONCE, when
    // the shot is struck, and answer the football question directly —
    // was anybody between the shooter and the goal at all?
    /// Shots struck with a projected target (one sample each).
    pub static SHOTS_STRUCK: AtomicU64 = AtomicU64::new(0);
    /// Opposition outfielders goal-side of the ball at the strike,
    /// summed over `SHOTS_STRUCK`.
    pub static GOALSIDE_AT_STRIKE: AtomicU64 = AtomicU64::new(0);
    /// Of those, the ones also within 30u of the ball's line to goal —
    /// i.e. actually in a position to get a body in the way.
    pub static GOALSIDE_NEAR_LINE: AtomicU64 = AtomicU64::new(0);

    /// Distance from the ball to the goal it is aimed at, x100, summed
    /// over `SHOTS_STRUCK`. Says where shots are actually taken from.
    pub static SHOT_RANGE_X100: AtomicU64 = AtomicU64::new(0);
    /// Mean distance of the DEFENDING outfielders from their own goal
    /// line at the strike, x100, summed over `SHOTS_STRUCK`. Read against
    /// `SHOT_RANGE_X100`: if the defenders sit further out than the ball,
    /// the line never dropped; if they sit closer but nobody is in the
    /// lane, the line dropped and scattered.
    pub static DEF_DEPTH_X100: AtomicU64 = AtomicU64::new(0);

    /// Histogram of which `DefenderState` the defending back line is in
    /// at the moment a shot is struck, indexed by the enum's discriminant
    /// (21 variants). Without this the depth number says the line did not
    /// drop but not WHY — and the answer decides whether the fix belongs
    /// in a state's steering target or in the state selection above it.
    pub static DEF_STATE_AT_STRIKE: [AtomicU64; 21] = [const { AtomicU64::new(0) }; 21];

    /// Diagnostic accessors. Grouped on a struct so the module exposes
    /// no free functions — the statics stay module-level because Rust
    /// has no associated statics.
    pub struct BlockDiag;

    impl BlockDiag {
        /// Book one back-line defender's state at a strike.
        pub fn note_defender_state(state_id: usize) {
            if let Some(c) = DEF_STATE_AT_STRIKE.get(state_id) {
                c.fetch_add(1, Ordering::Relaxed);
            }
        }

        /// Per-state counts, in discriminant order.
        pub fn defender_state_snapshot() -> [u64; 21] {
            std::array::from_fn(|i| DEF_STATE_AT_STRIKE[i].load(Ordering::Relaxed))
        }

        /// Sample the defensive picture at the moment a shot is struck.
        /// `goalside` / `near_line` are counts for this one strike;
        /// `shot_range` / `def_depth` are distances to the defended goal.
        pub fn note_strike(goalside: u64, near_line: u64, shot_range: f32, def_depth: f32) {
            SHOTS_STRUCK.fetch_add(1, Ordering::Relaxed);
            GOALSIDE_AT_STRIKE.fetch_add(goalside, Ordering::Relaxed);
            GOALSIDE_NEAR_LINE.fetch_add(near_line, Ordering::Relaxed);
            SHOT_RANGE_X100.fetch_add((shot_range.max(0.0) * 100.0) as u64, Ordering::Relaxed);
            DEF_DEPTH_X100.fetch_add((def_depth.max(0.0) * 100.0) as u64, Ordering::Relaxed);
        }

        /// `(shots_struck, goalside_per_shot, near_line_per_shot,
        ///   mean_shot_range, mean_defender_depth)`
        pub fn strike_snapshot() -> (u64, f32, f32, f32, f32) {
            let n = SHOTS_STRUCK.load(Ordering::Relaxed);
            if n == 0 {
                return (0, 0.0, 0.0, 0.0, 0.0);
            }
            let per = |c: &AtomicU64| c.load(Ordering::Relaxed) as f32 / 100.0 / n as f32;
            (
                n,
                GOALSIDE_AT_STRIKE.load(Ordering::Relaxed) as f32 / n as f32,
                GOALSIDE_NEAR_LINE.load(Ordering::Relaxed) as f32 / n as f32,
                per(&SHOT_RANGE_X100),
                per(&DEF_DEPTH_X100),
            )
        }

        pub fn reset() {
            for c in [
                &SHOTS_SEEN,
                &TOO_HIGH,
                &CANDIDATES,
                &FIRED,
                &OPP_SEEN,
                &BEHIND_BALL,
                &BEYOND_LOOKAHEAD,
                &OUTSIDE_CORRIDOR,
                &PERP_SUM_X100,
                &IN_WINDOW,
                &SHOTS_STRUCK,
                &GOALSIDE_AT_STRIKE,
                &GOALSIDE_NEAR_LINE,
                &SHOT_RANGE_X100,
                &DEF_DEPTH_X100,
            ] {
                c.store(0, Ordering::Relaxed);
            }
            for c in &DEF_STATE_AT_STRIKE {
                c.store(0, Ordering::Relaxed);
            }
        }

        /// `(shots_seen, too_high, candidates, fired)`
        pub fn snapshot() -> (u64, u64, u64, u64) {
            (
                SHOTS_SEEN.load(Ordering::Relaxed),
                TOO_HIGH.load(Ordering::Relaxed),
                CANDIDATES.load(Ordering::Relaxed),
                FIRED.load(Ordering::Relaxed),
            )
        }

        /// `(opp_seen, behind_ball, beyond_lookahead, outside_corridor,
        ///   in_window, mean_perp)`
        pub fn lane_snapshot() -> (u64, u64, u64, u64, u64, f32) {
            let in_window = IN_WINDOW.load(Ordering::Relaxed);
            let mean_perp = if in_window == 0 {
                0.0
            } else {
                PERP_SUM_X100.load(Ordering::Relaxed) as f32 / 100.0 / in_window as f32
            };
            (
                OPP_SEEN.load(Ordering::Relaxed),
                BEHIND_BALL.load(Ordering::Relaxed),
                BEYOND_LOOKAHEAD.load(Ordering::Relaxed),
                OUTSIDE_CORRIDOR.load(Ordering::Relaxed),
                in_window,
                mean_perp,
            )
        }
    }
}

pub(crate) struct SaveModel;

impl SaveModel {
    /// Geometric ceiling for a dead-centre shot. Pure geometry — the
    /// keeper is standing where the ball is going.
    /// Re-anchored 0.88 → 0.76. The old value was calibrated for a
    /// keeper genuinely standing on the ball's line, which was ALWAYS
    /// true while the shot cache handed him the exact crossing point —
    /// so the geometric ceiling applied to essentially every shot and
    /// the population save rate sat at 82% against a real 67%. With the
    /// keeper's committed line now carrying a reading error, the
    /// dead-centre case is rare again and the ceiling can be what it
    /// says it is: the chance for a shot hit straight at him.
    ///
    /// Re-anchored 0.82 → 0.99 (with `STRETCH_PENALTY` 0.58 → 0.42)
    /// 2026-08-13, when shots started reaching the keeper at all. Every
    /// previous setting of this pair was calibrated against a population
    /// in which ~73% of shots were picked out of the air by an outfield
    /// defender within a tick of the strike (see `try_intercept`), so the
    /// keeper only ever faced the ~11% that survived — the short ones,
    /// hit from 73u against 102u for the population. With the full
    /// distribution arriving, the same curve saved 56% of what it faced
    /// against a real ~67%.
    ///
    /// ⚠ The population save rate belongs HERE, in the geometry, and not
    /// in `SKILL_FLOOR`. Lifting the floor instead was tried and is what
    /// `keeper_skill_spread_stays_wide` and
    /// `an_ordinary_duel_holds_the_calibrated_population_save_rate` exist
    /// to catch: the multiplier is a CONTEST pinned at `FLOOR + SLOPE/2`
    /// for an even duel, so raising the floor both breaks level-parity
    /// and squeezes the keeper-quality axis against `MAX_SAVE` — at 0.86
    /// the spread between the worst keeper alive and the best collapsed
    /// to 12.9 points against a real ~20.
    const CENTRED_BASE: f32 = 1.03;
    /// How much of that ceiling a full-stretch shot gives away.
    const STRETCH_PENALTY: f32 = 0.42;
    /// Save probability for the worst keeper alive on a centred shot,
    /// before geometry: `SKILL_FLOOR`. Real weak top-flight keepers save
    /// ~58% of what they face across a season; elite ones ~78%.
    /// Re-anchored 0.54 → 0.57 when the multiplier became a contest.
    /// Under the old absolute model the realised multiplier was
    /// `0.54 + mean_skill·SLOPE`, which at the mid-high levels the
    /// goals-per-match calibration was built on averaged ~0.72; the
    /// contest instead pins every ordinary duel at
    /// `FLOOR + SLOPE/2` at EVERY level, so leaving the floor alone
    /// silently moved the population save rate down and pushed
    /// goals/match from ~2.4 to ~2.8. The floor now carries the level
    /// that the skill term used to supply.
    ///
    /// NOT the lever for population goals/match, despite carrying the
    /// population level for save RATE. Measured 2026-08-08: dropping it
    /// 0.57 → 0.54 moved neither goals (2.28 → 2.22, inside noise) nor
    /// save% (68.5% → 68.9%). Roughly half of all credited saves come
    /// from the GK state machine rather than this physics roll (`SAVE
    /// PIPELINE`: 725 of 1482), so a 4% relative cut here is ~2%
    /// overall — below the run-to-run floor. Reach for shot volume or
    /// the willingness roll instead.
    const SKILL_FLOOR: f32 = 0.57;
    /// Width of the keeper-quality band. Mean skill (0.5) lands on
    /// 0.68 — the multiplier the ~67% population save rate is
    /// calibrated on — so restoring the spread is calibration-neutral
    /// at the population mean while the tails move where they should.
    const SKILL_SLOPE: f32 = 0.28;
    const MIN_SAVE: f32 = 0.08;
    const MAX_SAVE: f32 = 0.92;

    /// Geometric save chance by how far the keeper has to stretch
    /// (0 = shot straight at him, 1 = at the limit of his reach).
    #[inline]
    pub(crate) fn geometric_base(reach_ratio: f32) -> f32 {
        let r = reach_ratio.clamp(0.0, 1.0);
        Self::CENTRED_BASE - r * r * Self::STRETCH_PENALTY
    }

    /// Threat value standing in for "an ordinary shooter" on the paths
    /// that build a shot target without a striker behind it (tests,
    /// synthesised targets).
    ///
    /// Not 0.5: the two composites do not share a population mean, and
    /// an *ordinary* striker measures [`Self::CONTEST_BALANCE`] above an
    /// ordinary keeper. Feeding that here is what makes a mid-skill
    /// keeper facing a mid-skill striker resolve to the calibrated 0.68.
    pub(crate) const NEUTRAL_THREAT: f32 = 0.5 + Self::CONTEST_BALANCE;

    /// How far a quality mismatch can swing the duel. At 1.0 a keeper a
    /// full point of composite better than the striker would pin the
    /// multiplier at its ceiling; 1.30 makes the realistic ±0.25 spread
    /// within a division cover most of the band while keeping the
    /// extremes reachable only by genuine mismatches.
    const CONTEST_SPREAD: f32 = 1.30;

    /// Constant offset between the two composites' population means, so
    /// that an *ordinary* duel resolves to 0.5 rather than to whatever
    /// the two blends happen to average.
    ///
    /// `gk_shot_stopping` and `shot_threat` read different attributes,
    /// and the generator does not hand those attributes the same
    /// population mean — measured over generated squads (`dev_match
    /// audit_contest`), `shot_threat` runs ~0.11 above `gk_shot_stopping`
    /// for forwards, ~0.03 for midfielders and ~0.01 for defenders.
    /// Shot-weighted across the lines that actually shoot, that lands
    /// near 0.08. Without the correction every duel in the game was
    /// biased toward the shooter and goals/match jumped 2.3 → 3.0.
    ///
    /// What makes the contest work is not this constant but the fact
    /// that the offset is FLAT: the same audit shows the forward gap
    /// moving only −0.113 → −0.097 from level 1 to level 20, so one
    /// constant centres the duel at every level. Re-derive it from
    /// `audit_contest` if either composite's weights change.
    const CONTEST_BALANCE: f32 = 0.08;

    /// Keeper-quality multiplier on the geometric chance — scored as a
    /// **contest**. `skill` is the keeper's `gk_shot_stopping` composite
    /// and `threat` the striker's `shot_threat`, both 0..1 and both
    /// linear blends so they share a scale.
    ///
    /// Level-to-level parity (~69% save rate in every division) is a
    /// property of the *relative* quality of the two men, and reading
    /// the keeper's absolute ability cannot produce it: squads scale
    /// with the division, so an absolute bar makes a lower-division
    /// keeper worse without making the strikers he faces any less
    /// dangerous. Measured, that slid save% from 75.8% at levels 16-20
    /// to 61.3% at 1-5, against a real ~69-71% at every level, and the
    /// gap was almost exactly the multiplier's own span: on a dead-centre
    /// shot a weak keeper sat at 0.512 and an elite one at 0.635.
    ///
    /// An equal-quality duel returns `SKILL_FLOOR + SKILL_SLOPE/2` —
    /// the same 0.68 the population save rate was calibrated on — so
    /// this is calibration-neutral at the mean while removing the drift.
    /// Crucially it does NOT delete the keeper axis, which the previous
    /// flat-multiplier attempts did: a keeper better than the strikers
    /// he faces still saves more, and that difference is now measured
    /// against his actual opposition rather than against the whole game.
    #[inline]
    pub(crate) fn skill_multiplier(skill: f32, threat: f32) -> f32 {
        let edge = skill.clamp(0.0, 1.0) - threat.clamp(0.0, 1.0) + Self::CONTEST_BALANCE;
        let advantage = (0.5 + edge * Self::CONTEST_SPREAD).clamp(0.0, 1.0);
        Self::SKILL_FLOOR + advantage * Self::SKILL_SLOPE
    }

    /// Full per-shot save probability for the physics roll.
    #[inline]
    pub(crate) fn save_probability(
        reach_ratio: f32,
        speed_penalty: f32,
        skill: f32,
        threat: f32,
        env_handling_delta: f32,
    ) -> f32 {
        ((Self::geometric_base(reach_ratio) - speed_penalty)
            * Self::skill_multiplier(skill, threat)
            + env_handling_delta)
            .clamp(Self::MIN_SAVE, Self::MAX_SAVE)
    }

    /// Reference point for the spread guard: an ordinary centred shot
    /// from an ordinary striker, no speed penalty, no weather.
    #[inline]
    pub(crate) fn centred_save_probability(skill: f32) -> f32 {
        Self::save_probability(0.0, 0.0, skill, Self::NEUTRAL_THREAT, 0.0)
    }

    // ── Post-shot expectation (xGoT) ────────────────────────────────
    //
    // What a *league-average* keeper would have conceded from this exact
    // strike. The rating model needs it to separate a keeper from the
    // defence in front of him: `goals_prevented` is only an honest
    // measure of shot-stopping if the expectation it subtracts knows
    // whether the shots were corner-bound rockets or tame efforts down
    // the middle. Every input below is a property of the STRIKE — where
    // it is going, how fast, how high — and none of them is a property
    // of the keeper, which is what makes the resulting expectation
    // something he can be measured against rather than something he
    // moves by playing well.

    /// Reach of a population-mean keeper, in game units. The live model
    /// is `20 + agility01·8 + reflexes01·4` (see [`Ball::try_save_shot`]);
    /// at the mid-band agility/reflexes generated squads carry (~0.55
    /// normalised) that lands on ~26u. Fixed rather than read from the
    /// keeper on purpose — a keeper with elite reach would otherwise
    /// lower his own expectation and cancel his own advantage.
    const REFERENCE_REACH: f32 = 26.0;

    /// Normalised reflexes of that same reference keeper, feeding the
    /// speed penalty exactly as the live path does.
    const REFERENCE_REFLEXES: f32 = 0.55;

    /// Multiplier an evenly-matched duel resolves to — the contest's own
    /// definition of "ordinary keeper against the striker who hit it"
    /// ([`Self::skill_multiplier`] with `edge == 0`). Using it here is
    /// what keeps the expectation level-invariant: it is the same
    /// relative bar in every division, so a lower-division keeper is not
    /// judged against a top-flight keeper's hands.
    const NEUTRAL_MULTIPLIER: f32 = Self::SKILL_FLOOR + Self::SKILL_SLOPE * 0.5;

    /// Probability that a league-average keeper concedes this strike —
    /// the engine's own post-shot expected-goal value for one shot on
    /// target.
    ///
    /// `lateral` is the shot's placement measured from the GOAL CENTRE
    /// (not from where the keeper happens to be standing), `speed` the
    /// ball's velocity magnitude, `height` its projected height at the
    /// line. Deliberately built from [`Self::geometric_base`] and the
    /// same speed penalty the live roll uses, so the expectation and the
    /// outcome are produced by one model: whatever calibration moves the
    /// save rate moves the bar it is measured against by the same
    /// amount.
    pub(crate) fn expected_goal_on_target(lateral: f32, speed: f32, height: f32) -> f32 {
        // Beyond a league-average keeper's dive there is no save to
        // make — the live path returns before rolling in exactly this
        // case, so the expectation has to agree.
        if lateral.abs() > Self::REFERENCE_REACH {
            return 1.0 - Self::MIN_SAVE;
        }
        let reach_ratio = (lateral.abs() / Self::REFERENCE_REACH).clamp(0.0, 1.0);
        let speed_excess = (speed - 3.0).max(0.0);
        let speed_penalty =
            (speed_excess * 0.08 * (1.0 - Self::REFERENCE_REFLEXES * 0.5)).min(0.40);
        // Height is not in the live geometric term (the save model is
        // lateral-only), but a ball lifted toward the angle is measurably
        // harder and ignoring it would let a keeper's expectation read
        // the same for a rolling shot and one under the bar. Kept small
        // so the lateral geometry stays dominant.
        let height_penalty = (height / GOAL_HEIGHT).clamp(0.0, 1.0) * Self::HEIGHT_PENALTY;
        let save = ((Self::geometric_base(reach_ratio) - speed_penalty - height_penalty)
            * Self::NEUTRAL_MULTIPLIER)
            .clamp(Self::MIN_SAVE, Self::MAX_SAVE);
        1.0 - save
    }

    /// How much of the geometric ceiling a shot lifted to the crossbar
    /// gives away for the reference keeper. Small next to
    /// `STRETCH_PENALTY` (0.58) — going wide beats a keeper far more
    /// often than going high.
    const HEIGHT_PENALTY: f32 = 0.10;
}

impl Ball {
    /// Opposing players near the ball's flight path can intercept passes.
    /// Interception chance depends on tackling, anticipation, positioning skills
    /// and proximity to the ball's trajectory.
    pub fn try_intercept(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // `context` is held even when this site does not currently
        // draw from `context.rng` so future calibration / env-modifier
        // wiring (slide tackle range, sliding_tackle_success) lands
        // without changing the signature again.
        let _ = context;
        // Only intercept unowned balls that are in flight (active pass).
        // A ball in a keeper's gloves is neither, but guard it explicitly
        // — it is the one state where "unowned" could ever be wrong.
        if self.current_owner.is_some() || self.held_in_hands || self.flags.in_flight_state == 0 {
            return;
        }

        // HEIGHT IS A DIFFICULTY, NOT A DOOR.
        //
        // This was a single `z > 2.5` gate and nothing else in the whole
        // pass read the height again. Both halves of that were wrong, and
        // they produce the two complaints that sound like opposites:
        //
        //   * Below the bar, a ball at 2.4 m — a foot above a standing
        //     player's head — was exactly as interceptable as one rolling
        //     along the floor. A defender plucked it out of the air
        //     without moving, because nothing asked him to jump and
        //     nothing made it harder that he hadn't.
        //   * Above the bar the ball simply did not exist, whoever was
        //     under it and however well he leaps. 2.51 m was unreachable
        //     for the best header of the ball in the division.
        //
        // The ceiling now belongs to the PLAYER (`AerialReach::ceiling`,
        // from his `jumping`), the chance falls away as the ball climbs
        // toward it, and a defender who takes one above his standing
        // reach actually leaves the ground for it — see the `Leap` event
        // below. The flat cut here only skips the loop early for balls
        // beyond any human being.
        if self.position.z > AerialReach::ceiling(20.0) {
            return;
        }

        // Need to know who passed to determine the opposing team
        let passer_team = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id).map(|p| p.team_id),
            None => return,
        };
        let passer_team = match passer_team {
            Some(t) => t,
            None => return,
        };

        // Ball velocity determines the interception corridor width.
        //
        // The floor only exists to hand a near-stationary ball to normal
        // claiming — it is not a calibration knob. It was `speed < 1.0`,
        // set when passes were struck at 0.5-2.7 u/tick under friction
        // ~3.7× real. With `GROUND_FRICTION` corrected, a real pass now
        // leaves the foot at 0.5-2.2 and arrives slower still, so a 1.0
        // floor excluded most passes outright and interceptions fell from
        // 37 to 2.6 per team against a real ~10. 0.25 u/tick is 3.1 m/s —
        // the same physical meaning of "the ball is actually travelling"
        // that 1.0 carried before the units moved under it.
        const MIN_INTERCEPTABLE_SPEED: f32 = 0.25;
        let ball_speed_sq = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;
        if ball_speed_sq < MIN_INTERCEPTABLE_SPEED * MIN_INTERCEPTABLE_SPEED {
            return; // Ball too slow, normal claiming handles it
        }

        // Interception reach in game units. Field is 840u = 105m, so 1u =
        // 0.125m. Old 2.5u left average defenders mathematically
        // unable to intercept (max score 0.039 vs 0.04 threshold). 5u
        // produced ~0.1 interceptions/team/match — defenders within
        // the radius hit ~0.025 chance, below the 0.035 threshold for
        // anyone but the closest, fastest, best-positioned. 6.5u
        // (~0.8m — a stretch-extension radius for the planted leg) and
        // a slightly higher base coefficient produces ~10
        // interceptions/team/match (real-football band) without the
        // intercept→snap→re-pass loops the previous 8u radius caused.
        const INTERCEPT_RADIUS: f32 = 5.5;
        const INTERCEPT_RADIUS_SQ: f32 = INTERCEPT_RADIUS * INTERCEPT_RADIUS;

        let mut best_interceptor: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing team players can intercept
            if player.team_id == passer_team {
                continue;
            }

            // Don't let the pass target's team intercept their own pass target
            if Some(player.id) == self.pass_target_player_id {
                continue;
            }

            // Distance from player to ball
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > INTERCEPT_RADIUS_SQ {
                continue;
            }

            // How high the ball is, measured against THIS player's leap.
            // Zero means it is over him however well he reads the play,
            // and he is skipped rather than scored at a token chance.
            let height_factor =
                AerialReach::reach_difficulty(self.position.z, player.skills.physical.jumping);
            if height_factor <= 0.0 {
                continue;
            }

            // Base chance: dedicated `interception` composite — anticipation,
            // positioning, concentration, marking, etc. routed through
            // `effective_skill` so fatigue applies. Drop-in replacement for
            // the legacy 4-skill average; magnitude lands in the same band
            // (0..1). Minute derived from the cached tick (10ms ticks).
            let minute = sc::minute_from_ticks(self.current_tick_cached);
            let skill_factor = sc::interception(player, minute);

            // Proximity factor: closer = higher chance (1.0 at 0m, 0.3 at max radius)
            let dist = dist_sq.sqrt();
            let proximity_factor = 1.0 - (dist / INTERCEPT_RADIUS) * 0.7;

            // Fast passes are harder to intercept — penalty coefficient
            // moderated from 0.10 (which made 7 u/tick passes 41% harder
            // than slow ones) back toward a lighter slope.
            let speed_penalty = 1.0 / (1.0 + ball_speed_sq.sqrt() * 0.06);

            // Per-tick interception chance. The 0.13 coefficient with
            // the 0.035 threshold mathematically excluded average
            // defenders (skill 0.5 × proximity 0.65 × speed 0.6 ≈
            // 0.025 per the old radius), so observed interceptions
            // were ~0.1/team/match vs real ~10/team. 0.16 (with the
            // bumped 5.5u radius and lowered 0.030 threshold) brings
            // an average-positioned defender to ~0.038 (above
            // threshold), and an elite defender at point-blank to
            // ~0.07, while still leaving peripheral or off-the-pace
            // defenders below the bar. Population per-team
            // interceptions land near 12–13/match.
            let chance = skill_factor * proximity_factor * speed_penalty * height_factor * 0.16;

            if chance > best_chance {
                best_chance = chance;
                best_interceptor = Some(player.id);
            }
        }

        // ONE PASS, ONE ATTEMPT — and it is a roll, not a threshold.
        //
        // This used to fire deterministically whenever `best_chance`
        // cleared 0.030, re-evaluated every tick the ball was in flight.
        // That made the interception RATE a function of how long the
        // flight window happened to be rather than of the defending, and
        // the previous note here recorded the consequence honestly: ~120
        // interceptions per team against a real ~10, "~3× of that from
        // the flight-protection extension tripling the per-pass intercept
        // window". Correcting `GROUND_FRICTION` made the flights longer
        // and more realistic still, and the deterministic form promptly
        // ran to 1000+ per team.
        //
        // The chance the loop builds is already a per-event probability,
        // so roll it. Latch on the first tick a defender is genuinely in
        // reach — that is the moment the ball comes past him, and he gets
        // one go at it, exactly as `try_block_shot` gives one roll per
        // shot. Rate is now independent of the window length.
        // ── A SHOT IS NOT A PASS, AND THIS SITE ONLY KNOWS ABOUT PASSES ──
        //
        // Live shots used to keep a per-tick DETERMINISTIC path here
        // (`best_chance > 0.030`, re-evaluated every tick of the flight),
        // on the argument that `try_block_shot` was too weak to carry the
        // channel. Measured with the shot-lifecycle census, that is what
        // it was actually doing:
        //
        //   * **72.6% of every shot struck ended here** — claimed clean,
        //     mid-flight, by an outfield defender. Not blocked, not
        //     deflected: `velocity = zeros()` and possession handed over.
        //   * Shots lived **8.3 ticks** on average despite being struck
        //     from 102u (12.8 m), a distance that needs 40-60 ticks of
        //     flight. They were being eaten within a tick of leaving the
        //     boot.
        //   * Only 11% of shots ever reached the goal at all, and those
        //     that did were struck from 74u against 102u for the
        //     population — so the leak was distance-selective and long
        //     shots produced essentially no goals.
        //
        // That is the "aimed on frame but never resolves" report: 63% of
        // shots leave the boot between the posts and 9% are credited on
        // target. It is also why blocks read 4.84 per defender against a
        // real ~0.9 — this path was filing its takings as blocks.
        //
        // A defender getting a body in front of a strike is
        // `try_block_shot`: one roll per shot, a real corridor, a real
        // height limit, and a DEFLECTION rather than a clean pick-up.
        // Shots are excluded here so that model owns them, which is also
        // what makes its rate rise — it rolls on the first tick a
        // defender is in the lane, and shots now survive long enough to
        // find one.
        // `cached_shot_target` is exactly the right test: it is set at the
        // strike and cleared the moment anybody touches the ball, so a
        // shot the keeper has parried or a defender has deflected is a
        // genuine loose ball again and IS interceptable from here.
        if self.cached_shot_target.is_some() {
            return;
        }
        if let Some(interceptor_id) = best_interceptor.filter(|_| !self.intercept_rolled) {
            self.intercept_rolled = true;
            let fires = context.rng.unit_f32() < best_chance;
            if fires {
                // A ball taken above standing reach is taken in the air.
                // The ball code holds the squad immutably, so it asks for
                // the jump rather than performing it — see
                // `PlayerEvent::Leap`.
                let interceptor_jumping = players
                    .iter()
                    .find(|p| p.id == interceptor_id)
                    .map_or(10.0, |p| p.skills.physical.jumping);
                let leap = AerialReach::leap_for(self.position.z, interceptor_jumping);
                if leap > 0.0 {
                    events.add_player_event(PlayerEvent::Leap(interceptor_id, leap));
                }

                // How high was it, and was he off the ground? A ball
                // picked out of the air at head height by a man standing
                // flat-footed is the reported symptom, and no existing
                // counter can see it.
                #[cfg(feature = "match-logs")]
                {
                    let airborne = players
                        .iter()
                        .find(|p| p.id == interceptor_id)
                        .is_some_and(|p| p.is_airborne());
                    super::flight_diag::FlightDiag::note_intercept(
                        self.position.z,
                        AerialReach::STANDING,
                        airborne || leap > 0.0,
                    );
                }
                // Snap the ball to the interceptor and zero the
                // velocity. Before this, velocity was just scaled to
                // Zeroing velocity + handing ownership to the defender
                // prevents the old "own-goal after intercept" bug without
                // needing to teleport the ball. `move_to` will track the
                // ball toward its new owner at 1.5 u/tick over the next
                // 2-3 ticks, so visually the ball decelerates into the
                // defender's feet instead of jumping instantly from its
                // flight path onto the defender — which was visible to
                // the user as "ball appearing on another player without
                // moving".
                //
                // OG risk is fully handled by `self.velocity = zeros()`:
                // a stationary ball can't roll past the 15u owner-drop
                // threshold, so it can't cross the goal line unowned.
                let _ = interceptor_id; // no teleport, keep position as-is
                self.current_owner = Some(interceptor_id);
                self.pass_target_player_id = None;
                self.flags.in_flight_state = 0;
                self.claim_cooldown = 15;
                self.velocity = Vector3::zeros();
                self.position.z = 0.0;
                // Interception ends any in-flight shot — a defender taking
                // control downfield extinguishes the shot. Without this,
                // the next time the keeper grabs a moving ball from an
                // opponent (a long pass that loops to them), the stale
                // shot flag credits a phantom save and inflates the
                // saves/on-target ratio above 100%.
                //
                // Note what was extinguished: if this was a live shot the
                // defender did not intercept a pass, he blocked a strike,
                // and that is what the stat sheet should say. Captured
                // before the flag is cleared and carried on the event.
                // A shot the defender got a body in front of is a BLOCK,
                // and the stat sheet should say so. Keying that purely
                // off `cached_shot_target` under-reported it badly: the
                // target is cleared by several paths (a failed save, a
                // keeper touch, a deflection) while the ball is still
                // very much a shot in flight, and every stop after that
                // point was filed as an ordinary interception. Blocks
                // measured 0.18 per defender against a real ~0.9 for
                // exactly this reason. `last_shot_struck_tick` is the
                // robust question — was this BALL struck at goal
                // recently — and is cleared on any dead ball.
                let was_live_shot = self.cached_shot_target.is_some()
                    || (self.last_shot_struck_tick > 0
                        && self
                            .current_tick_cached
                            .saturating_sub(self.last_shot_struck_tick)
                            < 400);
                self.cached_shot_target = None;
                let interceptor_team = players
                    .iter()
                    .find(|p| p.id == interceptor_id)
                    .map(|p| p.team_id)
                    .unwrap_or(0);
                let tick = self.current_tick_cached;
                self.record_touch(interceptor_id, interceptor_team, tick, true);
                self.offside_snapshot = None;
                self.pass_origin_restart = PassOriginRestart::OpenPlay;
                events.add_ball_event(BallEvent::Intercepted(
                    interceptor_id,
                    self.previous_owner,
                    was_live_shot,
                ));
            }
        }
    }

    /// Shot-block check. Runs only when the ball is a shot in flight
    /// (has a cached goal-line target). A defender whose body is in
    /// the shot's corridor between the current ball position and the
    /// goal line has a skill-weighted chance to block — the ball
    /// deflects to a loose state rather than reaching the keeper.
    /// Real football blocks ~6-10% of shots; we aim for that band.
    ///
    /// Distinct from `try_intercept`:
    /// - Intercept: ≤ 2.5u radius, pass-targeted; tiny per-tick chance
    /// - Block:     ≤ 4u radius, shot-targeted; higher per-event chance
    /// Both are scoped to unowned balls with `in_flight_state > 0`.
    pub fn try_block_shot(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Only live shots — no cache means no shot in flight, no block.
        let shot_target = match self.cached_shot_target {
            Some(t) => t,
            None => return,
        };
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }
        // One shot, one roll — see `ShotTarget::block_rolled`.
        if shot_target.block_rolled {
            return;
        }
        #[cfg(feature = "match-logs")]
        block_diag::SHOTS_SEEN.fetch_add(1, Ordering::Relaxed);
        // Ball above defender reach. This read `> 2.0` and the comment
        // called it "chest height" — but 1u is 0.125 m, so the bar was
        // 25 CENTIMETRES. Anything above ankle height was unblockable,
        // which excluded 23% of all shot-ticks outright. A defender
        // blocks with whatever he can get in the way, up to a raised
        // boot or a head: 16u is 2 m.
        // 2.2 m — a defender's raised-arm reach. Was 16.0, which on a
        // vertical axis measured in metres put the block ceiling above the
        // stands; it never rejected anything.
        const MAX_BLOCK_HEIGHT: f32 = 2.2;
        if self.position.z > MAX_BLOCK_HEIGHT {
            #[cfg(feature = "match-logs")]
            block_diag::TOO_HIGH.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let shooter_team = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id).map(|p| p.team_id),
            None => return,
        };
        let shooter_team = match shooter_team {
            Some(t) => t,
            None => return,
        };

        // Defender must be in the shot's path: between the ball and
        // the goal line, in the corridor defined by the shot direction.
        let ball_velocity_2d =
            (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if ball_velocity_2d < 0.5 {
            return; // Ball has stopped / nearly — not a live shot.
        }
        let shot_dir_x = self.velocity.x / ball_velocity_2d;
        let shot_dir_y = self.velocity.y / ball_velocity_2d;

        // Block window. Widened from 30u lookahead + 4u corridor so
        // defenders near the shot line have a real chance to get a
        // leg/body in. Real football blocks ~18-22% of shots (2-3 per
        // team per match from ~13 shots); the engine emits ~0.01 blocks
        // per defender per match.
        //
        // ⚠ That gap is NOT this window. Measured with `block_diag`
        // (2026-08, n=400 at L14): of 246k shot-ticks reaching the
        // check, 28% are above blocking height and **0.1% ever find a
        // defender in the lane at all** — so the roll below almost never
        // gets to happen. Widening the lookahead to 120u (15m, the
        // distance shots are really taken from) and the corridor to 16u
        // (2m, a committed lunge rather than a standing body) moved
        // candidates from 0.0% to 0.1% and blocks not at all. Defenders
        // are simply not between the ball and the goal while a shot is
        // in flight, which is a positioning property of the engine and a
        // separate piece of work from the block model. Both constants
        // are therefore left where they were rather than carrying an
        // unmeasured widening for no benefit.
        // Widened 40/7 → 90/13 once the 25cm height bar above was lifted.
        // The earlier attempt recorded in this comment measured no gain,
        // but it was made while that bar silently threw away every ball
        // above ankle height, so the corridor was never the thing being
        // tested. 90u is 11 m — the range over which a defender can still
        // get across to a shot — and 13u is 1.6 m, a committed lunge or
        // slide rather than a standing body.
        const BLOCK_LOOKAHEAD: f32 = 90.0;
        const BLOCK_CORRIDOR: f32 = 16.0;

        let mut best_blocker: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing outfielders block (GK save pipeline handles
            // shots that reach the line; a GK blocking a shot at 5u
            // out is already Catching/Diving).
            if player.team_id == shooter_team {
                continue;
            }
            if player.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }

            #[cfg(feature = "match-logs")]
            block_diag::OPP_SEEN.fetch_add(1, Ordering::Relaxed);

            // Project defender position onto the shot line.
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let projection = dx * shot_dir_x + dy * shot_dir_y;
            // Must be ahead of the ball along the shot line, within
            // the lookahead window. 1u minimum so a defender level
            // with the ball (who's already been passed) doesn't count.
            if projection < 1.0 {
                #[cfg(feature = "match-logs")]
                block_diag::BEHIND_BALL.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if projection > BLOCK_LOOKAHEAD {
                #[cfg(feature = "match-logs")]
                block_diag::BEYOND_LOOKAHEAD.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            // Perpendicular distance to the line.
            let perp =
                (dx - projection * shot_dir_x).powi(2) + (dy - projection * shot_dir_y).powi(2);
            let perp_dist = perp.sqrt();
            #[cfg(feature = "match-logs")]
            {
                block_diag::IN_WINDOW.fetch_add(1, Ordering::Relaxed);
                block_diag::PERP_SUM_X100.fetch_add((perp_dist * 100.0) as u64, Ordering::Relaxed);
            }
            if perp_dist > BLOCK_CORRIDOR {
                #[cfg(feature = "match-logs")]
                block_diag::OUTSIDE_CORRIDOR.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Skill mix: bravery (willingness to step into shot),
            // positioning (read the angle), anticipation (read the
            // cue), jumping/agility (get the body in the way), plus
            // tackling (stretching / last-ditch leg out). Weighted
            // toward mental attributes since shot-blocking is 70%
            // reading the shooter's body shape. Routed through
            // `effective_skill` so a tired defender blocks worse.
            let block_minute = sc::minute_from_ticks(self.current_tick_cached);
            let block_tech = EffSkillCtx::technical(block_minute);
            let block_mental = EffSkillCtx::mental(block_minute);
            let block_expl = EffSkillCtx::explosive(block_minute);
            let bravery = effective_skill(player, player.skills.mental.bravery, block_mental);
            let positioning =
                effective_skill(player, player.skills.mental.positioning, block_mental);
            let anticipation =
                effective_skill(player, player.skills.mental.anticipation, block_mental);
            let agility = effective_skill(player, player.skills.physical.agility, block_expl);
            let tackling = effective_skill(player, player.skills.technical.tackling, block_tech);
            let skill_factor = (bravery * 0.25
                + positioning * 0.25
                + anticipation * 0.25
                + agility * 0.15
                + tackling * 0.10)
                / 20.0;

            // Line factor — closer to the ball is better because the
            // defender's body is actually in the way. Farther along the
            // line means the shot has had time to rise / dip / move.
            let line_factor = 1.0 - (projection / BLOCK_LOOKAHEAD) * 0.4;
            // Perp factor — right on the line is best. Steeper fall-off
            // than before (0.5 from center → basically full chance;
            // 1.0 from edge → 60% chance) so wings-of-corridor still
            // produce blocks at meaningful rates.
            let perp_factor = 1.0 - (perp_dist / BLOCK_CORRIDOR) * 0.5;
            // Fast shots are harder to get in front of — but reaction
            // reflexes matter too. Elite defender reads the shape and
            // steps a tick earlier.
            let speed_penalty = 1.0 / (1.0 + ball_velocity_2d * 0.10);

            // Base multiplier 0.55 (was 0.35) — elite defenders
            // (skill_factor ≈ 0.85) at a good angle now block at
            // 30-40% chance, matching the real "closed-down striker
            // gets the ball blocked" rate.
            let chance = skill_factor * line_factor * perp_factor * speed_penalty * 0.95;

            if chance > best_chance {
                best_chance = chance;
                best_blocker = Some(player.id);
            }
        }

        // RNG threshold instead of deterministic cutoff: a 30% block
        // chance still allows the shot through 70% of the time, which
        // is what we want — defenders block but don't always block.
        //
        // Latch BEFORE rolling so a shot that survives the best-placed
        // defender is not re-offered to him (or to a worse one) on the
        // next tick.
        if best_blocker.is_some() {
            #[cfg(feature = "match-logs")]
            block_diag::CANDIDATES.fetch_add(1, Ordering::Relaxed);
            if let Some(t) = self.cached_shot_target.as_mut() {
                t.block_rolled = true;
            }
        }
        let blocker_id = match best_blocker {
            Some(id) if context.rng.unit_f32() < best_chance.clamp(0.03, 0.70) => id,
            _ => return,
        };
        #[cfg(feature = "match-logs")]
        block_diag::FIRED.fetch_add(1, Ordering::Relaxed);

        // Outcome distribution. Real blocks rarely produce clean
        // possession — they produce loose balls, deflections wide for a
        // corner, sideways skips, or (rarely) deflections back into
        // danger. The previous deterministic ownership flow over-credited
        // defenders.
        let blocker = match players.iter().find(|p| p.id == blocker_id) {
            Some(p) => p,
            None => return,
        };
        let blocker_pos = blocker.position;
        let blocker_team = blocker.team_id;
        let blocker_side = blocker.side;
        let composure = (blocker.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let technique = (blocker.skills.technical.technique / 20.0).clamp(0.0, 1.0);
        let ball_speed_low_bonus = if ball_velocity_2d < 2.0 { 0.06 } else { 0.0 };
        // Taking the ball cleanly off a block means having it at your
        // feet, so it is only available to a defender the ball actually
        // reached — see the position note below. Blocking at a stretch
        // from range always leaves a loose ball. Without this the
        // controlled branch could hand ownership to a man 11 m away, and
        // `move_to` would drop it again on the next tick as an
        // unreachable owner.
        let blocker_gap = (blocker_pos.x - self.position.x).hypot(blocker_pos.y - self.position.y);
        let blocker_in_reach = blocker_gap <= super::CONTROL_DISTANCE;
        let controlled_block_prob = if blocker_in_reach {
            (0.06 + composure * 0.05 + technique * 0.04 + ball_speed_low_bonus).clamp(0.06, 0.30)
        } else {
            0.0
        };

        // Deflection direction: away from the shot line, with a random ±45° spread.
        let angle: f32 = (context.rng.unit_f32() - 0.5) * 1.56;
        let rev_x = -shot_dir_x * angle.cos() - (-shot_dir_y) * angle.sin();
        let rev_y = -shot_dir_x * angle.sin() + (-shot_dir_y) * angle.cos();
        let tick = self.current_tick_cached;

        let roll = context.rng.unit_f32();
        let p_controlled = controlled_block_prob;
        let p_corner = p_controlled + 0.23;
        let p_safe = p_corner + 0.23;
        let p_loose = p_safe + 0.40; // ~40% loose central rebound
        // remainder ~14% → unlucky deflection toward goal (slows but stays live)

        // A block happens where the BALL is, not where the defender is.
        //
        // This used to be an unconditional `self.position = blocker_pos`,
        // and the block window reaches 90u ahead and 16u across — so a
        // successful block could pick the ball up in mid-flight and set
        // it down eleven metres away inside a single tick. That is the
        // "ball suddenly somewhere else for no reason" report exactly:
        // there is no kick, no bounce and no carry to explain it, the
        // ball is simply elsewhere on the next frame.
        //
        // Moving the ball to the blocker at all is only defensible when
        // he is close enough that the two were going to meet anyway.
        // Beyond that the contact is a stretched leg or a slide, and the
        // ball deflects from its own position. `CONTROL_DISTANCE` is the
        // engine's existing answer to "close enough to have played it".
        if blocker_in_reach {
            self.position = blocker_pos;
        }
        self.position.z = 0.0;
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.pass_target_player_id = None;
        self.cached_shot_target = None;
        self.record_touch(blocker_id, blocker_team, tick, false);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        // Dedicated Blocked event so the block credit can't leak into a
        // separate Intercepted that happens to share the same tick — the
        // ordering of events in `EventCollection` is no longer load-
        // bearing for stat correctness.
        let block_position = self.position;
        events.add_ball_event(BallEvent::Blocked(blocker_id, block_position));

        if roll < p_controlled {
            // Clean block — defender gets the ball at his feet.
            self.velocity = Vector3::zeros();
            self.current_owner = Some(blocker_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 25;
            events.add_ball_event(BallEvent::Intercepted(
                blocker_id,
                self.previous_owner,
                false,
            ));
            return;
        }

        // Deflection branches below leave the ball loose (no owner) and
        // do NOT emit `Intercepted` — block credit was already booked
        // via the dedicated `Blocked` event above. Emitting `Intercepted`
        // here would double-credit (interception + block), and worse,
        // its `ClaimBall` follow-up would force ownership onto a
        // defender who in physics terms hasn't actually picked the ball
        // up. Possession is decided by whoever claims the loose ball
        // next, not by the block itself.
        if roll < p_corner {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::BLOCK_CORNER_FIRED.fetch_add(1, Ordering::Relaxed);
            // Deflection out for a corner — push the ball past the
            // defender's OWN byline and WIDE OF THE POST (toward the corner
            // flag) so the endline resolver awards a corner (defender = last
            // toucher → corner for the attackers). Aiming merely at the
            // byline (the old ±1.2 y nudge) left a central block crossing
            // BETWEEN the posts → goal kick / own goal, so blocks almost
            // never became corners (engine ran ~0.5 corners/match vs ~10
            // real). The ball must finish outside `center ± GOAL_WIDTH`.
            let endline_x = match blocker_side {
                Some(PlayerSide::Left) => 0.0_f32,
                Some(PlayerSide::Right) => self.field_width,
                None => {
                    if self.position.x < self.field_width * 0.5 {
                        0.0
                    } else {
                        self.field_width
                    }
                }
            };
            let center_y = self.field_height * 0.5;
            // Deflect toward the touchline the ball is already drifting to
            // (sign of the reverse-deflection y), past the post.
            let to_top = if rev_y.abs() > 0.01 {
                rev_y < 0.0
            } else {
                self.position.y < center_y
            };
            let wide_y = if to_top {
                (center_y - GOAL_WIDTH - self.field_height * 0.05).max(2.0)
            } else {
                (center_y + GOAL_WIDTH + self.field_height * 0.05).min(self.field_height - 2.0)
            };
            let dx = endline_x - self.position.x;
            let dy = wide_y - self.position.y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let speed = (ball_velocity_2d * 0.6).clamp(3.0, 6.0);
            self.velocity.x = (dx / dist) * speed;
            self.velocity.y = (dy / dist) * speed;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 30;
            // Hold off re-claims so the deflection crosses the byline before
            // a covering defender grabs it back (else it never becomes a
            // corner — the whole point of this branch).
            self.claim_cooldown = 16;
            return;
        }

        if roll < p_safe {
            // Safe sideways deflection — perpendicular skip away from
            // both goals. Loose ball; either team can recover.
            let safe_speed = (ball_velocity_2d * 0.35).clamp(1.5, 3.5);
            // Rotate shot direction 90° (sign chosen by random) to skip sideways.
            let sign = if context.rng.unit_f32() < 0.5 {
                -1.0
            } else {
                1.0
            };
            self.velocity.x = -shot_dir_y * sign * safe_speed;
            self.velocity.y = shot_dir_x * sign * safe_speed;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 25;
            self.claim_cooldown = 0;
            return;
        }

        if roll < p_loose {
            // Loose central rebound — ball trickles in front of the
            // defender, often producing a second-ball contest. Arms the
            // rebound window (team shot-spacing exemption) so the
            // second ball can actually be struck. The blocker is the
            // last player the ball came off — recording him as previous
            // owner makes the ATTACKERS the intercept-eligible side
            // during the flight window (the spill is the defender's
            // touch, not the shooter's pass), restoring the two-sided
            // second-ball race.
            self.last_rebound_tick = tick;
            self.previous_owner = Some(blocker_id);
            let loose_speed = (ball_velocity_2d * 0.30).clamp(1.0, 2.8);
            self.velocity.x = rev_x * loose_speed;
            self.velocity.y = rev_y * loose_speed;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 20;
            self.claim_cooldown = 0;
            return;
        }

        // Unlucky deflection: ball loses pace but keeps drifting toward
        // goal. The shot flag is already cleared, so the keeper save
        // pipeline won't credit a phantom save — but the ball is still
        // live and can be a tap-in opportunity. Arms the rebound window;
        // blocker booked as previous owner (see the loose branch above).
        self.last_rebound_tick = tick;
        self.previous_owner = Some(blocker_id);
        let unlucky_speed = (ball_velocity_2d * 0.50).clamp(1.5, 3.5);
        self.velocity.x = shot_dir_x * unlucky_speed * 0.7;
        self.velocity.y = shot_dir_y * unlucky_speed * 0.7;
        self.velocity.z = 0.0;
        self.current_owner = None;
        self.flags.in_flight_state = 25;
        self.claim_cooldown = 0;
    }

    /// Goalkeeper save check. Runs during shot flight: when the ball
    /// approaches the goal line and the defending keeper's body is
    /// within reach of the shot's trajectory, roll a skill-weighted
    /// save. The keeper state machine's `is_catch_successful` path
    /// timed saves to player-state ticks that didn't line up with the
    /// ball's physics step — saves fired too early or too late, and
    /// shots past the keeper cleared into the net. A physics-level
    /// save runs every ball tick with fresh ball position and commits
    /// the ball to the keeper at the moment of contact.
    pub fn try_save_shot(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let shot_target = match self.cached_shot_target {
            Some(t) => t,
            None => return,
        };
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }

        // Only consider the shot once it's close to the goal line —
        // the save resolves at the moment of contact. Distance in
        // x-units the ball will cover in a single tick determines the
        // window: we check within ~2 ticks of arrival.
        let (goal_x, goal_y) = match shot_target.defending_side {
            PlayerSide::Left => (context.goal_positions.left.x, context.goal_positions.left.y),
            PlayerSide::Right => (
                context.goal_positions.right.x,
                context.goal_positions.right.y,
            ),
        };

        // Reject balls that have already crossed the goal line. Using
        // `.abs()` below meant a shot 2u behind the goal at goal_y+15
        // still satisfied "close to goal line" and "moving toward goal"
        // and got saved out of thin air — the visible bug: ball flies
        // past the goal, then teleports into the keeper's hands. Once
        // the ball is past the line (goal or goal kick, depending on Y),
        // the shot is over.
        let past_goal_line = match shot_target.defending_side {
            PlayerSide::Left => self.position.x < goal_x,
            PlayerSide::Right => self.position.x > goal_x,
        };
        if past_goal_line {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::SAVE_TICKS_PAST_GOAL_LINE.fetch_add(1, Ordering::Relaxed);
            self.cached_shot_target = None;
            return;
        }

        let dist_to_goal_x = (self.position.x - goal_x).abs();
        let ball_vx = self.velocity.x.abs().max(0.5);
        if dist_to_goal_x > ball_vx * 2.5 {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::SAVE_TICKS_OUT_OF_REACH.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // The ball has reached the goal line. Anything off the frame at
        // this point is a MISS, and the shot is over — retire it.
        //
        // Retiring matters as much as not saving it. `cached_shot_target`
        // is what `gk_clearing_shot` reads to decide that a keeper who
        // has just gathered the ball made a save, and it credits the
        // shooter an on-target shot at the same time. Leaving the cache
        // armed on a miss meant every skied or wide shot the keeper
        // subsequently collected — which is most of them, since the
        // restart is his goal kick — was booked as a save on target.
        //
        // That is why the on-target rate would not respond to the aim
        // model: forcing 8 percentage points more shots off the frame
        // moved the measured rate by ~2, because the misses were being
        // credited anyway. The over-the-bar test in particular used to
        // sit ABOVE the arrival-window check and return without
        // clearing, so it fired mid-flight on any shot whose apex
        // cleared 2.8 m and left the cache armed for the rest of the
        // flight.
        let off_frame_high = self.position.z > 2.8;
        let off_frame_wide = (self.position.y - goal_y).abs() > GOAL_WIDTH + 1.0;
        if off_frame_high || off_frame_wide {
            self.cached_shot_target = None;
            return;
        }

        // One shot, one roll — see `ShotTarget::save_rolled`.
        if shot_target.save_rolled {
            return;
        }
        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_TICKS_REACHED.fetch_add(1, Ordering::Relaxed);

        // Ball must still be traveling toward that goal line.
        let moving_toward_goal = match shot_target.defending_side {
            PlayerSide::Left => self.velocity.x < -0.2,
            PlayerSide::Right => self.velocity.x > 0.2,
        };
        if !moving_toward_goal {
            return;
        }

        // Frame test already applied above, where a miss also retires
        // the shot.

        // Find the defending keeper.
        let keeper = players.iter().find(|p| {
            p.side == Some(shot_target.defending_side)
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
                && !p.is_sent_off
        });
        let keeper = match keeper {
            Some(k) => k,
            None => return,
        };

        // Route through `effective_skill` so a tired keeper has worse
        // reach / handling / reflexes than a fresh one. Routing minute
        // is taken from `MatchContext::total_match_time`.
        let minute_for_effective = sc::minute_from_ms(context.total_match_time);
        let tech_ctx = EffSkillCtx::technical(minute_for_effective);
        let mental_ctx = EffSkillCtx::mental(minute_for_effective);
        let expl_ctx = EffSkillCtx::explosive(minute_for_effective);
        let handling = effective_skill(keeper, keeper.skills.goalkeeping.handling, tech_ctx);
        let reflexes = effective_skill(keeper, keeper.skills.goalkeeping.reflexes, tech_ctx);
        let agility = effective_skill(keeper, keeper.skills.physical.agility, expl_ctx);
        // Concentration acts on the catch / parry split — focused
        // keepers catch cleaner, distracted ones parry into danger.
        let concentration = effective_skill(keeper, keeper.skills.mental.concentration, mental_ctx);
        let scaled_handling = ((handling - 1.0) / 19.0).max(0.0);
        let scaled_reflexes = ((reflexes - 1.0) / 19.0).max(0.0);
        let scaled_agility = ((agility - 1.0) / 19.0).max(0.0);
        let scaled_concentration = ((concentration - 1.0) / 19.0).max(0.0);

        // Diving reach in game units. Field is 840u = 105m, so 1u = 0.126m
        // (half-goal 29u = 3.66m matches real 3.66m). Every keeper, even a
        // youth-level one, can physically dive across most of the goal
        // — skill determines whether they *catch* the ball, not whether
        // they can reach it. The previous 10u floor made corner shots
        // literally unreachable for weak keepers, so blowouts in youth
        // leagues (hnd=1, ref=1) pushed matches to 10+ goals. New reach:
        //   skills 1   → 20u (2.5m, standing dive — can touch the post)
        //   skills 10  → 26u (3.25m, covers most of the goal)
        //   skills 20  → 32u (4.0m, elite full-stretch — beyond the post)
        let reach = 20.0 + scaled_agility * 8.0 + scaled_reflexes * 4.0;
        let lateral_error = (keeper.position.y - shot_target.goal_line_y).abs();
        if lateral_error > reach {
            return;
        }

        // Base save chance. Centered shot ~0.88; full-stretch ~0.30.
        // Skill handles the rest; this curve is purely geometry.
        let reach_ratio = (lateral_error / reach).clamp(0.0, 1.0);

        // Shot-speed penalty — elite shots beat keepers more often.
        let ball_speed = self.velocity.norm();
        let speed_excess = (ball_speed - 3.0).max(0.0);
        let speed_penalty = (speed_excess * 0.08 * (1.0 - scaled_reflexes * 0.5)).min(0.40);

        // Keeper quality. The composite blend (`gk_shot_stopping`) feeds
        // reflexes, handling, agility, positioning, concentration,
        // anticipation and one_on_ones through `effective_skill`, so a
        // tired keeper late in the match plays worse.
        let skill = sc::gk_shot_stopping(keeper, minute_for_effective);
        // Per-SHOT save probability (single roll — see `save_rolled`).
        // The curve lives on `SaveModel` so it can be pinned by test.
        //
        // History worth keeping: this slope has been flattened twice to
        // buy level-to-level save% parity, ending at `0.667 + 0.032·skill`
        // — a 4.8%-wide band between the worst keeper alive and the best.
        // That does hold ~67% at every level, but only by making keeper
        // ability irrelevant: a 17-year-old debutant saved shots like an
        // international and rated like one. Parity has to come from shot
        // quality scaling with the shooters (placement feeds `reach_ratio`,
        // power feeds `speed_penalty` — both already do), not from
        // deleting the axis. Restored to a real spread; the population
        // mean is unchanged because mean skill lands mid-band.
        //
        // NB the save path is LAYERED: this roll compounds with the GK
        // state machine's own `GkProfile::save_probability` sigmoid
        // (goalkeeper_skill.rs, deliberately compressed to steepness
        // 1.40). Keeper quality is restored at THIS boundary only —
        // cranking both is what caused the oscillation the comment
        // history in both files records.
        //
        // Environment shifts keeper handling — heavy rain spills more,
        // wind on cross-claims has a subtler effect (the keeper still
        // sets feet under a regular shot).
        let env_mod = context.environment.modifiers();
        let env_handling_delta = env_mod.goalkeeper_handling;
        let save_prob = SaveModel::save_probability(
            reach_ratio,
            speed_penalty,
            skill,
            shot_target.shooter_threat,
            env_handling_delta,
        );

        // Latch BEFORE rolling: whatever this roll decides is final for
        // this shot, so a beaten keeper doesn't get a second chance on
        // the next tick of the same flight.
        if let Some(t) = self.cached_shot_target.as_mut() {
            t.save_rolled = true;
        }

        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_PHYSICS_FIRED.fetch_add(1, Ordering::Relaxed);

        if context.rng.unit_f32() >= save_prob {
            return; // Keeper beaten — shot goes on.
        }
        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_PHYSICS_PASSED.fetch_add(1, Ordering::Relaxed);

        // Save outcome distribution. Catch / safe parry / dangerous
        // parry / corner — the previous code always caught.
        //   catch_prob   = 0.12 + handling*0.26 + positioning*0.10
        //                  + concentration*0.06
        //                  - shot_power*0.18 - reach_stretch*0.18
        //   safe_parry   = 0.20 + reflexes*0.10 + handling*0.07 + agility*0.05
        //                  + concentration*0.04
        //   dangerous    = remainder
        // Concentration shifts the split toward catch/safe parry: a
        // focused keeper does NOT spill the ball back into danger.
        let positioning = (effective_skill(keeper, keeper.skills.mental.positioning, mental_ctx)
            / 20.0)
            .clamp(0.0, 1.0);
        let shot_power_norm = (ball_speed / 8.0).clamp(0.0, 1.0);
        let reach_stretch = reach_ratio;
        let catch_prob =
            (0.12 + scaled_handling * 0.26 + positioning * 0.10 + scaled_concentration * 0.06
                - shot_power_norm * 0.18
                - reach_stretch * 0.18)
                .clamp(0.04, 0.62);
        let safe_parry_prob = (0.20
            + scaled_reflexes * 0.10
            + scaled_handling * 0.07
            + scaled_agility * 0.05
            + scaled_concentration * 0.04)
            .clamp(0.12, 0.52);

        let keeper_id = keeper.id;
        let keeper_pos = keeper.position;
        let keeper_team = keeper.team_id;
        let keeper_side = keeper.side;

        let outcome_roll = context.rng.unit_f32();
        let p_catch = catch_prob;
        let p_safe = (catch_prob + safe_parry_prob).min(0.92);

        self.position.z = 0.0;
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.pass_target_player_id = None;
        // Stage the save credit before clearing the shot target. This
        // marker is consumed by the event-dispatch step so the GK earns
        // a save in the stats sheet and the shooter's on-target count
        // increments. Without this, the physics save changes ball state
        // (catch/parry) but bypasses the state-machine save events that
        // were the only path crediting saves — leaving ~90% of resolved
        // shots stat-less.
        if let Some(shooter_id) = self.previous_owner {
            self.pending_save_credit = Some((keeper_id, shooter_id));
        }
        self.cached_shot_target = None;
        let tick = self.current_tick_cached;
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;

        if outcome_roll < p_catch {
            // Clean catch — keeper holds.
            self.position = keeper_pos;
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();
            self.current_owner = Some(keeper_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 200;
            self.record_touch(keeper_id, keeper_team, tick, true);
            events.add_ball_event(BallEvent::Claimed(keeper_id));
            return;
        }

        if outcome_roll < p_safe {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::SAVE_PARRY_FIRED.fetch_add(1, Ordering::Relaxed);
            // Parried OUT for a corner. The outcome is already decided, so
            // resolve it POSITIONALLY — place the ball just past the byline,
            // wide of the post — rather than driving it there by velocity.
            // The velocity approach half-failed: the keeper sits on the goal
            // line, so the ball only reached the post (y±GOAL_WIDTH) by the
            // time it crossed x=0, landing borderline → ~half fell inside
            // for a goal kick. Placing it out (outside `goal_y ± GOAL_WIDTH`,
            // a few units past x=0) makes the endline resolver award the
            // corner reliably next tick (keeper = last toucher → corner for
            // the attackers; save already booked via `pending_save_credit`).
            let goal_y_for_side = match keeper_side {
                Some(PlayerSide::Left) => context.goal_positions.left.y,
                Some(PlayerSide::Right) => context.goal_positions.right.y,
                None => self.position.y,
            };
            let to_top = self.position.y < goal_y_for_side;
            self.position.x = match keeper_side {
                Some(PlayerSide::Left) => -3.0,
                Some(PlayerSide::Right) => self.field_width + 3.0,
                None => self.position.x,
            };
            self.position.y = if to_top {
                (goal_y_for_side - GOAL_WIDTH - 10.0).max(3.0)
            } else {
                (goal_y_for_side + GOAL_WIDTH + 10.0).min(self.field_height - 3.0)
            };
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();
            self.current_owner = None;
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 30;
            self.record_touch(keeper_id, keeper_team, tick, false);
            // NB: do NOT emit Intercepted here — its ClaimBall follow-up
            // forces ownership onto the keeper, which CANCELS the corner
            // (the ball must stay loose and cross out). The save is already
            // booked via `pending_save_credit`, and `record_touch` marks the
            // keeper as last toucher so the endline resolver awards the
            // corner to the attackers.
            return;
        }

        // Dangerous parry — ball spills off the keeper's hands. Arms the
        // rebound window so the attacking team's follow-up shot isn't
        // killed by the team shot-spacing gate.
        self.last_rebound_tick = tick;
        // Real goalkeepers under pressure push the ball toward the side
        // they're already diving, not back into the central goalmouth
        // where the attacking team gets a free tap-in. The previous
        // ±15u y-spread around the ball position landed ~50% of parries
        // in the six-yard tap-in lane.
        let drop_distance = 12.0 + context.rng.unit_f32() * 18.0;
        let drop_x = match keeper_side {
            Some(PlayerSide::Left) => keeper_pos.x + drop_distance,
            Some(PlayerSide::Right) => keeper_pos.x - drop_distance,
            None => keeper_pos.x,
        };
        // Outward y-bias: push the ball *away* from the goal centre. If
        // the ball was already lateral, push further laterally; for
        // central shots, pick a random side and push 14-30u outward.
        let goal_center_y = match keeper_side {
            Some(PlayerSide::Left) => context.goal_positions.left.y,
            Some(PlayerSide::Right) => context.goal_positions.right.y,
            None => self.field_height * 0.5,
        };
        let outward_sign = if (self.position.y - goal_center_y).abs() < 1.0 {
            if context.rng.unit_f32() < 0.5 {
                -1.0
            } else {
                1.0
            }
        } else {
            (self.position.y - goal_center_y).signum()
        };
        let outward_offset = (14.0 + context.rng.unit_f32() * 16.0) * outward_sign;
        let drop_y = self.position.y + outward_offset + (context.rng.unit_f32() - 0.5) * 10.0;
        let drop_y = drop_y.clamp(0.0, self.field_height);
        let drop_x = drop_x.clamp(0.0, self.field_width);
        let dx = drop_x - self.position.x;
        let dy = drop_y - self.position.y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        // Spill speed: energy shed off the hands, NOT a clearance. The
        // previous constant 3.5 u/tick (43.75 m/s — harder than the
        // engine's hardest shot, capped at 3.2) carried the ball ~10m
        // through the box during the protected flight window, so every
        // "dangerous" parry physically exited the danger zone before
        // anyone could touch it. A real spill comes off the gloves at a
        // fraction of shot speed, worse for keepers with poor handling:
        // ~0.7-1.2 u/tick lands the ball in the 1.5-3.75m drop zone the
        // direction model already aims for, where the box contest can
        // actually happen.
        let parry_speed = (ball_speed * (0.22 + 0.18 * (1.0 - scaled_handling))).clamp(0.6, 1.3);
        self.velocity.x = (dx / dist) * parry_speed;
        self.velocity.y = (dy / dist) * parry_speed;
        self.velocity.z = 0.0;
        self.current_owner = None;
        // Flight window 30 → 10 ticks: the genuine time a spilled ball
        // is ungatherable. At 30 the entire rebound lived inside the
        // claims-locked window — and because `previous_owner` stayed the
        // SHOOTER, try_intercept treated the spill as an attacker pass,
        // making DEFENDERS the only players able to win it. Setting the
        // keeper as previous owner (he is physically the last player the
        // ball came off) flips the intercept population to its realistic
        // one — attackers pouncing on the spill — through the untouched
        // existing gate, and the keeper's own bounce-back reclaim still
        // lets him smother a ball that dies at his feet.
        self.previous_owner = Some(keeper_id);
        self.flags.in_flight_state = 10;
        self.claim_cooldown = 0;
        self.record_touch(keeper_id, keeper_team, tick, false);
        events.add_ball_event(BallEvent::Intercepted(
            keeper_id,
            self.previous_owner,
            false,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::SaveModel;

    /// The keeper-quality axis must stay wide enough that a youth keeper
    /// and an international are visibly different players.
    ///
    /// This guard exists because the slope was silently flattened to a
    /// 4.8-point band and no test noticed: equal-level harness runs can't
    /// see it (both keepers are equally good, so the population save rate
    /// is identical whatever the slope is), and every other GK test feeds
    /// hand-built stat lines that never touch this curve. Real
    /// within-league season save rates run ~58% for the worst regular
    /// starter to ~78% for an elite one — a ~20-point spread.
    #[test]
    fn keeper_skill_spread_stays_wide() {
        let worst = SaveModel::centred_save_probability(0.0);
        let best = SaveModel::centred_save_probability(1.0);
        let spread = best - worst;
        assert!(
            spread >= 0.15,
            "keeper quality must move the save rate by >= 15 points on a centred shot; \
             worst {worst:.3} best {best:.3} spread {spread:.3}"
        );
        assert!(
            best > worst,
            "save probability must increase with keeper skill"
        );
    }

    /// The POPULATION save rate must not move: ~67% saves/on-target is
    /// what every goals-per-match number depends on.
    ///
    /// Band re-anchored 0.66-0.70 → 0.69-0.73 when the multiplier became
    /// a contest. It is pinning the same physical quantity, but the
    /// quantity is now reached differently: the old model realised
    /// `0.54 + mean_skill·SLOPE` — which varied by division — while an
    /// ordinary duel here always resolves to `FLOOR + SLOPE/2`, so the
    /// floor absorbs the level the skill term used to supply. Measured
    /// at the calibration reference (`dev_match stats 200 14 14`),
    /// saves/on-target is 66.7% against a real ~67%, and goals/match
    /// 2.59 against a real ~2.5.
    #[test]
    fn an_ordinary_duel_holds_the_calibrated_population_save_rate() {
        // An evenly-matched duel — which is what a division's average
        // keeper faces every week, at every level.
        let mid = SaveModel::skill_multiplier(0.5, SaveModel::NEUTRAL_THREAT);
        assert!(
            (0.69..=0.73).contains(&mid),
            "an ordinary duel must stay in the calibrated 0.69-0.73 band, got {mid:.3}"
        );
    }

    /// The contest must be LEVEL-INVARIANT: scale both men together, as
    /// a division does, and the duel must not move.
    ///
    /// This is the property the absolute-skill multiplier lacked, and
    /// the reason engine save% slid ~15 points from the top division to
    /// the bottom. The composite pair is measured to keep a flat offset
    /// as level rises (`dev_match audit_contest`), so walking a keeper
    /// and a striker up the scale together must leave the multiplier
    /// where it started.
    #[test]
    fn an_evenly_matched_duel_is_the_same_in_every_division() {
        // gk / striker composites measured at levels 1, 10 and 20.
        let divisions = [(0.255, 0.368), (0.511, 0.620), (0.787, 0.884)];
        let mults: Vec<f32> = divisions
            .iter()
            .map(|(gk, striker)| SaveModel::skill_multiplier(*gk, *striker))
            .collect();
        let spread = mults.iter().cloned().fold(f32::MIN, f32::max)
            - mults.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            spread <= 0.02,
            "an ordinary keeper facing an ordinary striker must resolve the same in \
             every division; got {mults:?} (spread {spread:.3})"
        );
    }

    /// ...but a mismatch inside one division must still be visible.
    /// Parity must not be bought by making every keeper the same keeper,
    /// which is what the earlier flat-multiplier attempts did.
    #[test]
    fn quality_still_separates_keepers_within_a_division() {
        let striker = 0.620;
        let weak = SaveModel::skill_multiplier(0.40, striker);
        let strong = SaveModel::skill_multiplier(0.65, striker);
        assert!(
            strong - weak >= 0.05,
            "a better keeper must still save more against the same striker; \
             weak {weak:.3} strong {strong:.3}"
        );
    }

    /// Geometry still dominates placement: a shot at the limit of the
    /// keeper's reach must be much harder than one hit at him, whoever
    /// is in goal.
    #[test]
    fn stretch_beats_an_elite_keeper_more_than_skill_saves_him() {
        let t = SaveModel::NEUTRAL_THREAT;
        let elite_stretched = SaveModel::save_probability(1.0, 0.0, 1.0, t, 0.0);
        let weak_centred = SaveModel::save_probability(0.0, 0.0, 0.0, t, 0.0);
        assert!(
            elite_stretched < weak_centred,
            "a full-stretch shot must beat an elite keeper more often than a centred one \
             beats a weak keeper; elite {elite_stretched:.3} weak {weak_centred:.3}"
        );
    }

    // ── Post-shot expectation ───────────────────────────────────────

    /// The keeper's expectation must READ the strike. A corner-bound
    /// shot, a rocket and a ball lifted under the bar each have to be
    /// worth more than the tame equivalent, or `goals_prevented` is back
    /// to assuming every shot on target was the same shot — which is the
    /// bug the whole post-shot model exists to remove.
    #[test]
    fn expected_goal_on_target_reads_placement_power_and_height() {
        let tame = SaveModel::expected_goal_on_target(0.0, 4.0, 0.2);
        let corner = SaveModel::expected_goal_on_target(22.0, 4.0, 0.2);
        let rocket = SaveModel::expected_goal_on_target(0.0, 8.0, 0.2);
        let lifted = SaveModel::expected_goal_on_target(0.0, 4.0, 2.2);
        assert!(
            corner > tame,
            "placement must raise the expectation: tame {tame:.3} corner {corner:.3}"
        );
        assert!(
            rocket > tame,
            "power must raise the expectation: tame {tame:.3} rocket {rocket:.3}"
        );
        assert!(
            lifted > tame,
            "height must raise the expectation: tame {tame:.3} lifted {lifted:.3}"
        );
        // Placement is the dominant axis — that is what `STRETCH_PENALTY`
        // (0.58) against `HEIGHT_PENALTY` (0.10) says, and it is what
        // real post-shot models find too.
        assert!(
            corner - tame > lifted - tame,
            "placement must move the expectation more than height; \
             corner {corner:.3} lifted {lifted:.3} tame {tame:.3}"
        );
    }

    /// Bounded by construction, and the rating's difficulty clamp is
    /// derived from these bounds — if they move, `keeper::DIFFICULTY_MAX`
    /// has to move with them.
    #[test]
    fn expected_goal_on_target_stays_within_the_save_models_own_bounds() {
        for lateral in [0.0f32, 5.0, 15.0, 25.9, 26.1, 40.0] {
            for speed in [0.0f32, 3.0, 6.0, 12.0] {
                for height in [0.0f32, 1.0, 2.44, 4.0] {
                    let x = SaveModel::expected_goal_on_target(lateral, speed, height);
                    assert!(
                        (1.0 - SaveModel::MAX_SAVE..=1.0 - SaveModel::MIN_SAVE).contains(&x),
                        "xGoT out of the save model's own range at \
                         lateral={lateral} speed={speed} height={height}: {x:.3}"
                    );
                }
            }
        }
    }

    /// It must not read the KEEPER. Nothing in the signature can carry
    /// him, and that is the point being pinned: the moment the
    /// expectation moves with the man it is measuring, a well-positioned
    /// keeper shrinks his own bar and cancels the advantage his
    /// positioning earned. Sign is measured from the goal CENTRE, so
    /// mirrored placements are worth exactly the same.
    #[test]
    fn expected_goal_on_target_is_symmetric_about_the_goal_centre() {
        for lateral in [1.0f32, 9.0, 18.0, 27.0] {
            let left = SaveModel::expected_goal_on_target(-lateral, 5.0, 1.0);
            let right = SaveModel::expected_goal_on_target(lateral, 5.0, 1.0);
            assert_eq!(
                left.to_bits(),
                right.to_bits(),
                "mirrored strikes must be worth the same at lateral {lateral}"
            );
        }
    }
}
