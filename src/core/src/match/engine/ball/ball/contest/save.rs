//! **The save** — the goalkeeper's physics-layer shot-stopping contest,
//! and the model that prices it.

use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::player::events::players::save_accounting_stats;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::events::EventCollection;
#[cfg(feature = "match-logs")]
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
#[cfg(feature = "match-logs")]
use crate::r#match::player::state::PlayerState;
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
    ///
    /// # 2026-08-20 — 1.03 → 1.12, for the population save rate
    ///
    /// Measured 61.9% saves/on-target against the harness's real ~67%,
    /// stable across four independent 400-match runs (61.9 / 61.7 / 60.0 /
    /// 61.9) and unmoved by the score-reactive regime, so it is a property
    /// of this curve and not of the football around it.
    ///
    /// The arithmetic is direct. Of 2856 on-target shots the physics roll
    /// adjudicated 2597 and passed 1689 — a 65.0% hit rate — and the state
    /// machine added 80, for 1769 saves. Reaching 67% needs 1914, i.e. the
    /// roll at ~70.6%. An ordinary duel is `skill_multiplier` 0.68 against
    /// a mean realised 0.65, so the mean `geometric_base` in force is
    /// 0.956, which back-solves to a mean reach ratio of 0.42; holding
    /// that ratio and asking for 0.706/0.68 = 1.038 gives this value. The
    /// clamp at `MAX_SAVE` does not bind on the way — an elite keeper on a
    /// dead-centre shot reaches 1.12 × 0.82 = 0.92 exactly, so the ceiling
    /// still means what it says.
    ///
    /// ⚠ It goes HERE and not in `SKILL_FLOOR` — see the note there.
    /// Paired with `SHOT_BAR_BASE`, which was carrying the opposite error:
    /// too few shots converting too well.
    const CENTRED_BASE: f32 = 1.12;
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
    ///
    /// ⚠ THAT MEASUREMENT PREDATES THE SAVE-CREDIT LEAK FIX (2026-08-16)
    /// and its central premise was void. Physics saves were being staged
    /// on `Ball::pending_save_credit` and then DELETED by the dead-ball
    /// clear before delivery, which is why the physics roll looked like it
    /// accounted for only half the credited saves. With delivery at 100%
    /// the split is **85% physics** (10720 of 12665), so a cut here now
    /// carries most of the way through — which is what the 0.57 → 0.54
    /// re-measurement below tested.
    ///
    /// **Back to 0.54, deliberately.** With honest accounting the engine
    /// measured saves/on-target at 71.3%, against the harness's stated
    /// real ~67%. 0.54 puts an ordinary duel back on `FLOOR + SLOPE/2` =
    /// **0.68**. The keeper-quality axis is untouched — only the level
    /// moves, and it moves at every division equally because the
    /// multiplier is a contest.
    const SKILL_FLOOR: f32 = 0.54;
    /// Width of the keeper-quality band.
    ///
    /// Mean skill (0.5) lands on `FLOOR + SLOPE/2` = **0.68**, which is
    /// what `an_ordinary_duel_holds_the_calibrated_population_save_rate`
    /// pins.
    ///
    /// Be careful which figure you are chasing: the harness prints "real
    /// ~67%" while `skill_multiplier`'s own header cites "a real ~69-71%
    /// at every level". Both are defensible against real football and
    /// they are not the same target. Between 2026-08-08 and 2026-08-16
    /// the floor sat at 0.57 (an ordinary duel = 0.71) while three
    /// separate prose comments still advertised 0.68, so the code and its
    /// documentation disagreed about which of the two was in force.
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

    /// Ticks a keeper needs, from set, to reach full stretch. ~0.45 s at
    /// 100 engine ticks a second: the reaction plus the dive. Inside that
    /// he is still getting there and only part of his reach is available,
    /// which is why a shot from six yards beats keepers a shot from
    /// twenty-five does not.
    const FULL_STRETCH_TICKS: f32 = 45.0;
    /// Floor on that: even a point-blank strike can hit a raised hand.
    ///
    /// **This is where the close-range population save rate lives**, and it
    /// is the only term that binds there: a shot struck from inside 11 m is
    /// in the air about 14 ticks, so `flight / FULL_STRETCH_TICKS` is well
    /// under the floor and every one of them is priced at exactly this
    /// fraction of his reach. `FULL_STRETCH_TICKS` never enters. Measured,
    /// 71-73% of on-frame shots from that band arrive beyond his reach, so
    /// this constant sets the largest single block of goals in the model.
    ///
    /// ⚠ **RE-DERIVED 0.42 → 0.46, Aug 2026, and the reason is the whole
    /// point.** 0.42 was measured while the universal loose-ball override
    /// was dragging the keeper out of `PreparingForSave` and into
    /// `TakeBall` for any unowned ball within 60 u — which a struck shot
    /// is. So through the last **0.22 s** of every close-range flight he
    /// was *sprinting at the ball* on the `Active` band with none of
    /// `KeeperShotReaction`'s set-keeper cap and no plant cost, and this
    /// floor was calibrated on top of that. `should_force_takeball` now
    /// declines live shots at his own goal — correctly; he sets himself for
    /// them — and the same floor then under-priced him by exactly the
    /// closing that chase used to do: measured at **0.24 m of mean lateral
    /// miss** (3.91 → 4.15 m inside 11 m), which is 9.7% of the 2.48 m his
    /// reach is worth there. 0.42 × 1.097 = 0.46.
    ///
    /// It is pinned by the population save rate rather than by physics —
    /// 0.42 and 0.46 of a 2.5-4.0 m reach are both plausible for a hand at
    /// a point-blank strike — which is the sanctioned place to carry it.
    /// See the note on `SKILL_FLOOR` for why it must NEVER go there
    /// instead. Re-derive from `KEEPER GUARD CENSUS`
    /// (`shots arriving on frame … BEYOND HIS REACH`) and the `< 11 m` row
    /// of `KEEPER BY SHOT RANGE` if the keeper's behaviour during a flight
    /// changes again.
    ///
    /// # 2026-08-20 — 0.46 → 0.54, paired with `base_reach` 20 → 23
    ///
    /// Re-derived when the BEATEN-KEEPER adjudication was removed — see the
    /// save plane in `Ball::try_save_shot`, and `OF_SAVE_BEATEN`, which is
    /// the A/B that produced these numbers. A shot that had already gone
    /// past the keeper used to get a second roll at the goal line with his
    /// LATER position feeding `wedge`, and that second roll was carrying
    /// real save rate. Three 400-match runs an arm: **67.6% saves/on-target
    /// and 2.59 goals/match with it, 64.2% and 2.89 without.**
    ///
    /// Those saves were not real — he was behind the ball, and the catch
    /// then dragged it back onto him, which is the reported bug the removal
    /// exists to fix. The population rate they carried IS real, so it has
    /// to come back from something that is. It comes back from the two
    /// terms that say how much of the goal he covers, because covering
    /// ground he had not covered yet is exactly what the second roll was
    /// silently crediting him with. One 400-match run an arm:
    ///
    /// | arm                              | goals | saves/on-target |
    /// |----------------------------------|-------|-----------------|
    /// | no compensation                  | 2.89  | 64.2%           |
    /// | `base_reach` 22                  | 2.68  | 65.3%           |
    /// | `base_reach` 24                  | 2.55  | 67.6%           |
    /// | this floor 0.52                  | 2.67  | 66.6%           |
    /// | this floor 0.58                  | 2.61  | 67.2%           |
    /// | `CENTRED_BASE` 1.20              | 2.64  | 67.2%           |
    /// | `CENTRED_BASE` 1.28              | 2.23  | 72.8%           |
    /// | `base_reach` 22 + floor 0.52     | 2.69  | 66.2%           |
    /// | **`base_reach` 23 + floor 0.54** | 2.64  | 67.1%           |
    ///
    /// The last two rows are five and four 400-match runs rather than one;
    /// every other row is a single run and is indicative only. The 22 +
    /// 0.52 pair looked right on one run and measured a point light over
    /// five, which is why the landed pair is 23 + 0.54 — and that one is
    /// within half a point and six hundredths of a goal of the 67.6% /
    /// 2.59 the removal cost, i.e. inside the run-to-run floor.
    ///
    /// The PAIR was taken over any single constant for two reasons. Each
    /// moves least that way, and each stays inside its own physical story.
    /// And it is the arm that also reproduces the per-band profile:
    /// `KEEPER BY SHOT RANGE` reads 67/40/21% beyond reach against 67/44/19
    /// before, saved 22/43/61 against 21/39/60. `CENTRED_BASE` restores the
    /// aggregate from the WRONG band — it leaves close-range saves at 17%
    /// against 21% and makes it up at range — which is why the population
    /// lever this file names in `SKILL_FLOOR`'s note is not the one used
    /// here. The note stands for a rate that has drifted on its own; this
    /// is a reach that was being over-credited by a bug.
    /// # 2026-08-24 — 0.54 → 0.38, paired with `base_reach` 23 → 20
    ///
    /// Re-derived when the adjudication stopped reading the keeper's own
    /// misread of the crossing point and started reading the BALL — see the
    /// note at the [`SaveModel::contact`] call in `Ball::try_save_shot`, and
    /// `OF_GK_READ_OFF`, which is the A/B for the read error itself.
    ///
    /// The old test declared **37.7%** of on-frame arrivals beyond his reach
    /// against **20.0%** on the truth, so it was refusing a save roll on
    /// roughly one shot on target in five that he was physically in range
    /// of. Those refusals were not real, and the population rate they
    /// carried is: taking them away read **72.0% saves / 2.25 goals**
    /// against a 67.7% / 2.51 baseline, so it has to come back out of the
    /// two terms that say how much of the goal he covers. Same pair, same
    /// reasoning and the same procedure as the 2026-08-20 re-derivation
    /// below it.
    ///
    /// Three 400-match runs an arm:
    ///
    /// | arm | goals | saves/on-target |
    /// |---|---|---|
    /// | no compensation (23 + 0.54) | 2.25 | 72.0% |
    /// | 23 + 0.36 | 2.34/2.40 | 70.9/70.1% |
    /// | **20 + 0.38** | 2.59/2.49/2.61 | 67.6/68.4/67.3% |
    ///
    /// 2.56 and 67.8% against the 2.51 / 67.7% it replaced — inside the
    /// run-to-run floor on both. ⚠ The floor does most of the work and
    /// `base_reach` almost none: in the plane space `contact` prices, a
    /// shot from inside 11 m is airborne ~14 ticks, so `flight /
    /// FULL_STRETCH_TICKS` sits under the floor and every one of those
    /// shots is priced at exactly this fraction of his reach. Sweeping
    /// `base_reach` alone moved the save rate less than one run of noise.
    const REFLEX_FLOOR: f32 = 0.38;

    /// Share of a comfortable save a median keeper HOLDS.
    ///
    /// The base of the hold/tip/spill split — see the note at the
    /// `hold_difficulty` call site in `try_save_shot` for why that split
    /// stopped being two additive sums. Set so that a median keeper on an
    /// ordinary strike, taken where the population takes them, holds
    /// something over half of what he saves, which is what real
    /// goalkeepers do.
    pub(crate) const HOLD_BASE: f32 = 0.71;
    /// …and how much of it the pace and the stretch take away. At the
    /// measured population difficulty this turns `HOLD_BASE` into the
    /// realised hold rate.
    pub(crate) const HOLD_DIFFICULTY: f32 = 1.30;
    /// Population mean of `scaled_handling` — the peer-shifted value the
    /// hold terms read, NOT the raw 1-20 attribute and NOT 0.5.
    ///
    /// ⚠ Measured, from the `KEEPER HANDLING` block in `dev_match stats`,
    /// which prints it beside the raw mean for exactly this purpose.
    /// `MatchStandard::keeper_shift` already re-centres a keeper against
    /// the goalkeeping standard of his match, so this sits near the middle
    /// by construction — but "near" is not "at", and an uncentred quality
    /// term does not add a skill axis, it silently recalibrates the split.
    /// Re-fit it from the harness if the shift or the scaling moves.
    pub(crate) const POPULATION_HANDLING: f32 = 0.530;
    /// How wide the hold rate opens between the worst pair of hands in the
    /// game and the best. 0.80 puts a 1/20 handler at ~0.60× a median
    /// keeper's hold rate and a 20/20 handler at ~1.40×.
    pub(crate) const HANDS_SPREAD: f32 = 1.10;
    /// Of the saves he cannot hold, the share a median keeper still puts
    /// somewhere safe — round the post or wide of it — rather than back
    /// off his palms into the six-yard box.
    pub(crate) const SAFE_SHARE: f32 = 0.68;
    /// Ceiling on the angle projection. Physically it runs away as the
    /// keeper closes on the ball, and past a certain point the model stops
    /// describing a save and starts describing a block.
    const MAX_PROJECTION: f32 = 2.0;

    /// **The angle the keeper is covering, priced properly.**
    ///
    /// # The defect this replaces
    ///
    /// Both save paths asked one question: how far is the keeper's `y`
    /// from the `y` at which the ball crosses the goal line. That is the
    /// right question for a keeper standing ON the line, and the wrong one
    /// for every other keeper, because a keeper off his line is *between*
    /// the ball and the goal — he covers a WEDGE, and the same dive covers
    /// proportionally more of the mouth the further out he is. Priced flat
    /// at the goal line, every metre he advanced to narrow the angle
    /// counted as a metre of error instead.
    ///
    /// The consequence was total and invisible: the strategy that
    /// maximised saves under it was to **stand dead centre on the goal
    /// line and never move**. A median keeper reaches 26u and the goal is
    /// 29u to a post, so a keeper who never leaves the middle of his line
    /// covers 90% of the mouth by width, while the keeper who plays the
    /// angle correctly — which is what `KeeperRestPosition` builds, and
    /// what real keepers do — reads as out of position by construction.
    /// Measured over 60 matches: **22% of every shot that arrived inside
    /// the frame never reached the save roll at all, the keeper a mean
    /// 6.10 m from the crossing point** — the reported "he is on the other
    /// side of the goal and it goes in". No amount of state-machine work
    /// could move that number, because the state machine was being
    /// punished for getting it right.
    ///
    /// # The model
    ///
    /// Two terms, and they pull AGAINST each other — which is exactly why
    /// a keeper's decision to come out is a decision at all rather than
    /// free:
    ///
    /// * **Projection.** From the striker's eye the keeper's body subtends
    ///   an angle; extended to the goal line that shadow is `r` times his
    ///   real width, where `r = ball_depth / (ball_depth − keeper_depth)`
    ///   measures the two along the goal-to-goal axis. Both where he is
    ///   covering and how much he covers scale by it.
    /// * **Time.** Coming to meet it costs him the flight time he needs to
    ///   get to full stretch. The ball reaches a keeper six metres out
    ///   sooner than one on his line, so the closer he comes the less of
    ///   his reach he can actually deploy.
    ///
    /// A keeper on his line gets `r = 1` and a full flight to read it, so
    /// this is **bit-identical to the old test for him** — the calibration
    /// it was tuned on is preserved, and only the behaviour that was being
    /// wrongly punished changes.
    pub(crate) fn wedge(
        struck_from: Vector3<f32>,
        ball_speed: f32,
        keeper: Vector3<f32>,
        base_reach: f32,
        goal_x: f32,
        goal_line_y: f32,
    ) -> (f32, f32) {
        let ball_depth = (struck_from.x - goal_x).abs();
        let keeper_depth = (keeper.x - goal_x).abs();
        let gap = ball_depth - keeper_depth;

        // How long the ball is in the air before it reaches HIM — the time
        // he has to extend, not the time it takes to reach the goal.
        let flight =
            ((struck_from - keeper).magnitude() / ball_speed.max(0.05)) / Self::FULL_STRETCH_TICKS;
        let ready = flight.clamp(Self::REFLEX_FLOOR, 1.0);

        // Level with the ball or beyond it: he has been passed, there is
        // no wedge left to cover and only his own body is in the way.
        if gap <= 1.0 || ball_depth <= 1.0 {
            return ((keeper.y - goal_line_y).abs(), base_reach * ready);
        }

        let projection = (ball_depth / gap).clamp(1.0, Self::MAX_PROJECTION);
        // Where his body shadows the goal line, seen from the strike.
        let shadow_y = struck_from.y + (keeper.y - struck_from.y) * projection;
        (
            (shadow_y - goal_line_y).abs(),
            base_reach * projection * ready,
        )
    }

    /// **The same reach, priced where the CONTACT happens.**
    ///
    /// [`Self::wedge`] answers an *ex ante* question — how much of the goal
    /// mouth does his body shadow, seen from the striker — and that is the
    /// right question for a keeper deciding whether to leave his feet for a
    /// ball whose line he has not read yet. It is the wrong one for the
    /// adjudication, which happens when the ball is level with him: by then
    /// there is no shadow left to argue about, only a gap between his hands
    /// and the ball.
    ///
    /// # This is not a recalibration
    ///
    /// For a ball travelling in a straight line the two are the SAME TEST.
    /// `wedge` magnifies both the gap and the reach by the identical
    /// projection `P`, so the ratio it feeds `geometric_base` — and the
    /// `lateral > reach` gate — are unchanged by dividing it out:
    ///
    /// ```text
    /// shadow_y − goal_line_y = (keeper.y − ball_y_at_his_plane) · P
    /// reach                  = base_reach · ready · P
    /// ```
    ///
    /// The credit for narrowing the angle survives too, and for the reason
    /// `wedge` documents: coming out compresses the band of `y` the ball can
    /// be in when it reaches him, so a fixed reach covers more of it.
    ///
    /// # Where they part company, and why the plane wins there
    ///
    /// A ball that BENDS. `ShotTarget`'s sidespin is solved so the Magnus
    /// force is worth the whole `curl_units` the launch line was offset by,
    /// so the crossing at the goal line is not the linear extension of the
    /// ball's line through the keeper — and a shot that passed within a
    /// metre of his hands could still be scored against a crossing point
    /// three metres away, because it bent after it had already beaten him.
    /// Measured: after the adjudication moved onto the ball's real crossing,
    /// **7.8% of the shots that passed within a metre of him were still
    /// called out of his reach**. Priced here, the ball is where it is.
    pub(crate) fn contact(
        struck_from: Vector3<f32>,
        ball_speed: f32,
        keeper: Vector3<f32>,
        base_reach: f32,
        ball_y_at_his_plane: f32,
    ) -> (f32, f32) {
        // Identical to `wedge`'s: the time he has to extend is a property
        // of the flight to HIM, and it does not care which space the gap
        // is measured in.
        let flight =
            ((struck_from - keeper).magnitude() / ball_speed.max(0.05)) / Self::FULL_STRETCH_TICKS;
        let ready = flight.clamp(Self::REFLEX_FLOOR, 1.0);
        ((keeper.y - ball_y_at_his_plane).abs(), base_reach * ready)
    }
    /// Speed at which a strike starts to beat a keeper on pace alone, in
    /// game units per tick (1 u/tick = 12.5 m/s). Below this he has time
    /// to set himself and how hard it was hit does not matter.
    const HARD_STRUCK: f32 = 1.2;
    /// Speed above `HARD_STRUCK` at which the curve reaches half its
    /// ceiling. `MAX_SHOT_VELOCITY` is 3.2, so the hardest strike the
    /// engine can produce sits on the half-way point and the curve
    /// saturates smoothly beyond it rather than clipping.
    const SPEED_HALF_SPAN: f32 = 2.0;
    /// Width of the pace axis for a keeper with no reflexes at all.
    const SPEED_SPREAD: f32 = 0.44;
    /// Where an ORDINARY strike sits on that curve — measured as the mean
    /// speed of shots arriving at the save roll (`GOALKEEPER ACTION
    /// CENSUS`: **2.62 u/tick**, 13 687 shots over 200 matches at L14),
    /// pushed through `pace_position`. Subtracting it is what makes the
    /// term a SPREAD rather than a tax: the average shot in the game costs
    /// the keeper nothing, a rocket costs him, and a tame effort hands him
    /// a little back.
    ///
    /// **Re-derive it from the census whenever the strike model moves.**
    /// It is the whole of this term's calibration: set 0.09 too low it
    /// took saves/on-target from 77.3% to 72.6% on its own, with the
    /// shape of the curve unchanged.
    const ORDINARY_PACE: f32 = 0.4169;

    /// Position of a strike on the pace curve, 0..1 and strictly
    /// increasing in speed at every input. Kept separate so
    /// `ORDINARY_PACE` above can be quoted in the same units.
    #[inline]
    fn pace_position(speed: f32) -> f32 {
        let excess = (speed - Self::HARD_STRUCK).max(0.0);
        excess / (excess + Self::SPEED_HALF_SPAN)
    }

    /// Speed an ORDINARY shot arrives at, in game units per tick —
    /// measured off `GOALKEEPER ACTION CENSUS` (2.63 u/tick over 14 068
    /// shots, 200 matches at L14). The anchor every "how hard was it hit"
    /// term in the goalkeeping model is centred on, here and in the
    /// goalkeeper states.
    pub(crate) const ORDINARY_STRIKE: f32 = 2.63;
    /// Half-width of the strike-speed band. `MAX_SHOT_VELOCITY` is 3.2 and
    /// a ball sheds pace in flight, so ±1.4 covers everything from a
    /// scuffed effort to a piledriver.
    const STRIKE_SPAN: f32 = 1.4;

    /// How much harder than an ORDINARY shot this one was, −1..1.
    ///
    /// **Centred, and that is the point.** Every keeper site that reads
    /// shot power feeds it into an additive difficulty sum, so an
    /// uncentred 0..1 term does not just restore the power axis, it makes
    /// every shot in the game harder and drops the population save rate
    /// with it. A signed deviation spreads the difficulty around an
    /// ordinary strike instead: a rocket is harder, a scuffed one easier,
    /// and the average shot is exactly what it was.
    #[inline]
    pub(crate) fn strike_power(speed: f32) -> f32 {
        ((speed - Self::ORDINARY_STRIKE) / Self::STRIKE_SPAN).clamp(-1.0, 1.0)
    }

    /// How much of the geometric ceiling the ball's PACE takes away —
    /// negative for a shot struck softer than average.
    ///
    /// ⚠ **UNITS — this was inert.** Both call sites computed
    /// `(speed − 3.0).max(0) * 0.08`, written when a shot could leave the
    /// boot at 5.6 u/tick. `MAX_SHOT_VELOCITY` is now **3.2**, and a ball
    /// sheds pace in flight, so `speed − 3.0` was **at most 0.2** and the
    /// penalty at most **0.016** — three decimal places below anything
    /// that could move a save. How hard a shot was struck had, in
    /// practice, no effect on whether the keeper saved it, which is why a
    /// powerful finisher gained nothing from his power and a keeper with
    /// elite reflexes gained nothing from his.
    ///
    /// Rebuilt as a saturating curve **centred on an ordinary strike**,
    /// which is what keeps it calibration-neutral: restoring an axis that
    /// had gone flat must not also move the population save rate, and a
    /// one-sided penalty does (measured, an uncentred version took
    /// saves/on-target 77.3% → 68.8% in one step). Monotone at any input
    /// because the post-shot expectation is pinned on that — a clamped
    /// line made a rocket and a firm shot worth exactly the same.
    ///
    /// `reflexes01` is the keeper's normalised reflexes: a quick keeper
    /// gives away half as much to pace as a slow one.
    #[inline]
    pub(crate) fn speed_penalty(speed: f32, reflexes01: f32) -> f32 {
        (Self::pace_position(speed) - Self::ORDINARY_PACE)
            * Self::SPEED_SPREAD
            * (1.0 - reflexes01.clamp(0.0, 1.0) * 0.5)
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
    /// An equal-quality duel returns `SKILL_FLOOR + SKILL_SLOPE/2` =
    /// **0.68**, so this is calibration-neutral at the mean while
    /// removing the drift.
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
        let speed_penalty = Self::speed_penalty(speed, Self::REFERENCE_REFLEXES);
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
    /// Goalkeeper save check. Runs during shot flight: when the ball
    /// approaches the goal line and the defending keeper's body is
    /// within reach of the shot's trajectory, roll a skill-weighted
    /// save. The keeper state machine's `is_catch_successful` path
    /// timed saves to player-state ticks that didn't line up with the
    /// ball's physics step — saves fired too early or too late, and
    /// shots past the keeper cleared into the net. A physics-level
    /// save runs every ball tick with fresh ball position and commits
    /// the ball to the keeper at the moment of contact.
    /// Diagnostic switch: with `OF_SAVE_AT_LINE` set, the save resolves
    /// when the ball reaches the GOAL LINE again, as it did before the
    /// contact point was moved onto the keeper.
    ///
    /// The A/B control for that change. It moves which of the two layered
    /// save paths adjudicates a shot — the physics roll or the keeper state
    /// machine's own catch — and therefore the population save rate, so
    /// "what did this cost?" cannot be answered by reading the diff. Same
    /// pattern and purpose as `OF_SHAPE_OFF` / `OF_KEEPER_SERVO`; read once
    /// per process. Debug infrastructure — do not remove.
    fn save_at_line() -> bool {
        static AT_LINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *AT_LINE.get_or_init(|| std::env::var("OF_SAVE_AT_LINE").is_ok())
    }

    /// Diagnostic switch: with `OF_SAVE_BEATEN` set, a keeper the ball has
    /// already gone past gets his old second adjudication at the goal line.
    ///
    /// The A/B control for that removal. It decides whether a shot that has
    /// beaten the keeper can still be saved, so it moves the population save
    /// rate and "what did this cost?" cannot be answered by reading the
    /// diff. Same pattern and purpose as [`Self::save_at_line`]. Debug
    /// infrastructure — do not remove.
    fn save_when_beaten() -> bool {
        static BEATEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *BEATEN.get_or_init(|| std::env::var("OF_SAVE_BEATEN").is_ok())
    }

    /// Where the ball is projected to cross `goal_x`, as `(y, z)`.
    ///
    /// The real thing: [`Ball::ballistic_crossing`] integrates the same
    /// drag-then-gravity-then-step sequence the physics runs, so this and
    /// the flight cannot drift apart. It replaced a closed-form
    /// extrapolation of the ball's current state, which is exact on the
    /// lateral axis (drag is isotropic, so `vy/vx` survives it) and biased
    /// high on the vertical one.
    ///
    /// **This is the TRUTH of the shot**, and the distinction matters:
    /// `ShotTarget::goal_line_y` is what the keeper BELIEVES, and is
    /// deliberately wrong. Anything asking "was he actually in the way"
    /// must read this; anything asking "where is he going" must read his
    /// belief. See `try_save_shot`.
    ///
    /// Falls back to the ball's current position when it is not travelling
    /// toward that line, because then there is no crossing to project.
    pub(crate) fn projected_crossing(&self, goal_x: f32) -> (f32, f32) {
        if Self::save_at_line() {
            return (self.position.y, self.position.z);
        }
        match Ball::ballistic_crossing(self.position, self.velocity, self.spin, goal_x) {
            Some((y, z, _)) => (y, z),
            None => (self.position.y, self.position.z),
        }
    }

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

        let (goal_x, goal_y) = match shot_target.defending_side {
            PlayerSide::Left => (context.goal_positions.left.x, context.goal_positions.left.y),
            PlayerSide::Right => (
                context.goal_positions.right.x,
                context.goal_positions.right.y,
            ),
        };

        // Find the defending keeper. Read BEFORE the arrival window,
        // because he is what the window is measured against — see
        // `save_plane_x` below.
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

        // **The save resolves where the KEEPER is, not where the goal is.**
        //
        // This window used to be measured against the goal line, and that
        // is the wrong plane for every keeper who is not standing on it.
        // The reach model (`SaveModel::wedge`) correctly prices a keeper
        // off his line as covering a WEDGE — he is between the ball and the
        // goal, so the same dive covers more of the mouth — but the
        // resolution then waited for the ball to reach the LINE, several
        // ticks after it had already gone past him. Measured over 40
        // matches (`SaveContactDiag`), the point the ball turned at sat a
        // mean **3.5 m from the man credited with turning it, 2.2 m of it
        // along the goal-to-goal axis**, and **half of all shot
        // resolutions had nobody within 2.5 m at all**. On screen that is a
        // ball bouncing off empty space on the goal line and flying back
        // out to a follow-up shot, which is exactly how it was reported.
        //
        // The plane is his, clamped so it is never behind the goal line.
        let keeper_plane = match shot_target.defending_side {
            PlayerSide::Left => keeper.position.x.max(goal_x),
            PlayerSide::Right => keeper.position.x.min(goal_x),
        };
        // **And it STAYS his once the ball is past him.**
        //
        // It used to fall back to the goal LINE for a keeper the ball had
        // already gone by, on the reading that the line is his last chance.
        // It is not a chance at all — he is behind the ball — and what the
        // fallback actually bought was a second adjudication metres up the
        // pitch from him. The reach test below returns WITHOUT latching
        // `save_rolled` (deliberately: it is a positioning outcome, not a
        // roll), so a shot that beat him at his own plane had the window
        // re-open at the goal line and got rolled again, this time with his
        // position — which has moved since — feeding `wedge`. When that
        // second roll came off, the catch branch wrote the ball onto his
        // coordinate, and a ball a metre from crossing the line teleported
        // four and a half metres BACKWARDS into his gloves. That is the
        // reported bug, verbatim: *"it flies through his hands and into the
        // goal, and then instantly flips back into the goalkeeper's
        // hands."* Measured off recorded matches: 114 of them in 3 000 goal
        // clips, up to 4.7 m of pull-back, 27 of them beyond 3 m.
        //
        // Measuring against his own plane bounds it without a tolerance
        // constant. The arrival window below is ±2.5 ticks of flight wide
        // and centred on HIM, so the ball can be at most a couple of ticks
        // past him when the roll fires — a trailing hand, which is real —
        // and once it is beyond that the window closes and stays closed.
        let beaten = (keeper_plane - self.position.x) * (goal_x - self.position.x).signum() <= 0.0;
        let save_plane_x = if Self::save_at_line() || (beaten && Self::save_when_beaten()) {
            goal_x
        } else {
            keeper_plane
        };

        let dist_to_plane = (self.position.x - save_plane_x).abs();
        let ball_vx = self.velocity.x.abs().max(0.5);
        if dist_to_plane > ball_vx * 2.5 {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::SAVE_TICKS_OUT_OF_REACH.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // The ball has reached him. Anything off the frame is a MISS, and
        // the shot is over — retire it.
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
        //
        // Measured at the projected CROSSING rather than at the ball's
        // current position. The two are the same thing when the keeper is
        // on his line — which is the case the whole save model was
        // calibrated on, so this is bit-identical there — and they part
        // company by exactly the distance the window above now opens
        // early. Reading the ball's live position instead would let a shot
        // that is on-frame two metres out but climbing over the bar reach
        // the save roll, which is not a change to when a save happens but
        // a change to what counts as a shot on target.
        let (frame_y, frame_z) = self.projected_crossing(goal_x);
        // Does the keeper's own idea of the arrival height — the strike-time
        // arc every one of his height decisions reads — agree with the ball?
        // See `ShotHeightDiag`.
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::ShotHeightDiag::note(shot_target.goal_line_z, frame_z, GOAL_HEIGHT);
        let off_frame_high = frame_z > 2.8;
        let off_frame_wide = (frame_y - goal_y).abs() > GOAL_WIDTH + 1.0;
        if off_frame_high || off_frame_wide {
            #[cfg(feature = "match-logs")]
            if crate::r#match::engine::ball::ball::frame_trace::FrameTrace::captures_misses() {
                crate::r#match::engine::ball::ball::frame_trace::FrameTrace::open(format!(
                    "MISS {} crossing y={:+.1}u past the post, z={:.2} m; gk at gap {:.1}u ({:.2} m)",
                    if off_frame_wide { "WIDE" } else { "OVER" },
                    (frame_y - goal_y).abs() - GOAL_WIDTH,
                    frame_z,
                    (keeper.position.x - self.position.x)
                        .hypot(keeper.position.y - self.position.y),
                    (keeper.position.x - self.position.x)
                        .hypot(keeper.position.y - self.position.y)
                        * 0.125,
                ));
            }
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
        // the shot. The keeper was resolved at the top, because the
        // arrival window is measured against him.

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
        // …each measured against the standard of goalkeeping in this
        // match, so a keeper is compared with the football he is playing
        // in rather than with a fixed 1-20 scale. See `MatchStandard` and
        // the note at `base_reach` below for the measurement that forced
        // it. Applied here, at the normalisation, so the reach, the pace
        // penalty and the catch/parry split all agree about how good he
        // is.
        let gk_shift = MatchStandard::keeper_shift(context);
        let peer = |v: f32| (v - gk_shift).clamp(0.0, 1.0);
        let scaled_handling = peer(((handling - 1.0) / 19.0).max(0.0));
        let scaled_reflexes = peer(((reflexes - 1.0) / 19.0).max(0.0));
        let scaled_agility = peer(((agility - 1.0) / 19.0).max(0.0));
        let scaled_concentration = peer(((concentration - 1.0) / 19.0).max(0.0));
        // …and the two that decide what an UNHELD ball does. `punching` had
        // no path to the save outcome at all — it reached only
        // `GoalkeeperSkillProfile::parry_control` and the punch decision in
        // `GoalkeeperPunchingState`, neither of which runs on an ordinary
        // shot — so the attribute a keeper is bought for when he cannot
        // hold anything did nothing on the shots he could not hold.
        let punching = effective_skill(keeper, keeper.skills.goalkeeping.punching, tech_ctx);
        let strength = effective_skill(keeper, keeper.skills.physical.strength, expl_ctx);
        let scaled_punching = peer(((punching - 1.0) / 19.0).max(0.0));
        let scaled_strength = peer(((strength - 1.0) / 19.0).max(0.0));

        // Diving reach in game units. Field is 840u = 105m, so 1u = 0.126m
        // (half-goal 29u = 3.66m matches real 3.66m). Every keeper, even a
        // youth-level one, can physically dive across most of the goal
        // — skill determines whether they *catch* the ball, not whether
        // they can reach it. The previous 10u floor made corner shots
        // literally unreachable for weak keepers, so blowouts in youth
        // leagues (hnd=1, ref=1) pushed matches to 10+ goals. New reach:
        //   skills 1   → 23u (2.9m, standing dive — can touch the post)
        //   skills 10  → 29u (3.6m, covers most of the goal)
        //   skills 20  → 35u (4.4m, elite full-stretch — beyond the post)
        //
        // Intercept 20 → 23 on 2026-08-20, half of the re-derivation the
        // removal of the beaten-keeper adjudication forced. The other half
        // is `SaveModel::REFLEX_FLOOR`, whose doc carries the measurement
        // table and the reasoning for both. The SLOPE is untouched: the
        // spread between the worst keeper alive and the best is still 12u,
        // so nothing about the keeper-quality axis moves.
        //
        // ── …AND THE REACH IS MEASURED AGAINST THIS MATCH ──────────────
        //
        // `SaveModel::skill_multiplier` was rewritten as a CONTEST for
        // exactly one reason, and its own note states it: "squads scale
        // with the division, so an absolute bar makes a lower-division
        // keeper worse without making the strikers he faces any less
        // dangerous". The geometry was left absolute, and it carries the
        // same bias — the half-goal is a fixed 29u whatever league you
        // are in, so a reach that grows with the keeper's own attributes
        // covers a steadily larger share of it as everyone improves.
        //
        // Measured, `dev_match levels 300 4 20 2`, equal squads: saves
        // per shot on target ran **56.6% at level 4 to 70.7% at level
        // 18**, against a real ~68% at every level of every pyramid — and
        // the contest multiplier was already flat across that whole
        // sweep, so what remained was the geometry.
        //
        // `MatchStandard::keeper_shift` reads the two attributes against
        // the goalkeeping standard of the match. Zero at the calibration
        // division, so the 23/8/4 numbers above are untouched where they
        // were fitted; within a division the agile keeper still reaches
        // further than the heavy-legged one.
        // Intercept 23 → 20 on 2026-08-24, half of the re-derivation the
        // move onto the ball's real crossing forced. The other half is
        // `SaveModel::REFLEX_FLOOR`, whose doc carries the sweep and the
        // reasoning for the pair. The SLOPE is untouched again: the spread
        // between the worst keeper alive and the best is still 12 u, so
        // nothing about the keeper-quality axis moves.
        let base_reach = 20.0 + scaled_agility * 8.0 + scaled_reflexes * 4.0;
        // …and how much of the GOAL that reach is worth from where he is
        // standing. See `SaveModel::wedge`: measuring the gap flat at the
        // goal line charged him for every metre he came to narrow the
        // angle. Identical to the old test for a keeper on his line.
        //
        // ── SCORED AGAINST THE BALL, NOT AGAINST HIS READ ──────────────
        //
        // This used to pass `shot_target.goal_line_y`, and that number is
        // the keeper's BELIEF: it carries `KEEPER_PLACEMENT_READ`'s jitter,
        // the 35% of the curl he does not read, and — on a deflection — the
        // whole pre-deflection line. All three exist so that placement,
        // curl and deflection beat a keeper, and all three are supposed to
        // beat him by putting his BODY in the wrong place. Feeding the same
        // number to the adjudication made them beat him by decree instead,
        // and the two are not the same thing.
        //
        // Measured over 250 matches before this changed: the number the
        // save was scored against sat a mean **1.94 m** from the ball's
        // real crossing; **37.7%** of on-frame arrivals were declared
        // beyond his reach against **20.0%** on the truth, so **19.6% of
        // every shot on target was a shot he was physically in range of
        // that got no save roll at all**; and of the **half** of all
        // arrivals that passed within a metre of him, **15.4%** were
        // adjudicated as having beaten him. On screen that is a ball going
        // straight past a keeper who does not react — reported as "he
        // dives the wrong way" and "he doesn't save long shots", and
        // invisible to every save counter in the harness because the
        // keeper's steering and the scorer shared the mistake.
        //
        // The ball's own position is what it is priced against, through
        // [`SaveModel::contact`] rather than `wedge`: the same test for a
        // straight ball — the projection divides out of the ratio — and the
        // honest one for a bending shot, which by the time this runs has
        // already passed him. See its note.
        //
        // The off-frame test two blocks up reads the same live ballistics
        // (`projected_crossing`), so a shot can no longer be "on frame" by
        // one number and adjudicated by another two metres away.
        //
        // ⚠ The DECISION sites stay on the belief, deliberately:
        // `KeeperShotDive::should_launch`, `KeeperShotReaction::crossing_y`
        // and `crossing_at` all read `goal_line_y`, because a keeper
        // commits on what he has read. Only the question "was he in the
        // way" reads the ball.
        //
        // The population save rate this moved was re-derived into
        // `base_reach` and `SaveModel::REFLEX_FLOOR` — see their notes.
        let (lateral_error, reach) = SaveModel::contact(
            shot_target.struck_from,
            self.velocity.norm(),
            keeper.position,
            base_reach,
            self.position.y,
        );
        // How well his positioning served him on THIS shot, split by how
        // good a reader of the game he is. Recorded before the reach test
        // so both outcomes are in the same denominator.
        #[cfg(feature = "match-logs")]
        {
            // …and the same reach scored against what he BELIEVED, which is
            // what this used to be adjudicated on. Kept as the standing
            // regression guard: the two columns it prints must stay apart
            // (his read really is wrong, on purpose) while the "within a
            // metre of him and called beyond his reach" row stays near
            // zero. The belief is brought into the keeper's own plane the
            // same way `KeeperShotDive::crossing_at` does, so the two
            // columns are the same measurement of two different balls.
            // See `KeeperCommitDiag`.
            {
                let span = shot_target.struck_from.x - goal_x;
                let travelled = if span.abs() < 1.0 {
                    1.0
                } else {
                    ((shot_target.struck_from.x - keeper.position.x) / span).clamp(0.0, 1.0)
                };
                let believed_y = shot_target.struck_from.y
                    + (shot_target.goal_line_y - shot_target.struck_from.y) * travelled;
                let (read_error, _) = SaveModel::contact(
                    shot_target.struck_from,
                    self.velocity.norm(),
                    keeper.position,
                    base_reach,
                    believed_y,
                );
                crate::mid_run_diag::KeeperCommitDiag::note_physical(
                    self.position.y - keeper.position.y,
                    lateral_error > reach,
                );
                crate::mid_run_diag::KeeperCommitDiag::note_arrival(
                    believed_y - self.position.y,
                    read_error,
                    lateral_error,
                    reach,
                );
            }
            let m = &keeper.skills.mental;
            let read = (m.positioning + m.anticipation + m.decisions + m.concentration) / 80.0;
            let (n_slot, sum_slot) = if read >= 0.60 {
                (17, 18)
            } else if read <= 0.45 {
                (19, 20)
            } else {
                (usize::MAX, usize::MAX)
            };
            crate::mid_run_diag::KeeperGuardDiag::note(n_slot);
            crate::mid_run_diag::KeeperGuardDiag::add(sum_slot, (lateral_error * 100.0) as u64);
            // …and the same split by whether he had already left his feet
            // for it. See `KeeperShotDive`: a keeper who dives during the
            // flight ought to be NEARER the crossing point than one still
            // shuffling toward it, and if he is not the dive is aimed at
            // the wrong place.
            let diving = keeper.state == PlayerState::Goalkeeper(GoalkeeperState::Diving);
            let (arrivals, error) = if diving { (22, 23) } else { (24, 25) };
            crate::mid_run_diag::KeeperGuardDiag::note(arrivals);
            crate::mid_run_diag::KeeperGuardDiag::add(error, (lateral_error * 100.0) as u64);
            // …and the same arrival split by the KEEPER, so quality is
            // visible in whether he goes for it as well as in whether he
            // stops it. See `KeeperQualityDiag`; needs `SQUAD_SPREAD` set or
            // every keeper in the run is the same player.
            {
                use crate::mid_run_diag::KeeperQualityDiag as Q;
                let skill = sc::gk_shot_stopping(keeper, minute_for_effective);
                let band = Q::band(skill);
                Q::note(band, 0);
                if lateral_error > reach {
                    Q::note(band, 1);
                }
                if diving {
                    Q::note(band, 2);
                }
                Q::add(band, 4, (skill * 1000.0).max(0.0) as u64);
            }
            // …and the same arrival split by HOW FAR IT WAS STRUCK FROM.
            // "Beyond his reach" means one thing at six yards and the
            // opposite at twenty-five. See `KeeperRangeDiag`.
            {
                use crate::mid_run_diag::KeeperRangeDiag as R;
                let strike = (shot_target.struck_from
                    - Vector3::new(goal_x, goal_y, shot_target.struck_from.z))
                .magnitude();
                let band = R::band(strike);
                R::note(band, 0);
                if lateral_error > reach {
                    R::note(band, 1);
                }
                R::add(band, 2, (lateral_error * 100.0).max(0.0) as u64);
                R::add(band, 3, (reach * 100.0).max(0.0) as u64);
                if diving {
                    R::note(band, 6);
                }
                R::add(
                    band,
                    7,
                    ((shot_target.struck_from - self.position).magnitude()
                        / self.velocity.norm().max(0.05)
                        * 10.0)
                        .max(0.0) as u64,
                );
            }
        }
        if lateral_error > reach {
            // He was not there to be beaten. Counted separately from the
            // saves he loses on the roll — this is a POSITIONING outcome,
            // and lumping the two together is what let the keeper's
            // whereabouts during the build-up go unmeasured. See
            // `KeeperGuardDiag`.
            #[cfg(feature = "match-logs")]
            {
                crate::mid_run_diag::KeeperGuardDiag::note(10);
                crate::mid_run_diag::KeeperGuardDiag::add(11, (lateral_error * 100.0) as u64);
            }
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::KeeperGuardDiag::note(12);

        // Base save chance. Centered shot ~0.88; full-stretch ~0.30.
        // Skill handles the rest; this curve is purely geometry.
        let reach_ratio = (lateral_error / reach).clamp(0.0, 1.0);

        // Shot-speed penalty — elite shots beat keepers more often. Shared
        // with the post-shot expectation so the two cannot drift apart;
        // see `SaveModel::speed_penalty` for why the old inline form was
        // returning ~0.01 for every shot in the game.
        let ball_speed = self.velocity.norm();
        #[cfg(feature = "match-logs")]
        {
            crate::mid_run_diag::KeeperActionDiag::note(9);
            crate::mid_run_diag::KeeperActionDiag::add(10, (ball_speed * 100.0).max(0.0) as u64);
        }
        let speed_penalty = SaveModel::speed_penalty(ball_speed, scaled_reflexes);

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
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::{KeeperQualityDiag as Q, KeeperRangeDiag as R};
            let strike = (shot_target.struck_from
                - Vector3::new(goal_x, goal_y, shot_target.struck_from.z))
            .magnitude();
            R::note(R::band(strike), 4);
            Q::note(Q::band(skill), 3);
        }

        let shot_power_norm = SaveModel::strike_power(ball_speed);
        let reach_stretch = reach_ratio;
        // **What he DOES with the save, and HANDLING is what decides it.**
        //
        // Held, tipped round the post, or spilled back into the six-yard
        // box: three endings a viewer can tell apart, and the only three
        // that say whether the man in goal has a pair of hands. This used
        // to be two additive sums with clamps —
        //
        //   catch  = (0.12 + handling·0.26 + positioning·0.10 + …
        //             − power·0.18 − stretch·0.18).clamp(0.04, 0.62)
        //   safe   = (0.20 + reflexes·0.10 + handling·0.07 + …).clamp(0.12, 0.52)
        //
        // — and both halves of that were wrong in the same way.
        //
        // **The clamp-floor ratchet.** An additive skill term inside a
        // floored expression is RECTIFIED: on a hard shot taken at full
        // stretch the sum went negative and every keeper alive landed on
        // the 0.04 floor together, so exactly where handling should matter
        // most it did not enter at all. The project rule is that a skill
        // term into a floored expression must be multiplicative; see the
        // corner `att_win` note in the skill audit.
        //
        // **And the population was wrong.** Measured over 300 matches at
        // `SQUAD_SPREAD=3`: the WORST handler in the game held 22% of his
        // saves and spilled 45% of them back into danger, the best held
        // 36% and spilled 27%. Real keepers hold the majority of what they
        // save and put almost none of it back into the six-yard box.
        //
        // Rebuilt as a difficulty and a multiplier, the shape the rest of
        // this file uses. `hold_difficulty` is the pace on it and how far
        // out on the edge of his reach he took it — both already signed
        // against an ordinary strike. `hands` is CENTRED on the measured
        // population mean of the same scaled value the term reads, so a
        // median keeper multiplies by exactly 1.0 and the calibrated split
        // is untouched while the band opens around it.
        let hold_difficulty = (shot_power_norm * 0.55 + reach_stretch * 0.45).clamp(0.0, 1.0);
        let hands = (1.0
            + (scaled_handling - SaveModel::POPULATION_HANDLING) * SaveModel::HANDS_SPREAD
            + (scaled_concentration - 0.5) * 0.24)
            .max(0.0);
        let catch_prob =
            (SaveModel::HOLD_BASE * (1.0 - hold_difficulty * SaveModel::HOLD_DIFFICULTY) * hands)
                .clamp(0.05, 0.95);
        // Of the ones he cannot hold, the share he still puts somewhere
        // safe — round the post, or wide of it — rather than back off his
        // palms into the danger area. Expressed as a SHARE of the ones he
        // does not hold, so the three outcomes are a genuine decomposition
        // and improving his hands cannot quietly increase the spills.
        // PUNCHING leads it, and that is the point: getting a ball he
        // cannot catch AWAY from goal is a strength-and-technique job, not
        // a reflex one. Handling still carries a share — a keeper with a
        // pair of hands steers a parry rather than being hit by it — and
        // strength decides whether it clears the six-yard box at all.
        // Multiplicative and centred, for the same reason `hands` is.
        let safe_share = (SaveModel::SAFE_SHARE
            * (1.0
                + (scaled_punching - SaveModel::POPULATION_HANDLING) * 0.55
                + (scaled_handling - SaveModel::POPULATION_HANDLING) * 0.35
                + (scaled_strength - 0.5) * 0.20
                + (scaled_reflexes - 0.5) * 0.20))
            .clamp(0.15, 0.95);
        let safe_parry_prob = (1.0 - catch_prob) * safe_share;

        let keeper_id = keeper.id;
        let keeper_pos = keeper.position;
        let keeper_team = keeper.team_id;
        let keeper_side = keeper.side;

        let outcome_roll = context.rng.unit_f32();
        let p_catch = catch_prob;
        let p_safe = (catch_prob + safe_parry_prob).min(0.92);

        // How far the point the ball is about to turn at is from the only
        // man who could have turned it. See `SaveContactDiag`.
        #[cfg(feature = "match-logs")]
        let (contact_gap, contact_along, contact_across) = {
            let d = self.position - keeper_pos;
            // 8 units to the metre — the vertical axis is metric, the two
            // horizontal ones are the 0.125 m grid.
            (
                (d.x * d.x + d.y * d.y + (d.z * 8.0) * (d.z * 8.0)).sqrt(),
                d.x,
                d.y,
            )
        };
        #[cfg(feature = "match-logs")]
        let contact_height = self.position.z;

        // The height the contact happened at, kept. This used to be
        // `self.position.z = 0.0` for every outcome, which put a shot saved
        // at head height on the grass on the frame it was saved — the ball
        // dropped a metre and a half in one tick with nothing to explain
        // it. A keeper's hands are where the ball is, and where the ball
        // goes next is decided per outcome below.
        let contact_z = self.position.z;
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
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_STAGED.fetch_add(1, Ordering::Relaxed);
        } else {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_NO_SHOOTER.fetch_add(1, Ordering::Relaxed);
        }
        // How far he had to go for it — the state machine turns this into
        // a dive, a catch or a block. See `pending_save_reach`.
        self.pending_save_reach = reach_ratio;
        self.cached_shot_target = None;
        let tick = self.current_tick_cached;
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;

        if outcome_roll < p_catch {
            #[cfg(feature = "match-logs")]
            {
                crate::mid_run_diag::SaveContactDiag::note(
                    0,
                    contact_gap,
                    contact_height,
                    contact_along,
                    contact_across,
                );
                crate::mid_run_diag::KeeperGatherDiag::note_physics(contact_gap);
                crate::mid_run_diag::KeeperHandlingDiag::note(
                    keeper.skills.goalkeeping.handling,
                    0,
                    ball_speed,
                    scaled_handling,
                    hold_difficulty,
                );
            }
            // **Clean catch — and a catch means the ball is IN HIS GLOVES.**
            //
            // This used to set `current_owner` and nothing else, on the
            // belief that the `Claimed` event below routed through
            // `secure_ball_for`. It does not: `Claimed` becomes a
            // `ClaimBall` player event, and that handler's first branch is
            // "already owns the ball — return". So the one gather a keeper
            // makes most often, the save he catches, never raised
            // `held_in_hands`, and every consequence of the flag was lost
            // with it:
            //
            //   * `carry_height` is 0 without it, so the replay drew these
            //     possessions with the ball lying on the grass by his boots
            //     — "the goalkeeper holds it at his feet";
            //   * `carrier_id` reports him as a CARRIER, so the opposition
            //     press a man holding the ball;
            //   * `check_ball_ownership` treats it as a ball at his feet
            //     and hands it to the best tackler within 5u the moment
            //     `claim_cooldown` lapses — and the worst tackler on the
            //     pitch is the goalkeeper.
            //
            // Measured over 12 matches before the fix: 84% of the keeper's
            // possession WAS in his gloves, but the remaining 16% — 382
            // ticks a match — was spent standing on the ball, 40% of that in
            // `HoldingBall`, the state whose entire meaning is that the ball
            // is in his hands. He was robbed 1.5 times a match off his feet
            // and never once out of his gloves, which is the two reported
            // symptoms in one number. See `reception_diag::KEEPER_BALL`.
            //
            // Handling still has to be LEGAL. A shot is never a back-pass
            // and never his own second touch, so the area is the only test
            // that can fail — a sweeper-keeper who heads a shot clear from
            // outside his box keeps his feet.
            self.pending_save_site = 1; // catch
            // **His hands are where the BALL is — so leave it there.**
            //
            // This used to write his own coordinate into the ball, which is
            // the same defect the parry branches had before `contact_z` was
            // kept, on the other two axes. A save resolves anywhere inside
            // the reach `wedge` prices — up to `base_reach`, 2.5 to 4 m —
            // so on every caught save the ball jumped that far in a single
            // 10 ms tick, onto his chest, and then a second time up to
            // `carry_height`. Measured across 4 000 recorded goal clips:
            // 2 706 gathers, a median jump of 0.39 m, 306 of them over a
            // metre and 27 over three.
            //
            // Stopping it where it was caught and letting the ordinary
            // owner-tracking draw it in is both continuous and true to what
            // happened: he takes it at full stretch and brings it into his
            // body over the next tenth of a second (`Ball::move_to`,
            // `BALL_TRACK_SPEED` and `CARRY_RATE`). Nobody can take it off
            // him meanwhile — `held_in_hands` is raised below and
            // `check_ball_ownership` returns on it — and `move_to` will not
            // disown it for the distance either, for the same reason.
            self.velocity = Vector3::zeros();
            self.spin = Vector3::zeros();
            self.current_owner = Some(keeper_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 200;
            // Read off the KEEPER, not the contact point, and the two are
            // no longer the same thing now that the catch leaves the ball
            // where it happened. The Laws test the ball; this tests him.
            // Kept as it is deliberately: he is a good five metres inside
            // his own area whenever this fires and the contact is at most
            // four from him, so the two can only disagree for a
            // sweeper-keeper standing on the edge of the D — and the
            // alternative reading would put a catch made just outside it on
            // his FEET, with the ball then metres from an owner who is not
            // allowed to hold it. That is a worse failure than the one it
            // would fix. Revisit if the reach model ever grows.
            let area = context.penalty_area(keeper_side == Some(PlayerSide::Left));
            let hands_legal = (area.min.x..=area.max.x).contains(&keeper_pos.x)
                && (area.min.y..=area.max.y).contains(&keeper_pos.y);
            if hands_legal {
                self.gather_in_hands(keeper_id, keeper_team, tick);
            } else {
                self.record_touch(keeper_id, keeper_team, tick, true);
            }
            events.add_ball_event(BallEvent::Claimed(keeper_id));
            return;
        }

        if outcome_roll < p_safe {
            self.pending_save_site = 0; // parry — tipped round the post
            #[cfg(feature = "match-logs")]
            {
                crate::mid_run_diag::SAVE_PARRY_FIRED.fetch_add(1, Ordering::Relaxed);
                crate::mid_run_diag::KeeperHandlingDiag::note(
                    keeper.skills.goalkeeping.handling,
                    1,
                    ball_speed,
                    scaled_handling,
                    hold_difficulty,
                );
                crate::mid_run_diag::SaveContactDiag::note(
                    1,
                    contact_gap,
                    contact_height,
                    contact_along,
                    contact_across,
                );
            }
            // Parried OUT for a corner — and it TRAVELS there.
            //
            // This used to resolve POSITIONALLY: the ball was written just
            // past the byline, wide of the post, on the tick of the save.
            // That is a mean 6.5 m teleport (`flight_diag`'s `save_shot`
            // stage: 1.8 a match, worst 12.6 m), and on screen it is a shot
            // vanishing from in front of the keeper and reappearing beside
            // the corner flag — the same class of artefact as the catch
            // teleport above, and the reason it looked like the ball "flies
            // off to nowhere" around the goal.
            //
            // It was written that way because the first attempt gave the
            // ball a DIRECTION and let it run: the keeper is near his line,
            // so a ball merely pushed outward had only reached the post by
            // the time it crossed the byline and about half of them fell
            // inside for a goal kick. The fix for that is not to stop the
            // ball travelling, it is to aim it at the point it has to leave
            // the pitch through. Aimed at `(goal line, outside the post)`,
            // a straight run crosses the byline exactly there whatever the
            // keeper's own position, so the endline resolver awards the
            // corner as reliably as the teleport did — and the ball gets
            // there in about a fifth of a second, which is what a tip round
            // the post looks like.
            let goal_y_for_side = match keeper_side {
                Some(PlayerSide::Left) => context.goal_positions.left.y,
                Some(PlayerSide::Right) => context.goal_positions.right.y,
                None => self.position.y,
            };
            let to_top = self.position.y < goal_y_for_side;
            let exit_x = match keeper_side {
                Some(PlayerSide::Left) => context.goal_positions.left.x,
                Some(PlayerSide::Right) => context.goal_positions.right.x,
                None => self.position.x,
            };
            let exit_y = if to_top {
                (goal_y_for_side - GOAL_WIDTH - 10.0).max(3.0)
            } else {
                (goal_y_for_side + GOAL_WIDTH + 10.0).min(self.field_height - 3.0)
            };
            /// Ticks the tipped ball takes to reach the byline. 18 is
            /// 0.18 s — the ball comes off his fingertips still carrying
            /// most of the pace it arrived with.
            const PARRY_OUT_TICKS: f32 = 18.0;
            let dx = exit_x - self.position.x;
            let dy = exit_y - self.position.y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let out_speed = (dist / PARRY_OUT_TICKS).clamp(0.8, 3.0);
            self.velocity.x = (dx / dist) * out_speed;
            self.velocity.y = (dy / dist) * out_speed;
            self.velocity.z = 0.0;
            // Off his hands at the height his hands were — same rule as the
            // spilled parry below.
            self.position.z = contact_z;
            self.current_owner = None;
            // Nobody gets a touch on a ball that has been tipped round the
            // post: the window covers the whole run to the byline, so the
            // corner cannot be cancelled by a follow-up claim on the way.
            self.flags.in_flight_state = PARRY_OUT_TICKS as usize + 12;
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
        #[cfg(feature = "match-logs")]
        {
            crate::mid_run_diag::SaveContactDiag::note(
                2,
                contact_gap,
                contact_height,
                contact_along,
                contact_across,
            );
            crate::mid_run_diag::KeeperHandlingDiag::note(
                keeper.skills.goalkeeping.handling,
                2,
                ball_speed,
                scaled_handling,
                hold_difficulty,
            );
        }
        self.pending_save_site = 0; // parry — spilled
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
        // **A spill comes off his hands at the height his hands were, and
        // then it falls.** Every save used to slam the ball to `z = 0` on
        // the tick it resolved, so a shot pushed away at chest height
        // dropped a metre and a half in one frame with nothing to explain
        // it — the same class of artefact as the deflection happening away
        // from the keeper, on the other axis. Left at the contact height
        // with no upward push, the ordinary integrator drops it: 1.2 m
        // takes about half a second, which is the whole of the drop zone
        // the direction model above already aims for.
        self.position.z = contact_z;
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
    use nalgebra::Vector3;

    /// **The point-blank save rate lives in `REFLEX_FLOOR`, and nothing
    /// else can move it.**
    ///
    /// A shot from six yards is in the air about 15 ticks, so the ramp is
    /// below the floor and it is priced at exactly `REFLEX_FLOOR ×
    /// base_reach × projection`. The inside-11 m band is 65% of on-frame
    /// shots arriving beyond his reach and the largest single block of
    /// goals in the model, so it is worth a test saying out loud which
    /// constant owns it — `FULL_STRETCH_TICKS` looks like it does and never
    /// enters at all. Both were "corrected" once already because of that
    /// (see the notes on each).
    ///
    /// ⚠ **The crossover moved on 2026-08-24**, with `REFLEX_FLOOR`
    /// 0.54 → 0.38. The floor is a fraction of `FULL_STRETCH_TICKS` (45),
    /// so it now takes over below ~17 ticks of flight rather than below
    /// ~24: at 2.6 u/tick that is **44 u (5.5 m) rather than 63 u (7.9 m)**.
    /// Between those two distances the ramp is what prices him, and that is
    /// correct — he really does have time to start moving — but it means
    /// the floor no longer owns the whole of the inside-11 m band. Both
    /// halves are asserted below so the boundary cannot drift unnoticed.
    #[test]
    fn a_point_blank_strike_is_priced_by_the_reflex_floor_alone() {
        // Five metres out, keeper on his line: 40 u at 2.6 u/tick is ~15
        // ticks of flight.
        let keeper = Vector3::new(0.0, 270.0, 0.0);
        let (_, reach) = SaveModel::wedge(
            Vector3::new(40.0, 270.0, 0.0),
            2.6,
            keeper,
            26.0,
            0.0,
            270.0,
        );
        let floored = 26.0 * SaveModel::REFLEX_FLOOR;
        assert!(
            (reach - floored).abs() < 0.01,
            "a point-blank strike must be priced at the floor, got {reach:.2} against \
             {floored:.2} — if the ramp is binding here, FULL_STRETCH_TICKS is silently \
             carrying the point-blank save rate"
        );
        // …and the crossover is where the constants say it is. A strike
        // from eight metres gives him enough of the window that the ramp
        // takes over, and it must — a floor that swallowed that band too
        // would mean a keeper gets no credit for the extra tenth of a
        // second, which is the whole reason distance is survivable.
        let (_, ramped) = SaveModel::wedge(
            Vector3::new(64.0, 270.0, 0.0),
            2.6,
            keeper,
            26.0,
            0.0,
            270.0,
        );
        assert!(
            ramped > floored + 0.01,
            "at eight metres the ramp must be what prices him, got {ramped:.2} against a \
             floor of {floored:.2} — if this is floored, REFLEX_FLOOR has crept up far \
             enough to own the whole close-range band"
        );
        // …and the floor has to leave him a real hand, not a token one: at
        // the bottom of the reach band (20 u) this is what he covers against
        // a shot from six yards, and it is the whole of why that band is
        // survivable at all.
        assert!(SaveModel::REFLEX_FLOOR > 0.30 && SaveModel::REFLEX_FLOOR < 0.60);
    }

    /// A keeper standing ON his line must get exactly the treatment he got
    /// before the wedge existed. Everything about the population save rate
    /// was calibrated on that case, so if this drifts the calibration has
    /// been moved without anyone deciding to move it.
    #[test]
    fn a_keeper_on_his_line_is_priced_exactly_as_before() {
        let struck_from = Vector3::new(150.0, 300.0, 0.0);
        let keeper = Vector3::new(0.0, 285.0, 0.0);
        let (error, reach) = SaveModel::wedge(struck_from, 2.6, keeper, 26.0, 0.0, 270.0);
        assert!(
            (error - 15.0).abs() < 0.01,
            "on the line the error is the plain lateral gap, got {error:.2}"
        );
        assert!(
            (reach - 26.0).abs() < 0.01,
            "…and his full reach is available for a shot from 19 m, got {reach:.2}"
        );
    }

    /// Coming out to narrow the angle has to PAY, and it has to pay for
    /// the right reason: the same body covers more of the goal the further
    /// from it he stands. Before the wedge this was strictly punished —
    /// every metre off his line counted as a metre of error — which made
    /// standing dead centre on the line the optimal strategy in the engine
    /// and turned `KeeperRestPosition`'s angle model into a liability.
    #[test]
    fn narrowing_the_angle_pays_and_being_off_it_costs_more() {
        // Shot from 19 m, wide right. The goal centre is y = 270.
        let struck_from = Vector3::new(150.0, 300.0, 0.0);
        let goal_line_y = 255.0; // aimed back across, to the far post
        let on_the_line = Vector3::new(0.0, 270.0, 0.0);
        let advanced_on_angle = Vector3::new(50.0, 280.0, 0.0);
        let advanced_off_angle = Vector3::new(50.0, 295.0, 0.0);

        let deficit = |k| {
            let (e, r) = SaveModel::wedge(struck_from, 2.6, k, 26.0, 0.0, goal_line_y);
            e - r
        };
        assert!(
            deficit(advanced_on_angle) < deficit(on_the_line),
            "coming out ON the angle must improve his chance of reaching it"
        );
        assert!(
            deficit(advanced_off_angle) > deficit(advanced_on_angle),
            "…and coming out on the WRONG angle must cost more than staying home, \
             because the same error is magnified by the distance"
        );
    }

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
    ///
    /// ⚠ AND BACK TO 0.66-0.70 (2026-08-16), with the floor. The 66.7%
    /// quoted above was measured through the save-credit leak — physics
    /// saves staged on `Ball::pending_save_credit` were deleted by the
    /// dead-ball clear before they could be booked, so roughly a third of
    /// the engine's own saves never reached the counter. **Every
    /// saves/on-target figure recorded in this file before that date is
    /// understated**, including the 66.7% that justified the 0.69-0.73
    /// band. With delivery at 100% the same 0.57 floor measured 71.3%.
    #[test]
    fn an_ordinary_duel_holds_the_calibrated_population_save_rate() {
        // An evenly-matched duel — which is what a division's average
        // keeper faces every week, at every level.
        let mid = SaveModel::skill_multiplier(0.5, SaveModel::NEUTRAL_THREAT);
        // 0.66-0.70, centred on `SKILL_FLOOR + SKILL_SLOPE/2` = 0.68. The
        // band was 0.69-0.73 while the floor sat at 0.57; it moved with the
        // floor back to 0.54 — see the note on `SKILL_FLOOR`. Widening it
        // to span both would defeat the point: the whole job of this test
        // is to fail when the population save level drifts.
        assert!(
            (0.66..=0.70).contains(&mid),
            "an ordinary duel must stay in the calibrated 0.66-0.70 band, got {mid:.3}"
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

    /// The pace term must be a SPREAD, not a tax. It was rebuilt because
    /// the old form was inert (see `SaveModel::speed_penalty`), and the
    /// first working version promptly moved the population save rate by
    /// nearly five points — restoring a dead axis must not also re-calibrate
    /// the game. An ordinary strike is the zero point.
    ///
    /// This also pins `ORDINARY_PACE` to `ORDINARY_STRIKE`: the two are one
    /// measurement expressed twice, and re-deriving the speed from the
    /// census without re-deriving the pace anchor is exactly how the
    /// population save rate walks off unnoticed.
    #[test]
    fn an_ordinary_strike_costs_the_keeper_nothing_on_pace() {
        for reflexes in [0.0f32, 0.55, 1.0] {
            let p = SaveModel::speed_penalty(SaveModel::ORDINARY_STRIKE, reflexes);
            assert!(
                p.abs() < 0.005,
                "an average-paced shot must be pace-neutral at reflexes {reflexes}, got {p:.3}; \
                 ORDINARY_PACE must equal pace_position(ORDINARY_STRIKE)"
            );
        }
        assert!(
            SaveModel::strike_power(SaveModel::ORDINARY_STRIKE).abs() < 1e-6,
            "the state machine's power term must be centred on the same strike"
        );
    }

    /// …and either side of it, power has to matter — in both directions.
    ///
    /// Monotone everywhere and STRICTLY so above `HARD_STRUCK`. Below that
    /// the curve is deliberately flat: a ball arriving under ~15 m/s gives
    /// the keeper time to set himself and how hard it was hit stops
    /// mattering, so 0.0 and 0.8 u/tick are worth the same. The strict
    /// half is what the post-shot expectation depends on — a clamped
    /// version of this curve made a rocket and a firm shot identical, and
    /// `expected_goal_on_target_reads_placement_power_and_height` is the
    /// test that catches it.
    #[test]
    fn pace_is_monotone_and_cuts_both_ways() {
        let mut previous = f32::MIN;
        for speed in [0.0f32, 0.8, 1.2, 2.0, 2.63, 3.2, 5.0, 8.0, 12.0] {
            let p = SaveModel::speed_penalty(speed, 0.55);
            assert!(
                p >= previous,
                "a harder strike must never cost the keeper LESS; \
                 {speed} u/tick gave {p:.3} against {previous:.3}"
            );
            if speed > SaveModel::HARD_STRUCK {
                assert!(
                    p > previous,
                    "above the set-yourself speed it must be strictly increasing; \
                     {speed} u/tick gave {p:.3} against {previous:.3}"
                );
            }
            previous = p;
        }
        assert!(
            SaveModel::speed_penalty(0.5, 0.55) < 0.0,
            "a tame effort must be EASIER than an ordinary one, not merely no harder"
        );
    }

    /// Reflexes are what a keeper answers pace with, so they have to be
    /// the thing that shrinks it.
    #[test]
    fn reflexes_halve_what_pace_takes_away() {
        let slow = SaveModel::speed_penalty(3.2, 0.0);
        let quick = SaveModel::speed_penalty(3.2, 1.0);
        assert!(slow > 0.0 && quick > 0.0);
        assert!(
            (quick / slow - 0.5).abs() < 0.01,
            "an elite reactor must give away half what a slow one does; \
             slow {slow:.3} quick {quick:.3}"
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
