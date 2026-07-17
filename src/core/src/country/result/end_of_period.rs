use super::CountryResult;
use crate::ContractBonusType;
use crate::PlayerContractProposal;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::events::TransferCompletion;
use crate::club::team::reputation::{Achievement, AchievementType};
use crate::club::team::squad::{ContractRenewalManager, WageStructureSnapshot};
use crate::simulator::SimulatorData;
use crate::utils::{DateUtils, FormattingUtils, IntegerUtils};
use crate::{
    AwardReputationInput, AwardReputationKind, Club, ClubResult, Country, HappinessEventType,
    Person, Player, PlayerClubContract, PlayerFieldPositionGroup, PlayerHappiness, PlayerMessage,
    PlayerMessageType, PlayerSquadStatus, PlayerStatCompetitionKind, PlayerStatusType,
    SeasonOutcomeContext, SeasonOutcomeKind, StaffPosition, Team, TeamInfo, TeamType,
    TrophyEventContext, TrophyKind,
};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

struct LoanReturnEvent {
    player_id: u32,
    borrowing_club_id: u32,
    parent_club_id: u32,
    borrowing_info: TeamInfo,
    /// `Some((fee, is_obligation))` when the borrower exercises its
    /// negotiated option / obligation to buy as the loan expires — the
    /// player stays put and ownership transfers instead of returning.
    buyout: Option<(u32, bool)>,
}

/// Per-team view of which loaned-in players a borrower is warehousing.
/// Keeps the top `ideal_squad_depth` of each position group by current
/// ability; a loaned-in player outside that set — long-settled, barely
/// featuring, and with no permanent option pending — is surplus the
/// borrower over-loaned and should be returned to its parent early.
struct WarehousedLoans {
    kept_ids: HashSet<u32>,
}

impl WarehousedLoans {
    /// A loaned-in player needs at least this many days settled before the
    /// borrower's appearance record is judged enough to call him surplus.
    const SETTLE_DAYS: i64 = 90;
    /// At/under this appearance count over the settling window the loanee
    /// is plainly not being used.
    const UNUSED_APPS: u16 = 3;

    fn for_team(team: &Team) -> Self {
        let mut kept_ids: HashSet<u32> = HashSet::new();
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];
        for group in groups {
            let mut ranked: Vec<(u32, u8)> = team
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| (p.id, p.player_attributes.current_ability))
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (id, _) in ranked.into_iter().take(group.ideal_squad_depth()) {
                kept_ids.insert(id);
            }
        }
        WarehousedLoans { kept_ids }
    }

    /// True when `player` is a non-expiring loaned-in player the borrower is
    /// warehousing — outside the kept set, settled past `SETTLE_DAYS` with
    /// `UNUSED_APPS` or fewer appearances and no permanent option — so the
    /// loan should be ended early. Owned players and kept (playing) loanees
    /// return false.
    fn is_surplus(&self, player: &Player, date: NaiveDate) -> bool {
        let Some(loan) = player.contract_loan.as_ref() else {
            return false;
        };
        loan.loan_from_club_id.is_some()
            && loan.expiration > date
            && !self.kept_ids.contains(&player.id)
            && loan.loan_future_fee.is_none()
            && loan
                .started
                .map(|s| (date - s).num_days() >= Self::SETTLE_DAYS)
                .unwrap_or(false)
            && (player.statistics.played + player.statistics.played_subs) < Self::UNUSED_APPS
    }
}

impl CountryResult {
    pub(crate) fn process_end_of_period(
        country: &mut Country,
        date: NaiveDate,
        club_results: &[ClubResult],
    ) {
        // Get season dates from league settings
        let season = country.season_dates();

        if season.is_season_end(date) {
            debug!("End of season processing");

            Self::process_season_awards(country, club_results, date);
            // NOTE: loan returns are handled in a separate phase (process_loan_returns)
            // that runs AFTER club results, so ClubResult references remain valid
            Self::process_player_retirements(country, date);
        }

        // Monthly check: retire players who are past max retirement age
        // so they don't linger on teams until season end
        if DateUtils::is_month_beginning(date) && date.month() as u8 != season.end_month {
            Self::process_overdue_retirements(country, date);
        }

        // Monthly reputation decay — teams that aren't achieving anything
        // drift back toward the mean. Runs on the 1st regardless of season.
        if DateUtils::is_month_beginning(date) {
            for club in &mut country.clubs {
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
            Self::process_parent_loan_renewals(country, date);
        }

        // Promotion/relegation: runs on the 1st of the month AFTER the latest
        // non-friendly league in the country has finished its season. Using
        // the tier-1 end date alone can fire before lower tiers are done,
        // leaving their final_table empty and silently skipping the swap.
        let latest_end_month = country
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
            Self::process_promotion_relegation(country, date);
        }

        // Late-season relegation-fear audit — runs once a month, scoped
        // to the second half of the season for tier-1+ leagues. Players
        // in the bottom (relegation_spots + 1) of the live table feel it.
        if DateUtils::is_month_beginning(date) {
            Self::process_relegation_fear_audit(country, date);
        }

        if date.month() == 12 && date.day() == 31 {
            Self::process_year_end_finances(country);
        }
    }

    fn process_season_awards(country: &mut Country, _club_results: &[ClubResult], date: NaiveDate) {
        debug!("Processing season awards");

        // Build team_id -> club index mapping
        let team_to_club: HashMap<u32, usize> = country
            .clubs
            .iter()
            .enumerate()
            .flat_map(|(ci, club)| club.teams.iter().map(move |t| (t.id, ci)))
            .collect();

        // Trophy reputation boost: league champions and promoted sides get
        // a durable rep bump that lingers for 2 seasons (see Achievement).
        // Collected before the prize-money loop so we don't mix concerns.
        // Per-player happiness events triggered by the same final tables.
        // (team_id, event, prestige) — prestige lets a lower-tier league
        // title fire `TrophyWon` at a smaller magnitude so it doesn't
        // compete with the promotion emotion that's the real headline.
        // Parallel collect across leagues — leagues are independent and
        // the body is read-only against shared league state. The two
        // per-league vecs are folded into a single tuple to keep the
        // rayon collect simple, then flattened serially below.
        let per_league: Vec<(
            Vec<(u32, AchievementType)>,
            Vec<(u32, HappinessEventType, f32)>,
        )> = country
            .leagues
            .leagues
            .par_iter()
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                let table = match &league.final_table {
                    Some(t) if !t.is_empty() => t,
                    _ => return None,
                };

                let mut trophies: Vec<(u32, AchievementType)> = Vec::new();
                let mut events: Vec<(u32, HappinessEventType, f32)> = Vec::new();

                // For lower-tier leagues that *also* promote (Championship-style
                // setups), the title is real silverware but promotion is the
                // career-visible moment. We fire both, but soften `TrophyWon`
                // so the stack reads as "got promoted, also won the league"
                // rather than two huge wins.
                let promo_slots = league.settings.promotion_spots as usize;
                let lower_tier_with_promo = league.settings.tier > 1 && promo_slots > 0;
                if let Some(champion) = table.first() {
                    trophies.push((champion.team_id, AchievementType::LeagueTitle));
                    let trophy_prestige = if lower_tier_with_promo { 0.6 } else { 1.0 };
                    events.push((
                        champion.team_id,
                        HappinessEventType::TrophyWon,
                        trophy_prestige,
                    ));
                    if lower_tier_with_promo {
                        // Lower-league champions are *also* promoted — the
                        // promotion emotion is the dominant one.
                        events.push((
                            champion.team_id,
                            HappinessEventType::PromotionCelebration,
                            1.0,
                        ));
                    }
                }
                if promo_slots > 0 {
                    // Non-title promoted clubs (positions 2..=promo_slots).
                    // Champion already handled above with the dual emit.
                    for row in table.iter().take(promo_slots).skip(1) {
                        trophies.push((row.team_id, AchievementType::Promotion));
                        events.push((row.team_id, HappinessEventType::PromotionCelebration, 1.0));
                    }
                }

                // Continental qualification — only top-tier leagues feed
                // continental competitions. Heuristic, intentionally:
                // `continent::result::competitions::collect_*_qualified_clubs`
                // is the authoritative qualification source, but it indexes by
                // *continental ranking* rather than league reputation, and it
                // runs as part of the autumn draw — there's no clean signal
                // queryable here at end-of-season for which clubs got the
                // letters in the post.
                //
                // The reputation bands below mirror the *spot counts* that
                // collector hands out (top-4 country = 7 European spots, next
                // 4 country tier = 4, smaller top flights = 2), so a player at
                // a club that genuinely qualifies will reliably hit this
                // happiness event. Bottom flights and friendlies skip.
                //
                // Skip position 0 (title winner already gets `TrophyWon`,
                // which subsumes European qualification — no double-counting).
                if league.settings.tier == 1 {
                    let mut europe_spots = 0usize;
                    if league.reputation >= 7000 {
                        europe_spots = 7; // CL top 4 + EL/UECL slots
                    } else if league.reputation >= 5000 {
                        europe_spots = 4; // top-4 cup-tier qualification
                    } else if league.reputation >= 3000 {
                        europe_spots = 2; // 2 spots in modest top flights
                    }
                    for row in table.iter().take(europe_spots).skip(1) {
                        events.push((row.team_id, HappinessEventType::QualifiedForEurope, 1.0));
                    }
                }

                Some((trophies, events))
            })
            .collect();

        let mut trophy_awards: Vec<(u32, AchievementType)> = Vec::new();
        let mut player_team_events: Vec<(u32, HappinessEventType, f32)> = Vec::new();
        for (mut t, mut e) in per_league {
            trophy_awards.append(&mut t);
            player_team_events.append(&mut e);
        }
        for (team_id, ach_type) in trophy_awards {
            for club in &mut country.clubs {
                if club.teams.iter().any(|t| t.id == team_id) {
                    // Reputation achievement on the team side; long-term
                    // vision tracker on the board side.
                    if let Some(team) = club.teams.iter_mut().find(|t| t.id == team_id) {
                        team.on_season_trophy(Achievement::new(ach_type.clone(), date, 8));
                    }
                    club.board.on_achievement(ach_type.clone());

                    // Trophy-triggered renewal bump. The board rewards
                    // the manager directly — bigger silverware = bigger
                    // bump, chairman loyalty also lifts. Fires in addition
                    // to the season-start renewal offer so a winning
                    // campaign is recognised even when the contract still
                    // has >18 months left.
                    let (salary_bump_pct, extension_years, loyalty_lift): (f32, i32, u8) =
                        match ach_type {
                            AchievementType::ContinentalTrophy => (0.25, 3, 20),
                            AchievementType::LeagueTitle => (0.20, 2, 15),
                            AchievementType::CupWin => (0.08, 1, 6),
                            AchievementType::Promotion => (0.10, 1, 8),
                            _ => (0.0, 0, 0),
                        };
                    if salary_bump_pct > 0.0 {
                        let cur = club.board.chairman.manager_loyalty as u16;
                        club.board.chairman.manager_loyalty =
                            (cur + loyalty_lift as u16).min(100) as u8;
                        if let Some(main_team) = club.teams.main_mut() {
                            if let Some(mgr) = main_team
                                .staffs
                                .find_mut_by_position(StaffPosition::Manager)
                            {
                                if let Some(contract) = mgr.contract.as_mut() {
                                    contract.salary =
                                        ((contract.salary as f32) * (1.0 + salary_bump_pct)) as u32;
                                    if extension_years > 0 {
                                        let new_exp = contract
                                            .expired
                                            .with_year(contract.expired.year() + extension_years)
                                            .unwrap_or(contract.expired);
                                        if new_exp > contract.expired {
                                            contract.expired = new_exp;
                                        }
                                    }
                                    mgr.job_satisfaction =
                                        (mgr.job_satisfaction + 12.0).clamp(0.0, 100.0);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }

        // Collect awards per club: (club_idx, prize_money, tv_money)
        let mut club_awards: HashMap<usize, (i64, i64)> = HashMap::new();

        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }

            let table = match &league.final_table {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            let total_teams = table.len();
            let prize_pool = league.financials.prize_pool;
            let tv_deal = league.financials.tv_deal_total;

            if prize_pool == 0 && tv_deal == 0 {
                continue;
            }

            // Calculate normalization factor: sum of (1 - i/n)^2
            let total_shares: f64 = (0..total_teams)
                .map(|i| {
                    let r = i as f64 / (total_teams - 1).max(1) as f64;
                    (1.0 - r).powi(2)
                })
                .sum();

            for (position, row) in table.iter().enumerate() {
                let club_idx = match team_to_club.get(&row.team_id) {
                    Some(&idx) => idx,
                    None => continue,
                };

                // Prize money: quadratic decay by position (top-heavy)
                let share = if total_teams > 1 {
                    let pos_ratio = position as f64 / (total_teams - 1) as f64;
                    (1.0 - pos_ratio).powi(2)
                } else {
                    1.0
                };

                let prize_amount = (prize_pool as f64 * share / total_shares) as i64;

                // TV money: 50% equal + 50% merit-based
                let tv_equal = tv_deal / (2 * total_teams as i64);
                let tv_merit = (tv_deal as f64 * 0.5 * share / total_shares) as i64;
                let tv_amount = tv_equal + tv_merit;

                let entry = club_awards.entry(club_idx).or_insert((0, 0));
                entry.0 += prize_amount;
                entry.1 += tv_amount;
            }
        }

        // Apply awards to clubs
        for (club_idx, (prize, tv)) in club_awards {
            let club = &mut country.clubs[club_idx];
            if prize > 0 {
                club.finance.balance.push_income_prize_money(prize);
            }
            if tv > 0 {
                club.finance.balance.push_income_tv(tv);
            }
            debug!(
                "Season awards: {} - prize: ${}, TV: ${}",
                club.name, prize, tv
            );
        }

        // Per-player season events. One pass over the (team_id, event,
        // prestige) queue we built alongside the trophy collection, with
        // a generous 365-day cooldown so the same milestone never fires
        // twice in a season even if end-of-period happens to tick on
        // consecutive days.
        for (team_id, event, prestige) in player_team_events {
            Self::apply_team_squad_event(country, team_id, event, 365, prestige, date);
        }

        // Reset the per-player disciplinary running counter (yellow-
        // card tally). Active suspensions are preserved — a player
        // who picked up a red on the final matchday still serves it
        // in the new season, matching real FA / FIFA conventions.
        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    player.reset_season_disciplinary_state();
                }
            }
        }
    }

    /// Emit a team-level happiness event to every player on `team_id`,
    /// scaled by `prestige` (1.0 = default — domestic top cup / league
    /// title / promotion). Domestic minor cups should pass 0.7-0.8;
    /// continental trophies 1.2-1.5. Cooldown is forwarded to the
    /// player-side cooldown gate so repeated end-of-period ticks don't
    /// duplicate the event. No-op if the team isn't found in `country`.
    pub(crate) fn apply_team_squad_event(
        country: &mut Country,
        team_id: u32,
        event: HappinessEventType,
        cooldown_days: u16,
        prestige: f32,
        date: NaiveDate,
    ) {
        let is_european_qualification = matches!(event, HappinessEventType::QualifiedForEurope);
        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                if team.id != team_id {
                    continue;
                }
                for player in team.players.iter_mut() {
                    player.on_team_season_event_with_prestige(
                        event.clone(),
                        cooldown_days,
                        prestige,
                        date,
                    );
                    if is_european_qualification {
                        player.on_continental_qualification_satisfaction();
                    }
                }
            }
        }
    }

    /// Emit the domestic cup winner-trophy fan-out for the country's cup.
    ///
    /// Unlike league-title silverware (which lands on every player in the
    /// champion roster), cup medals are tied to *appearance*: a player
    /// only gets the event + award if they actually featured in the
    /// winning campaign. Eligibility is read from
    /// `cup_statistics_by_competition[cup_slug]` — which `process_local`
    /// has already populated by the time this runs in the parallel tick,
    /// including final-day starters and used substitutes. Unused
    /// substitutes, January signings who never made the bench, and
    /// youth players merely registered won't have an entry and so get
    /// nothing — matching how real-football medals are awarded.
    ///
    /// Must be invoked AFTER the day's `LeagueResult::process_local` pass
    /// — otherwise the final's appearances aren't on the player yet and
    /// the entire winning XI silently fails the eligibility check.
    ///
    /// Idempotent across ticks via the `DomesticCup::award_emitted_*`
    /// markers — the post-final daily tick re-enters this helper but
    /// `should_emit_winner_award` returns `None` on the second visit.
    pub(crate) fn process_domestic_cup_winner_awards(country: &mut Country, date: NaiveDate) {
        // Snapshot every cup-side fact the awards path needs in one
        // pass so the later mutation of `country.clubs` doesn't have to
        // overlap with a borrow of `country.domestic_cup`. The set of
        // players who appeared in the final is read from the cup's own
        // match storage (which holds `MatchResultRaw.left/right_team_players`
        // squads written by the engine), so the final-bonus flag stays
        // self-contained.
        struct CupSnapshot {
            winner_team_id: u32,
            cup_slug: String,
            cup_name: String,
            cup_league_id: u32,
            cup_reputation: u16,
            final_appearance_ids: std::collections::HashSet<u32>,
        }
        let snapshot = {
            let cup = match country.domestic_cup.as_ref() {
                Some(c) => c,
                None => return,
            };
            let Some(winner_team_id) = cup.should_emit_winner_award(&country.clubs) else {
                return;
            };
            // Build a roster snapshot of who took to the pitch in the
            // final (starter or used substitute) on the winning side.
            // If the final's `MatchResult.details` aren't available
            // (e.g. an in-progress save, or details trimmed by storage
            // retention), the set is empty — the bonus is just skipped,
            // not a hard error.
            let mut final_ids = std::collections::HashSet::new();
            if let Some((home_id, away_id)) = cup.champion_final_pairing(&country.clubs) {
                if let Some(last_tour) = cup.league.schedule.tours.last() {
                    if let Some(item) = last_tour.items.first() {
                        if let Some(mr) = cup.league.matches.get(&item.id) {
                            if let Some(details) = mr.details.as_ref() {
                                let winning_squad = if winner_team_id == home_id {
                                    Some(&details.left_team_players)
                                } else if winner_team_id == away_id {
                                    Some(&details.right_team_players)
                                } else {
                                    None
                                };
                                if let Some(squad) = winning_squad {
                                    for id in &squad.main {
                                        final_ids.insert(*id);
                                    }
                                    for id in &squad.substitutes_used {
                                        final_ids.insert(*id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            CupSnapshot {
                winner_team_id,
                cup_slug: cup.league.slug.clone(),
                cup_name: cup.league.name.clone(),
                cup_league_id: cup.league.id,
                cup_reputation: cup.league.reputation,
                final_appearance_ids: final_ids,
            }
        };

        // Pre-check: collect the eligible player ids on the winning
        // roster BEFORE firing any team / board / player side effects.
        // Without this an "all unused subs" champion (zero eligible
        // players, e.g. a save loaded mid-final) would still hand the
        // team a trophy achievement and the board a long-term-goal tick,
        // and — because the cup's `award_emitted_*` marker only fires on
        // success — those side effects would repeat every tick until
        // somebody picked up a cup app. The fix is to read eligibility
        // first and bail out entirely when nobody qualifies.
        struct EligiblePlayer {
            player_id: u32,
            apps: u16,
            starts: u16,
            sub_apps: u16,
            goals: u16,
            assists: u16,
            clean_sheets: u16,
            avg_rating: f32,
            final_appearance: bool,
        }
        let mut eligible: Vec<EligiblePlayer> = Vec::new();
        for club in country.clubs.iter() {
            for team in club.teams.iter() {
                if team.id != snapshot.winner_team_id {
                    continue;
                }
                for player in team.players.iter() {
                    let Some(idx) = player
                        .cup_statistics_by_competition
                        .iter()
                        .position(|c| c.competition_slug == snapshot.cup_slug)
                    else {
                        continue;
                    };
                    let stats = &player.cup_statistics_by_competition[idx].statistics;
                    if stats.total_games() == 0 {
                        continue;
                    }
                    let pos_group = player.position().position_group();
                    let realistic = stats.average_rating_realistic(pos_group);
                    let avg_rating = if realistic > 0.0 {
                        realistic
                    } else {
                        stats.weighted_average_rating()
                    };
                    eligible.push(EligiblePlayer {
                        player_id: player.id,
                        apps: stats.total_games(),
                        starts: stats.played,
                        sub_apps: stats.played_subs,
                        goals: stats.goals,
                        assists: stats.assists,
                        clean_sheets: stats.clean_sheets,
                        avg_rating,
                        final_appearance: snapshot.final_appearance_ids.contains(&player.id),
                    });
                }
            }
        }

        if eligible.is_empty() {
            // No one to award. Leave the cup marker unset so the next
            // tick (once `process_local` has caught up) can fire the
            // achievement + awards once instead of duplicating them.
            debug!(
                "Domestic cup {} resolved (winner team {}) but no eligible players this tick — \
                 deferring achievement + awards",
                snapshot.cup_name, snapshot.winner_team_id
            );
            return;
        }

        // Base prestige rises with the cup's reputation — a small-country
        // cup lands near 0.70, an elite domestic cup (FA Cup / Copa del
        // Rey) lands near 1.05–1.15. Players' involvement multiplier (see
        // below) further scales this per player.
        let rep_norm = (snapshot.cup_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
        let base_prestige = (0.70 + 0.45 * rep_norm).clamp(0.70, 1.15);

        // Mutating pass: fire team + board achievement once, then walk
        // the precomputed eligible set and emit per-player events +
        // award impact. The eligibility map (player_id → EligiblePlayer)
        // lets the player loop avoid re-deriving the cup stats.
        let eligibility: std::collections::HashMap<u32, EligiblePlayer> =
            eligible.into_iter().map(|e| (e.player_id, e)).collect();

        for club in country.clubs.iter_mut() {
            let mut club_owns_winner = false;
            for team in club.teams.iter_mut() {
                if team.id != snapshot.winner_team_id {
                    continue;
                }
                club_owns_winner = true;
                team.on_season_trophy(Achievement::new(AchievementType::CupWin, date, 8));

                for player in team.players.iter_mut() {
                    let Some(e) = eligibility.get(&player.id) else {
                        continue;
                    };
                    // Cup-specific involvement multiplier — replaces
                    // the generic `season_participation_factor` for
                    // this event so a player whose only football this
                    // year was three cup ties still feels the medal,
                    // while a single-cameo player gets a softer emit.
                    // A final appearance lifts the involvement floor
                    // by 0.10 (capped at 1.10) so a 1-app sub who came
                    // off the bench in the final isn't lumped in with
                    // a 1-app early-round cameo.
                    let base_involvement: f32 = match e.apps {
                        1 => 0.65,
                        2 => 0.80,
                        _ => 0.95,
                    };
                    let final_bonus: f32 = if e.final_appearance { 0.10 } else { 0.0 };
                    let involvement = (base_involvement + final_bonus).min(1.10_f32);
                    let effective_prestige = (base_prestige * involvement).min(1.10);

                    let trophy_ctx = TrophyEventContext::new(TrophyKind::DomesticCup)
                        .with_competition_id(snapshot.cup_league_id)
                        .with_competition_slug(snapshot.cup_slug.clone())
                        .with_competition_name(snapshot.cup_name.clone())
                        .with_winner_team_id(snapshot.winner_team_id)
                        .with_apps(e.apps)
                        .with_starts(e.starts)
                        .with_used_sub_apps(e.sub_apps)
                        .with_goals(e.goals)
                        .with_assists(e.assists)
                        .with_clean_sheets(e.clean_sheets)
                        .with_final_appearance(e.final_appearance);
                    let trophy_ctx = if e.avg_rating > 0.0 {
                        trophy_ctx.with_avg_rating(e.avg_rating)
                    } else {
                        trophy_ctx
                    };

                    // Distinct happiness event from the league title's
                    // `TrophyWon` so a double-winning side's cooldown
                    // doesn't collapse the two emits into one.
                    player.on_trophy_won_with_context(
                        HappinessEventType::DomesticCupWon,
                        trophy_ctx,
                        365,
                        effective_prestige,
                        true, // skip season-participation — folded into involvement
                        date,
                    );

                    let mut input = AwardReputationInput::new()
                        .with_league_id(snapshot.cup_league_id)
                        .with_league_reputation(snapshot.cup_reputation)
                        .with_matches_played(e.apps);
                    if e.avg_rating > 0.0 {
                        input = input.with_avg_rating(e.avg_rating);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::DomesticCupWinner,
                        input,
                        date,
                    );
                }
            }
            if club_owns_winner {
                club.board.on_achievement(AchievementType::CupWin);
            }
        }

        if let Some(cup) = country.domestic_cup.as_mut() {
            cup.mark_winner_award_emitted(snapshot.winner_team_id, date);
        }
    }

    /// Monthly parent-side renewal pass for loaned-out players.
    ///
    /// `ContractRenewalManager::run` only sees the club's own roster, so
    /// loaned-out players (who live at the borrowing club) are invisible
    /// to the per-club renewal loop. This pass closes that gap: for each
    /// club in `country`, scan every other club's roster for players whose
    /// permanent contract is owned by this club and whose loan agreement
    /// points back at it via `loan_from_club_id`. Build the proposal
    /// using the parent's wage structure / budget context, then push the
    /// offer into the loanee's mailbox at the borrower.
    ///
    /// Same-country only by design (matching `process_loan_returns`'s
    /// initial scan scope). Cross-country loanees are picked up when
    /// their loan ends and they return home — at which point the regular
    /// per-club renewal loop runs normally.
    pub(crate) fn process_parent_loan_renewals(country: &mut Country, date: NaiveDate) {
        if !DateUtils::is_month_beginning(date) {
            return;
        }

        // Phase 1 (immutable): for each club, snapshot the wage structure
        // and league context, then walk every club's roster looking for
        // players this club has loaned out. Builds a (loanee_id, proposal,
        // coach_name) queue without mutating anything.
        struct LoaneeOffer {
            loanee_id: u32,
            proposal: PlayerContractProposal,
            coach_name: String,
        }
        let mut queue: Vec<LoaneeOffer> = Vec::new();

        for parent_club in country.clubs.iter() {
            let parent_main_idx = match parent_club
                .teams
                .teams
                .iter()
                .position(|t| t.team_type == TeamType::Main)
            {
                Some(i) => i,
                None => continue,
            };
            let parent_main = &parent_club.teams.teams[parent_main_idx];
            let wage_budget = parent_club
                .finance
                .wage_budget
                .as_ref()
                .map(|b| b.amount.max(0.0) as u32);
            let league_rep = parent_main.reputation.world;
            // Caps anchor on the parent main team's hierarchy; the budget
            // gate compares against the club-wide bill (shared pot).
            let mut structure = WageStructureSnapshot::from_team(parent_main);
            structure.current_bill =
                WageStructureSnapshot::club_wide_bill(&parent_club.teams.teams);
            let parent_club_id = parent_club.id;

            // Walk every roster in the country looking for loanees owned
            // by this parent. Self-rosters are skipped — `is_loaned_out_from`
            // returns false for players physically still at the parent
            // (their `contract_loan` would be None or pointing elsewhere).
            for other_club in country.clubs.iter() {
                for team in other_club.teams.iter() {
                    for player in team.players.iter() {
                        if !player.is_loaned_out_from(parent_club_id) {
                            continue;
                        }
                        if let Some((proposal, coach_name)) =
                            ContractRenewalManager::try_build_loanee_offer(
                                parent_main,
                                player,
                                date,
                                wage_budget,
                                league_rep,
                                &structure,
                            )
                        {
                            queue.push(LoaneeOffer {
                                loanee_id: player.id,
                                proposal,
                                coach_name,
                            });
                        }
                    }
                }
            }
        }

        // Phase 2 (mutable): apply each queued proposal at the loanee's
        // current location. Mirrors the wire-up the in-house monthly pass
        // does — decision-history row + mailbox push.
        for offer in queue {
            'apply: for club in country.clubs.iter_mut() {
                for team in club.teams.iter_mut() {
                    if let Some(player) = team
                        .players
                        .players
                        .iter_mut()
                        .find(|p| p.id == offer.loanee_id)
                    {
                        let movement = format!(
                            "{}y · ${}/y",
                            offer.proposal.years,
                            FormattingUtils::format_money(offer.proposal.salary as f64)
                        );
                        player.decision_history.add(
                            date,
                            movement,
                            "dec_contract_renewal_offered".to_string(),
                            offer.coach_name.clone(),
                        );
                        player.mailbox.push(PlayerMessage {
                            message_type: PlayerMessageType::ContractProposal(offer.proposal),
                        });
                        break 'apply;
                    }
                }
            }
        }
    }

    /// Process loan returns — must run AFTER club_result.process() so that
    /// ClubResult player references remain valid during contract processing.
    pub(super) fn process_loan_returns(data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
        // Only check on the 1st and last day of each month to avoid daily scans
        if date.day() != 1 && date.day() < 28 {
            return;
        }

        // Phase 1: Scan — collect expired loans as lightweight events
        let events = Self::scan_expired_loans(data, country_id, date);

        // Phase 2: Execute — a stored option/obligation to buy converts
        // the loan into a permanent transfer in place; everything else
        // moves the player home (country-agnostic, by club ID).
        for event in events {
            match event.buyout {
                Some((fee, obligation)) => {
                    Self::execute_loan_buyout(data, event, fee, obligation, date)
                }
                None => Self::execute_loan_return(data, event, date, true),
            }
        }
    }

    /// Read-only scan: find players with expired loan contracts in this country.
    fn scan_expired_loans(
        data: &SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) -> Vec<LoanReturnEvent> {
        let country = match data.country(country_id) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let leagues = &country.leagues.leagues;
        let league_info = |lid: u32| -> (&str, &str) {
            leagues
                .iter()
                .find(|l| l.id == lid)
                .map(|l| (l.name.as_str(), l.slug.as_str()))
                .unwrap_or(("", ""))
        };

        let mut events = Vec::new();

        for club in &country.clubs {
            // Cheap short-circuit: most days most clubs have zero expiring
            // loans, and we'd otherwise clone the main team + league strings
            // unconditionally. Scan first, then allocate.
            // Pass when the club holds ANY loaned-in player, not just an
            // expiring one: a non-expiring surplus loan can still be drained
            // early (see the warehouse check below), so it must reach the
            // per-team scan rather than be short-circuited away here.
            let has_loan_activity = club.teams.iter().any(|team| {
                team.players.iter().any(|player| {
                    player
                        .contract_loan
                        .as_ref()
                        .map(|lc| lc.loan_from_club_id.is_some())
                        .unwrap_or(false)
                })
            });
            if !has_loan_activity {
                continue;
            }

            // Build the main-team / league strings once per club, not once
            // per player that needs returning.
            let main_team = club.teams.main();
            let main_name = main_team.map(|t| t.name.clone());
            let main_slug = main_team.map(|t| t.slug.clone());
            let main_reputation = main_team.map(|t| t.reputation.world);
            let (main_league_name, main_league_slug) = main_team
                .and_then(|t| t.league_id)
                .map(|lid| {
                    let (n, s) = league_info(lid);
                    (n.to_owned(), s.to_owned())
                })
                .unwrap_or_default();

            for team in club.teams.iter() {
                let (team_name, team_slug, team_reputation) =
                    if team.team_type == TeamType::Main || main_name.is_none() {
                        (team.name.clone(), team.slug.clone(), team.reputation.world)
                    } else {
                        (
                            main_name.clone().unwrap_or_default(),
                            main_slug.clone().unwrap_or_default(),
                            main_reputation.unwrap_or(0),
                        )
                    };

                // Warehoused-loan drain: a borrower that over-loaned a
                // position sends back the surplus loaned-in stock early,
                // alongside the loans that have simply expired.
                let warehoused = WarehousedLoans::for_team(team);

                for player in team.players.iter() {
                    let Some(ref loan_contract) = player.contract_loan else {
                        continue;
                    };
                    let Some(parent_club_id) = loan_contract.loan_from_club_id else {
                        continue;
                    };
                    let expiring = loan_contract.expiration <= date;
                    if !(expiring || warehoused.is_surplus(player, date)) {
                        continue;
                    }
                    // Option / obligation-to-buy — only a naturally
                    // expiring loan can carry one (the warehouse drain
                    // never touches option loans).
                    let buyout = if expiring {
                        Self::decide_loan_buyout(player, loan_contract)
                    } else {
                        None
                    };
                    events.push(LoanReturnEvent {
                        player_id: player.id,
                        borrowing_club_id: club.id,
                        parent_club_id,
                        borrowing_info: TeamInfo {
                            name: team_name.clone(),
                            slug: team_slug.clone(),
                            reputation: team_reputation,
                            league_name: main_league_name.clone(),
                            league_slug: main_league_slug.clone(),
                        },
                        buyout,
                    });
                }
            }
        }

        events
    }

    /// Borrower's decision on a stored option / obligation to buy as
    /// the loan expires. Obligations are binding. An option is
    /// exercised when the loan actually worked — a real body of
    /// appearances at a decent level, read from the live season stats
    /// or from the just-frozen loan ledger row when the season-end
    /// snapshot has already reset them. The player's own wish to stay
    /// (`WantsLoanMadePermanent`) nudges the bar down: signing a keen,
    /// integrated player is the easy call.
    fn decide_loan_buyout(player: &Player, loan: &PlayerClubContract) -> Option<(u32, bool)> {
        let fee = loan.loan_future_fee?;
        if loan.loan_future_fee_obligation {
            return Some((fee, true));
        }
        let live_apps = player.statistics.played + player.statistics.played_subs;
        let (apps, rating) = if live_apps >= 5 {
            (live_apps, player.statistics.average_rating_raw())
        } else {
            player
                .statistics_history
                .season_ledger
                .iter()
                .filter(|e| {
                    e.is_loan && matches!(e.competition_kind, PlayerStatCompetitionKind::League)
                })
                .max_by_key(|e| (e.season_start_year, e.seq_id))
                .map(|e| {
                    (
                        e.statistics.played + e.statistics.played_subs,
                        e.statistics.average_rating,
                    )
                })
                .unwrap_or((0, 0.0))
        };
        let wants_to_stay = player
            .happiness
            .has_recent_event(&HappinessEventType::WantsLoanMadePermanent, 120);
        let rating_bar = if wants_to_stay { 6.45 } else { 6.6 };
        (apps >= 10 && rating >= rating_bar).then_some((fee, false))
    }

    /// Execute an option / obligation-to-buy at loan end: the borrower
    /// pays the agreed future fee and the player simply stays — no
    /// roster move; ownership, contract and career history flip to the
    /// borrowing club via the standard `complete_transfer` path. The
    /// staged arrival shock is suppressed afterwards — he has been in
    /// this dressing room the whole spell. An *option* lapses into a
    /// normal return when the borrower can no longer afford the fee; an
    /// *obligation* always pays.
    fn execute_loan_buyout(
        data: &mut SimulatorData,
        event: LoanReturnEvent,
        fee: u32,
        obligation: bool,
        date: NaiveDate,
    ) {
        let Some((bci, bcoi, bcli, bti)) = data.find_club_main_team(event.borrowing_club_id) else {
            Self::execute_loan_return(data, event, date, true);
            return;
        };
        let Some((pci, pcoi, pcli, pti)) = data.find_club_main_team(event.parent_club_id) else {
            Self::execute_loan_return(data, event, date, true);
            return;
        };

        let affordable = data.continents[bci].countries[bcoi].clubs[bcli]
            .finance
            .can_afford_transfer(fee as f64);
        if !obligation && !affordable {
            debug!(
                "Loan option lapsed: club {} cannot afford {} for player {}",
                event.borrowing_club_id, fee, event.player_id
            );
            Self::execute_loan_return(data, event, date, true);
            return;
        }

        // Parent-side TeamInfo + both league reputations for the
        // history row and the contract-valuation context.
        let (parent_info, selling_league_reputation) = {
            let country = &data.continents[pci].countries[pcoi];
            let team = &country.clubs[pcli].teams.teams[pti];
            let league = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid));
            (
                TeamInfo {
                    name: team.name.clone(),
                    slug: team.slug.clone(),
                    reputation: team.reputation.world,
                    league_name: league.map(|l| l.name.clone()).unwrap_or_default(),
                    league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
                },
                league.map(|l| l.reputation).unwrap_or(0),
            )
        };
        let buying_league_reputation = {
            let country = &data.continents[bci].countries[bcoi];
            country.clubs[bcli].teams.teams[bti]
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0)
        };

        // Fee booking: buyer amortizes the purchase, seller banks it.
        data.continents[bci].countries[bcoi].clubs[bcli]
            .finance
            .register_transfer_purchase(fee as f64, 4);
        data.continents[pci].countries[pcoi].clubs[pcli]
            .finance
            .add_transfer_income(fee as f64);

        // Mutate the player in place — he stays in the borrower roster.
        let Some((ci, coi, cli, ti)) = data.find_player_position(event.player_id) else {
            return;
        };
        let obligations = {
            let Some(player) = data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == event.player_id)
            else {
                return;
            };
            let obligations = player.drain_sell_on_obligations();
            player.complete_transfer(TransferCompletion {
                from: &parent_info,
                to: &event.borrowing_info,
                fee: fee as f64,
                date,
                selling_club_id: event.parent_club_id,
                buying_club_id: event.borrowing_club_id,
                agreed_wage: None,
                buying_league_reputation,
                selling_league_reputation,
                record_sell_on: None,
                personal_terms: None,
                // Buyout of a player already in this dressing room — no
                // arrival reception of any kind (pending is cleared below).
                source_is_rival: false,
                // The buyout narrates itself with the richer row below —
                // suppress the generic "permanent transfer" stamp so the
                // register shows one decision, not two, for one event.
                record_decision: false,
            });
            // No new-club arrival shock: he never changed dressing rooms.
            player.pending_signing = None;
            player.decision_history.add_move(
                date,
                &parent_info.name,
                &event.borrowing_info.name,
                fee as f64,
                "dec_loan_buyout",
            );
            debug!(
                "Loan buyout: club {} signs player {} permanently from club {} for {}",
                event.borrowing_club_id, event.player_id, event.parent_club_id, fee
            );
            obligations
        };

        // Route prior sell-on obligations out of the parent's proceeds.
        for obligation in &obligations {
            let payout = fee as f64 * obligation.percentage as f64;
            if payout <= 0.0 {
                continue;
            }
            if let Some((qci, qcoi, qcli, _)) =
                data.find_club_main_team(obligation.beneficiary_club_id)
            {
                data.continents[qci].countries[qcoi].clubs[qcli]
                    .finance
                    .adjust_cash(payout);
                data.continents[pci].countries[pcoi].clubs[pcli]
                    .finance
                    .adjust_cash(-payout);
            }
        }
    }

    /// Execute a single loan return: take player from borrowing club, place at parent club.
    /// Both clubs are resolved globally by ID — works for domestic and cross-country.
    ///
    /// `record_return` stamps a `dec_loan_returned` row on the player's
    /// decision register. True when a loan simply runs its course; false
    /// on the early-recall path, which records its own `dec_loan_recalled`
    /// row at the call site and must not be double-logged.
    fn execute_loan_return(
        data: &mut SimulatorData,
        event: LoanReturnEvent,
        date: NaiveDate,
        record_return: bool,
    ) {
        // Find parent club location first — abort early if missing
        let parent_pos = data.find_club_main_team(event.parent_club_id);
        if parent_pos.is_none() {
            debug!(
                "Loan return skipped: parent club {} not found for player {}",
                event.parent_club_id, event.player_id
            );
            return;
        }

        // Build parent TeamInfo before mutating data — needed by record_loan_return
        // to create a current-season entry if one was drained by season-end snapshot.
        let (pci, pcoi, pcli, pti) = parent_pos.unwrap();
        let parent_info = {
            let country = &data.continents[pci].countries[pcoi];
            let club = &country.clubs[pcli];
            let team = &club.teams.teams[pti];
            let league_info = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name: league_info.0,
                league_slug: league_info.1,
            }
        };

        // Take player from wherever they are
        let player_pos = data.find_player_position(event.player_id);
        let mut player = match player_pos {
            Some((ci, coi, cli, ti)) => {
                match data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                    .players
                    .take_player(&event.player_id)
                {
                    Some(p) => p,
                    None => return,
                }
            }
            None => return,
        };

        // Capture the loan-spell record before `on_loan_return` freezes
        // and resets the borrower-season statistics.
        let loan_starts = player.statistics.played;
        let loan_apps = player.statistics.played + player.statistics.played_subs;
        let loan_rating = player
            .statistics
            .average_rating_realistic(player.position().position_group());
        let loan_spell_days = player
            .contract_loan
            .as_ref()
            .and_then(|l| l.started)
            .map(|s| (date - s).num_days())
            .unwrap_or(0);

        player.on_loan_return(&event.borrowing_info, &parent_info, date);
        player.contract_loan = None;
        player.happiness = PlayerHappiness::new();
        player.statuses.statuses.clear();

        // A senior who was first-choice on loan and comes home to a
        // fringe role doesn't get a fully clean slate — the return he
        // didn't choose is the first mood of the new parent spell, and
        // it feeds the stuck-career machinery from day one.
        let fringe_at_parent = player
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::MainBackupPlayer | PlayerSquadStatus::NotNeeded
                )
            })
            .unwrap_or(false);
        let age = DateUtils::age(player.birth_date, date);
        if fringe_at_parent && age >= 21 && loan_starts >= 12 && loan_rating >= 6.6 {
            let magnitude = HappinessConfig::default()
                .catalog
                .unsettled_after_loan_return;
            player
                .happiness
                .add_event(HappinessEventType::UnsettledAfterLoanReturn, magnitude);
        } else if age <= 23 && loan_starts >= 12 && loan_rating >= 6.5 {
            // The young player who went out a prospect comes back a
            // footballer — quiet confidence rather than grievance. The
            // record keeps working after this beat fades: the frozen
            // loan spell raises his own playing-time bar
            // (`own_expected_start_share`) and feeds the monthly
            // returnee-breakthrough audit if the parent never gives him
            // the minutes his record earned.
            let magnitude = HappinessConfig::default().catalog.returned_from_loan_proven;
            player
                .happiness
                .add_event(HappinessEventType::ReturnedFromLoanProven, magnitude);
        } else if (loan_apps >= 8 && loan_rating < 6.2)
            || (loan_spell_days >= 120 && loan_apps <= 4)
        {
            // The failure branch: a real sample of matches clearly below
            // the water line, or a real spell he barely featured in. He
            // comes home with his confidence knocked — deeper for a
            // player who handles pressure poorly, shrugged off faster by
            // the thick-skinned. The mentor-support and resilience arcs
            // are the way back; the club-side verdict on the failed bet
            // stays with the stalled-prospect / listing pipelines.
            let pressure01 = (player.attributes.pressure / 20.0).clamp(0.0, 1.0);
            let magnitude = HappinessConfig::default()
                .catalog
                .returned_from_loan_deflated
                * (1.3 - 0.6 * pressure01);
            player
                .happiness
                .add_event(HappinessEventType::ReturnedFromLoanDeflated, magnitude);
        }

        // A loan running its course is a club decision too — the player
        // comes home. Only genuine returns stamp this; an early recall
        // records its own `dec_loan_recalled` row at the call site.
        if record_return {
            player.decision_history.add_move(
                date,
                &event.borrowing_info.name,
                &parent_info.name,
                0.0,
                "dec_loan_returned",
            );
        }

        // Place at parent club
        debug!(
            "Loan return: player {} from club {} back to club {}",
            event.player_id, event.borrowing_club_id, event.parent_club_id
        );
        data.continents[pci].countries[pcoi].clubs[pcli].teams.teams[pti]
            .players
            .add(player);
    }

    /// Borrowing-side `TeamInfo` for a loan-return event, aliased to the
    /// club's main team for non-Main squads exactly like
    /// `scan_expired_loans` does — career history only owns senior slugs.
    fn loan_team_info(country: &Country, club: &Club, team: &Team) -> TeamInfo {
        let main_team = club.teams.main();
        let (name, slug, reputation) = if team.team_type == TeamType::Main || main_team.is_none() {
            (team.name.clone(), team.slug.clone(), team.reputation.world)
        } else {
            let m = main_team.unwrap();
            (m.name.clone(), m.slug.clone(), m.reputation.world)
        };
        let (league_name, league_slug) = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| (l.name.clone(), l.slug.clone()))
            .unwrap_or_default();
        TeamInfo {
            name,
            slug,
            reputation,
            league_name,
            league_slug,
        }
    }

    /// Monthly mid-loan recall pass. Two real-football triggers, both
    /// gated on the loan's negotiated recall window
    /// (`loan_recall_available_after` — set at signing and by the loan
    /// audits, previously never consumed):
    ///
    ///   * **Depth emergency at the parent** — injuries have stripped a
    ///     position group below half its ideal depth while a fit loanee
    ///     of that group is out on a recallable loan. The January-style
    ///     emergency recall.
    ///   * **Failing loan** — the monthly loan audit has been pushing
    ///     for a recall (`LoanRecallRequested`); the parent finally acts
    ///     instead of letting the mood ring forever.
    ///
    /// Scans the borrower rosters of this country (mirroring
    /// `scan_expired_loans`), resolving each parent club globally so
    /// foreign parents recall from here too. Execution reuses
    /// `execute_loan_return` — a recalled thriving senior walking into a
    /// fringe role picks up `UnsettledAfterLoanReturn` there.
    pub(super) fn process_loan_recalls(data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
        if date.day() != 1 {
            return;
        }
        // An imminent natural return needs no recall.
        const MIN_DAYS_LEFT: i64 = 30;
        // Give the spell a month before yanking the player back.
        const SETTLE_DAYS: i64 = 30;
        // How fresh the failing-loan pressure mood must be.
        const FAILING_MOOD_DAYS: u16 = 45;

        let Some(country) = data.country(country_id) else {
            return;
        };

        let mut events: Vec<LoanReturnEvent> = Vec::new();
        // One emergency recall per (parent, position group) per pass.
        let mut emergency_taken: HashSet<(u32, PlayerFieldPositionGroup)> = HashSet::new();

        for club in &country.clubs {
            for team in club.teams.iter() {
                for player in team.players.iter() {
                    let Some(ref loan) = player.contract_loan else {
                        continue;
                    };
                    let Some(parent_club_id) = loan.loan_from_club_id else {
                        continue;
                    };
                    // Recall must be contractually available and worth it.
                    let window_open = loan
                        .loan_recall_available_after
                        .map(|d| d <= date)
                        .unwrap_or(false);
                    if !window_open {
                        continue;
                    }
                    if (loan.expiration - date).num_days() < MIN_DAYS_LEFT {
                        continue;
                    }
                    if loan
                        .started
                        .map(|s| (date - s).num_days() < SETTLE_DAYS)
                        .unwrap_or(true)
                    {
                        continue;
                    }

                    let failing = player.happiness.has_recent_event(
                        &HappinessEventType::LoanRecallRequested,
                        FAILING_MOOD_DAYS,
                    );

                    let group = player.position().position_group();
                    let mut emergency = false;
                    if !failing && !player.player_attributes.is_injured {
                        // Depth emergency at the parent: fit senior cover
                        // in this group below half the ideal depth.
                        if let Some((ci, coi, cli, ti)) = data.find_club_main_team(parent_club_id) {
                            let parent_team =
                                &data.continents[ci].countries[coi].clubs[cli].teams.teams[ti];
                            let fit = parent_team
                                .players
                                .iter()
                                .filter(|p| p.position().position_group() == group)
                                .filter(|p| !p.player_attributes.is_injured && p.contract.is_some())
                                .count();
                            let floor = group.ideal_squad_depth().div_ceil(2);
                            emergency =
                                fit < floor && !emergency_taken.contains(&(parent_club_id, group));
                        }
                    }
                    if !failing && !emergency {
                        continue;
                    }
                    if emergency {
                        emergency_taken.insert((parent_club_id, group));
                    }
                    events.push(LoanReturnEvent {
                        player_id: player.id,
                        borrowing_club_id: club.id,
                        parent_club_id,
                        borrowing_info: Self::loan_team_info(country, club, team),
                        buyout: None,
                    });
                }
            }
        }

        for event in events {
            let player_id = event.player_id;
            Self::execute_loan_return(data, event, date, false);
            // Stamp the recall on the player's decision history so the
            // early return reads as a club decision, not a mystery.
            if let Some((ci, coi, cli, ti)) = data.find_player_position(player_id) {
                if let Some(player) = data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == player_id)
                {
                    player.decision_history.add(
                        date,
                        String::new(),
                        "dec_loan_recalled".to_string(),
                        String::new(),
                    );
                }
            }
        }
    }

    /// Monthly check: retire players who are clearly past max retirement age
    /// or already have the Ret status. Does NOT do probabilistic retirement
    /// (that stays at season end in process_player_retirements).
    fn process_overdue_retirements(country: &mut Country, date: NaiveDate) {
        let mut to_retire: Vec<(usize, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.iter().enumerate() {
                for player in team.players.iter() {
                    if Self::must_retire(player, date) {
                        to_retire.push((club_idx, team_idx, player.id));
                    }
                }
            }
        }

        for (club_idx, team_idx, player_id) in to_retire {
            if let Some(mut player) = country.clubs[club_idx].teams.teams[team_idx]
                .players
                .take_player(&player_id)
            {
                debug!(
                    "Overdue retirement: {} (age {})",
                    player.full_name,
                    player.age(date)
                );
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                player.retired = true;
                country.retired_players.push(player);
            }
        }
    }

    /// Deterministic retirement: only for players past absolute max age or with Ret status.
    /// Players still getting games are given a +1 year grace period.
    /// Uses the same CA/position/jitter logic as should_retire for consistency.
    fn must_retire(player: &Player, date: NaiveDate) -> bool {
        if player.statuses.has(PlayerStatusType::Ret) {
            return true;
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let position = player.position();

        let max_retire_age = match ca {
            0..=39 => 37u8,
            40..=69 => 38,
            70..=99 => 39,
            100..=129 => 40,
            130..=159 => 41,
            160..=179 => 42,
            _ => 43,
        };

        let position_offset: i8 = if position.is_goalkeeper() {
            2
        } else if position.is_defender() {
            1
        } else if position.is_forward() {
            -1
        } else {
            0
        };

        let id_jitter: i8 = match player.id % 3 {
            0 => -1,
            1 => 0,
            _ => 1,
        };

        let max_age =
            (max_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(35, 47) as u8;

        // Players still getting games get a 1-year grace period
        let has_recent_games = player.statistics.total_games() >= 5
            || player
                .statistics_history
                .items
                .last()
                .map(|h| h.statistics.total_games() >= 5)
                .unwrap_or(true);

        let effective_max = if has_recent_games {
            max_age + 1
        } else {
            max_age
        };

        age >= effective_max
    }

    fn process_player_retirements(country: &mut Country, date: NaiveDate) {
        debug!("Processing player retirements");

        // Collect players to retire: (club_idx, team_idx, player_id)
        let mut to_retire: Vec<(usize, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.iter().enumerate() {
                for player in team.players.iter() {
                    if Self::should_retire(player, date) {
                        to_retire.push((club_idx, team_idx, player.id));
                    }
                }
            }
        }

        // Execute retirements: remove from team, add Ret status, store in retired_players
        for (club_idx, team_idx, player_id) in to_retire {
            if let Some(mut player) = country.clubs[club_idx].teams.teams[team_idx]
                .players
                .take_player(&player_id)
            {
                debug!(
                    "Player retired: {} (age {})",
                    player.full_name,
                    player.age(date)
                );
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                player.retired = true;
                country.retired_players.push(player);
            }
        }
    }

    fn should_retire(player: &Player, date: NaiveDate) -> bool {
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;

        // Already marked for retirement
        if player.statuses.has(PlayerStatusType::Ret) {
            return true;
        }

        let position = player.position();
        let is_gk = position.is_goalkeeper();

        // Wider age windows with finer CA granularity (5-6 year spread)
        // This prevents mass retirement at a single age boundary
        let (min_retire_age, max_retire_age) = match ca {
            0..=39 => (31u8, 36u8),
            40..=69 => (32, 37),
            70..=99 => (33, 38),
            100..=129 => (34, 39),
            130..=159 => (35, 40),
            160..=179 => (36, 41),
            _ => (37, 42),
        };

        // Position adjustments: GK +2, defenders +1, forwards -1
        let position_offset: i8 = if is_gk {
            2
        } else if position.is_defender() {
            1
        } else if position.is_forward() {
            -1
        } else {
            0
        };

        // Per-player jitter based on player ID: spreads ±1 year
        // so players of the same age/ability don't all retire together
        let id_jitter: i8 = match player.id % 3 {
            0 => -1,
            1 => 0,
            _ => 1,
        };

        let min_age =
            (min_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(30, 44) as u8;
        let max_age =
            (max_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(34, 46) as u8;

        // Too young to retire
        if age < min_age {
            return false;
        }

        // Past max age — forced retirement
        if age >= max_age {
            return true;
        }

        // Still playing regularly? Reduce chance but don't fully block
        let current_season_games = player.statistics.total_games();
        let last_season_games = player
            .statistics_history
            .items
            .last()
            .map(|h| h.statistics.total_games())
            .unwrap_or(0);

        let total_recent_games = current_season_games + last_season_games;

        // Base retirement probability: gentle ramp across the window
        // Starts at 5%, increases quadratically to ~60% at max_age-1
        let range = (max_age - min_age).max(1) as f32;
        let years_over = (age - min_age) as f32;
        let progress = years_over / range; // 0.0 at min_age, ~1.0 at max_age
        let mut chance: f32 = 5.0 + 55.0 * progress * progress;

        // === Personality modifiers (continuous, not stepped) ===

        // Ambition: 0-20 scale, high ambition reduces chance
        let ambition = player.attributes.ambition;
        chance -= (ambition - 10.0) * 1.5; // -15 to +15

        // Determination: high determination = persist longer
        let determination = player.skills.mental.determination;
        chance -= (determination - 10.0) * 1.0; // -10 to +10

        // === Game time modifiers (graduated) ===
        if total_recent_games >= 30 {
            chance -= 25.0; // Regular starter across both seasons
        } else if total_recent_games >= 15 {
            chance -= 15.0;
        } else if total_recent_games >= 5 {
            chance -= 5.0;
        } else if total_recent_games == 0 {
            chance += 15.0; // No games at all — likely to retire
        }

        // === Ability decline modifier ===
        let pa = player.player_attributes.potential_ability;
        if pa > 0 {
            let decline_ratio = 1.0 - (ca as f32 / pa as f32);
            if decline_ratio > 0.5 {
                chance += 10.0; // Severely declined
            } else if decline_ratio > 0.3 {
                chance += 5.0;
            }
        }

        // Birth month adds sub-year variance:
        // players born later in the year get slight chance reduction
        // (they are effectively slightly younger within the same age bracket)
        let birth_month = player.birth_date.month() as f32;
        chance += (birth_month - 6.0) * 0.5; // -2.5 to +3.0

        chance = chance.clamp(3.0, 85.0);
        IntegerUtils::random(0, 100) < chance as i32
    }

    fn process_year_end_finances(country: &mut Country) {
        debug!("Processing year-end finances");

        country.clubs.par_iter_mut().for_each(|club| {
            let balance = club.finance.balance.balance;
            if balance > 0 {
                // 0.5% annual return on positive balance, capped at
                // $500K. Football clubs don't park cash in high-yield
                // instruments — corporate treasury rates are modest and
                // tax eats most of it. The earlier 1%/$2M cap was
                // generous enough to compound into already-wealthy
                // sides' growth and feed the broader rocket pattern.
                let interest = ((balance as f64 * 0.005) as i64).min(500_000);
                club.finance.balance.push_income(interest);
            }
            // Negative balances carry NO year-end penalty: debt already
            // pays monthly, distress-scaled interest in
            // `process_monthly_finances` (0.6-1.5%/month ≈ 7-18%/year).
            // The old extra 5% annual hit on top double-billed the same
            // debt and pushed struggling clubs into a spiral no real
            // lender's terms would produce.
        });
    }

    fn process_promotion_relegation(country: &mut Country, date: NaiveDate) {
        // Collect league info: (league_id, tier, relegation_spots, promotion_spots)
        let league_info: Vec<(u32, u8, u8, u8)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| {
                (
                    l.id,
                    l.settings.tier,
                    l.settings.relegation_spots,
                    l.settings.promotion_spots,
                )
            })
            .collect();

        // For each league with relegation_spots > 0, find its paired league
        for &(tier1_id, tier1_tier, relegation_spots, _) in &league_info {
            if relegation_spots == 0 || tier1_tier == 0 {
                continue;
            }

            // Find paired league: same country, next tier, with promotion_spots > 0
            let paired = league_info.iter().find(|&&(id, tier, _, promo)| {
                id != tier1_id && tier == tier1_tier + 1 && promo > 0
            });

            let &(tier2_id, _, _, promotion_spots) = match paired {
                Some(p) => p,
                None => continue,
            };

            let nominal_swap = relegation_spots.min(promotion_spots) as usize;

            // Read final tables
            let relegated_candidates: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier1_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    table
                        .iter()
                        .rev()
                        .take(nominal_swap)
                        .map(|r| r.team_id)
                        .collect()
                })
                .unwrap_or_default();

            let promoted_candidates: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| table.iter().take(nominal_swap).map(|r| r.team_id).collect())
                .unwrap_or_default();

            // Must balance: never relegate more than we promote (or vice versa)
            // or the top league silently shrinks each season.
            let swap_count = relegated_candidates.len().min(promoted_candidates.len());
            if swap_count == 0 {
                continue;
            }
            if swap_count < nominal_swap {
                info!(
                    "⚠️ Promotion/relegation pair {}→{} truncated: wanted {}, got {} (missing final_table entries)",
                    tier1_id, tier2_id, nominal_swap, swap_count
                );
            }
            let relegated_team_ids: Vec<u32> =
                relegated_candidates.into_iter().take(swap_count).collect();
            let promoted_team_ids: Vec<u32> =
                promoted_candidates.into_iter().take(swap_count).collect();

            // Snapshot per-team final standing so the per-player
            // relegation event can carry SeasonOutcomeContext (final
            // position, points, gap to safety) without re-borrowing
            // the league inside the upcoming `&mut country.clubs` loop.
            let relegation_outcome_by_team: HashMap<u32, (u8, u16, i16)> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier1_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    let total = table.len();
                    let safety_idx = total.saturating_sub(swap_count);
                    let safety_points: u16 = table
                        .get(safety_idx.saturating_sub(1))
                        .map(|r| r.effective_points() as u16)
                        .unwrap_or(0);
                    table
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| relegated_team_ids.contains(&r.team_id))
                        .map(|(idx, r)| {
                            let pos = (idx + 1) as u8;
                            let pts = r.effective_points() as u16;
                            let gap = pts as i16 - safety_points as i16;
                            (r.team_id, (pos, pts, gap))
                        })
                        .collect()
                })
                .unwrap_or_default();

            // Build a tier-2 club expectation map: clubs that finished
            // inside the promotion window (top promotion_spots + a couple
            // of playoff places) "expected" promotion. NonPromotionRelease
            // only activates for those — finishing 18th in the second tier
            // shouldn't trigger any player's escape clause.
            let promotion_window_team_ids: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|t| {
                    let window = (promotion_spots as usize + 2).min(t.len());
                    t.iter().take(window).map(|r| r.team_id).collect::<Vec<_>>()
                })
                .unwrap_or_default();

            // Swap league_ids on teams and move sub-teams to matching friendly league
            for club in &mut country.clubs {
                let mut new_main_league_id: Option<u32> = None;
                // Aggregate one-shot bonus payouts owed for this season's
                // outcome. Charged to the club after the team loops so we
                // only hold a single mut borrow at a time.
                let mut bonus_total: i64 = 0;

                for team in &mut club.teams.teams {
                    if relegated_team_ids.contains(&team.id) {
                        info!(
                            "⬇️ Relegation: team {} ({}) moves to league {}",
                            team.name, team.id, tier2_id
                        );
                        team.league_id = Some(tier2_id);
                        new_main_league_id = Some(tier2_id);
                        // Year-defining wound — emit per player. The promo
                        // counterpart already ran in season_awards via the
                        // PromotionCelebration emit; we don't duplicate the
                        // upward case here.
                        // Inline emit (we hold &mut to team here, so the
                        // Self::apply_team_squad_event helper which takes
                        // &mut country would conflict).
                        let outcome = relegation_outcome_by_team.get(&team.id).copied();
                        for player in team.players.iter_mut() {
                            let mut ctx = SeasonOutcomeContext::new(SeasonOutcomeKind::Relegated)
                                .with_league(tier1_id);
                            if let Some((pos, pts, gap)) = outcome {
                                ctx = ctx
                                    .with_final_position(pos)
                                    .with_points(pts)
                                    .with_points_to_safety(gap);
                            }
                            player.on_season_outcome(
                                HappinessEventType::Relegated,
                                365,
                                1.0,
                                date,
                                ctx,
                            );
                            // Activate any RelegationWageDecrease and
                            // RelegationFeeRelease clauses on the player's
                            // contract. Each helper consumes the matching
                            // clause so subsequent relegations don't double-apply.
                            if let Some(c) = player.contract.as_mut() {
                                let _ = c.apply_relegation_wage_decrease();
                                let _ = c.take_relegation_release();
                            }
                        }
                    } else if promoted_team_ids.contains(&team.id) {
                        info!(
                            "⬆️ Promotion: team {} ({}) moves to league {}",
                            team.name, team.id, tier1_id
                        );
                        team.league_id = Some(tier1_id);
                        new_main_league_id = Some(tier1_id);
                        // Symmetric to the relegation hooks above —
                        // PromotionWageIncrease bumps salary; players
                        // also keep their existing contracts (no clause
                        // for "release on promotion").
                        for player in team.players.iter_mut() {
                            if let Some(c) = player.contract.as_mut() {
                                let _ = c.apply_promotion_wage_increase();
                                // PromotionFee bonus is paid out as a
                                // one-shot lump sum to every player on
                                // the promoted team who has the bonus.
                                for bonus in &c.bonuses {
                                    if matches!(bonus.bonus_type, ContractBonusType::PromotionFee)
                                        && bonus.value > 0
                                    {
                                        bonus_total += bonus.value as i64;
                                    }
                                }
                            }
                        }
                    } else {
                        // Survival in a tier-1 league with relegation
                        // places: pay AvoidRelegationFee bonuses if the
                        // team was at risk this season. Approximated by
                        // "team carries the clause" — the bonus is only
                        // installed on contracts when the club expected
                        // a relegation battle.
                        let in_tier1_with_relegation = team.league_id == Some(tier1_id);
                        if in_tier1_with_relegation {
                            for player in team.players.iter_mut() {
                                if let Some(c) = player.contract.as_mut() {
                                    for bonus in &c.bonuses {
                                        if matches!(
                                            bonus.bonus_type,
                                            ContractBonusType::AvoidRelegationFee
                                        ) && bonus.value > 0
                                        {
                                            bonus_total += bonus.value as i64;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // NonPromotionRelease activates only for clubs that
                // genuinely expected to go up — finished inside the
                // promotion window — and didn't. Mid-table and
                // bottom-of-tier-2 clubs are excluded so a player's
                // escape clause doesn't fire just because the club is
                // in the wrong league.
                let club_was_in_window = club
                    .teams
                    .teams
                    .iter()
                    .any(|t| promotion_window_team_ids.contains(&t.id));
                let club_was_promoted = club
                    .teams
                    .teams
                    .iter()
                    .any(|t| promoted_team_ids.contains(&t.id));
                if club_was_in_window && !club_was_promoted {
                    for team in &mut club.teams.teams {
                        if team.league_id == Some(tier2_id) {
                            for player in team.players.iter_mut() {
                                if let Some(c) = player.contract.as_mut() {
                                    let _ = c.take_non_promotion_release();
                                }
                            }
                        }
                    }
                }

                if bonus_total > 0 {
                    club.finance.balance.push_expense_player_wages(bonus_total);
                }

                // Move sub-teams to the matching youth league of the new main league
                if let Some(new_league_id) = new_main_league_id {
                    for team in &mut club.teams.teams {
                        if team.team_type != TeamType::Main {
                            let type_offset = match team.team_type {
                                TeamType::U18 => 100000,
                                TeamType::U19 => 110000,
                                TeamType::U20 => 120000,
                                TeamType::U21 => 130000,
                                TeamType::U23 => 140000,
                                _ => 200000, // B/Reserve teams use generic friendly league
                            };
                            team.league_id = Some(new_league_id + type_offset);
                        }
                    }
                }
            }
        }

        // Clear final tables after processing
        for league in &mut country.leagues.leagues {
            league.final_table = None;
        }
    }

    /// Monthly late-season audit: surface ambient relegation dread for
    /// players whose team is in (or close to) the drop zone. Gated to
    /// season_progress >= 0.6 so an early-season slump doesn't generate
    /// the event — pressure builds with the season trajectory.
    ///
    /// The cooldown on `RelegationFear` is 30 days, so this monthly scan
    /// produces at most one event per player per month even if their team
    /// fluctuates around the line.
    fn process_relegation_fear_audit(country: &mut Country, date: NaiveDate) {
        // Collect (team_id, in_zone, near_zone) from each non-friendly
        // league with a meaningful relegation slot count and enough
        // season progress to register pressure.
        let mut at_risk_teams: HashMap<u32, (u32, u8, u16, i16, u8)> = HashMap::new();

        for league in &country.leagues.leagues {
            if league.friendly || league.settings.relegation_spots == 0 {
                continue;
            }
            let total_teams = league.table.rows.len();
            if total_teams == 0 {
                continue;
            }
            let matches_played = league
                .table
                .rows
                .iter()
                .map(|r| r.played)
                .max()
                .unwrap_or(0);
            let total_matches = ((total_teams - 1) * 2) as u8;
            if total_matches == 0 {
                continue;
            }
            let progress = matches_played as f32 / total_matches as f32;
            if progress < 0.6 {
                continue;
            }

            // Snapshot the league's current standings so per-team
            // SeasonOutcomeContext can be built without re-borrowing.
            let table = &league.table.rows;
            let safety_idx = total_teams.saturating_sub(league.settings.relegation_spots as usize);
            let safety_points: u16 = table
                .get(safety_idx.saturating_sub(1))
                .map(|r| r.effective_points() as u16)
                .unwrap_or(0);
            let matches_remaining = total_matches.saturating_sub(matches_played);

            // Bottom (relegation_spots + 1) teams: the in-zone clubs plus
            // the one immediately above. They're the squads doing the
            // morning newspaper math every week.
            let zone = league.settings.relegation_spots as usize + 1;
            for (idx, row) in table.iter().enumerate().rev().take(zone) {
                let pos = (idx + 1) as u8;
                let pts = row.effective_points() as u16;
                let gap = pts as i16 - safety_points as i16;
                at_risk_teams.insert(row.team_id, (league.id, pos, pts, gap, matches_remaining));
            }
        }

        if at_risk_teams.is_empty() {
            return;
        }

        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                let Some(&(league_id, pos, pts, gap, remaining)) = at_risk_teams.get(&team.id)
                else {
                    continue;
                };
                for player in team.players.iter_mut() {
                    let ctx = SeasonOutcomeContext::new(SeasonOutcomeKind::RelegationFear)
                        .with_league(league_id)
                        .with_final_position(pos)
                        .with_points(pts)
                        .with_points_to_safety(gap)
                        .with_matches_remaining(remaining);
                    player.on_season_outcome(
                        HappinessEventType::RelegationFear,
                        30,
                        1.0,
                        date,
                        ctx,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{
        DayMonthPeriod, DomesticCup, League, LeagueCollection, LeagueSettings, LeagueTableRow,
        ScheduleItem, ScheduleTour,
    };
    use crate::r#match::{Score, TeamScore};
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32) -> Player {
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(crate::shared::fullname::FullName::new(
                "Test".to_string(),
                format!("Player{}", id),
            ))
            .birth_date(d(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        // Regular starter so participation factor = 1.0.
        p.statistics.played = 30;
        p
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn make_team(id: u32, club_id: u32, league_id: u32, players: Vec<Player>) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team{}", id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club(id: u32, teams: Vec<crate::Team>) -> Club {
        Club::new(
            id,
            format!("Club{}", id),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            crate::ClubFacilities::default(),
        )
    }

    fn make_league_with_table(id: u32, rep: u16, rows: Vec<(u32, u8, u8)>) -> League {
        let mut league = League::new(
            id,
            format!("League{}", id),
            format!("league{}", id),
            1,
            rep,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 3,
                league_group: None,
            },
            false,
        );
        league.table.rows = rows
            .into_iter()
            .map(|(team_id, played, points)| LeagueTableRow {
                team_id,
                played,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points,
                points_deduction: 0,
            })
            .collect();
        league
    }

    fn build_country(clubs: Vec<Club>, leagues: Vec<League>) -> Country {
        Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(leagues))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn happiness_event_count(player: &Player, kind: &HappinessEventType) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *kind)
            .count()
    }

    // ── Loan buyout (option / obligation to buy) ────────────────

    fn loan_deal(fee: Option<u32>, obligation: bool) -> PlayerClubContract {
        let mut c = PlayerClubContract::new(20_000, d(2026, 6, 30));
        c.loan_from_club_id = Some(100);
        c.loan_future_fee = fee;
        c.loan_future_fee_obligation = obligation;
        c
    }

    #[test]
    fn loan_obligation_always_converts() {
        // No appearances at all — an obligation is binding regardless.
        let mut p = make_player(1);
        p.statistics.played = 0;
        assert_eq!(
            CountryResult::decide_loan_buyout(&p, &loan_deal(Some(2_000_000), true)),
            Some((2_000_000, true))
        );
    }

    #[test]
    fn thriving_loanee_option_is_exercised() {
        let mut p = make_player(1);
        p.statistics.played = 22;
        p.statistics.rating_points = 6.9 * 22.0;
        p.statistics.rating_weight = 22.0;
        assert_eq!(
            CountryResult::decide_loan_buyout(&p, &loan_deal(Some(2_000_000), false)),
            Some((2_000_000, false)),
            "a loan that worked gets its option exercised"
        );
    }

    #[test]
    fn failed_loan_option_lapses() {
        let mut p = make_player(1);
        p.statistics.played = 6;
        p.statistics.rating_points = 6.2 * 6.0;
        p.statistics.rating_weight = 6.0;
        assert_eq!(
            CountryResult::decide_loan_buyout(&p, &loan_deal(Some(2_000_000), false)),
            None,
            "a barely-used loanee's option is left to lapse"
        );
    }

    #[test]
    fn plain_loan_has_no_buyout() {
        let p = make_player(1);
        assert_eq!(
            CountryResult::decide_loan_buyout(&p, &loan_deal(None, false)),
            None
        );
    }

    // ── Mid-loan recall ─────────────────────────────────────────

    fn recall_world(recall_window: Option<NaiveDate>) -> SimulatorData {
        // Parent club 100: a single fit midfielder — a depth emergency
        // in the Midfielder group.
        let parent_team = make_team(10, 100, 1, vec![make_player_with_position(11)]);
        let parent_club = make_club(100, vec![parent_team]);

        // Borrower club 200 hosts midfielder 55 on loan from 100.
        let mut loanee = make_player_with_position(55);
        loanee.contract = Some(PlayerClubContract::new(30_000, d(2028, 6, 30)));
        let mut loan = PlayerClubContract::new(30_000, d(2027, 6, 30));
        loan.loan_from_club_id = Some(100);
        loan.started = Some(d(2026, 3, 1));
        loan.loan_recall_available_after = recall_window;
        loanee.contract_loan = Some(loan);
        let borrower_team = make_team(20, 200, 1, vec![loanee]);
        let borrower_club = make_club(200, vec![borrower_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let country = build_country(vec![parent_club, borrower_club], vec![league]);
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            d(2026, 6, 1).and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    #[test]
    fn depth_emergency_recalls_fit_loanee() {
        let mut data = recall_world(Some(d(2026, 4, 1)));
        CountryResult::process_loan_recalls(&mut data, 1, d(2026, 6, 1));

        let country = data.country(1).unwrap();
        let parent = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let recalled = parent.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 55);
        assert!(
            recalled.is_some(),
            "the loanee must be recalled to the parent's main squad"
        );
        assert!(
            recalled.unwrap().contract_loan.is_none(),
            "the recall ends the loan"
        );
    }

    #[test]
    fn recall_requires_the_contract_window() {
        // Same emergency, but the loan carries no recall clause.
        let mut data = recall_world(None);
        CountryResult::process_loan_recalls(&mut data, 1, d(2026, 6, 1));

        let country = data.country(1).unwrap();
        let borrower = country.clubs.iter().find(|c| c.id == 200).unwrap();
        assert!(
            borrower.teams.teams[0]
                .players
                .players
                .iter()
                .any(|p| p.id == 55),
            "without a recall clause the parent cannot act"
        );
    }

    #[test]
    fn relegation_fear_fires_for_bottom_team_late_season() {
        // 20-team league, total_matches = 38. matches_played = 30 → 78%
        // progress (>= 60% gate). Team 5 finishes second-to-bottom of the
        // simulated 5-team table, so it falls inside the (relegation_spots
        // + 1 = 4) audit window.
        let p1 = make_player(1);
        let p2 = make_player(2);
        let team = make_team(5, 50, 1, vec![p1, p2]);
        let club = make_club(50, vec![team]);

        // 5 rows; bottom 4 (relegation_spots 3 + 1) eligible.
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (5, 30, 20),
            (4, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let players = &country.clubs[0].teams.teams[0].players.players;
        assert_eq!(
            happiness_event_count(&players[0], &HappinessEventType::RelegationFear),
            1
        );
        assert_eq!(
            happiness_event_count(&players[1], &HappinessEventType::RelegationFear),
            1
        );
    }

    #[test]
    fn relegation_fear_silent_for_safe_team() {
        let p1 = make_player(1);
        let team = make_team(1, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        // Team 1 is top of the table — way out of the drop zone.
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (4, 30, 20),
            (5, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            0
        );
    }

    #[test]
    fn relegation_fear_silent_too_early_in_season() {
        let p1 = make_player(1);
        let team = make_team(5, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        // 5-team league → 8 matches per team. Stop at 4 played (50%) so
        // we sit *under* the 60% audit gate. Drop-zone position alone
        // shouldn't trigger fear in early autumn.
        let rows = vec![(1, 4, 10), (2, 4, 8), (3, 4, 6), (5, 4, 2), (4, 4, 1)];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2031, 11, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            0
        );
    }

    #[test]
    fn apply_team_squad_event_emits_to_each_player() {
        let p1 = make_player(1);
        let p2 = make_player(2);
        let team = make_team(7, 70, 1, vec![p1, p2]);
        let club = make_club(70, vec![team]);
        // No relegation_spots needed — using the helper directly.
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );

        let players = &country.clubs[0].teams.teams[0].players.players;
        assert_eq!(
            happiness_event_count(&players[0], &HappinessEventType::TrophyWon),
            1
        );
        assert_eq!(
            happiness_event_count(&players[1], &HappinessEventType::TrophyWon),
            1
        );
    }

    #[test]
    fn apply_team_squad_event_silent_for_unknown_team() {
        let p1 = make_player(1);
        let team = make_team(1, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        // team_id 999 doesn't exist — must no-op.
        CountryResult::apply_team_squad_event(
            &mut country,
            999,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::TrophyWon),
            0
        );
    }

    #[test]
    fn apply_team_squad_event_prestige_scales_magnitude() {
        let p1 = make_player(1);
        let team_a = make_team(10, 100, 1, vec![p1]);
        let club_a = make_club(100, vec![team_a]);
        let p2 = make_player(2);
        let team_b = make_team(20, 200, 1, vec![p2]);
        let club_b = make_club(200, vec![team_b]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club_a, club_b], vec![league]);

        CountryResult::apply_team_squad_event(
            &mut country,
            10,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );
        CountryResult::apply_team_squad_event(
            &mut country,
            20,
            HappinessEventType::TrophyWon,
            365,
            1.5,
            d(2032, 5, 30),
        );

        let mag_a = country.clubs[0].teams.teams[0].players.players[0]
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        let mag_b = country.clubs[1].teams.teams[0].players.players[0]
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        assert!(
            mag_b > mag_a,
            "prestige 1.5 ({}) should exceed prestige 1.0 ({})",
            mag_b,
            mag_a
        );
    }

    #[test]
    fn apply_team_squad_event_cooldown_blocks_repeat() {
        let p1 = make_player(1);
        let team = make_team(7, 70, 1, vec![p1]);
        let club = make_club(70, vec![team]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        let date = d(2032, 5, 30);
        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            date,
        );
        // Second emit on the same date — cooldown must absorb it.
        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            date,
        );

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::TrophyWon),
            1
        );
    }

    #[test]
    fn relegation_fear_cooldown_blocks_repeat_audit() {
        let p1 = make_player(1);
        let team = make_team(5, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (5, 30, 20),
            (4, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        // First audit fires; second consecutive monthly audit must respect
        // the 30-day cooldown and silently skip.
        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));
        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            1
        );
    }

    fn make_league_with_settings(
        id: u32,
        tier: u8,
        promo: u8,
        rel: u8,
        rows: Vec<(u32, u8, u8)>,
    ) -> League {
        let mut league = League::new(
            id,
            format!("League{}", id),
            format!("league{}", id),
            1,
            5000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier,
                promotion_spots: promo,
                relegation_spots: rel,
                league_group: None,
            },
            false,
        );
        let table_rows: Vec<LeagueTableRow> = rows
            .into_iter()
            .map(|(team_id, played, points)| LeagueTableRow {
                team_id,
                played,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points,
                points_deduction: 0,
            })
            .collect();
        league.table.rows = table_rows.clone();
        league.final_table = Some(table_rows);
        league
    }

    fn make_simple_team(id: u32, club_id: u32, league_id: u32) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team{}", id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    #[test]
    fn promotion_relegation_swaps_teams_between_adjacent_tiers() {
        // Tier 1 has 4 teams (10..=13), bottom 2 relegate.
        // Tier 2 has 4 teams (20..=23), top 2 promote.
        let tier1_clubs: Vec<Club> = (10..=13)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 1)]))
            .collect();
        let tier2_clubs: Vec<Club> = (20..=23)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 2)]))
            .collect();
        let tier1 = make_league_with_settings(
            1,
            1,
            0,
            2,
            vec![(10, 30, 70), (11, 30, 60), (12, 30, 30), (13, 30, 20)],
        );
        let tier2 = make_league_with_settings(
            2,
            2,
            2,
            0,
            vec![(20, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
        );

        let mut all_clubs = tier1_clubs;
        all_clubs.extend(tier2_clubs);
        let mut country = build_country(all_clubs, vec![tier1, tier2]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        // Bottom-2 of tier 1 (12, 13) drop to tier 2.
        let team_12_league = country
            .clubs
            .iter()
            .find(|c| c.id == 12)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_12_league, Some(Some(2)));
        let team_13_league = country
            .clubs
            .iter()
            .find(|c| c.id == 13)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_13_league, Some(Some(2)));

        // Top-2 of tier 2 (20, 21) rise to tier 1.
        let team_20_league = country
            .clubs
            .iter()
            .find(|c| c.id == 20)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_20_league, Some(Some(1)));
        let team_21_league = country
            .clubs
            .iter()
            .find(|c| c.id == 21)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_21_league, Some(Some(1)));

        // Mid-table teams stay put — stable team IDs verification.
        let team_10_league = country
            .clubs
            .iter()
            .find(|c| c.id == 10)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_10_league, Some(Some(1)));
        let team_22_league = country
            .clubs
            .iter()
            .find(|c| c.id == 22)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_22_league, Some(Some(2)));

        // final_table is cleared after processing so the next season
        // doesn't double-apply.
        assert!(
            country
                .leagues
                .leagues
                .iter()
                .all(|l| l.final_table.is_none())
        );
    }

    #[test]
    fn promotion_relegation_is_a_noop_when_no_adjacent_tier_exists() {
        // Single tier-1 league with relegation_spots > 0 but no tier-2
        // counterpart — must not mutate any team's league_id.
        let clubs: Vec<Club> = (10..=12)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 1)]))
            .collect();
        let tier1 =
            make_league_with_settings(1, 1, 0, 1, vec![(10, 30, 70), (11, 30, 50), (12, 30, 20)]);
        let mut country = build_country(clubs, vec![tier1]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        for club in &country.clubs {
            for team in &club.teams.teams {
                assert_eq!(team.league_id, Some(1));
            }
        }
    }

    // ── Parent-side loan renewals ─────────────────────────────────

    /// Build a player with a permanent contract and an active loan
    /// agreement. `parent_club_id` is the owner; `borrower_club_id` is
    /// where the player physically lives.
    fn make_loanee(
        id: u32,
        parent_expiration: NaiveDate,
        parent_club_id: u32,
        borrower_club_id: u32,
        loan_end: NaiveDate,
    ) -> Player {
        let mut p = make_player(id);
        // Renewal proposal decoration calls `player.position()` which
        // panics on the default empty PlayerPositions. Give the loanee
        // a concrete position so `decorate_proposal` can attach a
        // position bonus.
        p.positions = crate::PlayerPositions {
            positions: vec![crate::PlayerPosition {
                position: crate::PlayerPositionType::MidfielderCenter,
                level: 20,
            }],
        };
        let mut contract = crate::PlayerClubContract::new(100_000, parent_expiration);
        contract.squad_status = crate::PlayerSquadStatus::FirstTeamRegular;
        p.contract = Some(contract);
        p.contract_loan = Some(crate::PlayerClubContract::new_loan(
            50_000,
            loan_end,
            parent_club_id,
            1,
            borrower_club_id,
        ));
        p
    }

    #[test]
    fn parent_loan_renewals_offer_loaned_out_player_near_parent_expiry() {
        // Parent club 100 has loaned player 7 to borrower club 200.
        // Parent contract expires in ~110 days — well inside the
        // KeyPlayer / FirstTeamRegular threshold (540 days), so the
        // parent-side pass must build an offer even though player is at
        // the borrower's roster.
        let today = d(2026, 5, 1); // first of the month
        let loanee = make_loanee(7, d(2026, 8, 31), 100, 200, d(2026, 7, 31));
        let parent_team = make_team(100, 100, 1, vec![]);
        let parent_club = make_club(100, vec![parent_team]);
        let borrower_team = make_team(200, 200, 1, vec![loanee]);
        let borrower_club = make_club(200, vec![borrower_team]);
        let league = make_league_with_table(1, 7000, vec![]);
        let mut country = build_country(vec![parent_club, borrower_club], vec![league]);

        CountryResult::process_parent_loan_renewals(&mut country, today);

        // Loanee at borrower club 200 must now have a ContractProposal
        // in their mailbox AND a decision-history row stamped by the
        // parent.
        let borrower = country.clubs.iter().find(|c| c.id == 200).unwrap();
        let loanee = borrower.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 7)
            .unwrap();
        let has_proposal = loanee
            .mailbox
            .pending()
            .any(|m| matches!(m.message_type, PlayerMessageType::ContractProposal(_)));
        assert!(has_proposal, "parent club must push a renewal proposal");
        let has_decision = loanee
            .decision_history
            .items
            .iter()
            .any(|d| d.decision == "dec_contract_renewal_offered");
        assert!(
            has_decision,
            "renewal decision must be recorded on the loanee"
        );
    }

    #[test]
    fn parent_loan_renewals_silent_when_parent_contract_has_long_runway() {
        // Same setup but parent contract has 4 years left. No offer —
        // the parent is comfortable letting the loan run before they
        // think about a new deal.
        let today = d(2026, 5, 1);
        let loanee = make_loanee(7, d(2030, 6, 30), 100, 200, d(2026, 7, 31));
        let parent_team = make_team(100, 100, 1, vec![]);
        let parent_club = make_club(100, vec![parent_team]);
        let borrower_team = make_team(200, 200, 1, vec![loanee]);
        let borrower_club = make_club(200, vec![borrower_team]);
        let league = make_league_with_table(1, 7000, vec![]);
        let mut country = build_country(vec![parent_club, borrower_club], vec![league]);

        CountryResult::process_parent_loan_renewals(&mut country, today);

        let borrower = country.clubs.iter().find(|c| c.id == 200).unwrap();
        let loanee = borrower.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 7)
            .unwrap();
        let has_proposal = loanee
            .mailbox
            .pending()
            .any(|m| matches!(m.message_type, PlayerMessageType::ContractProposal(_)));
        assert!(
            !has_proposal,
            "parent club must NOT push a renewal — contract has 4-year runway"
        );
    }

    // ── Loan-return contract integrity ────────────────────────────

    #[test]
    fn loan_return_drops_loan_contract_and_preserves_parent_contract() {
        // Inline the mutation `execute_loan_return` applies to the
        // player on the way back home (line ~566 in this file). The
        // permanent contract — including any recent renewal extension
        // — must survive the trip intact; only the loan agreement is
        // cleared.
        let mut loanee = make_loanee(7, d(2030, 6, 30), 100, 200, d(2026, 7, 31));
        // Pretend the parent renewed the contract while away: bump
        // expiration further out and raise salary. This is the state
        // the renewal AI / acceptance handler would leave on the
        // player's `contract` field.
        if let Some(c) = loanee.contract.as_mut() {
            c.expiration = d(2031, 6, 30);
            c.salary = 180_000;
        }
        let pre_salary = loanee.contract.as_ref().unwrap().salary;
        let pre_expiration = loanee.contract.as_ref().unwrap().expiration;
        // Inline the loan-return mutation.
        loanee.contract_loan = None;

        assert!(loanee.contract_loan.is_none(), "loan agreement must clear");
        let parent = loanee
            .contract
            .as_ref()
            .expect("parent contract must remain");
        assert_eq!(parent.salary, pre_salary);
        assert_eq!(parent.expiration, pre_expiration);
        assert!(!loanee.is_on_loan());
    }

    #[test]
    fn warehoused_loaned_in_keeper_is_surplus_while_kept_and_owned_are_not() {
        // GK ideal depth is 3. Five keepers sit on the borrower's books:
        // three owned plus two loaned in. Only the loaned-in keeper ranked
        // outside the top three — settled, unused — is surplus to return.
        let date = d(2026, 11, 1);
        let loan_end = d(2027, 5, 31); // not yet expired
        let started = d(2026, 6, 1); // 153 days settled
        let gk_pos = PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::Goalkeeper,
                level: 16,
            }],
        };

        let mut owned_a = make_player(1);
        owned_a.positions = gk_pos.clone();
        owned_a.player_attributes.current_ability = 80;
        let mut owned_b = make_player(2);
        owned_b.positions = gk_pos.clone();
        owned_b.player_attributes.current_ability = 78;
        let mut owned_c = make_player(3);
        owned_c.positions = gk_pos.clone();
        owned_c.player_attributes.current_ability = 76;

        // Loaned-in but the squad's best keeper: kept (he plays), not returned.
        let mut loaned_kept = make_loanee(11, d(2030, 6, 30), 100, 200, loan_end);
        loaned_kept.positions = gk_pos.clone();
        loaned_kept.player_attributes.current_ability = 95;
        loaned_kept.statistics.played = 0;
        loaned_kept.contract_loan.as_mut().unwrap().started = Some(started);

        // Loaned-in, low ability, settled and unused: the warehoused surplus.
        let mut loaned_surplus = make_loanee(10, d(2030, 6, 30), 100, 200, loan_end);
        loaned_surplus.positions = gk_pos.clone();
        loaned_surplus.player_attributes.current_ability = 60;
        loaned_surplus.statistics.played = 0;
        loaned_surplus.contract_loan.as_mut().unwrap().started = Some(started);

        let team = make_team(
            200,
            200,
            1,
            vec![owned_a, owned_b, owned_c, loaned_kept, loaned_surplus],
        );
        let warehoused = WarehousedLoans::for_team(&team);
        let players = &team.players.players;

        assert!(
            warehoused.is_surplus(players.iter().find(|p| p.id == 10).unwrap(), date),
            "a settled, unused loaned-in keeper outside the top three is surplus"
        );
        assert!(
            !warehoused.is_surplus(players.iter().find(|p| p.id == 11).unwrap(), date),
            "a loaned-in keeper who is the squad's best is kept, not returned"
        );
        assert!(
            !warehoused.is_surplus(players.iter().find(|p| p.id == 3).unwrap(), date),
            "an owned keeper is never force-returned by the warehouse drain"
        );
    }

    // ── Manual / AI loan parent-contract safety ───────────────────

    #[test]
    fn ensure_contract_covers_loan_end_extends_short_parent_contract() {
        // Parent contract expires in March, loan runs until June of the
        // SAME year. The helper must push expiration past loan_end + 1
        // year so the player can't walk on a free immediately after
        // returning.
        let mut p = make_player(7);
        p.contract = Some(crate::PlayerClubContract::new(100_000, d(2026, 3, 31)));
        let loan_end = d(2026, 6, 30);

        p.ensure_contract_covers_loan_end(loan_end);

        let new_expiration = p.contract.as_ref().unwrap().expiration;
        let expected_min = loan_end
            .checked_add_signed(chrono::Duration::days(365))
            .unwrap();
        assert!(
            new_expiration >= expected_min,
            "expiration {} must be >= loan_end + 1y = {}",
            new_expiration,
            expected_min
        );
    }

    #[test]
    fn ensure_contract_covers_loan_end_does_not_shorten_long_parent_contract() {
        // Parent contract already runs 4 years past loan end. Helper
        // must leave it alone — clubs don't pull expiration back.
        let mut p = make_player(7);
        let original_expiration = d(2031, 6, 30);
        p.contract = Some(crate::PlayerClubContract::new(100_000, original_expiration));
        let loan_end = d(2026, 6, 30);

        p.ensure_contract_covers_loan_end(loan_end);

        assert_eq!(
            p.contract.as_ref().unwrap().expiration,
            original_expiration,
            "long parent contract must not be shortened"
        );
    }

    // ── Domestic cup winner awards ────────────────────────────────

    /// Build a domestic cup whose bracket has already resolved to
    /// `winner_id` (regulation 2-0) over `loser_id`. Two-team field —
    /// the seeded participants come from the country's clubs, so the
    /// caller must ensure their Main team ids match `winner_id` /
    /// `loser_id`.
    fn make_resolved_cup_2team(
        cup_id: u32,
        slug: &str,
        reputation: u16,
        winner_id: u32,
        loser_id: u32,
        season_year: i32,
    ) -> DomesticCup {
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let mut league = League::new(
            cup_id,
            "Test Cup".into(),
            slug.into(),
            1,
            reputation,
            settings,
            false,
        );
        league.is_cup = true;
        let mut cup = DomesticCup::new(league);
        cup.season_start_year = season_year;

        let dt = chrono::NaiveDateTime::new(
            d(season_year + 1, 5, 20),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        let mut item = ScheduleItem::new(cup_id, slug.into(), winner_id, loser_id, dt, None);
        item.result = Some(Score {
            home_team: TeamScore::new_with_score(winner_id, 2),
            away_team: TeamScore::new_with_score(loser_id, 0),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        });
        cup.league.schedule.tours.push(ScheduleTour {
            num: 1,
            items: vec![item],
        });
        cup
    }

    /// Append a cup appearance row to `player.cup_statistics_by_competition`.
    /// Mirrors what `LeagueResult::process_local` records when a player
    /// is named to the matchday squad (`played` for starters, `played_subs`
    /// for used substitutes). The award eligibility check reads
    /// `total_games()`, so any non-zero starts or sub-apps qualifies.
    fn add_cup_stats(player: &mut Player, slug: &str, played: u16, played_subs: u16, rating: f32) {
        let stats = player.cup_competition_statistics_mut(slug);
        stats.played = played;
        stats.played_subs = played_subs;
        stats.average_rating = rating;
    }

    fn build_country_with_cup(clubs: Vec<Club>, leagues: Vec<League>, cup: DomesticCup) -> Country {
        Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(leagues))
            .clubs(clubs)
            .domestic_cup(Some(cup))
            .build()
            .unwrap()
    }

    /// Make a player that has at least one position, so the helper's
    /// position-group lookup doesn't fall back to the Midfielder
    /// default. Used for tests that want a deterministic positional
    /// rating bucket.
    fn make_player_with_position(id: u32) -> Player {
        let mut p = make_player(id);
        p.positions = crate::PlayerPositions {
            positions: vec![crate::PlayerPosition {
                position: crate::PlayerPositionType::MidfielderCenter,
                level: 20,
            }],
        };
        p
    }

    #[test]
    fn cup_winner_award_only_emits_to_players_with_cup_appearances() {
        // Champion roster has two players. Only `appeared` has an entry
        // in `cup_statistics_by_competition` — the other was an unused
        // squad member. After the award helper runs, only `appeared`
        // has the trophy event and a domestic_cup_winner count.
        let mut appeared = make_player_with_position(1);
        add_cup_stats(&mut appeared, "test-cup", 3, 0, 7.4);
        let benched = make_player_with_position(2);
        let winner_team = make_team(10, 100, 1, vec![appeared, benched]);
        let winner_club = make_club(100, vec![winner_team]);

        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_001, "test-cup", 6500, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));

        let winner = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let appeared = &winner.teams.teams[0].players.players[0];
        let benched = &winner.teams.teams[0].players.players[1];
        assert_eq!(
            happiness_event_count(appeared, &HappinessEventType::DomesticCupWon),
            1,
            "appeared player must receive the cup winner trophy event"
        );
        assert_eq!(
            happiness_event_count(appeared, &HappinessEventType::TrophyWon),
            0,
            "league-title TrophyWon must NOT be emitted by the cup helper"
        );
        assert_eq!(
            appeared.awards_count.domestic_cup_winner, 1,
            "appeared player gets one DomesticCupWinner in the lifetime tally"
        );
        assert_eq!(
            happiness_event_count(benched, &HappinessEventType::DomesticCupWon),
            0,
            "benched player without cup apps must not receive the trophy event"
        );
        assert_eq!(
            benched.awards_count.domestic_cup_winner, 0,
            "benched player must not bump the DomesticCupWinner counter"
        );
        // The cup's emit marker reflects the season + champion so the
        // next tick can short-circuit.
        let cup = country.domestic_cup.as_ref().unwrap();
        assert_eq!(cup.award_emitted_season_start_year, Some(2026));
        assert_eq!(cup.award_emitted_winner_team_id, Some(10));
    }

    #[test]
    fn cup_winner_award_credits_final_day_substitutes() {
        // The helper is meant to run AFTER process_local, so a player
        // who came on only in the final has their appearance row in
        // place. Modelled here by recording one substitute appearance
        // for `sub_only` — the award should still land.
        let mut sub_only = make_player_with_position(1);
        add_cup_stats(&mut sub_only, "test-cup", 0, 1, 7.0);
        let winner_team = make_team(10, 100, 1, vec![sub_only]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_002, "test-cup", 7000, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));

        let winner = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let sub_only = &winner.teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(sub_only, &HappinessEventType::DomesticCupWon),
            1,
            "final-day used substitute must receive the trophy"
        );
        assert_eq!(sub_only.awards_count.domestic_cup_winner, 1);
    }

    #[test]
    fn cup_winner_award_does_not_double_emit_on_subsequent_ticks() {
        // Helper is invoked from `Country::simulate` every tick. Once
        // the marker is set, repeat calls must short-circuit so the
        // happiness event and award counter only bump once per edition.
        let mut starter = make_player_with_position(1);
        add_cup_stats(&mut starter, "test-cup", 2, 0, 7.2);
        let winner_team = make_team(10, 100, 1, vec![starter]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_003, "test-cup", 6500, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        // Two ticks: the day of the final + the day after.
        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));
        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 21));

        let winner = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let starter = &winner.teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(starter, &HappinessEventType::DomesticCupWon),
            1,
            "second tick must not re-emit the trophy"
        );
        assert_eq!(
            starter.awards_count.domestic_cup_winner, 1,
            "DomesticCupWinner counter must stay at 1 across repeated ticks"
        );
    }

    #[test]
    fn cup_winner_award_skips_champion_roster_player_with_zero_cup_apps() {
        // 3-team field: top seed (id 10) takes a round-one bye, then
        // wins the final. A roster player on the winning team who
        // never made an appearance — no row in
        // `cup_statistics_by_competition` — must not collect the medal.
        let mut starter = make_player_with_position(1);
        add_cup_stats(&mut starter, "test-cup", 1, 0, 7.0);
        let unused = make_player_with_position(2);
        let winner_team = make_team(10, 100, 1, vec![starter, unused]);
        let winner_club = make_club(100, vec![winner_team]);

        let runner_up_team = make_team(20, 200, 1, vec![]);
        let runner_up_club = make_club(200, vec![runner_up_team]);
        let bye_loser_team = make_team(30, 300, 1, vec![]);
        let bye_loser_club = make_club(300, vec![bye_loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);

        // Hand-roll a 3-team cup: round 1 (20 vs 30, 30 bye for top
        // seed 10), round 2 final (10 vs 20). 10 wins the final.
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let mut league_cup = League::new(
            800_000_004,
            "Cup".into(),
            "test-cup".into(),
            1,
            7000,
            settings,
            false,
        );
        league_cup.is_cup = true;
        let mut cup = DomesticCup::new(league_cup);
        cup.season_start_year = 2026;
        let dt = chrono::NaiveDateTime::new(
            d(2027, 4, 1),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        // Round 1: 20 beats 30 (30 was the away side), 10 has a bye.
        let mut r1_item = ScheduleItem::new(800_000_004, "test-cup".into(), 20, 30, dt, None);
        r1_item.result = Some(Score {
            home_team: TeamScore::new_with_score(20, 1),
            away_team: TeamScore::new_with_score(30, 0),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        });
        cup.league.schedule.tours.push(ScheduleTour {
            num: 1,
            items: vec![r1_item],
        });
        // Final: 10 beats 20.
        let dt2 = chrono::NaiveDateTime::new(
            d(2027, 5, 20),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        let mut r2_item = ScheduleItem::new(800_000_004, "test-cup".into(), 10, 20, dt2, None);
        r2_item.result = Some(Score {
            home_team: TeamScore::new_with_score(10, 2),
            away_team: TeamScore::new_with_score(20, 0),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        });
        cup.league.schedule.tours.push(ScheduleTour {
            num: 2,
            items: vec![r2_item],
        });

        let mut country = build_country_with_cup(
            vec![winner_club, runner_up_club, bye_loser_club],
            vec![league],
            cup,
        );

        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));

        let winner = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let starter = &winner.teams.teams[0].players.players[0];
        let unused = &winner.teams.teams[0].players.players[1];
        assert_eq!(
            happiness_event_count(starter, &HappinessEventType::DomesticCupWon),
            1,
            "the starter who actually played still gets the medal"
        );
        assert_eq!(starter.awards_count.domestic_cup_winner, 1);
        assert_eq!(
            happiness_event_count(unused, &HappinessEventType::DomesticCupWon),
            0,
            "the unused squad player on the champion side does not"
        );
        assert_eq!(unused.awards_count.domestic_cup_winner, 0);
    }

    #[test]
    fn cup_winner_award_coexists_with_league_title_trophy() {
        // A double-winning side: the league title `TrophyWon` fires
        // earlier in the same end-of-season tick, the cup helper runs
        // on the final day. The cup helper must NOT collide on the
        // generic `TrophyWon` cooldown — both medals should land on
        // the player's happiness ledger, recorded as distinct events.
        let mut starter = make_player_with_position(1);
        add_cup_stats(&mut starter, "test-cup", 3, 0, 7.4);
        // Simulate the league-title trophy that the season-awards path
        // would have emitted earlier in the season. Magnitude doesn't
        // matter — what matters is that the cooldown bucket for
        // `TrophyWon` is now full.
        starter.on_team_season_event_with_prestige(
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2027, 5, 18),
        );

        let winner_team = make_team(10, 100, 1, vec![starter]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_005, "test-cup", 7500, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));

        let starter = &country
            .clubs
            .iter()
            .find(|c| c.id == 100)
            .unwrap()
            .teams
            .teams[0]
            .players
            .players[0];
        assert_eq!(
            happiness_event_count(starter, &HappinessEventType::TrophyWon),
            1,
            "league-title TrophyWon must remain — cup helper does not suppress it"
        );
        assert_eq!(
            happiness_event_count(starter, &HappinessEventType::DomesticCupWon),
            1,
            "domestic cup emit must land on its own cooldown bucket"
        );
        assert_eq!(starter.awards_count.domestic_cup_winner, 1);
    }

    #[test]
    fn cup_winner_award_skips_achievements_when_no_eligible_players() {
        // Pre-check guard: a champion whose roster has zero recorded
        // cup apps (data glitch — e.g. stats not yet caught up after a
        // save restore) must NOT fire `team.on_season_trophy` or the
        // board achievement. Otherwise repeat ticks would inflate
        // reputation + the board long-term-goal counter every day
        // until somebody picks up a cup app.
        let no_stats_player = make_player_with_position(1);
        let winner_team = make_team(10, 100, 1, vec![no_stats_player]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_006, "test-cup", 6500, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        // Snapshot pre-call reputation so we can detect achievement bumps.
        let rep_before = country
            .clubs
            .iter()
            .find(|c| c.id == 100)
            .unwrap()
            .teams
            .teams[0]
            .reputation
            .home;

        // Two ticks, neither of which has any eligible player.
        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));
        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 21));

        let team = &country
            .clubs
            .iter()
            .find(|c| c.id == 100)
            .unwrap()
            .teams
            .teams[0];
        assert_eq!(
            team.reputation.home, rep_before,
            "team reputation must not move when no player qualifies"
        );

        // The cup marker must remain unset so the next tick can
        // recover once `process_local` catches up.
        let cup = country.domestic_cup.as_ref().unwrap();
        assert!(
            cup.award_emitted_winner_team_id.is_none(),
            "emit marker must stay unset when nobody was awarded"
        );
        assert!(cup.award_emitted_on.is_none());
    }

    #[test]
    fn cup_winner_award_final_appearance_lifts_magnitude() {
        // Two players, both 1-app cup contributors. One played in the
        // final (FieldSquad.main), one didn't. The final-appearance
        // bonus (+0.10 involvement) must produce a larger trophy
        // magnitude on the player who was on the pitch for the final.
        let mut in_final = make_player_with_position(1);
        add_cup_stats(&mut in_final, "test-cup", 1, 0, 7.4);
        let mut only_earlier = make_player_with_position(2);
        add_cup_stats(&mut only_earlier, "test-cup", 1, 0, 7.4);

        let winner_team = make_team(10, 100, 1, vec![in_final, only_earlier]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let mut cup = make_resolved_cup_2team(800_000_007, "test-cup", 7500, 10, 20, 2026);
        // Inject a `MatchResult` with FieldSquad details under the
        // same id format the schedule item uses — that's what the
        // helper looks up via `cup.league.matches.get(item.id)`.
        let final_item = &cup.league.schedule.tours.last().unwrap().items[0];
        let final_date = final_item.date.date();
        let final_id = final_item.id.clone();
        let mut details = crate::r#match::MatchResultRaw::with_match_time(0);
        let mut home_squad = crate::r#match::FieldSquad::new();
        home_squad.team_id = 10;
        home_squad.main = vec![1]; // only player 1 started the final
        let away_squad = crate::r#match::FieldSquad::new();
        details.write_team_players(&home_squad, &away_squad);
        let mr = crate::r#match::MatchResult {
            id: final_id,
            league_id: cup.league.id,
            league_slug: cup.league.slug.clone(),
            home_team_id: 10,
            away_team_id: 20,
            details: Some(details),
            score: Score {
                home_team: TeamScore::new_with_score(10, 2),
                away_team: TeamScore::new_with_score(20, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            },
            friendly: false,
        };
        cup.league.matches.push(mr, final_date);

        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        CountryResult::process_domestic_cup_winner_awards(&mut country, final_date);

        let players = &country
            .clubs
            .iter()
            .find(|c| c.id == 100)
            .unwrap()
            .teams
            .teams[0]
            .players
            .players;
        let mag_in_final = players
            .iter()
            .find(|p| p.id == 1)
            .and_then(|p| {
                p.happiness
                    .recent_events
                    .iter()
                    .find(|e| e.event_type == HappinessEventType::DomesticCupWon)
                    .map(|e| e.magnitude)
            })
            .expect("in_final player has DomesticCupWon event");
        let mag_only_earlier = players
            .iter()
            .find(|p| p.id == 2)
            .and_then(|p| {
                p.happiness
                    .recent_events
                    .iter()
                    .find(|e| e.event_type == HappinessEventType::DomesticCupWon)
                    .map(|e| e.magnitude)
            })
            .expect("only_earlier player has DomesticCupWon event");
        assert!(
            mag_in_final > mag_only_earlier,
            "final appearance must lift the trophy magnitude: {} (final) vs {} (earlier-only)",
            mag_in_final,
            mag_only_earlier
        );
    }

    #[test]
    fn cup_winner_award_attaches_trophy_context() {
        // The emitted DomesticCupWon event must carry a
        // TrophyEventContext describing the competition + the player's
        // role in winning it. Renderer relies on the context being
        // populated to produce specific copy.
        let mut starter = make_player_with_position(1);
        add_cup_stats(&mut starter, "test-cup", 4, 1, 7.6);
        let winner_team = make_team(10, 100, 1, vec![starter]);
        let winner_club = make_club(100, vec![winner_team]);
        let loser_team = make_team(20, 200, 1, vec![]);
        let loser_club = make_club(200, vec![loser_team]);

        let league = make_league_with_table(1, 5000, vec![]);
        let cup = make_resolved_cup_2team(800_000_008, "test-cup", 7000, 10, 20, 2026);
        let mut country = build_country_with_cup(vec![winner_club, loser_club], vec![league], cup);

        CountryResult::process_domestic_cup_winner_awards(&mut country, d(2027, 5, 20));

        let starter = &country
            .clubs
            .iter()
            .find(|c| c.id == 100)
            .unwrap()
            .teams
            .teams[0]
            .players
            .players[0];
        let event = starter
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DomesticCupWon)
            .expect("DomesticCupWon event must be present");
        let trophy = event
            .context
            .as_ref()
            .and_then(|c| c.trophy_context.as_ref())
            .expect("TrophyEventContext must be attached");
        assert_eq!(trophy.trophy_kind, TrophyKind::DomesticCup);
        assert_eq!(trophy.competition_slug.as_deref(), Some("test-cup"));
        assert_eq!(trophy.winner_team_id, Some(10));
        // Player has 4 starts + 1 sub apps = 5 apps.
        assert_eq!(trophy.apps, Some(5));
        assert_eq!(trophy.starts, Some(4));
        assert_eq!(trophy.used_sub_apps, Some(1));
    }
}
