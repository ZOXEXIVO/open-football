use crate::PlayerFieldPositionGroup;
use crate::club::player::behaviour_config::PassEvaluatorConfig;
use crate::club::player::registry::has_risk_tolerant_passing_trait;
use crate::club::player::traits::PlayerTrait;
use crate::r#match::{
    BallSideZone, GamePhase, MatchPlayer, MatchPlayerLite, PlayerSide, StateProcessingContext,
};

/// Comprehensive pass evaluation result
#[derive(Debug, Clone)]
pub struct PassEvaluation {
    /// Overall success probability [0.0 - 1.0]
    pub success_probability: f32,

    /// Risk level [0.0 - 1.0] where 1.0 is highest risk
    pub risk_level: f32,

    /// Expected value of the pass
    pub expected_value: f32,

    /// Breakdown of factors
    pub factors: PassFactors,

    /// Whether this pass is recommended
    pub is_recommended: bool,
}

#[derive(Debug, Clone)]
pub struct PassFactors {
    pub distance_factor: f32,
    pub angle_factor: f32,
    pub pressure_factor: f32,
    pub receiver_positioning: f32,
    pub passer_ability: f32,
    pub receiver_ability: f32,
    pub tactical_value: f32,
}

pub struct PassEvaluator;

impl PassEvaluator {
    /// Evaluate a potential pass from one player to another
    pub fn evaluate_pass(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> PassEvaluation {
        let pass_vector = receiver.position - passer.position;
        let pass_distance = pass_vector.norm();

        // Calculate individual factors
        let distance_factor = Self::calculate_distance_factor(pass_distance, passer);
        let angle_factor = Self::calculate_angle_factor(ctx, passer, receiver);
        let pressure_factor = Self::calculate_pressure_factor(ctx, passer);
        let receiver_positioning = Self::calculate_receiver_positioning(ctx, receiver);
        let passer_ability = Self::calculate_passer_ability(ctx, passer, pass_distance);
        let receiver_ability = Self::calculate_receiver_ability(ctx, receiver);
        let tactical_value = Self::calculate_tactical_value(ctx, receiver);

        let factors = PassFactors {
            distance_factor,
            angle_factor,
            pressure_factor,
            receiver_positioning,
            passer_ability,
            receiver_ability,
            tactical_value,
        };

        // Calculate success probability using weighted factors
        let success_probability = Self::calculate_success_probability(&factors);

        // Calculate risk level (inverse of some success factors)
        let risk_level = Self::calculate_risk_level(&factors);

        // Calculate expected value considering success probability and tactical value
        let expected_value = success_probability * tactical_value;

        // Determine if pass is recommended based on thresholds. Players with
        // killer-ball / playmaker PPMs are willing to attempt riskier passes
        // because they value the through ball / chance-creation upside.
        // Which traits flag a player as risk-tolerant lives in the trait
        // registry (`risk_tolerant_passer` field) — adding a new such
        // trait no longer requires touching this evaluator.
        let risk_tolerant = has_risk_tolerant_passing_trait(&passer.traits);
        let is_recommended = PassEvaluatorConfig::default().is_recommended(
            success_probability,
            risk_level,
            risk_tolerant,
        );

        PassEvaluation {
            success_probability,
            risk_level,
            expected_value,
            factors,
            is_recommended,
        }
    }

    /// Calculate how distance affects pass success
    fn calculate_distance_factor(distance: f32, passer: &MatchPlayer) -> f32 {
        let cfg = PassEvaluatorConfig::default();
        let passing_skill = passer.skills.technical.passing;
        let vision_skill = passer.skills.mental.vision;
        let technique_skill = passer.skills.technical.technique;

        // Vision and technique extend effective passing range. The
        // bonus values are baked into the config helper calls below;
        // raw `(vision_skill / scale)` is still used inside the
        // long-pass skill-factor branches further down.
        let optimal_range = cfg.optimal_range(passing_skill, vision_skill);
        let max_effective_range = cfg.max_effective_range(passing_skill, vision_skill);
        let ultra_long_threshold = cfg.ultra_long_threshold;
        let extreme_long_threshold = cfg.extreme_long_threshold;

        if distance <= optimal_range {
            // Short to medium passes - very high success
            1.0 - (distance / optimal_range * 0.1)
        } else if distance <= max_effective_range {
            // Long passes (60-100m) - declining success (less penalty with high vision)
            let excess = distance - optimal_range;
            let range = max_effective_range - optimal_range;
            let base_decline = 0.9 - (excess / range * 0.5);
            // Vision reduces the decline penalty
            base_decline + (vision_skill / 20.0 * 0.1)
        } else if distance <= ultra_long_threshold {
            // Very long passes (100-200m) - vision and technique critical
            let excess = distance - max_effective_range;
            let range = ultra_long_threshold - max_effective_range;
            let skill_factor = (vision_skill / 20.0 * 0.6) + (technique_skill / 20.0 * 0.3);

            let base_factor = 0.4 - (excess / range * 0.25);
            (base_factor + skill_factor * 0.2).clamp(0.15, 0.55)
        } else if distance <= extreme_long_threshold {
            // Ultra-long passes (200-300m) - only elite players can execute
            let skill_factor = (vision_skill / 20.0 * 0.7) + (technique_skill / 20.0 * 0.3);

            // Require high skills for these passes
            if skill_factor > 0.7 {
                (0.3 + skill_factor * 0.15).clamp(0.2, 0.45)
            } else if skill_factor > 0.5 {
                (0.2 + skill_factor * 0.1).clamp(0.15, 0.35)
            } else {
                0.1
            }
        } else {
            // Extreme long passes (300m+) - goalkeeper clearances, desperate plays
            let skill_factor = (vision_skill / 20.0 * 0.5)
                + (technique_skill / 20.0 * 0.35)
                + (passing_skill / 20.0 * 0.15);

            if skill_factor > 0.8 {
                0.35
            } else if skill_factor > 0.6 {
                0.2
            } else {
                0.1
            }
        }
    }

    /// Calculate how the angle between passer's facing and pass direction affects success
    fn calculate_angle_factor(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        let cfg = PassEvaluatorConfig::default();
        let pass_direction = (receiver.position - passer.position).normalize();
        let passer_velocity = ctx.tick_context.positions.players.velocity(passer.id);

        if passer_velocity.norm() < cfg.stationary_velocity_threshold {
            // Standing still - can pass in any direction easily
            return cfg.stationary_angle_factor;
        }

        let facing_direction = passer_velocity.normalize();
        let dot_product = pass_direction.dot(&facing_direction);
        cfg.angle_factor_from_dot(dot_product)
    }

    /// Calculate pressure on the passer from opponents
    fn calculate_pressure_factor(ctx: &StateProcessingContext, passer: &MatchPlayer) -> f32 {
        let pressure_radius = PassEvaluatorConfig::default().pressure_radius;

        // Compute closest distance and count without allocation
        let mut closest_distance = pressure_radius;
        let mut num_opponents: f32 = 0.0;

        for (_, dist) in ctx.tick_context.grid.opponents(passer.id, pressure_radius) {
            num_opponents += 1.0;
            if dist < closest_distance {
                closest_distance = dist;
            }
        }

        if num_opponents == 0.0 {
            return 1.0; // No pressure
        }

        // Pressure from distance
        let distance_pressure = (closest_distance / pressure_radius).clamp(0.0, 1.0);

        // Additional pressure from multiple opponents
        let number_pressure = (1.0 - (num_opponents - 1.0) * 0.15).max(0.5);

        // Mental attributes help under pressure
        let composure_factor = passer.skills.mental.composure / 20.0;
        let decision_factor = passer.skills.mental.decisions / 20.0;

        let base_pressure = distance_pressure * number_pressure;
        let pressure_with_mentals =
            base_pressure + (1.0 - base_pressure) * composure_factor * decision_factor;

        pressure_with_mentals.clamp(0.3, 1.0)
    }

    /// Evaluate receiver's positioning quality
    fn calculate_receiver_positioning(
        ctx: &StateProcessingContext,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        const VERY_CLOSE_RADIUS: f32 = 8.0;
        const CLOSE_RADIUS: f32 = 18.0;
        const MEDIUM_RADIUS: f32 = 30.0;

        // Count opponents in each zone without allocation (single pass)
        let mut very_close_opponents: usize = 0;
        let mut close_opponents: usize = 0;
        let mut medium_opponents: usize = 0;

        for (_, dist) in ctx.tick_context.grid.opponents(receiver.id, MEDIUM_RADIUS) {
            if dist < VERY_CLOSE_RADIUS {
                very_close_opponents += 1;
            } else if dist < CLOSE_RADIUS {
                close_opponents += 1;
            } else {
                medium_opponents += 1;
            }
        }

        // Calculate space quality with heavy penalties for nearby opponents
        let space_factor = if very_close_opponents > 0 {
            // Very tightly marked - poor option
            0.15 - (very_close_opponents as f32 * 0.1).min(0.12)
        } else if close_opponents > 0 {
            // Marked — risky target
            0.45 - (close_opponents as f32 * 0.15).min(0.3)
        } else if medium_opponents > 0 {
            // Some pressure but workable
            0.75 - (medium_opponents as f32 * 0.1).min(0.2)
        } else {
            // Completely free - excellent option
            1.0
        };

        // Check if receiver is moving into space or standing still
        let receiver_velocity = ctx.tick_context.positions.players.velocity(receiver.id);
        let movement_factor = if receiver_velocity.norm() > 1.5 {
            // Moving into space - excellent
            1.15
        } else if receiver_velocity.norm() > 0.5 {
            // Some movement - good
            1.05
        } else {
            // Standing still - acceptable but not ideal
            0.95
        };

        // Off the ball movement skill affects positioning quality
        let players = ctx.player();
        let skills = players.skills(receiver.id);

        let off_ball_factor = skills.mental.off_the_ball / 20.0;
        let positioning_factor = skills.mental.positioning / 20.0;

        (space_factor
            * movement_factor
            * (0.7 + off_ball_factor * 0.15 + positioning_factor * 0.15))
            .clamp(0.1, 1.0)
    }

    /// Calculate passer's ability to execute this pass
    fn calculate_passer_ability(
        _ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        distance: f32,
    ) -> f32 {
        let passing_skill = passer.skills.technical.passing / 20.0;
        let technique_skill = passer.skills.technical.technique / 20.0;
        let vision_skill = passer.skills.mental.vision / 20.0;

        // For short passes, technique matters more
        // For long passes, passing skill matters more
        let short_pass_weight = 1.0 - (distance / 100.0).min(1.0);

        let ability = passing_skill * (0.5 + short_pass_weight * 0.2)
            + technique_skill * (0.3 + short_pass_weight * 0.2)
            + vision_skill * 0.2;

        // Condition affects ability
        let condition_factor = passer.player_attributes.condition as f32 / 10000.0;

        (ability * condition_factor).clamp(0.3, 1.0)
    }

    /// Calculate receiver's ability to control the pass
    fn calculate_receiver_ability(ctx: &StateProcessingContext, receiver: &MatchPlayerLite) -> f32 {
        let players = ctx.player();
        let skills = players.skills(receiver.id);

        let first_touch = skills.technical.first_touch / 20.0;
        let technique = skills.technical.technique / 20.0;
        let anticipation = skills.mental.anticipation / 20.0;

        let ability = first_touch * 0.5 + technique * 0.3 + anticipation * 0.2;

        // Condition affects ability
        let player_attributes = players.attributes(receiver.id);
        let condition_factor = player_attributes.condition as f32 / 10000.0;

        (ability * condition_factor).clamp(0.3, 1.0)
    }

    /// Calculate tactical value of the pass
    fn calculate_tactical_value(ctx: &StateProcessingContext, receiver: &MatchPlayerLite) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let receiver_position = receiver.position;
        let passer_position = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;
        let field_center_y = field_height / 2.0;

        // Determine which direction is forward based on player side.
        // Use the `PlayerSide` helpers so right-side normalization stays
        // correct — see `PlayerSide::attacking_progress_x` for why the
        // legacy `x * dir / width` formula was buggy.
        let side = ctx.player.side.unwrap_or(PlayerSide::Left);
        let field_width = ctx.context.field_size.width as f32;

        // Forward progress as a signed [-1, 1]-ish ratio.
        let forward_progress =
            side.forward_delta_norm(ball_position.x, receiver_position.x, field_width);

        // Strong penalty for backward passes, strong reward for forward
        // Defenders get extra penalty for backward passes since they're already deep
        let is_defender = ctx
            .player
            .tactical_position
            .current_position
            .position_group()
            == PlayerFieldPositionGroup::Defender;

        // Penalize pure sideways passes that don't progress the ball
        // But exempt wide switches — lateral passes that spread the play are valuable
        let lateral_change = (receiver_position.y - passer_position.y).abs();
        let forward_change = side
            .forward_delta(passer_position.x, receiver_position.x)
            .abs();
        let sideways_penalty = if forward_change < 10.0 && lateral_change > 20.0 {
            if lateral_change > field_height * 0.25 {
                // Wide switch — this is good, no penalty
                0.0
            } else {
                // Short sideways pass in a cluster — discourage
                -0.25
            }
        } else {
            0.0
        };

        // Phase-aware modulation — modern football varies its
        // forward/backward valuation by team phase. In settled build-up
        // a backward pass to the keeper or CB is a normal recycle, not
        // a sin; in transition it's the death of the counter. The
        // multipliers below come straight off `team().phase()` so every
        // player on the side reads the same tactical weather.
        let phase = ctx.team().phase();
        let (phase_forward_mult, phase_backward_mult): (f32, f32) = match phase {
            // Recycling is correct, line-breaking forward less critical.
            GamePhase::BuildUp => (0.65, 0.30),
            // Direct: every forward yard is gold.
            GamePhase::AttackingTransition => (1.40, 1.20),
            // Cutbacks and resets to the edge of the box are normal.
            GamePhase::Attack => (1.05, 0.55),
            // Standard: forward ≥ backward.
            GamePhase::Progression => (1.00, 1.00),
            // Settled defending — out of possession, but if a turnover
            // gives the ball briefly we'd still want a forward look.
            _ => (1.00, 1.00),
        };

        // Risk appetite biases forward over backward. Late chase = the
        // pass evaluator should over-prefer the forward option.
        let risk_appetite = ctx.team().risk_appetite();
        let risk_forward_bias = 0.7 + risk_appetite * 0.6; // 0.7..1.3
        let risk_backward_bias = 1.4 - risk_appetite * 0.8; // 1.4..0.6

        let forward_value = if forward_progress < 0.0 {
            // Backward pass - penalty, but softened by phase + risk.
            let composure_reduction = (ctx.player.skills.mental.composure / 20.0) * 0.3;
            let base_penalty = forward_progress * 3.0 * (1.0 - composure_reduction).max(0.5);
            let phase_adjusted = base_penalty * phase_backward_mult * risk_backward_bias;
            if is_defender {
                // Defenders: residual backward penalty — even in build-up
                // we don't want CBs hoof-ing back into pressure for fun.
                // The phase factor already eased the penalty, so the 1.5x
                // multiplier still holds shape but on a softer base.
                phase_adjusted * 1.5
            } else {
                phase_adjusted
            }
        } else {
            // Forward pass - strong reward, especially in transition.
            if is_defender {
                forward_progress * 3.0 * phase_forward_mult * risk_forward_bias
            } else {
                forward_progress * 2.5 * phase_forward_mult * risk_forward_bias
            }
        };

        // Distance bonus: prefer passes of 20-50m over very short (< 15m) or very long
        let pass_distance = (receiver_position - passer_position).norm();
        let distance_value = if pass_distance < 10.0 {
            // Very short pass - only good under pressure
            0.3
        } else if pass_distance < 20.0 {
            // Short pass - acceptable
            0.6
        } else if pass_distance < 50.0 {
            // Ideal passing range - good progression
            1.0
        } else if pass_distance < 80.0 {
            // Long pass - still valuable
            0.8
        } else if pass_distance < 120.0 {
            // Long pass - declining value
            0.5
        } else if pass_distance < 200.0 {
            // Very long pass - risky
            0.3
        } else {
            // Extreme distance - rarely accurate
            let vision_skill = ctx.player.skills.mental.vision / 20.0;
            0.2 * vision_skill
        };

        // === WIDTH AND FLANKS BONUS ===
        // Reward passes to wide positions - creates more varied play
        let receiver_distance_from_center = (receiver_position.y - field_center_y).abs();
        let passer_distance_from_center = (passer_position.y - field_center_y).abs();

        // How wide is the receiver? (0.0 = center, 1.0 = touchline)
        let receiver_width_ratio =
            (receiver_distance_from_center / (field_height / 2.0)).clamp(0.0, 1.0);
        let passer_width_ratio =
            (passer_distance_from_center / (field_height / 2.0)).clamp(0.0, 1.0);

        // Width bonus - reward passes to wide areas
        // Extra bonus if passer is central and receiver is wide (spreading play)
        let spreading_play_bonus = if passer_width_ratio < 0.4 && receiver_width_ratio > 0.5 {
            0.25 // Central player finding wide teammate — strong incentive
        } else {
            0.0
        };

        // Midfielder-specific width incentive — midfielders should distribute wide
        let is_midfielder = ctx
            .player
            .tactical_position
            .current_position
            .position_group()
            == PlayerFieldPositionGroup::Midfielder;
        let midfielder_width_bonus = if is_midfielder && receiver_width_ratio > 0.4 {
            0.15 // Midfielders get extra reward for wide distribution
        } else {
            0.0
        };

        let width_bonus = if receiver_width_ratio > 0.7 {
            // Very wide (near touchline) - excellent for stretching play
            0.5 + spreading_play_bonus + midfielder_width_bonus
        } else if receiver_width_ratio > 0.5 {
            // Wide areas - good for creating space
            0.35 + spreading_play_bonus + midfielder_width_bonus
        } else if receiver_width_ratio > 0.3 {
            // Half-spaces - valuable attacking zones
            0.2 + midfielder_width_bonus
        } else {
            // Central - no bonus (already gets forward progress bonus usually)
            0.0
        };

        // === SWITCHING PLAY BONUS ===
        // Reward passes that switch the play from one side to the other
        let lateral_change = (receiver_position.y - passer_position.y).abs();
        let is_switching_play = lateral_change > field_height * 0.3; // Significant lateral movement

        let switch_play_bonus = if is_switching_play {
            let vision_skill = ctx.player.skills.mental.vision / 20.0;
            // Big bonus for switching play - opens up space
            0.45 + (vision_skill * 0.25)
        } else {
            0.0
        };

        // Side-overload is now a single path: `same_side_density_penalty`
        // below (driven by the team-shared `side_density_*` signals).
        // The legacy half-pitch overload_penalty was double-counting the
        // same situation and was removed during the polish pass.

        // Long cross-field passes - reward vision players for switching play
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let technique_skill = ctx.player.skills.technical.technique / 20.0;

        let long_pass_bonus = if pass_distance > 300.0 {
            // Extreme distance (300m+) - very risky, minimal bonus
            (vision_skill * 0.3 + technique_skill * 0.2) * 0.2
        } else if pass_distance > 200.0 {
            // Ultra-long diagonal (200-300m) - risky
            (vision_skill * 0.3 + technique_skill * 0.15) * 0.2
        } else if pass_distance > 100.0 {
            // Very long pass (100-200m) - small bonus for high vision
            vision_skill * 0.15
        } else if pass_distance > 60.0 {
            // Long pass (60-100m) - modest bonus
            vision_skill * 0.1
        } else {
            0.0
        };

        // Passes to advanced positions are more valuable
        let position_value = match receiver.tactical_positions.position_group() {
            PlayerFieldPositionGroup::Forward => 1.0,
            PlayerFieldPositionGroup::Midfielder => 0.7,
            PlayerFieldPositionGroup::Defender => 0.4,
            PlayerFieldPositionGroup::Goalkeeper => 0.2,
        };

        // === CUTBACK / HIGH-xG RECEIVER BONUS ===
        // Modern football: a pass from the byline that pulls the ball
        // BACK to a runner at the penalty spot is one of the highest-xG
        // passes there is. The classical evaluator scored that as a
        // backward sideways ball and slammed it. We now detect:
        //   * passer is wide AND inside the attacking third
        //     (using `attacking_progress_x` so right-side teams aren't
        //     locked out by the legacy negative-progress bug)
        //   * receiver is in the central high-xG corridor near opp goal
        //   * pass distance is short-to-medium (real cutbacks, not
        //     desperate long crosses)
        // The bonus is graded on receiver space, passer decisions, and
        // teamwork — a tight cutback under heavy marking is worth less.
        let passer_progress = side.attacking_progress_x(passer_position.x, field_width);
        let receiver_progress = side.attacking_progress_x(receiver_position.x, field_width);
        let receiver_y_offset = (receiver_position.y - field_center_y).abs();
        let passer_y_offset = (passer_position.y - field_center_y).abs();
        let cutback_pattern = passer_progress > 0.70
            && receiver_progress > 0.78
            && receiver_y_offset < field_height * 0.15
            && passer_y_offset > field_height * 0.20
            && pass_distance < 60.0;
        let cutback_bonus = if cutback_pattern {
            // Receiver space inferred from receiver_positioning (already
            // computed above as one of the PassFactors): higher = freer.
            // Range 0.30 .. 0.50 per spec.
            let receiver_space_factor = {
                // Re-read receiver positioning instead of plumbing the
                // factors through — the math is dominated by opponent
                // proximity, which is what we want here.
                let opps = ctx.tick_context.grid.opponents(receiver.id, 12.0).count();
                match opps {
                    0 => 1.0,
                    1 => 0.6,
                    _ => 0.2,
                }
            };
            let decisions = (ctx.player.skills.mental.decisions / 20.0).clamp(0.0, 1.0);
            let teamwork = (ctx.player.skills.mental.teamwork / 20.0).clamp(0.0, 1.0);
            (0.30 + receiver_space_factor * 0.10
                + decisions * 0.05
                + teamwork * 0.05)
                .clamp(0.30, 0.50)
        } else {
            0.0
        };

        // === BUILD-UP RECYCLING BONUS ===
        // In build-up, a short pass to a CB / DM / GK that resets play
        // is a healthy modern pattern, not a panic option. Gated on:
        //   * phase == BuildUp
        //   * pass distance 12..65 u (genuine recycle, not a hoof or
        //     a one-touch trade)
        //   * receiver is GK/CB/DM
        //   * passer under press OR build_up_patience > 0.65
        // Range 0.15 .. 0.40 — 0.15 baseline, +0.25 if all conditions
        // including pressure are present.
        // `is_defender()` already includes the DefensiveMidfielder
        // role (see `PlayerPositionType::position_group`), so this
        // covers GK + CB + DM together.
        let receiver_is_recycle_target = receiver.tactical_positions.is_defender()
            || matches!(
                receiver.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Goalkeeper
            );
        let build_up_recycle_bonus = if matches!(phase, GamePhase::BuildUp)
            && pass_distance >= 12.0
            && pass_distance <= 65.0
            && receiver_is_recycle_target
        {
            let under_press =
                ctx.players().opponents().nearby(12.0).next().is_some();
            let patient = ctx.team().build_up_patience() > 0.65;
            if under_press || patient {
                let mut bonus: f32 = 0.15;
                if under_press {
                    bonus += 0.15;
                }
                if patient {
                    bonus += 0.10;
                }
                bonus.clamp(0.15, 0.40)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // === COUNTER-PRESS DIRECT-FIRST-PASS BONUS ===
        // After winning the ball back, the first pass should be direct
        // — feed a forward making a run. Gated additionally on the
        // receiver actually having forward space and the pass not
        // running through opponents (low interception risk implied by
        // receiver_positioning > 0.5).
        let counter_first_pass_bonus = if matches!(phase, GamePhase::AttackingTransition)
            && forward_value > 0.0
            && receiver.tactical_positions.is_forward()
        {
            // Gate on receiver having space — checked by counting
            // opponents in their immediate area.
            let receiver_opps = ctx.tick_context.grid.opponents(receiver.id, 15.0).count();
            if receiver_opps == 0 {
                0.40
            } else if receiver_opps == 1 {
                0.30
            } else {
                // Crowded receiver — direct ball is wasted. Skip bonus.
                0.0
            }
        } else {
            0.0
        };

        // === SIDE-DENSITY OVERLOAD ===
        // Use the team-shared side density signal: too many of OUR
        // players on one side discourages another pass into that side
        // and rewards a switch. `ball_side` already tells us which
        // lateral third the ball is in (= the pass-source side, modulo
        // ball motion).
        let team_state = ctx.context.tactical_for_team(ctx.player.team_id);
        let receiver_side_zone =
            BallSideZone::for_y(field_height, receiver_position.y);
        let receiver_side_density = match receiver_side_zone {
            BallSideZone::Left => team_state.side_density_left,
            BallSideZone::Center => team_state.side_density_center,
            BallSideZone::Right => team_state.side_density_right,
        };
        let same_side_density_penalty =
            Self::same_side_density_penalty(receiver_side_density);
        // Reward switches to underloaded sides with a vision-graded
        // bonus. Two-band threshold: any pass that crosses lateral
        // thirds and lands in a side with ≤3 own players counts.
        let passer_side_zone = BallSideZone::for_y(field_height, passer_position.y);
        let crosses_sides = passer_side_zone != receiver_side_zone;
        let vision = (ctx.player.skills.mental.vision / 20.0).clamp(0.0, 1.0);
        let underload_switch_bonus = Self::underload_switch_bonus(
            crosses_sides,
            receiver_side_density,
            vision,
        );

        // Cap the combined "switch reward" so a wide-vision playmaker
        // doesn't double-dip the classic switch_play_bonus and the
        // density-driven underload_switch_bonus. Polish spec: total
        // switch reward ≤ 0.45. Applied flat (not re-weighted): that
        // ceiling is the absolute contribution to tactical_value.
        let switch_total = (switch_play_bonus + underload_switch_bonus).min(0.45);

        // Weighted combination - includes width and switching bonuses.
        // Phase-aware bonuses (cutback, build-up recycle, counter first
        // pass) are added flat — they're already gated tightly on
        // phase + receiver type so they only fire in the situations
        // they were designed for. Side-overload is owned by
        // `same_side_density_penalty` (legacy half-pitch path removed).
        let mut tactical_value = forward_value * 0.32 +
            distance_value * 0.10 +
            position_value * 0.08 +
            long_pass_bonus * 0.05 +
            width_bonus * 0.22 +
            switch_total +                   // Capped flat: classic + underload ≤ 0.45
            cutback_bonus +
            build_up_recycle_bonus +
            counter_first_pass_bonus +
            same_side_density_penalty +
            sideways_penalty;

        // PPM biases. Players with killer-ball / playmaker traits love the
        // forward pass and should see it as more valuable even when risky.
        // Trait-driven switch boosts apply to switch_total (the capped
        // sum) so the 0.45 ceiling is the single switching budget for
        // the whole tactical_value calculation.
        let passer = ctx.player;
        let forward_trait_bias = passer.has_trait(PlayerTrait::TriesThroughBalls)
            || passer.has_trait(PlayerTrait::KillerBallOften);
        if forward_trait_bias && forward_value > 0.0 {
            tactical_value += forward_value * 0.25;
        }
        if passer.has_trait(PlayerTrait::Playmaker) {
            if forward_value > 0.0 {
                tactical_value += forward_value * 0.20;
            }
            tactical_value += switch_total * 0.10;
        }
        if passer.has_trait(PlayerTrait::LikesToSwitchPlay) {
            tactical_value += switch_total * 0.15;
        }
        if passer.has_trait(PlayerTrait::PlaysShortPasses) {
            tactical_value -= long_pass_bonus * 0.20;
        }
        if passer.has_trait(PlayerTrait::PlaysLongPasses) {
            tactical_value += long_pass_bonus * 0.15;
        }

        // Allow negative tactical values for backward passes
        tactical_value.clamp(-0.5, 1.8)
    }

    /// Calculate overall success probability from factors
    fn calculate_success_probability(factors: &PassFactors) -> f32 {
        // Weighted combination of all factors
        // Receiver positioning is the dominant factor — free players are far better targets
        let probability = factors.distance_factor * 0.10 +
                factors.angle_factor * 0.08 +
                factors.pressure_factor * 0.08 +
                factors.receiver_positioning * 0.40 +  // Dominant: free receivers are far better
                factors.passer_ability * 0.10 +
                factors.receiver_ability * 0.08 +
                factors.tactical_value * 0.16;

        probability.clamp(0.1, 0.99)
    }

    /// Calculate overall risk level
    fn calculate_risk_level(factors: &PassFactors) -> f32 {
        // Risk is inverse of safety factors
        // Poor receiver positioning (crowded by opponents) is now a major risk
        let risk = (1.0 - factors.distance_factor) * 0.20 +
                (1.0 - factors.pressure_factor) * 0.20 +
                (1.0 - factors.receiver_positioning) * 0.40 +  // Increased from 0.20
                (1.0 - factors.receiver_ability) * 0.20;

        risk.clamp(0.0, 1.0)
    }

    /// Calculate interception risk from opponents along the pass path
    fn calculate_interception_risk(
        ctx: &StateProcessingContext,
        passer: &MatchPlayer,
        receiver: &MatchPlayerLite,
    ) -> f32 {
        let pass_vector = receiver.position - passer.position;
        let pass_distance = pass_vector.norm();
        let pass_direction = pass_vector.normalize();

        // Minimum distance along the pass line before an opponent counts as a blocker.
        // A pressing opponent near the passer cannot intercept a driven forward pass —
        // the ball clears them before they can react. In real football this is ~10m (~20 units).
        // Use 25% of pass distance as alternative for short passes.
        let min_intercept_projection = 20.0_f32.min(pass_distance * 0.25);

        // Check for opponents who could intercept the pass
        let intercepting_opponents = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let to_opponent = opponent.position - passer.position;
                let projection_distance = to_opponent.dot(&pass_direction);

                // Ignore opponents behind passer, past receiver, or too close to passer
                if projection_distance <= min_intercept_projection
                    || projection_distance >= pass_distance
                {
                    return false;
                }

                // Calculate perpendicular distance from pass line
                let projected_point = passer.position + pass_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).norm();

                // Consider opponent's interception ability
                let players = ctx.player();
                let opponent_skills = players.skills(opponent.id);
                let interception_ability = opponent_skills.technical.tackling / 20.0;
                let anticipation = opponent_skills.mental.anticipation / 20.0;

                // Better opponents can intercept from further away
                let effective_radius = 3.0 + (interception_ability + anticipation) * 2.0;

                perp_distance < effective_radius
            })
            .count();

        // Convert count to risk factor — aggressive penalties to prevent suicidal passes
        if intercepting_opponents == 0 {
            0.0 // No risk
        } else if intercepting_opponents == 1 {
            0.55 // Significant risk — one opponent in the lane
        } else if intercepting_opponents == 2 {
            0.85 // Very high risk — two opponents blocking
        } else {
            0.97 // Near-certain interception
        }
    }

    /// Find the best pass option from available teammates with skill-based personality
    /// Returns (teammate, reason) tuple
    pub fn find_best_pass_option(
        ctx: &StateProcessingContext,
        max_distance: f32,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        // Determine player's passing personality based on skills
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let flair_skill = ctx.player.skills.mental.flair / 20.0;
        let decision_skill = ctx.player.skills.mental.decisions / 20.0;
        let composure_skill = ctx.player.skills.mental.composure / 20.0;
        let teamwork_skill = ctx.player.skills.mental.teamwork / 20.0;
        let _anticipation_skill = ctx.player.skills.mental.anticipation / 20.0;

        // Define passing personalities
        let is_playmaker = vision_skill > 0.75 && flair_skill > 0.65; // Creative, through balls
        let is_direct = flair_skill > 0.7 && pass_skill > 0.65; // Risky, aggressive forward passes
        let is_conservative = decision_skill < 0.5 || composure_skill < 0.5; // Safe, sideways passes
        let is_team_player = teamwork_skill > 0.75 && pass_skill > 0.65; // Finds best positioned teammates
        let is_pragmatic = decision_skill > 0.75 && pass_skill > 0.6; // Smart, calculated passes

        // Calculate minimum pass distance based on pressure
        // NOTE: This filter prevents "too short" passes that don't progress the ball
        let is_under_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let min_pass_distance = if is_under_pressure {
            // Under pressure, allow shorter passes but still avoid huddle passes
            12.0
        } else {
            // Not under pressure, still allow short-to-medium passes
            20.0
        };

        for teammate in ctx.players().teammates().nearby(max_distance) {
            // GRADUATED RECENCY PENALTY: Penalize recent passers instead of hard-skipping
            let recency_penalty = ctx.ball().passer_recency_penalty(teammate.id);

            let pass_distance = (teammate.position - ctx.player.position).norm();

            // MINIMUM DISTANCE FILTER: Skip teammates that are too close unless under pressure
            if pass_distance < min_pass_distance {
                continue;
            }

            // CONGESTION PENALTY: Heavily penalize passing into crowded areas.
            // Opponents near the receiver are weighted more heavily than teammates,
            // and close opponents are weighted much more than distant ones.
            let nearby_teammates_count = ctx
                .tick_context
                .grid
                .teammates(teammate.id, 0.0, 50.0)
                .count();
            let close_opponents_count = ctx.tick_context.grid.opponents(teammate.id, 30.0).count();
            let medium_opponents_count =
                ctx.tick_context.grid.opponents(teammate.id, 60.0).count() - close_opponents_count;

            // Close opponents count triple — passing into tight marking is very risky
            let weighted_nearby =
                nearby_teammates_count + close_opponents_count * 3 + medium_opponents_count;

            let congestion_penalty = match weighted_nearby {
                0 => 1.8,  // Completely isolated — excellent target
                1 => 1.3,  // One nearby player — good
                2 => 0.9,  // Normal
                3 => 0.4,  // Getting crowded — discouraged
                4 => 0.15, // Congested — strongly discouraged
                5 => 0.06, // Huddle — almost never pass here
                _ => 0.02, // Extremely congested — effectively blocked
            };

            let evaluation = Self::evaluate_pass(ctx, ctx.player, &teammate);
            let interception_risk = Self::calculate_interception_risk(ctx, ctx.player, &teammate);

            // Base positioning bonus
            let positioning_bonus = evaluation.factors.receiver_positioning * 2.0;

            // Skill-based space quality evaluation
            let space_quality = if is_conservative {
                // Conservative players prefer free receivers but less extreme
                if evaluation.factors.receiver_positioning > 0.85 {
                    1.8 // Reduced from 2.0 - completely free players
                } else if evaluation.factors.receiver_positioning > 0.65 {
                    1.3 // Increased from 1.2 - good space
                } else if evaluation.factors.receiver_positioning > 0.45 {
                    0.8 // New tier - acceptable space
                } else {
                    0.4 // Increased from 0.3 - will attempt if needed
                }
            } else if is_playmaker {
                // Playmakers trust teammates to handle some pressure
                if evaluation.factors.receiver_positioning > 0.75 {
                    1.7 // Increased from 1.6
                } else if evaluation.factors.receiver_positioning > 0.5 {
                    1.4 // Increased from 1.3 - still okay with moderate space
                } else if evaluation.factors.receiver_positioning > 0.3 {
                    1.0 // New tier - willing to attempt tighter passes
                } else {
                    0.7 // Reduced penalty for very tight spaces
                }
            } else if is_direct {
                // Direct players less concerned about space, more about attacking position
                if evaluation.factors.receiver_positioning > 0.6 {
                    1.6 // Increased from 1.5
                } else if evaluation.factors.receiver_positioning > 0.4 {
                    1.2 // New tier
                } else {
                    0.9 // Reduced from 1.0 - will attempt most passes
                }
            } else {
                // Standard space evaluation - slightly more aggressive
                if evaluation.factors.receiver_positioning > 0.75 {
                    1.6 // Increased from 1.5
                } else if evaluation.factors.receiver_positioning > 0.55 {
                    1.3 // Increased from 1.2
                } else if evaluation.factors.receiver_positioning > 0.35 {
                    1.0 // Improved threshold from 0.4
                } else {
                    0.7 // Increased from 0.6
                }
            };

            // Skill-based interception risk tolerance — higher = more penalty applied
            let risk_tolerance = if is_direct {
                0.5 // Still somewhat aggressive but respects blockers
            } else if is_conservative {
                0.9 // Almost never pass through opponents
            } else if is_playmaker {
                0.6 // Moderate — will try creative passes but not suicidal
            } else {
                0.7 // Standard — significant penalty for blocked lanes
            };

            let interception_penalty = 1.0 - (interception_risk * risk_tolerance);

            // Add distance preference bonus - widened optimal range to encourage penetration
            let optimal_distance_bonus = if is_under_pressure {
                // Under pressure, all safe passes are good
                1.0
            } else if pass_distance >= 20.0 && pass_distance <= 70.0 {
                // Widened optimal range (was 15-40m, now 20-70m) for penetrating passes
                1.4 // Increased from 1.3
            } else if pass_distance >= 15.0 && pass_distance < 20.0 {
                // Short passes - acceptable
                1.1 // New tier
            } else if pass_distance < 15.0 {
                // Very short - strongly discouraged (keeps ball in huddle)
                0.4
            } else if pass_distance <= 100.0 {
                // Long passes (70-100m) - moderate value
                1.1
            } else if pass_distance <= 150.0 {
                // Very long passes - declining value
                0.85
            } else {
                // Extreme long passes - discouraged
                0.6
            };

            // Distance preference based on personality
            let distance_preference = if is_playmaker {
                // Playmakers prefer through balls but not unrealistic long passes
                if pass_distance > 300.0 {
                    // Extreme passes - very risky even for elite
                    if vision_skill > 0.85 { 1.1 } else { 0.6 }
                } else if pass_distance > 200.0 {
                    // Ultra-long switches - risky
                    if vision_skill > 0.75 { 1.15 } else { 0.8 }
                } else if pass_distance > 100.0 {
                    1.2 // Long passes - moderate bonus
                } else if pass_distance > 80.0 {
                    1.25 // Medium-long - sweet spot for playmakers
                } else if pass_distance > 50.0 {
                    1.2
                } else {
                    1.0
                }
            } else if is_direct {
                // Direct players strongly prefer forward passes
                let side_now = ctx.player.side.unwrap_or(PlayerSide::Left);
                let forward_progress =
                    side_now.forward_delta(ctx.player.position.x, teammate.position.x);
                if forward_progress > 0.0 {
                    1.4
                } else {
                    0.5 // Strongly avoid backward passes
                }
            } else if is_conservative {
                // Conservative players prefer short, safe passes
                if pass_distance < 30.0 {
                    1.4
                } else if pass_distance < 50.0 {
                    1.0
                } else {
                    0.7 // Avoid long passes
                }
            } else if is_team_player {
                // Team players maximize teammate positioning
                1.0 + (evaluation.factors.receiver_positioning * 0.3)
            } else if is_pragmatic {
                // Pragmatic players balance all factors
                if evaluation.expected_value > 0.6 {
                    1.3 // Good tactical value
                } else {
                    1.0
                }
            } else {
                1.0
            };

            // GOALKEEPER PENALTY: Almost completely eliminate passing to goalkeeper
            let is_goalkeeper = matches!(
                teammate.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Goalkeeper
            );

            let goalkeeper_penalty = if is_goalkeeper {
                // Side-correct math via PlayerSide helpers. The
                // previous formulas
                //   `(teammate.x - player.x) * dir < 0`
                //   `(player.x * dir) / width > 0.66`
                // were wrong for right-side teams (the second produced
                // negative values which can never exceed 0.66, so a
                // right-side team was never classified as "in attacking
                // third" — which silently broke the block that
                // SHOULD reject GK passes from advanced positions).
                let side = ctx.player.side.unwrap_or(PlayerSide::Left);
                let is_backward_pass =
                    side.forward_delta(ctx.player.position.x, teammate.position.x) < 0.0;

                let field_width = ctx.context.field_size.width as f32;
                let player_progress =
                    side.attacking_progress_x(ctx.player.position.x, field_width);
                let in_attacking_third = player_progress > 0.66;

                let phase_now = ctx.team().phase();
                if in_attacking_third && is_backward_pass {
                    // In attacking third, passing backward to GK is NEVER acceptable
                    0.00001 // Virtually zero
                } else if matches!(phase_now, GamePhase::BuildUp) && is_backward_pass {
                    // Build-up to GK is a normal modern pattern: pivot
                    // through the keeper to escape the press / switch
                    // play. Allow it as a real option (much higher than
                    // the legacy ~0.0001 ceiling) but only when the
                    // passer is genuinely under pressure or wants to
                    // recycle (low risk_appetite).
                    let under_press =
                        ctx.player().pressure().is_under_immediate_pressure_with_distance(8.0);
                    let recycle_intent = ctx.team().risk_appetite() < 0.45;
                    if under_press || recycle_intent {
                        // GK is a real option in build-up under press,
                        // not the only option.
                        0.55
                    } else {
                        // Build-up but no genuine recycle trigger —
                        // still allow but discount.
                        0.10
                    }
                } else if is_backward_pass {
                    // Backward pass to GK in middle/defensive third - still very bad
                    0.0001
                } else if evaluation.factors.pressure_factor < 0.2 {
                    // Forward/sideways pass under EXTREME pressure - GK is emergency option
                    0.02
                } else {
                    // Normal play - virtually eliminate GK passes
                    0.0005
                }
            } else {
                1.0 // No penalty for non-GK
            };

            // Calculate final score with personality-based weighting
            let score = if evaluation.factors.pressure_factor < 0.5 {
                // Under heavy pressure - personality affects decision
                if is_conservative {
                    // Conservative: safety is paramount
                    (evaluation.success_probability * 2.0 + positioning_bonus)
                        * interception_penalty
                        * space_quality
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else if is_direct {
                    // Direct: still look for forward options
                    (evaluation.expected_value * 1.5 + positioning_bonus * 0.3)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else {
                    // Others: prioritize safety AND space
                    (evaluation.success_probability + positioning_bonus)
                        * interception_penalty
                        * space_quality
                        * 1.3
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                }
            } else {
                // Normal situation - personality-based preferences apply
                if is_playmaker {
                    // Playmakers prioritize tactical value and vision
                    (evaluation.expected_value * 1.3 + positioning_bonus * 0.4)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else if is_direct {
                    // Direct players maximize attack
                    (evaluation.expected_value * 1.4 + evaluation.factors.tactical_value * 0.5)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else if is_team_player {
                    // Team players maximize receiver's situation
                    (evaluation.success_probability + positioning_bonus * 0.8)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else if is_conservative {
                    // Conservative: success probability is key
                    (evaluation.success_probability * 1.5 + positioning_bonus * 0.3)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else if is_pragmatic {
                    // Pragmatic: balanced approach
                    (evaluation.expected_value * 1.2 + positioning_bonus * 0.5)
                        * interception_penalty
                        * space_quality
                        * distance_preference
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                } else {
                    // Standard scoring
                    (evaluation.expected_value + positioning_bonus * 0.5)
                        * interception_penalty
                        * space_quality
                        * optimal_distance_bonus
                        * goalkeeper_penalty
                }
            };

            // Hard reject: never pass through 2+ opponents unless playmaker with high vision
            let interception_blocked = if interception_risk >= 0.85 {
                // 2+ opponents in the lane — almost always reject
                if is_playmaker && vision_skill > 0.8 {
                    false // Elite playmakers can attempt
                } else {
                    true
                }
            } else if interception_risk >= 0.55 {
                // 1 opponent in the lane — reject for conservative, allow others with caution
                is_conservative
            } else {
                false
            };

            // Personality-based acceptance threshold - more aggressive to encourage penetration
            let is_acceptable = if interception_blocked {
                false
            } else if is_goalkeeper {
                // Goalkeeper passes are normally rare; in build-up
                // they're a textbook pattern (recycle through the GK to
                // bait a press, then switch). Phase gates this:
                //   * BuildUp: allow when in own defensive third with a
                //     reasonable success probability and either pressure
                //     or low risk_appetite (recycle intent).
                //   * Otherwise: only as an emergency escape from
                //     extreme pressure deep in own half.
                let side_now = ctx.player.side.unwrap_or(PlayerSide::Left);
                let fw = ctx.context.field_size.width as f32;
                let progress = side_now.attacking_progress_x(ctx.player.position.x, fw);
                let in_defensive_third = progress < 0.33;
                let phase_now = ctx.team().phase();

                if matches!(phase_now, GamePhase::BuildUp) && in_defensive_third {
                    let under_press = ctx
                        .player()
                        .pressure()
                        .is_under_immediate_pressure_with_distance(8.0);
                    let recycle_intent = ctx.team().risk_appetite() < 0.45;
                    evaluation.success_probability > 0.55
                        && (under_press || recycle_intent)
                } else {
                    evaluation.factors.pressure_factor < 0.2
                        && evaluation.success_probability > 0.85
                        && in_defensive_third
                }
            } else if is_conservative {
                evaluation.success_probability > 0.60
                    && evaluation.factors.receiver_positioning > 0.55
            } else if is_direct {
                evaluation.success_probability > 0.40 && evaluation.factors.tactical_value > 0.35
            } else if is_playmaker {
                evaluation.success_probability > 0.45
                    || (evaluation.factors.tactical_value > 0.60 && pass_distance > 50.0)
            } else {
                // Standard - more willing to pass
                evaluation.is_recommended
                    || (evaluation.factors.receiver_positioning > 0.5
                        && evaluation.success_probability > 0.42)
            };

            // Game-management bias: a team protecting a lead (especially
            // late, or as the weaker side) prefers sideways / backward
            // balls over risky forward ones — real "hold the score"
            // football.
            let gm_intensity = ctx
                .context
                .tactical_for_team(ctx.player.team_id)
                .game_management_intensity;
            let gm_modifier = if gm_intensity > 0.05 {
                let side_now = ctx.player.side.unwrap_or(PlayerSide::Left);
                let forward_progress =
                    side_now.forward_delta(ctx.player.position.x, teammate.position.x);
                if forward_progress > 5.0 {
                    (1.0 - gm_intensity * 0.45).max(0.3)
                } else {
                    1.0 + gm_intensity * 0.60
                }
            } else {
                1.0
            };

            // Apply graduated recency penalty to discourage ping-pong passing
            // Apply congestion penalty to force ball out of huddles
            let score = score * recency_penalty * congestion_penalty * gm_modifier;

            if score > best_score && is_acceptable {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        // Minimum score threshold: if the best option scores too low,
        // return None so the player dribbles/runs instead of making a bad pass
        const MIN_PASS_SCORE: f32 = 0.15;
        if best_score < MIN_PASS_SCORE {
            return None;
        }

        best_option.map(|teammate| (teammate, "PASS_EVALUATOR"))
    }

    // ──────────────────────────────────────────────────────────────────
    // Pure helpers — pulled out of `calculate_tactical_value` so they
    // can be unit-tested without spinning up a full match field.
    // ──────────────────────────────────────────────────────────────────

    /// Penalty for passing into a flank that already has too many of
    /// our own players. Polish-spec curve: 0..3 → 0, 4 → -0.08,
    /// 5 → -0.18, 6+ → -0.30. The legacy half-pitch overload penalty
    /// has been removed so this is the single side-overload signal.
    pub fn same_side_density_penalty(receiver_side_density: u8) -> f32 {
        match receiver_side_density {
            0..=3 => 0.0,
            4 => -0.08,
            5 => -0.18,
            _ => -0.30,
        }
    }

    /// Bonus for switching the play into an underloaded flank.
    /// Vision-graded so playmakers see the switch as more valuable.
    /// Polish-spec curve: 0.08 + vision * 0.12 → 0.08..0.20.
    /// Returns 0 when the pass doesn't cross flanks or the target side
    /// is not underloaded.
    pub fn underload_switch_bonus(
        crosses_sides: bool,
        receiver_side_density: u8,
        vision: f32,
    ) -> f32 {
        let underloaded = receiver_side_density <= 3;
        if crosses_sides && underloaded {
            let vision = vision.clamp(0.0, 1.0);
            0.08 + vision * 0.12
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PassEvaluator;

    #[test]
    fn density_penalty_zero_when_uncrowded() {
        assert_eq!(PassEvaluator::same_side_density_penalty(0), 0.0);
        assert_eq!(PassEvaluator::same_side_density_penalty(3), 0.0);
    }

    #[test]
    fn density_penalty_increases_with_crowding() {
        assert!(
            PassEvaluator::same_side_density_penalty(4)
                > PassEvaluator::same_side_density_penalty(5)
        );
        assert!(
            PassEvaluator::same_side_density_penalty(5)
                > PassEvaluator::same_side_density_penalty(7)
        );
        assert_eq!(PassEvaluator::same_side_density_penalty(7), -0.30);
    }

    #[test]
    fn underload_switch_zero_when_not_crossing() {
        assert_eq!(PassEvaluator::underload_switch_bonus(false, 0, 1.0), 0.0);
    }

    #[test]
    fn underload_switch_zero_when_target_already_full() {
        // 5 players on the receiver side — not underloaded, no bonus.
        assert_eq!(PassEvaluator::underload_switch_bonus(true, 5, 1.0), 0.0);
    }

    #[test]
    fn underload_switch_grows_with_vision() {
        let low = PassEvaluator::underload_switch_bonus(true, 2, 0.0);
        let high = PassEvaluator::underload_switch_bonus(true, 2, 1.0);
        assert!(high > low);
        assert!((low - 0.08).abs() < 1e-4);
        assert!((high - 0.20).abs() < 1e-4);
    }

    #[test]
    fn underload_switch_bonus_within_spec_range() {
        // Spec range: 0.08 + vision*0.12 → 0.08..0.20
        for v_int in 0..=20 {
            let v = v_int as f32 / 20.0;
            let bonus = PassEvaluator::underload_switch_bonus(true, 1, v);
            assert!(bonus >= 0.08 - 1e-4);
            assert!(bonus <= 0.20 + 1e-4);
        }
    }

    #[test]
    fn density_penalty_curve_matches_polish_spec() {
        assert_eq!(PassEvaluator::same_side_density_penalty(4), -0.08);
        assert_eq!(PassEvaluator::same_side_density_penalty(5), -0.18);
        assert_eq!(PassEvaluator::same_side_density_penalty(6), -0.30);
        assert_eq!(PassEvaluator::same_side_density_penalty(11), -0.30);
    }

    #[test]
    fn switch_total_caps_at_zero_point_four_five() {
        // The capped switch reward path inside `calculate_tactical_value`
        // is `(classic + underload).min(0.45)`. Verify the helpers feed a
        // sensible joint maximum: classic max is 0.45 + vision*0.25 = 0.70,
        // underload max is 0.20. The cap therefore truly bites.
        let underload_max =
            PassEvaluator::underload_switch_bonus(true, 0, 1.0);
        assert!(
            underload_max + 0.70 > 0.45,
            "cap must actually bite — sum without cap = {}",
            underload_max + 0.70
        );
    }
}
