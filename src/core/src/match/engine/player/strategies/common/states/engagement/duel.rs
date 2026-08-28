use super::distances::TackleEngagement;
use crate::r#match::engine::ball::ball::MAX_OWNER_TRACK_DISTANCE;
use crate::r#match::engine::context::PenaltyArea;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::player::events::FoulSeverity;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, PlayerSide, StateProcessingContext};
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
/// [`FoulResolver::award_restart_for_foul`] — the code that actually
/// awards the penalty — tests **the BALL's position** against the
/// fouler's own penalty area. Both restraint models tested **the
/// FOULER's position** instead.
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
    /// ⚠ RE-ANCHORED AGAIN, 0.098 → 0.073, AND AGAIN NOT BECAUSE THE
    /// DECISION CHANGED — because the DIVISOR did.
    ///
    /// [`Self::is_decision_tick`] now counts time in CONTACT rather than
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
    const BASE: f32 = 0.062;

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
    const BOX_RESTRAINT: f32 = 0.32;
    /// …and when he is the last man, where the challenge has to be made.
    const BOX_RESTRAINT_LAST_MAN: f32 = 0.72;

    /// How often the decision is taken while containing, in AI ticks.
    ///
    /// ONE ROLL PER MOMENT, not one per tick — the same discipline
    /// `intercept_rolled` / `save_rolled` / `block_rolled` enforce
    /// elsewhere, and for the same reason. Rolling every tick makes the
    /// rate a function of how long the defender happens to stay in range
    /// rather than of the defending: at 100 ticks in contact and any
    /// per-tick probability above ~3%, a challenge becomes a certainty,
    /// so the tackle cooldown silently remained the only real limiter and
    /// the whole decision was decorative. Measured that way it moved
    /// tackles per defender only 11.9 → 10.3 against a real 1.6.
    ///
    /// One second is the natural cadence: it is roughly how long a
    /// carrier holds a shape before his next touch, which is what creates
    /// or denies the moment.
    ///
    /// ⚠ 100 → 50, AND THAT IS A UNITS FIX, NOT A RATE CHANGE.
    ///
    /// The engine alternates full AI ticks with movement-only light ones
    /// and only the full ones run the state machine, so **one AI tick is
    /// 20 ms**, not 10 (`MatchPlayer::in_state_time`, `game_tick_light`).
    /// 100 was therefore a TWO-second cadence while the comment above —
    /// and the whole argument for the number — says one. 50 is what the
    /// doc has always described.
    const DECISION_INTERVAL_TICKS: u16 = 50;

    /// Is this tick one on which the defender re-decides?
    ///
    /// ⚠ THE CLOCK IS TIME IN CONTACT, NOT TIME IN THE STATE.
    ///
    /// This read `ctx.in_state_time`, and the comment claimed "entry
    /// always counts, so a defender arriving on a carrier who has already
    /// lost control can challenge immediately". Neither half held.
    /// `Tackling` is entered from up to 25u — every one of the five
    /// box-emergency routes hands over at that range, and so does
    /// `Pressing` — while an attempt is only ever rolled inside
    /// [`TackleEngagement::CONTACT`] (10u), and the state's own distance
    /// guard `return None`s above this call. So the entry roll was spent
    /// three metres away and discarded, and the defender then contained
    /// in silence until the phase came round again.
    ///
    /// Measured: **46% of the players in a `Tackling` state are inside
    /// contact**, and a carrier inside our own area — where a possession
    /// lasts a second or two — drew 20 commit decisions a match between
    /// every defender on the pitch. The commitment model was not
    /// declining those duels; it was never asked about them.
    ///
    /// [`MatchPlayer::contact_ticks`] counts the thing the cadence is
    /// about. It is incremented before the state machine runs, so the
    /// first tick in contact reads 1 and the roll happens on arrival.
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        let t = ctx.player.contact_ticks;
        t > 0 && (t - 1) % Self::DECISION_INTERVAL_TICKS == 0
    }

    /// Probability this defender commits to a challenge at this decision.
    ///
    /// `distance` is defender-to-carrier; closer is a better moment
    /// because the angle to the ball is better.
    pub fn commit_probability(ctx: &StateProcessingContext, distance: f32) -> f32 {
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
        // Subtracting [`MatchStandard::shift`] reads each attribute
        // against the football around it instead of against a yardstick
        // from another league. It is exactly neutral at the calibration
        // level — the shift is 0 there — and leaves the within-division
        // spread untouched: the aggressive defender still dives in, the
        // thoughtful one still picks his moment.
        let shift = MatchStandard::shift(ctx.context);
        let peer = |v: f32| (v / 20.0 - shift).clamp(0.0, 1.0);
        let skills = &ctx.player.skills;
        let aggression = peer(skills.mental.aggression);
        let decisions = peer(skills.mental.decisions);
        let anticipation = peer(skills.mental.anticipation);

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
        let tackling = peer(skills.technical.tackling);
        let temperament =
            (0.55 + aggression * 0.90 - decisions * 0.30 + (tackling - 0.70) * 1.10).max(0.12);

        // Cover behind me. This is the single biggest real-world licence
        // to commit: with a spare man you can afford to miss, without one
        // a failed tackle is a clear run at goal. The plan knows who the
        // cover is, so this is a fact rather than a guess.
        let cover_licence = if Self::cover_exists(ctx) { 1.55 } else { 0.75 };

        // Necessity. Near our own goal the cost of NOT engaging rises
        // faster than the cost of missing — this is where last-ditch
        // challenges come from.
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_goal = (ctx.tick_context.positions.ball.position - own_goal).magnitude();
        // 1.0 on the goal line, 0 at ~30 m out.
        let danger = (1.0 - ball_to_goal / 240.0).clamp(0.0, 1.0);
        let necessity = 1.0 + danger * 1.9;

        // The moment. A carrier moving quickly across or past the
        // defender has committed his weight and can be challenged; one
        // standing still shielding cannot. Anticipation is how well the
        // defender reads that.
        let carrier_speed = ctx
            .players()
            .opponents()
            .with_ball()
            .next()
            .map(|c| c.velocity(ctx).norm())
            .unwrap_or(0.0);
        let exposure = (carrier_speed / 0.45).clamp(0.0, 1.0);
        let timing = 1.0 + exposure * anticipation * 1.1;

        // Angle. At the outer edge of contact range the ball is a lunge
        // away; at his feet it is a clean block tackle.
        let proximity = (1.0 - distance / TackleEngagement::CONTACT.max(1.0)).clamp(0.0, 1.0);
        let reach = 0.65 + proximity * 0.70;

        let p = (Self::BASE
            * temperament
            * cover_licence
            * necessity
            * timing
            * reach
            * Self::urgency(ctx)
            * Self::box_restraint(ctx))
        .clamp(0.0, 0.55);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::DuelDiag::note_decision(p, PenaltyRisk::in_own_box(ctx));
        p
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
    /// Gated on [`MatchContext::behavioral_score_visible`], like every
    /// other score-reactive read in the engine — the score-reaction
    /// regime is deliberately bounded to the closing half-hour to cap its
    /// draw-correlation budget, and `OF_SCORE_BLIND` has to switch ALL of
    /// it off or the A/B control means nothing. Within that window
    /// `pressure` ramps continuously to the whistle, so there is no tick
    /// where a match visibly changes character.
    fn urgency(ctx: &StateProcessingContext) -> f32 {
        if !ctx.context.behavioral_score_visible() {
            return 1.0;
        }
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let from = MatchContext::SCORE_REACTION_FROM_MINUTE as f32;
        // Scaled with the rest of the regime — see
        // `MatchContext::SCORE_REACTION_GAIN`.
        let pressure = ((minute as f32 - from) / (90.0 - from)).clamp(0.0, 1.0)
            * MatchContext::score_reaction_gain();
        let home = ctx.player.team_id == ctx.context.field_home_team_id;
        let (mine, theirs) = if home {
            (
                ctx.context.score.home_team.get() as i32,
                ctx.context.score.away_team.get() as i32,
            )
        } else {
            (
                ctx.context.score.away_team.get() as i32,
                ctx.context.score.home_team.get() as i32,
            )
        };
        let deficit = theirs - mine;
        if deficit == 0 {
            return 1.0;
        }
        // Two goals down is as urgent as it gets — three down is a
        // different kind of resignation, not more chasing.
        let magnitude = (deficit.abs().min(2) as f32) / 2.0;
        let drive = sc::resilience(ctx.player, minute);
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
    fn box_restraint(ctx: &StateProcessingContext) -> f32 {
        if !PenaltyRisk::in_own_box(ctx) {
            return 1.0;
        }
        if Self::cover_exists(ctx) {
            // Somebody is behind me — there is no excuse at all.
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
    /// man behave like one: `box_restraint` 0.14 → 0.60 and
    /// `cover_licence` 1.55 → 0.75, a net ×2.1 on exactly the duels
    /// where a real defender does commit.
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

    /// Where a containing defender stands: goal-side of the carrier, a
    /// stride off him. This is the jockey — he is between his man and the
    /// goal, close enough to challenge the moment the touch is loose, and
    /// NOT running through him.
    pub fn contain_position(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> Vector3<f32> {
        let own_goal = ctx.ball().direction_to_own_goal();
        let to_goal = (own_goal - carrier)
            .try_normalize(0.01)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        carrier + to_goal * TackleEngagement::CONTACT * 0.8
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
/// [`TackleDecision::contain_position`] sends him at
/// `carrier + toward_own_goal x 1 m`, and for a man who is LEVEL WITH or
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
/// the jockey model was built to close. The two bands are disjoint by
/// construction: inside `CONTACT` only [`TackleDecision`] fires, outside
/// it only this does, and only to a man who has been gone past.
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

    /// How often the decision is taken, in AI ticks — one real second,
    /// the same cadence [`TackleDecision`] uses and for the same reason.
    /// One AI tick is 20 ms (see `ai_tick_is_20ms`).
    const DECISION_INTERVAL_TICKS: u16 = 50;

    /// Is this tick one on which the beaten defender decides?
    ///
    /// Keyed to [`MatchPlayer::stretch_ticks`], which counts dwell inside
    /// [`Self::REACH`] rather than inside `CONTACT`. A defender running
    /// at the carrier's shoulder two metres off him accrues no contact at
    /// all, so `TackleDecision`'s clock never starts for him — which is
    /// the whole reason this population rolls nothing today.
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        let t = ctx.player.stretch_ticks;
        t > 0 && (t - 1) % Self::DECISION_INTERVAL_TICKS == 0
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
    /// Outside block-tackle range, inside his own reach, off cooldown,
    /// and not in front of the man. The goal-side test is `lead <= 0`
    /// with no slack: a defender with any of the carrier's body between
    /// him and the goal is jockeying, and jockeying is right.
    pub fn is_available(ctx: &StateProcessingContext, distance: f32, lead: f32) -> bool {
        !Self::legacy()
            && distance > TackleEngagement::CONTACT
            && distance <= Self::REACH
            && lead <= 0.0
            && ctx.player.can_attempt_tackle()
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
        let shift = MatchStandard::shift(ctx.context);
        let peer = |v: f32| (v / 20.0 - shift).clamp(0.0, 1.0);
        let skills = &ctx.player.skills;
        let aggression = peer(skills.mental.aggression);
        let decisions = peer(skills.mental.decisions);
        let tackling = peer(skills.technical.tackling);
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
            * TackleDecision::box_restraint(ctx))
        .clamp(0.0, 0.45);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::RecoveryDiag::note_decision(p);
        p
    }

    /// Resolve the stretch: `(won the ball, fouled, severity)`.
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
        tackle_profile: f32,
        discipline: f32,
        attacker_score: f32,
        distance: f32,
        lead: f32,
    ) -> (bool, bool, FoulSeverity) {
        let rng = &ctx.context.rng;
        let aggression = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let beaten = Self::beaten(lead);
        let over = ((distance - TackleEngagement::CONTACT)
            / (Self::REACH - TackleEngagement::CONTACT))
            .clamp(0.0, 1.0);

        // Winning it. The duel is the same contest of ability the block
        // tackle scores — `tackle_profile` against the carry — handicapped
        // by the two things that make this challenge hard: he is behind,
        // and he is at full stretch. The ceiling is well under the block
        // tackle's 0.55 because from here even a good defender is mostly
        // hoping.
        let handicap = 0.55 + beaten * 0.85 + over * 0.55;
        let raw_diff = tackle_profile - attacker_score - handicap * 0.45;
        let success = (1.0 / (1.0 + (-raw_diff * 2.4).exp())).clamp(0.05, 0.40);
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
        // See `DefenderTacklingState::attempt_sliding_tackle`.
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
        (won, fouled, severity)
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
/// Rolled on the same one-decision-per-second cadence as
/// [`TackleDecision`], for the same reason: a per-tick roll makes the
/// rate a function of how long two players happen to stand near each
/// other rather than of the defending.
pub struct ContactFoul;

impl ContactFoul {
    /// How often the decision is taken while engaged, in AI ticks — so
    /// **two real seconds**, not one. One AI tick is 20 ms: the engine
    /// alternates full ticks with movement-only light ones and only the
    /// full ones advance `in_state_time` (see `game_tick_light`).
    ///
    /// Left at 100 deliberately. Unlike [`TackleDecision`]'s clock this
    /// one is anchored to the right thing already — dwell in the
    /// engagement is exactly what a shirt-pull is a function of — so the
    /// only defect was the doc, and `BASE` below is fitted against this
    /// cadence and a foul rate that is already 16.3 per team against a
    /// real ~12. Halving the interval here doubles contact fouls and
    /// nothing else; it is a calibration change wearing a units fix's
    /// clothes.
    const DECISION_INTERVAL_TICKS: u64 = 100;
    /// Close enough for contact (~2.5 m).
    const CONTACT_RANGE: f32 = 20.0;
    /// Per-decision chance for an ordinary engagement.
    ///
    /// Re-fitted (0.034 → 0.017) after the box restraint landed on
    /// [`TackleDecision`]. The two foul models are coupled through DWELL:
    /// a defender who declines the challenge stays engaged, and this model
    /// rolls once a second *for as long as he is engaged*. Restraining
    /// tackles in and around the box therefore lengthened engagements and
    /// **tripled** defender contact fouls (435 → 1062 emitted per 20
    /// matches), taking the whistled rate from 14.3 to 23.6 per team
    /// against a real ~12. The base was fitted when engagements were
    /// shorter; this is the same rate expressed over the new dwell.
    const BASE: f32 = 0.021;

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
    /// Re-fitted 0.30 → 0.11 when the marking work landed. This model
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
    const BOX_PENALTY_RESTRAINT: f32 = 0.11;
    /// Restraint when the player is in his own area but the ball is not
    /// yet — marking a runner at the back post, tracking into the box.
    /// Careful, but not the catastrophic case.
    const BOX_POSITION_RESTRAINT: f32 = 0.10;

    /// Is this tick one on which a contact foul is considered?
    pub fn is_decision_tick(ctx: &StateProcessingContext) -> bool {
        ctx.in_state_time > 0 && ctx.in_state_time % Self::DECISION_INTERVAL_TICKS == 0
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
        let shift = MatchStandard::shift(ctx.context);
        let peer = |v: f32| (v - shift).clamp(0.0, 1.0);
        let skills = &ctx.player.skills;
        let aggression = peer(skills.mental.aggression / 20.0);
        let discipline = peer((skills.mental.composure + skills.mental.concentration) / 40.0);

        // Temperament — the same pull the tackle model uses.
        let temperament = (0.55 + aggression * 0.95 - discipline * 0.35).clamp(0.2, 1.6);
        // Being beaten is what turns contact into a foul.
        let desperation = if losing_him { 2.1 } else { 1.0 };
        // Touch-tight contact fouls more than a covering position.
        let proximity = (1.0 - gap / Self::CONTACT_RANGE).clamp(0.0, 1.0);
        let closeness = 0.55 + proximity * 0.90;

        // Nobody grabs a shirt when the downside is a penalty, and real
        // defenders visibly stop doing it. Same test the tackle model
        // uses — see `PenaltyRisk`.
        // Nobody grabs a shirt when the downside is a penalty. Graded,
        // because "would this be a penalty" and "am I standing in the
        // box" are different questions with different answers — see
        // `PenaltyRisk`.
        let box_restraint = if PenaltyRisk::applies(ctx) {
            Self::BOX_PENALTY_RESTRAINT
        } else if PenaltyRisk::in_own_box(ctx) {
            Self::BOX_POSITION_RESTRAINT
        } else {
            1.0
        };

        (Self::BASE * temperament * desperation * closeness * box_restraint).clamp(0.0, 0.30)
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
