//! Disciplinary effects of a finished match — yellow / red card
//! suspension bookkeeping. Lives on the player so `LeagueRegulations`
//! / `LeagueResult::process_match_events` can state the fact ("you got
//! a red, in this family of competitions") and the player decides the
//! reaction (set ban, increment season counter, etc.).
//!
//! A suspension belongs to the family it was earned in: a country's
//! domestic competitions together, or continental club competitions
//! together. It is served by missing a fixture of that family and bars
//! the player from no other.

use crate::HappinessEventType;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use serde::{Deserialize, Serialize};

/// Yellow-card accumulation threshold that triggers a 1-match
/// suspension under the standard FA / FIFA rule. After the threshold
/// fires the season counter is reset by the threshold (kept rolling
/// rather than zeroed) so subsequent yellows accumulate toward the
/// next ban without losing the most recent card.
pub const YELLOW_CARD_BAN_THRESHOLD: u8 = 5;

/// The competitions a suspension is earned and served in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CompetitionFamily {
    /// A country's leagues, cup and play-offs.
    #[default]
    Domestic,
    /// Continental club competitions.
    Continental,
}

/// One family's suspensions and running bookings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DisciplinaryBook {
    /// Matches still to be served.
    pub suspension_matches: u8,
    /// Yellow cards toward the next accumulation ban. Rolled past the
    /// threshold each time it triggers one, so later cards still count.
    pub yellow_card_running: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayerDiscipline {
    pub domestic: DisciplinaryBook,
    pub continental: DisciplinaryBook,
    /// The family of his side's next competitive fixture, as his team last
    /// told him.
    pub awaiting: CompetitionFamily,
}

impl PlayerDiscipline {
    pub fn book(&self, family: CompetitionFamily) -> &DisciplinaryBook {
        match family {
            CompetitionFamily::Domestic => &self.domestic,
            CompetitionFamily::Continental => &self.continental,
        }
    }

    fn book_mut(&mut self, family: CompetitionFamily) -> &mut DisciplinaryBook {
        match family {
            CompetitionFamily::Domestic => &mut self.domestic,
            CompetitionFamily::Continental => &mut self.continental,
        }
    }
}

impl Player {
    /// A suspension he carries keeps him out of a fixture of `family`.
    pub fn is_suspended_in(&self, family: CompetitionFamily) -> bool {
        self.player_attributes
            .discipline
            .book(family)
            .suspension_matches
            > 0
    }

    /// His team's next competitive fixture belongs to `family`. Whether he
    /// is banned for it follows from the suspensions he carries there.
    pub fn on_next_fixture_family(&mut self, family: CompetitionFamily) {
        self.player_attributes.discipline.awaiting = family;
        self.refresh_ban();
    }

    fn refresh_ban(&mut self) {
        self.player_attributes.is_banned =
            self.is_suspended_in(self.player_attributes.discipline.awaiting);
    }

    /// React to a finished match's disciplinary stats in a competition of
    /// `family`. `yellow_cards` is the number of yellows received this
    /// match (1 normally; already 0 if the second yellow was promoted to a
    /// red by the engine), `red_cards` is 1 if the player was sent off.
    ///
    /// Returns the number of additional suspension matches added — 0
    /// if the cards didn't escalate to a ban this match.
    pub fn on_match_disciplinary_result(
        &mut self,
        yellow_cards: u8,
        red_cards: u8,
        season_yellow_threshold: u8,
        family: CompetitionFamily,
    ) -> u8 {
        let book = self.player_attributes.discipline.book_mut(family);
        // Direct red or second-yellow → 1 match ban. Engine promotes a
        // second yellow to a red, so a player with red_cards>0 cannot
        // also have yellow_cards>0 in the same match — we treat the
        // red as the only contributor here.
        if red_cards > 0 {
            book.suspension_matches = book.suspension_matches.saturating_add(1);
            self.refresh_ban();
            return 1;
        }
        if yellow_cards == 0 {
            return 0;
        }

        // The running tally is apart from `PlayerStatistics::yellow_cards`
        // so the threshold logic doesn't collide with display / archive
        // figures. Promote on threshold crossings.
        let prev = book.yellow_card_running;
        let new = prev.saturating_add(yellow_cards);
        let threshold = season_yellow_threshold.max(1);
        if prev >= threshold || new < threshold {
            book.yellow_card_running = new;
            return 0;
        }
        book.suspension_matches = book.suspension_matches.saturating_add(1);
        book.yellow_card_running = new - threshold;
        self.refresh_ban();
        // The avoidable, self-inflicted ban — the red-card route is
        // narrated by RedCardFallout; this covers the bookings tally
        // tipping over.
        let mag = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::BannedThroughAccumulation);
        self.happiness
            .add_event(HappinessEventType::BannedThroughAccumulation, mag);
        1
    }

    /// Mark one suspension match of `family` as served. Called by the
    /// matchday pipeline for every suspended player whose team played a
    /// fixture of that family without him. No-op when he carries none.
    pub fn serve_suspension_match(&mut self, family: CompetitionFamily) {
        let book = self.player_attributes.discipline.book_mut(family);
        if book.suspension_matches == 0 {
            return;
        }
        book.suspension_matches -= 1;
        let served = book.suspension_matches == 0;
        self.refresh_ban();
        if served {
            // Available again — the relief beat that closes the ban
            // story the feed opened when the suspension began.
            let mag = HappinessConfig::default()
                .catalog
                .magnitude(HappinessEventType::SuspensionServed);
            self.happiness
                .add_event(HappinessEventType::SuspensionServed, mag);
        }
    }

    /// Last season's bans expire with it.
    pub fn clear_suspensions(&mut self) {
        let discipline = &mut self.player_attributes.discipline;
        discipline.domestic.suspension_matches = 0;
        discipline.continental.suspension_matches = 0;
        self.refresh_ban();
    }

    /// Reset the season yellow-card running tallies. Called at season
    /// rollover so accumulated yellows don't leak between seasons.
    /// Does not touch active suspensions — those are served regardless
    /// of whether the season turned over while they were pending.
    pub fn reset_season_disciplinary_state(&mut self) {
        let discipline = &mut self.player_attributes.discipline;
        discipline.domestic.yellow_card_running = 0;
        discipline.continental.yellow_card_running = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills};
    use chrono::NaiveDate;

    fn make_player() -> Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    #[test]
    fn red_card_sets_one_match_suspension() {
        let mut p = make_player();
        let added = p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(added, 1);
        assert!(p.player_attributes.is_banned);
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
    }

    #[test]
    fn single_yellow_does_not_trigger_ban() {
        let mut p = make_player();
        let added = p.on_match_disciplinary_result(
            1,
            0,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(added, 0);
        assert!(!p.player_attributes.is_banned);
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            0
        );
        assert_eq!(
            p.player_attributes.discipline.domestic.yellow_card_running,
            1
        );
    }

    #[test]
    fn yellow_accumulation_crosses_threshold_for_ban() {
        let mut p = make_player();
        // Pile 4 yellows — no ban yet.
        for _ in 0..4 {
            p.on_match_disciplinary_result(
                1,
                0,
                YELLOW_CARD_BAN_THRESHOLD,
                CompetitionFamily::Domestic,
            );
        }
        assert!(!p.player_attributes.is_banned);
        assert_eq!(
            p.player_attributes.discipline.domestic.yellow_card_running,
            4
        );
        // 5th yellow crosses the threshold → 1-match ban.
        let added = p.on_match_disciplinary_result(
            1,
            0,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(added, 1);
        assert!(p.player_attributes.is_banned);
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
        // Tally rolled past the threshold instead of zeroed.
        assert_eq!(
            p.player_attributes.discipline.domestic.yellow_card_running,
            0
        );
    }

    #[test]
    fn serving_match_decrements_and_clears_ban() {
        let mut p = make_player();
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match(CompetitionFamily::Domestic);
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            0
        );
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn serving_match_when_unbanned_is_a_noop() {
        let mut p = make_player();
        // Not banned to start with.
        p.serve_suspension_match(CompetitionFamily::Domestic);
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            0
        );
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn red_card_during_existing_ban_extends_it() {
        let mut p = make_player();
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
        // Player got banned and somehow got another red — extend.
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            2
        );
    }

    #[test]
    fn season_reset_clears_running_yellows_only() {
        let mut p = make_player();
        // Build up yellows AND a suspension.
        for _ in 0..4 {
            p.on_match_disciplinary_result(
                1,
                0,
                YELLOW_CARD_BAN_THRESHOLD,
                CompetitionFamily::Domestic,
            );
        }
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
        p.reset_season_disciplinary_state();
        assert_eq!(
            p.player_attributes.discipline.domestic.yellow_card_running,
            0
        );
        // Active ban survives — players carrying a ban into the new
        // season still serve it.
        assert_eq!(
            p.player_attributes.discipline.domestic.suspension_matches,
            1
        );
        assert!(p.player_attributes.is_banned);
    }

    #[test]
    fn a_red_card_in_europe_does_not_cost_a_league_match() {
        let mut p = make_player();
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Continental,
        );
        // Saturday is a league match.
        p.on_next_fixture_family(CompetitionFamily::Domestic);
        assert!(!p.player_attributes.is_banned);
        assert!(!p.is_suspended_in(CompetitionFamily::Domestic));
        // Missing a league match serves nothing.
        p.serve_suspension_match(CompetitionFamily::Domestic);
        assert!(p.is_suspended_in(CompetitionFamily::Continental));
        // The next continental tie is the one he misses.
        p.on_next_fixture_family(CompetitionFamily::Continental);
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match(CompetitionFamily::Continental);
        assert!(!p.player_attributes.is_banned);
        assert!(!p.is_suspended_in(CompetitionFamily::Continental));
    }

    #[test]
    fn a_league_red_card_is_served_in_the_cup_tie_that_follows() {
        let mut p = make_player();
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        // The cup is domestic too: he misses it, and that serves the ban.
        assert!(p.is_suspended_in(CompetitionFamily::Domestic));
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match(CompetitionFamily::Domestic);
        assert!(!p.is_suspended_in(CompetitionFamily::Domestic));
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn a_ban_in_each_family_is_served_by_its_own_next_fixture() {
        let mut p = make_player();
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Domestic,
        );
        p.on_match_disciplinary_result(
            0,
            1,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Continental,
        );
        assert!(p.is_suspended_in(CompetitionFamily::Domestic));
        assert!(p.is_suspended_in(CompetitionFamily::Continental));

        p.on_next_fixture_family(CompetitionFamily::Continental);
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match(CompetitionFamily::Continental);
        assert!(
            !p.player_attributes.is_banned,
            "the continental ban is done"
        );
        assert!(
            p.is_suspended_in(CompetitionFamily::Domestic),
            "the domestic one is not"
        );

        p.on_next_fixture_family(CompetitionFamily::Domestic);
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match(CompetitionFamily::Domestic);
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn bookings_in_one_family_do_not_count_toward_the_other() {
        let mut p = make_player();
        for _ in 0..4 {
            p.on_match_disciplinary_result(
                1,
                0,
                YELLOW_CARD_BAN_THRESHOLD,
                CompetitionFamily::Domestic,
            );
        }
        let added = p.on_match_disciplinary_result(
            1,
            0,
            YELLOW_CARD_BAN_THRESHOLD,
            CompetitionFamily::Continental,
        );
        assert_eq!(added, 0);
        assert_eq!(
            p.player_attributes.discipline.domestic.yellow_card_running,
            4
        );
        assert_eq!(
            p.player_attributes
                .discipline
                .continental
                .yellow_card_running,
            1
        );
    }
}
