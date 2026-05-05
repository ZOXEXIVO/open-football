//! Disciplinary effects of a finished match — yellow / red card
//! suspension bookkeeping. Lives on the player so `LeagueRegulations`
//! / `LeagueResult::process_match_events` can state the fact ("you got
//! a red") and the player decides the reaction (set ban, increment
//! season counter, etc.).
//!
//! Suspensions are competition-agnostic: a red in the league bans the
//! player from the next league match; we don't currently model the
//! cup/league firewall, so the running counter is shared across
//! competitions. Decrement happens in `serve_suspension_match`, which
//! the matchday-result pipeline calls for every banned player whose
//! team played without them.

use crate::club::player::player::Player;

/// Yellow-card accumulation threshold that triggers a 1-match
/// suspension under the standard FA / FIFA rule. After the threshold
/// fires the season counter is reset by the threshold (kept rolling
/// rather than zeroed) so subsequent yellows accumulate toward the
/// next ban without losing the most recent card.
pub const YELLOW_CARD_BAN_THRESHOLD: u8 = 5;

impl Player {
    /// React to a finished match's disciplinary stats. `yellow_cards`
    /// is the number of yellows received this match (1 normally;
    /// already 0 if the second yellow was promoted to a red by the
    /// engine), `red_cards` is 1 if the player was sent off.
    ///
    /// Returns the number of additional suspension matches added — 0
    /// if the cards didn't escalate to a ban this match.
    pub fn on_match_disciplinary_result(
        &mut self,
        yellow_cards: u8,
        red_cards: u8,
        season_yellow_threshold: u8,
    ) -> u8 {
        let mut added: u8 = 0;
        // Direct red or second-yellow → 1 match ban. Engine promotes a
        // second yellow to a red, so a player with red_cards>0 cannot
        // also have yellow_cards>0 in the same match — we treat the
        // red as the only contributor here.
        if red_cards > 0 {
            self.player_attributes.suspension_matches =
                self.player_attributes.suspension_matches.saturating_add(1);
            self.player_attributes.is_banned = true;
            added = added.saturating_add(1);
            return added;
        }
        if yellow_cards == 0 {
            return added;
        }

        // Bump the running season yellow tally — `PlayerStatistics::yellow_cards`
        // is updated separately from match stats; we use a parallel
        // running counter on the player attributes so the threshold
        // logic doesn't collide with display / archive figures.
        // Promote on threshold crossings.
        let prev = self.player_attributes.yellow_card_running;
        let new = prev.saturating_add(yellow_cards);
        let threshold = season_yellow_threshold.max(1);
        if prev < threshold && new >= threshold {
            self.player_attributes.suspension_matches =
                self.player_attributes.suspension_matches.saturating_add(1);
            self.player_attributes.is_banned = true;
            added = added.saturating_add(1);
            // Roll the running tally past the threshold so subsequent
            // yellows continue to accumulate naturally.
            self.player_attributes.yellow_card_running = new - threshold;
        } else {
            self.player_attributes.yellow_card_running = new;
        }
        added
    }

    /// Mark one suspension match as served. Called by the matchday
    /// pipeline for every banned player whose team played a fixture
    /// they did not appear in. Clears `is_banned` when the counter
    /// reaches zero. No-op for players who aren't currently banned.
    pub fn serve_suspension_match(&mut self) {
        if self.player_attributes.suspension_matches == 0 {
            self.player_attributes.is_banned = false;
            return;
        }
        self.player_attributes.suspension_matches -= 1;
        if self.player_attributes.suspension_matches == 0 {
            self.player_attributes.is_banned = false;
        }
    }

    /// Reset the season yellow-card running tally. Called at season
    /// rollover so accumulated yellows don't leak between seasons.
    /// Does not touch active suspensions — those are served regardless
    /// of whether the season turned over while they were pending.
    pub fn reset_season_disciplinary_state(&mut self) {
        self.player_attributes.yellow_card_running = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills,
    };
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
        let added = p.on_match_disciplinary_result(0, 1, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(added, 1);
        assert!(p.player_attributes.is_banned);
        assert_eq!(p.player_attributes.suspension_matches, 1);
    }

    #[test]
    fn single_yellow_does_not_trigger_ban() {
        let mut p = make_player();
        let added = p.on_match_disciplinary_result(1, 0, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(added, 0);
        assert!(!p.player_attributes.is_banned);
        assert_eq!(p.player_attributes.suspension_matches, 0);
        assert_eq!(p.player_attributes.yellow_card_running, 1);
    }

    #[test]
    fn yellow_accumulation_crosses_threshold_for_ban() {
        let mut p = make_player();
        // Pile 4 yellows — no ban yet.
        for _ in 0..4 {
            p.on_match_disciplinary_result(1, 0, YELLOW_CARD_BAN_THRESHOLD);
        }
        assert!(!p.player_attributes.is_banned);
        assert_eq!(p.player_attributes.yellow_card_running, 4);
        // 5th yellow crosses the threshold → 1-match ban.
        let added = p.on_match_disciplinary_result(1, 0, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(added, 1);
        assert!(p.player_attributes.is_banned);
        assert_eq!(p.player_attributes.suspension_matches, 1);
        // Tally rolled past the threshold instead of zeroed.
        assert_eq!(p.player_attributes.yellow_card_running, 0);
    }

    #[test]
    fn serving_match_decrements_and_clears_ban() {
        let mut p = make_player();
        p.on_match_disciplinary_result(0, 1, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(p.player_attributes.suspension_matches, 1);
        assert!(p.player_attributes.is_banned);
        p.serve_suspension_match();
        assert_eq!(p.player_attributes.suspension_matches, 0);
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn serving_match_when_unbanned_is_a_noop() {
        let mut p = make_player();
        // Not banned to start with.
        p.serve_suspension_match();
        assert_eq!(p.player_attributes.suspension_matches, 0);
        assert!(!p.player_attributes.is_banned);
    }

    #[test]
    fn red_card_during_existing_ban_extends_it() {
        let mut p = make_player();
        p.on_match_disciplinary_result(0, 1, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(p.player_attributes.suspension_matches, 1);
        // Player got banned and somehow got another red — extend.
        p.on_match_disciplinary_result(0, 1, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(p.player_attributes.suspension_matches, 2);
    }

    #[test]
    fn season_reset_clears_running_yellows_only() {
        let mut p = make_player();
        // Build up yellows AND a suspension.
        for _ in 0..4 {
            p.on_match_disciplinary_result(1, 0, YELLOW_CARD_BAN_THRESHOLD);
        }
        p.on_match_disciplinary_result(0, 1, YELLOW_CARD_BAN_THRESHOLD);
        assert_eq!(p.player_attributes.suspension_matches, 1);
        p.reset_season_disciplinary_state();
        assert_eq!(p.player_attributes.yellow_card_running, 0);
        // Active ban survives — players carrying a ban into the new
        // season still serve it.
        assert_eq!(p.player_attributes.suspension_matches, 1);
        assert!(p.player_attributes.is_banned);
    }
}
