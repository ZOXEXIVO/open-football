//! Career match-experience background — the observable record of official
//! football a player carries behind him, and what that record does to his
//! thinking and to how a dressing room receives him.
//!
//! Built from the canonical `season_ledger` plus the current season's
//! already-closed spells (a loan returned from mid-season is closed the
//! day the player walks back in), so a returning loanee's record counts
//! immediately — the sim does not wait for the season-end freeze. The
//! player's ACTIVE spell is deliberately excluded: those minutes are the
//! present, judged by the playing-time model, not the background.
//!
//! Everything here is observable (appearances, starts, the reputation of
//! the teams the record was earned at). No hidden CA/PA — a club, a
//! teammate, or the player himself can only reason from what actually
//! happened on the pitch.
//!
//! Consumers:
//!   * `adaptation_score` — seasoned professionals settle into a new
//!     dressing room faster ([`MatchExperienceBackground::adaptation_points`]).
//!   * the playing-time frustration model — a real record of official
//!     starts raises the player's own expectation bar above his squad
//!     status ([`MatchExperienceBackground::expected_start_share_floor`]).
//!   * the big-club-aura squad-perception audit — a spell at a club that
//!     outshines the current one draws star treatment from teammates
//!     ([`MatchExperienceBackground::aura_over`]).

use crate::Player;
use crate::league::Season;
use chrono::NaiveDate;
use std::collections::HashMap;

/// One closed season-spell slice used while aggregating the background.
struct SpellSlice {
    season_year: u16,
    team_slug: String,
    team_reputation: u16,
    starts: u16,
    apps: u16,
    is_loan: bool,
}

/// Aggregated official-football record behind a player. See the module
/// docs for sources and consumers.
#[derive(Debug, Clone)]
pub struct MatchExperienceBackground {
    /// Career official appearances (league + domestic + continental cups;
    /// friendlies never count).
    pub career_official_apps: u32,
    /// Career official starts.
    pub career_official_starts: u32,
    /// Start share across the most recent one or two completed
    /// season-years of the record, weighted toward the most recent.
    /// 0..1 of an assumed full league season.
    pub recent_start_share: f32,
    /// Official starts inside the recent window that came in LOAN spells
    /// — the "proved himself out on loan" discriminator.
    pub recent_loan_starts: u16,
    /// Apps-weighted team reputation of the recent record — WHERE the
    /// recent football was played, so a League-Two record doesn't read
    /// as a Serie-A record.
    pub recent_record_reputation: u16,
    /// Highest team reputation at which the player had a real spell
    /// (enough apps in one season to have genuinely played there).
    pub peak_spell_reputation: u16,
    /// Season-start year of that peak spell, for recency fading.
    pub peak_spell_season_year: u16,
    /// Apps in the peak spell — a two-cameo season at a giant is not
    /// "played for the giant".
    pub peak_spell_apps: u16,
    /// Distinct clubs the record spans.
    pub distinct_clubs: u16,
    /// Continuous 0..1 "established professional" saturation over career
    /// official apps — no cliff between prospect and pro.
    pub established: f32,
}

impl MatchExperienceBackground {
    /// Assumed league-season length for start-share normalisation.
    const SEASON_MATCHES: f32 = 34.0;
    /// Apps in one season-spell before it anchors the peak level — below
    /// this the player visited the club, he didn't play for it.
    const REAL_SPELL_APPS: u16 = 8;
    /// Saturation constant for `established`: ~50 apps ≈ 0.63, ~100 ≈ 0.86.
    const ESTABLISHED_SCALE: f32 = 50.0;

    pub fn from_player(player: &Player) -> Self {
        let mut slices: Vec<SpellSlice> = Vec::new();

        for e in &player.statistics_history.season_ledger {
            if !e.competition_kind.counts_toward_career_history() {
                continue;
            }
            slices.push(SpellSlice {
                season_year: e.season_start_year,
                team_slug: e.team_slug.clone(),
                team_reputation: e.team_reputation,
                starts: e.statistics.played,
                apps: e.statistics.played + e.statistics.played_subs,
                is_loan: e.is_loan,
            });
        }

        // Current-season spells the player has already LEFT — the loan he
        // just returned from lives here until the season-end freeze. The
        // still-open spell (departed_date None) is the present, not the
        // background.
        for e in &player.statistics_history.current {
            if e.departed_date.is_none() {
                continue;
            }
            slices.push(SpellSlice {
                season_year: Season::from_date(e.joined_date).start_year,
                team_slug: e.team_slug.clone(),
                team_reputation: e.team_reputation,
                starts: e.statistics.played,
                apps: e.statistics.played + e.statistics.played_subs,
                is_loan: e.is_loan,
            });
        }

        let career_official_apps: u32 = slices.iter().map(|s| s.apps as u32).sum();
        let career_official_starts: u32 = slices.iter().map(|s| s.starts as u32).sum();
        let established = 1.0 - (-(career_official_apps as f32) / Self::ESTABLISHED_SCALE).exp();

        let mut distinct: Vec<&str> = slices.iter().map(|s| s.team_slug.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        let distinct_clubs = distinct.len() as u16;

        // Peak spell: per (season, club) totals; only seasons with real
        // involvement anchor the level the player has actually played at.
        let mut per_spell: HashMap<(u16, &str), (u16, u16)> = HashMap::new();
        for s in &slices {
            let entry = per_spell
                .entry((s.season_year, s.team_slug.as_str()))
                .or_insert((0, s.team_reputation));
            entry.0 = entry.0.saturating_add(s.apps);
            entry.1 = entry.1.max(s.team_reputation);
        }
        let mut peak_spell_reputation = 0u16;
        let mut peak_spell_season_year = 0u16;
        let mut peak_spell_apps = 0u16;
        for ((year, _slug), (apps, reputation)) in &per_spell {
            if *apps < Self::REAL_SPELL_APPS {
                continue;
            }
            if *reputation > peak_spell_reputation
                || (*reputation == peak_spell_reputation && *year > peak_spell_season_year)
            {
                peak_spell_reputation = *reputation;
                peak_spell_season_year = *year;
                peak_spell_apps = *apps;
            }
        }

        // Recent window: the latest season-year in the record plus the one
        // before it, weighted toward the latest.
        let latest_year = slices.iter().map(|s| s.season_year).max();
        let (recent_start_share, recent_loan_starts, recent_record_reputation) =
            match latest_year {
                None => (0.0, 0, 0),
                Some(latest) => {
                    let share_of = |year: u16| -> Option<f32> {
                        let starts: u16 = slices
                            .iter()
                            .filter(|s| s.season_year == year)
                            .map(|s| s.starts)
                            .sum();
                        let any = slices.iter().any(|s| s.season_year == year);
                        any.then(|| (starts as f32 / Self::SEASON_MATCHES).clamp(0.0, 1.0))
                    };
                    let last = share_of(latest).unwrap_or(0.0);
                    let share = match latest.checked_sub(1).and_then(share_of) {
                        Some(prev) => 0.65 * last + 0.35 * prev,
                        None => last,
                    };
                    let recent = |s: &&SpellSlice| s.season_year + 1 >= latest;
                    let loan_starts: u16 = slices
                        .iter()
                        .filter(recent)
                        .filter(|s| s.is_loan)
                        .map(|s| s.starts)
                        .sum();
                    let (rep_weighted, weight): (u32, u32) = slices
                        .iter()
                        .filter(recent)
                        .fold((0, 0), |(rw, w), s| {
                            (rw + s.team_reputation as u32 * s.apps as u32, w + s.apps as u32)
                        });
                    let record_rep = if weight > 0 {
                        (rep_weighted / weight) as u16
                    } else {
                        0
                    };
                    (share, loan_starts, record_rep)
                }
            };

        MatchExperienceBackground {
            career_official_apps,
            career_official_starts,
            recent_start_share,
            recent_loan_starts,
            recent_record_reputation,
            peak_spell_reputation,
            peak_spell_season_year,
            peak_spell_apps,
            distinct_clubs,
            established,
        }
    }

    /// Adaptation-score contribution: official football behind him means
    /// a new dressing room is routine, not a leap — and a player who has
    /// changed clubs before knows how to arrive. Continuous, 0..+8.
    pub fn adaptation_points(&self) -> f32 {
        let seasoned = self.established * 5.0;
        let journeyman = self.distinct_clubs.saturating_sub(1).min(3) as f32;
        seasoned + journeyman
    }

    /// The player's OWN expectation of a start share, grounded in his
    /// recent record rather than the role the club assigned him. A young
    /// returnee who started a full season on loan no longer thinks like a
    /// prospect — but a record earned two levels down is discounted when
    /// he looks around a much stronger squad, and history alone never
    /// entitles anyone to more than a regular's share.
    pub fn expected_start_share_floor(&self, current_team_reputation: u16) -> f32 {
        if self.recent_start_share <= 0.0 {
            return 0.0;
        }
        // Conviction grows with the size of the career behind the share —
        // one good half-season convinces less than an established record.
        let conviction = 0.55 + 0.25 * self.established;
        let base = self.recent_start_share * conviction;
        let gap = current_team_reputation.saturating_sub(self.recent_record_reputation) as f32;
        let level_damp = 1.0 - (gap / 3000.0).clamp(0.0, 0.7);
        (base * level_damp).min(0.65)
    }

    /// How much the player's background outshines the squad he is in —
    /// the "he played for Juventus" aura, 0..1. Needs a real spell at a
    /// clearly bigger club, fades as the spell recedes into the past.
    pub fn aura_over(&self, current_team_reputation: u16, today: NaiveDate) -> f32 {
        if self.peak_spell_reputation == 0 {
            return 0.0;
        }
        let gap = self
            .peak_spell_reputation
            .saturating_sub(current_team_reputation) as f32;
        let gap01 = (gap / 2500.0).clamp(0.0, 1.0);
        let involvement = (self.peak_spell_apps as f32 / 15.0).clamp(0.0, 1.0);
        let years_since = Season::from_date(today)
            .start_year
            .saturating_sub(self.peak_spell_season_year);
        let recency = (1.0 - 0.35 * years_since.saturating_sub(1) as f32).clamp(0.0, 1.0);
        gap01 * involvement * recency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::history::CurrentSeasonEntry;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatCompetitionKind, PlayerStatLedgerEntry, PlayerStatistics,
    };
    use chrono::NaiveDate;

    fn make_player() -> Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".into(), "1".into()))
            .birth_date(NaiveDate::from_ymd_opt(2006, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn ledger_row(year: u16, starts: u16, is_loan: bool, reputation: u16) -> PlayerStatLedgerEntry {
        PlayerStatLedgerEntry {
            seq_id: 0,
            season_start_year: year,
            team_slug: format!("club-{}", reputation),
            team_name: "T".into(),
            team_reputation: reputation,
            league_slug: "l".into(),
            league_name: "L".into(),
            competition_kind: PlayerStatCompetitionKind::League,
            competition_slug: String::new(),
            is_loan,
            transfer_fee: None,
            coverage_days: None,
            statistics: PlayerStatistics {
                played: starts,
                ..Default::default()
            },
        }
    }

    #[test]
    fn no_official_record_means_no_background() {
        let p = make_player();
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(b.career_official_apps, 0);
        assert_eq!(b.adaptation_points(), 0.0);
        assert_eq!(b.expected_start_share_floor(3_000), 0.0);
        assert_eq!(
            b.aura_over(1_000, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
            0.0
        );
    }

    #[test]
    fn friendlies_never_count() {
        let mut p = make_player();
        let mut row = ledger_row(2025, 20, false, 4_000);
        row.competition_kind = PlayerStatCompetitionKind::Friendly;
        p.statistics_history.season_ledger.push(row);
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(b.career_official_apps, 0);
    }

    #[test]
    fn full_loan_season_raises_the_expectation_bar_at_the_same_level() {
        let mut p = make_player();
        p.statistics_history
            .season_ledger
            .push(ledger_row(2025, 24, true, 6_000));
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(b.recent_loan_starts, 24);
        assert!(b.recent_start_share > 0.65, "24/34 starts is a regular");
        let floor = b.expected_start_share_floor(6_000);
        assert!(
            floor > 0.35,
            "a full loan season must lift the bar far above the prospect's 0.10 (got {floor})"
        );
    }

    #[test]
    fn record_earned_below_is_discounted_at_a_giant() {
        let mut p = make_player();
        p.statistics_history
            .season_ledger
            .push(ledger_row(2025, 24, true, 6_000));
        let b = MatchExperienceBackground::from_player(&p);
        let floor = b.expected_start_share_floor(9_000);
        assert!(
            floor < 0.20,
            "a mid-level record must not demand starts at a giant (got {floor})"
        );
    }

    #[test]
    fn closed_current_season_spell_counts_immediately() {
        let mut p = make_player();
        p.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Torino".into(),
            team_slug: "torino".into(),
            team_reputation: 6_000,
            league_name: "Serie A".into(),
            league_slug: "serie-a".into(),
            is_loan: true,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played: 20,
                ..Default::default()
            },
            departed_date: Some(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()),
            joined_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            seq_id: 0,
        });
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(
            b.recent_loan_starts, 20,
            "the just-returned loan must count before the season-end freeze"
        );
    }

    #[test]
    fn open_spell_is_the_present_not_the_background() {
        let mut p = make_player();
        p.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Parent".into(),
            team_slug: "parent".into(),
            team_reputation: 5_000,
            league_name: "L".into(),
            league_slug: "l".into(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played: 15,
                ..Default::default()
            },
            departed_date: None,
            joined_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            seq_id: 0,
        });
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(b.career_official_apps, 0);
    }

    #[test]
    fn real_spell_at_a_giant_carries_an_aura_at_a_small_club() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let mut p = make_player();
        p.statistics_history
            .season_ledger
            .push(ledger_row(2025, 25, true, 8_000));
        let b = MatchExperienceBackground::from_player(&p);
        assert!(
            b.aura_over(2_500, today) > 0.8,
            "a fresh full season at the giant should shine brightly at a minnow"
        );
        assert_eq!(
            b.aura_over(8_000, today),
            0.0,
            "no aura among peers at the same level"
        );
    }

    #[test]
    fn cameo_at_a_giant_is_not_an_aura() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let mut p = make_player();
        // Three cup cameos at the giant, a real season lower down.
        p.statistics_history
            .season_ledger
            .push(ledger_row(2025, 3, false, 9_000));
        p.statistics_history
            .season_ledger
            .push(ledger_row(2024, 30, false, 3_000));
        let b = MatchExperienceBackground::from_player(&p);
        assert_eq!(
            b.peak_spell_reputation, 3_000,
            "three cameos do not anchor the level he has played at"
        );
        assert_eq!(b.aura_over(3_000, today), 0.0);
    }

    #[test]
    fn established_grows_continuously_with_the_record() {
        let mut rookie = make_player();
        rookie
            .statistics_history
            .season_ledger
            .push(ledger_row(2025, 5, false, 3_000));
        let mut veteran = make_player();
        for year in 2018..=2025u16 {
            veteran
                .statistics_history
                .season_ledger
                .push(ledger_row(year, 30, false, 3_000));
        }
        let rb = MatchExperienceBackground::from_player(&rookie);
        let vb = MatchExperienceBackground::from_player(&veteran);
        assert!(rb.established < 0.2);
        assert!(vb.established > 0.9);
        assert!(vb.adaptation_points() > rb.adaptation_points());
    }
}
