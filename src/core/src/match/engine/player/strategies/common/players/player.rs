use crate::r#match::player::strategies::players::{
    DefensiveOperationsImpl, MovementOperationsImpl, PassingOperationsImpl, PressureOperationsImpl,
    ShootingOperationsImpl, SkillOperationsImpl,
};
use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    MatchPlayer, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateProcessingContext,
};
use crate::{PlayerAttributes, PlayerSkills};
use nalgebra::Vector3;
use rand::RngExt;
use std::sync::OnceLock;

/// Zero-valued fallback returned by [`PlayerOperationsImpl::skills`] /
/// [`PlayerOperationsImpl::attributes`] when the requested `player_id`
/// is no longer present in `MatchContext::players` — typically because
/// the player was substituted off but a stale id is still being chased
/// by another AI strategy's spatial-grid query or remembered target.
/// Using a default instead of panicking keeps the simulation running;
/// downstream calculations against a zero-skilled player produce a
/// degraded-but-bounded result (e.g. tackle skill 0/20 = 0.0).
fn fallback_skills() -> &'static PlayerSkills {
    static FALLBACK: OnceLock<PlayerSkills> = OnceLock::new();
    FALLBACK.get_or_init(PlayerSkills::default)
}

fn fallback_attributes() -> &'static PlayerAttributes {
    static FALLBACK: OnceLock<PlayerAttributes> = OnceLock::new();
    FALLBACK.get_or_init(PlayerAttributes::default)
}

pub struct PlayerOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PlayerOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PlayerOperationsImpl { ctx }
    }
}

impl<'p> PlayerOperationsImpl<'p> {
    pub fn get(&self, player_id: u32) -> MatchPlayerLite {
        // Stale id from a subbed-off opponent or a grid race — fall back
        // to a sensible neutral position so callers don't crash. They
        // still get a valid lite handle; spatial calculations using its
        // tactical_position will pick the goalkeeper default but that's
        // bounded behaviour, not a hard panic mid-tick.
        let tactical_positions = self
            .ctx
            .context
            .players
            .by_id(player_id)
            .map(|p| p.tactical_position.current_position)
            .unwrap_or_else(|| {
                log::debug!(
                    "get() lookup for unknown player_id={} — using default position",
                    player_id
                );
                crate::PlayerPositionType::Goalkeeper
            });
        MatchPlayerLite {
            id: player_id,
            position: self.ctx.tick_context.positions.players.position(player_id),
            tactical_positions,
        }
    }

    /// Returns this player's skills. If `player_id` is no longer in the
    /// match context (subbed off, stale grid hit, AI memory pointing at a
    /// gone player), returns a static zero-valued default instead of
    /// panicking. See [`fallback_skills`].
    pub fn skills(&self, player_id: u32) -> &PlayerSkills {
        match self.ctx.context.players.by_id(player_id) {
            Some(player) => &player.skills,
            None => {
                log::debug!(
                    "skills() lookup for unknown player_id={} — falling back to default",
                    player_id
                );
                fallback_skills()
            }
        }
    }

    /// Returns this player's attributes. If `player_id` is no longer in
    /// the match context, returns a static default instead of panicking.
    /// See [`fallback_attributes`].
    pub fn attributes(&self, player_id: u32) -> &PlayerAttributes {
        match self.ctx.context.players.by_id(player_id) {
            Some(player) => &player.player_attributes,
            None => {
                log::debug!(
                    "attributes() lookup for unknown player_id={} — falling back to default",
                    player_id
                );
                fallback_attributes()
            }
        }
    }

    pub fn on_own_side(&self) -> bool {
        let field_half_width = self.ctx.context.field_size.width / 2;

        self.ctx.player.side == Some(PlayerSide::Left)
            && self.ctx.player.position.x < field_half_width as f32
    }

    /// Clearing direction for defenders: aims AWAY from own goal with moderate randomness.
    /// Unlike shooting_direction(), this doesn't target a specific goal - it just clears danger.
    pub fn clearing_direction(&self) -> Vector3<f32> {
        let own_goal = match self.ctx.player.side {
            Some(PlayerSide::Left) => self.ctx.context.goal_positions.left,
            Some(PlayerSide::Right) => self.ctx.context.goal_positions.right,
            _ => self.ctx.player.position,
        };

        // Clear AWAY from own goal — direction from own goal through player position
        let away_from_goal = (self.ctx.player.position - own_goal).normalize();

        // Add moderate lateral randomness for realistic clearances (not laser-guided)
        let heading_skill = (self.ctx.player.skills.technical.heading - 1.0) / 19.0;
        let randomness = (1.0 - heading_skill) * 0.4; // 0.0-0.4 based on skill

        let mut rng = rand::rng();
        let lateral_offset = rng.random_range(-randomness..randomness);

        // Perpendicular direction for lateral spread
        let perp = Vector3::new(-away_from_goal.y, away_from_goal.x, 0.0);

        let direction = (away_from_goal + perp * lateral_offset).normalize();

        // Target point far in the clearing direction
        self.ctx.player.position + direction * 200.0
    }

    pub fn shooting_direction(&self) -> Vector3<f32> {
        let goal_position = self.opponent_goal_position();
        let distance_to_goal = self.goal_distance();

        let skills = &self.ctx.player.skills;

        // Normalize skills (0.0 to 1.0)
        let finishing_f = (skills.technical.finishing - 1.0) / 19.0;
        let technique_f = (skills.technical.technique - 1.0) / 19.0;
        let first_touch_f = (skills.technical.first_touch - 1.0) / 19.0;
        let long_shots_f = (skills.technical.long_shots - 1.0) / 19.0;
        let composure_f = (skills.mental.composure - 1.0) / 19.0;

        // Core shot accuracy: finishing and technique are dominant
        // Blend finishing vs long_shots based on distance
        let max_field_distance = self.ctx.context.field_size.width as f32;
        let distance_blend = (distance_to_goal / (max_field_distance * 0.3)).clamp(0.0, 1.0);
        let shot_skill = finishing_f * (1.0 - distance_blend) + long_shots_f * distance_blend;

        let base_accuracy =
            shot_skill * 0.45 + technique_f * 0.25 + first_touch_f * 0.15 + composure_f * 0.15;

        // Distance modifier: closer = more accurate (multiplicative, not part of accuracy blend)
        let distance_modifier = if distance_to_goal < 100.0 {
            1.0
        } else if distance_to_goal < 200.0 {
            1.0 - (distance_to_goal - 100.0) / 400.0 // 1.0 → 0.75
        } else {
            0.6 + long_shots_f * 0.2 // Long shots: 0.6-0.8 based on skill
        };

        // Pressure modifier
        let nearby_defenders = self.ctx.players().opponents().nearby(10.0).count();
        let pressure_modifier = 1.0 - (nearby_defenders as f32 * 0.12).min(0.4);

        // Condition modifier: slight accuracy loss when exhausted
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_modifier = 0.93 + condition * 0.07;

        // Final accuracy (0.0 to ~1.0)
        let accuracy = (base_accuracy * distance_modifier * pressure_modifier * condition_modifier)
            .clamp(0.0, 1.0);

        // Inaccuracy factor: even good players miss sometimes
        // Base inaccuracy + distance penalty means long shots are genuinely difficult
        let base_inaccuracy = (1.0 - accuracy) * (1.0 - accuracy);
        let distance_inaccuracy = if distance_to_goal > 80.0 {
            0.15 // significant extra miss chance for long shots
        } else if distance_to_goal > 40.0 {
            0.06 // moderate extra miss for medium range
        } else {
            0.0
        };
        let inaccuracy = (base_inaccuracy + distance_inaccuracy).min(1.0);

        let goal_width = 58.0; // matches GOAL_WIDTH * 2 (29 half-width)
        let mut rng = rand::rng();

        // Placement shot: skilled finishers pick corners from close range
        let is_placement_shot = distance_to_goal < 150.0 && finishing_f > 0.55;

        let mut target = goal_position;

        if is_placement_shot {
            // Close range: aim for a corner
            let y_target = if rng.random_range(0.0..1.0) < 0.5 {
                -goal_width * 0.35
            } else {
                goal_width * 0.35
            };

            // Better finishing = tighter grouping around intended corner
            let y_deviation = rng.random_range(-goal_width * 0.2..goal_width * 0.2) * inaccuracy;
            target.y += y_target + y_deviation;
        } else {
            // Long range / low skill: aim more central with wider spread
            let y_base = rng.random_range(-goal_width * 0.1..goal_width * 0.1);
            let y_deviation = rng.random_range(-goal_width * 0.4..goal_width * 0.4) * inaccuracy;
            target.y += y_base + y_deviation;
        }

        // Technique affects clean contact — poor technique sprays the ball
        let x_deviation = rng.random_range(-5.0..5.0) * inaccuracy;
        target.x += x_deviation;

        // Composure under pressure: defenders nearby cause panic deviation
        if nearby_defenders > 0 {
            let panic = (1.0 - composure_f) * (1.0 - composure_f);
            let panic_y = rng.random_range(-goal_width * 0.12..goal_width * 0.12) * panic;
            let panic_x = rng.random_range(-6.0..6.0) * panic;
            target.y += panic_y;
            target.x += panic_x;
        }

        target
    }

    pub fn opponent_goal_position(&self) -> Vector3<f32> {
        match self.ctx.player.side {
            Some(PlayerSide::Left) => self.ctx.context.goal_positions.right,
            Some(PlayerSide::Right) => self.ctx.context.goal_positions.left,
            _ => Vector3::new(0.0, 0.0, 0.0),
        }
    }

    pub fn distance_from_start_position(&self) -> f32 {
        self.ctx
            .player
            .start_position
            .distance_to(&self.ctx.player.position)
    }

    pub fn position_to_distance(&self) -> PlayerDistanceFromStartPosition {
        MatchPlayerLogic::distance_to_start_position(self.ctx.player)
    }

    pub fn is_tired(&self) -> bool {
        self.ctx.player.player_attributes.condition_percentage() > 50
    }

    pub fn pass_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self
            .ctx
            .tick_context
            .grid
            .get(self.ctx.player.id, teammate_id);

        let skills = &self.ctx.player.skills;

        // Technical: passing for weight, technique for clean contact
        let passing = skills.technical.passing / 20.0;
        let technique = skills.technical.technique / 20.0;
        // Physical: strength for raw power capability
        let strength = skills.physical.strength / 20.0;
        // Mental: vision for pass weight judgement, composure for consistency
        let vision = skills.mental.vision / 20.0;
        let composure = skills.mental.composure / 20.0;

        let skill_factor =
            passing * 0.35 + technique * 0.2 + strength * 0.15 + vision * 0.15 + composure * 0.15;

        // Condition: slight power loss when exhausted (0-10000 scale)
        // Ranges from 0.92 (exhausted) to 1.0 (fresh)
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_factor = 0.92 + condition * 0.08;

        // Distance scaling: pass power proportional to distance needed
        let max_pass_distance = self.ctx.context.field_size.width as f32 * 0.8;
        let distance_factor = if distance < 20.0 {
            // Short passes: minimum floor so ball visibly travels
            (0.15 + (distance / 20.0) * 0.1).clamp(0.15, 0.25)
        } else {
            (distance / max_pass_distance).clamp(0.25, 1.0)
        };

        let min_power = 0.5;
        let max_power = 2.0;
        let base_power = min_power + (max_power - min_power) * skill_factor * distance_factor;

        base_power * condition_factor
    }

    pub fn kick_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self
            .ctx
            .tick_context
            .grid
            .get(self.ctx.player.id, teammate_id);

        let kick_skill = self.ctx.player.skills.technical.free_kicks / 20.0;

        let raw_power = distance / (kick_skill * 100.0);

        let min_power = 0.1;
        let max_power = 1.0;
        let normalized_power = (raw_power - min_power) / (max_power - min_power);

        normalized_power.clamp(0.0, 1.0)
    }

    pub fn throw_teammate_power(&self, teammate_id: u32) -> f32 {
        let distance = self
            .ctx
            .tick_context
            .grid
            .get(self.ctx.player.id, teammate_id);

        let throw_skill = self.ctx.player.skills.technical.long_throws / 20.0;

        let raw_power = distance / (throw_skill * 100.0);

        let min_power = 0.1;
        let max_power = 1.0;
        let normalized_power = (raw_power - min_power) / (max_power - min_power);

        normalized_power.clamp(0.0, 1.0)
    }

    pub fn shoot_goal_power(&self) -> f64 {
        let goal_distance = self.goal_distance();

        let skills = &self.ctx.player.skills;

        // Technical skills
        let technique = skills.technical.technique / 20.0;
        let long_shots = skills.technical.long_shots / 20.0;
        let finishing = skills.technical.finishing / 20.0;
        // Physical
        let strength = skills.physical.strength / 20.0;
        // Mental: composure under pressure
        let composure = skills.mental.composure / 20.0;

        // Blend finishing (close) vs long_shots (far) based on distance
        let max_field_distance = self.ctx.context.field_size.width as f32;
        let distance_blend = (goal_distance / (max_field_distance * 0.3)).clamp(0.0, 1.0);
        let shot_skill = finishing * (1.0 - distance_blend) + long_shots * distance_blend;

        // Skill multiplier with floor so even low-skill players generate some power
        let skill_multiplier =
            0.2 + 0.8 * (shot_skill * 0.3 + technique * 0.25 + strength * 0.25 + composure * 0.2);

        // Distance factor: longer shots need more power (1.0 close, up to 1.4 far)
        let distance_ratio = (goal_distance / max_field_distance).clamp(0.0, 1.0);
        let distance_factor = 1.0 + distance_ratio * 0.4;

        // Condition: slight power loss when exhausted (0.90 exhausted to 1.0 fresh)
        let condition = self.ctx.player.player_attributes.condition as f32 / 10000.0;
        let condition_factor = 0.90 + condition * 0.10;

        let base_power = 2.2;
        let shooting_power = base_power * skill_multiplier * distance_factor * condition_factor;

        shooting_power.clamp(1.4, 3.8) as f64
    }

    pub fn distance_to_player(&self, player_id: u32) -> f32 {
        self.ctx
            .tick_context
            .grid
            .get(self.ctx.player.id, player_id)
    }

    pub fn goal_angle(&self) -> f32 {
        // Calculate the angle between the player's facing direction and the goal direction
        let player_direction = self.ctx.player.velocity.normalize();
        let goal_direction = (self.goal_position() - self.ctx.player.position).normalize();
        player_direction.angle(&goal_direction)
    }

    pub fn goal_distance(&self) -> f32 {
        let player_position = self.ctx.player.position;
        let goal_position = self.goal_position();
        (player_position - goal_position).magnitude()
    }

    pub fn goal_position(&self) -> Vector3<f32> {
        let field_width = self.ctx.context.field_size.width as f32;
        let field_height = self.ctx.context.field_size.height as f32;

        if self.ctx.player.side == Some(PlayerSide::Left) {
            Vector3::new(field_width, field_height / 2.0, 0.0)
        } else {
            Vector3::new(0.0, field_height / 2.0, 0.0)
        }
    }

    pub fn has_clear_pass(&self, player_id: u32) -> bool {
        let player_position = self.ctx.player.position;
        let target_player_position = self.ctx.tick_context.positions.players.position(player_id);
        let direction_to_player = (target_player_position - player_position).normalize();

        let distance_to_player = self.distance_to_player(player_id);

        let ray_cast_result = self.ctx.tick_context.space.cast_ray(
            player_position,
            direction_to_player,
            distance_to_player,
            false,
        );

        ray_cast_result.is_none()
    }

    /// Can this player physically strike the ball right now? Gated by
    /// the player's per-shot cooldown in `PlayerMemory::can_shoot` — a
    /// forward who just shot needs ~1.5 s to reset before striking
    /// again. Consulted by the running/dribbling shot-trigger paths so
    /// the same striker can't fire a shot every AI tick while camped
    /// at the post.
    pub fn can_shoot(&self) -> bool {
        self.ctx
            .player
            .memory
            .can_shoot(self.ctx.context.current_tick())
    }

    pub fn has_clear_shot(&self) -> bool {
        let player_position = self.ctx.player.position;
        let goal_position = self.opponent_goal_position();
        let direction_to_goal = (goal_position - player_position).normalize();
        let distance_to_goal = self.goal_distance();

        // SHOOTING-ANGLE GATE. Real football: the angle the goal
        // subtends from the shooter's position is the single best
        // predictor of xG. From the edge of the 18-yard box at a
        // steep diagonal, the visible goal opening is a few metres
        // wide; forwards in that spot square the ball or cross,
        // they don't shoot. Computed via the half-angle at which the
        // shooter sees the goal: atan(goal_half_width / distance).
        // Under 12° → pass or cross, not shoot. Matches real shot
        // location data — almost no shots from Y>30m wide at <18m
        // from the byline.
        const GOAL_HALF_WIDTH: f32 = 29.0;
        let player_y = player_position.y;
        let goal_y = goal_position.y;
        let lateral_offset = (player_y - goal_y).abs();
        let x_offset = distance_to_goal.max(1.0);
        // Effective opening: if the shooter is lateral, the goal
        // opening they see shrinks. Use the narrower side.
        let near_post_offset = (lateral_offset - GOAL_HALF_WIDTH).max(0.0);
        let far_post_offset = lateral_offset + GOAL_HALF_WIDTH;
        let visible_opening =
            (far_post_offset.atan2(x_offset) - near_post_offset.atan2(x_offset)).abs();
        const MIN_SHOOTING_ANGLE_RAD: f32 = 0.21; // ~12°
        if visible_opening < MIN_SHOOTING_ANGLE_RAD {
            return false;
        }

        // Close-pressure check: a defender within 5u of the shooter
        // closes the shooting angle regardless of where they stand
        // relative to the direct line. 5u (~2.5 m) matches "defender
        // breathing on you, but shot still possible"; 8u was
        // shoulder-to-shoulder and rejected nearly every box shot.
        let immediate_pressure = self
            .ctx
            .players()
            .opponents()
            .nearby(5.0)
            .any(|opp| !opp.tactical_positions.is_goalkeeper());
        if immediate_pressure {
            return false;
        }

        // Only check outfield defenders — the GK is handled by save
        // mechanics. Skip the last 20% of distance to goal.
        let check_distance = distance_to_goal * 0.80;

        // Count blockers along the shot path.
        let blockers = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                if opp.tactical_positions.is_goalkeeper() {
                    return false;
                }

                let to_opp = opp.position - player_position;
                let projection = to_opp.x * direction_to_goal.x + to_opp.y * direction_to_goal.y;

                if projection < 5.0 || projection > check_distance {
                    return false;
                }

                let closest_point = player_position + direction_to_goal * projection;
                let perp_distance = ((opp.position.x - closest_point.x).powi(2)
                    + (opp.position.y - closest_point.y).powi(2))
                .sqrt();

                let opp_skills = self.skills(opp.id);
                let def_quality = (opp_skills.technical.marking
                    + opp_skills.technical.tackling
                    + opp_skills.mental.positioning)
                    / 60.0; // 0..1
                let corridor_half_width = 4.0 + def_quality * 6.0; // 4..10

                perp_distance < corridor_half_width
            })
            .count();

        // DEFENDER-DENSITY GATE. Zero blockers = clearly clear. One
        // blocker = contested — average forwards (fin ~9+) can still
        // shoot through. Two or more = organised defence, needs a
        // quality striker (fin ~14+). Real football defences pack the
        // box but strikers still get shots away; the earlier 0.75 / 0.90
        // thresholds meant only fin-15+ / fin-18+ forwards ever shot in
        // traffic, which collapsed total shot volume.
        match blockers {
            0 => true,
            1 => {
                let finishing = self.ctx.player.skills.technical.finishing / 20.0;
                finishing >= 0.45
            }
            _ => {
                let finishing = self.ctx.player.skills.technical.finishing / 20.0;
                finishing >= 0.70
            }
        }
    }

    pub fn separation_velocity(&self) -> Vector3<f32> {
        // Separation parameters. Boosted from the previous 20-radius,
        // 20-strength, cubic-falloff setup — that configuration meant
        // the force was near-zero beyond ~15u and states could pull
        // teammates into the same yard without resistance. The widened
        // radius + linear falloff + higher strength + higher cap
        // produces a force teammates ACTUALLY respond to, so
        // formations stay spread even when multiple state targets
        // converge on the same ball area.
        const SEPARATION_RADIUS: f32 = 40.0;
        const OPP_SEPARATION_RADIUS: f32 = SEPARATION_RADIUS * 0.8; // 32.0
        const SEPARATION_STRENGTH: f32 = 45.0;
        const MIN_SEPARATION_DISTANCE: f32 = 5.0;
        const MAX_SEPARATION_FORCE: f32 = 40.0;

        // Early exit: check if anyone is nearby before iterating
        let players = self.ctx.players();
        let teammates = players.teammates();
        let opponents = players.opponents();

        if !teammates.exists(SEPARATION_RADIUS) && !opponents.exists(OPP_SEPARATION_RADIUS) {
            return Vector3::zeros();
        }

        let mut separation = Vector3::zeros();
        let player_pos = self.ctx.player.position;

        // Apply separation from teammates. Quadratic falloff rather
        // than cubic — at d=20 (half of radius) strength is 11.25 vs
        // 1.4 under cubic. Keeps the force meaningful across the whole
        // "could bunch here" band, not just at the very-close edge.
        for other_player in teammates.nearby(SEPARATION_RADIUS) {
            let to_other = other_player.position - player_pos;
            let distance = to_other.magnitude();

            if distance > 0.0 {
                let inv_dist = 1.0 / distance;
                let direction = -to_other * inv_dist;
                let t = 1.0 - distance / SEPARATION_RADIUS;
                let strength = SEPARATION_STRENGTH * t * t; // quadratic
                separation += direction * strength;

                if distance < MIN_SEPARATION_DISTANCE {
                    let emergency_multiplier = (MIN_SEPARATION_DISTANCE * inv_dist).min(1.5);
                    separation += direction * SEPARATION_STRENGTH * emergency_multiplier * 0.5;
                }
            }
        }

        // Apply separation from opponents — but only if we DON'T have
        // the ball. A ball carrier must face defenders, not flee via
        // separation force. With the boosted strength, opponent
        // separation was pushing the dribbler sideways around any
        // chaser — causing the "player running around" circular
        // motion the user saw. Ball carriers get zero opponent
        // separation so state velocity (aim at goal / find a pass)
        // controls their movement cleanly.
        let i_have_ball = self.ctx.ball().owner_id() == Some(self.ctx.player.id);
        if !i_have_ball {
            for other_player in opponents.nearby(OPP_SEPARATION_RADIUS) {
                let to_other = other_player.position - player_pos;
                let distance = to_other.magnitude();

                if distance > 0.0 {
                    let inv_dist = 1.0 / distance;
                    let direction = -to_other * inv_dist;
                    let t = 1.0 - distance / OPP_SEPARATION_RADIUS;
                    let strength = SEPARATION_STRENGTH * 0.8 * t * t * t;
                    separation += direction * strength;

                    if distance < MIN_SEPARATION_DISTANCE {
                        let emergency_multiplier = (MIN_SEPARATION_DISTANCE * inv_dist).min(1.5);
                        separation += direction * SEPARATION_STRENGTH * 0.4 * emergency_multiplier;
                    }
                }
            }
        }

        // Clamp separation force
        let separation_magnitude_sq = separation.magnitude_squared();
        if separation_magnitude_sq > MAX_SEPARATION_FORCE * MAX_SEPARATION_FORCE {
            separation *= MAX_SEPARATION_FORCE / separation_magnitude_sq.sqrt();
        }

        separation
    }

    /// Get pressure operations for assessing game pressure
    pub fn pressure(&self) -> PressureOperationsImpl<'p> {
        PressureOperationsImpl::new(self.ctx)
    }

    /// Get shooting operations for shooting decisions
    pub fn shooting(&self) -> ShootingOperationsImpl<'p> {
        ShootingOperationsImpl::new(self.ctx)
    }

    /// Get passing operations for passing decisions
    pub fn passing(&self) -> PassingOperationsImpl<'p> {
        PassingOperationsImpl::new(self.ctx)
    }

    /// Get defensive operations for defensive positioning
    pub fn defensive(&self) -> DefensiveOperationsImpl<'p> {
        DefensiveOperationsImpl::new(self.ctx)
    }

    /// Get movement operations for space-finding and positioning
    pub fn movement(&self) -> MovementOperationsImpl<'p> {
        MovementOperationsImpl::new(self.ctx)
    }

    /// Get skill operations for skill-based calculations
    pub fn skill(&self) -> SkillOperationsImpl<'p> {
        SkillOperationsImpl::new(self.ctx)
    }

    /// Check if the player should attempt a shot based on shooting range
    pub fn should_attempt_shot(&self) -> bool {
        self.shooting().in_shooting_range()
    }
}

pub struct MatchPlayerLogic;

impl MatchPlayerLogic {
    pub fn distance_to_start_position(player: &MatchPlayer) -> PlayerDistanceFromStartPosition {
        let start_position_distance = player.position.distance_to(&player.start_position);

        if start_position_distance < 100.0 {
            PlayerDistanceFromStartPosition::Small
        } else if start_position_distance < 250.0 {
            PlayerDistanceFromStartPosition::Medium
        } else {
            PlayerDistanceFromStartPosition::Big
        }
    }
}
