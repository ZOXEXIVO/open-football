//! What a player carries for his own development.

use super::blueprint::SkillBlueprint;
use super::ceilings::PositionalSkillCeilings;
use super::football::BankedFootball;
use super::layoff::Layoff;
use super::ledger::GrowthLedger;
use super::skills_array::SKILL_COUNT;
use super::timing::MaturationTiming;
use crate::club::player::maturation::{MaturationGroup, SkillMaturation};
use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::NaiveDate;

#[derive(Debug, Clone, Default)]
pub struct PlayerDevelopment {
    pub timing: MaturationTiming,
    pub blueprint: SkillBlueprint,
    pub football: BankedFootball,
    pub layoff: Layoff,
    pub ledger: GrowthLedger,
}

impl PlayerDevelopment {
    /// His age as his development reads it: his own pace of maturing
    /// applied to his fractional age.
    pub fn age(&self, age: f32) -> f32 {
        self.timing.age(age)
    }

    /// How readily work becomes ability in `group` at `age`.
    pub fn plasticity(&self, age: f32, group: MaturationGroup) -> f32 {
        SkillMaturation::plasticity(self.age(age), group)
    }

    /// The shape he is growing into, tilted toward what his individual plan
    /// works on. A player whose blueprint was never drawn (one built outside
    /// a world or a generator) has the shape of his profile as it stands.
    pub fn weights(&self, player: &Player) -> [f32; SKILL_COUNT] {
        let weights = if self.blueprint.is_drawn() {
            self.blueprint.weights()
        } else {
            SkillBlueprint::project(&player.skills_to_array(), &[1.0; MaturationGroup::COUNT])
                .weights()
        };
        match &player.individual_training {
            Some(plan) => SkillBlueprint::tilted(weights, &plan.focus_areas),
            None => weights,
        }
    }
}

impl Player {
    /// A player created during play: a drawn timing, and a blueprint from
    /// the profile he was generated with.
    pub fn begin_development(&mut self, today: NaiveDate) {
        self.development.timing = MaturationTiming::draw();
        self.draw_blueprint(today);
    }

    /// A world opens with this player in it. A generated player draws his
    /// timing; a recorded one takes the timing that puts his recorded
    /// ability on his own curve. His blueprint is drawn after both.
    pub fn seed_development(&mut self, opening: NaiveDate) {
        self.development.timing = if self.generated {
            MaturationTiming::draw()
        } else {
            let potential = self.player_attributes.potential_ability.max(1) as f32;
            MaturationTiming::of_record(
                DateUtils::age_in_years(self.birth_date, opening),
                self.player_attributes.current_ability as f32 / potential,
                self.position(),
            )
        };
        self.draw_blueprint(opening);
    }

    /// Draw his blueprint from his profile as it stands today.
    pub(crate) fn draw_blueprint(&mut self, today: NaiveDate) {
        let age = DateUtils::age_in_years(self.birth_date, today);
        self.development.blueprint = SkillBlueprint::project(
            &self.skills_to_array(),
            &PositionalSkillCeilings::held_shares(self, age),
        );
    }
}
