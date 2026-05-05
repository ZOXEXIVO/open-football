use crate::Club;
use crate::club::player::events::discipline::YELLOW_CARD_BAN_THRESHOLD;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use chrono::{Duration, NaiveDate};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LeagueRegulations {
    /// player_id → matches still to serve. Mirrors the same counter on
    /// `Player.player_attributes.suspension_matches`; the league copy is
    /// kept for league-level analytics and to surface a "currently
    /// suspended" view without walking every club.
    pub suspended_players: HashMap<u32, u8>,
    /// player_id → running yellow-card tally toward the next ban. Reset
    /// each time the threshold triggers a suspension.
    pub yellow_card_accumulation: HashMap<u32, u8>,
    /// FFP cases in flight — opened on a deficit threshold breach,
    /// resolved on `hearing_date`. One sanction per case.
    pub ffp_cases: Vec<FFPCase>,
    /// Concluded FFP cases. Kept around for the financial-history page.
    pub ffp_history: Vec<FFPCase>,
    pub pending_cases: Vec<DisciplinaryCase>,
    /// Yellow-card threshold for an accumulation ban. Default follows the
    /// FA / FIFA five-yellows rule but the field is configurable per
    /// league (some leagues use 4, some 6, some only count first-half-
    /// of-season yellows).
    pub yellow_card_ban_threshold: u8,
    /// Configurable FFP thresholds. UEFA's actual rules are far more
    /// nuanced; this is a tractable approximation tuned to the
    /// simulator's revenue scale. See `FFPThresholds::default()` for
    /// the warning / sanction bands.
    pub ffp_thresholds: FFPThresholds,
}

#[derive(Debug, Clone)]
pub struct FFPThresholds {
    /// Rolling deficit (annual) above which a club is warned but not
    /// sanctioned.
    pub warning: i64,
    /// Above this, a fine sanction is appropriate.
    pub fine: i64,
    /// Above this, a points deduction.
    pub points_deduction: i64,
    /// Above this, a transfer ban.
    pub transfer_ban: i64,
    /// Days from violation detection to the hearing where the sanction
    /// is applied. Mirrors UEFA's "case is opened, club has 30 days
    /// to respond" cadence.
    pub hearing_offset_days: i64,
    /// Days after a case concludes before the same club can be
    /// re-investigated under a new case. Stops the rolling deficit
    /// from immediately re-triggering a duplicate sanction tick.
    pub cooldown_days: i64,
}

impl Default for FFPThresholds {
    fn default() -> Self {
        FFPThresholds {
            warning: 30_000_000,
            fine: 60_000_000,
            points_deduction: 100_000_000,
            transfer_ban: 200_000_000,
            hearing_offset_days: 30,
            cooldown_days: 365,
        }
    }
}

/// New disciplinary effects produced by processing a single match.
/// Returned by `process_disciplinary_actions` so the caller can apply
/// the per-player suspension once it has mutable access to clubs.
#[derive(Debug, Clone, Default)]
pub struct DisciplinaryActions {
    /// Players newly banned this match — each entry is the count of
    /// extra suspension matches added by this match. Includes both
    /// direct red recipients and yellow-accumulation crossings.
    pub new_suspensions: Vec<(u32, u8)>,
}

impl LeagueRegulations {
    pub fn new() -> Self {
        LeagueRegulations {
            suspended_players: HashMap::new(),
            yellow_card_accumulation: HashMap::new(),
            ffp_cases: Vec::new(),
            ffp_history: Vec::new(),
            pending_cases: Vec::new(),
            yellow_card_ban_threshold: YELLOW_CARD_BAN_THRESHOLD,
            ffp_thresholds: FFPThresholds::default(),
        }
    }

    /// Walk a finished match's player_stats and update the league's
    /// disciplinary tracking. Direct reds (and second yellows promoted
    /// to reds by the engine) trigger a 1-match ban. Single yellows
    /// accumulate toward `yellow_card_ban_threshold`; crossing it
    /// triggers a 1-match ban and rolls the counter past the threshold.
    /// Returns the suspensions to apply to players.
    pub fn process_disciplinary_actions(
        &mut self,
        result: &MatchResult,
    ) -> DisciplinaryActions {
        let mut actions = DisciplinaryActions::default();
        let Some(details) = result.details.as_ref() else {
            return actions;
        };

        for (pid, stats) in &details.player_stats {
            let pid = *pid;
            // The engine promotes a second yellow into a red, so a
            // player flagged with `red_cards > 0` should never also
            // carry `yellow_cards > 0` in the same match. Treat the
            // red as the only contributor.
            if stats.red_cards > 0 {
                let entry = self.suspended_players.entry(pid).or_insert(0);
                *entry = entry.saturating_add(1);
                actions.new_suspensions.push((pid, 1));
                continue;
            }
            if stats.yellow_cards == 0 {
                continue;
            }
            let prev = *self.yellow_card_accumulation.get(&pid).unwrap_or(&0);
            let new = prev.saturating_add(stats.yellow_cards as u8);
            let threshold = self.yellow_card_ban_threshold.max(1);
            if prev < threshold && new >= threshold {
                self.yellow_card_accumulation.insert(pid, new - threshold);
                let entry = self.suspended_players.entry(pid).or_insert(0);
                *entry = entry.saturating_add(1);
                actions.new_suspensions.push((pid, 1));
            } else {
                self.yellow_card_accumulation.insert(pid, new);
            }
        }
        actions
    }

    /// Mark one match as served against the league's tracking copy.
    /// Used to keep the league-level analytics in step with player-side
    /// `serve_suspension_match` decrements. Returns true when the
    /// counter hits zero and the player is removed from the map.
    pub fn record_suspension_served(&mut self, player_id: u32) -> bool {
        let Some(remaining) = self.suspended_players.get_mut(&player_id) else {
            return false;
        };
        if *remaining > 0 {
            *remaining -= 1;
        }
        if *remaining == 0 {
            self.suspended_players.remove(&player_id);
            true
        } else {
            false
        }
    }

    /// True when the club's rolling deficit (outcome - income, taken
    /// over the full ledger horizon already on the balance) exceeds
    /// the warning band. Detection is country-FFP-gated by the
    /// caller.
    pub fn check_ffp_violation(&self, club: &Club) -> bool {
        self.club_rolling_deficit(club) > self.ffp_thresholds.warning
    }

    /// Open a new FFP case if this club is over the warning band and
    /// not already inside the post-hearing cooldown of an earlier
    /// case. Idempotent — calling twice on the same matchday only
    /// opens one case.
    pub fn maybe_open_ffp_case(&mut self, club: &Club, today: NaiveDate) {
        let deficit = self.club_rolling_deficit(club);
        if deficit <= self.ffp_thresholds.warning {
            return;
        }
        // Already an open case for this club? Don't double-open.
        if self.ffp_cases.iter().any(|c| c.club_id == club.id) {
            return;
        }
        // Cooldown — last concluded case must be older than the
        // configured cooldown_days for this club.
        if let Some(last) = self
            .ffp_history
            .iter()
            .filter(|c| c.club_id == club.id)
            .max_by_key(|c| c.hearing_date)
        {
            let cooldown = Duration::days(self.ffp_thresholds.cooldown_days);
            if today < last.hearing_date + cooldown {
                return;
            }
        }
        let sanction = self.escalate_sanction(deficit);
        let hearing_date = today + Duration::days(self.ffp_thresholds.hearing_offset_days);
        self.ffp_cases.push(FFPCase {
            club_id: club.id,
            violation_type: FFPViolationType::ExcessiveDeficit,
            sanction,
            opened_date: today,
            hearing_date,
            applied: false,
            recorded_deficit: deficit,
        });
    }

    /// Resolve every FFP case whose hearing date is now in the past.
    /// Each case applies its sanction exactly once: a point deduction
    /// updates the league table, a fine charges the club's finance
    /// ledger, a transfer ban flags the club, and a warning emits
    /// nothing tangible. Resolved cases move to `ffp_history`.
    pub fn process_pending_cases(&mut self, current_date: NaiveDate) {
        // Disciplinary case retention is independent.
        self.pending_cases
            .retain(|case| case.hearing_date > current_date);
    }

    /// Drain due FFP cases against the live league table. Returns the
    /// resolved cases so callers (e.g. processing.rs) can charge fines
    /// against the club balance and toggle transfer bans.
    pub fn resolve_due_ffp_cases(
        &mut self,
        current_date: NaiveDate,
        table: &mut LeagueTable,
    ) -> Vec<FFPCase> {
        let mut due: Vec<FFPCase> = Vec::new();
        let mut keep: Vec<FFPCase> = Vec::with_capacity(self.ffp_cases.len());
        for case in self.ffp_cases.drain(..) {
            if case.hearing_date <= current_date && !case.applied {
                let mut applied = case.clone();
                if let FFPSanction::PointDeduction(p) = case.sanction {
                    table.apply_points_deduction(case.club_id, p);
                }
                applied.applied = true;
                due.push(applied.clone());
                self.ffp_history.push(applied);
            } else {
                keep.push(case);
            }
        }
        self.ffp_cases = keep;
        due
    }

    fn club_rolling_deficit(&self, club: &Club) -> i64 {
        // Existing balance is a running aggregate of revenue and
        // expense over the ledger horizon. We treat (outcome - income)
        // as the deficit signal — same as the previous implementation,
        // but read from the balance struct so a future "trailing 12-
        // month" rebuild only changes one place.
        club.finance.balance.outcome - club.finance.balance.income
    }

    fn escalate_sanction(&self, deficit: i64) -> FFPSanction {
        let t = &self.ffp_thresholds;
        if deficit > t.transfer_ban {
            FFPSanction::TransferBan
        } else if deficit > t.points_deduction {
            // Severity scales modestly with the overshoot — 6 points at
            // band entry, +1 per ~50M over the line, capped at 12.
            let over = (deficit - t.points_deduction).max(0) as f64;
            let extra = (over / 50_000_000.0).floor() as u8;
            FFPSanction::PointDeduction((6u8.saturating_add(extra)).min(12))
        } else if deficit > t.fine {
            // Fines scale with the overshoot — round to a clean 5M
            // step so the UI displays a recognisable number.
            let over = (deficit - t.fine).max(0) as u64;
            let fine = ((over / 5_000_000) * 5_000_000 + 5_000_000).min(u32::MAX as u64) as u32;
            FFPSanction::Fine(fine)
        } else {
            FFPSanction::Warning
        }
    }
}

#[derive(Debug, Clone)]
pub struct FFPCase {
    pub club_id: u32,
    pub violation_type: FFPViolationType,
    pub sanction: FFPSanction,
    /// Day the violation was detected and the case opened.
    pub opened_date: NaiveDate,
    /// Day the sanction is applied. Until then the case is pending.
    pub hearing_date: NaiveDate,
    /// True after the sanction has been carried out (so the case can
    /// be archived without re-applying).
    pub applied: bool,
    /// Deficit reading at case open. Surfaced in the UI so a club can
    /// see how far past the threshold they were.
    pub recorded_deficit: i64,
}

/// Backwards-compat alias — old code referred to FFPViolation. Kept
/// here as a type alias rather than a parallel struct so we don't
/// introduce a divergent type.
pub type FFPViolation = FFPCase;

#[derive(Debug, Clone)]
pub enum FFPViolationType {
    ExcessiveDeficit,
    UnpaidDebts,
    FalseAccounting,
}

#[derive(Debug, Clone)]
pub enum FFPSanction {
    Warning,
    Fine(u32),
    PointDeduction(u8),
    TransferBan,
}

#[derive(Debug, Clone)]
pub struct DisciplinaryCase {
    pub player_id: u32,
    pub incident_type: String,
    pub hearing_date: NaiveDate,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::result::{FieldSquad, MatchResultRaw};
    use crate::r#match::result::ResultMatchPositionData;
    use crate::r#match::{PlayerMatchEndStats, Score, TeamScore};
    use crate::PlayerFieldPositionGroup;
    use std::collections::HashMap;

    fn make_match_with_stats(stats: HashMap<u32, PlayerMatchEndStats>) -> MatchResult {
        let raw = MatchResultRaw {
            score: Some(Score::new(1, 2)),
            position_data: ResultMatchPositionData::empty(),
            left_team_players: FieldSquad::new(),
            right_team_players: FieldSquad::new(),
            match_time_ms: 90 * 60 * 1000,
            additional_time_ms: 0,
            player_stats: stats,
            substitutions: Vec::new(),
            penalty_shootout: Vec::new(),
            player_of_the_match_id: None,
        };
        MatchResult {
            id: "test".to_string(),
            league_id: 1,
            league_slug: "test".to_string(),
            home_team_id: 1,
            away_team_id: 2,
            score: Score {
                home_team: TeamScore::new_with_score(1, 0),
                away_team: TeamScore::new_with_score(2, 0),
                details: vec![],
                home_shootout: 0,
                away_shootout: 0,
            },
            details: Some(raw),
            friendly: false,
        }
    }

    fn end_stats(yellow: u16, red: u16) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: 0,
            shots_total: 0,
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            goals: 0,
            assists: 0,
            match_rating: 6.0,
            xg: 0.0,
            position_group: PlayerFieldPositionGroup::Midfielder,
            fouls: 0,
            yellow_cards: yellow,
            red_cards: red,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
        }
    }

    #[test]
    fn red_card_returns_one_match_suspension() {
        let mut regs = LeagueRegulations::new();
        let mut stats = HashMap::new();
        stats.insert(42u32, end_stats(0, 1));
        let result = make_match_with_stats(stats);
        let actions = regs.process_disciplinary_actions(&result);
        assert_eq!(actions.new_suspensions, vec![(42u32, 1)]);
        assert_eq!(regs.suspended_players.get(&42), Some(&1));
    }

    #[test]
    fn yellow_card_under_threshold_does_not_ban() {
        let mut regs = LeagueRegulations::new();
        let mut stats = HashMap::new();
        stats.insert(42u32, end_stats(1, 0));
        let result = make_match_with_stats(stats);
        let actions = regs.process_disciplinary_actions(&result);
        assert!(actions.new_suspensions.is_empty());
        assert_eq!(regs.yellow_card_accumulation.get(&42), Some(&1));
    }

    #[test]
    fn yellow_accumulation_triggers_ban_at_threshold() {
        let mut regs = LeagueRegulations::new();
        // Pump 4 yellows.
        for _ in 0..4 {
            let mut stats = HashMap::new();
            stats.insert(42u32, end_stats(1, 0));
            let result = make_match_with_stats(stats);
            let actions = regs.process_disciplinary_actions(&result);
            assert!(actions.new_suspensions.is_empty());
        }
        // 5th crosses the FIFA threshold.
        let mut stats = HashMap::new();
        stats.insert(42u32, end_stats(1, 0));
        let result = make_match_with_stats(stats);
        let actions = regs.process_disciplinary_actions(&result);
        assert_eq!(actions.new_suspensions, vec![(42u32, 1)]);
        assert_eq!(regs.suspended_players.get(&42), Some(&1));
    }

    #[test]
    fn record_suspension_served_clears_counter() {
        let mut regs = LeagueRegulations::new();
        regs.suspended_players.insert(7, 2);
        assert_eq!(regs.record_suspension_served(7), false);
        assert_eq!(regs.suspended_players.get(&7), Some(&1));
        assert_eq!(regs.record_suspension_served(7), true);
        assert!(!regs.suspended_players.contains_key(&7));
    }

    fn d(y: i32, m: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_club_with_deficit(deficit: i64) -> crate::Club {
        use crate::academy::ClubAcademy;
        use crate::shared::Location;
        use crate::{
            Club, ClubColors, ClubFinances, ClubFacilities, ClubStatus, TeamCollection,
        };
        let mut club = Club::new(
            1,
            "Test".to_string(),
            Location::new(1),
            ClubFinances::new(0, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(Vec::new()),
            ClubFacilities::default(),
        );
        // The deficit signal is `outcome - income` per
        // club_rolling_deficit. Set the balance to encode that.
        if deficit > 0 {
            club.finance.balance.outcome = deficit;
            club.finance.balance.income = 0;
        } else {
            club.finance.balance.income = deficit.abs();
            club.finance.balance.outcome = 0;
        }
        club
    }

    #[test]
    fn ffp_case_opens_only_above_warning_band() {
        let mut regs = LeagueRegulations::new();
        // Below warning — no case opens.
        let safe_club = make_club_with_deficit(20_000_000);
        regs.maybe_open_ffp_case(&safe_club, d(2032, 1, 1));
        assert!(regs.ffp_cases.is_empty());

        // Above warning — case opens.
        let unsafe_club = make_club_with_deficit(40_000_000);
        regs.maybe_open_ffp_case(&unsafe_club, d(2032, 1, 1));
        assert_eq!(regs.ffp_cases.len(), 1);
    }

    #[test]
    fn ffp_case_escalates_with_deficit_size() {
        let mut regs = LeagueRegulations::new();
        let huge_club = {
            let mut c = make_club_with_deficit(220_000_000);
            c.id = 99;
            c
        };
        regs.maybe_open_ffp_case(&huge_club, d(2032, 1, 1));
        // 220M > transfer_ban (200M) → TransferBan sanction.
        assert!(matches!(
            regs.ffp_cases[0].sanction,
            FFPSanction::TransferBan
        ));
    }

    #[test]
    fn ffp_case_does_not_double_open_for_same_club() {
        let mut regs = LeagueRegulations::new();
        let club = make_club_with_deficit(40_000_000);
        regs.maybe_open_ffp_case(&club, d(2032, 1, 1));
        assert_eq!(regs.ffp_cases.len(), 1);
        regs.maybe_open_ffp_case(&club, d(2032, 1, 5));
        assert_eq!(regs.ffp_cases.len(), 1);
    }

    #[test]
    fn ffp_case_resolves_at_hearing_with_point_deduction() {
        use crate::league::LeagueTable;
        let mut regs = LeagueRegulations::new();
        let club = make_club_with_deficit(110_000_000); // points-deduction band
        regs.maybe_open_ffp_case(&club, d(2032, 1, 1));
        let hearing = regs.ffp_cases[0].hearing_date;
        let mut table = LeagueTable::new(&[1u32]);
        table.rows[0].points = 30;

        // Before hearing — case still pending.
        let resolved = regs.resolve_due_ffp_cases(hearing - chrono::Duration::days(1), &mut table);
        assert!(resolved.is_empty());
        assert_eq!(table.rows[0].points_deduction, 0);

        // On hearing — case resolves, deduction recorded, case
        // archived.
        let resolved = regs.resolve_due_ffp_cases(hearing, &mut table);
        assert_eq!(resolved.len(), 1);
        assert!(table.rows[0].points_deduction > 0);
        assert!(regs.ffp_cases.is_empty());
        assert_eq!(regs.ffp_history.len(), 1);
    }

    #[test]
    fn ffp_case_does_not_reopen_inside_cooldown_window() {
        use crate::league::LeagueTable;
        let mut regs = LeagueRegulations::new();
        let club = make_club_with_deficit(110_000_000);
        regs.maybe_open_ffp_case(&club, d(2032, 1, 1));
        let hearing = regs.ffp_cases[0].hearing_date;
        let mut table = LeagueTable::new(&[1u32]);
        regs.resolve_due_ffp_cases(hearing, &mut table);

        // Even with the same overspend, the same club shouldn't
        // re-open inside the cooldown window.
        regs.maybe_open_ffp_case(&club, hearing + chrono::Duration::days(30));
        assert!(regs.ffp_cases.is_empty());
        // Past the cooldown — we re-open.
        regs.maybe_open_ffp_case(&club, hearing + chrono::Duration::days(400));
        assert_eq!(regs.ffp_cases.len(), 1);
    }
}
