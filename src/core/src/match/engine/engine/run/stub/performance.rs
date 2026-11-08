//! How a man plays on the day, read before anything is drawn from it.
//!
//! One number in log units of output, so every volume on his line and his
//! share of the side's goals move together, and a good or bad day shows in
//! what he did. The rating never sees it.

use crate::PlayerSkills;
use crate::club::player::personality::MatchdayForm;
use crate::r#match::engine::psychology::Psychology;
use crate::r#match::{MatchPlayer, MatchSquad};
use crate::utils::DateUtils;
use chrono::{Datelike, NaiveDate};

/// What everyone in one fixture shares.
pub(super) struct StubFixture {
    pub standard: f32,
    pub occasion: f32,
    pub today: NaiveDate,
}

pub(super) struct StubPerformance;

impl StubPerformance {
    /// Log output per point of current ability over the opposition's mean.
    /// Summed over an eleven it is the goal-rate slope: forty points, a
    /// title side against a relegation one, is roughly 2.3 goals to 0.8.
    const ABILITY_SLOPE: f32 = 0.013;
    /// Kickoff confidence less nerves, in log output. The engine's own
    /// reading of his state of mind runs ±0.5, so a man who believes in
    /// himself plays about a tenth above his level.
    const MIND_WEIGHT: f32 = 0.20;
    /// A season's deviation against a day's form, from the same spread.
    /// Forty matches of a day's noise average out, so a season has to move
    /// more than a single afternoon for two equal men to read differently.
    const SEASON_WEIGHT: f32 = 2.5;
    /// Log output per skill point above the level his ability implies.
    /// Skills say what kind of player he is; how much he produces against
    /// this opposition is the ability gap's to say.
    const SKILL_SHIFT: f32 = 0.05;

    pub(super) fn read(
        player: &MatchPlayer,
        opposition_ability: f32,
        fixture: &StubFixture,
    ) -> f32 {
        let ability = player.player_attributes.current_ability as f32;
        let quality = Self::ABILITY_SLOPE * (ability - opposition_ability);
        let day = (player.matchday_form - 1.0) + (player.settledness - 1.0);
        let state = Psychology::kickoff_state(player, fixture.standard, fixture.occasion, 0);
        let mind = Self::MIND_WEIGHT * (state.confidence - state.nervousness);
        quality + day + mind + Self::season_deviation(player, fixture.today)
    }

    /// Mean performance of a starting eleven: the ability gap's slope
    /// survives the average, so a side's goal rate keeps its calibration.
    pub(super) fn of_eleven(
        squad: &MatchSquad,
        opposition_ability: f32,
        fixture: &StubFixture,
    ) -> f32 {
        if squad.main_squad.is_empty() {
            return 0.0;
        }
        squad
            .main_squad
            .iter()
            .map(|p| Self::read(p, opposition_ability, fixture))
            .sum::<f32>()
            / squad.main_squad.len() as f32
    }

    /// How far his whole season runs from his ability. Held by nobody:
    /// the same man in the same season always reads the same.
    pub(super) fn season_deviation(player: &MatchPlayer, today: NaiveDate) -> f32 {
        let spread = MatchdayForm::spread(
            Some(DateUtils::age(player.birth_date, today) as f32),
            player.attributes.consistency,
            player.attributes.professionalism,
            player.skills.mental.concentration,
            player.skills.mental.composure,
        );
        let season_start = if today.month() < 7 {
            today.year() - 1
        } else {
            today.year()
        };
        Self::SEASON_WEIGHT * spread * MatchdayForm::season_draw(player.id as u64, season_start)
    }

    /// Log output a skill adds to what it moves, against his own level.
    pub(super) fn skill_shift(player: &MatchPlayer, skill: f32) -> f32 {
        Self::SKILL_SHIFT * Self::skill_gap(player, skill)
    }

    pub(super) fn skill_gap(player: &MatchPlayer, skill: f32) -> f32 {
        skill - PlayerSkills::ability_skill_level(player.player_attributes.current_ability)
    }
}
