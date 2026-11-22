use super::distances::TackleEngagement;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::MAX_OWNER_TRACK_DISTANCE;
use crate::r#match::engine::context::PenaltyArea;
use crate::r#match::engine::officiating::management::{CounterAttackThreat, ProfessionalFoul};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::psychology::lapse::lapse_diag;
use crate::r#match::engine::psychology::{ActionLapse, LapseFamily, PsychState};
use crate::r#match::player::events::FoulSeverity;
use crate::r#match::player::strategies::players::ops::defender_skill::{
    DefenderSkillInputs, DefenderSkillProfile,
};
use crate::r#match::player::strategies::players::ops::skill::traits_bias::{
    defending_bias, personality_bias,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    MATCH_TIME_INCREMENT_MS, MatchContext, MatchPlayer, MatchPlayerLite, MatchRng, PlayerSide,
    StateProcessingContext,
};
use nalgebra::Vector3;

/// How far behind a challenging player a team-mate can be and still count
/// as cover (~30 m). Wide, because the question is "will somebody deal
/// with it if the carrier goes past me", not "is he standing next to me".
const COVER_SCAN: f32 = 240.0;

/// Would a foul by this player, right now, be a penalty?
///
/// # Why this is one function
///
/// Three places need this answer and two of them used to compute it
/// differently from the one that matters.
/// `PlayerEventDispatcher::award_restart_for_foul` — the code that
/// actually awards the penalty — tests **where the foul was**, the fouled
/// man's position, against the fouler's own penalty area; for a challenge
/// that is the man on the ball, which is why this reads the ball. Both
/// restraint models tested **the FOULER's position** instead.
///
/// Those differ constantly, and the gap is not symmetric: a defender
/// standing on the edge of his box challenging for a ball inside it gets
/// no restraint and concedes a penalty. Measured, that was the whole
/// story — restraining tackles on the defender's own position cut tackles
/// 22.4 → 18.3 per team and moved the penalty rate by nothing, and the
/// foul-source census then showed **41 of 43 box fouls were being emitted
/// by midfielders**, who are exactly the players standing at the edge of
/// the area rather than inside it.
///
/// So the restraint asks the referee's question, in the referee's terms.
pub struct PenaltyRisk;

impl PenaltyRisk {
    /// True when a foul by this player would be given as a penalty —
    /// the ball is inside the area his own team is defending. This is the
    /// referee's exact test, mirrored from `award_restart_for_foul`.
    pub fn applies(ctx: &StateProcessingContext) -> bool {
        Self::own_box(ctx).contains(&ctx.tick_context.positions.ball.position)
    }

    /// True when the player should be defending carefully because of
    /// where he is standing, whether or not the ball is with him yet.
    ///
    /// Both halves of this are needed and they are not the same
    /// population. Swapping the models from the standing test to the ball
    /// test alone fixed penalties (1.03 → 0.10) and sent fouls from 14.3
    /// to 23.5 per team, because most defensive engagements inside the
    /// area happen while the ball is still on its way in — a defender
    /// picking up a runner at the back post is in the box, the ball is
    /// not, and the old test was quietly suppressing every one of those.
    ///
    /// A defender is careful in his own box because of the consequences
    /// of being *there*, and doubly so once the ball arrives. Taking the
    /// union keeps the first and lets [`applies`](Self::applies) carry
    /// the second.
    pub fn in_own_box(ctx: &StateProcessingContext) -> bool {
        Self::own_box(ctx).contains(&ctx.player.position) || Self::applies(ctx)
    }

    fn own_box(ctx: &StateProcessingContext) -> PenaltyArea {
        ctx.context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
    }
}

/// The clock every engagement decision is drawn against.
///
/// Rolling once every N ticks makes the rate a function of the tick size,
/// and forfeits outright any opportunity whose tick lands on one of the
/// state's early returns — the next chance is then a whole interval away
/// because the phase is fixed by when contact began. Drawing the same
/// rate on every eligible tick removes both: over one window,
/// `1 - (1-p)^(dt/window)` compounded across `window/dt` ticks is exactly
/// `p`, at any tick size. It is `P = 1 - exp(-lambda*t)` with
/// `lambda = -ln(1-p)`, written without the transcendentals.
pub struct EngagementClock;

impl EngagementClock {
    /// Seconds of match time one full AI tick advances. The engine
    /// alternates full ticks with movement-only light ones and only the
    /// full ones run the state machine.
    pub const TICK_SECONDS: f32 = (MATCH_TIME_INCREMENT_MS * 2) as f32 / 1000.0;

    /// One AI tick's share of a rate quoted over `window` seconds.
    #[inline]
    pub fn per_tick(rate: f32, window: f32) -> f32 {
        1.0 - (1.0 - rate).powf(Self::TICK_SECONDS / window)
    }

    /// One engine tick's share — for a rate drawn by the ball, which runs
    /// on every tick, full or light.
    #[inline]
    pub fn per_engine_tick(rate: f32, window: f32) -> f32 {
        1.0 - (1.0 - rate).powf(MATCH_TIME_INCREMENT_MS as f32 / 1000.0 / window)
    }
}

/// **What a man at the carrier's feet weighs** when he decides whether to go
/// in ([`TackleDecision`]): read off his state, or off the two players where
/// the ball has them and no state to ask.
pub struct ChallengeMoment<'a> {
    pub player: &'a MatchPlayer,
    pub context: &'a MatchContext,
    /// A team-mate is goal-side of him within `COVER_SCAN`.
    pub cover: bool,
    /// He is in his own penalty area.
    pub in_own_box: bool,
    /// The ball's distance from the goal he defends, and the carrier's
    /// pace, in units.
    pub ball_to_goal: f32,
    pub carrier_speed: f32,
    /// His distance from the carrier.
    pub distance: f32,
}

impl<'a> ChallengeMoment<'a> {
    pub fn of(ctx: &'a StateProcessingContext<'a>, distance: f32) -> Self {
        let own_goal = ctx.ball().direction_to_own_goal();
        ChallengeMoment {
            player: ctx.player,
            context: ctx.context,
            cover: TackleDecision::cover_exists(ctx),
            in_own_box: PenaltyRisk::in_own_box(ctx),
            ball_to_goal: (ctx.tick_context.positions.ball.position - own_goal).magnitude(),
            carrier_speed: ctx
                .players()
                .opponents()
                .with_ball()
                .next()
                .map(|c| c.velocity(ctx).norm())
                .unwrap_or(0.0),
            distance,
        }
    }

    /// `challenger` at `carrier`'s feet, among `players`.
    pub fn between(
        challenger: &'a MatchPlayer,
        carrier: &MatchPlayer,
        players: &[MatchPlayer],
        context: &'a MatchContext,
    ) -> Self {
        let left = challenger.side == Some(PlayerSide::Left);
        let own_goal = if left {
            context.goal_positions.left
        } else {
            context.goal_positions.right
        };
        let depth = (own_goal - challenger.position).magnitude();
        ChallengeMoment {
            player: challenger,
            context,
            cover: players.iter().any(|mate| {
                mate.id != challenger.id
                    && mate.team_id == challenger.team_id
                    && !mate.tactical_position.current_position.is_goalkeeper()
                    && (own_goal - mate.position).magnitude() < depth
                    && (mate.position - challenger.position).magnitude() < COVER_SCAN
            }),
            in_own_box: context.penalty_area(left).contains(&challenger.position),
            ball_to_goal: (carrier.position - own_goal).magnitude(),
            carrier_speed: carrier.velocity.norm(),
            distance: (carrier.position - challenger.position).xy().norm(),
        }
    }
}

/// **Bringing a breaking man down on purpose.** The cynical foul a
/// defender commits when the carrier is through on goal with the lane open
/// and almost nobody left behind him — rolled at the same cadence as any
/// other challenge he could make, and only ever in contact range. The odds
/// are `ProfessionalFoul::commit_prob`: an aggressive, calculating man
/// with a lead to protect late on does it, a sporting one or one already
/// on a yellow mostly does not.
pub struct TacticalFoul;

impl TacticalFoul {
    /// Window the commit odds are quoted over, as for a tackle.
    const RATE_WINDOW: f32 = 1.0;

    pub fn threat(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> CounterAttackThreat {
        let goal = ctx.ball().direction_to_own_goal();
        CounterAttackThreat::from_positions(
            carrier,
            goal,
            ctx.player.position,
            ctx.players().teammates().all().map(|mate| {
                (
                    mate.position,
                    mate.tactical_positions.position_group()
                        == PlayerFieldPositionGroup::Goalkeeper,
                )
            }),
        )
    }

    /// Whether he brings the carrier down on this AI tick.
    pub fn commits_now(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> bool {
        let threat = Self::threat(ctx, carrier);
        let minute = (ctx.context.total_match_time / 60_000) as u32;
        let desperation = if ctx.team().score_diff() >= 0 && ctx.context.is_running_out() {
            1.0
        } else {
            0.5
        };
        let p = ProfessionalFoul::commit_prob(
            threat,
            ctx.player.skills.mental.aggression,
            ctx.player.skills.mental.decisions,
            ctx.player.attributes.sportsmanship,
            desperation,
            minute,
            ctx.player.yellow_cards > 0,
        ) * ContactFoul::box_restraint(ctx);
        p > 0.0
            && ctx
                .context
                .rng
                .bernoulli(EngagementClock::per_tick(p, Self::RATE_WINDOW))
    }
}

/// When a defender in contact range actually commits to a challenge,
/// rather than staying on his feet and containing.
///
/// # Why the volume was wrong
///
/// The tackling state attempted a challenge on every tick it was in
/// contact range and off cooldown. That produced **11.9 tackles per
/// defender per match against a real ~1.6** (84.9 per team against 18),
/// with 441k state entries collapsing to 9.5k attempts and 4.2k
/// successes. `TackleEngagement`'s own notes had already established that
/// the volume does not live in the engagement GEOMETRY — successful
/// tackles held at 92.8/team across 25u, 16u and 10u commit distances —
/// so it has to live here, in whether the attempt is made at all.
///
/// # The football
///
/// A defender does not lunge because he can reach. Most of a duel is
/// spent containing: standing the carrier up, showing him away from goal,
/// waiting. He commits when something gives him the moment — the carrier
/// takes a touch too far, commits his weight, or runs out of pitch — and
/// he commits more readily when there is somebody behind him to clean up
/// if he misses.
///
/// Every term below is continuous, so the tackle rate emerges from the
/// situation and the defender's temperament instead of a threshold.
pub struct TackleDecision;

impl TackleDecision {
    /// The minute a side behind starts to press and challenge harder.
    const URGENCY_FROM_MINUTE: f32 = 62.0;

    /// Per-DECISION commitment chance for an ordinary contain, before the
    /// situational terms. Containing is the default action in a duel, so
    /// a typical decision lands near 0.15-0.20 once the multipliers are
    /// applied — a defender who stands his man up for a second or two
    /// usually does not dive in.
    ///
    /// ⚠ RE-ANCHORED 0.16 → 0.098, AND NOT BECAUSE THE DECISION CHANGED.
    ///
    /// This is a rate per second OF CONTACT, so its calibration depends
    /// entirely on how much contact there is — and until the chase-speed
    /// inversion was fixed (see `MovementEffort::carrier_ceiling`) there
    /// was very little, because the man on the ball had a higher speed
    /// ceiling than the man chasing him and defenders spent their duels
    /// trailing at three metres rather than standing anybody up.
    ///
    /// Measured across that fix, same 120 fixtures: the carrier's nearest
    /// opponent inside 2 m went from **37% of ticks to 54%**, and mean
    /// engagement distance 3.5 m → 2.6 m. The same 0.16 then produced
    /// 29.5 successful tackles per team per match against a real ~18, and
    /// 22.4 fouls against ~12 — a foul being a failed challenge, the two
    /// move together and both are a function of this number.
    ///
    /// ⚠ RE-ANCHORED AGAIN, 0.098 → 0.073 (since re-fitted to 0.062), AND
    /// AGAIN NOT BECAUSE THE DECISION CHANGED — because the DIVISOR did.
    ///
    /// [`Self::is_eligible`] now counts time in CONTACT rather than
    /// time in the `Tackling` state, and the interval is the one real
    /// second the doc always claimed rather than two. Both corrections
    /// pull the same way: a defender who arrives on the carrier is asked
    /// on the tick he arrives, and asked again every second he stands him
    /// up. Measured over the change, 120 fixtures at L14: commit
    /// decisions **470 → 632 per match**, and inside our own penalty area
    /// — the case that had almost no decisions at all — **35.6 → 75.0**.
    ///
    /// The same 0.098 over that many more moments gave 24.5 successful
    /// tackles per team against a real ~18 and 20.0 fouls against ~12.
    /// Scaled back by the ratio the volume moved by, so the per-match
    /// rate returns to where it was calibrated and only its DISTRIBUTION
    /// changes — which is the point: challenges now land where the
    /// contact is, and a carrier in our own box draws roughly three times
    /// as many as before.
    ///
    /// 0.062 → 0.093 with the defender re-fit: a defender who walks and is
    /// set before he challenges commits less often per second of contact,
    /// and the engine was making 10.6 tackles a team against a real ~18.
    ///
    /// 0.093 → 0.15 with the midfielder re-fit: a midfielder now has the
    /// block tackle only goal-side and in contact, and the engine was making
    /// 7.4 tackles a team; this makes 10.1, with shots back in their band.
    const BASE: f32 = 0.15;

    /// Commitment multiplier inside the defender's own penalty area when
    /// there is cover behind him — see [`Self::box_restraint`].
    ///
    /// 0.22 → 0.14 alongside `ContactFoul::BOX_PENALTY_RESTRAINT`, and
    /// for the same reason: this is a per-DECISION rate, and the marking
    /// work put far more defenders into duels inside their own area, so
    /// the same rate produced a different number of penalties per match.
    ///
    /// ⚠ 0.14 → 0.32, BECAUSE 0.14 WAS NOT RESTRAINT, IT WAS ABSENCE.
    ///
    /// Measured over twelve fixtures at level 14, the commitment
    /// probability inside a defender's own area was **0.040 against 0.101
    /// in open play** — a defender challenged the ball less than half as
    /// often in the one part of the pitch where a goal is about to be
    /// conceded. What that produces is visible in a replay and was the
    /// complaint that opened this work: an attacker with the ball in the
    /// penalty area, 1.39 defenders inside three metres of him, and only
    /// 72% of those ticks carrying a live challenge — bodies standing
    /// around a man they are not trying to dispossess.
    ///
    /// The restraint being modelled is real, and it is real about the
    /// KIND of challenge, not about whether to make one. A professional
    /// in his own box stays on his feet, blocks with the front foot and
    /// does not go through the man; he does not stand off and watch. So
    /// the volume comes back and the cost per challenge goes down with
    /// it: `DefenderTacklingState`'s own in-box foul multiplier moves
    /// 0.06 → 0.032 in the same change, which holds the penalty rate
    /// roughly where it was calibrated (~1.8% of box challenges, against
    /// a real ~2.5% of which not all come from tackles) while roughly
    /// doubling how often somebody actually goes and gets it.
    ///
    /// `necessity` already multiplies a box duel by up to 2.9, so at 0.32
    /// a defender in his own area commits at about the same per-second
    /// rate as one in midfield — which is the floor of what the football
    /// asks for, not the ceiling.
    ///
    /// ⚠ 0.32 → 0.44, AND THE PRICE THAT PAID FOR 0.32 HAS SINCE FALLEN.
    ///
    /// The pairing note above is the thing to read: this number was set
    /// against the PENALTY rate, because a per-challenge foul model turns
    /// extra box challenges into extra spot kicks. 0.32 was chosen when
    /// that bought penalties at 0.300 a match against a real 0.25-0.30 —
    /// i.e. it was spending the whole budget. Re-measured over 5×200
    /// fixtures at level 14 they read **0.110-0.175, mean ~0.14** — half
    /// the real rate. Something downstream has since made box challenges
    /// cheaper, and the restraint has been quietly over-tight ever since.
    ///
    /// The report this reopens is the same one it was set for: *"they run
    /// in groups and hold players while the forwards freely pass the ball
    /// around the penalty area"*. The gate census says the bodies are
    /// there and challenging — 83% of defender-ticks inside commit range
    /// of a carrier in our own box are in a challenging state — and that
    /// the commitment roll is what refuses: **p = 0.072 in our own box
    /// against 0.111 in open play**, 75.5 decisions a match.
    ///
    /// 0.44 restores roughly a third of the gap and is sized off the
    /// penalty budget, not off the challenge count: at ~0.14 penalties a
    /// match against a real 0.25-0.30 there is room for a ×1.4 on box
    /// challenges before the spot-kick rate reaches its own reference,
    /// and both numbers move toward real together.
    const BOX_RESTRAINT: f32 = 0.44;
    /// The value before that, kept as the A/B arm — see
    /// `MatchContext::box_defence_off`.
    const BOX_RESTRAINT_LEGACY: f32 = 0.32;
    /// …and when he is the last man, where the challenge has to be made.
    const BOX_RESTRAINT_LAST_MAN: f32 = 0.72;

    /// The window [`Self::commit_probability`] is quoted over.
    ///
    /// One second is the natural cadence: roughly how long a carrier
    /// holds a shape before his next touch, which is what creates or
    /// denies the moment. It is no longer an INTERVAL — see
    /// [`EngagementClock`] — so a decision can never be forfeited by
    /// landing on a tick the state returns early from.
    const RATE_WINDOW: f32 = 1.0;

    /// Can this defender make the block tackle at all right now?
    ///
    /// [`MatchPlayer::contact_ticks`] counts dwell inside
    /// [`TackleEngagement::CONTACT`] of an opposing CARRIER, which is the
    /// thing the rate is about, and nothing about which state he is in —
    /// so a hand-off between the states that share one duel does not
    /// reset it. It is advanced before the state machine runs, so the
    /// first tick in contact already counts.
    pub fn is_eligible(ctx: &StateProcessingContext) -> bool {
        ctx.player.contact_ticks > 0
    }

    /// [`Self::commit_probability`] drawn over this AI tick.
    pub fn commits_now(ctx: &StateProcessingContext, distance: f32) -> f32 {
        EngagementClock::per_tick(Self::commit_probability(ctx, distance), Self::RATE_WINDOW)
    }

    /// Probability this defender commits to a challenge at this decision.
    ///
    /// `distance` is defender-to-carrier; closer is a better moment
    /// because the angle to the ball is better.
    pub fn commit_probability(ctx: &StateProcessingContext, distance: f32) -> f32 {
        let p = Self::odds(&ChallengeMoment::of(ctx, distance));
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::DuelDiag::note_decision(p, PenaltyRisk::in_own_box(ctx));
        p
    }

    /// The same odds, drawn over one engine tick, for the ball's scan of a
    /// man in contact with the carrier — see [`ChallengeMoment::between`].
    pub fn commits_this_engine_tick(moment: &ChallengeMoment) -> f32 {
        EngagementClock::per_engine_tick(Self::odds(moment), Self::RATE_WINDOW)
    }

    fn odds(moment: &ChallengeMoment) -> f32 {
        // ── …AND EVERY ATTRIBUTE IN IT IS READ AGAINST THIS MATCH ──────
        //
        // The `tackling` term below has always been centred on the
        // calibration division's population mean, for the reason its own
        // note gives. That fixes the LEVEL and not the GRADIENT: the
        // slope is steep enough that the term alone spans −0.45 at the
        // bottom of the pyramid to 0.00 at the top, and `aggression`,
        // `decisions` and `anticipation` were not centred at all.
        //
        // Measured, `dev_match stats 150 L L` (CHALLENGE GATE CENSUS,
        // equal squads, so none of this is a mismatch):
        //
        // | level | commit p | tackles/DEF | carrier-in-our-box ticks |
        // |---|---|---|---|
        // | 6  | 0.050 | 1.08 | 18 140 |
        // | 8  | 0.067 | 1.19 | 12 929 |
        // | 12 | 0.110 | 1.80 |  4 119 |
        // | 14 | 0.126 | 2.07 |  3 842 |
        // | 20 | 0.161 | 3.35 |  5 869 |
        //
        // A real back four makes ~1.6 challenges a match in every
        // division. The fourth tier never challenges the carrier and the
        // top flight challenges three times as often, and the third
        // column is the consequence the whole engine feels: an attacker
        // holds the ball inside the area **4.7× longer** at the bottom of
        // the pyramid, which is the chance SUPPLY that `SHOT_BAR_BASE`'s
        // own note names as the residual it cannot price ("nobody stops
        // them getting into the box at all").
        //
        // The squads are measured against the standard of the match at
        // kickoff (`MatchStandard::measure`), so each attribute reads
        // against the football around it and the within-division spread
        // is untouched: the aggressive defender still dives in, the
        // thoughtful one still picks his moment.
        let read = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let skills = &moment.player.skills;
        let aggression = read(skills.mental.aggression);
        let decisions = read(skills.mental.decisions);
        let anticipation = read(skills.mental.anticipation);

        // Temperament. An aggressive defender dives in; a good
        // decision-maker picks his moment, which means fewer challenges
        // but better ones. They pull in opposite directions on purpose.
        //
        // …and how good a tackler he actually IS decides whether the
        // moment looks like a moment at all. This model read no technical
        // attribute whatever, so a striker with `tackling` 5 weighed a
        // challenge exactly as a centre-half with 17 does — which is why
        // the front line kept out-tackling the back four however the duel
        // gate was arranged: **2.51 tackles per forward per match against
        // a real ~0.8, with defenders on 1.23 against ~1.6**, measured
        // over 120 fixtures with the plan already winning the gate.
        //
        // A striker presses to cut the pass and force the error; he
        // rarely goes to ground, because he is not good at it and knows
        // it. A centre-half's whole trade is the challenge. That is one
        // continuous attribute, not a role switch, and it costs the
        // calibration nothing: the term is centred on the MEASURED
        // population mean (`dev_match audit_levels`: outfield `tackling`
        // 14.03 at squad level 14, the harness's calibration level), so
        // the average player's commitment rate is unchanged and only the
        // spread is new. Centring it on a guessed 12/20 instead moved the
        // whole population — total tackles 18.0 → 19.8 per team — which
        // is a calibration change wearing a modelling change's clothes.
        //
        // `tackle_profile` already prices whether he WINS it. This prices
        // whether he goes — the two are different questions and the
        // second one had no answer.
        let tackling = read(skills.technical.tackling);
        let temperament =
            (0.55 + aggression * 0.90 - decisions * 0.30 + (tackling - 0.70) * 1.10).max(0.12);

        // Cover behind me. This is the single biggest real-world licence
        // to commit: with a spare man you can afford to miss, without one
        // a failed tackle is a clear run at goal. The plan knows who the
        // cover is, so this is a fact rather than a guess.
        let cover_licence = if moment.cover { 1.55 } else { 0.75 };

        // Necessity. Near our own goal the cost of NOT engaging rises
        // faster than the cost of missing — this is where last-ditch
        // challenges come from.
        // 1.0 on the goal line, 0 at ~30 m out.
        let danger = (1.0 - moment.ball_to_goal / 240.0).clamp(0.0, 1.0);
        let necessity = 1.0 + danger * 1.9;

        // The moment. A carrier moving quickly across or past the
        // defender has committed his weight and can be challenged; one
        // standing still shielding cannot. Anticipation is how well the
        // defender reads that.
        let exposure = (moment.carrier_speed / 0.45).clamp(0.0, 1.0);
        let timing = 1.0 + exposure * anticipation * 1.1;

        // Angle. At the outer edge of contact range the ball is a lunge
        // away; at his feet it is a clean block tackle.
        let proximity =
            (1.0 - moment.distance / TackleEngagement::CONTACT.max(1.0)).clamp(0.0, 1.0);
        let reach = 0.65 + proximity * 0.70;

        (Self::BASE
            * temperament
            * cover_licence
            * necessity
            * timing
            * reach
            * Self::urgency(moment.player, moment.context)
            * Self::box_restraint(moment.in_own_box, moment.cover)
            * Self::booked_caution(moment.player)
            * Self::trait_temper(moment.player))
        .clamp(0.0, 0.55)
    }

    /// A man on a yellow picks his moments, because the next mistimed one
    /// is a red. A clear-headed player holds back more than a hothead.
    /// Read where he decides whether to go in, not after he has — a booked
    /// player who commits is no less likely to get it wrong.
    pub fn booked_caution(player: &MatchPlayer) -> f32 {
        if player.yellow_cards == 0 {
            return 1.0;
        }
        let decisions = (player.skills.mental.decisions / 20.0).clamp(0.0, 1.0);
        let aggression = (player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        (0.80 - 0.25 * decisions + 0.15 * aggression).clamp(0.5, 0.95)
    }

    /// His own nature: a man who dives into tackles goes in more readily,
    /// one who stays on his feet less, and one who winds opponents up
    /// carries the needle into every duel.
    pub fn trait_temper(player: &MatchPlayer) -> f32 {
        let diving_in = defending_bias(player).tackle_attempt_threshold_delta;
        let needle = personality_bias(player).own_card_risk_delta;
        (1.0 - diving_in * 2.5 + needle * 5.0).clamp(0.5, 1.6)
    }

    /// How much the state of the match makes this defender go and get it.
    ///
    /// Nothing in the model above knows the score. A side a goal down with
    /// ten minutes left presses and challenges far more than the same side
    /// at 0-0, and a side protecting a lead drops off and stays on its
    /// feet — this is one of the most visible things in real football and
    /// the engine had no channel for it at all.
    ///
    /// The size of the swing is the player's, through
    /// [`skill_composites::resilience`]: `determination` heads that blend,
    /// and before this the attribute reached exactly one thing in the
    /// whole engine — a secondary mitigation inside the fatigue model's
    /// late-game mental penalty. A 19-determination player and a
    /// 5-determination player behaved identically at 0-1 down in the 85th
    /// minute, which is the one moment the attribute is *about*.
    ///
    /// It ramps through the closing half-hour, and `OF_SCORE_BLIND`
    /// switches it off with every other read of the score, or the A/B
    /// control means nothing.
    fn urgency(player: &MatchPlayer, context: &MatchContext) -> f32 {
        if !context.behavioral_score_visible() {
            return 1.0;
        }
        let minute = sc::minute_from_ms(context.total_match_time);
        let from = Self::URGENCY_FROM_MINUTE;
        // Scaled with the rest of the regime — see
        // `MatchContext::SCORE_REACTION_GAIN`.
        let pressure = ((minute as f32 - from) / (90.0 - from)).clamp(0.0, 1.0)
            * MatchContext::score_reaction_gain();
        let home = player.team_id == context.field_home_team_id;
        let (mine, theirs) = if home {
            (
                context.score.home_team.get() as i32,
                context.score.away_team.get() as i32,
            )
        } else {
            (
                context.score.away_team.get() as i32,
                context.score.home_team.get() as i32,
            )
        };
        let deficit = theirs - mine;
        if deficit == 0 {
            return 1.0;
        }
        // Two goals down is as urgent as it gets — three down is a
        // different kind of resignation, not more chasing.
        let magnitude = (deficit.abs().min(2) as f32) / 2.0;
        let drive = sc::resilience(player, minute);
        if deficit > 0 {
            // Behind: chase. A driven player goes after it much harder
            // than a passenger.
            1.0 + pressure * magnitude * (0.10 + drive * 0.35)
        } else {
            // Ahead: see it out. Determination reads as game management
            // here, not as diving in — the driven player is the one who
            // stays on his feet and holds the shape.
            1.0 - pressure * magnitude * (0.08 + drive * 0.22)
        }
    }

    /// How much a defender holds back from a challenge because he is
    /// inside his own penalty area.
    ///
    /// `necessity` above is right that the cost of NOT engaging rises
    /// toward your own goal — but it peaks at the EDGE of the box, not
    /// inside it. Inside, the cost of the challenge itself jumps
    /// discontinuously: a missed tackle is a penalty and usually a card,
    /// so the professional response is to stay on your feet, show him
    /// wide, and block the shot. Real defenders visibly stop diving in
    /// there, which is why penalties are ~1% of all fouls.
    ///
    /// [`ContactFoul::probability`] has always modelled this — its own
    /// box restraint is 0.004, and its comment says "same restraint the
    /// tackle model applies". **The tackle model did not apply it.** The
    /// shirt-pull path was restrained and the sliding-tackle path was
    /// *encouraged*, by `necessity`, in exactly the same square of grass.
    ///
    /// It stayed invisible while the pitch was sparse. Compacting the
    /// team shape put far more bodies in and around the box, `necessity`
    /// multiplied every one of those engagements by up to 2.9, and
    /// penalties went from 0.27 a match to 1.0 — against a real 0.25-0.30.
    ///
    /// The last-ditch challenge survives: when the carrier has beaten
    /// everyone and this defender is the last man, the restraint lifts,
    /// because at that point conceding a penalty really is better than
    /// conceding a goal. That is the case the dramatic box challenge is
    /// FOR, and it is rare — which is why the rate it produces is rare too.
    fn box_restraint(in_own_box: bool, cover: bool) -> f32 {
        if !in_own_box {
            return 1.0;
        }
        if cover {
            // Somebody is behind me — there is no excuse at all.
            // A/B control — see `MatchContext::box_defence_off`.
            if MatchContext::box_defence_off() {
                return Self::BOX_RESTRAINT_LEGACY;
            }
            Self::BOX_RESTRAINT
        } else {
            // Last man. Still more careful than in open play, but this is
            // the challenge that has to be made.
            Self::BOX_RESTRAINT_LAST_MAN
        }
    }

    /// Is there somebody behind me if I miss?
    ///
    /// Asked geometrically rather than from the plan. The plan's `Cover`
    /// duty only exists for the back line, so reading it directly told
    /// every midfielder in the game that he had no cover — and midfield
    /// is the one part of the pitch that always does, because the whole
    /// defence is behind it. That suppressed midfield challenges to 628
    /// attempts against the defenders' 1,172, and dragged successful
    /// tackles to 12.9 per team against a real ~18.
    ///
    /// "A team-mate goal-side of me and near enough to deal with it" is
    /// the question a player actually asks, it works for every role, and
    /// it does not depend on which unit the plan happens to cover.
    ///
    /// ⚠ THE GOALKEEPER IS NOT COVER, AND COUNTING HIM MADE
    /// [`Self::BOX_RESTRAINT_LAST_MAN`] DEAD CODE.
    ///
    /// The scan walked the whole roster, and the keeper is goal-side of
    /// every outfielder who is not standing on his own goal line and
    /// well inside `COVER_SCAN` of anybody defending his own box. So
    /// inside our own penalty area — the one place the last-man branch
    /// exists for — `cover_exists` was unconditionally true: every box
    /// challenge took `BOX_RESTRAINT` (0.14) and `cover_licence` 1.55,
    /// and the 0.60 arm written for "the carrier has beaten everyone and
    /// I am the last man" could not be reached.
    ///
    /// He is also the wrong answer to the question. "Is there somebody
    /// behind me if I miss" asks who deals with the carrier once he is
    /// past me; the keeper is the thing being protected, and a defender
    /// who misses in front of him has produced a one-on-one, which is
    /// the opposite of cover. Excluding him is what makes a genuine last
    /// man behave like one: `box_restraint` reads its last-man value (0.72
    /// against 0.44 with cover) and `cover_licence` its lower one, on
    /// exactly the duels where a real defender does commit.
    fn cover_exists(ctx: &StateProcessingContext) -> bool {
        let own_goal = ctx.ball().direction_to_own_goal();
        let me = ctx.player.position;
        let my_depth = (own_goal - me).magnitude();
        ctx.players().teammates().all().any(|t| {
            t.id != ctx.player.id
                && !t.tactical_positions.is_goalkeeper()
                && (own_goal - t.position).magnitude() < my_depth
                && (t.position - me).magnitude() < COVER_SCAN
        })
    }

    /// How dangerous the man on the ball is, on the same absolute
    /// composite scale as [`sc::defensive_duel`] — so a duel scored as
    /// the difference of the two is level for equal quality at every
    /// level of the pyramid.
    pub fn carrier_threat(ctx: &StateProcessingContext, opponent: &MatchPlayerLite) -> f32 {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        if let Some(att) = ctx.context.players.by_id(opponent.id) {
            return sc::dribble_attack(att, minute);
        }
        let players = ctx.player();
        let s = players.skills(opponent.id);
        (sc::n(s.technical.dribbling) + sc::n(s.physical.agility)) * 0.5
    }
}

/// How one physical challenge ended.
///
/// Replaces a `(won, fouled, severity)` tuple whose two booleans could
/// both be true and whose precedence lived at the call site — and
/// differed between roles.
/// **The block tackle, decided once.** A challenge from in front of the
/// carrier, at his feet: won on the two men's duel scores, and a foul a share
/// of the time, more often when it fails. Every path that makes one resolves
/// it here — the defender's `Tackling`, and the scan's challenge for an owned
/// ball (`Ball::check_ball_ownership`).
pub struct BlockChallenge {
    /// The challenger's duel score (`sc::defensive_duel`).
    pub tackle: f32,
    /// The carrier's (`sc::dribble_attack`).
    pub carry: f32,
    /// The challenger's aggression, 1..20.
    pub aggression: f32,
    /// `DefenderSkillProfile::discipline` and `def_condition_mult`.
    pub discipline: f32,
    pub condition: f32,
    /// The ball is in the challenger's own penalty area.
    pub in_own_box: bool,
    /// The challenger already carries a yellow card.
    pub booked: bool,
    /// How much he was still diving in, 0..1 (`Self::rush_of`).
    pub rush: f32,
}

impl BlockChallenge {
    /// What a man diving in costs: at a full rush his odds fall by this share
    /// and his foul rises by this much again.
    const RUSH_ODDS: f32 = 0.40;
    const RUSH_FOUL: f32 = 0.80;
    /// AI ticks set before he has his full value back, half a second.
    const SET_TICKS: f32 = 25.0;

    /// A/B control: with `OF_DEFENDER_DUEL=off` how he arrives does not count,
    /// a man containing may sprint, and the closing boosts are back. Read
    /// once per process — debug infrastructure, do not remove.
    pub fn duel_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_DEFENDER_DUEL").is_ok_and(|v| v == "off"))
    }

    /// How much he is still diving in as he challenges, 0..1: his closing
    /// speed above the carrier's, as a share of his top speed, fading to
    /// nothing as he has been set at the man for half a second.
    pub fn rush_of(
        challenger: &MatchPlayer,
        carrier: Vector3<f32>,
        carrier_velocity: Vector3<f32>,
    ) -> f32 {
        if Self::duel_off() {
            return 0.0;
        }
        let Some(towards) = (carrier - challenger.position).xy().try_normalize(1.0e-4) else {
            return 0.0;
        };
        let closing = challenger.velocity.xy().dot(&towards);
        let carrier_pace = carrier_velocity.xy().norm();
        let excess = ((closing - carrier_pace)
            / challenger.max_speed_with_condition_cached().max(1.0e-3))
        .clamp(0.0, 1.0);
        let set = (challenger.contact_ticks as f32 / Self::SET_TICKS).clamp(0.0, 1.0);
        excess * (1.0 - set)
    }

    /// The challenge `challenger` makes at `carrier`'s feet, priced from the
    /// two men as they stand — for a path that has the players and not a
    /// state to ask (the ball's scan, a claim on a carried ball).
    pub fn between(
        challenger: &MatchPlayer,
        carrier: &MatchPlayer,
        context: &MatchContext,
    ) -> Self {
        let minute = sc::minute_from_ms(context.total_match_time);
        let left = challenger.side == Some(PlayerSide::Left);
        let (own_goal, their_goal) = if left {
            (context.goal_positions.left, context.goal_positions.right)
        } else {
            (context.goal_positions.right, context.goal_positions.left)
        };
        let profile = DefenderSkillProfile::from_player(
            challenger,
            &DefenderSkillInputs {
                minute,
                condition_pct: challenger.player_attributes.condition_percentage() as f32 / 100.0,
                pressure_count_5u: 0,
                pressure_count_10u: 0,
                distance_to_own_goal: (challenger.position - own_goal).magnitude(),
                distance_to_opponent_goal: (challenger.position - their_goal).magnitude(),
                recent_high_intensity: false,
            },
        );
        BlockChallenge {
            tackle: sc::defensive_duel(challenger, minute),
            carry: sc::dribble_attack(carrier, minute),
            aggression: challenger.skills.mental.aggression,
            discipline: profile.discipline,
            condition: profile.def_condition_mult,
            in_own_box: context.penalty_area(left).contains(&carrier.position),
            booked: challenger.yellow_cards > 0,
            rush: Self::rush_of(challenger, carrier.position, carrier.velocity),
        }
    }

    /// The block tackle's odds: the duel's two composites, and nothing of
    /// the pitch — he stays on his feet, so a wet surface carries no slide.
    pub fn success_chance(tackle: f32, carry: f32) -> f32 {
        let raw_diff = tackle - carry;
        (1.0 / (1.0 + (-raw_diff * 2.4).exp())).clamp(0.06, 0.55)
    }

    pub fn resolve(&self, rng: &MatchRng, psychology: Option<&PsychState>) -> TackleOutcome {
        let aggression01 = (self.aggression / 20.0).clamp(0.0, 1.0);

        // ⚠ BOTH SIDES OF THE DUEL MUST BE ON ONE SCALE.
        //
        // This scored `tackle_profile` against the carry composite, two
        // quantities on different scales, and the difference left the
        // standard of the match in the result: the same equal-quality
        // matchup won 55% at the bottom of the pyramid and 24% at the
        // top. `defensive_duel` is the same family of composite as
        // `dribble_attack`, so the difference is level at every level. The profile still drives the decision, the
        // movement and the discipline — it is a selection model, not a
        // contest one.
        let set_odds =
            Self::success_chance(self.tackle, self.carry) * (1.0 - Self::RUSH_ODDS * self.rush);
        let tackle_success = rng.random::<f32>() < set_odds;

        // Foul chance driven by discipline (composure/decisions/
        // concentration/tackling blend) instead of raw composure +
        // aggression. Tired defenders foul more (low def_condition_mult
        // adds to foul rate). Failed tackles roughly double the rate.
        //
        // Base 0.030 → 0.052 (2026-06 discipline recalibration): the
        // engine whistled 6.3 fouls/team vs real ~12 — duel contact was
        // committing too cleanly given the volume of attempts. Raising
        // the contact-foul rate (paired with the referee Normal-band
        // lift) brings whistled fouls, free kicks, and the
        // persistent-infringement card pipeline toward real rates.
        // Second lift 0.052 → 0.075: the first round's gain was eaten by
        // the Reckless probability gate reclassifying card-fouls into
        // normal contact (whistled at ~0.5 instead of ~0.85), netting
        // only +10% whistles. Real per-duel foul rates are ~15-16%; the
        // engine sat at ~10% after round one.
        // A FOUL IS A FAILED CHALLENGE, and the rate has to be read
        // against the number of challenges actually made.
        //
        // These constants were fitted when the engine attempted ten times
        // as many tackles as football does — the per-attempt rate had to
        // be tiny to keep the foul TOTAL anywhere near real. With the
        // jockey model bringing attempts down to a realistic count, the
        // same rate produced 2.1 fouls per team against a real ~12, 0.88
        // yellows against 3.5-4.5, and 1.5 direct free kicks against
        // 20-24: the whole disciplinary chain starved.
        //
        // Real football makes roughly 30 challenges a team and fouls on
        // about 12 of them, so a challenge that does not win the ball
        // cleanly is a foul a large fraction of the time. Lifted ~3× to
        // match, with the own-box restraint and the booking damping below
        // still holding penalties and second yellows down.
        let mut base_foul = 0.34 + aggression01 * 0.34 - self.discipline * 0.20;
        if !tackle_success {
            base_foul *= 1.80;
        }
        base_foul += (1.0 - self.condition).max(0.0) * 0.08;
        base_foul *= 1.0 + Self::RUSH_FOUL * self.rush;
        // Own-box restraint: real defenders stay on their feet inside
        // their own penalty area — they jockey, contain and block
        // rather than commit, because the downside is a spot kick.
        // Without this, the foul-rate lift would inflate penalties
        // (already over real rate before the lift). Uses the actual
        // penalty-area RECTANGLE (the same one the restart award
        // checks) — an earlier radius-from-goal proxy missed the box
        // corners and let wide-channel box fouls escape the restraint.
        // ⚠ 0.008 → 0.06, AND THE OLD VALUE WAS BELOW ITS OWN CLAMP.
        //
        // `base_foul` before this line is 0.34-0.68 for a plausible
        // defender, so ×0.008 lands on 0.003-0.005 — under the
        // `clamp(0.006, …)` two lines down. Every in-box challenge
        // therefore fouled at exactly 0.6%, identically, whatever the
        // defender's aggression, discipline or condition and whether or
        // not he won it: the constant was saturating a floor rather than
        // expressing a tendency, and any value below ~0.012 produced the
        // same match.
        //
        // It was also the THIRD restraint stacked on one event.
        // `TackleDecision::box_restraint` already prices whether he
        // commits at all (0.44 with cover, 0.72 as the last man) and
        // `ContactFoul::BOX_PENALTY_RESTRAINT` prices the non-challenge
        // fouls; between them the tackle path was contributing on the
        // order of 0.02 penalties a match and the engine sat at 0.18
        // against a real 0.25-0.30 — under the target while reading as
        // if the penalty budget were spent, which is why every previous
        // pass over this area correctly refused to loosen anything.
        //
        // 0.06 says a challenge in your own box is about fifteen times
        // less likely to be a foul than the same challenge in open play
        // — he goes in with his feet and not through the man — which is
        // a tendency rather than a floor, and lets the attributes
        // upstream reach the outcome again.
        //
        // ⚠ 0.06 → 0.032, PAIRED WITH `TackleDecision::BOX_RESTRAINT`.
        //
        // That constant went 0.14 → 0.32 because a defender was
        // challenging the carrier in his own area less than half as often
        // as in open play, which is not how anybody defends a penalty
        // box. This is a per-CHALLENGE rate and the challenge count
        // roughly doubles with it, so holding it at 0.06 would have
        // bought the defending with penalties: the pair together keep the
        // per-match penalty rate near where it was calibrated.
        //
        // It is also the more honest of the two numbers. The whole reason
        // a defender may commit inside his own box is that he commits
        // DIFFERENTLY there — front foot, ball first, no follow-through —
        // and the rate at which that produces a spot kick is the thing
        // this line is for. ~1.8% of box challenges, against a real ~2.5%
        // of which the shirt-pull path owns a share of its own.
        if self.in_own_box {
            base_foul *= 0.032;
        }
        if self.booked {
            base_foul *= 0.70;
        }
        let foul_chance = ActionLapse::scaled(
            LapseFamily::Challenge,
            base_foul.clamp(0.006, 0.60),
            psychology,
        );

        let committed_foul = rng.random::<f32>() < foul_chance;
        #[cfg(feature = "match-logs")]
        lapse_diag::note(LapseFamily::Challenge, committed_foul);

        let box_severity = if self.in_own_box { 0.30 } else { 1.0 };
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression01 > 0.75
            && !tackle_success
            && rng.random::<f32>() < 0.008 * box_severity
        {
            FoulSeverity::Violent
        } else if !tackle_success
            && aggression01 > 0.55
            && rng.random::<f32>() < 0.16 * box_severity
        {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        TackleOutcome::of(tackle_success, committed_foul, severity)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TackleOutcome {
    /// He took the ball. Winning it cleanly is not a foul.
    Won,
    Foul(FoulSeverity),
    Missed,
}

impl TackleOutcome {
    /// The two rolls, resolved in football order.
    pub fn of(won: bool, fouled: bool, severity: FoulSeverity) -> Self {
        if won {
            Self::Won
        } else if fouled {
            Self::Foul(severity)
        } else {
            Self::Missed
        }
    }
}

/// The challenge a BEATEN defender makes — the poke, the stretch, the
/// slide across from behind.
///
/// # Why this exists
///
/// The engine had exactly one challenge: the block tackle, made front-on
/// from a goal-side jockey position inside [`TackleEngagement::CONTACT`]
/// (1.25 m). Every defender who was not in front of the carrier had
/// nothing at all to do about him.
///
/// The jockey ([`ClosingPoint`](super::ClosingPoint)) sends him a metre
/// goal-side of the carrier, and for a man who is LEVEL WITH or
/// BEHIND the carrier that point is *past the carrier*: reaching it means
/// overtaking, and a defender's speed edge over a man on the ball is
/// about two per cent. So he runs at a point he can never arrive at, on a
/// heading that necessarily parallels the carrier's, and rolls nothing
/// for as long as the carry lasts.
///
/// Measured over 120 fixtures at L14 (CLOSING CENSUS,
/// `mid_run_diag::CLOSE_SAMPLES`): the nearest defender to a moving
/// carrier sits **2.66 m** off him and is running PARALLEL on **47%** of
/// samples — and the worst state on the pitch for it is `Tackling`, 51%
/// of the samples at 56% parallel, the one state whose whole job is to
/// win the ball. The CHALLENGE GATE CENSUS says the same thing from the
/// other side: of the players in a `Tackling` state, only **49% are
/// inside contact range**, the only band in which an attempt is ever
/// rolled. 22% are at 2 m, 12% at 3 m.
///
/// Those men are not out of position. Two and a half metres from a
/// carrier is where a real defender makes a real challenge — he throws a
/// leg at it. He just cannot do it front-on, so it is a different action
/// with a different risk: longer reach, a far worse chance of winning it
/// cleanly, and a far better chance of taking the man. That is where a
/// large share of real yellow cards come from.
///
/// # What it is NOT
///
/// It is not a licence to lunge. A defender who is GOAL-SIDE of the
/// carrier keeps doing what he does now — he jockeys, closes to contact,
/// and blocks — because from in front the stretch is the wrong choice and
/// modelling it would re-open the "defenders lunge at everything" defect
/// the jockey model was built to close. The two are disjoint by side: a
/// man in front of the carrier contains and blocks, and only one who has
/// been gone past stretches — at the carrier's heels as much as a stride
/// off him.
pub struct RecoveryChallenge;

impl RecoveryChallenge {
    /// How far a defender reaches when he stretches for it.
    ///
    /// **The distance at which a player can kick a ball** —
    /// [`MAX_OWNER_TRACK_DISTANCE`], which is also `KICKABLE_DISTANCE`,
    /// and the engine's own answer to "close enough to play it". A poke
    /// tackle is a kick, so it is the same number, and using anything
    /// wider would let a defender win a ball the ownership layer then has
    /// to TELEPORT to him: `secure_ball_for` draws an owned ball to its
    /// owner over a few ticks below this distance and snaps it beyond,
    /// which its own note calls "a grant to somebody who was never at the
    /// ball".
    ///
    /// A real slide covers more ground than this, because the defender's
    /// body travels with it — but this engine resolves a challenge as an
    /// instant, and a model that wins the ball from further than a man
    /// can reach is not a slide, it is a teleport with a name.
    pub const REACH: f32 = MAX_OWNER_TRACK_DISTANCE;

    /// Per-DECISION commitment for an ordinary recovery, before the
    /// situational terms.
    ///
    /// Lower than [`TackleDecision::BASE`] would suggest for the same
    /// dwell, and deliberately: most of what a beaten defender does is
    /// keep running and hope. The stretch is the exception, not the rule
    /// — but it has to be POSSIBLE, and before this it was not.
    const BASE: f32 = 0.085;

    /// The window [`Self::commit_probability`] is quoted over — the same
    /// one second the block tackle uses, so how readily a given defender
    /// goes is one property of the player. See [`EngagementClock`].
    const RATE_WINDOW: f32 = 1.0;

    /// Can this beaten defender stretch for it at all right now?
    ///
    /// Keyed to [`MatchPlayer::stretch_ticks`], which counts dwell inside
    /// [`Self::REACH`] rather than inside `CONTACT`: a defender running
    /// at the carrier's shoulder two metres off him accrues no contact at
    /// all, so the block tackle's clock never starts for him.
    pub fn is_eligible(ctx: &StateProcessingContext) -> bool {
        ctx.player.stretch_ticks > 0
    }

    /// [`Self::commit_probability`] drawn over this AI tick.
    pub fn commits_now(ctx: &StateProcessingContext, distance: f32, lead: f32) -> f32 {
        EngagementClock::per_tick(
            Self::commit_probability(ctx, distance, lead),
            Self::RATE_WINDOW,
        )
    }

    /// How far the carrier has got past this defender, in units, along
    /// the line to the goal being defended.
    ///
    /// Positive means the defender is goal-side — in front of the man,
    /// where the block tackle belongs. Zero is shoulder to shoulder.
    /// Negative means he has been gone past.
    pub fn lead(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> f32 {
        let own_goal = ctx.ball().direction_to_own_goal();
        match (own_goal - carrier).try_normalize(0.01) {
            Some(to_goal) => (ctx.player.position - carrier).dot(&to_goal),
            None => 0.0,
        }
    }

    /// How comprehensively the man has gone past, 0 to 1.
    ///
    /// Scaled on [`TackleEngagement::DISENGAGE`] — the distance at which
    /// a defender gives the carrier up — because that is what "past me"
    /// means in this engine. Scaling it on `CONTACT` instead saturates:
    /// measured, a recovery challenge is made with the man a mean 1.26 m
    /// past his defender, which is already a full `CONTACT`, so every
    /// challenge in the census took the maximum of every term this
    /// multiplies and the model had no gradient left where all its
    /// samples live.
    fn beaten(lead: f32) -> f32 {
        (-lead / TackleEngagement::DISENGAGE).clamp(0.0, 1.0)
    }

    /// Can this defender stretch for it at all?
    ///
    /// Inside his own reach, off cooldown, and not in front of the man.
    /// The goal-side test is `lead <= 0` with no slack: a defender with any
    /// of the carrier's body between him and the goal is jockeying, and
    /// jockeying is right.
    pub fn is_available(ctx: &StateProcessingContext, distance: f32, lead: f32) -> bool {
        !Self::legacy() && distance <= Self::REACH && lead <= 0.0 && ctx.player.can_attempt_tackle()
    }

    /// Probability this defender throws a leg at it now.
    ///
    /// Shares the football [`TackleDecision`] already prices — a defender
    /// dives in because of who he is and because of what it costs not to
    /// — and adds the one term that is specific to this action: **how
    /// beaten he is**. A man who is a stride behind still believes he can
    /// get across and stays on his feet. One who has been gone past has
    /// nothing left but the stretch, and takes it.
    pub fn commit_probability(ctx: &StateProcessingContext, distance: f32, lead: f32) -> f32 {
        let read = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let skills = &ctx.player.skills;
        let aggression = read(skills.mental.aggression);
        let decisions = read(skills.mental.decisions);
        let tackling = read(skills.technical.tackling);
        // Same shape and same centring as `TackleDecision`'s temperament,
        // so how readily a given defender goes is one property of the
        // player rather than two unrelated numbers.
        let temperament =
            (0.55 + aggression * 0.90 - decisions * 0.30 + (tackling - 0.70) * 1.10).max(0.12);

        // How beaten am I — see [`Self::beaten`].
        let beaten = Self::beaten(lead);
        let desperation = 0.55 + beaten * 1.10;

        // Reach. At the edge of a full stretch this is a hopeful lunge;
        // just outside block range it is a poke he expects to make.
        let over = ((distance - TackleEngagement::CONTACT)
            / (Self::REACH - TackleEngagement::CONTACT))
            .clamp(0.0, 1.0);
        let stretch = 1.0 - over * 0.65;

        // Necessity, exactly as the block tackle prices it: the cost of
        // NOT engaging rises toward your own goal, and this is the
        // challenge last-ditch defending is made of.
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_goal = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        let danger = (1.0 - ball_to_goal / 240.0).clamp(0.0, 1.0);
        let necessity = 1.0 + danger * 1.9;

        let p = (Self::BASE
            * temperament
            * desperation
            * stretch
            * necessity
            * TackleDecision::box_restraint(
                PenaltyRisk::in_own_box(ctx),
                TackleDecision::cover_exists(ctx),
            )
            * TackleDecision::booked_caution(ctx.player)
            * TackleDecision::trait_temper(ctx.player))
        .clamp(0.0, 0.45);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::RecoveryDiag::note_decision(p);
        p
    }

    /// Resolve the stretch.
    ///
    /// # The three ways it ends
    ///
    /// A recovery challenge is a much worse bet than a block tackle and
    /// the model says so on both axes. He is reaching round or through a
    /// man who is moving away from him, so he wins it cleanly far less
    /// often, and when he does not the odds are he has taken the man
    /// rather than simply missed. `beaten` and the reach drive both: a
    /// poke at the shoulder is a fair challenge, a slide across a man who
    /// is already past is where the cards come from.
    pub fn resolve(
        ctx: &StateProcessingContext,
        defender_score: f32,
        discipline: f32,
        attacker_score: f32,
        distance: f32,
        lead: f32,
    ) -> TackleOutcome {
        let rng = &ctx.context.rng;
        let aggression = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let beaten = Self::beaten(lead);
        let over = Self::over(ctx, distance);
        let success = Self::success_chance(ctx, defender_score, attacker_score, distance, lead);
        let won = rng.random::<f32>() < success;

        // Fouling. A block tackle that misses often just misses; a
        // stretch that misses has usually caught the man, which is the
        // whole reason a beaten defender hesitates before making it.
        //
        // ⚠ RE-FITTED AGAINST THE CENSUS, 0.30 BASE → 0.13.
        //
        // The first cut of this model produced **63% fouls, 12% wins,
        // 25% misses** over 4.85 challenges a match (RECOVERY CHALLENGE
        // census, 300 fixtures at L14). A defender who takes the man two
        // times in three every time he stretches is not a footballer, he
        // is a booking generator: it carried team fouls 15.9 → 16.4 and
        // yellows 4.83 → 5.01 a match against a real 3.5-4.5.
        //
        // The cause was saturation rather than the base — `beaten` was
        // scaled on `CONTACT` and the mean challenge is made with the man
        // 1.26 m past, so every sample took the maximum of every term.
        // [`Self::beaten`] now scales on `DISENGAGE` and the terms below
        // are re-fitted over the range that actually occurs. The target
        // shape is a genuinely bad option that is still worth taking when
        // the alternative is letting him go: roughly a third fouls, a
        // quarter wins, the rest an honest miss.
        let mut foul = 0.13 + aggression * 0.20 - discipline * 0.12;
        if !won {
            foul *= 1.25 + beaten * 0.55;
        }
        foul *= 1.0 + over * 0.35;
        // The same own-box restraint the block tackle applies, and for
        // the same reason: inside your own area the downside of catching
        // him is a spot kick, and real defenders visibly stop doing it.
        // See `DefenderTacklingState::attempt_block_tackle`.
        if PenaltyRisk::applies(ctx) {
            foul *= 0.06;
        }
        if ctx.player.yellow_cards > 0 {
            foul *= 0.70;
        }
        let fouled = rng.random::<f32>() < foul.clamp(0.006, 0.70);

        // Severity. A challenge from behind on a man going away from you
        // is the professional foul, and it carries a heavier tail than a
        // front-on miss — but violence is still a once-in-many-matches
        // event, so the top band stays as thin as the block tackle's.
        let severity = if !fouled {
            FoulSeverity::Normal
        } else if aggression > 0.75 && !won && rng.random::<f32>() < 0.008 {
            FoulSeverity::Violent
        } else if !won && rng.random::<f32>() < 0.10 + beaten * 0.16 {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::RecoveryDiag::note_attempt(won, fouled, distance, lead);
        TackleOutcome::of(won, fouled, severity)
    }

    /// Winning it. The duel is the same contest of ability the block
    /// tackle scores — `defensive_duel` against the carry — handicapped by
    /// the two things that make this challenge hard: he is behind, and he
    /// is at full stretch. The ceiling is well under the block tackle's
    /// 0.55 because from here even a good defender is mostly hoping. It is
    /// the challenge that slides, so it is the one a wet pitch carries.
    pub(crate) fn success_chance(
        ctx: &StateProcessingContext,
        defender_score: f32,
        attacker_score: f32,
        distance: f32,
        lead: f32,
    ) -> f32 {
        let handicap = 0.55 + Self::beaten(lead) * 0.85 + Self::over(ctx, distance) * 0.55;
        let raw_diff = defender_score - attacker_score - handicap * 0.45;
        (1.0 / (1.0 + (-raw_diff * 2.4).exp()) + ctx.context.conditions.sliding_tackle_success)
            .clamp(0.05, 0.40)
    }

    /// How far past contact range he is reaching, 0..1. A wet surface
    /// carries a slide, so the same reach is less of a stretch on it — the
    /// reach itself stays the distance a man can play a ball from.
    fn over(ctx: &StateProcessingContext, distance: f32) -> f32 {
        let carry = ctx.context.conditions.slide_tackle_range_units;
        ((distance - TackleEngagement::CONTACT - carry) / (Self::REACH - TackleEngagement::CONTACT))
            .clamp(0.0, 1.0)
    }

    /// Diagnostic switch: with `OF_NO_RECOVERY_TACKLE` set, a beaten
    /// defender has no challenge available again and the engine reverts
    /// to the block tackle as its only one.
    ///
    /// The A/B control for this work — it reaches every duel on every
    /// tick, so the effect cannot be read off a diff, and it must not be
    /// answered by checking out an older revision either, because the
    /// working tree moves underneath you. Same pattern and purpose as
    /// `LooseBallChase::tail_chase`; read once per process. Debug
    /// infrastructure — do not remove.
    pub fn legacy() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| std::env::var("OF_NO_RECOVERY_TACKLE").is_ok())
    }
}

/// Fouls that are not tackle attempts — the shirt pull on a runner, the
/// block across his path, the hold at the shoulder.
///
/// # Why this exists
///
/// Tackling was the engine's **only** source of fouls, and the numbers
/// say that cannot reach football. Traced end to end: ~25 failed
/// challenges per team, ~45% of which roll a foul, ~55% of which the
/// referee whistles, gives ~6 fouls per team against a real ~12. Every
/// rate in that chain is defensible on its own; the chain is simply too
/// short. Free kicks starved with it (3.3 per match against 20-24) and
/// so did the card pipeline (2.4 yellows against 3.5-4.5).
///
/// Real defending fouls constantly without attempting a tackle. A player
/// who has been beaten grabs a shirt; one tracking a runner leans across
/// him; one marking at a set piece holds. None of those are challenges
/// for the ball, and none of them existed here.
///
/// Priced per two seconds in contact and drawn through [`EngagementClock`],
/// so the rate is a property of the defending and does not move when the
/// marker's state does.
pub struct ContactFoul;

impl ContactFoul {
    /// Seconds the per-engagement probability is quoted over.
    pub(crate) const RATE_WINDOW: f32 = 2.0;
    /// Close enough for contact (~2.5 m).
    const CONTACT_RANGE: f32 = 20.0;
    /// Chance over [`Self::RATE_WINDOW`] for an ordinary engagement.
    ///
    /// Fitted on the engagement clock against realism 300, once churn had
    /// fallen and marking spells had grown: 0.013 read 28.4 fouls a match and
    /// this reads 23.7, against 22.5 before the clock.
    ///
    /// The two foul models are coupled through dwell: a defender who
    /// declines the challenge stays engaged, and this rate runs for as long
    /// as he is. Anything that lengthens engagements raises the fouls here.
    const BASE: f32 = 0.0104;

    /// Restraint when a foul here would be a PENALTY (the ball is in our
    /// own area).
    ///
    /// Fitted to the real penalty rate rather than asserted. The previous
    /// 0.004 was written as "nobody grabs a shirt inside his own area",
    /// which is an exaggeration of a real tendency: the shirt-pull at a
    /// corner and the clumsy challenge on a crosser are precisely where
    /// real penalties come from, and at 0.004 the engine emitted **four
    /// box contacts in twenty matches** and awarded 0.08 penalties a match
    /// against a real 0.25-0.30.
    ///
    /// 0.30 said a defender is about three times less likely to foul with
    /// the ball in his own box than in open play — the real tendency, at
    /// a strength that produced the real rate **for the engagement volume
    /// of the day**.
    ///
    /// Re-fitted down from 0.30 when the marking work landed. This model
    /// rolls once a second *per engagement*, so its output scales with
    /// how many defenders are engaged, and putting the back line on its
    /// men took box contacts from 95 to 154 per 200 matches (`FOUL SOURCE
    /// CENSUS`, `Defender: Standing` 42 → 114) with penalties following
    /// 0.35 → 0.55 a match against a real 0.25-0.30. The rate per
    /// engagement was never the thing that was calibrated; the rate per
    /// MATCH was, and the divisor moved.
    ///
    /// Fitted in two steps because the first (0.17) recovered only a
    /// third of it — 0.55 → 0.45 — which is the measurement that says the
    /// box contacts really had roughly tripled rather than risen 60%.
    ///
    /// The engagement clock raised it again, because the resets it replaced
    /// fell hardest on marking spells near the box, and marking spells grew
    /// again when churn fell: 0.38-0.43 penalties a match with 0.06 here, and
    /// 0.30 at this value (n=300).
    const BOX_PENALTY_RESTRAINT: f32 = 0.0375;
    /// Restraint when the player is in his own area but the ball is not
    /// yet — marking a runner at the back post, tracking into the box.
    /// Careful, but not the catastrophic case. Re-fitted with the one above.
    const BOX_POSITION_RESTRAINT: f32 = 0.034;

    /// Nobody grabs a shirt when the downside is a penalty, and real
    /// defenders visibly stop doing it. Graded, because "would this be a
    /// penalty" and "am I standing in the box" are different questions with
    /// different answers — see `PenaltyRisk`. The professional foul reads
    /// the same restraint.
    pub(crate) fn box_restraint(ctx: &StateProcessingContext) -> f32 {
        if PenaltyRisk::applies(ctx) {
            Self::BOX_PENALTY_RESTRAINT
        } else if PenaltyRisk::in_own_box(ctx) {
            Self::BOX_POSITION_RESTRAINT
        } else {
            1.0
        }
    }

    /// Whether this engagement becomes a foul on this AI tick. A hazard on
    /// the time spent in contact, so nothing about the marker's state can
    /// reset, delay or repeat it.
    pub fn commits_now(ctx: &StateProcessingContext, gap: f32, losing_him: bool) -> bool {
        let p = Self::probability(ctx, gap, losing_him);
        p > 0.0 && ctx.context.rng.bernoulli(Self::hazard(p))
    }

    /// One AI tick's share of a probability quoted over the rate window.
    #[inline]
    pub(crate) fn hazard(p: f32) -> f32 {
        EngagementClock::per_tick(p, Self::RATE_WINDOW)
    }

    /// Probability this engagement becomes a foul now.
    ///
    /// `gap` is the distance to the man being engaged. `losing_him` is
    /// true when he is pulling away — which is when a beaten defender
    /// actually grabs, and the single biggest driver of this kind of
    /// foul.
    pub fn probability(ctx: &StateProcessingContext, gap: f32, losing_him: bool) -> f32 {
        if gap > Self::CONTACT_RANGE {
            return 0.0;
        }
        // …measured against this match, exactly as `TackleDecision` is
        // and for the same reason. A foul is dead-ball time in somebody's
        // final third, and an uncentred temperament spends far more of
        // the top flight's match with the ball out of play: measured over
        // `dev_match stats 16 L L`, fouls ran **9.9 a team at level 6
        // against 18.0 at level 18** and direct free kicks 10.2 against
        // 18.3, which resets both shapes every time.
        let skills = &ctx.player.skills;
        let aggression = (skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let discipline =
            ((skills.mental.composure + skills.mental.concentration) / 40.0).clamp(0.0, 1.0);

        // Temperament — the same pull the tackle model uses.
        let temperament = (0.55 + aggression * 0.95 - discipline * 0.35).clamp(0.2, 1.6);
        // Being beaten is what turns contact into a foul.
        let desperation = if losing_him { 2.1 } else { 1.0 };
        // Touch-tight contact fouls more than a covering position.
        let proximity = (1.0 - gap / Self::CONTACT_RANGE).clamp(0.0, 1.0);
        let closeness = 0.55 + proximity * 0.90;

        let box_restraint = Self::box_restraint(ctx);

        // His head scales what is left: a nervous marker grabs more.
        ActionLapse::scaled(
            LapseFamily::Challenge,
            (Self::BASE
                * temperament
                * desperation
                * closeness
                * box_restraint
                * TackleDecision::booked_caution(ctx.player)
                * TackleDecision::trait_temper(ctx.player))
            .clamp(0.0, 0.30),
            ctx.context.psychology.get(ctx.player.id),
        )
    }

    /// Severity of a non-tackle foul. These are overwhelmingly cynical
    /// rather than dangerous — a shirt pull is a yellow at worst — so the
    /// reckless tail is thin and there is no violent one.
    ///
    /// The `losing_him` tail was trimmed `0.10 + aggr·0.22` → `0.06 +
    /// aggr·0.14` alongside [`Self::BOX_PENALTY_RESTRAINT`], and for the
    /// same reason. `losing_him` is true whenever the man is pulling away
    /// from his marker, so its frequency rose with the number of live
    /// marking duels — red cards went **0.24 → 0.46 a match against a
    /// real 0.15-0.20** while total fouls rose only 21%, which is the
    /// signature of a severity tail being sampled more often rather than
    /// of more fouls.
    pub fn severity(ctx: &StateProcessingContext, losing_him: bool) -> FoulSeverity {
        let aggression = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        // Stopping a man who has gone past you is the professional foul.
        let reckless_p = if losing_him {
            0.06 + aggression * 0.14
        } else {
            0.04
        };
        if ctx.context.rng.unit_f32() < reckless_p {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        }
    }
}
