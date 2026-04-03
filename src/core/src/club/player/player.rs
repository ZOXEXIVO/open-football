use crate::club::player::utils::PlayerUtils;
use crate::club::{
    PersonBehaviour, PlayerAttributes, PlayerClubContract, PlayerMailbox,
    PlayerResult, PlayerSkills, PlayerTraining,
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{
    Person, PersonAttributes, PlayerDecisionHistory, PlayerHappiness,
    PlayerPositionType, PlayerPositions,
    PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerTrainingHistory, PlayerValueCalculator, Relations,
};
use chrono::NaiveDate;
use std::fmt::{Display, Formatter, Result};
use crate::club::player::builder::PlayerBuilder;

#[derive(Debug, Clone)]
pub struct Player {
    //person data
    pub id: u32,
    pub full_name: FullName,
    pub birth_date: NaiveDate,
    pub country_id: u32,
    pub behaviour: PersonBehaviour,
    pub attributes: PersonAttributes,

    //player data
    pub happiness: PlayerHappiness,
    pub statuses: PlayerStatus,
    pub skills: PlayerSkills,
    pub contract: Option<PlayerClubContract>,
    pub contract_loan: Option<PlayerClubContract>,
    pub positions: PlayerPositions,
    pub preferred_foot: PlayerPreferredFoot,
    pub player_attributes: PlayerAttributes,
    pub mailbox: PlayerMailbox,
    pub training: PlayerTraining,
    pub training_history: PlayerTrainingHistory,
    pub relations: Relations,

    pub statistics: PlayerStatistics,
    pub friendly_statistics: PlayerStatistics,
    pub cup_statistics: PlayerStatistics,
    pub statistics_history: PlayerStatisticsHistory,
    pub decision_history: PlayerDecisionHistory,

    /// Languages the player speaks, with proficiency levels.
    pub languages: Vec<crate::club::player::language::PlayerLanguage>,

    /// Set when a player transfers/loans to a new club. Used by season snapshot
    /// to detect recently transferred players and avoid phantom history entries.
    pub last_transfer_date: Option<NaiveDate>,

    /// The club's strategic intent for this signing.
    /// Set when a player is permanently transferred. Protects the player from
    /// being sold before the club has given them a fair evaluation.
    pub plan: Option<crate::club::player::plan::PlayerPlan>,

    /// Clubs this player supports or has an affinity for (like FM's "Favoured Clubs").
    /// Affects willingness to join, morale when playing for them, etc.
    pub favorite_clubs: Vec<u32>,

    /// The club that sold this player and the fee paid.
    /// Prevents unrealistic buy-back scenarios where a club sells a player
    /// cheaply and then buys them back at a huge markup one season later.
    pub sold_from: Option<(u32, f64)>, // (club_id, fee_paid)
}

impl Player {
    pub fn builder() -> PlayerBuilder {
        PlayerBuilder::new()
    }

    /// Is this player protected from being targeted by other clubs?
    /// A player is protected if they were recently signed (< 120 days) or
    /// their club has an active signing plan that hasn't been evaluated yet.
    /// This prevents unrealistic transfer chains where a player bounces
    /// between multiple clubs in the same season.
    pub fn is_transfer_protected(&self, date: NaiveDate) -> bool {
        // Recently transferred — settling-in period
        if let Some(transfer_date) = self.last_transfer_date {
            let days_since = (date - transfer_date).num_days();
            if days_since < 120 {
                return true;
            }
        }

        // Club has a signing plan — don't poach until the plan concludes
        if let Some(ref plan) = self.plan {
            let total_apps = self.statistics.played + self.statistics.played_subs;
            if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date) {
                return true;
            }
        }

        false
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> PlayerResult {
        let now = ctx.simulation.date;

        let mut result = PlayerResult::new(self.id);

        // Birthday
        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
        }

        // Injury recovery (daily)
        self.process_injury(&mut result, now.date());

        // Natural condition recovery for non-injured players
        self.process_condition_recovery(now.date());

        // Match readiness decay for players not playing
        self.process_match_readiness_decay();

        // Player happiness & morale evaluation (weekly)
        let team_reputation = ctx.team.as_ref().map(|t| t.reputation).unwrap_or(0.0);
        if ctx.simulation.is_week_beginning() {
            self.process_happiness(&mut result, now.date(), team_reputation);
            // Natural skill development (weekly)
            let league_reputation = ctx.league.as_ref().map(|l| l.reputation).unwrap_or(0);
            self.process_development(now.date(), league_reputation);
            // Language learning when playing abroad
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            self.process_language_learning(now.date(), country_code);
        }

        // Contract processing
        self.process_contract(&mut result, now);
        self.process_mailbox(&mut result, now.date());

        // Transfer desire based on multiple factors
        self.process_transfer_desire(&mut result, now.date());

        result
    }

    pub fn shirt_number(&self) -> u8 {
        if let Some(contract) = &self.contract {
            return contract.shirt_number.unwrap_or(0);
        }

        0
    }

    pub fn value(&self, date: NaiveDate, league_reputation: u16, club_reputation: u16) -> f64 {
        PlayerValueCalculator::calculate(self, date, 1.0, league_reputation, club_reputation)
    }

    pub fn value_with_price_level(&self, date: NaiveDate, price_level: f32, league_reputation: u16, club_reputation: u16) -> f64 {
        PlayerValueCalculator::calculate(self, date, price_level, league_reputation, club_reputation)
    }

    #[inline]
    pub fn positions(&self) -> Vec<PlayerPositionType> {
        self.positions.positions()
    }

    #[inline]
    pub fn position(&self) -> PlayerPositionType {
        *self
            .positions
            .positions()
            .first()
            .expect("no position found")
    }

    pub fn preferred_foot_str(&self) -> &'static str {
        match self.preferred_foot {
            PlayerPreferredFoot::Left => "Left",
            PlayerPreferredFoot::Right => "Right",
            PlayerPreferredFoot::Both => "Both",
        }
    }

    pub fn is_on_loan(&self) -> bool {
        self.contract_loan.is_some()
    }

    pub fn is_ready_for_match(&self) -> bool {
        !self.player_attributes.is_injured
            && !self.player_attributes.is_banned
            && !self.player_attributes.is_in_recovery()
            && self.player_attributes.condition_percentage() > 30
    }

    pub fn growth_potential(&self, now: NaiveDate) -> u8 {
        PlayerUtils::growth_potential(self, now)
    }

    /// Weekly language learning: if the player is in a country whose language
    /// they don't speak natively, they gradually learn it.
    fn process_language_learning(&mut self, now: NaiveDate, country_code: &str) {
        use crate::club::player::language::{Language, PlayerLanguage, weekly_language_progress};

        if country_code.is_empty() {
            return;
        }

        let country_languages = Language::from_country_code(country_code);
        if country_languages.is_empty() {
            return;
        }

        let age = crate::utils::DateUtils::age(self.birth_date, now);

        for target_lang in &country_languages {
            // Check if player already speaks this language natively
            let already_native = self.languages.iter().any(|l| l.language == *target_lang && l.is_native);
            if already_native {
                continue;
            }

            // Check if already fully fluent (proficiency >= 100)
            let already_fluent = self.languages.iter().any(|l| l.language == *target_lang && l.proficiency >= 100);
            if already_fluent {
                continue;
            }

            let current_prof = self.languages.iter()
                .find(|l| l.language == *target_lang)
                .map(|l| l.proficiency)
                .unwrap_or(0);

            let gain = weekly_language_progress(
                self.attributes.adaptability,
                self.attributes.professionalism,
                age,
                self.player_attributes.current_ability,
                current_prof,
            );

            if gain == 0 {
                continue;
            }

            if let Some(lang_entry) = self.languages.iter_mut().find(|l| l.language == *target_lang) {
                lang_entry.proficiency = (lang_entry.proficiency + gain).min(100);
            } else {
                self.languages.push(PlayerLanguage::learning(*target_lang, gain));
            }
        }
    }
}

impl Person for Player {
    fn id(&self) -> u32 {
        self.id
    }

    fn fullname(&self) -> &FullName {
        &self.full_name
    }

    fn birthday(&self) -> NaiveDate {
        self.birth_date
    }

    fn behaviour(&self) -> &PersonBehaviour {
        &self.behaviour
    }

    fn attributes(&self) -> &PersonAttributes {
        &self.attributes
    }

    fn relations(&self) -> &Relations {
        &self.relations
    }
}

#[derive(Debug, Clone)]
pub enum PlayerPreferredFoot {
    Left,
    Right,
    Both,
}

//DISPLAY
impl Display for Player {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}, {}", self.full_name, self.birth_date)
    }
}

impl PartialEq for Player {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn player_is_correct() {
        assert_eq!(10, 10);
    }
}
