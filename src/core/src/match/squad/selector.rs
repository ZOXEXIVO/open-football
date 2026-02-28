use crate::club::team::coach_perception::CoachProfile;
use crate::club::{PlayerFieldPositionGroup, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{Player, PlayerStatusType, Tactics, Team};
use chrono::NaiveDate;
use log::{debug, warn};
use std::borrow::Borrow;

pub struct SquadSelector;

const DEFAULT_SQUAD_SIZE: usize = 11;
const DEFAULT_BENCH_SIZE: usize = 7;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
}

// ========== SELECTION CONTEXT ==========

pub struct SelectionContext {
    pub is_friendly: bool,
    pub date: NaiveDate,
}

impl Default for SelectionContext {
    fn default() -> Self {
        SelectionContext {
            is_friendly: false,
            date: chrono::Utc::now().date_naive(),
        }
    }
}

// ========== SCORING ENGINE ==========

struct ScoringEngine {
    profile: CoachProfile,
}

impl ScoringEngine {
    fn from_staff(staff: &Staff) -> Self {
        ScoringEngine {
            profile: CoachProfile::from_staff(staff),
        }
    }

    /// Lens-weighted skill composite using the coach's perception lens
    fn perceived_quality(&self, player: &Player) -> f32 {
        let lens = &self.profile.perception_lens;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;

        // Technical composite
        let atk_tech =
            (t.finishing + t.dribbling + t.crossing + t.first_touch + t.technique + t.long_shots)
                / 6.0;
        let def_tech = (t.tackling + t.marking + t.heading + t.passing) / 4.0;
        let tech_score =
            atk_tech * lens.attacking_focus + def_tech * (1.0 - lens.attacking_focus);

        // Mental composite
        let creative_mental =
            (m.flair + m.vision + m.composure + m.decisions + m.anticipation) / 5.0;
        let discipline_mental =
            (m.work_rate + m.determination + m.positioning + m.teamwork + m.concentration) / 5.0;
        let mental_score = creative_mental * lens.creativity_focus
            + discipline_mental * (1.0 - lens.creativity_focus);

        // Physical composite
        let explosive = (p.pace + p.acceleration + p.strength + p.jumping) / 4.0;
        let endurance = (p.stamina + p.natural_fitness + p.agility + p.balance) / 4.0;
        let physical_score =
            explosive * lens.physicality_focus + endurance * (1.0 - lens.physicality_focus);

        let skill_composite = tech_score * lens.technical_weight
            + mental_score * lens.mental_weight
            + physical_score * lens.physical_weight;

        // Position mastery dampened by tactical blindness
        let position_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;
        let position_contribution =
            position_level * (1.0 - self.profile.tactical_blindness * 0.5);

        let base = skill_composite * 0.75 + position_contribution * 0.25;

        // Form bonus amplified by recency_bias
        let raw_form_bonus =
            if player.statistics.played + player.statistics.played_subs > 3 {
                (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
            } else {
                0.0
            };
        let form_bonus = raw_form_bonus * (1.0 + self.profile.recency_bias * 0.8);

        // Attitude bleed
        let attitude_bleed = {
            let visible_effort =
                (player.skills.mental.work_rate + player.skills.mental.determination) / 2.0;
            (visible_effort - 10.0) * self.profile.attitude_weight * 0.15
        };

        // Condition factor
        let condition =
            (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.5, 1.0);

        (base + form_bonus + attitude_bleed) * condition
    }

    /// Match readiness: condition + fitness + sharpness + physical_readiness
    fn match_readiness(&self, player: &Player) -> f32 {
        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let fitness = player.player_attributes.fitness as f32 / 10000.0;

        let days_since = player.player_attributes.days_since_last_match as f32;
        let sharpness = if days_since <= 3.0 {
            1.0
        } else if days_since <= 7.0 {
            0.95
        } else if days_since <= 14.0 {
            0.85
        } else if days_since <= 28.0 {
            0.70
        } else {
            0.55
        };

        let physical_readiness = player.skills.physical.match_readiness / 20.0;

        let raw_readiness = (condition * 0.35
            + fitness.clamp(0.0, 1.0) * 0.25
            + sharpness * 0.25
            + physical_readiness * 0.15)
            .clamp(0.0, 1.0);

        // Noise scaled by readiness_intuition
        let noise_scale = (1.0 - self.profile.readiness_intuition) * 0.25;
        let noise = self.profile.perception_noise(player.id, 0xFE57) * noise_scale;

        (raw_readiness + noise).clamp(0.0, 1.0) * 20.0
    }

    /// Training impression: professionalism + determination + work_rate
    fn training_impression(&self, player: &Player) -> f32 {
        let visible_effort = (player.skills.mental.work_rate
            + player.skills.mental.determination
            + player.skills.mental.teamwork)
            / 3.0;

        // Attitude-sensitive coaches overweight visible effort
        visible_effort * (0.5 + self.profile.attitude_weight * 0.5)
    }

    // ========== REPUTATION & RELATIONSHIP SCORING ==========

    /// Player status/reputation score.
    ///
    /// Real-world effect: star players are harder to drop. A coach needs a reason
    /// to bench someone with 60 international caps. Less accurate coaches are
    /// more swayed by big names; analytical coaches look past the hype.
    ///
    /// Returns 0..~2.5 (world-class star with susceptible coach).
    fn reputation_score(&self, player: &Player) -> f32 {
        let p = &self.profile;

        // Current reputation: 0-3000 → 0..1
        let current_rep =
            (player.player_attributes.current_reputation as f32 / 3000.0).clamp(0.0, 1.0);

        // World reputation: 0-1200 → 0..1
        let world_rep =
            (player.player_attributes.world_reputation as f32 / 1200.0).clamp(0.0, 1.0);

        // International experience: 50+ caps = fully established
        let intl_factor =
            (player.player_attributes.international_apps as f32 / 50.0).clamp(0.0, 1.0);

        // Combine: league standing, global profile, international pedigree
        let raw_rep = current_rep * 0.4 + world_rep * 0.4 + intl_factor * 0.2;

        // Susceptibility: inaccurate coaches are more swayed by reputation
        // Accurate coaches (judging_accuracy ~1.0) see through the hype (factor ~0.5)
        // Poor judges (judging_accuracy ~0.0) lean heavily on name (factor ~1.0)
        let rep_susceptibility = 1.0 - p.judging_accuracy * 0.5;

        raw_rep * rep_susceptibility * 2.5
    }

    /// Coach-player relationship score (continuous, replaces binary favorite check).
    ///
    /// Real-world effects modeled:
    /// - A trusted player gets picked even when form dips slightly
    /// - A disliked player must be clearly better to earn selection
    /// - Professional respect is separate from personal feelings:
    ///   a coach can personally dislike a player but still respect their ability
    /// - Stubborn coaches weight personal feelings more heavily
    /// - Conservative coaches weight trust more (pick "safe" known players)
    ///
    /// Returns roughly -2.0..+2.0 (strong dislike to strong favorite).
    fn relationship_score(&self, player: &Player, staff: &Staff) -> f32 {
        let p = &self.profile;

        let relation = match staff.relations.get_player(player.id) {
            Some(r) => r,
            None => return 0.0, // No history = neutral
        };

        // Level: -100..100 → -1..1 (overall sentiment)
        let level_norm = relation.level / 100.0;

        // Trust: 0..100, neutral at 50 → -0.5..0.5
        let trust_norm = (relation.trust - 50.0) / 100.0;

        // Professional respect: 0..100, neutral at 50 → -0.5..0.5
        // This is separate from personal feelings — a coach can dislike a player
        // personally but still recognize their professional quality
        let prof_respect_norm = (relation.professional_respect - 50.0) / 100.0;

        // Weight distribution depends on coach personality:
        // Stubborn coaches: personal feelings dominate (level matters most)
        // Conservative coaches: trust matters more (pick reliable knowns)
        // Professional coaches: professional_respect has the biggest say
        let personal_weight = 0.3 + p.stubbornness * 0.2;  // 0.3-0.5
        let trust_weight = 0.3 + p.conservatism * 0.1;      // 0.3-0.4
        let professional_weight = 0.4 - p.stubbornness * 0.1; // 0.3-0.4

        let raw_score = level_norm * personal_weight
            + trust_norm * trust_weight
            + prof_respect_norm * professional_weight;

        // Asymmetric scaling: negative relationships hit harder than positive help.
        // In real football, a coach-player conflict (Mourinho-Pogba, Conte-Diego Costa)
        // leads to outright exclusion, while a good relationship is just a small edge.
        let scaled = if raw_score < 0.0 {
            raw_score * 2.5 // conflict amplified
        } else {
            raw_score * 1.5 // good relationship gives moderate boost
        };

        scaled
    }

    // ========== MAIN SCORING METHODS ==========

    /// Coach-driven score for a specific tactical slot
    fn score_player_for_slot(
        &self,
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
    ) -> f32 {
        let mut score: f32 = 0.0;
        let p = &self.profile;

        // 1. Position fit — blind coaches undervalue position
        let position_fit = SquadSelector::position_fit_score(player, slot_position, slot_group);
        let position_weight = 0.30 * (1.0 - p.tactical_blindness * 0.3);
        score += position_fit * position_weight;

        // 2. Perceived quality — accurate judges trust quality more
        let quality = self.perceived_quality(player);
        let quality_weight = 0.25 + p.judging_accuracy * 0.05;
        score += quality * quality_weight;

        // 3. Match readiness — conservative coaches want match-fit players
        let readiness = self.match_readiness(player);
        let readiness_weight = 0.15 + p.conservatism * 0.05;
        score += readiness * readiness_weight;

        // 4. Condition — observable, less coach-dependent
        let condition =
            (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;
        score += condition * 0.20;

        // 5. Tactical style fit
        let tactical_bonus =
            SquadSelector::tactical_style_bonus(player, slot_position, tactics);
        let tactical_weight = 0.05 * (1.0 - p.tactical_blindness * 0.5);
        score += tactical_bonus * tactical_weight;

        // 6. Reputation — star players are harder to drop
        score += self.reputation_score(player);

        // 7. Coach-player relationship — continuous, not binary
        score += self.relationship_score(player, staff);

        // 8. Youth bonus
        let age = DateUtils::age(player.birth_date, date);
        if age <= 21 {
            score += p.youth_preference * 1.5;
        }

        // 9. Training impression bleed
        let training = self.training_impression(player);
        score += (training - 10.0) * p.attitude_weight * 0.3;

        score
    }

    /// Coach-driven overall quality (for bench selection)
    fn overall_quality(
        &self,
        player: &Player,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
    ) -> f32 {
        let p = &self.profile;

        let quality = self.perceived_quality(player);
        let readiness = self.match_readiness(player);
        let condition =
            (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;

        let mut score = quality * (0.25 + p.judging_accuracy * 0.05)
            + condition * 0.20
            + readiness * (0.15 + p.conservatism * 0.05)
            + primary_level * (0.30 * (1.0 - p.tactical_blindness * 0.3));

        // Reputation
        score += self.reputation_score(player);

        // Relationship
        score += self.relationship_score(player, staff);

        let best_pos = SquadSelector::best_tactical_position(player, tactics);
        if player.positions.get_level(best_pos) > 0 {
            score += 0.5;
        }

        let age = DateUtils::age(player.birth_date, date);
        if age <= 21 {
            score += p.youth_preference * 1.5;
        }

        let training = self.training_impression(player);
        score += (training - 10.0) * p.attitude_weight * 0.3;

        score
    }

    /// Coach-driven goalkeeper score
    fn goalkeeper_score(&self, player: &Player, staff: &Staff) -> f32 {
        let p = &self.profile;

        let gk_level = player
            .positions
            .get_level(PlayerPositionType::Goalkeeper) as f32;
        let quality = self.perceived_quality(player);
        let condition =
            (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;
        let readiness = self.match_readiness(player);

        let mut score = gk_level * (0.30 * (1.0 - p.tactical_blindness * 0.3))
            + quality * (0.25 + p.judging_accuracy * 0.05)
            + condition * 0.25
            + readiness * (0.20 + p.conservatism * 0.05);

        // Reputation and relationship matter for GKs too
        score += self.reputation_score(player) * 0.5;
        score += self.relationship_score(player, staff) * 0.5;

        score
    }
}

// ========== AVAILABILITY CHECK ==========

fn is_available(player: &Player, is_friendly: bool) -> bool {
    if player.player_attributes.is_injured {
        return false;
    }
    if player.statuses.get().contains(&PlayerStatusType::Int) {
        return false;
    }
    if !is_friendly {
        if player.player_attributes.is_banned {
            return false;
        }
        let s = player.statuses.get();
        if s.contains(&PlayerStatusType::Lst) || s.contains(&PlayerStatusType::Loa) {
            return false;
        }
    }
    true
}

impl SquadSelector {
    // ========== PUBLIC API (backward compatibility) ==========

    pub fn select(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_with_reserves(team, staff, &[])
    }

    pub fn select_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_with_context(team, staff, reserve_players, &SelectionContext::default())
    }

    /// Select squad with additional reserve/youth players available for selection
    pub fn select_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        let engine = ScoringEngine::from_staff(staff);

        // Collect all available players (first team + reserves)
        let mut available: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| is_available(p, ctx.is_friendly))
            .copied()
            .collect();

        for &rp in reserve_players {
            if is_available(rp, ctx.is_friendly) && !available.iter().any(|p| p.id == rp.id) {
                available.push(rp);
            }
        }

        let outfield_count = available
            .iter()
            .filter(|p| !Self::is_goalkeeper_player(p))
            .count();
        let gk_count = available.len() - outfield_count;

        if available.len() < DEFAULT_SQUAD_SIZE {
            warn!(
                "Squad selection for team {}: only {} available ({} outfield, {} GK, {} reserves offered)",
                team.name, available.len(), outfield_count, gk_count, reserve_players.len()
            );
        } else {
            debug!(
                "Squad selection: {} available ({} outfield, {} GK, {} reserves)",
                available.len(),
                outfield_count,
                gk_count,
                reserve_players.len()
            );
        }

        // Select starting 11
        let main_squad = Self::select_starting_eleven(
            team.id,
            &available,
            staff,
            tactics.borrow(),
            &engine,
            ctx.date,
        );

        // Remaining pool for substitutes
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad.iter().any(|mp| mp.id == p.id))
            .copied()
            .collect();

        let mut substitutes = Self::select_substitutes(
            team.id,
            &remaining,
            staff,
            tactics.borrow(),
            &engine,
            ctx.date,
        );

        // Guarantee: never leave bench empty when players exist
        if substitutes.is_empty() && !remaining.is_empty() {
            warn!(
                "Substitute selection produced empty bench with {} remaining players — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= DEFAULT_BENCH_SIZE {
                    break;
                }
                let pos = Self::best_tactical_position(player, tactics.borrow());
                substitutes.push(MatchPlayer::from_player(team.id, player, pos, false));
            }
        }

        debug!(
            "Final squad: {} starters, {} subs",
            main_squad.len(),
            substitutes.len()
        );

        PlayerSelectionResult {
            main_squad,
            substitutes,
        }
    }

    // ========== STARTING 11 SELECTION ==========

    /// Select the best starting 11 for the given formation.
    ///
    /// Algorithm:
    /// 1. Always pick the best goalkeeper first
    /// 2. For each tactical position, score ALL available players using position group
    ///    compatibility (not exact position match) combined with condition/readiness/ability
    /// 3. Greedy assignment: fill each slot with the highest-scoring unused player
    /// 4. If any slots remain unfilled, use the best available player regardless of position
    fn select_starting_eleven(
        team_id: u32,
        available: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
        engine: &ScoringEngine,
        date: NaiveDate,
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();
        let required = tactics.positions();

        // STEP 1: Goalkeeper — must always be filled
        if let Some(gk) = Self::pick_best_goalkeeper(available, &used_ids, engine, staff) {
            squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        } else {
            warn!("No goalkeeper found at all — picking any player as GK");
            if let Some(any) = Self::pick_best_unused(available, &used_ids) {
                squad.push(MatchPlayer::from_player(
                    team_id,
                    any,
                    PlayerPositionType::Goalkeeper,
                    false,
                ));
                used_ids.push(any.id);
            }
        }

        // STEP 2: Fill each outfield position using position-group scoring
        for &pos in required.iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }

            let target_group = pos.position_group();

            // Score all unused players for this position
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p))
                .max_by(|a, b| {
                    let sa = engine.score_player_for_slot(
                        a,
                        pos,
                        target_group,
                        staff,
                        tactics,
                        date,
                    );
                    let sb = engine.score_player_for_slot(
                        b,
                        pos,
                        target_group,
                        staff,
                        tactics,
                        date,
                    );
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // STEP 3: Fill any remaining slots with best available outfield players
        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p))
                .max_by(|a, b| {
                    let sa = engine.overall_quality(a, staff, tactics, date);
                    let sb = engine.overall_quality(b, staff, tactics, date);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // STEP 4: LAST RESORT — use ANY remaining player (even GKs as outfield)
        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = a.player_attributes.current_ability;
                    let sb = b.player_attributes.current_ability;
                    sa.cmp(&sb)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    warn!(
                        "Emergency fill: using {} (GK) as outfield player",
                        player.full_name
                    );
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        if squad.len() < DEFAULT_SQUAD_SIZE {
            warn!(
                "Could only select {} of 11 starting players",
                squad.len()
            );
        }

        squad
    }

    // ========== SUBSTITUTE SELECTION ==========

    fn select_substitutes(
        team_id: u32,
        remaining: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
        engine: &ScoringEngine,
        date: NaiveDate,
    ) -> Vec<MatchPlayer> {
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_BENCH_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        // 1. Backup goalkeeper (always first on the bench)
        if let Some(gk) = Self::pick_best_goalkeeper(remaining, &used_ids, engine, staff) {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // 2. Fill remaining bench spots with best overall outfield players
        //    Prefer positional variety: try to get at least one defender, midfielder, forward
        for target_group in &[
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            if subs.len() >= DEFAULT_BENCH_SIZE {
                break;
            }
            let has_group = subs.iter().any(|s| {
                s.tactical_position.current_position.position_group() == *target_group
            });
            if has_group {
                continue;
            }

            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| p.position().position_group() == *target_group)
                .max_by(|a, b| {
                    let sa = engine.overall_quality(a, staff, tactics, date);
                    let sb = engine.overall_quality(b, staff, tactics, date);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                let pos = Self::best_tactical_position(player, tactics);
                subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // 3. Fill remaining spots with the best available players by quality
        while subs.len() < DEFAULT_BENCH_SIZE {
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = engine.overall_quality(a, staff, tactics, date);
                    let sb = engine.overall_quality(b, staff, tactics, date);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        subs
    }

    // ========== SCORING (static helpers) ==========

    /// Calculate how well a player fits a target position.
    /// Returns 0..20 score.
    fn position_fit_score(
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
    ) -> f32 {
        let exact_level = player.positions.get_level(slot_position);
        if exact_level > 0 {
            return exact_level as f32;
        }

        let player_group = player.position().position_group();

        if player_group == slot_group {
            let primary_level = player
                .positions
                .positions
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0);
            return primary_level as f32 * 0.7;
        }

        let adjacent = matches!(
            (player_group, slot_group),
            (
                PlayerFieldPositionGroup::Defender,
                PlayerFieldPositionGroup::Midfielder
            ) | (
                PlayerFieldPositionGroup::Midfielder,
                PlayerFieldPositionGroup::Defender
            ) | (
                PlayerFieldPositionGroup::Midfielder,
                PlayerFieldPositionGroup::Forward
            ) | (
                PlayerFieldPositionGroup::Forward,
                PlayerFieldPositionGroup::Midfielder
            )
        );

        if adjacent {
            let primary_level = player
                .positions
                .positions
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0);
            return primary_level as f32 * 0.4;
        }

        1.0
    }

    /// Tactical style bonus for a player in a given position
    fn tactical_style_bonus(
        player: &Player,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let mut bonus = 0.0;

        match tactics.tactical_style() {
            crate::TacticalStyle::Attacking => {
                if position.is_forward()
                    || position == PlayerPositionType::AttackingMidfielderCenter
                {
                    bonus += player.skills.technical.finishing * 0.1;
                    bonus += player.skills.mental.off_the_ball * 0.1;
                }
            }
            crate::TacticalStyle::Defensive => {
                if position.is_defender() || position == PlayerPositionType::DefensiveMidfielder {
                    bonus += player.skills.technical.tackling * 0.1;
                    bonus += player.skills.mental.positioning * 0.1;
                }
            }
            crate::TacticalStyle::Possession => {
                bonus += player.skills.technical.passing * 0.08;
                bonus += player.skills.mental.vision * 0.08;
            }
            crate::TacticalStyle::Counterattack => {
                if position.is_forward() || position.is_midfielder() {
                    bonus += player.skills.physical.pace * 0.1;
                    bonus += player.skills.mental.off_the_ball * 0.08;
                }
            }
            crate::TacticalStyle::WingPlay | crate::TacticalStyle::WidePlay => {
                if position == PlayerPositionType::WingbackLeft
                    || position == PlayerPositionType::WingbackRight
                    || position == PlayerPositionType::MidfielderLeft
                    || position == PlayerPositionType::MidfielderRight
                {
                    bonus += player.skills.technical.crossing * 0.1;
                    bonus += player.skills.physical.pace * 0.08;
                }
            }
            _ => {}
        }

        bonus
    }

    // ========== HELPERS ==========

    fn is_goalkeeper_player(player: &Player) -> bool {
        player
            .positions
            .positions
            .iter()
            .any(|p| p.position == PlayerPositionType::Goalkeeper)
    }

    fn pick_best_goalkeeper<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
        engine: &ScoringEngine,
        staff: &Staff,
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| Self::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let score_a = engine.goalkeeper_score(a, staff);
                let score_b = engine.goalkeeper_score(b, staff);
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    }

    fn pick_best_unused<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = a.player_attributes.current_ability;
                let sb = b.player_attributes.current_ability;
                sa.cmp(&sb)
            })
            .copied()
    }

    /// Find the best tactical position for a player within the formation
    fn best_tactical_position(player: &Player, tactics: &Tactics) -> PlayerPositionType {
        let player_group = player.position().position_group();

        for &pos in tactics.positions() {
            if player.positions.get_level(pos) > 0 {
                return pos;
            }
        }

        for &pos in tactics.positions() {
            if pos.position_group() == player_group && pos != PlayerPositionType::Goalkeeper {
                return pos;
            }
        }

        for &pos in tactics.positions() {
            if pos != PlayerPositionType::Goalkeeper {
                return pos;
            }
        }

        player.position()
    }

    // ========== ROTATION SELECTION (for friendly/development leagues) ==========

    pub fn select_for_rotation(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_for_rotation_with_reserves(team, staff, &[])
    }

    pub fn select_for_rotation_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_for_rotation_with_context(
            team,
            staff,
            reserve_players,
            &SelectionContext {
                is_friendly: true,
                ..SelectionContext::default()
            },
        )
    }

    /// Select squad with rotation priority, supplemented by players from other club teams.
    pub fn select_for_rotation_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();

        // Collect all available players from own team
        let mut available: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| is_available(p, ctx.is_friendly))
            .copied()
            .collect();

        // Add supplementary players from other club teams only if squad is short
        if available.len() < DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE {
            let needed = (DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE) - available.len();
            let mut supplements: Vec<&Player> = reserve_players
                .iter()
                .filter(|&&rp| {
                    is_available(rp, ctx.is_friendly)
                        && !available.iter().any(|p| p.id == rp.id)
                })
                .copied()
                .collect();

            // Sort by days since last match (prefer players who need game time)
            supplements.sort_by(|a, b| {
                b.player_attributes
                    .days_since_last_match
                    .cmp(&a.player_attributes.days_since_last_match)
            });

            for rp in supplements.into_iter().take(needed) {
                available.push(rp);
            }

            if available.len() < DEFAULT_SQUAD_SIZE {
                warn!(
                    "Rotation selection for team {}: only {} available after borrowing ({} reserves offered)",
                    team.name, available.len(), reserve_players.len()
                );
            }
        }

        // Select starting 11 with rotation scoring
        let main_squad =
            Self::select_rotation_starting_eleven(team.id, &available, staff, tactics.borrow());

        // Remaining pool for substitutes
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad.iter().any(|mp| mp.id == p.id))
            .copied()
            .collect();

        let mut substitutes =
            Self::select_rotation_substitutes(team.id, &remaining, staff, tactics.borrow());

        // Guarantee: never leave bench empty when players exist
        if substitutes.is_empty() && !remaining.is_empty() {
            warn!(
                "Rotation substitute selection produced empty bench with {} remaining — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= DEFAULT_BENCH_SIZE {
                    break;
                }
                let pos = Self::best_tactical_position(player, tactics.borrow());
                substitutes.push(MatchPlayer::from_player(team.id, player, pos, false));
            }
        }

        PlayerSelectionResult {
            main_squad,
            substitutes,
        }
    }

    fn select_rotation_starting_eleven(
        team_id: u32,
        available: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();
        let required = tactics.positions();

        if let Some(gk) = Self::pick_rotation_goalkeeper(available, &used_ids) {
            squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        } else if let Some(any) = Self::pick_best_unused(available, &used_ids) {
            squad.push(MatchPlayer::from_player(
                team_id,
                any,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(any.id);
        }

        for &pos in required.iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }

            let target_group = pos.position_group();

            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p))
                .max_by(|a, b| {
                    let sa = Self::rotation_score_for_slot(a, pos, target_group, staff, tactics);
                    let sb = Self::rotation_score_for_slot(b, pos, target_group, staff, tactics);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p))
                .max_by(|a, b| {
                    let sa = Self::rotation_overall_quality(a);
                    let sb = Self::rotation_overall_quality(b);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // Last resort — any player
        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = a.player_attributes.days_since_last_match;
                    let sb = b.player_attributes.days_since_last_match;
                    sa.cmp(&sb)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        squad
    }

    fn select_rotation_substitutes(
        team_id: u32,
        remaining: &[&Player],
        _staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_BENCH_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        if let Some(gk) = Self::pick_rotation_goalkeeper(remaining, &used_ids) {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        while subs.len() < DEFAULT_BENCH_SIZE {
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = Self::rotation_overall_quality(a);
                    let sb = Self::rotation_overall_quality(b);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        subs
    }

    fn rotation_score_for_slot(
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        _staff: &Staff,
        _tactics: &Tactics,
    ) -> f32 {
        let mut score: f32 = 0.0;

        let days = player.player_attributes.days_since_last_match as f32;
        let rest_score = (days / 14.0).min(1.0) * 20.0;
        score += rest_score * 0.40;

        let position_fit = Self::position_fit_score(player, slot_position, slot_group);
        score += position_fit * 0.30;

        let condition =
            (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0);
        score += condition * 20.0 * 0.20;

        let ability = player.player_attributes.current_ability as f32 / 200.0;
        score += ability * 20.0 * 0.10;

        score
    }

    fn rotation_overall_quality(player: &Player) -> f32 {
        let days = player.player_attributes.days_since_last_match as f32;
        let rest_score = (days / 14.0).min(1.0) * 20.0;
        let condition =
            (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;

        rest_score * 0.50
            + condition * 0.30
            + (player.player_attributes.current_ability as f32 / 200.0 * 20.0) * 0.20
    }

    fn pick_rotation_goalkeeper<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| Self::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let da = a.player_attributes.days_since_last_match;
                let db = b.player_attributes.days_since_last_match;
                da.cmp(&db)
            })
            .copied()
    }

    // ========== LEGACY PUBLIC API ==========

    pub fn calculate_player_rating_for_position(
        player: &Player,
        staff: &Staff,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let group = position.position_group();
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        engine.score_player_for_slot(player, position, group, staff, tactics, date)
    }

    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        Self::select_starting_eleven(team_id, players, staff, tactics, &engine, date)
    }

    pub fn select_substitutes_legacy(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        Self::select_substitutes(team_id, players, staff, tactics, &engine, date)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntegerUtils, MatchTacticType, PlayerCollection, PlayerGenerator, StaffCollection,
        TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveTime, Utc};

    #[test]
    fn test_squad_selection_always_produces_11() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        assert_eq!(result.main_squad.len(), 11);
        assert!(!result.substitutes.is_empty());
        assert!(result.substitutes.len() <= DEFAULT_BENCH_SIZE);
    }

    #[test]
    fn test_squad_always_has_goalkeeper() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        let has_gk = result.main_squad.iter().any(|p| {
            p.tactical_position.current_position == PlayerPositionType::Goalkeeper
        });
        assert!(has_gk, "Starting 11 must always have a goalkeeper");
    }

    #[test]
    fn test_squad_no_duplicate_players() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
        all_ids.extend(result.substitutes.iter().map(|p| p.id));

        let unique_count = {
            let mut sorted = all_ids.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len()
        };
        assert_eq!(all_ids.len(), unique_count, "No player should appear twice");
    }

    #[test]
    fn test_position_group_matching() {
        let score = SquadSelector::position_fit_score(
            &generate_defender_center(),
            PlayerPositionType::DefenderCenterLeft,
            PlayerFieldPositionGroup::Defender,
        );
        assert!(score > 5.0, "Same-group player should score well: {}", score);
    }

    // ========== Test helpers ==========

    fn generate_test_team() -> Team {
        let mut team = TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Test Team".to_string())
            .slug("test-team".to_string())
            .team_type(TeamType::Main)
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            ))
            .reputation(TeamReputation::new(100, 100, 100))
            .players(PlayerCollection::new(generate_test_players()))
            .staffs(StaffCollection::new(Vec::new()))
            .tactics(Some(Tactics::new(MatchTacticType::T442)))
            .build()
            .expect("Failed to build test team");

        team.tactics = Some(Tactics::new(MatchTacticType::T442));
        team
    }

    fn generate_test_staff() -> Staff {
        crate::StaffStub::default()
    }

    fn generate_test_players() -> Vec<Player> {
        let mut players = Vec::new();

        for &position in &[
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::Striker,
        ] {
            for _ in 0..3 {
                let level = IntegerUtils::random(15, 20) as u8;
                let player =
                    PlayerGenerator::generate(1, Utc::now().date_naive(), position, level);
                players.push(player);
            }
        }

        players
    }

    fn generate_defender_center() -> Player {
        PlayerGenerator::generate(
            1,
            Utc::now().date_naive(),
            PlayerPositionType::DefenderCenter,
            18,
        )
    }
}
