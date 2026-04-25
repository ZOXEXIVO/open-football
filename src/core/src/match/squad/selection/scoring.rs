use crate::club::player::load::{FATIGUE_LOAD_DANGER, FATIGUE_LOAD_THRESHOLD};
use crate::club::staff::perception::CoachProfile;
use crate::club::{ClubPhilosophy, PlayerFieldPositionGroup, PlayerPositionType, Staff};
use crate::utils::DateUtils;
use crate::{Player, Tactics};
use chrono::NaiveDate;

use super::helpers;

pub(crate) struct ScoringEngine {
    pub(crate) profile: CoachProfile,
    /// Club philosophy tilts selection — DevelopAndSell pushes youth further
    /// up the XI, LoanFocused prefers loan signings when merit is close.
    pub(crate) philosophy: Option<ClubPhilosophy>,
}

impl ScoringEngine {
    pub fn from_staff(staff: &Staff) -> Self {
        ScoringEngine {
            profile: CoachProfile::from_staff(staff),
            philosophy: None,
        }
    }

    pub fn from_staff_with_philosophy(staff: &Staff, philosophy: Option<ClubPhilosophy>) -> Self {
        ScoringEngine {
            profile: CoachProfile::from_staff(staff),
            philosophy,
        }
    }

    /// Philosophy-specific selection tilt. Small magnitudes so philosophy
    /// biases but doesn't swamp real quality signals.
    pub fn philosophy_bonus(&self, player: &Player, date: NaiveDate) -> f32 {
        let Some(phil) = self.philosophy.as_ref() else {
            return 0.0;
        };
        let age = DateUtils::age(player.birth_date, date);
        let is_loan_in = player.contract_loan.is_some();
        match phil {
            ClubPhilosophy::DevelopAndSell => {
                // Clubs built around developing and selling push youth up
                // the XI even in important matches.
                match age {
                    0..=17 => 0.5,
                    18..=21 => 1.2,
                    22..=23 => 0.6,
                    _ => 0.0,
                }
            }
            ClubPhilosophy::LoanFocused => {
                if is_loan_in {
                    0.9
                } else {
                    0.0
                }
            }
            ClubPhilosophy::SignToCompete => {
                // Experienced heads get the nod; youngsters are backup.
                match age {
                    25..=32 => 0.4,
                    18..=21 => -0.4,
                    _ => 0.0,
                }
            }
            _ => 0.0,
        }
    }

    /// Lens-weighted skill composite using the coach's perception lens
    pub fn perceived_quality(&self, player: &Player) -> f32 {
        let lens = &self.profile.perception_lens;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;

        // Technical composite
        let atk_tech =
            (t.finishing + t.dribbling + t.crossing + t.first_touch + t.technique + t.long_shots)
                / 6.0;
        let def_tech = (t.tackling + t.marking + t.heading + t.passing) / 4.0;
        let tech_score = atk_tech * lens.attacking_focus + def_tech * (1.0 - lens.attacking_focus);

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
        let position_contribution = position_level * (1.0 - self.profile.tactical_blindness * 0.5);

        let base = skill_composite * 0.75 + position_contribution * 0.25;

        // Form bonus amplified by recency_bias. Prefer the fast-moving EMA
        // (`load.form_rating`) when the player has accumulated form data;
        // fall back to the season-average only for players without a
        // recent match rating (e.g. just arrived from another club).
        let raw_form_bonus = if player.load.form_rating > 0.0 {
            (player.load.form_rating - 6.5).clamp(-1.5, 1.5)
        } else if player.statistics.played + player.statistics.played_subs > 3 {
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
    pub fn match_readiness(&self, player: &Player) -> f32 {
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

    /// Training impression: blends visible effort with actual training performance.
    pub fn training_impression(&self, player: &Player) -> f32 {
        let visible_effort = (player.skills.mental.work_rate
            + player.skills.mental.determination
            + player.skills.mental.teamwork)
            / 3.0;

        let actual_performance = player.training.training_performance;

        let actual_weight = 0.30 + self.profile.judging_accuracy * 0.40;
        let blended = actual_performance * actual_weight + visible_effort * (1.0 - actual_weight);

        blended * (0.5 + self.profile.attitude_weight * 0.5)
    }

    /// Recent-workload penalty for squad rotation. Returns a non-positive
    /// bonus: zero for fresh players, down to roughly −6 for players on
    /// the edge of overload. Combined with selection scoring so managers
    /// naturally rotate through weeks of congested fixtures instead of
    /// flogging the same XI into the ground.
    ///
    /// Friendlies don't rotate — preseason / testimonial XIs already
    /// feature a different player pool — so this returns 0 there.
    pub fn fatigue_penalty(&self, player: &Player, is_friendly: bool) -> f32 {
        if is_friendly {
            return 0.0;
        }
        let load = player.load.minutes_last_7;
        if load <= FATIGUE_LOAD_THRESHOLD {
            return 0.0;
        }
        // Linear ramp from 0 at the threshold to -3.0 at the danger line.
        let over = load - FATIGUE_LOAD_THRESHOLD;
        let span = (FATIGUE_LOAD_DANGER - FATIGUE_LOAD_THRESHOLD).max(1.0);
        let t = (over / span).min(2.0); // allow overshoot beyond danger
                                        // Risk-tolerant coaches shrug off load; conservative coaches rotate early.
        let scale = 1.0 - self.profile.risk_tolerance * 0.4;
        -(t * 3.0) * scale
    }

    /// Unified condition floor penalty
    pub fn condition_floor_penalty(&self, player: &Player, is_friendly: bool) -> f32 {
        let p = &self.profile;
        let condition_pct = player.player_attributes.condition_percentage() as f32;
        let condition_threshold = if is_friendly {
            25.0
        } else {
            40.0 - p.risk_tolerance * 8.0
        };
        if condition_pct < condition_threshold {
            let deficit = (condition_threshold - condition_pct) / condition_threshold;
            deficit * 40.0 * (1.0 - p.risk_tolerance * 0.3)
        } else {
            0.0
        }
    }

    /// Player reputation score (0..~2.5)
    pub fn reputation_score(&self, player: &Player) -> f32 {
        let p = &self.profile;

        let current_rep =
            (player.player_attributes.current_reputation as f32 / 3000.0).clamp(0.0, 1.0);
        let world_rep = (player.player_attributes.world_reputation as f32 / 1200.0).clamp(0.0, 1.0);
        let intl_factor =
            (player.player_attributes.international_apps as f32 / 50.0).clamp(0.0, 1.0);

        let raw_rep = current_rep * 0.4 + world_rep * 0.4 + intl_factor * 0.2;
        let rep_susceptibility = 1.0 - p.judging_accuracy * 0.5;

        raw_rep * rep_susceptibility * 2.5
    }

    /// Coach-player relationship score (-2.0..+2.0)
    pub fn relationship_score(&self, player: &Player, staff: &Staff) -> f32 {
        let p = &self.profile;

        let relation = match staff.relations.get_player(player.id) {
            Some(r) => r,
            None => return 0.0,
        };

        let level_norm = relation.level / 100.0;
        let trust_norm = (relation.trust - 50.0) / 100.0;
        let prof_respect_norm = (relation.professional_respect - 50.0) / 100.0;

        let personal_weight = 0.3 + p.stubbornness * 0.2;
        let trust_weight = 0.3 + p.conservatism * 0.1;
        let professional_weight = 0.4 - p.stubbornness * 0.1;

        let raw_score = level_norm * personal_weight
            + trust_norm * trust_weight
            + prof_respect_norm * professional_weight;

        if raw_score < 0.0 {
            raw_score * 2.5
        } else {
            raw_score * 1.5
        }
    }

    /// Newcomer integration penalty
    pub fn newcomer_penalty(player: &Player, date: NaiveDate, profile: &CoachProfile) -> f32 {
        let transfer_date = match player.last_transfer_date {
            Some(d) => d,
            None => return 0.0,
        };

        let days_at_club = (date - transfer_date).num_days().max(0) as f32;
        let appearances = (player.statistics.played + player.statistics.played_subs) as f32;

        let rep_factor =
            (player.player_attributes.world_reputation as f32 / 1200.0).clamp(0.0, 1.0);
        let max_penalty = 3.5 * (1.0 - rep_factor * 0.77);
        let apps_to_integrate = 3.0 + (1.0 - rep_factor) * 5.0;

        let coach_speed = 1.0 + profile.risk_tolerance * 0.3 - profile.conservatism * 0.3
            + profile.judging_accuracy * 0.2;

        let app_factor = (appearances * coach_speed / apps_to_integrate).clamp(0.0, 1.0);

        let time_to_integrate = 30.0 + (1.0 - rep_factor) * 30.0;
        let time_factor = (days_at_club / time_to_integrate).clamp(0.0, 1.0);

        let integration = (app_factor * 0.7 + time_factor * 0.3).clamp(0.0, 1.0);

        max_penalty * (1.0 - integration)
    }

    /// Pairwise chemistry bonus (-0.8..+1.0)
    pub fn cohesion_bonus(
        &self,
        player: &Player,
        selected_players: &[&Player],
        slot_group: PlayerFieldPositionGroup,
    ) -> f32 {
        if selected_players.is_empty() {
            return 0.0;
        }

        let p = &self.profile;
        let mut total = 0.0f32;
        let mut weight_sum = 0.0f32;

        for teammate in selected_players {
            let proximity_weight = {
                let teammate_group = teammate.position().position_group();
                if teammate_group == slot_group {
                    1.0
                } else if helpers::is_adjacent_group(teammate_group, slot_group) {
                    0.5
                } else {
                    0.2
                }
            };

            let rel_quality = match player.relations.get_player(teammate.id) {
                Some(rel) => {
                    let level_norm = rel.level / 100.0;
                    let trust_norm = (rel.trust - 50.0) / 100.0;
                    let prof_norm = (rel.professional_respect - 50.0) / 100.0;
                    level_norm * 0.4 + trust_norm * 0.3 + prof_norm * 0.3
                }
                None => 0.0,
            };

            total += rel_quality * proximity_weight;
            weight_sum += proximity_weight;
        }

        if weight_sum == 0.0 {
            return 0.0;
        }

        let avg = total / weight_sum;
        let scale = 1.0 + p.conservatism * 0.3;

        (avg * scale * 2.0).clamp(-0.8, 1.0)
    }

    /// Score for a specific tactical slot (starting XI selection)
    pub fn score_player_for_slot(
        &self,
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
        is_friendly: bool,
        selected_players: &[&Player],
    ) -> f32 {
        let mut score: f32 = 0.0;
        let p = &self.profile;

        score += helpers::position_fit_score(player, slot_position, slot_group)
            * (0.20 * (1.0 - p.tactical_blindness * 0.3));

        score += self.perceived_quality(player) * (0.40 + p.judging_accuracy * 0.05);

        score += self.match_readiness(player) * (0.15 + p.conservatism * 0.05);

        score -= self.condition_floor_penalty(player, is_friendly);

        score += helpers::tactical_style_bonus(player, slot_position, tactics)
            * (0.05 * (1.0 - p.tactical_blindness * 0.5));
        score += helpers::side_foot_bonus(player, slot_position)
            * (0.6 * (1.0 - p.tactical_blindness * 0.3));

        let rep = self.reputation_score(player);
        let rel = self.relationship_score(player, staff);
        score += rep;
        let rel_dampening = if rel < 0.0 {
            1.0
        } else {
            (1.0 - rep * 0.15).max(0.3)
        };
        score += rel * rel_dampening;

        score -= Self::newcomer_penalty(player, date, p);

        let age = DateUtils::age(player.birth_date, date);
        let youth_multiplier = match age {
            0..=16 => 0.0,
            17..=18 => 2.5,
            19..=20 => 1.5,
            21 => 0.8,
            _ => 0.0,
        };
        score += p.youth_preference * youth_multiplier;

        score += (self.training_impression(player) - 10.0) * p.attitude_weight * 0.3;

        score += self.cohesion_bonus(player, selected_players, slot_group);

        // Squad status tilt — labelled starters get their planned minutes.
        score += self.squad_status_bonus(player);

        // Club philosophy tilt — development clubs push youth up, loan-
        // focused clubs reward borrowed talent.
        score += self.philosophy_bonus(player, date);

        if player.position().position_group() != slot_group {
            score -= 1.5;
        }

        score
    }

    /// Squad status tilt — the coach has a plan for each player's minutes
    /// at the start of the season. KeyPlayer and FirstTeamRegular always
    /// play when fit; NotNeeded is a bench dweller; HotProspect gets a small
    /// preferential nod in rotation calls. Conservative coaches lean into
    /// the plan; risk-takers override it on form.
    pub fn squad_status_bonus(&self, player: &Player) -> f32 {
        use crate::club::PlayerSquadStatus;
        let Some(contract) = player.contract.as_ref() else {
            return 0.0;
        };
        let raw = match contract.squad_status {
            PlayerSquadStatus::KeyPlayer => 1.8,
            PlayerSquadStatus::FirstTeamRegular => 1.0,
            PlayerSquadStatus::FirstTeamSquadRotation => 0.3,
            PlayerSquadStatus::HotProspectForTheFuture => 0.5,
            PlayerSquadStatus::DecentYoungster => 0.1,
            PlayerSquadStatus::MainBackupPlayer => -0.2,
            PlayerSquadStatus::NotNeeded => -1.2,
            _ => 0.0,
        };
        // Conservative coaches respect the label; risk-takers ignore it.
        let weight = 0.6 + self.profile.conservatism * 0.8 - self.profile.risk_tolerance * 0.3;
        raw * weight.clamp(0.2, 1.4)
    }

    /// Bonus for underplayed players in low-importance matches.
    /// When match_importance < 0.4, reserve/youth players who haven't played
    /// much get a significant boost — simulates managers resting stars and
    /// giving fringe players chances in dead rubbers.
    pub fn development_minutes_bonus(&self, player: &Player, match_importance: f32) -> f32 {
        if match_importance >= 0.5 {
            return 0.0;
        }

        let rotation_factor = (0.5 - match_importance) * 2.0; // 0.0 at 0.5, 1.0 at 0.0

        let days_idle = player.player_attributes.days_since_last_match as f32;
        let total_games = (player.statistics.played + player.statistics.played_subs) as f32;

        // Players who haven't played recently need minutes
        let rest_bonus = (days_idle / 21.0).min(1.0) * 2.0;

        // Players with few season appearances need development time
        let minutes_bonus = if total_games < 10.0 {
            (10.0 - total_games) * 0.3
        } else {
            0.0
        };

        (rest_bonus + minutes_bonus) * rotation_factor
    }

    /// Risk of asking a player to start while physically fragile. This is
    /// separate from the hard availability gate: managers will sometimes
    /// risk a tired star in a final, but usually protect them in normal games.
    pub fn injury_risk_penalty(
        &self,
        player: &Player,
        match_importance: f32,
        is_friendly: bool,
    ) -> f32 {
        if is_friendly {
            return 0.0;
        }

        let condition = player.player_attributes.condition_percentage() as f32;
        let fitness = (player.player_attributes.fitness as f32 / 10000.0).clamp(0.0, 1.0);
        let natural_fitness = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
        let load_7 = (player.load.minutes_last_7 / FATIGUE_LOAD_DANGER).clamp(0.0, 1.8);
        let matches_14 = player.load.matches_last_14() as f32;

        let condition_risk = ((65.0 - condition) / 65.0).clamp(0.0, 1.0);
        let fitness_risk = 1.0 - fitness;
        let durability_risk = 1.0 - natural_fitness;
        let match_density_risk = ((matches_14 - 3.0) / 3.0).clamp(0.0, 1.0);

        let raw = condition_risk * 2.4
            + fitness_risk * 1.4
            + durability_risk * 0.8
            + load_7 * 1.6
            + match_density_risk * 1.2;

        let importance_dampener = (1.15 - match_importance).clamp(0.25, 1.10);
        raw * importance_dampener * (1.0 - self.profile.risk_tolerance * 0.35)
    }

    /// Overall quality score (bench selection)
    pub fn overall_quality(
        &self,
        player: &Player,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
        is_friendly: bool,
    ) -> f32 {
        let p = &self.profile;

        let quality = self.perceived_quality(player);
        let readiness = self.match_readiness(player);
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;

        let mut score = quality * (0.40 + p.judging_accuracy * 0.05)
            + readiness * (0.15 + p.conservatism * 0.05)
            + primary_level * (0.20 * (1.0 - p.tactical_blindness * 0.3));

        score -= self.condition_floor_penalty(player, is_friendly);

        let rep = self.reputation_score(player);
        let rel = self.relationship_score(player, staff);
        score += rep;
        let rel_dampening = if rel < 0.0 {
            1.0
        } else {
            (1.0 - rep * 0.15).max(0.3)
        };
        score += rel * rel_dampening;

        let best_pos = helpers::best_tactical_position(player, tactics);
        if player.positions.get_level(best_pos) > 0 {
            score += 0.5;
        }

        score -= Self::newcomer_penalty(player, date, p);

        let age = DateUtils::age(player.birth_date, date);
        let youth_multiplier = match age {
            0..=16 => 0.0,
            17..=18 => 2.5,
            19..=20 => 1.5,
            21 => 0.8,
            _ => 0.0,
        };
        score += p.youth_preference * youth_multiplier;

        score += (self.training_impression(player) - 10.0) * p.attitude_weight * 0.3;

        // Squad status tilt applies to bench ordering too: a KeyPlayer
        // resting on the bench still gets priority to come on.
        score += self.squad_status_bonus(player) * 0.6;

        // Philosophy bench tilt — half as strong as in the XI, since
        // bench selection is already broad.
        score += self.philosophy_bonus(player, date) * 0.5;

        // Bench integration bonus: coaches want to give minutes to players
        // who haven't played much — loan players, youth, returning from injury.
        // A good coach includes them on the bench to sub in when possible.
        let total_games = (player.statistics.played + player.statistics.played_subs) as f32;
        if total_games < 5.0 {
            let loan_factor = if player.contract_loan.is_some() {
                1.3
            } else {
                1.0
            };
            let need_minutes_bonus = (5.0 - total_games) * 0.4 * loan_factor;
            score += need_minutes_bonus;
        }

        // Loan match fee incentive: if the parent club pays per appearance,
        // the borrowing club has a financial reason to include the player.
        if let Some(ref loan) = player.contract_loan {
            if let Some(fee) = loan.loan_match_fee {
                // Small score bonus proportional to the fee — capped so it
                // nudges selection without overriding quality.
                let fee_bonus = (fee as f32 / 10000.0).min(1.0);
                score += fee_bonus;
            }
        }

        score
    }

    /// Goalkeeper score — CA first, keeper-specific skills second, everything
    /// else a tiebreaker. `perceived_quality()` composes from outfield skills
    /// (finishing, dribbling, tackling, passing…) and never reads the
    /// goalkeeping block, so for a keeper-vs-keeper comparison it reflects
    /// the wrong attributes. We anchor on `current_ability` (so the better
    /// keeper plays) and add a GK-specific skill composite that actually
    /// reads handling, reflexes, aerial ability, and distribution.
    pub fn goalkeeper_score(&self, player: &Player, staff: &Staff, is_friendly: bool) -> f32 {
        let ca = player.player_attributes.current_ability as f32 / 10.0;
        let gk_q = self.gk_perceived_quality(player);
        let gk_level = player.positions.get_level(PlayerPositionType::Goalkeeper) as f32;
        let readiness = self.match_readiness(player);

        let mut score = ca * 2.0 + gk_q * 1.0 + gk_level * 0.10 + readiness * 0.20;

        score -= self.condition_floor_penalty(player, is_friendly);

        score += self.reputation_score(player) * 0.30;
        score += self.relationship_score(player, staff) * 0.30;

        score
    }

    /// Keeper-specific skill composite. Mirrors `perceived_quality` but
    /// built from the goalkeeping skill block plus the mental/physical
    /// attributes that actually matter for shot-stopping, crosses, and
    /// distribution. All inputs are on the 1-20 scale, so the result is
    /// 1-20 too — directly comparable to readiness and gk_level terms.
    fn gk_perceived_quality(&self, player: &Player) -> f32 {
        let gk = &player.skills.goalkeeping;
        let m = &player.skills.mental;
        let ph = &player.skills.physical;

        let shot_stopping = (gk.handling + gk.reflexes + gk.one_on_ones) / 3.0;
        let aerial = (gk.aerial_reach + gk.command_of_area + ph.jumping) / 3.0;
        let distribution = (gk.kicking + gk.throwing + gk.passing) / 3.0;
        let sweeper = (gk.rushing_out + gk.communication + m.decisions + m.anticipation) / 4.0;
        let keeper_mind = (m.concentration + m.positioning + m.composure + m.bravery) / 4.0;

        shot_stopping * 0.40
            + aerial * 0.20
            + keeper_mind * 0.20
            + sweeper * 0.10
            + distribution * 0.10
    }
}
