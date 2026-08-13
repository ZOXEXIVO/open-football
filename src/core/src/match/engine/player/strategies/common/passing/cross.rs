//! Shared crossing model. Both the forwarder and midfielder crossing
//! states drive their delivery through [`CrossModel`], so the cross-type
//! / aim-point / aerial-duel model is consistent across roles.
//!
//! # Why a cross is not a pass
//!
//! A cross used to be emitted as a plain `PassTo` at a teammate's feet,
//! and the trajectory solver then chose its shape from *lane traffic*
//! (`select_trajectory_type_contextual`). Those two rules are mutually
//! exclusive: the crossing states only accepted a target with a clear
//! straight lane, and the solver only lifted the ball off the deck when
//! the lane was blocked — so by construction an open-play cross was a
//! ground pass rolled across the face of goal. Nobody could contest it
//! either, because a named pass target holds exclusive claim for the
//! whole flight.
//!
//! So a cross is modelled here as its own action:
//!
//! * a [`CrossType`] chosen from geometry and the target's profile,
//! * an **aim point** — a patch of the box, not a pair of feet — because
//!   that is what a crosser actually hits and what lets more than one
//!   player attack the delivery,
//! * a flight shape ([`CrossType::apex_metres`]) the pass solver honours
//!   directly instead of re-deriving from lane traffic,
//! * and, for lofted deliveries, a single aerial contest resolved by the
//!   engine (`resolve_cross_contest`) rather than by whichever player's
//!   state machine happened to run first.

use crate::PlayerFieldPositionGroup;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{MatchPlayer, MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// Half-width of the penalty area in game units (20.16 m at 0.125 m/u).
const BOX_HALF_WIDTH: f32 = 161.0;
/// Depth of the penalty area from the goal line (16.5 m).
const BOX_DEPTH: f32 = 132.0;
/// Contact radius of an aerial challenge, in game units (~4.3 m).
const AERIAL_MARKER_RADIUS: f32 = 34.0;

/// Cross delivery type. Drives flight shape, aim point, and whether the
/// delivery is resolved through the aerial contest or as a ground ball.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossType {
    /// High lofted ball hung up to the back post — attackers attacking
    /// the second six. The slowest delivery and the easiest to defend,
    /// but the one that beats a packed near post.
    FloatedFarPost,
    /// Hard low-driven cross a yard above the grass. Fast and difficult
    /// to defend even with a clean run; resolved on the deck, not in the
    /// air, so a poor header can still attack it.
    DrivenLowCross,
    /// Pulled back from the byline to a runner at the edge of the area.
    /// Ground ball, away from the keeper, into the highest-value shooting
    /// zone in football.
    Cutback,
    /// Whipped delivery across the six-yard line for a flick-on or a
    /// first-time finish. Fast and flat enough that the keeper has to
    /// commit, which is why it is the highest-value aerial ball.
    WhippedNearPost,
    /// Early ball clipped in behind a high line before the defence can
    /// set — aimed at space for a striker to run onto rather than at a
    /// stationary aerial target.
    EarlyCross,
}

impl CrossType {
    /// Whether the delivery arrives through the air (and therefore
    /// resolves through the aerial contest) or along the ground.
    pub fn is_lofted(self) -> bool {
        matches!(
            self,
            CrossType::FloatedFarPost | CrossType::WhippedNearPost | CrossType::EarlyCross
        )
    }

    /// Peak height of the delivery in metres. This is the knob a crosser
    /// actually turns, and it is the only one expressible on the ball's
    /// mixed axes (units horizontally, metres vertically) — see
    /// `PlayerEventDispatcher::target_apex`.
    ///
    /// Longer deliveries climb higher, as they must to carry; the caps
    /// keep every type inside the band its name claims.
    pub fn apex_metres(self, distance_units: f32) -> f32 {
        let metres = distance_units * 0.125;
        match self {
            // Hung up: peaks around head height plus the whole flight,
            // ~2.5 s of hang from a 30 m ball.
            CrossType::FloatedFarPost => (3.2 + metres * 0.13).clamp(3.2, 9.0),
            // Skims the grass — under the knee, so a defender has to get
            // down to it and the keeper cannot claim.
            CrossType::DrivenLowCross => (0.35 + metres * 0.012).clamp(0.35, 1.1),
            // Along the deck by definition.
            CrossType::Cutback => 0.03,
            // Flat and fast — clears the first defender's head and drops
            // onto the six-yard line, no more.
            CrossType::WhippedNearPost => (1.9 + metres * 0.055).clamp(1.9, 4.2),
            // Clipped over the line into space for a runner.
            CrossType::EarlyCross => (2.4 + metres * 0.075).clamp(2.4, 5.5),
        }
    }

    /// Extra weighting-error multiplier. Crossing is a low-percentage
    /// skill, and the hardest deliveries are the ones that have to beat a
    /// defender AND drop inside the six-yard box. Applied on top of the
    /// crossing-skill shortfall in `handle_pass_to_event`.
    pub fn difficulty(self) -> f32 {
        match self {
            CrossType::Cutback => 0.75,
            CrossType::DrivenLowCross => 1.0,
            CrossType::EarlyCross => 1.1,
            CrossType::WhippedNearPost => 1.25,
            CrossType::FloatedFarPost => 1.15,
        }
    }

    /// Stable index for diagnostics bucketing. Kept next to the variants
    /// so a new delivery type can't silently land in someone else's bin.
    pub fn diag_index(self) -> usize {
        match self {
            CrossType::FloatedFarPost => 0,
            CrossType::DrivenLowCross => 1,
            CrossType::Cutback => 2,
            CrossType::WhippedNearPost => 3,
            CrossType::EarlyCross => 4,
        }
    }

    /// How much easier this delivery is for an ATTACKER to win in the
    /// air. A whipped or driven ball arrives before the defence can set;
    /// a floated one hangs long enough for them to.
    pub fn contest_edge(self) -> f32 {
        match self {
            CrossType::WhippedNearPost => 0.06,
            CrossType::DrivenLowCross => 0.05,
            CrossType::EarlyCross => 0.02,
            CrossType::FloatedFarPost => -0.02,
            CrossType::Cutback => 0.0,
        }
    }

    /// Multiplier on the keeper's claim probability. A ball fizzed across
    /// the six-yard box is one he cannot come for.
    pub fn keeper_claim_scale(self) -> f32 {
        match self {
            CrossType::WhippedNearPost | CrossType::DrivenLowCross => 0.55,
            _ => 1.0,
        }
    }

    /// Human-readable label for diagnostics output.
    pub fn label(self) -> &'static str {
        match self {
            CrossType::FloatedFarPost => "floated-far",
            CrossType::DrivenLowCross => "driven-low",
            CrossType::Cutback => "cutback",
            CrossType::WhippedNearPost => "whipped-near",
            CrossType::EarlyCross => "early",
        }
    }

    /// Every delivery type, in `diag_index` order.
    pub const ALL: [CrossType; 5] = [
        CrossType::FloatedFarPost,
        CrossType::DrivenLowCross,
        CrossType::Cutback,
        CrossType::WhippedNearPost,
        CrossType::EarlyCross,
    ];
}

/// Decision a crossing state has resolved this tick: which cross to play,
/// who it is for, and where it is actually aimed.
#[derive(Debug, Clone, Copy)]
pub struct CrossDecision {
    pub cross_type: CrossType,
    /// The runner the delivery is FOR. Still needed for assist / pass
    /// accounting, but the ball is not aimed at their feet.
    pub target_id: u32,
    /// Where the ball is actually struck — a patch of the box the runner
    /// is attacking. Aiming at a space rather than a player is what lets
    /// a second attacker and a defender contest the same delivery.
    pub aim_point: Vector3<f32>,
    /// 0..1 quality of the delivery lane, before execution error.
    pub lane_quality: f32,
}

/// Crossing decision + aerial-duel calculators. Everything the crossing
/// and heading states need to agree on lives here so the two roles can't
/// drift apart.
pub struct CrossModel;

impl CrossModel {
    /// True once an attacking corner's box is "loaded": at least one of
    /// our pushed-up centre-backs has arrived within heading range. The
    /// corner taker holds the delivery until this returns true (or the
    /// set-up window expires) so the run from defence has time to arrive
    /// — there is no dead-ball pause in the sim, so the taker has to
    /// create the window itself.
    ///
    /// Keyed off a centre-back specifically: the forwards and midfielders
    /// are already up, so an "≥N attackers" test would fire instantly and
    /// the CB run from defence would never have time to arrive.
    pub fn box_loaded_for_corner(ctx: &StateProcessingContext) -> bool {
        let goal = ctx.player().opponent_goal_position();
        ctx.players().teammates().all().any(|t| {
            t.id != ctx.player.id
                && t.tactical_positions.is_central_defender()
                && (t.position - goal).magnitude() < 130.0
        })
    }

    /// Whether a player is wide enough to cross. Used by the crossing
    /// states' entry guard.
    pub fn is_in_wide_position(ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;
        y < wide_margin || y > field_height - wide_margin
    }

    /// Pick the best cross for the current context. `None` when the
    /// crosser has no viable target — the caller should fall back to a
    /// regular pass.
    pub fn pick(ctx: &StateProcessingContext<'_>) -> Option<CrossDecision> {
        let goal_pos = ctx.player().opponent_goal_position();
        let crosser_pos = ctx.player.position;
        let crosser_dist_to_goal = (crosser_pos - goal_pos).magnitude();
        let field_height = ctx.context.field_size.height as f32;
        let forward_dir = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let on_corner = ctx.ball().is_team_attacking_corner();

        let gk_pos = ctx
            .players()
            .opponents()
            .goalkeeper()
            .next()
            .map(|gk| gk.position);

        let mut best: Option<(CrossDecision, f32)> = None;

        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id {
                continue;
            }
            if (teammate.position - goal_pos).magnitude() > 260.0 {
                continue;
            }

            // Resolve the runner's profile from the full player record.
            let Some(runner) = ctx.context.players.by_id(teammate.id) else {
                continue;
            };

            let off_the_ball = (runner.skills.mental.off_the_ball / 20.0).clamp(0.0, 1.0);
            let heading = (runner.skills.technical.heading / 20.0).clamp(0.0, 1.0);
            let jumping = (runner.skills.physical.jumping / 20.0).clamp(0.0, 1.0);
            let strength = (runner.skills.physical.strength / 20.0).clamp(0.0, 1.0);
            let anticipation = (runner.skills.mental.anticipation / 20.0).clamp(0.0, 1.0);
            let composure = (runner.skills.mental.composure / 20.0).clamp(0.0, 1.0);
            let finishing = (runner.skills.technical.finishing / 20.0).clamp(0.0, 1.0);

            let cross_type = Self::pick_type(
                ctx,
                crosser_pos,
                crosser_dist_to_goal,
                teammate.position,
                goal_pos,
                heading,
            );

            // Lane requirement, by delivery.
            //
            // A LOFTED ball is played over the traffic by definition —
            // demanding an unobstructed raycast for it is what made
            // open-play crosses impossible, since the trajectory solver
            // then read the clear lane as "keep it down".
            //
            // A CUTBACK is a pass to feet and genuinely needs the lane. A
            // DRIVEN LOW cross does not: it is deliberately fizzed through
            // the six-yard area precisely because bodies are in the way,
            // and it is aimed at a space rather than at the man. Requiring
            // a clean raycast to the runner's feet for it filtered nearly
            // every ground delivery out of the model (driven-low 1%,
            // cutback 0% of deliveries).
            let needs_lane = matches!(cross_type, CrossType::Cutback);
            if needs_lane && !on_corner && !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            let aim_point = Self::aim_point_for(
                cross_type,
                goal_pos,
                crosser_pos,
                teammate.position,
                forward_dir,
                field_height,
            );

            // How far the runner has to travel to attack the delivery. A
            // ball hung to a spot nobody can reach is a bad cross however
            // good the runner is.
            let runner_gap = (teammate.position - aim_point).magnitude();
            let reachability = (1.0 - (runner_gap / 130.0)).clamp(0.0, 1.0);

            // Marker proximity around the AIM POINT, not around the runner
            // — that is the space actually being contested.
            let contesting = ctx
                .players()
                .opponents()
                .all()
                .filter(|o| (o.position - aim_point).magnitude() < 45.0)
                .count();
            let separation = match contesting {
                0 => 1.0,
                1 => 0.72,
                2 => 0.45,
                _ => 0.25,
            };

            // Goalkeeper claim risk: a ball dropped inside the keeper's
            // zone gets caught. Real crossers aim away from him, and the
            // whipped near-post ball is dangerous precisely because it is
            // too fast to claim.
            let gk_claim_risk = gk_pos
                .map(|gk| {
                    let gap = (aim_point - gk).magnitude();
                    let base = if gap < 90.0 { 1.0 - (gap / 90.0) } else { 0.0 };
                    base * cross_type.keeper_claim_scale()
                })
                .unwrap_or(0.0);

            // Depth bonus: deliveries into the prime zone are worth more.
            let aim_depth = (aim_point - goal_pos).magnitude();
            let depth_bonus = (1.0 - (aim_depth / 190.0)).clamp(0.0, 1.0);

            let lane_quality = (separation * 0.45 + reachability * 0.35 + depth_bonus * 0.20)
                * (1.0 - gk_claim_risk * 0.55);

            // A ground delivery is attacked with the feet, an aerial one
            // with the head — score the runner on the attribute the
            // delivery actually asks of them.
            let attack_ability = if cross_type.is_lofted() {
                heading * 0.42 + jumping * 0.32 + strength * 0.26
            } else {
                finishing * 0.45 + composure * 0.30 + anticipation * 0.25
            };

            let mut score = attack_ability * 0.34
                + off_the_ball * 0.20
                + anticipation * 0.08
                + reachability * 0.16
                + separation * 0.14
                + depth_bonus * 0.08
                - gk_claim_risk * 0.22;

            // On a corner the pushed-up centre-back is the designated
            // target ("find the big man"). Inert in open play — CBs aren't
            // in the box.
            if on_corner && teammate.tactical_positions.is_central_defender() {
                score += 0.35;
            }

            let candidate = CrossDecision {
                cross_type,
                target_id: teammate.id,
                aim_point,
                lane_quality,
            };

            if best.as_ref().map_or(true, |(_, bs)| score > *bs) {
                best = Some((candidate, score));
            }
        }

        best.map(|(d, _)| d)
    }

    /// Where the ball is actually struck for a given cross type. Zones are
    /// expressed as (depth from the goal line, lateral offset from the
    /// goal centre) so they read the way a coach describes them, and are
    /// mirrored onto the crosser's flank — a cross from the left has its
    /// near post on the left.
    fn aim_point_for(
        cross_type: CrossType,
        goal_pos: Vector3<f32>,
        crosser_pos: Vector3<f32>,
        target_pos: Vector3<f32>,
        forward_dir: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        let centre_y = field_height / 2.0;
        // +1 when the crosser is on the high-y flank, -1 on the low-y one.
        let near_side = if crosser_pos.y >= centre_y { 1.0 } else { -1.0 };

        let (depth, lateral): (f32, f32) = match cross_type {
            // Back post: deeper than the six-yard line, on the far flank.
            CrossType::FloatedFarPost => (58.0, -near_side * 52.0),
            // Across the face of the six-yard box on the near side.
            CrossType::WhippedNearPost => (42.0, near_side * 30.0),
            // Hard and low through the corridor of uncertainty — between
            // the keeper and his back line, level with the penalty spot.
            CrossType::DrivenLowCross => (70.0, -near_side * 14.0),
            // Pulled back to the edge of the area on the crosser's side,
            // the classic arriving-runner ball.
            CrossType::Cutback => (118.0, near_side * 34.0),
            // Clipped in behind for a runner: still wide of the keeper,
            // but deep enough that the ball beats the line, not the man.
            CrossType::EarlyCross => (96.0, -near_side * 40.0),
        };

        // Bias a little toward where the runner actually is, so the aim
        // point tracks the attack instead of being a fixed rosette on the
        // pitch. Bounded so it can never drag the ball out of the zone.
        let nominal_y =
            (goal_pos.y + lateral).clamp(centre_y - BOX_HALF_WIDTH, centre_y + BOX_HALF_WIDTH);
        let pull = (target_pos.y - nominal_y).clamp(-26.0, 26.0);

        Vector3::new(
            goal_pos.x - forward_dir * depth.min(BOX_DEPTH + 40.0),
            nominal_y + pull,
            0.0,
        )
    }

    fn pick_type(
        ctx: &StateProcessingContext,
        crosser_pos: Vector3<f32>,
        crosser_dist_to_goal: f32,
        target_pos: Vector3<f32>,
        goal_pos: Vector3<f32>,
        target_heading_skill: f32,
    ) -> CrossType {
        let near_byline = crosser_dist_to_goal < 90.0;
        let target_inside_box = (target_pos - goal_pos).norm_squared() < BOX_DEPTH * BOX_DEPTH;

        // `target_heading_skill` is already normalised (raw/20). Compute
        // the sigmoid probability of "poor header" so the cutback /
        // driven-low choices scale smoothly with the target's actual
        // heading, instead of cliff-gating everyone below a threshold into
        // the same bucket.
        let raw_heading = target_heading_skill * 20.0;
        let p_poor_header_byline = 1.0 - SkillCurve::new(raw_heading, 11.0, 0.6).probability();
        let p_poor_header_wide = 1.0 - SkillCurve::new(raw_heading, 10.0, 0.6).probability();

        if near_byline && target_inside_box {
            // At the byline the pull-back is on. Whether it is taken
            // depends on whether the runner is trailing the play (a
            // cutback needs somebody arriving behind the ball) and on
            // their aerial profile.
            let trailing = (target_pos - goal_pos).magnitude() > crosser_dist_to_goal;
            if trailing && ctx.context.rng.unit_f32() < p_poor_header_byline {
                return CrossType::Cutback;
            }
            return CrossType::WhippedNearPost;
        }

        if crosser_dist_to_goal > 340.0 {
            // Deep in the middle third (>42 m) — the early ball in behind,
            // before the line can drop.
            return CrossType::EarlyCross;
        }

        // The delivery is chosen from HOW DEEP the crosser is and WHO he
        // is crossing to — not from the raw crosser→target distance.
        //
        // That distance was the selector, at a 210u (26 m) threshold, and
        // it collapsed the whole model into one branch: a wide player and
        // a box runner are routinely 200-270u apart on a 545u-tall pitch
        // through the lateral axis ALONE, so almost every delivery cleared
        // the bar and 98% of crosses came out `FloatedFarPost`. Distance
        // is a property of the pitch's geometry here, not of the crosser's
        // decision.
        let poor_header = ctx.context.rng.unit_f32() < p_poor_header_wide;
        if poor_header {
            // Foot-runner profile — keep it out of the air. Deep enough
            // in and the pull-back is the better ball.
            return if crosser_dist_to_goal < 150.0 && target_inside_box {
                CrossType::Cutback
            } else {
                CrossType::DrivenLowCross
            };
        }

        // Aerial target. WHICH aerial ball is decided by where the runner
        // is relative to the crosser's flank, not by how deep the crosser
        // is: you hang it to the back post because that is where your man
        // is, and you whip it across the near post because that is where
        // yours is. Keying this off depth alone left 93% of deliveries
        // floated, since most crossers sit beyond any fixed depth bar.
        let centre_y = ctx.context.field_size.height as f32 / 2.0;
        let crosser_high = crosser_pos.y >= centre_y;
        let target_high = target_pos.y >= centre_y;
        if crosser_high != target_high {
            // Runner attacking the far flank — the ball has to carry.
            CrossType::FloatedFarPost
        } else {
            CrossType::WhippedNearPost
        }
    }

    /// Resolve an aerial duel between an attacker and the closest
    /// defender. Returns true if the attacker wins the header.
    ///
    /// `minute` lets the duel feed through the engine's fatigue model: a
    /// tired CB late in the game genuinely loses more aerials. Routes both
    /// sides through the existing aerial composites
    /// (`aerial_outfield_attacker` weights `off_the_ball`,
    /// `aerial_outfield_defender` weights `positioning`) so the duel reads
    /// consistent with every other aerial composite read.
    pub fn resolve_aerial_duel(
        ctx: &StateProcessingContext,
        attacker: &MatchPlayer,
        defender: Option<&MatchPlayer>,
        minute: u32,
    ) -> bool {
        let attacker_score = sc::aerial_outfield_attacker(attacker, minute);
        let defender_score = defender
            .map(|d| sc::aerial_outfield_defender(d, minute))
            // Empty box → easier for the attacker, but not a free win.
            .unwrap_or(0.40);

        let win_prob = Self::sigmoid((attacker_score - defender_score) * 2.2).clamp(0.18, 0.82);
        ctx.context.rng.unit_f32() < win_prob
    }

    /// Pick the closest opposing outfielder to a delivery, for the aerial
    /// duel. Goalkeepers handle their own claim / punch model.
    pub fn pick_aerial_marker(
        ctx: &StateProcessingContext<'_>,
        target_pos: Vector3<f32>,
        radius: f32,
    ) -> Option<MatchPlayerLite> {
        let mut best: Option<(MatchPlayerLite, f32)> = None;
        for opp in ctx.players().opponents().all() {
            if let Some(full) = ctx.context.players.by_id(opp.id) {
                if full.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
                {
                    continue;
                }
            }
            let dist = (opp.position - target_pos).magnitude();
            if dist > radius {
                continue;
            }
            match best {
                None => best = Some((opp, dist)),
                Some((_, d)) if dist < d => best = Some((opp, dist)),
                _ => {}
            }
        }
        best.map(|(p, _)| p)
    }

    /// The marker contesting a delivery at standard aerial-challenge
    /// range. Convenience over [`pick_aerial_marker`](Self::pick_aerial_marker)
    /// so callers don't each invent their own radius.
    pub fn nearest_marker(
        ctx: &StateProcessingContext<'_>,
        target_pos: Vector3<f32>,
    ) -> Option<MatchPlayerLite> {
        Self::pick_aerial_marker(ctx, target_pos, AERIAL_MARKER_RADIUS)
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }
}
