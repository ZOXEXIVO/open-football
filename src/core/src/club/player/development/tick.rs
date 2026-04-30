//! Weekly development tick: the public entry point that wires every
//! component (age curve, position weights, modifiers, coaching, rolls)
//! into a single per-skill update for one player.

use super::age_curve::*;
use super::coaching::CoachingEffect;
use super::modifiers::*;
use super::position_weights::*;
use super::rolls::{RollSource, ThreadRolls};
use super::skills_array::*;

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::NaiveDate;

impl Player {
    /// Weekly development tick. See module docs for the model.
    ///
    /// Routes through the deterministic roll seam under the hood, using
    /// the thread-local RNG. Tests should call
    /// [`Player::process_development_with`] with a deterministic source.
    pub fn process_development(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
    ) {
        self.process_development_with(
            now,
            league_reputation,
            coach,
            club_rep_0_to_1,
            &mut ThreadRolls,
        );
    }

    /// Same as [`Player::process_development`] but the per-skill rolls
    /// come from `rolls`. This is the testable seam — pin the rolls to a
    /// known value and the output becomes a pure function of the inputs.
    pub fn process_development_with(
        &mut self,
        now: NaiveDate,
        league_reputation: u16,
        coach: &CoachingEffect,
        club_rep_0_to_1: f32,
        rolls: &mut impl RollSource,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let pa = self.player_attributes.potential_ability as f32;

        // Body state gates everything else.
        let fitness = if self.player_attributes.is_injured {
            FitnessState::Injured
        } else if self.player_attributes.is_in_recovery() {
            FitnessState::Recovering
        } else {
            FitnessState::Fit
        };

        // Injured players don't develop. Their skills are frozen until they
        // come back — no growth, no decline. The CA recalculation is also
        // skipped because the underlying skills haven't moved.
        if fitness == FitnessState::Injured {
            return;
        }

        let pos = self.position();
        let pos_group = pos_group_from(pos);
        let dev_weights = position_dev_weights(pos_group);

        // Base ceiling from PA (PA 200 -> ceiling 20.0)
        let base_ceiling = (pa / 200.0 * 20.0).clamp(1.0, 20.0);

        // ── Compute shared multipliers ────────────────────────────────

        let personality = personality_multiplier(
            self.attributes.professionalism,
            self.attributes.ambition,
            self.skills.mental.determination,
            self.skills.mental.work_rate,
        );

        let official_games = self.statistics.total_games() + self.cup_statistics.total_games();
        let friendly_games = self.friendly_statistics.total_games();

        let match_exp = match_experience_multiplier(
            self.statistics.played + self.cup_statistics.played,
            self.statistics.played_subs + self.cup_statistics.played_subs,
            self.friendly_statistics.played,
            self.friendly_statistics.played_subs,
        );

        let official_bonus = official_match_bonus(official_games, friendly_games);

        let rating_mult = rating_multiplier(self.statistics.average_rating, official_games);

        let decline_prot = decline_protection(
            self.skills.physical.natural_fitness,
            self.attributes.professionalism,
        );

        let comp_quality = competition_quality_multiplier(league_reputation);

        // Extra boost while the player catches up to a clearly better club.
        let step_up_mult = self.step_up_development_multiplier(now, club_rep_0_to_1);

        // Workload / fitness / readiness modifiers.
        let condition_pct = self.player_attributes.condition_percentage();
        let jadedness = self.player_attributes.jadedness;
        let workload_growth = workload_growth_modifier(condition_pct, jadedness);
        let workload_decline = workload_decline_amplifier(condition_pct, jadedness);
        let readiness_mult = match_readiness_multiplier(self.skills.physical.match_readiness);

        // Recovering from an injury: the body is healing, not adapting.
        // Mental skills (study video, learn the playbook) can still nudge
        // forward at a reduced rate; everything else is frozen.
        let recovering = fitness == FitnessState::Recovering;

        // ── Process each skill ────────────────────────────────────────

        let mut skills = skills_to_array(self);

        for i in 0..SKILL_COUNT {
            if i == SK_MATCH_READINESS {
                continue; // managed by training/match system
            }

            let cat = skill_category(i);

            if recovering && cat != SkillCategory::Mental {
                continue;
            }

            let peak_offset = individual_peak_offset(i);
            let effective_age = (age as i16 - peak_offset as i16).clamp(14, 45) as u8;

            // Per-skill ceiling: position weight determines how high this skill can go.
            let skill_ceiling = (base_ceiling * dev_weights[i]).clamp(1.0, 20.0);

            // Per-skill gap factor (replaces global PA-CA gap).
            let gap = skill_gap_factor(skills[i], skill_ceiling);

            // Base rate from age curve.
            let (min_rate, max_rate) = base_weekly_rate(effective_age, cat);
            let roll = rolls.roll_unit().clamp(0.0, 1.0);
            let base = min_rate + roll * (max_rate - min_rate);

            // Position weight scales growth rate: key skills develop faster.
            let pos_rate_mult = dev_weights[i];

            // Coach effectiveness by category, plus a youth bonus for
            // players under 23 (using Head of Youth Development attribute).
            let coach_mult = coach.for_category(cat);
            let youth_coach_mult = if age < 23 { coach.youth_bonus } else { 1.0 };

            let change = if base > 0.0 {
                // Growth: scale by all positive multipliers + position relevance + competition quality
                base * personality
                    * match_exp
                    * official_bonus
                    * rating_mult
                    * gap
                    * pos_rate_mult
                    * comp_quality
                    * coach_mult
                    * youth_coach_mult
                    * step_up_mult
                    * workload_growth
                    * readiness_mult
            } else {
                // Decline: position-irrelevant skills decline slightly faster;
                // key skills are more "maintained" by regular use. Great
                // coaches slow decline a little (load + technique management).
                // Workload amplifier accelerates decline for chronically tired
                // players.
                let decline_pos_mult = (2.0 - dev_weights[i]).clamp(0.5, 1.5);
                let decline_coach_protection = ((coach_mult - 1.0) * 0.5 + 1.0).clamp(0.6, 1.0);
                base * decline_prot * decline_pos_mult * decline_coach_protection * workload_decline
            };

            let new_val = skills[i] + change;

            skills[i] = if change > 0.0 {
                new_val.min(skill_ceiling).clamp(1.0, 20.0)
            } else {
                new_val.clamp(1.0, 20.0)
            };
        }

        write_skills_back(self, &skills);

        // ── Recalculate current_ability from updated skills ───────────

        let position = self.position();
        self.player_attributes.current_ability =
            self.skills.calculate_ability_for_position(position);

        // PA must never be lower than CA. Generation can occasionally produce
        // CA > PA, which would otherwise crush all per-skill ceilings.
        if self.player_attributes.potential_ability < self.player_attributes.current_ability {
            self.player_attributes.potential_ability = self.player_attributes.current_ability;
        }
    }
}
