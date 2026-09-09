//! **The voice from behind** — what the goalkeeper does for the people in
//! front of him.
//!
//! # Why this exists
//!
//! Reported from the viewer, alongside the box-passing complaint:
//! *"make sure the goalkeeper is involved in the defenders' positioning
//! and gives them advice."*
//!
//! He was not. `sc::gk_communication` — the composite whose own doc says
//! "shouting, marshalling the defensive line, calling for crosses" —
//! reached exactly three things before this module:
//!
//! * his own claim radius (`KeeperBallClaim::is_favourite`),
//! * `on_field_leadership`, a squad-level composite,
//! * and, through [`ShapeDiscipline::line_band`], the DEPTH BAND the back
//!   four is allowed to spread across.
//!
//! All three are about the line as a body. None of them is a keeper
//! talking to a defender about a specific opponent, which is what a
//! goalkeeper spends ninety minutes doing and what the report is asking
//! for. Inside his own penalty area — the one place his view is worth
//! more than anybody else's, because he is the only man on the pitch with
//! all twenty-one in front of him — his voice reached nothing at all.
//!
//! # The three calls
//!
//! Every keeper in football makes the same three, and each maps onto a
//! decision the engine was already taking blind:
//!
//! 1. **"Pick him up!"** — he can see the man nobody has. [`Self::free_man`]
//!    names him, and the duty assigner ranks that opponent FIRST, so the
//!    exclusive man-marking assignment covers him before it covers
//!    anybody else.
//! 2. **"Get in front of him!"** — the marker who is goal-side of his man
//!    but not in the lane the ball is coming down. [`Self::front_foot`]
//!    is how much of the marking offset the keeper's voice moves from the
//!    goal side to the BALL side, inside the area.
//! 3. **"Away!"** — the clearance. [`Self::away_shout`] lowers the
//!    tolerance in [`ClearanceCall`](super::super::players::ops::defending::ClearanceCall)
//!    so a defender squeezed in his own six-yard box stops looking for a
//!    pass.
//!
//! # ⚠ Every term here is CENTRED on the measured population value
//!
//! `TeamSkillAggregates::KEEPER_VOICE_REFERENCE` is **0.560**, measured
//! off the `KEEPER VOICE` block in `dev_match stats` and nowhere near
//! 0.5 — `sc::gk_communication` runs through the same curve that puts
//! `POPULATION_READ` at 0.479 and `SaveModel::POPULATION_HANDLING` at
//! 0.530. An uncentred multiplier here would not add a skill axis, it
//! would silently re-tune box defending for every side in the game. A
//! median keeper must leave all three calls exactly where the engine was
//! calibrated; the voice is a spread around that, not a level.
//!
//! # Measured — and the A/B is the proof, not the level
//!
//! 400 fixtures at level 14 in each arm, against `OF_BOX_DEFENCE_OFF`.
//! What matters here is not the aggregate (this rides with the pass
//! block, which is the loud half of the same change) but whether a
//! commanding keeper's defence behaves differently from a quiet one's.
//! The read-out is `mid_run_diag::MARKLANE_PERP_X100` — how often an
//! ASSIGNED marker is standing in the passing lane to the man he was
//! given:
//!
//! | | quiet | commanding |
//! |---|---|---|
//! | off | 14.3% | 12.6% |
//! | on | 13.7% | **15.8%** |
//!
//! Off, the two bands are indistinguishable and if anything inverted —
//! that is population noise, which is what a null channel looks like.
//! On, the ordering is real. The organising zone is 20.8 m for a quiet
//! keeper and 32.5 m for a commanding one (26.2 m for everybody with the
//! switch set, which is the base, exactly as centring requires), and the
//! "pick him up" shout fires on 3% of refreshes behind a quiet keeper
//! against 9% behind a commanding one.
//!
//! ⚠ **The first version of `front_foot` was a null, and the reason is
//! worth keeping.** It expressed the lean as a shift on
//! `goal_side_weight`, which is scaled by `ideal_marking_distance` —
//! 0.9-1.75 m — so the whole channel was worth 7 cm against a marker
//! measured sitting 4.17 m from his own target. The lane census read
//! 1.75 / 1.81 / 1.79 / 1.79 across the four bands: no response at all.
//! A keeper's shout is worth about a metre of standing position, so it
//! is now expressed in metres.

use crate::r#match::engine::teamplay::tactical::inputs::TeamSkillAggregates;
use crate::r#match::{MatchContext, MatchField, StateProcessingContext};
use nalgebra::Vector3;

/// What the keeper saw when he last looked up.
#[derive(Debug, Clone, Copy)]
pub struct KeeperCall {
    /// The man he is shouting about, if anybody.
    pub man: Option<u32>,
    /// How many opponents inside his zone had nobody near them.
    pub free: u32,
    /// …out of how many were inside it at all.
    pub in_zone: u32,
    /// Mean distance from a man in the zone to the nearest of ours — the
    /// number that says what "free" is worth against this population,
    /// and the reason [`KeeperVoice::COVERED`] is 7 m and not 12.
    pub mean_gap: f32,
    /// How far his voice was carrying, in game units.
    pub reach: f32,
}

pub struct KeeperVoice;

impl KeeperVoice {
    /// How far from his own goal a MEDIAN keeper organises, in game
    /// units. 210u ≈ 26 m — the penalty area (16.5 m deep) and the
    /// approach to it, which is the distance a shout genuinely carries
    /// over a crowd and the distance over which his view of the play is
    /// better than the defender's own.
    const ORGANISE_BASE: f32 = 210.0;
    /// …and what his voice is worth either side of that, per unit of
    /// centred `gk_communication`. The composite's usable spread is
    /// roughly ±0.15 around the reference, so this is ±63u (±8 m): a
    /// commanding keeper is organising the whole of the D, a quiet one
    /// barely past his own six-yard box.
    const ORGANISE_PER_VOICE: f32 = 420.0;
    /// Never less than the six-yard box (~7.3 m) — even a silent keeper
    /// shouts at somebody standing on him — and never more than half the
    /// pitch, which is the point past which he is organising nothing.
    const ORGANISE_MIN: f32 = 60.0;
    const ORGANISE_MAX: f32 = 330.0;

    /// A man with one of ours inside this is somebody's problem already.
    /// 56u = 7 m — the same distance `ClearanceCall::OUTLET_MARKED` calls
    /// "near enough that the opponent reads the pass and arrives with
    /// it", which is exactly the question here with the sides swapped.
    ///
    /// ⚠ **Measured, not guessed. 96u (12 m) was, and it was a null
    /// channel**: over 7.4 M plan refreshes it found 0.00-0.01 free men
    /// and the keeper shouted on 0-1% of them, because at 12 m every
    /// opponent inside the zone has somebody nominally near him. The
    /// engine's problem is not unmarked men — the box-pass census counts
    /// 4.4 defenders standing in the area — it is loose ones: the marking
    /// duel census has markers sitting **5.7 m** off their man, with the
    /// attacker clear by more than 4 m on 54% of samples. 7 m is where
    /// "free" starts to mean something against that population.
    const COVERED: f32 = 56.0;

    /// His voice, centred: negative for a quiet keeper, positive for a
    /// commanding one, zero for the median. Read from the team
    /// aggregates, which already carry `sc::gk_communication` maxed over
    /// the eleven — in practice the keeper's own, since it is a
    /// goalkeeping composite.
    pub fn edge(aggregates: &TeamSkillAggregates) -> f32 {
        // A/B control — see `MatchContext::box_defence_off`. A zero edge
        // makes all three calls neutral by construction, which is the
        // whole point of centring them.
        if MatchContext::box_defence_off() {
            return 0.0;
        }
        aggregates.keeper_voice - TeamSkillAggregates::KEEPER_VOICE_REFERENCE
    }

    /// The same, for a player who has a live context.
    pub fn edge_for(ctx: &StateProcessingContext) -> f32 {
        let aggregates = if ctx.player.team_id == ctx.context.field_home_team_id {
            &ctx.context.home_skill_aggregates
        } else {
            &ctx.context.away_skill_aggregates
        };
        Self::edge(aggregates)
    }

    /// How far from his own goal his voice is carrying, in game units.
    pub fn organise_reach(aggregates: &TeamSkillAggregates) -> f32 {
        Self::organise_reach_from(aggregates.keeper_voice)
    }

    /// The same, from the raw aggregate — for the duty assigner, which
    /// is handed the voice rather than the whole aggregate block.
    pub fn organise_reach_from(keeper_voice: f32) -> f32 {
        let edge = if MatchContext::box_defence_off() {
            0.0
        } else {
            keeper_voice - TeamSkillAggregates::KEEPER_VOICE_REFERENCE
        };
        (Self::ORGANISE_BASE + edge * Self::ORGANISE_PER_VOICE)
            .clamp(Self::ORGANISE_MIN, Self::ORGANISE_MAX)
    }

    /// **"Pick him up!"** — the most dangerous opponent inside the part
    /// of the pitch this keeper is organising who has nobody near him.
    ///
    /// Ranked by depth alone: the keeper is not weighing off-the-ball
    /// movement or finishing, he is looking at who is nearest his goal
    /// with nobody on him, which is exactly the read a keeper actually
    /// makes and the one his position gives him.
    ///
    /// `man` is `None` when everybody in the zone is covered — which is
    /// most of the time, and is the point: this is an exception call, not
    /// a permanent re-ranking. `free` and `in_zone` are the census behind
    /// it: how many men were loose inside the zone, out of how many were
    /// in it at all.
    pub fn free_man(
        field: &MatchField,
        team_id: u32,
        own_goal: Vector3<f32>,
        reach: f32,
    ) -> KeeperCall {
        let mut call: Option<(f32, u32)> = None;
        let mut free = 0u32;
        let mut gap_sum = 0.0f32;
        let mut in_zone = 0u32;
        for opponent in field.players.iter() {
            if opponent.team_id == team_id
                || opponent.tactical_position.current_position.is_goalkeeper()
            {
                continue;
            }
            let depth = (opponent.position - own_goal).magnitude();
            if depth > reach {
                continue;
            }
            in_zone += 1;
            // Is anybody on him? The keeper himself does not count — he
            // is the thing being protected, the same reason
            // `TackleDecision::cover_exists` excludes him.
            let nearest = field
                .players
                .iter()
                .filter(|d| {
                    d.team_id == team_id && !d.tactical_position.current_position.is_goalkeeper()
                })
                .map(|d| (d.position - opponent.position).magnitude())
                .fold(f32::MAX, f32::min);
            if nearest.is_finite() {
                gap_sum += nearest;
            }
            if nearest < Self::COVERED {
                continue;
            }
            free += 1;
            if call.is_none_or(|(best, _)| depth < best) {
                call = Some((depth, opponent.id));
            }
        }
        KeeperCall {
            man: call.map(|(_, id)| id),
            free,
            in_zone,
            mean_gap: if in_zone == 0 {
                0.0
            } else {
                gap_sum / in_zone as f32
            },
            reach,
        }
    }

    /// How far toward the BALL the keeper's voice pushes a marker inside
    /// the zone he is organising, in game units. See
    /// [`Self::front_foot`].
    ///
    /// ⚠ **Measured. The first version of this was a shift on
    /// `goal_side_weight`, and it was worth SEVEN CENTIMETRES.** That
    /// blend is scaled by `ideal_marking_distance`, which is 7-14u
    /// (0.9-1.75 m), so ±0.055 of it moves the marking target ±0.07 m —
    /// invisible against a marker who is measured sitting 4.17 m from
    /// that target in the first place. The lane census confirmed it
    /// exactly: 1.75 / 1.81 / 1.79 / 1.79 m off the lane across the four
    /// voice bands, no response at all.
    ///
    /// A shout is worth about a metre of standing position, so that is
    /// what it is expressed in. 80u per unit of centred voice, over a
    /// composite whose usable spread is ±0.10, is ±8u = ±1 m.
    const FRONT_FOOT_GAIN: f32 = 80.0;
    /// …and never more than this either way (1.5 m). A keeper can move
    /// his defender across; he cannot re-define where the man is marking
    /// from.
    const FRONT_FOOT_MAX: f32 = 12.0;

    /// **"Get in front of him!"** — how far toward the BALL the keeper's
    /// voice moves a marker inside the zone he is organising, in game
    /// units.
    ///
    /// A marker who is purely goal-side of his man is BEHIND him when the
    /// ball arrives; the whole of box defending is being across him, on
    /// the side the ball is coming from, so that the pass has to beat you
    /// before it beats the goal. That is the single most repeated
    /// instruction shouted from a goal, and the man shouting it is the
    /// only one who can see whether it is being obeyed.
    ///
    /// Positive for a commanding keeper (his defenders stand across the
    /// lane), negative for a quiet one (his stand a step behind). Zero
    /// outside the zone and zero for a median keeper, so the calibrated
    /// marking geometry is exactly unchanged for the median side.
    pub fn front_foot(ctx: &StateProcessingContext, own_goal: Vector3<f32>) -> f32 {
        let depth = (ctx.player.position - own_goal).magnitude();
        let aggregates = if ctx.player.team_id == ctx.context.field_home_team_id {
            &ctx.context.home_skill_aggregates
        } else {
            &ctx.context.away_skill_aggregates
        };
        if depth > Self::organise_reach(aggregates) {
            return 0.0;
        }
        (Self::edge(aggregates) * Self::FRONT_FOOT_GAIN)
            .clamp(-Self::FRONT_FOOT_MAX, Self::FRONT_FOOT_MAX)
    }

    /// **"Away!"** — how much the keeper's shout lowers a defender's
    /// tolerance for playing out of his own area.
    ///
    /// Positive for a commanding keeper (he clears it sooner), negative
    /// for a quiet one, zero for the median. In the same units as
    /// `ClearanceCall::TOLERANCE_BASE`, whose whole span is 0.40-0.82, so
    /// ±0.05 is worth about an eighth of the skill axis — audible, not
    /// decisive. A defender who can play still plays.
    pub fn away_shout(ctx: &StateProcessingContext) -> f32 {
        (Self::edge_for(ctx) * 0.35).clamp(-0.06, 0.06)
    }

    /// Where a team's own goal is. Shared so the callers cannot disagree
    /// about which end they are defending — sides swap at half time.
    pub fn own_goal(
        field: &MatchField,
        context: &MatchContext,
        team_id: u32,
    ) -> Option<Vector3<f32>> {
        let side = field
            .players
            .iter()
            .find(|p| p.team_id == team_id)
            .and_then(|p| p.side)?;
        Some(Vector3::new(
            match side {
                crate::r#match::PlayerSide::Left => 0.0,
                crate::r#match::PlayerSide::Right => context.field_size.width as f32,
            },
            context.field_size.height as f32 / 2.0,
            0.0,
        ))
    }
}

#[cfg(test)]
mod keeper_voice_tests {
    use super::*;

    fn aggregates_with(voice: f32) -> TeamSkillAggregates {
        let mut a = TeamSkillAggregates::neutral();
        a.keeper_voice = voice;
        a
    }

    /// ⚠ The centring is the whole safety argument: a median keeper must
    /// leave every calibrated quantity exactly where it was.
    #[test]
    fn a_median_keeper_changes_nothing() {
        let a = aggregates_with(TeamSkillAggregates::KEEPER_VOICE_REFERENCE);
        assert_eq!(KeeperVoice::edge(&a), 0.0);
        assert_eq!(
            KeeperVoice::organise_reach(&a),
            KeeperVoice::ORGANISE_BASE,
            "the median reach must be the base, or the zone itself is a silent re-tune"
        );
    }

    /// A louder keeper organises further out, a quieter one less far, and
    /// neither runs away with it.
    #[test]
    fn the_voice_moves_the_zone_and_is_bounded() {
        let quiet = KeeperVoice::organise_reach(&aggregates_with(0.20));
        let loud = KeeperVoice::organise_reach(&aggregates_with(0.95));
        assert!(quiet < KeeperVoice::ORGANISE_BASE, "{quiet}");
        assert!(loud > KeeperVoice::ORGANISE_BASE, "{loud}");
        assert!(quiet >= KeeperVoice::ORGANISE_MIN);
        assert!(loud <= KeeperVoice::ORGANISE_MAX);
        // Monotone across the whole attribute range.
        let mut previous = 0.0;
        for step in 0..=20 {
            let v = KeeperVoice::organise_reach(&aggregates_with(step as f32 / 20.0));
            assert!(v >= previous, "not monotone at {step}");
            previous = v;
        }
    }
}
