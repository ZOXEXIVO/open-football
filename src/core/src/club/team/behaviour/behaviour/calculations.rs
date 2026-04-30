//! Stateless scoring helpers consumed across the `process_*` passes.
//! Each function is a pure read of player state — no side effects,
//! no allocations — so the caller can apply the result wherever the
//! domain logic in its own module dictates.

use super::TeamBehaviour;
use crate::context::GlobalContext;
use crate::utils::FloatUtils;
use crate::{PersonBehaviourState, Player, PlayerPositionType, PlayerRelation};

impl TeamBehaviour {
    pub(super) fn calculate_daily_interaction_change(
        player_a: &Player,
        player_b: &Player,
        existing_relationship: &PlayerRelation,
        _ctx: &GlobalContext<'_>,
    ) -> f32 {
        let relationship_level = existing_relationship.level;

        let temperament_factor =
            (player_a.attributes.temperament + player_b.attributes.temperament) / 40.0;
        // Tighter random envelope than the previous -0.02..0.02 — daily
        // drift was overpowering the football-context passes (partnership
        // chemistry, rivalry, manager-credibility) over a season. Most
        // movement should come from those signals; this is just the
        // small day-to-day noise on top.
        let base_random = FloatUtils::random(-0.008, 0.008) * temperament_factor;

        // Interaction-frequency modulator — chatty pairs swing more,
        // pairs who barely interact see only a fraction of the random
        // drift. Without this, every pair drifts identically regardless
        // of how often they actually meet on the training pitch.
        let interaction_mult = if existing_relationship.interaction_frequency > 0.6 {
            1.25
        } else if existing_relationship.interaction_frequency < 0.2 {
            0.5
        } else {
            1.0
        };
        let base_change = base_random * interaction_mult;

        let trust_factor = existing_relationship.trust / 100.0;
        let friendship_factor = existing_relationship.friendship / 100.0;

        // Strong relationships should trend stable — a level-70 pair
        // shouldn't keep drifting toward 100 on random noise alone. We
        // let negatives through (someone *can* fall out with a friend)
        // but heavily damp positive drift past the threshold.
        if relationship_level > 60.0 {
            if base_change >= 0.0 {
                base_change * 0.25
            } else {
                base_change * 0.6
            }
        } else if relationship_level > 50.0 {
            let stability_bonus = (trust_factor * 0.3 + friendship_factor * 0.2) * base_change;
            base_change * 0.5 + stability_bonus
        } else if relationship_level < -50.0 {
            base_change + 0.01 * (1.0 - trust_factor)
        } else {
            let professional_factor = existing_relationship.professional_respect / 100.0;
            base_change * (1.0 - professional_factor * 0.3)
        }
    }

    pub(super) fn calculate_mood_spread(
        unhappy_player: &Player,
        other_player: &Player,
        happiness: f32,
    ) -> f32 {
        // Unhappy players with high leadership or reputation spread negativity more
        let leadership_influence = unhappy_player.skills.mental.leadership / 20.0;
        let rep_influence =
            (unhappy_player.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let influence = ((leadership_influence + rep_influence) / 2.0) * happiness.abs() * 0.1;

        // Players with high professionalism resist negative influence
        let resistance = other_player.attributes.professionalism / 20.0;

        // Return negative: mood spread from unhappy players damages relationships
        -(influence * (1.0 - resistance).max(0.0))
    }

    /// Returns (a_toward_b, b_toward_a) jealousy values.
    /// The lower-paid player feels jealousy (negative); the higher-paid player is unaffected.
    pub(super) fn calculate_contract_jealousy(player_a: &Player, player_b: &Player) -> (f32, f32) {
        let salary_a = player_a.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let salary_b = player_b.contract.as_ref().map(|c| c.salary).unwrap_or(0);

        if salary_a == 0 || salary_b == 0 {
            return (0.0, 0.0);
        }

        let salary_ratio = salary_a as f32 / salary_b as f32;

        if salary_ratio > 2.0 || salary_ratio < 0.5 {
            let rep_a = player_a.player_attributes.current_reputation as f32;
            let rep_b = player_b.player_attributes.current_reputation as f32;

            if salary_a > salary_b {
                // A earns more — B feels jealousy toward A
                // Jealousy is reduced if A also has higher reputation (justified pay)
                let rep_alignment = if rep_a > rep_b { 0.5 } else { 1.5 };
                let jealousy = -0.08 * (player_b.attributes.ambition / 20.0) * rep_alignment;
                (0.0, jealousy)
            } else {
                // B earns more — A feels jealousy toward B
                let rep_alignment = if rep_b > rep_a { 0.5 } else { 1.5 };
                let jealousy = -0.08 * (player_a.attributes.ambition / 20.0) * rep_alignment;
                (jealousy, 0.0)
            }
        } else {
            (0.0, 0.0)
        }
    }

    pub(super) fn calculate_injury_sympathy(_injured_player: &Player, other_player: &Player) -> f32 {
        let empathy = other_player.attributes.sportsmanship / 20.0;
        let team_spirit = other_player.skills.mental.teamwork / 20.0;

        (empathy + team_spirit) * 0.08
    }

    pub(super) fn calculate_national_team_bond(player_a: &Player, player_b: &Player) -> f32 {
        let int_experience_a =
            (player_a.player_attributes.international_apps as f32 / 50.0).min(1.0);
        let int_experience_b =
            (player_b.player_attributes.international_apps as f32 / 50.0).min(1.0);

        // Reputation similarity among compatriots strengthens bonds
        let rep_a = player_a.player_attributes.current_reputation as f32;
        let rep_b = player_b.player_attributes.current_reputation as f32;
        let rep_similarity = 1.0 - ((rep_a - rep_b).abs() / 10000.0).clamp(0.0, 1.0);

        (int_experience_a + int_experience_b) * 0.04 * (0.7 + 0.3 * rep_similarity)
    }

    pub(super) fn calculate_player_happiness(player: &Player) -> f32 {
        let mut happiness = 0.0;

        // Contract satisfaction - high reputation players have higher expectations
        let rep_expectation =
            (player.player_attributes.current_reputation as f32 / 5000.0).clamp(0.5, 2.0);

        happiness += player
            .contract
            .as_ref()
            .map(|c| (c.salary as f32 / (10000.0 * rep_expectation)).min(1.0))
            .unwrap_or(-0.5);

        // Playing time satisfaction - star players expect to start
        if player.statistics.played > 20 {
            happiness += 0.3;
        } else if player.statistics.played > 10 {
            happiness += 0.1;
        } else {
            // High rep players get more upset about not playing
            happiness -= 0.2 * (1.0 + (rep_expectation - 1.0) * 0.5);
        }

        // Performance satisfaction
        let goals_ratio =
            player.statistics.goals as f32 / player.statistics.played.max(1) as f32;
        if player.position().is_forward() && goals_ratio > 0.5 {
            happiness += 0.2;
        } else if !player.position().is_forward() && goals_ratio > 0.3 {
            happiness += 0.15;
        }

        // Personality factors
        happiness += (player.attributes.professionalism - 10.0) / 100.0;
        happiness -= (player.attributes.controversy - 10.0) / 50.0;

        // Behavior state
        match player.behaviour.state {
            PersonBehaviourState::Good => happiness += 0.2,
            PersonBehaviourState::Poor => happiness -= 0.3,
            PersonBehaviourState::Normal => {}
        }

        happiness.clamp(-1.0, 1.0)
    }

    pub(super) fn calculate_competition_factor(player_a: &Player, player_b: &Player) -> f32 {
        let ability_diff = (player_a.player_attributes.current_ability as f32
            - player_b.player_attributes.current_ability as f32)
            .abs();

        // Similar abilities = more competition
        let competition_base = 0.3 - (ability_diff / 100.0);

        // Ambition increases competition
        let ambition_factor =
            (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

        // Reputation amplifies competition: both high-rep players fight harder for spots
        let rep_a =
            (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_b =
            (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_factor = 1.0 + (rep_a + rep_b) * 0.25;

        (competition_base * ambition_factor * rep_factor).clamp(0.0, 0.5)
    }

    pub(super) fn calculate_synergy_factor(player_a: &Player, player_b: &Player) -> f32 {
        let teamwork_factor =
            (player_a.skills.mental.teamwork + player_b.skills.mental.teamwork) / 40.0;
        let professionalism_factor =
            (player_a.attributes.professionalism + player_b.attributes.professionalism) / 40.0;

        // Higher combined reputation means higher-quality partnership
        let rep_a =
            (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_b =
            (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_bonus = 1.0 + (rep_a + rep_b) * 0.15;

        (teamwork_factor * professionalism_factor * 0.2 * rep_bonus).min(0.3)
    }

    pub(super) fn are_complementary_positions(
        pos_a: &PlayerPositionType,
        pos_b: &PlayerPositionType,
    ) -> bool {
        use PlayerPositionType::*;

        match (pos_a, pos_b) {
            (
                DefenderCenter | DefenderLeft | DefenderRight,
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
            ) => true,
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
            ) => true,
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
                DefenderCenter | DefenderLeft | DefenderRight,
            ) => true,
            (
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
            ) => true,
            _ => false,
        }
    }

    pub(super) fn calculate_age_relationship_factor(age_a: u8, age_b: u8, age_diff: i32) -> f32 {
        match (age_a, age_b) {
            // Both young (16-22) - natural bonding
            (16..=22, 16..=22) if age_diff <= 3 => FloatUtils::random(0.1, 0.25),

            // Young and experienced (30+) - mentorship potential
            (16..=22, 30..) | (30.., 16..=22) => FloatUtils::random(-0.05, 0.2),

            // Prime age players (23-29) - competitive tension
            (23..=29, 23..=29) if age_diff <= 2 => FloatUtils::random(-0.1, 0.1),

            // Large age gaps - respect or indifference
            _ if age_diff > 8 => FloatUtils::random(-0.1, 0.1),

            // Similar ages in general - slight positive
            _ if age_diff <= 2 => FloatUtils::random(0.0, 0.1),

            _ => 0.0,
        }
    }

    pub(super) fn calculate_player_performance_rating(player: &Player) -> f32 {
        let goals_factor =
            (player.statistics.goals as f32 / (player.statistics.played.max(1) as f32)) * 10.0;
        let assists_factor =
            (player.statistics.assists as f32 / (player.statistics.played.max(1) as f32)) * 5.0;
        let appearance_factor = (player.statistics.played as f32 / 30.0).min(1.0) * 5.0;
        let rating_factor = player.statistics.average_rating;

        // Factor in reputation: a high-reputation player who performs poorly stands out
        let rep_factor =
            (player.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_adjustment = rep_factor * 2.0;

        (goals_factor + assists_factor + appearance_factor + rating_factor + rep_adjustment) / 5.0
    }

    pub(super) fn calculate_performance_relationship_factor(
        perf_a: f32,
        perf_b: f32,
        diff: f32,
        player_a: &Player,
        player_b: &Player,
    ) -> f32 {
        if diff < 1.0 {
            // Similar performance - mutual respect
            FloatUtils::random(0.05, 0.15)
        } else if diff > 3.0 {
            // Large performance gap
            let higher_rep = player_a
                .player_attributes
                .current_reputation
                .max(player_b.player_attributes.current_reputation)
                as f32;
            let rep_scale = (higher_rep / 10000.0).clamp(0.1, 1.0);

            if perf_a > perf_b {
                // Higher performer: professional players give credit, ambitious ones resent
                let sportsmanship_a =
                    (player_a.attributes.sportsmanship / 20.0).clamp(0.0, 1.0);
                FloatUtils::random(-0.1, 0.05) * (1.0 + sportsmanship_a * 0.3) * rep_scale
            } else {
                FloatUtils::random(-0.12, 0.08) * rep_scale
            }
        } else {
            0.0
        }
    }

    pub(super) fn calculate_personality_conflict(player_a: &Player, player_b: &Player) -> f32 {
        // High controversy players clash with professional players
        let controversy_clash = if player_a.attributes.controversy > 15.0
            && player_b.attributes.professionalism > 15.0
            || player_b.attributes.controversy > 15.0
                && player_a.attributes.professionalism > 15.0
        {
            -0.25
        } else {
            0.0
        };

        // High temperament players clash
        let temperament_clash =
            if player_a.attributes.temperament > 18.0 && player_b.attributes.temperament > 18.0 {
                FloatUtils::random(-0.15, -0.03)
            } else {
                0.0
            };

        // Different behavioral states cause friction
        let behavior_clash = match (&player_a.behaviour.state, &player_b.behaviour.state) {
            (PersonBehaviourState::Poor, PersonBehaviourState::Good)
            | (PersonBehaviourState::Good, PersonBehaviourState::Poor) => -0.12,
            _ => 0.0,
        };

        // Mutual loyalty and professionalism create bonds
        let positive_traits =
            if player_a.attributes.loyalty > 15.0 && player_b.attributes.loyalty > 15.0 {
                0.08
            } else {
                0.0
            };

        // Mutual sportsmanship creates bonds
        let sportsmanship_bond = if player_a.attributes.sportsmanship > 14.0
            && player_b.attributes.sportsmanship > 14.0
        {
            0.05
        } else {
            0.0
        };

        controversy_clash + temperament_clash + behavior_clash + positive_traits + sportsmanship_bond
    }

    pub(super) fn calculate_leadership_influence(leader: &Player, player: &Player) -> f32 {
        let leadership_strength = leader.skills.mental.leadership / 20.0;

        // Reputation amplifies leadership: respected players are listened to more
        let rep_boost =
            (leader.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let effective_leadership = leadership_strength * (1.0 + rep_boost * 0.5);

        let influence = match player.behaviour.state {
            PersonBehaviourState::Good => effective_leadership * 0.15,
            PersonBehaviourState::Normal => effective_leadership * 0.08,
            PersonBehaviourState::Poor => {
                if player.attributes.professionalism > 10.0 {
                    effective_leadership * 0.12
                } else {
                    -effective_leadership * 0.08
                }
            }
        };

        influence
    }

    pub(super) fn calculate_playing_time_jealousy(
        time_a: u16,
        time_b: u16,
        player_a: &Player,
        player_b: &Player,
    ) -> f32 {
        let time_diff = (time_a as i32 - time_b as i32).abs();

        if time_diff < 3 {
            return FloatUtils::random(0.03, 0.1);
        }

        if time_diff > 10 {
            let ambition_factor =
                (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

            // High reputation players who don't play feel it more acutely
            let rep_a =
                (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
            let rep_b =
                (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);

            if time_a < time_b && player_a.attributes.ambition > 15.0 {
                return -0.15 * ambition_factor * (1.0 + rep_a * 0.3);
            } else if time_b < time_a && player_b.attributes.ambition > 15.0 {
                return -0.15 * ambition_factor * (1.0 + rep_b * 0.3);
            }
        }

        0.0
    }
}
