use crate::PlayerFieldPositionGroup;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderGuardingState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::onball_diag::{self, Exit};
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, LaneAhead, MidfieldPlay, MidfieldRole, MidfielderCondition, Opportunity,
    ShapeStation, TakeOn, U_PER_M,
};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::{FlankAction, FlankPlay, ThroughBall};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::common::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::common::states::MarkEngagement;
use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::{
    ConditionContext, DefensiveDuty, GamePhase, MatchContext, MatchPlayerLite, PassEvaluator,
    PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

/// **The arriving runner's shot, as a licence rather than a rule.**
///
/// # What this replaces, and why it had to go
///
/// The tier used to be DETERMINISTIC — five booleans and a `return`. Its
/// own note above says why that cannot be bounded: *"wherever its gate
/// ends, it saturates the band underneath, because nothing else in it is
/// a probability"*. What that note could not see is the second half of
/// the problem: **the one term in it that varies is the DEFENDING**, and
/// so the whole tier's firing rate is a function of how good the
/// opposition is. Measured over 300 matches at each level with equal
/// squads, this single path was
///
/// | level | share of every shot in the match |
/// |---|---|
/// | 6 | **44%** |
/// | 10 | 17% |
/// | 14 | 4.5% |
/// | 18 | 2% |
///
/// and switching it off took level 6 from 3.46 goals a match to 1.69
/// while leaving level 14 and 18 untouched. It is, on its own, the reason
/// lower divisions played 3-2 every week and the top flight played 0-0.
///
/// # The model
///
/// Two changes, both of which keep the football the tier was written for.
///
/// **Space is relative.** "Nobody within three metres" is a clear chance
/// in a well-defended match and an ordinary Tuesday in a badly-defended
/// one — in a poor game everybody has three metres, all the time, and it
/// does not make every touch in the box a sitter. So the space this look
/// needs is measured against how tightly the match is actually being
/// defended, from the opposing side's defensive aggregate. Same idea as
/// `SaveModel::ORDINARY_PACE` and `ShotBarPopulation`: subtract the
/// population, and what is left is the part that is genuinely a chance.
///
/// **And it is a licence, not a certainty.** A contest between the man
/// arriving and the defence he is arriving against, so an even matchup
/// resolves to the same number at every level. Space and chance quality
/// then scale it continuously — there is no cliff anywhere in it, and a
/// man with two metres and a half-angle gets a real but small licence
/// rather than a flat no.
///
/// The draw is **once per possession**, salted away from the shot bar's
/// own draw so the two decisions are independent. That is the same
/// "one opportunity, one decision" construction the bar uses, and it is
/// what stops "ask again next tick" from being a winning strategy.
struct ArrivingRunner;

impl ArrivingRunner {
    /// Space that counts as a clear chance in an ORDINARILY-defended
    /// match, in game units (1u = 0.125 m). 24u = 3 m — the arriving
    /// eight at the penalty spot with nobody close enough to commit.
    const REFERENCE_SPACE: f32 = 24.0;
    /// How far out to look for the nearest man. Beyond this the answer is
    /// "nobody near", and the ramp below has already saturated.
    const LOOK_RADIUS: f32 = 64.0;
    /// Defensive-aggregate value `REFERENCE_SPACE` is quoted at.
    /// `TeamSkillAggregates::neutral()` is 0.5 on every axis.
    const REFERENCE_DEFENCE: f32 = 0.5;
    /// How hard the requirement scales with how tightly the match is
    /// defended, as an exponent on the ratio of defensive qualities.
    ///
    /// A RATIO and not a linear slope, for two reasons. It cannot go
    /// negative, which a linear form does at the top of the generator —
    /// there the requirement collapsed onto its own rail and the tier
    /// stopped discriminating at exactly the levels it needed to. And
    /// space genuinely is inversely proportional to pressure: halve the
    /// closing-down and you double the room, you do not subtract a fixed
    /// number of metres from it.
    ///
    /// This is the term that carries the tier's divisional flatness,
    /// because the COUNT of unmarked moments is what swung 15×, not the
    /// conversion of them. At the bottom of the pyramid it asks for ~5 m
    /// before a look counts as clear, which is the right football — in a
    /// match where nobody closes down you have to be genuinely alone for
    /// it to be a chance rather than one more crowded touch.
    ///
    /// Fitted on `dev_match levels 200 4 20 2`, reading the FLATNESS
    /// spread: 2.0 → 1.24, **2.2 → 0.63**, 2.5 → 1.12, 3.0 → 1.25,
    /// 4.5 → 1.75. Either side of 2.2 the bottom of the pyramid moves
    /// faster than the middle: too low and level 6 runs 3.4 goals a
    /// match, too high and level 4 collapses to 1.6 and starts railing
    /// against `MAX_REQUIRED`, which is a step in the clamp rather than
    /// in the football.
    ///
    /// ⚠ Those are single runs and the spread is a max-minus-min over
    /// nine points, so it carries roughly +0.25 of pure sampling. The
    /// confirming run at 300 matches a level reads **0.92**, against a
    /// baseline of 3.10 before any of this. Re-fit against the confirm,
    /// not the fit.
    ///
    /// # 2026-08-21 — SUPERSEDED, and set to zero
    ///
    /// This was a compensation made at the SYMPTOM: the requirement was
    /// scaled by the division's defending because the division's
    /// defending was itself a function of the division. That cause is
    /// now removed at source — see `MatchStandard`, which prices every
    /// absolute attribute read against the standard of football in the
    /// match, so a defender challenges, marks and closes down the same
    /// way in every league. With the cause gone the scaling
    /// DOUBLE-COUNTS, and the double-count is not small: measured on the
    /// harness's own level curve, `defensive_quality` runs 0.375 at
    /// level 4 against 0.833 at level 20, which put the requirement at
    /// 45u at the bottom of the pyramid and on its 12u floor from level
    /// 16 up — a 3.8× swing in how much space an arriving runner needed,
    /// on top of a defence that no longer varies.
    ///
    /// Zero leaves the requirement at [`Self::REFERENCE_SPACE`] in every
    /// division, which is what the constant always claimed to be. The
    /// ratio machinery and `OF_CLEAR_TIGHTNESS` are kept so it can be
    /// re-fitted without a rebuild if the two ever diverge again.
    ///
    /// Measured at zero, `dev_match stats 140 14 14`: 2.74 goals a match,
    /// 13.0 shots a team, 30.7% on target, 65.5% saved — the calibration
    /// division exactly where it belongs.
    const TIGHTNESS_EXPONENT: f32 = 0.0;
    /// Rails on that, so the requirement stays a footballing distance at
    /// both ends of the generator — between ~1.5 m and ~8 m.
    const MIN_REQUIRED: f32 = 12.0;
    const MAX_REQUIRED: f32 = 64.0;

    /// Chance quality the licence starts to open at, and where it is
    /// fully open. `expected_xg(d, true)` is the LOCATION value with a
    /// clear lane assumed, so these are location numbers: the ramp runs
    /// across roughly the penalty spot to the six-yard line.
    const XG_OPENS: f32 = 0.08;
    const XG_FULL: f32 = 0.16;

    /// Licence for the worst possible arriving runner against the best
    /// possible defence, and the width of the axis. An even duel
    /// resolves to `FLOOR + SLOPE/2` = **0.62**: a genuinely unmarked
    /// midfielder with a good look shoots about three times in five,
    /// which is what "he shoots without thinking about it" is worth once
    /// it is a probability instead of a rule.
    const FLOOR: f32 = 0.30;
    const SLOPE: f32 = 0.64;
    /// Width of the duel axis, matching the other two contests in the
    /// engine (`SaveModel::skill_multiplier`, `InterceptionDuel`).
    const SPREAD: f32 = 1.30;

    /// `OF_CLEAR_TIGHTNESS` overrides [`Self::TIGHTNESS_EXPONENT`] so the
    /// fit can be swept without a rebuild, the same way `OF_BAR_POPQ`
    /// carries the shot bar's own free parameter.
    #[inline]
    fn tightness_exponent() -> f32 {
        use std::sync::OnceLock;
        static E: OnceLock<f32> = OnceLock::new();
        *E.get_or_init(|| {
            std::env::var("OF_CLEAR_TIGHTNESS")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(Self::TIGHTNESS_EXPONENT)
        })
    }

    /// Space this look needs before it counts as clear, given how
    /// tightly the match is being defended.
    #[inline]
    fn required_space(defence: f32) -> f32 {
        let tightness = Self::REFERENCE_DEFENCE / defence.clamp(0.05, 1.0);
        (Self::REFERENCE_SPACE * tightness.powf(Self::tightness_exponent()))
            .clamp(Self::MIN_REQUIRED, Self::MAX_REQUIRED)
    }

    /// Probability that this arriving runner hits it first time.
    /// `nearest` is the distance to the closest opponent in game units,
    /// `INFINITY` when there is nobody inside [`Self::LOOK_RADIUS`].
    #[inline]
    fn licence(finishing: f32, defence: f32, nearest: f32, xg: f32) -> f32 {
        let required = Self::required_space(defence);
        // Squared so a near-miss on space costs real licence: half the
        // space he needs is a quarter of the chance, not half of it.
        let space01 = (nearest / required).clamp(0.0, 1.0).powi(2);
        let quality01 = ((xg - Self::XG_OPENS) / (Self::XG_FULL - Self::XG_OPENS)).clamp(0.0, 1.0);
        let edge = finishing.clamp(0.0, 1.0) - defence.clamp(0.0, 1.0);
        let advantage = (0.5 + edge * Self::SPREAD).clamp(0.0, 1.0);
        (Self::FLOOR + advantage * Self::SLOPE) * space01 * quality01
    }
}

#[cfg(test)]
mod arriving_runner_tests {
    use super::ArrivingRunner;

    /// **The tier's licence must not be a function of the division.**
    /// It was, completely: as a deterministic rule whose only varying
    /// term was the defending, it produced 44% of every shot in a
    /// level-6 match and 2% in a level-18 one, and switching it off took
    /// level 6 from 3.46 goals a match to 1.69 while leaving 14 and 18
    /// untouched.
    ///
    /// The two halves of the fix are checked separately below; this is
    /// the one that matters. Equal squads, the same amount of space
    /// RELATIVE to how tightly the match is defended, and the licence
    /// comes out the same at every level.
    #[test]
    fn an_even_matchup_licences_the_same_at_every_level() {
        let mut seen: Option<f32> = None;
        for level in [0.25_f32, 0.40, 0.55, 0.70, 0.85] {
            let space = ArrivingRunner::required_space(level);
            let licence = ArrivingRunner::licence(level, level, space, 0.20);
            match seen {
                None => seen = Some(licence),
                Some(first) => assert!(
                    (licence - first).abs() < 1e-5,
                    "licence drifted with the level: {first} at the first, {licence} at {level}"
                ),
            }
        }
    }

    /// **Three metres is three metres in every division**, because the
    /// defending no longer varies with the division — `MatchStandard`
    /// prices it at source. Scaling the requirement on top of that was a
    /// double-count; see `TIGHTNESS_EXPONENT`'s note for the measurement
    /// that retired it. The ratio machinery stays live and `required_space`
    /// must remain on its rails whatever `OF_CLEAR_TIGHTNESS` is set to.
    #[test]
    fn the_requirement_does_not_move_with_the_division() {
        let tight = ArrivingRunner::required_space(0.85);
        let ordinary = ArrivingRunner::required_space(ArrivingRunner::REFERENCE_DEFENCE);
        let loose = ArrivingRunner::required_space(0.30);
        assert!((tight - ordinary).abs() < 1e-4, "{tight} vs {ordinary}");
        assert!((loose - ordinary).abs() < 1e-4, "{loose} vs {ordinary}");
        assert!(
            (ordinary - ArrivingRunner::REFERENCE_SPACE).abs() < 1e-4,
            "an ordinarily-defended match must ask for the reference space"
        );
        for defence in [0.0_f32, 0.05, 0.5, 1.0] {
            let r = ArrivingRunner::required_space(defence);
            assert!(
                (ArrivingRunner::MIN_REQUIRED..=ArrivingRunner::MAX_REQUIRED).contains(&r),
                "off the rails at {defence}: {r}"
            );
        }
    }

    /// It is a licence and never a certainty, and a man with nobody near
    /// him is likelier to hit it than a man with a defender on his toe.
    #[test]
    fn the_licence_is_bounded_and_rises_with_space() {
        let crowded = ArrivingRunner::licence(0.5, 0.5, 2.0, 0.20);
        let free = ArrivingRunner::licence(0.5, 0.5, 64.0, 0.20);
        assert!(crowded < free, "{crowded} vs {free}");
        assert!(free < 1.0, "a licence, not a rule: {free}");
        assert!(crowded >= 0.0);
        // A poor look is not a chance however much room he has.
        assert_eq!(ArrivingRunner::licence(0.5, 0.5, 64.0, 0.02), 0.0);
    }
}

// Shooting distance constants for midfielders — more conservative than forwards
/// Furthest a midfielder will even CONSIDER a strike, in game units
/// (1u = 0.125 m).
///
/// This is a reachability gate, not a decision. It used to be 208u
/// (26 m), and because the whole shooting block below sits behind it, a
/// midfielder further out than 26 m never reached the shot decision at
/// all — he fell through to passing and dribbling every time. That is
/// the entire reason the engine produced **zero shots from 30 m+**
/// against a real ~5% of all shots, and only 4.7% from 22-30 m against a
/// real ~13%: the long shot was not rare, it was unreachable.
///
/// The decision itself belongs to `evaluate_forward_shot_decision`, which
/// already models it properly and per-player: `StrikingRange::of` gives
/// each man his own range from long_shots / technique / strength (25 m
/// for a poacher, 49 m for a specialist), `reach` fades his appetite
/// across it, and the appetite is compared against a per-opportunity
/// threshold. A deep-lying playmaker with a hammer clears that bar from
/// 30 m with a clear sight; an average midfielder does not. That is the
/// behaviour we want, and it was being pre-empted by a flat constant.
///
/// 320u = 40 m matches the helper's own absolute cap, so the two agree
/// on where hopeless begins and the helper is the only thing deciding
/// inside it.
const MAX_SHOOTING_DISTANCE: f32 = 320.0;

/// AI ticks in a second (`MATCH_TIME_INCREMENT_MS` is 10). Every clock in
/// this state is written against it so the durations read as the seconds
/// they are — the previous bare integers (60, 150, 300) were routinely
/// mistaken for something longer than the half-second they described.
const TICKS_PER_SECOND: f32 = 100.0;

/// Per-call-site salts for `Opportunity` — see its doc comment. Distinct
/// values so declining to shoot is not also declining to pass.
const RELEASE_SALT: u64 = 0x8EBC_6AF0_9C88_C6E3;
const CLEAR_CHANCE_SALT: u64 = 0x2545_F491_4F6C_DD1D;
const SNAPSHOT_SALT: u64 = 0x1405_7B7E_F767_814F;

/// Once-per-possession equivalents of two per-tick rolls that used to
/// live in the shooting block (0.3% and 10% a tick). Calibrated to the
/// same MEASURED volume, so the change is in the SHAPE of the decision
/// — a player makes his mind up about a chance rather than re-rolling
/// it a hundred times a second — and not in how many shots there are.
/// See `Opportunity`.
///
/// ⚠ Calibrate these against the census, not by reasoning about how
/// long the window stays open. The first attempt at `CLEAR_CHANCE_RATE`
/// was 0.14, argued from a half-second window; measured, it produced
/// **1178 Tier-1 shots per 120 matches against the 126 the per-tick
/// form produced**, and since Tier-1 bypasses the shot helper entirely
/// that single number moved the whole match — shots/team 20.4 → 24.0,
/// goals 4.48 → 5.37, MID goal share 45% → 52%. The window is far
/// longer than it looks, because an arriving runner holds a clear
/// central look for as long as he holds the ball.
///
/// 2026-08-16: RETIRED (kept for reference — the block no longer rolls
/// it). A per-tick, then per-possession, probability that an arriving
/// midfielder declines a clear chance in the six-yard area is a cooldown
/// by another name: it does not read the chance, only the dice. An
/// arriving runner with a clear look in the box shoots, every time.
/// See `PlayerMemory::can_shoot` for the rest of the teardown.
#[allow(dead_code)]
const CLEAR_CHANCE_RATE: f32 = 0.015;
const SNAPSHOT_RATE: f32 = 0.55;

/// How close an assigned man has to be before a midfielder abandons the
/// shape logic to go and mark him (~19 m). Same figure the back line
/// uses, so a duty means the same thing wherever it is held — the ENGAGE
/// half of [`MarkEngagement`], against which `Guarding` gives the man up
/// at the strictly wider `RELEASE`.
const MARK_BREAK_DISTANCE: f32 = MarkEngagement::ENGAGE;

const STANDARD_SHOOTING_DISTANCE: f32 = 104.0; // 13m — standard shooting range for midfielders
const POINT_BLANK_DISTANCE: f32 = 40.0; // 5m - must shoot, goalkeeper is right there

// Aerial-contest band, matching the Intercepting hand-off and the
// defender's equivalents so the same dropping ball reads identically
// whichever role reaches it first.
const AERIAL_HEADING_HEIGHT: f32 = 1.5;
const AERIAL_HEADING_DISTANCE: f32 = 4.0;

#[derive(Default, Clone)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Offside discipline — if we don't have the ball and we've run
        // past the opposing defensive line, drop back before a teammate
        // plays a pass that finds us offside.
        if !ctx.player.has_ball(ctx) && ctx.player().defensive().is_stranded_offside() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // AERIAL BALL IN REACH: a lofted clearance or long ball dropping
        // onto a running midfielder is contested in the air, not waited
        // out. Checked early — the heading window is only a couple of
        // ticks wide, and everything below this point assumes a ball that
        // can be played with the feet. Without it, midfielders let every
        // aerial ball bounce and the second ball always fell to whoever
        // reacted quickest after it landed.
        // Our own corner delivery is owned by the discrete corner
        // contest (`resolve_corner_contest`) — a midfielder jumping at it
        // uninvited would re-decide an aerial the resolver already
        // resolved. Defensive corner headers are unaffected (the origin
        // check reads the attacking side).
        //
        // ⚠ UNLESS THE CONTEST ELECTED HIM. It drops the ball on the
        // winner's head expecting his state machine to strike it, and
        // this was the door that machine had to come through — so a
        // midfielder who won the corner was shut out of the header he had
        // just been awarded, and the ball simply fell to the floor. Rare
        // while the box was empty (the election nearly always fell to a
        // pushed-up centre-back); routine once `CornerShape` loads the
        // box with the side's best heads.
        let contest_awarded = ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id);
        if !ctx.player.has_ball(ctx)
            && ctx.tick_context.positions.ball.position.z > AERIAL_HEADING_HEIGHT
            && ctx.ball().distance() < AERIAL_HEADING_DISTANCE
            && (!ctx.ball().is_team_attacking_corner() || contest_awarded)
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Heading,
            ));
        }

        // COUNTER-PRESS: we just lost the ball. The closest midfielder
        // to the new carrier committing to an immediate press is the
        // single biggest recovery mechanism in real football — it's
        // why modern high-tempo sides look so relentless. Mirrors the
        // defender counter-press in `defenders/running` line ~108. Only
        // fires for the midfielder best positioned to chase (avoids
        // whole midfield collapsing on one runner).
        if !ctx.player.has_ball(ctx)
            && ctx.team().counterpress_window()
            && !ctx.team().is_control_ball()
        {
            let ball_dist = ctx.ball().distance();
            // Use the per-player eligibility helper so only the
            // best-positioned 2-3 midfielders engage. The ball-best
            // chaser check (already enforced at squad level) layers on
            // top so we don't double-up with a defender or forward who
            // also won the eligibility roll.
            let elected = ctx.player().pressure().should_counterpress()
                && ctx.team().is_best_player_to_chase_ball();
            let immediate = ball_dist < 25.0;
            if immediate {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            if elected && ball_dist < 80.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Phase-first dispatch — midfielders are the engine's pivot
        // between defence and attack, so the phase signal matters most
        // for them. See `phase_dispatch` for behaviour per phase.
        if let Some(phase_action) = self.phase_dispatch(ctx) {
            return Some(phase_action);
        }

        if ctx.player.has_ball(ctx) {
            // Corner taker: set the corner up via Crossing (which holds the
            // delivery until centre-backs have pushed up to attack it).
            if ctx.ball().is_team_attacking_corner() {
                onball_diag::record(Exit::Corner);
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Crossing,
                ));
            }

            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            let coach = ctx.team().coach_instruction();
            let can_shoot = ctx.team().can_shoot();

            // **Who this midfielder is.** Read once at the top of the
            // on-ball tree and threaded through every branch below that
            // used to treat a holding midfielder and a number ten as the
            // same footballer — which was all of them. See
            // [`MidfieldRole`]: the engine dispatches every midfielder
            // into this one function and nothing in it had ever read his
            // slot, so the deep man shot from 22 m and the ten recycled
            // sideways. `from_ctx` is memoised on the frozen tick
            // snapshot, so this costs nothing the tree was not about to
            // spend on the profile anyway.
            let role = MidfieldRole::of(ctx, &MidfielderSkillProfile::from_ctx(ctx));

            // ── Midfielder snapshot under pressure ─────────────────────
            //
            // Same asymmetric pattern as the forward snapshot in
            // `forwarders/states/running/mod.rs`: a midfielder who just
            // received the ball (in_state_time < 8) inside shooting
            // range (< 60u) with a defender right on them (within 10u),
            // AND whose first_touch is below the defender's tackling
            // (by ≥0.5), fires immediately instead of going through
            // the normal control + decision tree. Without this,
            // arriving box-runners who would-be cut-back recipients
            // get tackled before they can shoot — they're MIDfielders
            // and have lower first-touch than dedicated forwards, so
            // strong defenders out-touch them on virtually every
            // reception. Adding the midfielder path lifts the
            // weak-team scoring contribution from runners-into-the-
            // box, which the forward-only path missed entirely.
            //
            // Calibration-neutral at equal skill: at first_touch =
            // tackling, 11 < 10.5 is false, snapshot doesn't fire.
            if can_shoot && ctx.in_state_time < 8 && distance_to_goal < 60.0 {
                let nearest_threat = ctx
                    .players()
                    .opponents()
                    .nearby_raw(10.0)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
                    .map(|(id, _)| id);
                if let Some(threat_id) = nearest_threat {
                    let defender_tackling = ctx.player().skills(threat_id).technical.tackling;
                    let attacker_first_touch = ctx.player.skills.technical.first_touch;
                    // Chance-quality floor — the snapshot bypasses the
                    // unified helper via pending_shot_reason, so it
                    // carries its own xG bar (see FWD_SNAPSHOT_PRESSED).
                    // Same probability gate as the forward snapshot — this
                    // path tags a reason and so bypasses the helper roll.
                    if attacker_first_touch < defender_tackling - 0.5
                        && ctx.player().shooting().expected_xg() >= 0.20
                        && Opportunity::draw(ctx, SNAPSHOT_SALT) < SNAPSHOT_RATE
                    {
                        onball_diag::record(Exit::SnapshotShot);
                        return Some(
                            StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                                .with_shot_reason("MID_SNAPSHOT_PRESSED"),
                        );
                    }
                }
            }

            // Emergency clearance: under heavy pressure in our own box.
            // Route to Passing so its emergency-clearance code path fires
            // (Passing already has `emit_emergency_clearance` gated on
            // `in_box_danger_zone` + `is_under_heavy_pressure`). Running
            // previously had no such escape hatch — a midfielder under
            // two-defender press in their own area kept trying to play
            // out, lost the ball, and conceded via the ensuing turnover.
            if ctx.player().pressure().is_under_heavy_pressure()
                && ctx.ball().distance_to_own_goal() < ctx.context.field_size.width as f32 * 0.18
            {
                onball_diag::record(Exit::EmergencyClear);
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }

            // ── MIDFIELDER SHOOTING (unified, skill-driven) ──────────────
            // Every midfielder in shooting range consults the SAME shot
            // helper the forwards use, so the shoot/pass/hold decision
            // scales continuously with the player's actual shooting
            // attributes (selection / execution / composure) rather than
            // the old binary `mid_shot_selection >= 0.32/0.42` cliffs —
            // which scored a default central mid ~0 yet let an elite one
            // fire unlimited, additive shots. The helper applies its xG
            // floor, inside-six floor, GK / 1v1 read, pass-EV deferral
            // (a playmaker lays it off when a teammate is better placed),
            // and the anti-monopoly volume cap. Net effect: a deep regista
            // rarely shoots, a box-to-box #8 arriving centrally shoots like
            // a forward, and no single player monopolises the attempts.
            // Hoisted above the possession / recycle defaults so a real
            // opening isn't recycled back to a forward.
            // ── HOW FAR OUT HE IS ENTITLED TO STRIKE IT ──────────────
            //
            // The gate above is a REACHABILITY bound — 40 m, where the
            // helper itself calls a strike hopeless — and it is the same
            // 40 m for the holding midfielder and for the number ten.
            // That is the last place in the tree where the two were still
            // the same player, and it is the one that showed most:
            // measured over 140 matches, **49.4% of every midfield shot
            // came from 16.5-22 m** and midfielders scored **61.4% of all
            // goals against a target of 32%**, while forwards took 37.2%
            // against 58%. The engine's midfield was not creating; it was
            // finishing, from range, instead of the men in front of it.
            //
            // This is not a bar on the shot — the helper still owns
            // whether he hits it, with its xG floor, its willingness roll
            // and its pass-EV deferral untouched. It is a bar on the
            // RANGE, which is what a role actually decides in football: a
            // regista does not shoot from twenty-five yards and an
            // arriving eight does, and neither of them is thinking about
            // expected goals when he decides that. Beyond his range the
            // tree simply carries on to the creative branches below,
            // which is where a deep midfielder's answer lives.
            //
            // Continuous, and it never closes: the worst licence still
            // reaches `STANDARD_SHOOTING_DISTANCE` (13 m), so a holding
            // midfielder with the ball at the edge of the six-yard box
            // shoots like anybody else.
            //
            // ⚠ AND THIS IS THE MOST EXPENSIVE NUMBER IN THE PASS —
            // don't widen it without re-reading the goal total.
            //
            // Fitted on `dev_match stats 140 14 14`:
            //
            // | midfielder's range | goals | shots/team | MID 22-30 m |
            // |---|---|---|---|
            // | **linear from 13 m** | **3.51** | **16.9** | 0.5% |
            // | floor at the box edge | 3.83 | 17.4 | 4.0% |
            // | `powf(0.75)` from 13 m | 3.96 / 3.97 | 18.3 | 2.9% |
            //
            // ⚠ Single runs except the last, and the harness carries
            // **±0.15 goals of sampling at this n** — three runs of one
            // build read 3.51 / 3.81 / 3.61. Two consecutive runs of the
            // `powf` arm agreeing to 0.01 is a coincidence and was very
            // nearly read as precision. The ORDERING above is what the
            // table supports; the gaps are one sigma each. Re-fit against
            // `OF_MID_LEGACY`, which prices the whole pass properly.
            //
            // Every attempt to give the midfield its 22-30 m shot back
            // costs a third to a half of a goal a match, because the
            // shots it adds are not only the long ones: a midfielder who
            // is allowed to strike it from range enters this whole block
            // more often, and the block contains the close-range tiers
            // too. `StrikingRange::of` inside the helper already gives
            // each man his own reach off `long_shots` / technique /
            // strength — this gate is the ROLE on top of that, and the
            // role is the half that was missing.
            let strike_range = if MidfieldPlay::legacy() {
                MAX_SHOOTING_DISTANCE
            } else {
                (STANDARD_SHOOTING_DISTANCE
                    + role.shooting * (MAX_SHOOTING_DISTANCE - STANDARD_SHOOTING_DISTANCE))
                    .min(MAX_SHOOTING_DISTANCE)
            };
            if can_shoot && distance_to_goal <= strike_range {
                #[cfg(feature = "match-logs")]
                {
                    use std::sync::atomic::Ordering;
                    crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_INRANGE_TICKS.fetch_add(1, Ordering::Relaxed);
                }
                // Tier 1 — a CLEAR, good-angle chance in range is taken by
                // ANY midfielder. This is chance-quality, NOT a skill gate:
                // whether you SHOOT a clear central look doesn't depend on
                // how good a finisher you are (skill decides whether it goes
                // IN — see the conversion gradient). Without this the
                // willingness roll declines half of even the clean chances
                // and they're recycled away, dropping mid goals AND team
                // totals. The anti-monopoly cap still applies: once a player
                // has hogged (>6 attempts) this falls through to the
                // willingness roll like everyone else, so it can't be abused.
                let sp = ctx.player().shooting().shot_profile();
                // xG bar 0.085 → 0.110 → 0.180: a tighter Tier-1 keeps the
                // box-to-box mid path open for genuine sitters only while
                // the helper's willingness roll handles the grey zone.
                // 0.110 was tuned while the fatigue cliff silently
                // flattened midfielders from ~minute 20 (they sprint the
                // most box-to-box, so they hit the old 15%-condition floor
                // first and stopped arriving in range at all). Fatigue
                // normalization revived them and this deterministic tier —
                // which bypasses the willingness roll entirely — ballooned
                // MID share to 45% of goals (target 32%) at 0.110 and was
                // still 55% of MID shot volume at 0.140. At 0.180 only a
                // true big chance (penalty-spot central, keeper at mercy)
                // fires deterministically — which is the realistic shape:
                // nobody declines those. The anti-monopoly cap also
                // tightens 6 → 4 to mirror the FWD path.
                // Bar raised 0.180 → 0.240 in the 2026-08 state-repair
                // recalibration. The repair made midfielders genuinely
                // fitter (walking / recovery states now work), so they
                // arrive in the box far more often — in-range ticks +18%,
                // runner-in-box ticks +50% — and at 0.180 this
                // deterministic tier tripled MID box-shots at low levels
                // (1090 → 3120 per 200 matches), ballooning MID goal
                // share to 54-58% (target 32%). Same ladder, same reason
                // as every previous rung: whenever mids get more arrival
                // supply, the no-decline bar has to describe a bigger
                // sitter. Tier-2's willingness roll (measured flat) still
                // owns the grey zone below the bar.
                // Post geometry-fix (2026-08, true 0.125m/unit scale)
                // this tier stopped being deterministic-per-tick: with
                // arriving runners now legitimately parked at 10.5-16.5m
                // the old unconditional fire maxed the shot cap every
                // match (~20 box shots/team from this path alone). A
                // clear penalty-spot sitter still gets taken — the
                // per-tick roll integrates to near-certainty over the
                // ~half-second such a window stays open — but a mid no
                // longer machine-guns every clear look the instant it
                // appears. Range tightened to 11m (88u): beyond that,
                // Tier-2's willingness roll owns the decision.
                // AN ARRIVING MIDFIELDER IN THE BOX SHOOTS — the same rule
                // the striker's box block now follows, and it was blocked
                // the same two ways.
                //
                // Range was 88u (11 m), so the late-arriving eight who
                // reaches the penalty spot or the edge of the six had no
                // Tier-1 path at all and fell through to a willingness
                // roll calibrated for speculative efforts. Widened to the
                // penalty area, which is where an arriving runner's whole
                // job is to finish.
                //
                // `has_clear_shot()` drops for the reason it dropped on
                // both forward paths: it is a hard binary veto on lane
                // quality, and lane is already priced — continuously by
                // the helper, and here by `expected_xg(.., true)`, which
                // takes clarity into account. Keeping it meant a marked
                // runner could never qualify, which is every arriving
                // runner worth marking.
                //
                // The 0.30 xG floor, `has_good_angle`, the ≤2-shot cap
                // and the 0.003 throttle below are untouched: those are
                // chance-quality and anti-monopoly gates, not range gates.
                // ⚠ THE xG FLOOR HAS TO BE ON THE ENGINE'S OWN xG SCALE.
                //
                // This asked for 0.30 expected xG before an arriving
                // midfielder in the box was allowed a Tier-1 shot. The
                // engine's xG never gets there: measured, xG/shot is
                // **0.120 at under 6 m and 0.118 at 6-11 m** — the two
                // best bands on the pitch. A 0.30 floor is therefore
                // unreachable from anywhere, and the tier fired **4 times
                // in 20 matches**. It read like a chance-quality gate and
                // behaved like an off switch.
                //
                // 0.10 is a genuinely clear look on this scale: it is met
                // inside ~11 m and not beyond it, which is the shape the
                // gate was written to have.
                // ⚠ …AND THE RANGE HAS TO BE THE ARRIVING RUNNER'S, NOT
                // THE WHOLE PENALTY AREA.
                //
                // This tier is deterministic — no bar, no willingness
                // roll, no cooldown, all three deliberately removed on
                // 2026-08-16 — so its range gate is the ONLY thing
                // deciding how often it fires. Widened from 88u to the
                // full 132u penalty area in the same batch, it became the
                // largest single shot source in the engine: measured over
                // 40 matches, **82.1% of every shot struck from 11-16.5 m
                // carries this reason**, 32 a team a match against a real
                // 2.9 for the whole band, and the band emits 5.4 shots for
                // every one the decision layer approves. Raising or
                // lowering `SHOT_BAR_BASE` cannot touch any of them.
                //
                // The prose above describes an arriving eight at the
                // penalty spot or the edge of the six. That is 11 m, which
                // is what the gate used to say. Beyond it a midfielder is
                // making an ordinary shooting decision and belongs in the
                // decision layer with everybody else — Tier 2, immediately
                // below, is exactly that path.
                //
                // The xG floor cannot substitute for the range gate: it
                // reads `expected_xg(d, true)`, the LOCATION value with a
                // clear lane assumed, so at 14 m it returns a comfortable
                // number for chances that realise at 0.056.
                const ARRIVING_RUNNER_RANGE: f32 = 88.0; // 11 m
                // ⚠ …AND "CLEAR" HAS TO MEAN CLEAR.
                //
                // Narrowing the range alone does not bound a DETERMINISTIC
                // tier — it just moves which band it owns. Measured: with
                // the range back at 11 m, this path stopped producing 82%
                // of the 11-16.5 m shots and immediately produced 37.7% of
                // all shots from 6-11 m instead, at an emit-to-approval
                // ratio of 297%. Wherever its gate ends, it saturates the
                // band underneath, because nothing else in it is a
                // probability.
                //
                // ⚠ **AND IT IS NOT DETERMINISTIC ANY MORE — the prose
                // above and below still describes the old rule.** Every
                // note here diagnosed the shape correctly and none of
                // them could fix it, because a rule cannot be bounded by
                // moving its gate: whichever band the gate lands in, it
                // owns. Worse, the one term in it that varied was the
                // DEFENDING, so the tier's firing rate was a function of
                // the division — 44% of every shot in a level-6 match
                // against 2% in a level-18 one, and switching it off took
                // level 6 from 3.46 goals a match to 1.69 while leaving
                // 14 and 18 where they were. See `ArrivingRunner`, which
                // replaces the boolean with a licence: space priced
                // against how tightly this match is actually defended,
                // chance quality on a ramp, a contest between the man
                // arriving and the defence he is arriving against, and
                // one draw per possession.
                //
                // A deterministic bypass of the whole decision layer has
                // to be reserved for the look that genuinely admits no
                // deliberation, and that is not "a midfielder inside
                // eleven metres". The xG floor cannot draw the line
                // either — recorded xG/shot measures 0.096 inside six
                // metres and 0.104 from 6-11 m, i.e. the two bands are
                // indistinguishable by chance quality, because what
                // separates a sitter from a scramble is not where he is
                // standing but whether anybody is near him.
                //
                // So: space. Three metres of it, inside eleven, facing a
                // good angle — that is the arriving eight at the penalty
                // spot this tier was written for, and he shoots without
                // thinking about it. A midfielder with a defender on him
                // is making an ordinary decision and goes through Tier 2
                // with everybody else. The old `has_clear_shot()` veto was
                // rightly removed (it asked about the LANE, which is
                // priced continuously downstream); this asks about the MAN,
                // which nothing downstream prices at all.
                // ── …AND IT IS A LICENCE, NOT A RULE ──────────────────
                //
                // Everything above still holds; what changed is that the
                // answer is now a probability whose terms are priced
                // against the match rather than a boolean read off one
                // radius. See `ArrivingRunner` for the measurements that
                // forced it — this path alone was 44% of every shot in a
                // level-6 match and 2% in a level-18 one.
                //
                // A/B control for the whole tier — see
                // `MatchContext::mid_clear_off`.
                let defence = if ctx.player.team_id == ctx.context.field_home_team_id {
                    ctx.context.away_skill_aggregates.defensive_quality
                } else {
                    ctx.context.home_skill_aggregates.defensive_quality
                };
                let nearest_opponent = ctx
                    .tick_context
                    .grid
                    .opponents(ctx.player.id, ArrivingRunner::LOOK_RADIUS)
                    .map(|(_, d)| d)
                    .fold(f32::INFINITY, f32::min);
                let clear_good = !MatchContext::mid_clear_off()
                    && distance_to_goal <= ARRIVING_RUNNER_RANGE
                    && coach.shooting_reluctance() < 0.5
                    && ctx.player().shooting().has_good_angle()
                    && Opportunity::draw(ctx, CLEAR_CHANCE_SALT)
                        < ArrivingRunner::licence(
                            sc::shot_threat(
                                ctx.player,
                                sc::minute_from_ms(ctx.context.total_match_time),
                            ),
                            defence,
                            nearest_opponent,
                            sp.expected_xg(distance_to_goal, true),
                        )
                        // …and whether arriving to finish is his game at
                        // all. The tier is named for the ARRIVING RUNNER
                        // and every term in it was about the space and
                        // the chance; none of them was about the man. A
                        // holding midfielder who has ended up at the
                        // penalty spot is not the player the move was
                        // built around, and he takes the shot at a
                        // discount rather than at the same rate as the
                        // eight whose run it was.
                        //
                        // Never a veto — the worst licence still shoots
                        // half as often as the best, which is what a
                        // sitter in front of an open goal deserves from
                        // anybody.
                        * if MidfieldPlay::legacy() {
                            1.0
                        } else {
                            0.52 + role.shooting * 0.62
                        };
                // 2026-08-16: the `shots_taken <= 2` anti-monopoly cap is
                // REMOVED. A midfielder who has already had two efforts is
                // not thereby barred from a clear chance at the penalty
                // spot — there is no such rule in football, and a
                // match-long counter cannot see the chance in front of
                // him. Removed with the other shot quotas and cooldowns;
                // see `PlayerMemory::can_shoot`.
                // One draw per possession, salted away from every other
                // decision on this tree, so a runner who declines does not
                // re-ask on the next tick and walk the gate down.
                if clear_good {
                    // ── …UNLESS SOMEBODY IS SQUARE OF HIM ─────────────
                    //
                    // This tier bypasses the shot helper entirely — its
                    // own note above says so — and with it the helper's
                    // pass-EV deferral, the one place the engine asks
                    // "is a team-mate better placed than me?". So the
                    // arriving midfielder inside eleven metres shot
                    // every time, including with an unmarked striker
                    // four metres square of him, which is the one ball
                    // no footballer alive gets wrong.
                    //
                    // It shows in the totals rather than in the tier.
                    // Once the role gate stopped midfielders shooting
                    // from twenty-two metres they arrived inside eleven
                    // with the ball instead, and this tier — unbounded
                    // by anything but geometry — took **984 of the 6-11 m
                    // shots against 275 before**, which was the whole of
                    // the goal surplus that came with the fix. The
                    // answer is not to send him back out to twenty-two
                    // metres. It is that from eleven, with a better man
                    // beside him, he gives it.
                    if let Some(square) = (!MidfieldPlay::legacy())
                        .then(|| self.find_square_ball(ctx, distance_to_goal))
                        .flatten()
                    {
                        onball_diag::record(Exit::ShootLayoff);
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(square.id)
                                    .with_reason("MID_SQUARE_BALL")
                                    .build(ctx),
                            )),
                        ));
                    }
                    #[cfg(feature = "match-logs")]
                    {
                        use std::sync::atomic::Ordering;
                        crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_SHOOT_FIRED.fetch_add(1, Ordering::Relaxed);
                    }
                    onball_diag::record(Exit::ShootClearChance);
                    return Some(
                        StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                            .with_shot_reason("MID_CLEAR_CHANCE"),
                    );
                }
                // Tier 2 — speculative / long-range / hogger: skill-driven
                // willingness via the shared helper.
                match evaluate_forward_shot_decision(ctx, "MID_SHOOT") {
                    ShotDecision::Shoot { reason } => {
                        #[cfg(feature = "match-logs")]
                        {
                            use std::sync::atomic::Ordering;
                            crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_SHOOT_FIRED.fetch_add(1, Ordering::Relaxed);
                        }
                        // Beyond standard range, route to the dedicated
                        // long-range strike; closer is a normal finish.
                        let state = if distance_to_goal > STANDARD_SHOOTING_DISTANCE {
                            MidfielderState::DistanceShooting
                        } else {
                            MidfielderState::Shooting
                        };
                        onball_diag::record(Exit::ShootHelper);
                        return Some(
                            StateChangeResult::with_midfielder_state(state)
                                .with_shot_reason(reason),
                        );
                    }
                    ShotDecision::Pass => {
                        // Helper judged a teammate the better option — lay
                        // it off (the playmaker's creative choice).
                        if let Some((target, _)) = self.find_best_pass_option(ctx) {
                            onball_diag::record(Exit::ShootLayoff);
                            return Some(StateChangeResult::with_midfielder_state_and_event(
                                MidfielderState::Standing,
                                Event::PlayerEvent(PlayerEvent::PassTo(
                                    PassingEventContext::new()
                                        .with_from_player_id(ctx.player.id)
                                        .with_to_player_id(target.id)
                                        .with_reason("MID_SHOOT_LAYOFF")
                                        .build(ctx),
                                )),
                            ));
                        }
                    }
                    ShotDecision::Hold => {}
                }
            }

            // Coach tempo: if wasting time or slowing down, prefer
            // possession — UNLESS a counter window is open: a team
            // protecting a lead that wins the ball against an
            // overcommitted opponent breaks at full speed, instruction
            // or not (see TeamOps::counter_window).
            if coach.prefer_possession()
                && distance_to_goal > POINT_BLANK_DISTANCE
                && !ctx.team().counter_window()
            {
                let ownership_ticks = ctx.tick_context.ball.ownership_duration;
                if ownership_ticks < coach.min_possession_ticks() {
                    onball_diag::record(Exit::TempoHold);
                    return None;
                }
            }

            // What is actually in front of him. Read once and shared by
            // every branch below that used to draw its own circle —
            // patient possession, the carry, the take-on and the pass
            // veto in `should_pass` all now argue about the same picture
            // instead of three mutually contradictory ones.
            let lane = LaneAhead::read(ctx);
            let mid_profile = MidfielderSkillProfile::from_ctx(ctx);

            // PATIENT POSSESSION: keep the ball moving while the shape
            // reforms — the team-level `should_play_possession` check
            // carries all the real triggers (just won it, tired,
            // leading, late, attack not set).
            //
            // It used to be a LOCK. Sitting above the carry, the
            // take-on, the counter and the cross, it ended in an
            // unconditional `return None` and accepted only passes with
            // a forward component below 0.4 — so for as long as the team
            // was in possession mode the midfielder was forbidden from
            // playing forwards, from running, and from beating anybody,
            // and `is_attack_ready` (a team-mate within 10 m of the
            // opposition goal) is false through most of a build-up, so
            // possession mode is the normal state of affairs rather than
            // the exception.
            //
            // Recycling is a PREFERENCE. If the safe ball is on he plays
            // it; if it is not, he keeps his options — including the run
            // — rather than standing on the ball waiting for one.
            let pressure = self.carry_pressure(ctx);
            if pressure < 0.45
                && distance_to_goal > 70.0
                && ctx.tick_context.ball.ownership_duration > 8
                && lane.openness < 0.5
                && ctx.team().should_play_possession()
            {
                if let Some(target) = self.find_best_pass_option(ctx).map(|(t, _)| t) {
                    // Prefer the sideways / backward retention ball, but
                    // the bar rises with how patient the side actually is
                    // rather than sitting at a flat 0.4 for everyone.
                    let player_pos = ctx.player.position;
                    let goal_pos = ctx.player().opponent_goal_position();
                    let to_goal = (goal_pos - player_pos).normalize();
                    let to_t = (target.position - player_pos).normalize();
                    let forward_component = to_t.dot(&to_goal);
                    let target_in_space = ctx
                        .tick_context
                        .grid
                        .opponents(target.id, 2.5 * U_PER_M)
                        .count()
                        < 2;
                    let forward_tolerance = 0.30 + ctx.team().risk_appetite() * 0.55;
                    if forward_component < forward_tolerance && target_in_space {
                        onball_diag::record(Exit::PatientRecycle);
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target.id)
                                    .with_reason("MID_PATIENT_POSSESSION")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
                // Nothing safe on — fall through to the rest of the
                // tree. He is not obliged to stand still just because
                // the manager wants the ball kept.
            }

            // (Shooting — including the box arrival / cutback finish and
            // point-blank chances — is handled by the unified skill-driven
            // helper block hoisted above; no separate carve-outs needed.)

            // Priority: Clear ball if congested anywhere (not just boundaries)
            // Only attempt after carrying ball for a while to prevent instant pass-after-receive
            if (self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested())
                && ctx.in_state_time > 20
                && ctx.tick_context.ball.ownership_duration > 15
                && lane.openness < 0.4
            {
                // Try to find a good pass option first using the standard evaluator
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    let dist = (target_teammate.position - ctx.player.position).magnitude();
                    // Only pass if target is far enough away to escape congestion
                    if dist > 40.0 {
                        onball_diag::record(Exit::CongestionPass);
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target_teammate.id)
                                    .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_BEST")
                                    .build(ctx),
                            )),
                        ));
                    }
                }

                // Fallback: find teammate at least 40 units away, not a recent passer,
                // and in open space (outside congestion zone)
                if let Some(target_teammate) = ctx
                    .players()
                    .teammates()
                    .nearby(200.0)
                    .filter(|t| {
                        let dist = (t.position - ctx.player.position).magnitude();
                        dist > 40.0
                            && ctx.ball().passer_recency_penalty(t.id) > 0.3
                            && ctx.tick_context.grid.opponents(t.id, 15.0).count() < 2
                    })
                    .max_by(|a, b| {
                        // Prefer the farthest teammate in open space
                        let da = (a.position - ctx.player.position).magnitude();
                        let db = (b.position - ctx.player.position).magnitude();
                        da.partial_cmp(&db).unwrap_or(Ordering::Equal)
                    })
                {
                    onball_diag::record(Exit::CongestionPass);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_NEARBY")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Shooting is evaluated earlier (the SHOOT-FIRST block above,
            // hoisted ahead of the possession / pass-recycling defaults).
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let field_width = ctx.context.field_size.width as f32;
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;

            // ── THE BALL THAT BEATS A LINE ────────────────────────────
            //
            // Asked before he does anything else with it, because that is
            // when a footballer asks it: head up on the first touch, is
            // anybody in behind? If there is, the ball goes; if there
            // isn't, he gets on with the rest of the tree.
            //
            // The engine had no way of playing this ball at all. Every
            // pass it emits is aimed at a team-mate's CURRENT POSITION and
            // then led along his CURRENT VELOCITY, which is a ball to
            // feet — so a ball slid into grass that nobody is standing in
            // yet was not expressible. The one function that claimed to
            // find one, `MidfielderPassingState::find_breakthrough_pass_option`,
            // required TWO OPPONENTS STANDING IN THE PASSING LANE before
            // it would call a pass line-breaking, i.e. it fired only for
            // a ball played through a crowd and never for the ball played
            // into an empty channel. See [`ThroughBall`] for the solve.
            //
            // Gated on his creative licence rather than on his position:
            // a deep midfielder who can see it plays it, an advanced one
            // who cannot does not. The bar inside is drawn once per
            // possession, so declining is a decision and not a dice roll
            // he can re-take next tick.
            if !MidfieldPlay::legacy() && ownership_ticks > 5 && ctx.ball().has_stable_possession()
            {
                if let Some(ball) = ThroughBall::find(ctx, role.creation) {
                    onball_diag::record(Exit::ThroughBall);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(ball.target_id)
                                .with_target_point(ball.aim_point)
                                .with_reason(ball.kind.reason())
                                .build(ctx),
                        )),
                    ));
                }
            }

            // ── THE MAN IN FRONT ──────────────────────────────────────
            //
            // Asked BEFORE the carry, because "keep running" and "go at
            // him" are the same picture read two ways and the carry used
            // to win by being written first. Under the old geometry that
            // did not matter — the carry's own space test could not see
            // a defender past 3.75 m, so anyone further than that simply
            // was not there. Measured: 91.1% of every on-ball tick left
            // at the carry and 1.3% ever reached this question.
            //
            // There is no clock on it. `ownership_ticks < 300` used to
            // close the window three seconds into a carry, which is
            // exactly when a driving run meets the covering midfielder.
            // The one thing worth waiting for is his first touch.
            let settled = ownership_ticks > 5;
            if settled && lane.has_man_to_beat() && goal_dist < field_width * 0.88 {
                let go = TakeOn::decide(ctx, &lane, &mid_profile);
                onball_diag::record_ahead(lane.occupancy, go);
                if go {
                    onball_diag::record(Exit::Dribble);
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Dribbling,
                    ));
                }
            }

            // COUNTER-ATTACK: Quick transition but not instant — need a
            // moment on the ball to see the picture. `is_counter_attack_opportunity`
            // carries the real gate (just won it, few bodies ahead).
            if settled
                && ctx.ball().has_stable_possession()
                && self.is_counter_attack_opportunity(ctx)
            {
                if let Some(forward_target) = self.find_counter_attack_pass(ctx) {
                    onball_diag::record(Exit::CounterPass);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(forward_target.id)
                                .with_reason("MID_RUNNING_COUNTER_ATTACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // ONE-TWO COMBINATION: the man who just gave it to us has
            // run past — give it back.
            //
            // The window was ticks 10..30 of the possession: a fifth of
            // a second, opening one tenth of a second after the ball
            // arrives. A give-and-go is played when the runner GETS
            // THERE, which is a second or more later, so the wall pass
            // was over before it could happen. `find_one_two_return`
            // already requires that the passer has run beyond us into
            // space with a clear lane — that IS the window.
            if settled
                && ownership_ticks <= (2.5 * TICKS_PER_SECOND) as u32
                && ctx.ball().has_stable_possession()
            {
                if let Some(return_target) = self.find_one_two_return(ctx) {
                    onball_diag::record(Exit::OneTwo);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(return_target.id)
                                .with_reason("MID_RUNNING_ONE_TWO_RETURN")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // DRAW AND RELEASE: If opponent is committing to tackle, draw them in
            // then pass to space they vacated — requires carrying to draw them
            if ownership_ticks > (0.3 * TICKS_PER_SECOND) as u32
                && ctx.ball().has_stable_possession()
            {
                if let Some(release_target) = self.find_draw_and_release_pass(ctx) {
                    onball_diag::record(Exit::DrawRelease);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(release_target.id)
                                .with_reason("MID_RUNNING_DRAW_AND_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // CUTBACK FROM WIDE: a wide carrier near the byline plays a low
            // cutback to a central midfielder arriving unmarked in the box
            // (a first-time shot for the runner) in preference to always
            // launching an aerial cross at the forwards. In a 442 the byline
            // carrier is usually a wide mid, so this is the main engine of
            // midfielder goals. Restricted to a true cutback origin (wide +
            // deep); the shared finder enforces the rest (central runner,
            // in range, unmarked, clear lane). Checked just before CROSSING
            // so a genuine cutback chance is taken over the speculative cross.
            if settled && ctx.ball().has_stable_possession() {
                let field_h = ctx.context.field_size.height as f32;
                let mid_goal = ctx.player().opponent_goal_position();
                // "Deep" = near the byline in X; "off-centre" = poor own
                // angle. Using goal-CENTRE distance here is wrong (a wide
                // carrier is always far from centre), so key off byline X.
                let carrier_byline = (mid_goal.x - ctx.player.position.x).abs() < 90.0;
                let carrier_offcenter =
                    (ctx.player.position.y - field_h / 2.0).abs() > field_h * 0.15;
                if carrier_byline && carrier_offcenter {
                    if let Some(runner) =
                        crate::r#match::player::strategies::common::players::ops::forward_shot_decision::find_cutback_to_arriving_runner(ctx)
                    {
                        #[cfg(feature = "match-logs")]
                        {
                            use std::sync::atomic::Ordering;
                            crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_CUTBACK.fetch_add(1, Ordering::Relaxed);
                        }
                        onball_diag::record(Exit::Cutback);
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(runner.id)
                                    .with_reason("MID_CUTBACK_TO_RUNNER")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
            }

            // CARRY FORWARD: grass in front, so run into it.
            //
            // No skill gate. `allows_carry_into_space` demanded
            // carry_selection >= 0.32, which is a bar a defensive
            // midfielder can fail — and the thing it was gating is
            // running forwards with the ball when nobody is near you,
            // which is not a skill. What IS a judgement (whether to go
            // at a man) is asked above; what is a fact (how much room is
            // there) is `lane.openness`, and a half-open lane is still a
            // lane. The lower bound keeps a carry out of the six-yard
            // box and the upper one out of our own keeper's area.
            //
            // ⚠ POSITION IN THE TREE IS THE WHOLE POINT. This used to sit
            // directly under the shooting block, and `return None` skips
            // everything after it — so the counter, the one-two, the
            // draw-and-release, the cutback, the cross and the switch
            // were all unreachable for as long as the carrier had grass
            // in front of him, which after the geometry fix is 96% of
            // the time. A driving midfielder keeps his head up: a ball
            // that beats the line is better than the run, and only the
            // GENERIC "give it to someone better placed" below is worse
            // than it.
            // ⚠ `openness` IS A DISTANCE IN DISGUISE, AND 0.22 IS FOUR
            // METRES. `openness = nearest / RUNNING_ROOM` with
            // `RUNNING_ROOM` = 18 m, so the bar for "there is grass in
            // front, run into it" was **a defender more than 4.0 m away**.
            //
            // Four metres is not space you carry into, it is space you
            // pass out of — and because this branch sits above `switch`
            // and `should_pass`, clearing that bar meant the generic
            // "give it to someone better placed" was never asked. The
            // result is the engine's largest departure from real football:
            // `CARRY` took **96.9% of every on-ball tick** a midfielder
            // had, progressive carries ran **36.8 per midfielder per match
            // against a real ~2**, and the ball reached shooting range by
            // being run there rather than passed there. Shots 120/team
            // against a real 13.
            //
            // 0.55 is 9.9 m — a genuinely clear channel, which is what a
            // player is looking at when he decides to drive rather than
            // release. Below that the man ahead is close enough to be a
            // decision, and the decision belongs to the branches beneath
            // this one.
            //
            // Titration note: this is the chance-SUPPLY lever, and it is
            // the one place raising the bar does not distort the shot mix
            // the way `TARGET_SIZE_WEIGHT` does — it removes the arrival,
            // not the attempt.
            // THE WIDE AREA, before the carry.
            //
            // ⚠ ORDER IS THE WHOLE FIX. This ladder used to sit BELOW the
            // carry, and the carry returns early — so a man on the
            // touchline with a runner in the box was asked "is there
            // grass in front of you?" first, and on a flank there always
            // is, because the flank is where nobody stands. Measured, the
            // crossing rung was reached on **0.005% of on-ball ticks**
            // and the engine struck 2.2 open-play crosses a team a match
            // against a real 16-18.
            //
            // Nothing here reads a role or a team assignment: it is
            // decided by where the man is standing, so a full-back, a
            // midfielder or a centre-half who finds himself wide and high
            // plays it the same way. See `FlankPlay`.
            if settled && ctx.ball().has_stable_possession() {
                match FlankPlay::decide(ctx) {
                    Some(FlankAction::ReleaseOutside { target }) => {
                        #[cfg(feature = "match-logs")]
                        crate::mid_run_diag::WideDiag::note(4);
                        onball_diag::record(Exit::FlankRelease);
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Running,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target)
                                    .with_reason("MID_FLANK_RELEASE")
                                    .build(ctx),
                            )),
                        ));
                    }
                    Some(FlankAction::Deliver) => {
                        onball_diag::record(Exit::Cross);
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Crossing,
                        ));
                    }
                    None => {}
                }
            }

            // ⚠ …AND ROOM IN FRONT IS A REASON TO RUN, NOT A REASON TO
            // STOP LOOKING.
            //
            // Everything above is right and it was still not enough,
            // because the test was a single number about the GRASS and
            // carried nothing at all about the man. Measured over 140
            // matches at level 14: **CARRY took 92.4% of every on-ball
            // tick a midfielder had**, progressive carries ran **73.5 per
            // midfielder per match against a real ~2** and successful
            // dribbles 18.1 against ~1. The ball reached the final third
            // by being run there, every time, by whoever happened to have
            // it — which is the whole of "the midfield creates nothing":
            // a man running with the ball is a man not playing a pass.
            //
            // Two things enter, and both are football rather than
            // calibration.
            //
            // **Whose job it is.** A holding midfielder driving out of
            // his own half is how a side gets countered, and a wide
            // player beating his full-back is most of what he is out
            // there for. [`MidfieldRole::carry`] carries that, and it is
            // his ABILITY scaled by his station — so a regista with
            // wonderful feet still releases it, because carrying is not
            // what he is for.
            //
            // **Whether a better ball is on.** `better_placed_gain` is
            // already the continuous "somebody ahead of me is freer and
            // closer to goal" read that `should_pass` uses; here it is
            // simply subtracted, weighted by how well this player sees
            // it. A carrier with nobody ahead of him keeps running. One
            // with a team-mate in space beyond him gives it, which is
            // the entire behaviour the census says the engine never had.
            //
            // **And the closer to goal, the less a carry is worth.**
            //
            // Territory is in every other risk decision in this tree —
            // the take-on has it, `should_pass` has it — and the carry
            // had none: the same amount of grass licensed the same run
            // in his own half and in the opposition penalty area. In the
            // final third that is the wrong football twice over. The ball
            // travels faster than the man, so a carry there costs the
            // one thing the attack has left; and defenders are close
            // enough that the grass in front of him is a moment old.
            //
            // Measured, it is also where the surplus lives. Capping the
            // deep midfielder's shooting RANGE (above) did not stop him
            // shooting — it moved him: Tier-1 `MID_CLEAR_CHANCE` went
            // **309 shots to 909** while the midfield's total stayed
            // flat, because a man who may not strike it from twenty-two
            // metres simply carried to nine and struck it there. Nine
            // metres converts four times as well, so the goal total went
            // with him. The answer is that the carry is what should have
            // ended, not that the shot should have been longer.
            //
            // Fitted on `dev_match stats 140 14 14`, three arms:
            //
            // | final-third term | goals | shots/team | MID / FWD goals |
            // |---|---|---|---|
            // | none (flat 0.55) | 4.56 | 20.2 | 50% / 50% |
            // | 0.42 | 2.88 | 15.3 | 27% / 73% |
            // | **0.28** | 3.5-3.8 | 16.9 | 45% / 53% |
            //
            // (Single runs, ±0.15 goals of sampling — read the ordering,
            // not the gaps. The shipped number is confirmed against
            // `OF_MID_LEGACY` in the paired A/B on `SHOT_BAR_BASE`.)
            //
            // The flat bar leaves the midfield finishing its own attacks
            // (the fault this whole pass exists to remove); 0.42 hands
            // the game to the forwards so completely that the midfield
            // stops being a scoring line at all, against a target of
            // 32%. The term is the dial between those two, and it is a
            // footballing dial rather than an arbitrary one: it says how
            // much sooner a man gives the ball up as he gets closer to
            // the goal.
            const CARRY_BAR: f32 = 0.52;
            const CARRY_BAR_FINAL_THIRD: f32 = 0.28;
            let progress = 1.0 - (goal_dist / field_width).clamp(0.0, 1.0);
            let bar = CARRY_BAR + progress.powf(1.6) * CARRY_BAR_FINAL_THIRD;
            let outlet = if MidfieldPlay::legacy() {
                0.0
            } else {
                self.better_placed_gain(ctx, goal_dist)
            };
            let (carry_urge, bar) = if MidfieldPlay::legacy() {
                (lane.openness, 0.55)
            } else {
                (
                    lane.openness * (0.72 + role.carry * 0.52)
                        - outlet * (0.34 + mid_profile.progressive_selection * 0.62),
                    bar,
                )
            };
            if goal_dist > POINT_BLANK_DISTANCE
                && goal_dist < field_width * 0.85
                && carry_urge > bar
            {
                onball_diag::record(Exit::Carry);
                return None;
            }

            // …and a refused carry is a PASS, not a pause.
            //
            // ⚠ THIS IS THE HALF THAT MAKES THE GATE ABOVE MEAN
            // ANYTHING. `Exit::Carry` and `Exit::NoDecision` both end in
            // `return None`, and `velocity()` drives the carrier at the
            // goal either way — so a refused carry that falls through to
            // the bottom of the tree is **still a carry**, just an
            // unlabelled one. The first cut of this gate proved it:
            // `CARRY` fell from 92.4% of on-ball ticks to 79.8%,
            // `no-decision` rose from 5.5% to 16.8%, and progressive
            // carries moved 73.5 → 71.6. Nothing had changed on the
            // pitch; only the census row.
            //
            // So the refusal has to move the ball. Every branch below
            // this one carries its own clock (`ownership_ticks > 15`) and
            // its own bar, and none of them is the sentence a footballer
            // is actually saying here: *I am not running with this, so
            // it goes.*
            //
            // ⚠ …TO SOMEBODY. Made unconditional this becomes "release
            // on every tick the run is not on", which is not a decision
            // either: passes went **1053 → 1276 a team** against a real
            // ~500 and the midfield stopped being a scoring line at all
            // (26.8% of goals against a 32% target, with the forwards on
            // 72.7% against 58%). A carrier with nobody better placed and
            // no run on keeps it and looks again — that is what the rest
            // of the tree is for.
            const RELEASE_OUTLET: f32 = 0.20;
            if !MidfieldPlay::legacy()
                && outlet > RELEASE_OUTLET
                && ctx.ball().has_stable_possession()
            {
                if let Some((target, _)) = self.find_best_pass_option(ctx) {
                    onball_diag::record(Exit::ShouldPass);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target.id)
                                .with_reason("MID_HEAD_UP_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // SWITCH PLAY: the ball side is crowded and the far flank is
            // not. Both halves of that sentence are now actually tested.
            //
            // It used to be `teammates_on_side >= 3 || is_congested`,
            // counting OUR OWN players on whichever lateral half the
            // carrier stood on. In any formation with four across the
            // middle that is true almost always, so "switch the play"
            // was the default action of every midfielder who reached it
            // — 98k ticks and 18.9% of everything he did with the ball
            // when it was allowed to run before the carry. A switch is a
            // ball you play because THEY are all on one side and there
            // is a free man on the other.
            //
            // ⚠ AND IT IS A BUILD-UP BALL, NOT A FINAL-THIRD ONE. There
            // was no distance gate here at all, so a wide midfielder
            // inside the penalty area — which in a 442 is the winger, the
            // man most likely to be there — would switch the play across
            // the pitch instead of crossing, cutting back or shooting.
            // Worse, this branch sits BELOW the carry, so it only reaches
            // a carrier whose lane is blocked: a winger stuck near the
            // byline, which is precisely the picture where the ball goes
            // INTO the box, not forty metres backwards to the far flank.
            //
            // Switches are how you move the point of attack while you are
            // still building. Inside 22 m the point of attack is the goal.
            //
            // ⚠ THAT SENTENCE DESCRIBED A GATE THAT WAS NEVER WRITTEN.
            // There is no distance term anywhere below it, and measured
            // over 140 matches the switch was the largest pass-emitting
            // branch in the whole tree — **542 a match across the two
            // sides, 271 per team**, more than every forward pass in the
            // tree combined, against a real handful. A ball played from
            // the edge of the box to the opposite touchline is not a
            // switch of play; it is an attack being abandoned.
            //
            // 176u is the 22 m the note asks for. Inside it the ball goes
            // toward the goal — the cutback, the cross and the through
            // ball above all live in that band and were being pre-empted
            // by this one.
            const SWITCH_BUILD_UP_RANGE: f32 = 176.0;
            let switch_range = if MidfieldPlay::legacy() {
                0.0
            } else {
                SWITCH_BUILD_UP_RANGE
            };
            if settled && ctx.ball().has_stable_possession() && goal_dist > switch_range {
                let field_height = ctx.context.field_size.height as f32;
                let field_center_y = field_height / 2.0;
                let ball_top = ctx.player.position.y < field_center_y;

                let mut opponents_ball_side = 0usize;
                let mut opponents_far_side = 0usize;
                for opp in ctx.players().opponents().all() {
                    if (opp.position.y < field_center_y) == ball_top {
                        opponents_ball_side += 1;
                    } else {
                        opponents_far_side += 1;
                    }
                }
                let overloaded = opponents_ball_side >= opponents_far_side + 3;

                // …and somebody to switch it TO: a team-mate on the far
                // flank with room around him.
                let far_man_free = ctx.players().teammates().all().any(|t| {
                    (t.position.y < field_center_y) != ball_top
                        && (t.position.y - field_center_y).abs() > field_height * 0.20
                        && ctx
                            .tick_context
                            .grid
                            .opponents(t.id, 6.0 * U_PER_M)
                            .next()
                            .is_none()
                });

                if overloaded && far_man_free && mid_profile.allows_switch_play() {
                    onball_diag::record(Exit::Switch);
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::SwitchingPlay,
                    ));
                }
            }

            // COACH TEMPO: When instructed to slow down, prefer passing back to defenders
            if coach.prefer_possession()
                && ownership_ticks > coach.min_possession_ticks()
                && ctx.ball().has_stable_possession()
            {
                if let Some(safe_target) = self.find_safe_backward_pass(ctx) {
                    onball_diag::record(Exit::TempoBackPass);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("MID_COACH_TEMPO_PASS_BACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Enhanced passing decision — look for a good pass
            if ownership_ticks > 15
                && ctx.ball().has_stable_possession()
                && self.should_pass(ctx, &mid_profile, &lane)
            {
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    onball_diag::record(Exit::ShouldPass);
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_SHOULD_PASS")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // ⚠ A "no pass on, so beat him" clause was tried here and
            // MEASURED WORSE — do not put it back. It fired 14.3k times
            // against 4.7k considered take-ons, mostly near the byline
            // where every team-mate is marked, so the answer to "nobody
            // to pass to" became "dribble into the six-yard box": MID
            // goal share 43% → 59%, shots 19.9 → 23.7, and the ball-stuck
            // clock went UP (59 s → 71 s a match) because a refused duel
            // just repeats. A blocked carrier with no out does not charge
            // the defender — he shields and turns, which is a STEERING
            // answer, not a state change. See `carry_steering`.
        } else {
            // Without ball — go to the man who has it.
            //
            // ⚠ THE PROXIMITY SCAN CANNOT COME FIRST. This was
            // `.nearby(150.0)`, which is **18.75 m**, and everything
            // below — including `is_best_player_to_chase_ball`, the
            // team-wide designation of who chases — sat inside it. So a
            // midfielder who WAS his side's designated chaser but stood
            // twenty-five metres away never entered the block at all and
            // did nothing. The designation has already picked the nearest
            // able man; bounding it by distance defeats the whole point
            // of having one, and when the carrier is beyond the bound
            // NOBODY on the team reacts.
            //
            // Measured with the carrier-pressure census: in the MIDDLE
            // THIRD, 24% of carrier ticks had no opponent within 10 m,
            // against 7% in the defensive third — a man walking through
            // the centre circle completely unopposed. The defender
            // equivalent of this block already scans unbounded
            // (`opponents().with_ball()`); this is the same shape.
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                let opponent_distance = (opponent.position - ctx.player.position).magnitude();

                // Close — tackle regardless (reactive)
                if opponent_distance < 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }

                // Only the best-positioned player presses — prevents team swarming
                if ctx.team().is_best_player_to_chase_ball() {
                    if opponent_distance < 50.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Tackling,
                        ));
                    }
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
                // Others: stay in running (will follow waypoints via velocity)
            }

            // Teammate has the ball — actively support the attack
            if ctx.team().is_control_ball() {
                // …unless the plan has given him a touchline. First,
                // because every branch below is written from the ball
                // outward and would pull him infield.
                if WideChannel::still_mine(ctx) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::HoldingWidth,
                    ));
                }

                let ball_distance = ctx.ball().distance();
                let goal_dist = ctx.ball().distance_to_opponent_goal();
                let field_width = ctx.context.field_size.width as f32;

                // ANTI-CLUSTERING: If too many teammates nearby, go find space
                let nearby_teammates = ctx.players().teammates().nearby(25.0).count();
                if nearby_teammates >= 2 && ball_distance > 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }

                // If ball is in attacking third and we're nearby, make attacking runs
                if goal_dist < field_width * 0.4 && ball_distance < 300.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }

                // If far from ball, create space to offer a passing option
                if ball_distance > 200.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }

                // Medium range: actively support (don't just drift)
                // Require enough time in Running to avoid rapid oscillation with AttackSupporting
                if ctx.in_state_time > 80 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }

                // First 80 ticks: stay in Running with active velocity
                return None;
            }

            // Loose ball nearby — only chase if we're the best positioned teammate
            if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 3.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::TakeBall,
                    ));
                }
            }

            // Loose-ball claim is handled universally at the dispatcher
            // (`PlayerFieldPositionGroup::process`). Duplicating the check
            // here with the tolerance-banded `is_best_player_to_chase_ball`
            // let multiple players enter TakeBall simultaneously.

            // Also respond to ball system notifications
            if ctx.ball().should_take_ball_immediately()
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            // DUTY BEFORE SHAPE — the midfield half of the rule the back
            // line already follows. A midfielder the plan gave a man goes
            // to him; `Guarding` is the midfielder's marking state.
            //
            // ⚠ THIS USED TO SIT INSIDE `ball().on_own_side()`, and that
            // is where midfield defending went to die. The plan only
            // assigns a duty while the team is DEFENDING and only to a
            // marker within `MARK_REACH` of his man, so the extra gate
            // added nothing except this: while the opposition built up or
            // carried through the middle — the whole period the report
            // "nobody presses, they just run at goal" is about — every
            // midfielder ignored the man he had been given.
            //
            // Measured: opening the plan up to mark the carrier's outlets
            // (`teamplay::defence::BALL_THREAT_RADIUS`) took assigned
            // marks from 2.15 to 4.40 per refresh and `Defender: Marking`
            // from 6.3% to 8.2% of all ticks, while `Midfielder: Guarding`
            // did not move **at all** — the back line acted on its new
            // duties from the equivalent branch, which carries no such
            // gate, and the midfield could not.
            if let Some(man) = ctx.team().my_mark() {
                if (man.position - ctx.player.position).magnitude() < MARK_BREAK_DISTANCE {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }

            // …and the same for the man on the ball. The back line has
            // carried this branch since the plan was built; the midfield
            // never did, so a midfielder nominated as the ENGAGER had no
            // way to act on it from `Running` and every press he made had
            // to come through the single-best-chaser election instead.
            //
            // No `on_own_side` gate here, unlike the defenders' copy: a
            // back four steps out to meet a carrier rather than chasing
            // him upfield, but pressing in the opposition half is exactly
            // what a midfield is for.
            //
            // …and no distance gate either. `MARK_BREAK_DISTANCE` (150u)
            // is a MARKING boundary — how far a defender travels to pick
            // a man up — and using it here made the branch narrower than
            // the duty it acts on: the plan nominates over `PRESS_REACH`
            // (200u), so a nominated presser between the two held the
            // duty and had no way to act on it, which is the same
            // COMMIT < DISENGAGE mismatch `TackleEngagement` exists to
            // remove. The plan's own reach is the bound.
            if matches!(ctx.team().my_duty(), DefensiveDuty::Press) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Track dangerous runners — opponent forwards sprinting toward our goal
            if ctx.ball().on_own_side() {
                let own_goal = ctx.ball().direction_to_own_goal();
                let has_dangerous_runner = ctx.players().opponents().forwards().any(|opp| {
                    let dist = (opp.position - ctx.player.position).magnitude();
                    if dist > 60.0 {
                        return false;
                    }
                    let vel = opp.velocity(ctx);
                    let speed = vel.norm();
                    if speed < 2.0 {
                        return false;
                    }
                    let to_goal = (own_goal - opp.position).normalize();
                    let alignment = vel.normalize().dot(&to_goal);
                    alignment > 0.5
                });

                // (The duty branch that used to sit here has been hoisted
                // above the `on_own_side` gate — see the note there.)

                // Dangerous runner detected — close them down via Guarding.
                // TrackingRunner was a single-entry ghost state that did the
                // same "stay goal-side of the runner" thing Guarding already
                // does; keeping Guarding removes the duplicate.
                if has_dangerous_runner {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }

            // Guard unmarked attackers on our side when we can't
            // press/intercept — but only if there is actually somebody to
            // guard. This condition is about where the BALL is; `Guarding`
            // answers "nobody to guard" by handing the player straight
            // back here, so without asking its question first the two
            // states ran as a two-cycle. Same memoised scan `Guarding`
            // itself uses, so this costs a cache lookup.
            if ctx.ball().on_own_side()
                && ctx.ball().distance() > 100.0
                && MidfielderGuardingState::default()
                    .find_committable_guard_target(ctx)
                    .is_some()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Guarding,
                ));
            }

            // Fatigue recovery during a lull — midfielders cover the most
            // ground yet were the one outfield role that never entered
            // Resting (defenders gate on stamina in Pressing/Marking,
            // forwards via needs_recovery; nothing produced the
            // midfielder variant).
            //
            // The original gate was a three-way conjunction — under 40%
            // condition AND ball beyond 150u AND play in the opposition
            // half — that was never satisfied in a real match: the
            // in-match condition floor keeps players well above 40% while
            // they are still far enough from the ball to stand down, so
            // `Resting` stayed empirically dead even though it was
            // statically reachable. Matched to the forward's
            // `needs_recovery` shape instead: genuinely tired, after a
            // sustained run, with the ball far enough away to stand down.
            // Resting's own exits (ball close, team under threat) pull the
            // player back out immediately.
            const REST_STAMINA_THRESHOLD: u32 = 60;
            const REST_BALL_DISTANCE: f32 = 150.0;
            const REST_MIN_RUN_TICKS: u64 = 60;
            if ctx.player.player_attributes.condition_percentage() < REST_STAMINA_THRESHOLD
                && ctx.in_state_time > REST_MIN_RUN_TICKS
                && ctx.ball().distance() > REST_BALL_DISTANCE
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Resting,
                ));
            }
        }

        // ANTI-OSCILLATION: If carrying ball too long without acting, force a decision
        // POSSESSION RETENTION: Allow longer holding when team is comfortable
        let anti_oscillation_threshold = if self.should_retain_possession(ctx) {
            250
        } else {
            150
        };
        if ctx.player.has_ball(ctx) && ctx.in_state_time > anti_oscillation_threshold {
            onball_diag::record(Exit::AntiOscillation);
            // Prefer passing first
            if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION")
                            .build(ctx),
                    )),
                ));
            }
            // An AM-only second call to `evaluate_forward_shot_decision`
            // used to sit here. The on-ball block at the top of this
            // state already calls the helper every tick for every
            // midfielder, so it could only repeat an answer just given,
            // and it made a #10 a different footballer from an #8 in the
            // same position. The point-blank fallback under it asked for
            // a clear shot inside 25u — 3.1 m, which is inside the
            // six-yard line — behind an absolute `mid_shot_selection`
            // bar. Anti-oscillation is a safety net for a player who has
            // stopped deciding; it is not a place to keep a second,
            // worse copy of the shot model.
            // Last resort: pass to any nearby teammate ahead of the ball (toward opponent goal)
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();
            if let Some(target_teammate) = ctx
                .players()
                .teammates()
                .nearby(200.0)
                .filter(|t| {
                    let to_teammate = (t.position - player_pos).normalize();
                    to_teammate.dot(&to_goal) > 0.0 // Teammate is ahead (toward opponent goal)
                })
                .next()
            {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK")
                            .build(ctx),
                    )),
                ));
            }
            // Absolute last resort: pass to any nearby teammate (even backward)
            if let Some(target_teammate) = ctx.players().teammates().nearby(200.0).next() {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK_ANY")
                            .build(ctx),
                    )),
                ));
            }
        }

        if ctx.player.has_ball(ctx) {
            onball_diag::record(Exit::NoDecision);
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Simplified waypoint following
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();
            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        crowd_offset: ctx.player().separation_offset(),
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        if ctx.player.has_ball(ctx) {
            // POSSESSION RETENTION: When in control mode, move slower with more lateral sway
            // to keep ball and tire opponents instead of always charging forward
            if self.should_retain_possession(ctx) {
                Some(self.calculate_possession_retention_movement(ctx))
            } else {
                Some(self.calculate_simple_ball_movement(ctx))
            }
        } else {
            let start_pos = ctx.player.start_position;
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;

            // Off-ball movement when our team isn't in control. Compact
            // defensive block rather than a frozen formation line:
            //   - Ball-side compaction: the whole midfield shifts toward
            //     the ball's lateral (y) position.
            //   - Depth compaction: the line pushes up when the ball
            //     advances, drops back when the ball retreats.
            //   - Ball-side / far-side stagger: the midfielder on the
            //     ball's half steps slightly forward to engage, the
            //     far-side one drops slightly back to cover. This is
            //     what breaks the straight-line "robot" look.
            //   - Separation from nearby teammates naturally spreads
            //     players when formation positions overlap.
            // Loose ball pulls harder (45%) because the designated chaser
            // has already transitioned to TakeBall via `process()`; the
            // rest gently close the gap in support.
            if !ctx.team().is_control_ball() {
                let ball_pos = ctx.tick_context.positions.ball.position;
                let ball_loose = !ctx.ball().is_owned();
                let field_half_x = field_width * 0.5;
                let field_half_y = field_height * 0.5;
                let attacking_left = ctx.player.side == Some(PlayerSide::Left);

                // Lateral (y) shift: always track ball laterally so the
                // midfield slides as a block. 0.3 = a normal compact block,
                // more during a loose-ball scramble (urgency).
                let lateral_coef = if ball_loose { 0.45 } else { 0.30 };
                let lateral_shift = (ball_pos.y - field_half_y) * lateral_coef;

                // Depth (x) shift: push with ball depth. Low coefficient so
                // we don't abandon the defensive line when ball is deep.
                let depth_shift = (ball_pos.x - field_half_x) * 0.15;

                // Ball-side stagger: player on the same lateral half as the
                // ball steps ~10 units toward the opponent's goal to
                // engage; the far-side player drops ~10 units back. A
                // diagonal stagger instead of a flat line.
                let ball_top = ball_pos.y < field_half_y;
                let player_top = start_pos.y < field_half_y;
                let on_ball_side = ball_top == player_top;
                let forward_sign = if attacking_left { 1.0 } else { -1.0 };
                let stagger_x = if on_ball_side { 10.0 } else { -10.0 } * forward_sign;

                let target_x =
                    (start_pos.x + depth_shift + stagger_x).clamp(30.0, field_width - 30.0);
                let target_y = (start_pos.y + lateral_shift).clamp(30.0, field_height - 30.0);

                let target = Vector3::new(target_x, target_y, 0.0);

                // No outer deadzone / zero-velocity early return: the hard
                // stop produced the "arrive-and-jitter" look. Arrive's own
                // quadratic slowing + 3-unit brake zone handles settling.
                // Add separation so players shuffle apart when formation
                // shifts stack them together.
                let arrive = SteeringBehavior::Arrive {
                    target,
                    slowing_distance: 25.0,
                }
                .calculate(ctx.player)
                .velocity;

                return Some(arrive + ctx.player().separation_velocity() * 0.4);
            }

            // Team has ball — off-ball movement: spread across the pitch using unique player slots
            let ball_pos = ctx.tick_context.positions.ball.position;
            let ball_distance = ctx.ball().distance();

            // ANTI-FOLLOWING: If very close to ball carrier, move toward start position
            // to create space instead of calculating a volatile escape direction
            if ball_distance < 40.0 {
                let spread_target = Vector3::new(
                    start_pos.x.clamp(30.0, field_width - 30.0),
                    start_pos.y.clamp(30.0, field_height - 30.0),
                    0.0,
                );
                let dist = (spread_target - ctx.player.position).magnitude();
                if dist < 8.0 {
                    return Some(Vector3::zeros());
                }
                return Some(
                    SteeringBehavior::Arrive {
                        target: spread_target,
                        slowing_distance: 20.0,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }

            let attacking_direction = match ctx.player.side {
                Some(PlayerSide::Left) => 1.0,
                Some(PlayerSide::Right) => -1.0,
                None => 0.0,
            };

            // Quantize ball position to 20-unit grid to prevent wobble
            let qball_x = (ball_pos.x / 20.0).round() * 20.0;
            let qball_y = (ball_pos.y / 20.0).round() * 20.0;

            // Formation-based positioning: stay near start_pos, shift slightly toward ball.
            // Each player keeps their unique formation position — no slot convergence.
            let ball_pull = 0.15; // How much the ball pulls the player (low = keep formation)

            // X: mostly start_pos, pulled slightly toward ball + forward offset
            let forward_offset = attacking_direction * 40.0;
            let target_x = start_pos.x * (1.0 - ball_pull) + (qball_x + forward_offset) * ball_pull;

            // Y: mostly start_pos, pulled slightly toward ball Y
            let target_y = start_pos.y * (1.0 - ball_pull) + qball_y * ball_pull;

            let target = Vector3::new(
                target_x.clamp(30.0, field_width - 30.0),
                target_y.clamp(30.0, field_height - 30.0),
                0.0,
            );

            let dist_to_target = (target - ctx.player.position).magnitude();

            if dist_to_target < 8.0 {
                return Some(Vector3::zeros());
            }

            let arrive_velocity = SteeringBehavior::Arrive {
                target,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;

            Some(arrive_velocity)
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Midfielders cover the most ground during a match - box to box running
        // High intensity with velocity-based adjustment
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderRunningState {
    /// Phase-first dispatch. Midfielders sit in the spine of the team,
    /// so the phase cue drives more of their behaviour than any other
    /// role. Settled phases fall through to the existing decision tree.
    fn phase_dispatch(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let phase = ctx.team().phase();
        let has_ball = ctx.player.has_ball(ctx);
        let ball_dist = ctx.ball().distance();
        match phase {
            // Counter-press window after losing the ball. The closest
            // midfielder to the ball engages; others drop into
            // Returning to rebuild shape. Without this window the
            // engine had no concept of "hunt the ball back now" —
            // every press was reactive to distance alone.
            GamePhase::DefensiveTransition if !has_ball => {
                if ball_dist < 45.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
                // Further-away midfielders reset shape rather than
                // ball-chase into space — but only if there is shape to
                // reset. A midfielder already standing on his mark has
                // nothing to recover, and sending him to `Returning`
                // anyway just bounced him back the next tick (see
                // `ShapeStation`).
                if ball_dist > 80.0 && ShapeStation::should_recover(ctx) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Returning,
                    ));
                }
            }
            // Fast-break window after winning the ball. Midfielder not
            // on the ball makes a forward run to support. The carrier
            // falls through to passing/dribbling below.
            GamePhase::AttackingTransition if !has_ball => {
                if ctx.ball().distance_to_opponent_goal() > 40.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }
            }
            // Coach-triggered high press — midfielders hunt the ball in
            // the opposition half alongside the forwards.
            GamePhase::HighPress if !has_ball => {
                if ball_dist < 70.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
            }
            // Low-block: cut passing lanes by dropping into the gap
            // between defenders and the ball. Midfielders shouldn't
            // continue chasing upfield in this phase.
            GamePhase::LowBlock if !has_ball => {
                // Same shared predicate: drop back only when actually out
                // of shape. Unconditional on `ball_dist` alone, this was
                // the single biggest midfield loop in the engine.
                if ball_dist > 50.0 && ShapeStation::should_recover(ctx) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Returning,
                    ));
                }
            }
            _ => {}
        }
        None
    }

    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(ctx, 300.0)
    }

    /// Where a carrier actually runs.
    ///
    /// This pointed at the goal and added a sine wave, whatever was in
    /// the way. So a midfielder who had just decided NOT to take his man
    /// on ran straight into him anyway, every tick, until somebody took
    /// the ball — which is most of what the ball-stuck clock was
    /// measuring. A player who declines the duel does the other thing
    /// footballers do: he takes it across the defender's face, off his
    /// shoulder, looking for the angle that opens.
    fn calculate_simple_ball_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        // How much of the line ahead survives contact with the man in
        // front. Fully open lane: straight at the aim. Closed: almost
        // all of the movement goes across him.
        let lane = LaneAhead::read(ctx);

        // …and WHERE he is driving. Every carry in this engine aims at
        // the goal, which from a touchline is a diagonal into the two
        // centre-backs — the carrier cuts infield into the crowd, gets
        // shut down, and the flank is never used. A wide carrier being
        // shown outside drives the byline instead; `FlankPlay` owns the
        // condition and returns `None` for everybody else, so this is
        // the only line that changes for a central carrier.
        let goal_pos = FlankPlay::carry_aim(ctx, lane.openness)
            .unwrap_or_else(|| ctx.player().opponent_goal_position());
        let to_goal = (goal_pos - player_pos).normalize();

        // Smooth sinusoidal lateral sway instead of binary flip
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 60.0;
        let sway = phase.sin() * 0.2;
        let mut lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);
        let mut forward = 1.0f32;
        if let Some(nearest_id) = lane.nearest_id {
            // 0.85, not 1.35: letting the forward component go NEGATIVE
            // (turn all the way back) was tried and measured worse on
            // both counts it was meant to help — goals 4.27 → 4.93 and
            // the ball-stuck clock 57.8 s → 60.4 s a match, because a
            // carrier who retreats simply re-attacks the same defender.
            // Taking it across his face is the right amount of turn.
            let block = 1.0 - lane.openness;
            forward = 1.0 - block * 0.85;
            // Away from the side he is standing on, so the carrier
            // shifts onto his free side rather than picking a direction
            // out of a sine wave.
            let opp_pos = ctx.tick_context.positions.players.position(nearest_id);
            let across = Vector3::new(-to_goal.y, to_goal.x, 0.0);
            let side = if (opp_pos - player_pos).dot(&across) > 0.0 {
                -1.0
            } else {
                1.0
            };
            lateral += across * side * block;
        }

        let heading = to_goal * forward + lateral;
        let heading = if heading.magnitude() > f32::EPSILON {
            heading.normalize()
        } else {
            to_goal
        };
        let target = player_pos + heading * 40.0;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
        .calculate(ctx.player)
        .velocity
    }

    /// Does he give it, right now?
    ///
    /// This used to be a six-rung ladder of step thresholds — pressure
    /// above 0.7, above 0.5, above 0.3, above 0.2; execution above 0.30,
    /// 0.45, 0.55 — laid over a pressure metric that was itself three
    /// buckets at 1.9 m / 3.8 m / 6.3 m. Two consequences, and both are
    /// the reported behaviour:
    ///
    ///  * the pressure metric could not reach the top rungs. It needed
    ///    three opponents inside 1.9 m to clear 0.7, so the "must pass"
    ///    and "forced pass" cases were unreachable and every decision
    ///    fell through to rung 6, which reads: more than 25 m from goal,
    ///    a competent passer, anybody better placed — pass. That is the
    ///    engine's default action on the ball, and it is why it plays
    ///    861 passes a team against a real ~500;
    ///  * every rung is a cliff. Two midfielders a hundredth apart in
    ///    `pass_execution` played completely different games, and the
    ///    same midfielder played the same ball every time he stood in
    ///    the same place, which is what "scripted" looks like from the
    ///    stands.
    ///
    /// Replaced by one urge, continuous in every input, compared against
    /// a bar drawn once per possession. Nothing here is a threshold on a
    /// skill: the skills scale the urge, and the situation decides.
    // `profile` is threaded in from the caller (built once per process()
    // tick) — `from_ctx` is a pure function of the frozen tick snapshot, so
    // the passed value is bit-identical to a fresh rebuild here.
    fn should_pass(
        &self,
        ctx: &StateProcessingContext,
        profile: &MidfielderSkillProfile,
        lane: &LaneAhead,
    ) -> bool {
        let pressure = self.carry_pressure(ctx);
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // ── Reasons to let it go ──────────────────────────────────────
        // Being closed down is the big one, and it is worth more to a
        // player who cannot handle it: press resistance is what buys the
        // extra second, so it is subtracted from the urge rather than
        // compared against a bar.
        let squeezed = (pressure - profile.press_resistance * 0.55).clamp(0.0, 1.0);

        // A team-mate in a better position is a reason in proportion to
        // how much better he is and how well this player sees it.
        let outlet = self.better_placed_gain(ctx, distance_to_goal);
        let vision = profile.progressive_selection;

        // Standing on it stops being a carry and starts being a dwell.
        // Continuous, so there is no cliff-edge second at which a
        // midfielder must suddenly release.
        let dwell =
            (ctx.tick_context.ball.ownership_duration as f32 / (4.5 * TICKS_PER_SECOND)).min(1.4);

        // ── Reasons to keep it ────────────────────────────────────────
        // Room in front of him. This is the same continuous lane read
        // the carry and the take-on use, so the three cannot disagree
        // the way the old trio of hand-drawn circles did.
        let running_room = lane.openness;

        let urge = squeezed * 1.25
            + outlet * (0.35 + vision * 0.65)
            + dwell * 0.55
            + profile.pass_execution * 0.30
            - running_room * 0.85;

        // Bar drawn once per possession — see `TakeOn::decide` for why
        // this must not be a per-tick roll. A directness-minded side
        // releases sooner; a patient one holds.
        let spread = Opportunity::draw(ctx, RELEASE_SALT);
        let bar = 0.46 + spread * 0.34 + ctx.team().build_up_patience() * 0.22;

        urge >= bar
    }

    /// **The squarer ball.** A team-mate inside the area with a plainly
    /// better sight of goal than the man on the ball has.
    ///
    /// Deliberately narrow, because this vetoes a shot: he has to be
    /// materially closer in, materially straighter on, genuinely
    /// unmarked, and reachable. Anything less and the carrier takes it
    /// himself, which is also football — the pass across the six-yard
    /// box that nobody was asking for is how chances get thrown away.
    ///
    /// Unlike `find_cutback_to_arriving_runner` this is not restricted
    /// to central midfielders: the man square of an arriving eight is
    /// usually the centre-forward, and he was the one player the engine
    /// could never find.
    fn find_square_ball<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
        my_goal_distance: f32,
    ) -> Option<MatchPlayerLite> {
        /// He has to be this much closer to goal to be worth the pass
        /// (~2.5 m), and inside the area himself.
        const CLOSER: f32 = 20.0;
        const IN_THE_AREA: f32 = 132.0;
        /// …with nobody within a stride of him (~2.5 m). Same figure the
        /// cutback finder uses for "unmarked", so the two agree about
        /// what the word means.
        const UNMARKED: f32 = 20.0;
        /// …and a better ANGLE, not just a shorter distance: half the
        /// width of the six-yard box either side of centre is where a
        /// square ball is worth playing.
        const ANGLE_GAIN: f32 = 12.0;

        let goal = ctx.player().opponent_goal_position();
        let field_height = ctx.context.field_size.height as f32;
        let centre_y = field_height * 0.5;
        let my_offset = (ctx.player.position.y - centre_y).abs();

        let mut best: Option<(MatchPlayerLite, f32)> = None;
        for t in ctx.players().teammates().nearby(160.0) {
            if t.tactical_positions.is_goalkeeper() {
                continue;
            }
            let theirs = (goal - t.position).magnitude();
            if theirs > IN_THE_AREA || theirs > my_goal_distance - CLOSER {
                continue;
            }
            let their_offset = (t.position.y - centre_y).abs();
            if their_offset > my_offset - ANGLE_GAIN {
                continue;
            }
            if ctx
                .tick_context
                .grid
                .opponents(t.id, UNMARKED)
                .next()
                .is_some()
            {
                continue;
            }
            if !ctx.player().has_clear_pass(t.id) {
                continue;
            }
            // The best of them is the one with the shortest, straightest
            // sight of goal.
            let score = -(theirs + their_offset);
            if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                best = Some((t, score));
            }
        }
        best.map(|(t, _)| t)
    }

    /// How much better placed the best outlet is, 0..1 — the continuous
    /// form of `has_better_positioned_teammate`'s yes/no.
    fn better_placed_gain(&self, ctx: &StateProcessingContext, current_distance: f32) -> f32 {
        let goal = ctx.player().opponent_goal_position();
        let mut best = 0.0f32;
        for teammate in ctx.players().teammates().nearby(300.0) {
            let their_distance = (teammate.position - goal).magnitude();
            if their_distance >= current_distance {
                continue;
            }
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }
            // Space around him, not a marker count: one man three
            // metres off is a different pass from two men on his toes.
            let crowding = ctx
                .tick_context
                .grid
                .opponents(teammate.id, 4.0 * U_PER_M)
                .count() as f32;
            let freedom = 1.0 / (1.0 + crowding);
            let gain = ((current_distance - their_distance) / current_distance.max(1.0)).min(1.0);
            best = best.max(gain * freedom);
        }
        best
    }

    /// How closed down the carrier is, 0..1.
    ///
    /// A smooth kernel over every opponent near him instead of three
    /// buckets whose edges (1.9 m / 3.8 m / 6.3 m) were unit counts
    /// mistaken for metres — an opponent seven metres away contributed
    /// nothing at all, and a man closing from ten metres is exactly the
    /// pressure a carrier reacts to. Each opponent contributes on a
    /// falling curve out to `PRESSURE_REACH`, and a man actively
    /// running at him counts for more than one standing off.
    fn carry_pressure(&self, ctx: &StateProcessingContext) -> f32 {
        const PRESSURE_REACH: f32 = 11.0 * U_PER_M;
        let me = ctx.player.position;
        let mut total = 0.0f32;
        for (opp_id, dist) in ctx
            .tick_context
            .grid
            .opponents(ctx.player.id, PRESSURE_REACH)
        {
            let proximity = 1.0 - (dist / PRESSURE_REACH).clamp(0.0, 1.0);
            // Quadratic: the last two metres are worth far more than the
            // first two, which is how being closed down actually feels.
            let mut weight = proximity * proximity;
            let vel = ctx.tick_context.positions.players.velocity(opp_id);
            let speed = vel.magnitude();
            if speed > 0.2 {
                let toward = (me - ctx.tick_context.positions.players.position(opp_id)).normalize();
                let closing = vel.normalize().dot(&toward).max(0.0);
                weight *= 1.0 + closing * 0.6;
            }
            total += weight;
        }
        total.min(1.0)
    }

    /// ONE-TWO COMBINATION: Check if the player who just passed to us has run into
    /// a better forward position with space. If so, return the ball for a wall-pass.
    fn find_one_two_return<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let recent_passers = ctx.tick_context.ball.recent_passers();
        // Get the most recent passer (last element in the ring buffer vec)
        let passer_id = *recent_passers.last()?;

        // Passer must be a teammate
        let passer = ctx.context.players.by_id(passer_id)?;
        if passer.team_id != ctx.player.team_id {
            return None;
        }

        // Find passer in nearby players
        let passer_lite = ctx
            .players()
            .teammates()
            .all()
            .find(|t| t.id == passer_id)?;

        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let passer_pos = passer_lite.position;

        // Passer must now be closer to opponent goal than us (they continued their run)
        let our_goal_dist = (goal_pos - player_pos).magnitude();
        let passer_goal_dist = (goal_pos - passer_pos).magnitude();
        if passer_goal_dist >= our_goal_dist * 0.9 {
            return None; // Passer didn't run ahead enough
        }

        // Passer must be in open space (no opponents within 50 units)
        let opponents_near_passer = ctx.tick_context.grid.opponents(passer_id, 50.0).count();
        if opponents_near_passer >= 1 {
            return None;
        }

        // Must have clear passing lane back to passer
        if !ctx.player().has_clear_pass(passer_id) {
            return None;
        }

        // Passer must be within reasonable passing distance
        let pass_distance = (passer_pos - player_pos).magnitude();
        if pass_distance > 200.0 || pass_distance < 10.0 {
            return None;
        }

        Some(passer_lite)
    }

    /// DRAW AND RELEASE: Detect an opponent committing to a tackle (approaching fast
    /// within 15-35 units). Find a teammate in the space the opponent is vacating.
    fn find_draw_and_release_pass<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;

        // The man committing to the tackle: close enough to have
        // committed, far enough that the ball is still ours to move.
        //
        // The band was 15-35 UNITS — 1.9 m to 4.4 m. By the time a
        // defender is inside two metres he is not "approaching", he is
        // tackling, so the window described a moment that has already
        // passed. 2 m to 6 m is a defender coming at you.
        const COMMIT_NEAR: f32 = 2.0 * U_PER_M;
        const COMMIT_FAR: f32 = 6.0 * U_PER_M;
        let approaching_opponent = ctx
            .players()
            .opponents()
            .nearby(COMMIT_FAR)
            .filter(|opp| {
                let dist = (opp.position - player_pos).magnitude();
                if dist < COMMIT_NEAR || dist > COMMIT_FAR {
                    return false;
                }

                // Check if opponent is moving toward us
                let opp_velocity = ctx.tick_context.positions.players.velocity(opp.id);
                if opp_velocity.magnitude() < 1.0 {
                    return false;
                }

                let to_us = (player_pos - opp.position).normalize();
                let opp_dir = opp_velocity.normalize();
                opp_dir.dot(&to_us) > 0.6 // Moving toward us
            })
            .min_by(|a, b| {
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            })?;

        // The space the opponent is vacating is roughly behind them (opposite of their movement)
        let opp_velocity = ctx
            .tick_context
            .positions
            .players
            .velocity(approaching_opponent.id);
        let vacated_zone = approaching_opponent.position - opp_velocity.normalize() * 30.0;

        // Find a teammate near the vacated space (or in the channel the opponent left)
        let best_teammate = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|t| {
                let t_dist_to_vacated = (t.position - vacated_zone).magnitude();
                // Teammate should be near the vacated space or generally in that direction
                t_dist_to_vacated < 60.0
                    && ctx.player().has_clear_pass(t.id)
                    && ctx.tick_context.grid.opponents(t.id, 10.0).count() < 2
            })
            .min_by(|a, b| {
                let da = (a.position - vacated_zone).magnitude();
                let db = (b.position - vacated_zone).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            })?;

        Some(best_teammate)
    }

    /// POSSESSION RETENTION: Determine if team should retain possession rather than
    /// attack directly. True when team is comfortable (not losing), in own/mid third,
    /// and not under heavy pressure.
    fn should_retain_possession(&self, ctx: &StateProcessingContext) -> bool {
        // Never retain if losing
        if ctx.team().is_loosing() {
            return false;
        }

        // Don't retain in attacking third - keep pressing forward
        let goal_dist = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        if goal_dist < field_width * 0.35 {
            return false;
        }

        // Don't retain under heavy pressure
        if self.carry_pressure(ctx) > 0.45 {
            return false;
        }

        // Don't retain if there is a lane in front — advance and make
        // them come to us.
        if LaneAhead::read(ctx).openness > 0.55 {
            return false;
        }

        // Retain possession when team is in control
        ctx.team().is_control_ball()
    }

    /// Movement for possession retention mode: slower, more lateral, controlled tempo
    fn calculate_possession_retention_movement(
        &self,
        ctx: &StateProcessingContext,
    ) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // Move laterally rather than directly toward goal
        // Wider sinusoidal sway with slower forward progress
        let to_goal = (goal_pos - player_pos).normalize();
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 100.0; // Slower period
        let sway = phase.sin() * 0.5; // Wider lateral sway
        let lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);

        // Move toward a midfield position rather than directly at goal
        // Blend between lateral movement and slight forward progress
        let mid_y = if player_pos.y < field_height / 2.0 {
            field_height * 0.35
        } else {
            field_height * 0.65
        };
        let retention_target = Vector3::new(
            player_pos.x + to_goal.x * 15.0, // Slow forward drift
            mid_y,
            0.0,
        );

        let blended_target =
            player_pos + (retention_target - player_pos).normalize() * 20.0 + lateral * 10.0;

        SteeringBehavior::Arrive {
            target: blended_target,
            slowing_distance: 30.0,
        }
        .calculate(ctx.player)
        .velocity
            * 0.6 // Slower overall speed in retention mode
    }

    /// COUNTER-ATTACK: Detect if a counter-attack opportunity exists.
    /// True when team just won possession, opponents are high, and space ahead is open.
    fn is_counter_attack_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let ownership_duration = ctx.tick_context.ball.ownership_duration;

        // Must have just won possession (< 15 ticks)
        if ownership_duration >= 15 {
            return false;
        }

        // Ball must be on own side or midfield (counter goes forward)
        if !ctx.ball().on_own_side() {
            // Allow early midfield counters too
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let field_width = ctx.context.field_size.width as f32;
            if goal_dist < field_width * 0.4 {
                return false; // Already in attacking third, no need for counter
            }
        }

        // Count opponents ahead of ball (between ball and opponent goal)
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - ball_pos).normalize();

        let opponents_ahead = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                let to_opp = opp.position - ball_pos;
                to_opp.normalize().dot(&to_goal) > 0.3 // Opponent is ahead of ball
            })
            .count();

        // Counter-attack opportunity if few opponents ahead
        opponents_ahead < 3
    }

    /// COUNTER-ATTACK: Find a forward pass target for quick transition.
    /// Prefers forwards making runs toward goal with space around them.
    fn find_counter_attack_pass<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(300.0) {
            let to_teammate = teammate.position - player_pos;

            // Must be ahead of us (toward opponent goal)
            if to_teammate.normalize().dot(&to_goal) < 0.3 {
                continue;
            }

            // Must have space (no opponent within 10 units)
            let opponents_near = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();
            if opponents_near >= 2 {
                continue;
            }

            // Must have clear passing lane
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Score: prefer forwards, closer to goal, making runs
            let is_forward = teammate.tactical_positions.is_forward();
            let goal_dist = (goal_pos - teammate.position).magnitude();
            let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
            let making_run = teammate_velocity.magnitude() > 1.0
                && teammate_velocity.normalize().dot(&to_goal) > 0.3;

            let mut score = 1000.0 - goal_dist; // Closer to goal = better
            if is_forward {
                score += 200.0;
            }
            if making_run {
                score += 150.0;
            }
            if opponents_near == 0 {
                score += 100.0;
            }

            if let Some((_, best_score)) = &best_target {
                if score > *best_score {
                    best_target = Some((teammate, score));
                }
            } else {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }

    // `has_open_space_ahead` (no opponent within 3.75 m in a wide cone)
    // and `has_running_lane` (none within 10 m in a 40° cone) both lived
    // here and disagreed with each other and with the take-on gate's own
    // 4.4 m count. Three hand-drawn circles for one question. They are
    // now one continuous read — see `LaneAhead`.

    fn is_congested_near_boundary(&self, ctx: &StateProcessingContext) -> bool {
        // Check if near any boundary (within 20 units)
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let pos = ctx.player.position;

        let near_boundary = pos.x < 20.0
            || pos.x > field_width - 20.0
            || pos.y < 20.0
            || pos.y > field_height - 20.0;

        if !near_boundary {
            return false;
        }

        // Count all nearby players (teammates + opponents) within 15 units
        let nearby_teammates = ctx
            .tick_context
            .grid
            .teammates(ctx.player.id, 0.0, 15.0)
            .count();
        let nearby_opponents = ctx.tick_context.grid.opponents(ctx.player.id, 15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    /// Find a safe backward/lateral pass target for tempo control.
    /// Prefers defenders and GK when coach says to slow down.
    fn find_safe_backward_pass(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let own_goal = ctx.ball().direction_to_own_goal();

        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(250.0) {
            let dist = (teammate.position - player_pos).magnitude();
            if dist < 15.0 {
                continue;
            }

            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            let opp_near = ctx.tick_context.grid.opponents(teammate.id, 12.0).count();
            if opp_near >= 2 {
                continue;
            }

            let group = teammate.tactical_positions.position_group();
            let mut score = 0.0f32;

            match group {
                PlayerFieldPositionGroup::Goalkeeper => score += 45.0,
                PlayerFieldPositionGroup::Defender => score += 35.0,
                PlayerFieldPositionGroup::Midfielder => score += 10.0,
                _ => {}
            }

            // Prefer players closer to own goal (backward direction)
            let teammate_to_own = (own_goal - teammate.position).magnitude();
            let self_to_own = (own_goal - player_pos).magnitude();
            if teammate_to_own < self_to_own {
                score += 20.0;
            }

            if opp_near == 0 {
                score += 15.0;
            }

            if let Some((_, best_score)) = &best {
                if score > *best_score {
                    best = Some((teammate, score));
                }
            } else {
                best = Some((teammate, score));
            }
        }

        best.map(|(t, _)| t)
    }
}

#[cfg(test)]
mod role_gate_tests {
    use super::{MAX_SHOOTING_DISTANCE, POINT_BLANK_DISTANCE, STANDARD_SHOOTING_DISTANCE};

    /// Reproduces the two role-driven expressions in `process` without a
    /// `StateProcessingContext`, which cannot be fixtured here. Kept
    /// beside the real ones deliberately, on the same footing as
    /// `arriving_runner_tests`: if the live form changes and these do
    /// not, the tests below stop describing the engine.
    fn strike_range(role_shooting: f32) -> f32 {
        (STANDARD_SHOOTING_DISTANCE
            + role_shooting * (MAX_SHOOTING_DISTANCE - STANDARD_SHOOTING_DISTANCE))
            .min(MAX_SHOOTING_DISTANCE)
    }

    fn carry_bar(progress: f32) -> f32 {
        0.52 + progress.powf(1.6) * 0.28
    }

    /// **A holding midfielder and a number ten are not the same
    /// footballer.** They were: `process` dispatches every midfielder
    /// down one tree and nothing in it read his slot, so the deep man
    /// shot from 22 m at the same rate as the man arriving in the box.
    /// Measured, midfielders scored 61.4% of every goal against a 32%
    /// target and 49.4% of their shots came from 16.5-22 m.
    #[test]
    fn the_deep_man_shoots_from_closer_than_the_advanced_one() {
        let holder = strike_range(0.10);
        let eight = strike_range(0.30);
        let ten = strike_range(0.60);
        assert!(holder < eight && eight < ten, "{holder} {eight} {ten}");
        // …in metres, so the football is legible: 1u = 0.125 m.
        assert!((14.0..18.0).contains(&(holder / 8.0)), "{}", holder / 8.0);
        assert!((25.0..34.0).contains(&(ten / 8.0)), "{}", ten / 8.0);
    }

    /// It is a bar on the RANGE and never on the shot. Every midfielder
    /// keeps a real shooting range, and none of them gets one longer
    /// than the distance the helper itself calls hopeless.
    #[test]
    fn the_range_never_closes_and_never_runs_away() {
        for r in [0.0_f32, 0.25, 0.5, 0.75, 1.0, 1.5] {
            let range = strike_range(r);
            assert!(range >= STANDARD_SHOOTING_DISTANCE, "{range}");
            assert!(range <= MAX_SHOOTING_DISTANCE, "{range}");
            assert!(range > POINT_BLANK_DISTANCE);
        }
    }

    /// **Territory belongs in the carry.** The take-on has it and
    /// `should_pass` has it; the carry had none, so the same patch of
    /// grass licensed the same run in a player's own half and in the
    /// opposition penalty area. It is the term that stops a midfielder
    /// running the ball into the six-yard box — measured, that single
    /// behaviour moved the goal total by half a goal a match.
    #[test]
    fn a_carry_costs_more_the_closer_it_gets_to_goal() {
        let own_half = carry_bar(0.2);
        let middle = carry_bar(0.5);
        let final_third = carry_bar(0.85);
        assert!(own_half < middle && middle < final_third);
        // …and the bar stays a bar: reachable at every point on the
        // pitch by a carrier with a genuinely open lane, or the branch
        // would simply be off.
        assert!(final_third < 1.0, "{final_third}");
    }
}
