//! Positional discipline — the elastic tether that keeps a player inside
//! the space his team has given him.
//!
//! # Why this is a movement-layer rule and not per-state logic
//!
//! [`TeamShape`](crate::r#match::TeamShape) hands every player a live
//! anchor, and the resting states (`Walking`, `Standing`, `Returning`)
//! steer at it. Rewiring those states moved the measured block length by
//! 1.4 m, because **players are almost never in them**. The state census
//! (`dev_match stats`, level 14, twelve matches) says where the match
//! actually goes:
//!
//! | state | share of AI ticks | mean anchor lag |
//! |---|---|---|
//! | Defender: Marking | 14.2% | 15.4 m |
//! | Midfielder: Supporting Attack | 12.8% | 16.2 m |
//! | Defender: Running | 9.3% | 15.5 m |
//! | Midfielder: Running | 7.9% | 15.4 m |
//! | Midfielder: Returning | 7.4% | 23.9 m |
//! | Forward: Creating Space | 6.1% | 17.0 m |
//! | Forward: Running | 4.6% | 28.8 m |
//!
//! Six states own more than half the match, each one is thousands of
//! lines of ball-relative reasoning, and the mean player sits 16-20 m
//! from the place his team wants him **at all times** — which is the
//! 85 m block, restated per player.
//!
//! Editing twenty state machines to each remember the shape is how the
//! engine got here: every one of them re-derives a destination from the
//! same global geometry, and they cannot see each other. The rule belongs
//! where all movement already converges — one place, applied once.
//!
//! # The rule
//!
//! A player has **freedom inside his zone and a leash outside it.** Up to
//! [`SLACK`] from his anchor his own state decides entirely; past that a
//! recall component fades in, and by [`SLACK`] + [`RANGE`] the shape has
//! the casting vote. That is what a positional game actually is: the
//! coach gives you a space, you do what you like in it, and if you leave
//! it somebody has to be told to cover.
//!
//! # What is exempt
//!
//! Everything to do with playing the ball. You break shape to make a
//! tackle, to press the carrier, to receive a pass, to finish a chance —
//! that is football, and a tether that fought it would produce eleven
//! players politely holding a formation while the opposition walked
//! through them. The exemptions are listed in
//! [`ShapeDiscipline::is_exempt`] and each one is a case where the ball,
//! not the shape, is the player's job this instant.

use crate::PlayerFieldPositionGroup;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::teamplay::tactical::inputs::TeamSkillAggregates;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, StateProcessingContext};
use nalgebra::Vector3;

/// How far a player may stray from his anchor before the shape says
/// anything at all. 64u = 8 m — a real positional zone: a player covers
/// roughly that radius on his own before a team-mate is expected to
/// shuffle across for him.
///
/// Calibrated, not picked. The first draft used 120u (15 m), which is
/// what a zone *feels* like — and measured almost nothing, because the
/// mean anchor lag was 15-17 m, so the leash sat exactly at the average
/// player's existing error and engaged for nobody. A tether whose slack
/// equals the drift it is meant to correct is a no-op.
const SLACK: f32 = 48.0;

/// Over how much further the recall fades from nothing to full. At
/// `SLACK + RANGE` = 128u = 16 m the player is out of his zone by more
/// than twice its radius and the shape wins outright.
///
/// The curve has to be steep as well as early. A shallow one settles at
/// an equilibrium rather than pulling players home: the recall grows with
/// distance, the state's outward intent does not, so they balance at
/// whatever lag makes the two equal. A first pass at (64u, 120u) parked
/// the mean lag at 12 m for exactly that reason.
const RANGE: f32 = 80.0;

/// Ceiling on the recall's share of the final velocity. Never 1.0: even
/// a badly out-of-position player keeps a fraction of his own intent, so
/// a defender sprinting back still drifts toward the man he was watching
/// instead of running a dead-straight line to a coordinate.
///
/// A softer setting was measured — (72u slack, 0.55 pull) — on the theory
/// that a gentler leash would keep most of the shape while adding less
/// duel density. It does not: the observed block went 54 m → 67 m, giving
/// back nearly all the compaction, and bought only ~0.3 goals a match
/// back. The tether is close to all-or-nothing, so it is set to work.
const MAX_PULL: f32 = 0.85;

/// Within this distance of the ball a player is *in the play*, and the
/// shape stops having opinions. 80u = 10 m — the range over which a
/// player can realistically affect the ball in the next second or two.
const BALL_ENGAGEMENT: f32 = 80.0;

pub struct ShapeDiscipline;

impl ShapeDiscipline {
    /// Blend the state's own velocity with a pull toward the team anchor,
    /// according to how far outside his zone the player has drifted.
    /// Returns the velocity unchanged whenever the ball is this player's
    /// job.
    pub fn apply(ctx: &StateProcessingContext, velocity: Vector3<f32>) -> Vector3<f32> {
        Self::apply_with_pull(ctx, velocity).0
    }

    /// As [`Self::apply`], but also reports how hard the recall is pulling
    /// so the caller can stop the movement-effort cap from throttling a
    /// recovery run — see `StateProcessingResult::shape_recall_pull`.
    pub fn apply_with_pull(
        ctx: &StateProcessingContext,
        velocity: Vector3<f32>,
    ) -> (Vector3<f32>, f32) {
        // A/B control for the positional layer — see
        // `MatchContext::shape_off`.
        if MatchContext::shape_off() || Self::is_exempt(ctx) {
            return (velocity, 0.0);
        }

        let anchor = ctx.team().my_anchor();
        let to_anchor = anchor - ctx.player.position;
        let lag = to_anchor.magnitude();
        if lag <= SLACK {
            return (velocity, 0.0);
        }

        let pull = (((lag - SLACK) / RANGE).clamp(0.0, 1.0)) * MAX_PULL * Self::organisation(ctx);

        // The recall runs at the player's own top speed, so a man 30 m out
        // of shape actually recovers rather than ambling back — the
        // recovery run is the thing being modelled here.
        //
        // ⚠ That intent was defeated downstream for as long as this layer
        // has existed. `state.rs` caps the finished velocity at
        // `max_speed * MovementEffort::speed_fraction(state_intensity)`,
        // and a player drifting out of shape is by definition NOT in a
        // high-effort state — `Standing` declares `Recovery` (0.12),
        // `Returning` and `CreatingSpace` declare `Moderate` (0.52). So
        // the recall was computed at 1.0 and then served at 0.12-0.52.
        // Reporting `pull` lets the cap floor itself at the recall's own
        // demand, which is the honest reading: being this far out of
        // position IS the effort, whatever the state was calling itself.
        let recall = (to_anchor / lag) * ctx.player.max_speed_with_condition_cached();

        (velocity * (1.0 - pull) + recall * pull, pull)
    }

    /// How much a goalkeeper's voice is worth to the line in front of him.
    ///
    /// Applied to the shape-recall multiplier, centred on
    /// `TeamSkillAggregates::KEEPER_VOICE_REFERENCE` so a median keeper
    /// changes nothing. The composite spans roughly 0.33-0.55 across a
    /// realistic population, so 0.60 is about **±7%** of recall between the
    /// quietest keeper in the game and the most commanding — the same order
    /// as the ±15% the `own`/`organiser` blend already carries, and
    /// deliberately not more: organisation is worth a lot in football and
    /// it is still not worth as much as being able to defend.
    const KEEPER_VOICE_DEFENDER: f32 = 0.60;
    /// …and less as you go up the pitch. He is audible to a holding
    /// midfielder and not to a winger.
    const KEEPER_VOICE_MIDFIELD: f32 = 0.24;
    /// …and what it is worth to the BAND the line is allowed to spread
    /// across. See [`Self::line_band`]. 0.90 is about ±10% of the band
    /// over the realistic spread of the composite: a metre either way on
    /// a 6 m allowance, which is the difference between a flat four and
    /// one with a man dropping off it.
    const KEEPER_VOICE_LINE: f32 = 0.90;

    /// How well this player is being held in the block, as a multiplier
    /// on the recall — 1.0 for an ordinarily-organised side.
    ///
    /// [`MAX_PULL`] was applied identically to every player in every
    /// team, so a back four marshalled by a 19-leadership centre-half
    /// held its shape exactly as well as four strangers. That is the one
    /// thing organisation visibly IS in football, and it was the reason
    /// `leadership` reached almost nothing: outside a 0.15 slot in the
    /// keeper's communication composite and one term in the defender
    /// profile, a maxed-out leadership attribute changed no behaviour.
    ///
    /// Two inputs, both continuous:
    ///
    /// * **His own discipline** — concentration, teamwork, work_rate and
    ///   positioning are what keep a player in his slot when the ball is
    ///   elsewhere. `decision_quality` is the existing composite for
    ///   reading the game off the ball.
    /// * **Who is organising the side** — `TeamSkillAggregates::top_leadership`,
    ///   the best organiser on the pitch rather than an average, because
    ///   organisation comes from the one man doing it. Lifted from the
    ///   team aggregate, which is recomputed once per ~100 ticks, so this
    ///   costs a field read instead of a per-tick walk over ten team-mates
    ///   building ten `SkillBands`.
    ///
    /// Centred so the population mean lands on 1.0: an ordinary player in
    /// an ordinarily-led side gets the same recall he always did, and the
    /// band is narrow (±15%) because this is organisation, not a
    /// different sport.
    ///
    /// Memoized per `(player, tick)` — `apply_with_pull` is on the
    /// positional hot path (every player, every tick, and both
    /// `velocity()` and `process()` reach it), and `decision_quality`
    /// builds a full `SkillBands` set. Every input is tick-frozen.
    fn organisation(ctx: &StateProcessingContext) -> f32 {
        let tick = ctx.current_tick();
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .shape_organisation;
        if let Some(v) = cached {
            debug_assert_eq!(
                v.to_bits(),
                Self::organisation_uncached(ctx).to_bits(),
                "shape-organisation memo mismatch"
            );
            return v;
        }
        let v = Self::organisation_uncached(ctx);
        ctx.tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .shape_organisation = Some(v);
        v
    }

    fn organisation_uncached(ctx: &StateProcessingContext) -> f32 {
        /// Population mean of the `own`/`organiser` blend below. Both
        /// composites sit near it for a mid-table squad, so a mid-table
        /// squad multiplies its recall by ~1.0.
        const REFERENCE: f32 = 0.50;

        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let own = sc::decision_quality(ctx.player, minute);
        let aggregates = if ctx.player.team_id == ctx.context.field_home_team_id {
            &ctx.context.home_skill_aggregates
        } else {
            &ctx.context.away_skill_aggregates
        };
        let organiser = aggregates.top_leadership;

        let shortfall = (own * 0.60 + organiser * 0.40) - REFERENCE;
        let held = (1.0 + shortfall * 0.30).clamp(0.85, 1.15);

        // **…AND THE MAN BEHIND THEM.**
        //
        // A goalkeeper is the only player on the pitch who can see the
        // whole shape from behind it, and the only one whose job
        // description includes saying so out loud. Nothing in the engine
        // let him: `goalkeeping.communication` reached the punch decision
        // and a dead composite, so the back four in front of a loud,
        // commanding keeper held its shape exactly as well as the one in
        // front of a silent teenager.
        //
        // Weighted BY LINE, because that is how it works. He can shout a
        // centre-half into position all afternoon; he can do very little
        // about a winger forty metres up the pitch, and nothing at all
        // about a striker pressing a goal kick. Applied on top of the
        // existing blend rather than inside it, so the calibrated
        // discipline is exactly unchanged for a median keeper and the two
        // channels stay separable.
        let voice_gain = match ctx
            .player
            .tactical_position
            .current_position
            .position_group()
        {
            PlayerFieldPositionGroup::Defender => Self::KEEPER_VOICE_DEFENDER,
            PlayerFieldPositionGroup::Midfielder => Self::KEEPER_VOICE_MIDFIELD,
            _ => 0.0,
        };
        if voice_gain == 0.0 {
            return held;
        }
        let voice =
            (aggregates.keeper_voice - TeamSkillAggregates::KEEPER_VOICE_REFERENCE) * voice_gain;
        (held * (1.0 + voice)).clamp(0.80, 1.22)
    }

    /// **How tightly the back line is allowed to spread, by the voice
    /// behind it.**
    ///
    /// `DefenderShape::DEPTH_DROP + DEPTH_STEP_UP` is, in its own words,
    /// "the spread the line is ALLOWED", and it was the same number for
    /// every side in every division. Marshalling a back four so that it
    /// stays flat is the single most recognisable thing a goalkeeper does
    /// for his defenders — it is what stops one man dropping off and
    /// playing the rest of them onside — and no attribute reached it.
    ///
    /// Below 1.0 for a commanding keeper: a **tighter** band, so the line
    /// holds together. Above 1.0 for a quiet one, whose back four is
    /// allowed to string out.
    ///
    /// ⚠ Centred on `TeamSkillAggregates::KEEPER_VOICE_REFERENCE`, which
    /// is **measured** (0.560, off the `KEEPER VOICE` block) and is
    /// nowhere near 0.5 — `sc::gk_communication` runs through the same
    /// curve that puts `POPULATION_READ` at 0.479 and
    /// `SaveModel::POPULATION_HANDLING` at 0.530. An uncentred term here
    /// would not add a skill axis, it would quietly re-tune the block
    /// span for every team in the game — and the measured span is already
    /// wide (49 m defending against a real 35-45), so it would re-tune it
    /// in the direction it can least afford.
    pub fn line_band(ctx: &StateProcessingContext) -> f32 {
        let aggregates = if ctx.player.team_id == ctx.context.field_home_team_id {
            &ctx.context.home_skill_aggregates
        } else {
            &ctx.context.away_skill_aggregates
        };
        let shortfall = aggregates.keeper_voice - TeamSkillAggregates::KEEPER_VOICE_REFERENCE;
        (1.0 - shortfall * Self::KEEPER_VOICE_LINE).clamp(0.78, 1.30)
    }

    /// Is the ball this player's job right now? Then the shape is not.
    fn is_exempt(ctx: &StateProcessingContext) -> bool {
        // The keeper's position is his own state machine's business
        // entirely — he is not part of the block.
        if ctx
            .player
            .tactical_position
            .current_position
            .is_goalkeeper()
        {
            return true;
        }

        // On the ball, or mid-way through an action that cannot be
        // abandoned (a shot, a header, a tackle, a dive).
        if ctx.player.has_ball(ctx) || ctx.player.state.is_committed_action() {
            return true;
        }

        // Injured players are not doing positional play.
        if matches!(ctx.player.state, PlayerState::Injured) {
            return true;
        }

        // In the immediate vicinity of the ball. Includes every chase,
        // press, interception and reception without needing to enumerate
        // the states that do them.
        if ctx.ball().distance() < BALL_ENGAGEMENT {
            return true;
        }

        // A pass is on its way to him — he must go and meet it wherever
        // it has been played, which is by design AHEAD of where he is.
        if ctx.tick_context.ball.pass_target == Some(ctx.player.id) {
            return true;
        }

        // He is holding a touchline, or running beyond the man who is.
        //
        // The general rule for an exemption is "something else is
        // definitely this player's job", and here it is stronger than
        // usual: these three states do not derive a destination from ball
        // geometry at all. They steer at [`WideChannel::target`], which is
        // itself built from `my_anchor` — so the tether would be
        // correcting a target that already came out of the team plan.
        //
        // It would also be correcting it in the wrong direction. The
        // touchline hold IS the anchor and needs no help; the byline run
        // deliberately leaves it, and measured at a mean 11.8 m of lag
        // that puts the recall at ~49% of the runner's velocity, aimed
        // back up the pitch. The one run this whole layer exists to
        // produce was being half-cancelled by the layer above it.
        if matches!(
            ctx.player.state,
            PlayerState::Midfielder(MidfielderState::HoldingWidth)
                | PlayerState::Forward(ForwardState::HoldingWidth)
                | PlayerState::Defender(DefenderState::Overlapping)
        ) {
            return true;
        }

        // He is the one taking a restart. Nothing else about the shape
        // applies to him: he has to leave it, go out into the run-off for
        // a ball that has gone off the pitch and bring it back.
        //
        // The `BALL_ENGAGEMENT` exemption above covers him only while the
        // ball is within 10 m, and a throw-in taker is routinely picked
        // from further out than that — at which point the recall takes up
        // to 85% of his velocity and points it back at his formation slot,
        // i.e. away from the ball he has been sent for. Same exemption
        // `CornerHold` already makes for the same man.
        if ctx.tick_context.ball.restart_taker == Some(ctx.player.id) {
            return true;
        }

        // ⚠ NOT EXEMPT: a player holding an individual defensive duty.
        //
        // Tried 2026-08-17 and REVERTED, because the reasoning was good
        // and the measurement disagreed. The argument: every exemption
        // above says "when something else is definitely this player's
        // job, the shape stops having opinions", and an assignment from
        // `DefensivePlan` is exactly that — a marker tracking a midfielder
        // eight metres in front of the back four is by definition outside
        // `SLACK` with the ball outside `BALL_ENGAGEMENT`, so up to 85% of
        // his velocity is replaced by a run back to his formation slot.
        //
        // Measured, exempting him made marking WORSE, not better: the duel
        // gap went 5.2 m → 5.6 m and against midfielders 7.1 m → 7.8 m.
        // Freed from the tether the marker does not close on his man, he
        // drifts — his own steering already has a lag, and the recall was
        // incidentally holding him inside a zone the man kept re-entering.
        // So this layer is NOT what keeps markers off their men; see the
        // note at `DefensiveLine::hold_shape_on_man` for what was.
        false
    }
}
