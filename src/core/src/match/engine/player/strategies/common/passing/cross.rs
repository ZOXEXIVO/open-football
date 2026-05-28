//! Shared crossing + aerial-duel resolution. Both forwarder and
//! midfielder crossing states drive their cross-target selection
//! through this module so the cross-type / target / delivery quality
//! model is consistent across roles.

use crate::PlayerFieldPositionGroup;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{MatchPlayer, MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// True once an attacking corner's box is "loaded": at least one of our
/// pushed-up centre-backs has arrived within heading range, or a full
/// complement of attackers is in the box. The corner taker holds the
/// delivery until this returns true (or the set-up window expires) so the
/// run from defence has time to arrive — there is no dead-ball pause in
/// the sim, so the taker has to create the window itself.
pub fn box_loaded_for_corner(ctx: &StateProcessingContext) -> bool {
    let goal = ctx.player().opponent_goal_position();
    // Deliver once a pushed-up centre-back has reached heading range. This
    // is the whole point of the set-up wait — the forwards/midfielders are
    // already up, so keying off "≥N attackers" would fire instantly and
    // the CB run from defence would never have time to arrive.
    ctx.players().teammates().all().any(|t| {
        t.id != ctx.player.id
            && t.tactical_positions.is_central_defender()
            && (t.position - goal).magnitude() < 130.0
    })
}

/// Cross delivery type. Drives flight, target selection, and the
/// downstream aerial-duel / header model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossType {
    /// High lofted ball aimed at the back post — attackers attacking
    /// the second six.
    FloatedFarPost,
    /// Hard low-driven cross 1-2 yards above the grass — fast and
    /// difficult to defend even with a clean run.
    DrivenLowCross,
    /// Pulled-back ball from the byline to a runner at the edge of
    /// the penalty area.
    Cutback,
    /// Whipped delivery to the near post for a flick-on or first-time
    /// finish.
    WhippedNearPost,
    /// Early ball played in behind a high line — typically for a
    /// striker attacking space rather than a stationary aerial target.
    EarlyCross,
}

/// Decision a crossing state has resolved this tick: which cross to
/// play, who to aim it at, and the projected lane quality.
#[derive(Debug, Clone, Copy)]
pub struct CrossDecision {
    pub cross_type: CrossType,
    pub target_id: u32,
    pub target_pos: Vector3<f32>,
    /// 0..1 quality score the delivery will be evaluated against.
    pub lane_quality: f32,
}

/// Pick the best cross for the current context. Returns None when the
/// crosser has no viable target — caller should fall back to a regular
/// pass.
pub fn pick_cross<'a>(ctx: &StateProcessingContext<'a>) -> Option<CrossDecision> {
    let goal_pos = ctx.player().opponent_goal_position();
    let crosser_pos = ctx.player.position;
    let crosser_dist_to_goal = (crosser_pos - goal_pos).magnitude();

    let mut best: Option<CrossDecision> = None;

    for teammate in ctx.players().teammates().all() {
        if teammate.id == ctx.player.id {
            continue;
        }
        let dist_to_goal = (teammate.position - goal_pos).magnitude();
        if dist_to_goal > 160.0 {
            continue;
        }

        // Skip targets without a clear-enough lane.
        if !ctx.player().has_clear_pass(teammate.id) {
            continue;
        }

        // Resolve the runner's profile from the full player record.
        let teammate_full = match ctx.context.players.by_id(teammate.id) {
            Some(p) => p,
            None => continue,
        };

        let off_the_ball = (teammate_full.skills.mental.off_the_ball / 20.0).clamp(0.0, 1.0);
        let heading = (teammate_full.skills.technical.heading / 20.0).clamp(0.0, 1.0);
        let jumping = (teammate_full.skills.physical.jumping / 20.0).clamp(0.0, 1.0);
        let strength = (teammate_full.skills.physical.strength / 20.0).clamp(0.0, 1.0);
        let anticipation = (teammate_full.skills.mental.anticipation / 20.0).clamp(0.0, 1.0);
        let composure = (teammate_full.skills.mental.composure / 20.0).clamp(0.0, 1.0);

        let dist_bonus = (1.0 - (dist_to_goal / 160.0)).clamp(0.0, 1.0);

        // Marker proximity penalty.
        let close_opponents = ctx.tick_context.grid.opponents(teammate.id, 8.0).count();
        let separation = match close_opponents {
            0 => 1.0,
            1 => 0.6,
            _ => 0.25,
        };

        // Goalkeeper claim risk: balls floated near the GK get
        // intercepted. Penalise targets that sit on a line between the
        // crosser and the keeper.
        let gk_claim_risk = ctx
            .players()
            .opponents()
            .goalkeeper()
            .next()
            .map(|gk| {
                let gk_to_target = (teammate.position - gk.position).magnitude();
                if gk_to_target < 18.0 {
                    1.0 - (gk_to_target / 18.0)
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        let lane_quality = (separation * 0.6 + dist_bonus * 0.4) * (1.0 - gk_claim_risk * 0.5);

        let score = off_the_ball * 0.20
            + heading * 0.16
            + jumping * 0.16
            + strength * 0.10
            + anticipation * 0.12
            + composure * 0.08
            + dist_bonus * 0.10
            + separation * 0.18
            + lane_quality * 0.20
            - gk_claim_risk * 0.18;

        let cross_type = pick_cross_type(
            ctx,
            crosser_pos,
            crosser_dist_to_goal,
            teammate.position,
            goal_pos,
            heading,
        );

        let candidate = CrossDecision {
            cross_type,
            target_id: teammate.id,
            target_pos: teammate.position,
            lane_quality,
        };

        let candidate_score = score;
        match &best {
            None => best = Some(candidate),
            Some(prev) => {
                let prev_score = score_decision(prev, ctx);
                if candidate_score > prev_score {
                    best = Some(candidate);
                }
            }
        }
    }
    best
}

fn score_decision(d: &CrossDecision, _ctx: &StateProcessingContext) -> f32 {
    // Lightweight rebuild — only used to tie-break two candidates so
    // we can avoid keeping the score around in the struct.
    d.lane_quality * 0.5
}

fn pick_cross_type(
    ctx: &StateProcessingContext,
    crosser_pos: Vector3<f32>,
    crosser_dist_to_goal: f32,
    target_pos: Vector3<f32>,
    goal_pos: Vector3<f32>,
    target_heading_skill: f32,
) -> CrossType {
    let near_byline = crosser_dist_to_goal < 70.0;
    let target_inside_box = (target_pos - goal_pos).norm_squared() < 80.0 * 80.0;

    // `target_heading_skill` is already normalised (raw/20). Compute
    // sigmoid probability of "poor header" so the cutback / driven-low
    // choices scale smoothly with the target's actual heading, instead
    // of cliff-gating everyone below 0.55 / 0.50 into the same bucket.
    let raw_heading = target_heading_skill * 20.0;
    let p_poor_header_byline = 1.0 - SkillCurve::new(raw_heading, 11.0, 0.6).probability();
    let p_poor_header_wide = 1.0 - SkillCurve::new(raw_heading, 10.0, 0.6).probability();

    if near_byline && target_inside_box {
        // Pulled-back option for a runner trailing the play.
        if target_pos.x.abs() > crosser_pos.x.abs() && ctx.context.rng.unit_f32() < p_poor_header_byline
        {
            return CrossType::Cutback;
        }
        return CrossType::WhippedNearPost;
    }

    let separation = (target_pos.x - crosser_pos.x).abs();
    if separation > 60.0 {
        // Long delivery — lofted to the back post.
        return CrossType::FloatedFarPost;
    }

    if separation > 25.0 && ctx.context.rng.unit_f32() < p_poor_header_wide {
        // Foot-runner profile — a low driven ball is the better choice.
        return CrossType::DrivenLowCross;
    }

    if crosser_dist_to_goal > 120.0 {
        return CrossType::EarlyCross;
    }

    CrossType::WhippedNearPost
}

/// Resolve an aerial duel between an attacker and the closest defender.
/// Returns true if the attacker wins the header.
///
/// `minute` lets the duel feed through the engine's fatigue model: a
/// tired CB late in the game genuinely loses more aerials. Routes
/// both sides through the existing aerial composites
/// (`aerial_outfield_attacker` weights `off_the_ball`,
/// `aerial_outfield_defender` weights `positioning`) so the duel
/// reads consistent with every other aerial composite read.
pub fn resolve_aerial_duel(
    ctx: &StateProcessingContext,
    attacker: &MatchPlayer,
    defender: Option<&MatchPlayer>,
    minute: u32,
) -> bool {
    let attacker_score = sc::aerial_outfield_attacker(attacker, minute);
    let defender_score = defender
        .map(|d| sc::aerial_outfield_defender(d, minute))
        .unwrap_or(0.40); // Empty box → easier for the attacker, but not a free win.

    let diff = attacker_score - defender_score;
    let win_prob = sigmoid(diff * 2.2).clamp(0.18, 0.82);
    ctx.context.rng.unit_f32() < win_prob
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Whether a player is in a wide enough crossing position. Used by
/// crossing states' guard.
pub fn is_in_wide_position(ctx: &StateProcessingContext) -> bool {
    let field_height = ctx.context.field_size.height as f32;
    let y = ctx.player.position.y;
    let wide_margin = field_height * 0.2;
    y < wide_margin || y > field_height - wide_margin
}

/// Pick the closest opposing defender in the box for the aerial duel.
pub fn pick_aerial_marker<'a>(
    ctx: &StateProcessingContext<'a>,
    target_pos: Vector3<f32>,
    radius: f32,
) -> Option<MatchPlayerLite> {
    let mut best: Option<(MatchPlayerLite, f32)> = None;
    for opp in ctx.players().opponents().all() {
        // Goalkeepers handle their own claim/punch model — skip them
        // here.
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
