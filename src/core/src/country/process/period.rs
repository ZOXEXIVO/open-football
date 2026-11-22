use crate::ClubResult;
use crate::Country;
use crate::club::finance::ParachuteEntitlement;
use crate::country::CountryResult;
use crate::utils::DateUtils;
use crate::{
    Club, ContractBonusType, HappinessEventType, SeasonOutcomeContext, SeasonOutcomeKind, Team,
    TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use rustc_hash::{FxHashMap, FxHashSet};

impl Country {
    pub(crate) fn on_end_of_period(&mut self, date: NaiveDate, club_results: &[ClubResult]) {
        // Get season dates from league settings
        let season = self.season_dates();

        if season.is_season_end(date) {
            debug!("End of season processing");

            CountryResult::process_season_awards(self, club_results, date);
            // NOTE: loan returns are handled in a separate phase (process_loan_returns)
            // that runs AFTER club results, so ClubResult references remain valid
            CountryResult::process_player_retirements(self, date);
        }

        // Monthly check: retire players who are past max retirement age
        // so they don't linger on teams until season end
        if DateUtils::is_month_beginning(date) && date.month() as u8 != season.end_month {
            CountryResult::process_overdue_retirements(self, date);
        }

        // Monthly reputation decay — teams that aren't achieving anything
        // drift back toward the mean. Runs on the 1st regardless of season.
        if DateUtils::is_month_beginning(date) {
            for club in &mut self.clubs {
                for team in club.teams.iter_mut() {
                    team.on_month_tick();
                }
            }
        }

        // Monthly parent-side renewal pass. Mirrors the per-club
        // `ContractRenewalManager::run` that runs inside `Club::simulate`,
        // but reaches across rosters to find players this club has loaned
        // out — those are invisible to the per-club loop because they
        // physically live at the borrower.
        if DateUtils::is_month_beginning(date) {
            CountryResult::process_parent_loan_renewals(self, date);
        }

        // Promotion/relegation: runs on the 1st of the month AFTER the latest
        // non-friendly league in the country has finished its season. Using
        // the tier-1 end date alone can fire before lower tiers are done,
        // leaving their final_table empty and silently skipping the swap.
        let latest_end_month = self
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| l.settings.season_ending_half.to_month)
            .max()
            .unwrap_or(season.end_month);
        let promo_month = if latest_end_month == 12 {
            1u8
        } else {
            latest_end_month + 1
        };
        if DateUtils::is_month_beginning(date) && date.month() as u8 == promo_month {
            // Age parachute entitlements before the swap runs: a club
            // relegated *this* season is then stamped back to season zero
            // below, and one whose three seasons are up drops off.
            CountryResult::age_parachute_entitlements(self);
            CountryResult::process_promotion_relegation(self, date);
        }

        // Late-season relegation-fear audit — runs once a month, scoped
        // to the second half of the season for tier-1+ leagues. Players
        // in the bottom (relegation_spots + 1) of the live table feel it.
        if DateUtils::is_month_beginning(date) {
            CountryResult::process_relegation_fear_audit(self, date);
            CountryResult::process_administration_sanctions(self);
        }

        if date.month() == 12 && date.day() == 31 {
            CountryResult::process_year_end_finances(self);
        }
    }
}

pub(crate) struct PromotionRelegationSwap {
    date: NaiveDate,
    tier1_id: u32,
    relegated_to: FxHashMap<u32, u32>,
    relegated_team_ids: Vec<u32>,
    pub(crate) promoted_team_ids: Vec<u32>,
    lower_ids: Vec<u32>,
    relegation_outcome_by_team: FxHashMap<u32, (u8, u16, i16)>,
    promotion_window_team_ids: Vec<u32>,
    vacated_tier: u8,
    crosses_tier: bool,
    senior_league_ids: FxHashSet<u32>,
}

impl PromotionRelegationSwap {
    pub(crate) fn new(
        country: &Country,
        date: NaiveDate,
        tier1_id: u32,
        relegated: &[(u32, u32)],
        promoted: &[(u32, u32)],
    ) -> Self {
        let relegated_to: FxHashMap<u32, u32> = relegated.iter().copied().collect();
        let relegated_team_ids: Vec<u32> = relegated.iter().map(|&(team, _)| team).collect();
        let promoted_team_ids: Vec<u32> = promoted.iter().map(|&(team, _)| team).collect();
        let mut lower_ids: Vec<u32> = relegated
            .iter()
            .chain(promoted)
            .map(|&(_, lower)| lower)
            .collect();
        lower_ids.sort_unstable();
        lower_ids.dedup();
        let swap_count = relegated_team_ids.len().max(1);

        let relegation_outcome_by_team = country
            .leagues
            .leagues
            .iter()
            .find(|league| league.id == tier1_id)
            .and_then(|league| league.final_table.as_ref())
            .map(|table| {
                let total = table.len();
                let safety_idx = total.saturating_sub(swap_count);
                let safety_points = table
                    .get(safety_idx.saturating_sub(1))
                    .map(|row| row.effective_points() as u16)
                    .unwrap_or(0);
                table
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| relegated_team_ids.contains(&row.team_id))
                    .map(|(idx, row)| {
                        let pos = (idx + 1) as u8;
                        let pts = row.effective_points() as u16;
                        let gap = pts as i16 - safety_points as i16;
                        (row.team_id, (pos, pts, gap))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let promotion_window_team_ids: Vec<u32> = country
            .leagues
            .leagues
            .iter()
            .filter(|league| lower_ids.contains(&league.id))
            .filter_map(|league| {
                let table = league.final_table.as_ref()?;
                let window = (league.settings.promotion_spots as usize + 2).min(table.len());
                Some(
                    table
                        .iter()
                        .take(window)
                        .map(|row| row.team_id)
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();

        let vacated_tier = country
            .leagues
            .leagues
            .iter()
            .find(|league| league.id == tier1_id)
            .map(|league| league.settings.tier.max(1))
            .unwrap_or(1);
        let crosses_tier = country
            .leagues
            .leagues
            .iter()
            .filter(|league| lower_ids.contains(&league.id))
            .any(|league| league.settings.tier > vacated_tier);
        let senior_league_ids = country
            .leagues
            .leagues
            .iter()
            .filter(|league| !league.friendly)
            .map(|league| league.id)
            .collect();

        Self {
            date,
            tier1_id,
            relegated_to,
            relegated_team_ids,
            promoted_team_ids,
            lower_ids,
            relegation_outcome_by_team,
            promotion_window_team_ids,
            vacated_tier,
            crosses_tier,
            senior_league_ids,
        }
    }

    pub(crate) fn swap_club(&self, club: &mut Club) {
        let mut new_main_league_id: Option<u32> = None;
        let mut bonus_total: i64 = 0;
        for team in &mut club.teams.teams {
            if let Some(&lower_id) = self.relegated_to.get(&team.id) {
                self.relegate_team(team, lower_id);
                if team.team_type == TeamType::Main {
                    new_main_league_id = Some(lower_id);
                }
            } else if self.promoted_team_ids.contains(&team.id) {
                self.promote_team(team, &mut bonus_total);
                if team.team_type == TeamType::Main {
                    new_main_league_id = Some(self.tier1_id);
                }
            } else {
                self.pay_survival_bonus(team, &mut bonus_total);
            }
        }

        self.release_non_promotion(club);
        if bonus_total > 0 {
            club.finance.balance.push_expense_player_wages(bonus_total);
        }
        self.apply_parachute(club);
        if let Some(new_league_id) = new_main_league_id {
            self.move_sub_teams(club, new_league_id);
        }
    }

    fn relegate_team(&self, team: &mut Team, lower_id: u32) {
        info!(
            "⬇️ Relegation: team {} ({}) moves to league {}",
            team.name, team.id, lower_id
        );
        team.move_to_league(lower_id);
        let outcome = self.relegation_outcome_by_team.get(&team.id).copied();
        for player in team.players.iter_mut() {
            let mut ctx =
                SeasonOutcomeContext::new(SeasonOutcomeKind::Relegated).with_league(self.tier1_id);
            if let Some((pos, pts, gap)) = outcome {
                ctx = ctx
                    .with_final_position(pos)
                    .with_points(pts)
                    .with_points_to_safety(gap);
            }
            player.on_season_outcome(HappinessEventType::Relegated, 365, 1.0, self.date, ctx);
            if let Some(contract) = player.contract.as_mut() {
                let _ = contract.apply_relegation_wage_decrease();
                let _ = contract.take_relegation_release();
            }
        }
    }

    fn promote_team(&self, team: &mut Team, bonus_total: &mut i64) {
        info!(
            "⬆️ Promotion: team {} ({}) moves to league {}",
            team.name, team.id, self.tier1_id
        );
        team.move_to_league(self.tier1_id);
        for player in team.players.iter_mut() {
            if let Some(contract) = player.contract.as_mut() {
                let _ = contract.apply_promotion_wage_increase();
                for bonus in &contract.bonuses {
                    if matches!(bonus.bonus_type, ContractBonusType::PromotionFee)
                        && bonus.value > 0
                    {
                        *bonus_total += bonus.value as i64;
                    }
                }
            }
        }
    }

    fn pay_survival_bonus(&self, team: &mut Team, bonus_total: &mut i64) {
        if team.league_id != Some(self.tier1_id) {
            return;
        }
        for player in team.players.iter_mut() {
            if let Some(contract) = player.contract.as_mut() {
                for bonus in &contract.bonuses {
                    if matches!(bonus.bonus_type, ContractBonusType::AvoidRelegationFee)
                        && bonus.value > 0
                    {
                        *bonus_total += bonus.value as i64;
                    }
                }
            }
        }
    }

    fn release_non_promotion(&self, club: &mut Club) {
        let club_was_in_window = club
            .teams
            .teams
            .iter()
            .any(|team| self.promotion_window_team_ids.contains(&team.id));
        let club_was_promoted = club
            .teams
            .teams
            .iter()
            .any(|team| self.promoted_team_ids.contains(&team.id));
        if !club_was_in_window || club_was_promoted {
            return;
        }
        for team in &mut club.teams.teams {
            if team
                .league_id
                .is_some_and(|league_id| self.lower_ids.contains(&league_id))
            {
                for player in team.players.iter_mut() {
                    if let Some(contract) = player.contract.as_mut() {
                        let _ = contract.take_non_promotion_release();
                    }
                }
            }
        }
    }

    fn apply_parachute(&self, club: &mut Club) {
        let first_team_in =
            |ids: &[u32]| club.teams.main().is_some_and(|team| ids.contains(&team.id));
        let first_team_relegated = first_team_in(&self.relegated_team_ids);
        let first_team_promoted = first_team_in(&self.promoted_team_ids);
        if !self.crosses_tier {
            return;
        }
        if first_team_relegated {
            club.finance.parachute = Some(ParachuteEntitlement {
                from_tier: self.vacated_tier,
                seasons_elapsed: 0,
            });
        } else if first_team_promoted {
            club.finance.parachute = None;
        }
    }

    fn move_sub_teams(&self, club: &mut Club, new_league_id: u32) {
        for team in &mut club.teams.teams {
            if team.team_type == TeamType::Main
                || team
                    .league_id
                    .is_some_and(|league_id| self.senior_league_ids.contains(&league_id))
            {
                continue;
            }
            let type_offset = match team.team_type {
                TeamType::U18 => 100_000,
                TeamType::U19 => 110_000,
                TeamType::U20 => 120_000,
                TeamType::U21 => 130_000,
                TeamType::U23 => 140_000,
                _ => 200_000,
            };
            team.move_to_league(new_league_id + type_offset);
        }
    }
}
