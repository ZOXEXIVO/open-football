//! The weekly tick: what time does to a player without anybody's work — a
//! growing body matures, an ageing one gives back, an idle one loses its
//! edge — and the ledger and the ability mark that read him.

use super::blueprint::SkillBlueprint;
use super::ceilings::PositionalSkillCeilings;
use super::layoff::Layoff;
use super::ledger::GrowthSource;
use super::skills_array::{SKILL_COUNT, skills_to_array};
use crate::club::player::maturation::{MaturationGroup, SkillMaturation};
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::{Datelike, NaiveDate};

impl Player {
    const WEEK: f32 = 1.0 / 52.0;
    /// Share of a growing body's maturing that reaches its strength and
    /// speed without any work on them.
    const MATURATION: f32 = 0.6;
    /// Natural fitness and professionalism over 40 at the database's medians.
    const MEDIAN_CARE: f32 = 0.625;

    /// A week passed — for every player, fit, injured or without a club.
    pub fn process_development(&mut self, now: NaiveDate) {
        self.development.ledger.roll(now);
        let age = DateUtils::age_in_years(self.birth_date, now);
        let worked = self.development.layoff.close_week();
        if !self.development.layoff.laid_off() {
            self.mature(age);
        }
        self.age_a_week(age);
        self.idle_or_repay(worked);
        self.refresh_ability_marker(now);
    }

    /// The physical and explosive families of a body still growing rise with
    /// its maturity, each attribute by the share of its height its family
    /// gained this week, up to its ceiling.
    fn mature(&mut self, age: f32) {
        let held_now = PositionalSkillCeilings::unplayed_shares(self, age);
        let held_next = PositionalSkillCeilings::unplayed_shares(self, age + Self::WEEK);
        let ceilings = PositionalSkillCeilings::without_football(self, age + Self::WEEK);
        let targets = SkillBlueprint::targets(
            &self.development.weights(self),
            self.position(),
            self.player_attributes.potential_ability,
        );
        let before = skills_to_array(self);
        let mut after = before;
        for i in 0..SKILL_COUNT {
            let family = PositionalSkillCeilings::maturation_group(i);
            if before[i] <= 0.0
                || PositionalSkillCeilings::is_condition(i)
                || !matches!(
                    family,
                    MaturationGroup::Physical | MaturationGroup::Explosive
                )
            {
                continue;
            }
            let f = family as usize;
            let growth = held_next[f] / held_now[f].max(0.05) - 1.0;
            if growth <= 0.0 || before[i] >= ceilings.at(i) {
                continue;
            }
            let floor = SkillBlueprint::ZERO.min(targets[i] * 0.5);
            let rise = (before[i] - floor).max(0.0) * growth * Self::MATURATION;
            after[i] = (before[i] + rise).min(ceilings.at(i));
        }
        self.settle(before, after, GrowthSource::Maturation);
    }

    /// How much of what ageing takes he still gives: natural fitness and
    /// professionalism above the median slow it, below it speed it up.
    pub(super) fn ageing_keep(&self) -> f32 {
        let care = (self.skills.physical.natural_fitness + self.attributes.professionalism) / 40.0;
        (SkillMaturation::MEDIAN_KEEP - (care - Self::MEDIAN_CARE)).clamp(0.3, 1.0)
    }

    /// Each attribute above the scale's zero loses the share of its height
    /// his body no longer sustains this week, slowed by natural fitness and
    /// professionalism. Position does not enter.
    fn age_a_week(&mut self, age: f32) {
        let maturity_age = self.development.age(age);
        let keep = self.ageing_keep();
        let before = skills_to_array(self);
        let mut after = before;
        for i in 0..SKILL_COUNT {
            if before[i] <= SkillBlueprint::ZERO || PositionalSkillCeilings::is_condition(i) {
                continue;
            }
            let now = SkillMaturation::retention(maturity_age, i);
            let next = SkillMaturation::retention(maturity_age + Self::WEEK, i);
            if next >= now {
                continue;
            }
            let rate = 1.0 - next / now;
            after[i] =
                SkillBlueprint::ZERO + (before[i] - SkillBlueprint::ZERO) * (1.0 - rate * keep);
        }
        self.settle(before, after, GrowthSource::Ageing);
    }

    /// A body without work gives back a little of its physical and explosive
    /// height each week once laid off; a week of work repays what was given
    /// back first, at several times the rate it was built.
    fn idle_or_repay(&mut self, worked: bool) {
        let before = skills_to_array(self);
        let mut after = before;
        for family in [MaturationGroup::Physical, MaturationGroup::Explosive] {
            let given = self.development.layoff.given_back(family);
            let change = if worked {
                -given.min(Layoff::REPAY)
            } else if self.development.layoff.laid_off() {
                Layoff::STEP.min(Layoff::MAX_GIVEN - given)
            } else {
                0.0
            };
            if change == 0.0 {
                continue;
            }
            let scale = (1.0 - given - change) / (1.0 - given);
            for i in 0..SKILL_COUNT {
                if before[i] > SkillBlueprint::ZERO
                    && !PositionalSkillCeilings::is_condition(i)
                    && PositionalSkillCeilings::maturation_group(i) == family
                {
                    after[i] = SkillBlueprint::ZERO + (before[i] - SkillBlueprint::ZERO) * scale;
                }
            }
            self.development.layoff.record(family, change);
        }
        self.settle(before, after, GrowthSource::Layoff);
    }

    /// How long a development mark stands before it is replaced.
    ///
    /// Roughly a quarter of a season. The question the mark exists to
    /// answer — has this player got better lately — is one that only
    /// resolves over months; a baseline that moved every week would
    /// answer it over a week and therefore never answer it at all.
    const ABILITY_MARK_DAYS: i64 = 84;

    /// Lay down, or refresh, the ability mark this player's progress is
    /// measured against.
    ///
    /// The first call on a player simply records where he is — a
    /// baseline cannot be a comparison — so nothing is reported about
    /// anybody until a full window has passed under it.
    fn refresh_ability_marker(&mut self, now: NaiveDate) {
        let today = now.num_days_from_ce();
        let marked_on = self.player_attributes.ability_marked_on_day;
        let stale = marked_on == 0 || (today - marked_on) as i64 >= Self::ABILITY_MARK_DAYS;
        if stale {
            self.player_attributes.ability_marker = self.player_attributes.current_ability;
            self.player_attributes.ability_marked_on_day = today;
        }
    }
}
