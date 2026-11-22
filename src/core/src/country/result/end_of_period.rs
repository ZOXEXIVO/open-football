use super::CountryResult;
use super::transfers::settlement::TransferClauseSettler;
use crate::PlayerContractProposal;
use crate::club::CareerRunway;
use crate::club::SquadDepartures;
use crate::club::board::mandate::SigningMandate;
use crate::club::board::{BoardTransferConcern, BoardTransferDecision};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::events::TransferCompletion;
use crate::club::player::mind::SpellChange;
use crate::club::process::{DepartedPlayer, DepartureKind};
use crate::club::staff::SeparationCause;
use crate::club::staff::SpellCloser;
use crate::club::staff::perception::AbilityEstimator;
use crate::club::team::reputation::{Achievement, AchievementType};
use crate::club::team::squad::{ContractRenewalManager, WageStructureSnapshot};
use crate::country::process::PromotionRelegationSwap;
use crate::league::LeagueLadder;
use crate::transfers::MarketMap;
use crate::transfers::deal::offer::PromisedSquadStatus;
use crate::transfers::gate::build::Suitor;
use crate::transfers::gate::{
    AppraisalConfig, MoveTerms, OfferKind, OfferViewBuilder, PlayerDisposition,
    PlayerOfferAppraisal, PlayerStanceBuilder, StanceInputs, TermsRefusalCause,
    TransferMovePlausibility,
};
use crate::transfers::loan::LoanPipeline;
use crate::transfers::squad::LevelBand;
use crate::transfers::value::wage::BuyerLevelWage;
use crate::transfers::view::player::{PlayerSummaryBuilder, SummaryClubContext};
use crate::utils::{DateUtils, FormattingUtils};
use crate::world::SimulatorData;
use crate::world::bootstrap::ClubIdentity;
use crate::{
    Club, ClubResult, Country, HappinessEventCause, HappinessEventContext, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, LoanEventContext, LoanEventKind, LoanSpellRecord,
    Person, Player, PlayerClubContract, PlayerFieldPositionGroup, PlayerHappiness, PlayerMessage,
    PlayerMessageType, PlayerSquadStatus, RetirementReason, SeasonOutcomeContext,
    SeasonOutcomeKind, StaffPosition, Team, TeamInfo, TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;

struct LoanReturnEvent {
    player_id: u32,
    borrowing_club_id: u32,
    parent_club_id: u32,
    borrowing_info: TeamInfo,
    /// The borrower buys him as the loan expires, through its negotiated
    /// option or obligation — the player stays put and ownership transfers
    /// instead of returning.
    buyout: Option<LoanBuyout>,
    /// The borrower's board would not buy him: the concern it named, for
    /// his record. He goes home like any other returning loanee.
    turned_down: Option<BoardTransferConcern>,
    /// The borrower wanted him and he would not sign: why, for his
    /// record. He goes home like any other returning loanee.
    declined_buyout: Option<TermsRefusalCause>,
}

/// A purchase at the end of a loan.
#[derive(Debug, Clone, Copy)]
struct LoanBuyout {
    fee: u32,
    /// Agreed with the loan, so nobody is asked again at its end.
    obligation: bool,
    /// What the borrower is buying him for: the purpose its board heard
    /// for an option, and the same reading of the role he held there for
    /// an obligation. The plan he signs onto carries it.
    mandate: SigningMandate,
}

/// The borrower's answer to a stored option or obligation to buy.
#[derive(Debug, Clone, Copy)]
enum LoanBuyoutDecision {
    Buy(LoanBuyout),
    /// The board heard the option and would not buy him.
    TurnedDown(BoardTransferConcern),
}

/// Per-team view of which loaned-in players a borrower is warehousing.
/// Keeps the top `ideal_squad_depth` of each position group by current
/// ability; a loaned-in player outside that set — long-settled, barely
/// featuring, and with no permanent option pending — is surplus the
/// borrower over-loaned and should be returned to its parent early.
struct WarehousedLoans {
    kept_ids: FxHashSet<u32>,
}

impl WarehousedLoans {
    /// A loaned-in player needs at least this many days settled before the
    /// borrower's appearance record is judged enough to call him surplus.
    const SETTLE_DAYS: i64 = 90;
    /// At/under this appearance count over the settling window the loanee
    /// is plainly not being used.
    const UNUSED_APPS: u16 = 3;

    fn for_team(team: &Team) -> Self {
        let mut kept_ids: FxHashSet<u32> = FxHashSet::default();
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
            ranked.sort_by_key(|r| Reverse(r.1));
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
    pub(crate) fn process_season_awards(
        country: &mut Country,
        _club_results: &[ClubResult],
        date: NaiveDate,
    ) {
        debug!("Processing season awards");

        let team_to_club = Self::team_to_club(country);
        let ladder = LeagueLadder::new(&country.leagues.leagues);
        let (trophies, events) = Self::collect_trophy_events(country, &ladder);
        Self::apply_trophy_awards(country, trophies, date);

        let club_awards = Self::collect_club_awards(country, &team_to_club);
        Self::apply_club_awards(country, club_awards);

        for (team_id, event, prestige) in events {
            Self::apply_team_squad_event(country, team_id, event, 365, prestige, date);
        }
        Self::reset_disciplinary_state(country);
    }

    fn team_to_club(country: &Country) -> FxHashMap<u32, usize> {
        country
            .clubs
            .iter()
            .enumerate()
            .flat_map(|(club_idx, club)| club.teams.iter().map(move |team| (team.id, club_idx)))
            .collect()
    }

    fn collect_trophy_events(
        country: &Country,
        ladder: &LeagueLadder,
    ) -> (
        Vec<(u32, AchievementType)>,
        Vec<(u32, HappinessEventType, f32)>,
    ) {
        let per_league: Vec<_> = country
            .leagues
            .leagues
            .par_iter()
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                let table = match &league.final_table {
                    Some(table) if !table.is_empty() => table,
                    _ => return None,
                };

                let mut trophies: Vec<(u32, AchievementType)> = Vec::new();
                let mut events: Vec<(u32, HappinessEventType, f32)> = Vec::new();
                let sends_sides_up = ladder.promoted_from_table(league) > 0;
                let playoff_crowned = league
                    .settings
                    .league_group
                    .as_ref()
                    .is_some_and(|group| group.playoff.is_some());
                if let Some(champion) = table.first().filter(|_| !playoff_crowned) {
                    trophies.push((champion.team_id, AchievementType::LeagueTitle));
                    let prestige = if sends_sides_up { 0.6 } else { 1.0 };
                    events.push((champion.team_id, HappinessEventType::TrophyWon, prestige));
                }

                if league.settings.tier == 1 {
                    let europe_spots = if league.reputation >= 7000 {
                        7
                    } else if league.reputation >= 5000 {
                        4
                    } else if league.reputation >= 3000 {
                        2
                    } else {
                        0
                    };
                    for row in table.iter().take(europe_spots).skip(1) {
                        events.push((row.team_id, HappinessEventType::QualifiedForEurope, 1.0));
                    }
                }

                Some((trophies, events))
            })
            .collect();

        let mut trophy_awards = Vec::new();
        let mut player_team_events = Vec::new();
        for (mut trophies, mut events) in per_league {
            trophy_awards.append(&mut trophies);
            player_team_events.append(&mut events);
        }
        (trophy_awards, player_team_events)
    }

    fn apply_trophy_awards(
        country: &mut Country,
        trophies: Vec<(u32, AchievementType)>,
        date: NaiveDate,
    ) {
        for (team_id, achievement) in trophies {
            if let Some(club) = country
                .clubs
                .iter_mut()
                .find(|club| club.teams.iter().any(|team| team.id == team_id))
            {
                Self::award_trophy(club, team_id, achievement, date);
            }
        }
    }

    fn collect_club_awards(
        country: &Country,
        team_to_club: &FxHashMap<u32, usize>,
    ) -> FxHashMap<usize, (i64, i64)> {
        let mut club_awards: FxHashMap<usize, (i64, i64)> = FxHashMap::default();
        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }
            let table = match &league.final_table {
                Some(table) if !table.is_empty() => table,
                _ => continue,
            };
            let total_teams = table.len();
            let prize_pool = league.financials.prize_pool;
            let tv_deal = league.financials.tv_deal_total;
            if prize_pool == 0 && tv_deal == 0 {
                continue;
            }

            let total_shares: f64 = (0..total_teams)
                .map(|position| {
                    let ratio = position as f64 / (total_teams - 1).max(1) as f64;
                    (1.0 - ratio).powi(2)
                })
                .sum();

            for (position, row) in table.iter().enumerate() {
                let Some(&club_idx) = team_to_club.get(&row.team_id) else {
                    continue;
                };
                let share = if total_teams > 1 {
                    let ratio = position as f64 / (total_teams - 1) as f64;
                    (1.0 - ratio).powi(2)
                } else {
                    1.0
                };
                let prize_amount = (prize_pool as f64 * share / total_shares) as i64;
                let tv_equal = tv_deal / (2 * total_teams as i64);
                let tv_merit = (tv_deal as f64 * 0.5 * share / total_shares) as i64;
                let entry = club_awards.entry(club_idx).or_insert((0, 0));
                entry.0 += prize_amount;
                entry.1 += tv_equal + tv_merit;
            }
        }
        club_awards
    }

    fn apply_club_awards(country: &mut Country, club_awards: FxHashMap<usize, (i64, i64)>) {
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
    }

    fn reset_disciplinary_state(country: &mut Country) {
        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    player.reset_season_disciplinary_state();
                }
            }
        }
    }

    /// Bank a season trophy won by `team_id` on its club.
    fn award_trophy(club: &mut Club, team_id: u32, ach_type: AchievementType, date: NaiveDate) {
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
        let (salary_bump_pct, extension_years, loyalty_lift): (f32, i32, u8) = match ach_type {
            AchievementType::ContinentalTrophy => (0.25, 3, 20),
            AchievementType::LeagueTitle => (0.20, 2, 15),
            AchievementType::CupWin => (0.08, 1, 6),
            AchievementType::Promotion => (0.10, 1, 8),
            _ => (0.0, 0, 0),
        };
        if salary_bump_pct > 0.0 {
            let cur = club.board.chairman.manager_loyalty as u16;
            club.board.chairman.manager_loyalty = (cur + loyalty_lift as u16).min(100) as u8;
            if let Some(main_team) = club.teams.main_mut()
                && let Some(mgr) = main_team
                    .staffs
                    .find_mut_by_position(StaffPosition::Manager)
                && let Some(contract) = mgr.contract.as_mut()
            {
                contract.salary = ((contract.salary as f32) * (1.0 + salary_bump_pct)) as u32;
                if extension_years > 0 {
                    let new_exp = contract
                        .expired
                        .with_year(contract.expired.year() + extension_years)
                        .unwrap_or(contract.expired);
                    if new_exp > contract.expired {
                        contract.expired = new_exp;
                    }
                }
                mgr.job_satisfaction = (mgr.job_satisfaction + 12.0).clamp(0.0, 100.0);
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
            let club_id = club.id;
            for team in club.teams.iter_mut() {
                if team.id != team_id {
                    continue;
                }
                for player in team.players.iter_mut() {
                    let fired = player.on_team_season_event_with_prestige(
                        event.clone(),
                        cooldown_days,
                        prestige,
                        date,
                    );
                    // Only what actually reached him as a mood reaches
                    // him as a memory — a title he was cooled down out of
                    // is a title the season already counted.
                    if fired {
                        player.remember_season_outcome(&event, club_id, date);
                    }
                    if is_european_qualification {
                        player.on_continental_qualification_satisfaction();
                    }
                }
            }
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
                Some(buyout) => Self::execute_loan_buyout(data, event, buyout, date),
                None => Self::execute_loan_return(data, event, date, true),
            }
        }
    }

    fn scan_expired_loans(
        data: &SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) -> Vec<LoanReturnEvent> {
        let country = match data.country(country_id) {
            Some(country) => country,
            None => return Vec::new(),
        };

        let mut events = Vec::new();
        for club in &country.clubs {
            if !Self::has_loan_activity(club) {
                continue;
            }
            events.extend(Self::expired_loan_events(data, country, club, date));
        }
        events
    }

    fn has_loan_activity(club: &Club) -> bool {
        club.teams.iter().any(|team| {
            team.players.iter().any(|player| {
                player
                    .contract_loan
                    .as_ref()
                    .is_some_and(|contract| contract.loan_from_club_id.is_some())
            })
        })
    }

    fn expired_loan_events(
        data: &SimulatorData,
        country: &Country,
        club: &Club,
        date: NaiveDate,
    ) -> Vec<LoanReturnEvent> {
        let (main_name, main_slug, main_reputation, main_league_name, main_league_slug) =
            Self::borrowing_identity(country, club);
        let mut events = Vec::new();

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
            let borrowing_info = TeamInfo {
                name: team_name,
                slug: team_slug,
                reputation: team_reputation,
                league_name: main_league_name.clone(),
                league_slug: main_league_slug.clone(),
            };
            let warehoused = WarehousedLoans::for_team(team);
            for player in team.players.iter() {
                if let Some(event) = Self::loan_return_event(
                    data,
                    country,
                    club,
                    player,
                    &warehoused,
                    date,
                    &borrowing_info,
                ) {
                    events.push(event);
                }
            }
        }
        events
    }

    fn borrowing_identity(
        country: &Country,
        club: &Club,
    ) -> (Option<String>, Option<String>, Option<u16>, String, String) {
        let main_team = club.teams.main();
        let main_name = main_team.map(|team| team.name.clone());
        let main_slug = main_team.map(|team| team.slug.clone());
        let main_reputation = main_team.map(|team| team.reputation.world);
        let (main_league_name, main_league_slug) = main_team
            .and_then(|team| team.league_id)
            .and_then(|league_id| {
                country
                    .leagues
                    .leagues
                    .iter()
                    .find(|league| league.id == league_id)
                    .map(|league| (league.name.clone(), league.slug.clone()))
            })
            .unwrap_or_default();
        (
            main_name,
            main_slug,
            main_reputation,
            main_league_name,
            main_league_slug,
        )
    }

    fn loan_return_event(
        data: &SimulatorData,
        country: &Country,
        club: &Club,
        player: &Player,
        warehoused: &WarehousedLoans,
        date: NaiveDate,
        borrowing_info: &TeamInfo,
    ) -> Option<LoanReturnEvent> {
        let loan_contract = player.contract_loan.as_ref()?;
        let parent_club_id = loan_contract.loan_from_club_id?;
        let expiring = loan_contract.expiration <= date;
        if !(expiring || warehoused.is_surplus(player, date)) {
            return None;
        }

        let mut event = LoanReturnEvent {
            player_id: player.id,
            borrowing_club_id: club.id,
            parent_club_id,
            borrowing_info: borrowing_info.clone(),
            buyout: None,
            turned_down: None,
            declined_buyout: None,
        };
        let decision = if expiring {
            Self::decide_loan_buyout(club, player, loan_contract, date)
        } else {
            None
        };
        match decision {
            Some(LoanBuyoutDecision::TurnedDown(concern)) => {
                event.turned_down = Some(concern);
            }
            Some(LoanBuyoutDecision::Buy(buyout)) if buyout.obligation => {
                event.buyout = Some(buyout);
            }
            Some(LoanBuyoutDecision::Buy(buyout)) => match Self::buyout_refusal(
                data,
                country,
                club,
                player,
                parent_club_id,
                buyout.fee,
                date,
            ) {
                Some(cause) => event.declined_buyout = Some(cause),
                None => event.buyout = Some(buyout),
            },
            None => {}
        }
        Some(event)
    }

    /// The borrower's answer to a stored option / obligation to buy as the
    /// loan expires. An obligation is binding. An option is a purchase, and
    /// the borrower's board hears it as one — the season he played there is
    /// its evidence, not its gate ([`Club::hear_option_to_buy`]). Either way
    /// the purpose is read from the role he held there, so the plan he signs
    /// onto is what the club believed it bought.
    fn decide_loan_buyout(
        club: &Club,
        player: &Player,
        loan: &PlayerClubContract,
        date: NaiveDate,
    ) -> Option<LoanBuyoutDecision> {
        let fee = loan.loan_future_fee?;
        if loan.loan_future_fee_obligation {
            let club_reputation = club.teams.main().map(|t| t.reputation.world).unwrap_or(0);
            return Some(LoanBuyoutDecision::Buy(LoanBuyout {
                fee,
                obligation: true,
                mandate: SigningMandate::unnegotiated(player, club_reputation, date),
            }));
        }
        let (proposal, decision) = club.hear_option_to_buy(player, fee as f64, date);
        Some(match decision {
            BoardTransferDecision::Vetoed(concern) => LoanBuyoutDecision::TurnedDown(concern),
            _ => LoanBuyoutDecision::Buy(LoanBuyout {
                fee,
                obligation: false,
                mandate: proposal.mandate,
            }),
        })
    }

    /// The player's side of an option to buy. The borrower's board has
    /// approved the purchase; whether he turns a season there into a
    /// permanent deal is the appraisal every personal-terms round runs,
    /// weighed against the club that owns him. `None` when he signs.
    #[allow(clippy::too_many_arguments)]
    fn buyout_refusal(
        data: &SimulatorData,
        country: &Country,
        borrower: &Club,
        player: &Player,
        parent_club_id: u32,
        fee: u32,
        date: NaiveDate,
    ) -> Option<TermsRefusalCause> {
        let (pci, pcoi, pcli, _) = data.find_club_main_team(parent_club_id)?;
        let parent_country = &data.continents[pci].countries[pcoi];
        let parent = &parent_country.clubs[pcli];
        let borrower_rep = borrower.teams.main()?.reputation.overall_score();
        let parent_rep = parent.teams.main()?.reputation.overall_score();
        let borrower_league_rep = LoanPipeline::club_league_reputation(country, borrower);
        let promised = player
            .contract_loan
            .as_ref()
            .and_then(|loan| PromisedSquadStatus::of_status(&loan.squad_status));
        // What he is where he has been playing all season, and how far
        // the club that owns him sits above it. Neither read prices a
        // market, so neither needs the world's geography.
        let suitor = Suitor::of(country, borrower, date);
        let terms = MoveTerms::for_player(player, borrower, date, false, false, fee as f64);
        let here = TransferMovePlausibility::read(
            &suitor,
            &PlayerSummaryBuilder::one(country, borrower, player, date)?,
            terms,
            &MarketMap::default(),
        )
        .diagnostics;
        let registered = borrower
            .teams
            .teams
            .iter()
            .find(|t| t.players.find(player.id).is_some())?;
        let from_parent = TransferMovePlausibility::read(
            &suitor,
            &PlayerSummaryBuilder::build(
                parent_country,
                parent,
                &SummaryClubContext::of(parent_country, parent, date, false),
                registered,
                player,
                date,
            ),
            terms,
            &MarketMap::default(),
        )
        .diagnostics;
        let stance = PlayerStanceBuilder::build(&StanceInputs {
            player,
            seller_country: parent_country,
            seller_club: parent,
            buyer_club_id: borrower.id,
            rep_diff: borrower_rep - parent_rep,
            importance: here.importance,
            listed_by_club: false,
            available: false,
            months_to_tournament: country.months_to_tournament_for(player.nationality_continent_id),
            date,
        });
        let wage = BuyerLevelWage::evaluate(
            player,
            player.age(date),
            borrower_rep,
            borrower_league_rep,
            promised,
        );
        let offer = OfferViewBuilder::build(
            OfferKind::Permanent,
            country,
            borrower.id,
            &stance,
            wage as f64,
            promised,
            from_parent.sporting_drop,
            borrower_rep,
            &MarketMap::default(),
        );
        let cfg = AppraisalConfig::default();
        let disposition = PlayerDisposition::for_negotiation(
            country.id,
            0,
            player.id,
            borrower.id,
            cfg.disposition_sigma,
        );
        let appraisal = PlayerOfferAppraisal::appraise(&stance, &offer, disposition, &cfg);
        debug!(
            "Loan option: player {} weighs a permanent deal at club {}: {}",
            player.id,
            borrower.id,
            appraisal.explain()
        );
        (!appraisal.accepts()).then(|| appraisal.refusal_cause(false))
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
        buyout: LoanBuyout,
        date: NaiveDate,
    ) {
        let LoanBuyout {
            fee,
            obligation,
            mandate,
        } = buyout;
        let Some(borrowing) = data.find_club_main_team(event.borrowing_club_id) else {
            Self::execute_loan_return(data, event, date, true);
            return;
        };
        let Some(parent) = data.find_club_main_team(event.parent_club_id) else {
            Self::execute_loan_return(data, event, date, true);
            return;
        };

        let affordable = data.continents[borrowing.0].countries[borrowing.1].clubs[borrowing.2]
            .finance
            .can_afford_transfer(fee as f64);
        if !obligation && !affordable {
            debug!(
                "Loan option lapsed: club {} cannot afford {} for player {}",
                event.borrowing_club_id, fee, event.player_id
            );
            let turned_down = LoanReturnEvent {
                turned_down: Some(BoardTransferConcern::ExceedsTransferBudget),
                ..event
            };
            Self::execute_loan_return(data, turned_down, date, true);
            return;
        }

        let (parent_info, selling_reputation, buying_reputation) =
            Self::buyout_team_info(data, borrowing, parent);
        Self::book_loan_buyout(data, borrowing, parent, &event, fee, date);
        Self::complete_loan_buyout(
            data,
            event,
            parent,
            fee,
            &parent_info,
            buying_reputation,
            selling_reputation,
            mandate,
            date,
        );
    }

    fn buyout_team_info(
        data: &SimulatorData,
        borrowing: (usize, usize, usize, usize),
        parent: (usize, usize, usize, usize),
    ) -> (TeamInfo, u16, u16) {
        let (bci, bcoi, bcli, bti) = borrowing;
        let (pci, pcoi, pcli, pti) = parent;
        let parent_country = &data.continents[pci].countries[pcoi];
        let parent_team = &parent_country.clubs[pcli].teams.teams[pti];
        let parent_league = parent_team.league_id.and_then(|league_id| {
            parent_country
                .leagues
                .leagues
                .iter()
                .find(|league| league.id == league_id)
        });
        let parent_info = TeamInfo {
            name: parent_team.name.clone(),
            slug: parent_team.slug.clone(),
            reputation: parent_team.reputation.world,
            league_name: parent_league
                .map(|league| league.name.clone())
                .unwrap_or_default(),
            league_slug: parent_league
                .map(|league| league.slug.clone())
                .unwrap_or_default(),
        };

        let borrowing_country = &data.continents[bci].countries[bcoi];
        let buying_reputation = borrowing_country.clubs[bcli].teams.teams[bti]
            .league_id
            .and_then(|league_id| {
                borrowing_country
                    .leagues
                    .leagues
                    .iter()
                    .find(|league| league.id == league_id)
            })
            .map(|league| league.reputation)
            .unwrap_or(0);
        (
            parent_info,
            parent_league.map(|league| league.reputation).unwrap_or(0),
            buying_reputation,
        )
    }

    fn book_loan_buyout(
        data: &mut SimulatorData,
        borrowing: (usize, usize, usize, usize),
        parent: (usize, usize, usize, usize),
        event: &LoanReturnEvent,
        fee: u32,
        date: NaiveDate,
    ) {
        let (bci, bcoi, bcli, _) = borrowing;
        let (pci, pcoi, pcli, _) = parent;
        data.continents[bci].countries[bcoi].clubs[bcli]
            .finance
            .register_obligated_purchase(fee as f64, 4);
        if let Some(departed) = data.player(event.player_id).map(|player| {
            DepartedPlayer::away(player, DepartureKind::BoughtOut, date).for_fee(fee as f64)
        }) {
            data.continents[pci].countries[pcoi].clubs[pcli].on_player_departed(&departed);
        }
    }

    fn complete_loan_buyout(
        data: &mut SimulatorData,
        event: LoanReturnEvent,
        parent: (usize, usize, usize, usize),
        fee: u32,
        parent_info: &TeamInfo,
        buying_league_reputation: u16,
        selling_league_reputation: u16,
        mandate: SigningMandate,
        date: NaiveDate,
    ) {
        let Some((ci, coi, cli, ti)) = data.find_player_position(event.player_id) else {
            return;
        };
        let obligations = {
            let Some(player) = data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                .players
                .players
                .iter_mut()
                .find(|player| player.id == event.player_id)
            else {
                return;
            };
            let obligations = player.drain_sell_on_obligations();
            player.complete_transfer(TransferCompletion {
                from: parent_info,
                history_source: parent_info,
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
                mandate: Some(mandate),
                source_is_rival: false,
                record_decision: false,
                loan_buyout: true,
            });
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

        let (pci, pcoi, pcli, _) = parent;
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
        let Some(parent_pos) = data.find_club_main_team(event.parent_club_id) else {
            debug!(
                "Loan return skipped: parent club {} not found for player {}",
                event.parent_club_id, event.player_id
            );
            return;
        };
        let parent_info = Self::parent_loan_team_info(data, parent_pos);
        let Some((mut player, borrower_country_id)) = Self::take_loaned_player(data, &event, date)
        else {
            return;
        };
        let (spell, loan_band, loan_apps, loan_rating) =
            Self::record_loan_spell(&mut player, &event, &parent_info, date);
        Self::place_returned_player(
            data,
            player,
            &event,
            parent_pos,
            &parent_info,
            spell,
            loan_band,
            borrower_country_id,
            loan_apps,
            loan_rating,
            record_return,
            date,
        );
    }

    fn parent_loan_team_info(
        data: &SimulatorData,
        parent_pos: (usize, usize, usize, usize),
    ) -> TeamInfo {
        let (pci, pcoi, pcli, pti) = parent_pos;
        let country = &data.continents[pci].countries[pcoi];
        let team = &country.clubs[pcli].teams.teams[pti];
        let league = team
            .league_id
            .and_then(|league_id| country.leagues.leagues.iter().find(|l| l.id == league_id));
        TeamInfo {
            name: team.name.clone(),
            slug: team.slug.clone(),
            reputation: team.reputation.world,
            league_name: league.map(|l| l.name.clone()).unwrap_or_default(),
            league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
        }
    }

    fn take_loaned_player(
        data: &mut SimulatorData,
        event: &LoanReturnEvent,
        date: NaiveDate,
    ) -> Option<(Player, u32)> {
        let player_pos = data.find_player_position(event.player_id);
        let borrower_country_id = player_pos
            .map(|(ci, coi, _, _)| data.continents[ci].countries[coi].id)
            .unwrap_or(0);
        let departed = player_pos.and_then(|(ci, coi, cli, _)| {
            DepartedPlayer::read(
                &data.continents[ci].countries[coi].clubs[cli],
                event.player_id,
                DepartureKind::LoanEnded,
                date,
            )
        });
        if let Some((ci, coi, cli, _)) = player_pos {
            SquadDepartures::notify(
                &mut data.continents[ci].countries[coi].clubs[cli],
                event.player_id,
                SeparationCause::LoanEnded,
                date,
            );
        }
        let mut player = match player_pos {
            Some((ci, coi, cli, ti)) => data.continents[ci].countries[coi].clubs[cli].teams.teams
                [ti]
                .players
                .take_player(&event.player_id)?,
            None => return None,
        };
        if let (Some((ci, coi, cli, _)), Some(departed)) = (player_pos, departed.as_ref()) {
            data.continents[ci].countries[coi].clubs[cli].on_player_departed(departed);
        }
        if let Some(concern) = event.turned_down {
            player.on_option_turned_down(concern, date);
        }
        if let Some(cause) = event.declined_buyout {
            player.on_buyout_declined(cause, date);
        }
        Some((player, borrower_country_id))
    }

    fn record_loan_spell(
        player: &mut Player,
        event: &LoanReturnEvent,
        parent_info: &TeamInfo,
        date: NaiveDate,
    ) -> (LoanSpellRecord, f32, u16, f32) {
        let loan_starts = player.statistics.played;
        let loan_apps = player.statistics.played + player.statistics.played_subs;
        let loan_rating = player
            .statistics
            .average_rating_realistic(player.position().position_group());
        let loan_spell_days = player
            .contract_loan
            .as_ref()
            .and_then(|loan| loan.started)
            .map(|started| (date - started).num_days())
            .unwrap_or(0);
        let spell = LoanSpellRecord::new(
            loan_starts,
            loan_apps,
            player.statistics.goals,
            player.statistics.assists,
            loan_rating,
            loan_spell_days,
        );
        let spell = if matches!(
            player.position().position_group(),
            PlayerFieldPositionGroup::Goalkeeper
        ) {
            spell.in_goal(player.statistics.clean_sheets, player.statistics.conceded)
        } else {
            spell
        };

        player.on_loan_return(&event.borrowing_info, parent_info, date);
        player.on_spell_change(
            SpellChange::loan_return(
                event.borrowing_club_id,
                event.borrowing_info.league_slug == parent_info.league_slug,
            ),
            0,
            date,
        );
        player.contract_loan = None;
        player.happiness = PlayerHappiness::new();
        player.reset_on_club_change();

        Self::apply_loan_return_mood(
            player,
            loan_starts,
            loan_apps,
            loan_rating,
            loan_spell_days,
            date,
        );

        let spell_verdict = spell.verdict;
        let loan_band = LevelBand::at_reputation(
            AbilityEstimator::observable_level(player),
            player.position().position_group(),
            (event.borrowing_info.reputation as f32 / 10_000.0).clamp(0.0, 1.0),
        );
        player.on_loan_spell_reviewed(spell_verdict, loan_band, date);
        let review = LoanEventContext::new(LoanEventKind::LoanSpellReviewed)
            .with_parent_club(event.parent_club_id)
            .with_loan_club(event.borrowing_club_id)
            .with_loan_days_elapsed(spell.days)
            .with_actual_apps(spell.appearances)
            .with_spell(spell);
        let review_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Boardroom,
        )
        .with_loan_context(review);
        player.happiness.add_event_with_context(
            HappinessEventType::LoanSpellReviewed,
            HappinessConfig::default().catalog.loan_spell_reviewed,
            None,
            review_ctx,
        );
        (spell, loan_band, loan_apps, loan_rating)
    }

    fn apply_loan_return_mood(
        player: &mut Player,
        loan_starts: u16,
        loan_apps: u16,
        loan_rating: f32,
        loan_spell_days: i64,
        date: NaiveDate,
    ) {
        let fringe_at_parent = player
            .contract
            .as_ref()
            .map(|contract| {
                matches!(
                    contract.squad_status,
                    PlayerSquadStatus::MainBackupPlayer | PlayerSquadStatus::NotNeeded
                )
            })
            .unwrap_or(false);
        let proved_himself = loan_starts >= 12 && loan_rating >= 6.5;
        let runway = CareerRunway::at(DateUtils::age(player.birth_date, date));
        if fringe_at_parent && proved_himself {
            let magnitude = HappinessConfig::default()
                .catalog
                .unsettled_after_loan_return
                * (1.0f32 - runway).max(0.35);
            player
                .happiness
                .add_event(HappinessEventType::UnsettledAfterLoanReturn, magnitude);
        } else if proved_himself {
            let magnitude = HappinessConfig::default().catalog.returned_from_loan_proven;
            player
                .happiness
                .add_event(HappinessEventType::ReturnedFromLoanProven, magnitude);
        } else if (loan_apps >= 8 && loan_rating < 6.2)
            || (loan_spell_days >= 120 && loan_apps <= 4)
        {
            let pressure01 = (player.attributes.pressure / 20.0).clamp(0.0, 1.0);
            let magnitude = HappinessConfig::default()
                .catalog
                .returned_from_loan_deflated
                * (1.3 - 0.6 * pressure01);
            player
                .happiness
                .add_event(HappinessEventType::ReturnedFromLoanDeflated, magnitude);
        }
    }

    fn place_returned_player(
        data: &mut SimulatorData,
        mut player: Player,
        event: &LoanReturnEvent,
        parent_pos: (usize, usize, usize, usize),
        parent_info: &TeamInfo,
        spell: LoanSpellRecord,
        loan_band: f32,
        borrower_country_id: u32,
        loan_apps: u16,
        loan_rating: f32,
        record_return: bool,
        date: NaiveDate,
    ) {
        if record_return {
            player.decision_history.add_move(
                date,
                &event.borrowing_info.name,
                &parent_info.name,
                0.0,
                "dec_loan_returned",
            );
        }
        Self::add_returned_player(
            data,
            player,
            event,
            parent_pos,
            spell,
            loan_band,
            borrower_country_id,
            loan_apps,
            loan_rating,
            date,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn add_returned_player(
        data: &mut SimulatorData,
        player: Player,
        event: &LoanReturnEvent,
        parent_pos: (usize, usize, usize, usize),
        spell: LoanSpellRecord,
        loan_band: f32,
        borrower_country_id: u32,
        loan_apps: u16,
        loan_rating: f32,
        date: NaiveDate,
    ) {
        let (pci, pcoi, pcli, pti) = parent_pos;
        debug!(
            "Loan return: player {} from club {} back to club {}",
            event.player_id, event.borrowing_club_id, event.parent_club_id
        );
        data.continents[pci].countries[pcoi].clubs[pcli].teams.teams[pti]
            .players
            .add(player);
        data.continents[pci].countries[pcoi].clubs[pcli].on_loanee_returned(
            event.player_id,
            &spell,
            loan_band,
            date,
        );
        if borrower_country_id != data.continents[pci].countries[pcoi].id {
            data.continents[pci].countries[pcoi].clubs[pcli]
                .on_loanee_returned_from(borrower_country_id, spell.start_share());
        }
        if let Some(coach) = data.continents[pci].countries[pcoi].clubs[pcli]
            .teams
            .main_mut()
            .and_then(|team| team.staffs.head_coach_mut())
        {
            SpellCloser::resume_from_loan(coach, event.player_id, loan_apps, loan_rating, date);
        }
    }

    /// Borrowing-side `TeamInfo` for a loan-return event, aliased to the
    /// club's main team for non-Main squads exactly like
    /// `scan_expired_loans` does — career history only owns senior slugs.
    fn loan_team_info(country: &Country, club: &Club, team: &Team) -> TeamInfo {
        let main_team = club.teams.main();
        let (name, slug, reputation) = match main_team {
            Some(m) if team.team_type != TeamType::Main => {
                (m.name.clone(), m.slug.clone(), m.reputation.world)
            }
            _ => (team.name.clone(), team.slug.clone(), team.reputation.world),
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
        // Twice a month. A spell that has gone wrong is a fortnight of a
        // boy's season, and a depth emergency at the parent is this
        // weekend's problem — a monthly pass answered either of them an
        // average of a fortnight late.
        if date.day() != 1 && date.day() != 15 {
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
        let mut emergency_taken: FxHashSet<(u32, PlayerFieldPositionGroup)> = FxHashSet::default();

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
                        turned_down: None,
                        declined_buyout: None,
                    });
                }
            }
        }

        for event in events {
            let player_id = event.player_id;
            Self::execute_loan_return(data, event, date, false);
            // Stamp the recall on the player's decision history so the
            // early return reads as a club decision, not a mystery.
            if let Some((ci, coi, cli, ti)) = data.find_player_position(player_id)
                && let Some(player) = data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
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

    /// Monthly backstop: players past the age any career runs to retire
    /// now rather than lingering in a squad until the season-end verdict.
    pub(crate) fn process_overdue_retirements(country: &mut Country, date: NaiveDate) {
        let retiring = Self::retirements(country, |player| player.overdue_retirement(date));
        Self::retire_players(country, retiring, date);
    }

    pub(crate) fn process_player_retirements(country: &mut Country, date: NaiveDate) {
        debug!("Processing player retirements");
        let retiring = Self::retirements(country, |player| player.season_end_retirement(date));
        Self::retire_players(country, retiring, date);
    }

    /// `(club_idx, team_idx, player_id, reason)` for every squad player
    /// whose own verdict says he stops.
    fn retirements(
        country: &Country,
        verdict: impl Fn(&Player) -> Option<RetirementReason>,
    ) -> Vec<(usize, usize, u32, RetirementReason)> {
        let mut retiring = Vec::new();
        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.iter().enumerate() {
                for player in team.players.iter() {
                    if let Some(reason) = verdict(player) {
                        retiring.push((club_idx, team_idx, player.id, reason));
                    }
                }
            }
        }
        retiring
    }

    fn retire_players(
        country: &mut Country,
        retiring: Vec<(usize, usize, u32, RetirementReason)>,
        date: NaiveDate,
    ) {
        let league_lookup = ClubIdentity::league_lookup(country);
        for (club_idx, team_idx, player_id, reason) in retiring {
            // The dugout hears about it first: a man retiring on him is one
            // of the few partings a manager remembers warmly.
            SquadDepartures::notify(
                &mut country.clubs[club_idx],
                player_id,
                SeparationCause::HeRetired,
                date,
            );
            let club = &mut country.clubs[club_idx];
            // The spell closes under the slug it was seeded with — youth
            // and Reserve squads alias to the parent club's main team.
            let from = ClubIdentity::resolve(club, &league_lookup)
                .team_info_for(&club.teams.teams[team_idx]);
            if let Some(mut player) = club.teams.teams[team_idx].players.take_player(&player_id) {
                debug!(
                    "Player retired: {} (age {})",
                    player.full_name,
                    player.age(date)
                );
                player.retire_from_squad(&from, date, reason);
                country.retired_players.add(player);
            }
        }
    }

    pub(crate) fn process_year_end_finances(country: &mut Country) {
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

    /// Hand the league the points deduction owed by any club that has just
    /// entered administration.
    ///
    /// The deduction is what makes administration a real sanction rather
    /// than a free debt write-off — without it, running up an unpayable
    /// wage bill and then having it cancelled would be a dominant strategy.
    /// Applied exactly once per administration via the `deduction_applied`
    /// memo on the club's state.
    pub(crate) fn process_administration_sanctions(country: &mut Country) {
        // Collect first: applying the deduction needs a mutable borrow of
        // the league table while the club is also borrowed mutably.
        let mut pending: Vec<(u32, u32, u8)> = Vec::new();
        for club in &country.clubs {
            let Some(state) = club.finance.debt.administration else {
                continue;
            };
            if state.deduction_applied {
                continue;
            }
            let Some(team) = club.teams.main() else {
                continue;
            };
            let Some(league_id) = team.league_id else {
                continue;
            };
            pending.push((club.id, league_id, state.points_deduction));
        }

        for (club_id, league_id, points) in pending {
            let team_id = country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.teams.main())
                .map(|t| t.id);
            let Some(team_id) = team_id else {
                continue;
            };
            if let Some(league) = country
                .leagues
                .leagues
                .iter_mut()
                .find(|l| l.id == league_id && !l.friendly)
            {
                league.table.apply_points_deduction(team_id, points);
                info!(
                    "⚖️ Administration: club {} docked {} points in league {}",
                    club_id, points, league_id
                );
            }
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id)
                && let Some(state) = club.finance.debt.administration.as_mut()
            {
                state.deduction_applied = true;
            }
        }
    }

    /// Step every parachute entitlement on by one season, dropping those
    /// that have run their three years. Runs immediately before the
    /// promotion/relegation swap, which then re-stamps this season's
    /// relegated clubs at season zero.
    pub(crate) fn age_parachute_entitlements(country: &mut Country) {
        for club in &mut country.clubs {
            if let Some(entitlement) = club.finance.parachute {
                club.finance.parachute = entitlement.advanced();
            }
        }
    }

    pub(crate) fn process_promotion_relegation(country: &mut Country, date: NaiveDate) {
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

        // Promoted CLUB ids across every league pair — the transfer
        // market's promotion add-on clauses key on the buying club, and
        // this pipeline is the only place that knows who went up.
        let mut promoted_club_ids: Vec<u32> = Vec::new();
        let reserve_ceilings = Self::reserve_ceilings(country);

        // Split-season grouped competitions (Argentine Primera) relegate
        // by the ANNUAL cross-zone table — both drops can come from one
        // zone, and each vacated slot is refilled by a lower-group
        // champion. Those zones are then skipped by the generic per-league
        // loop below.
        let (split_pairs, split_handled) =
            Self::split_competition_swap_pairs(&country.leagues.leagues);
        for (tier1_id, tier2_id, relegated, promoted) in split_pairs {
            let relegated: Vec<(u32, u32)> = relegated.iter().map(|&t| (t, tier2_id)).collect();
            let promoted: Vec<(u32, u32)> = promoted.iter().map(|&t| (t, tier2_id)).collect();
            Self::apply_promotion_relegation_swap(
                country,
                date,
                tier1_id,
                &relegated,
                &promoted,
                &mut promoted_club_ids,
            );
        }

        for &(tier1_id, tier1_tier, relegation_spots, _) in &league_info {
            if relegation_spots == 0 || tier1_tier == 0 || split_handled.contains(&tier1_id) {
                continue;
            }
            let Some((relegated, promoted)) =
                Self::boundary_swap(&country.leagues.leagues, tier1_id, &reserve_ceilings)
            else {
                continue;
            };
            Self::apply_promotion_relegation_swap(
                country,
                date,
                tier1_id,
                &relegated,
                &promoted,
                &mut promoted_club_ids,
            );
        }

        // Fire any promotion add-on clauses owed on the promoted clubs'
        // transfer deals — the resolver had no caller before this, so
        // every scheduled promotion bonus silently expired unpaid.
        TransferClauseSettler::settle_promotions(country, date, &promoted_club_ids);

        // Clear final tables after processing
        for league in &mut country.leagues.leagues {
            league.final_table = None;
        }
    }

    /// The season-end swap across `tier1_id`'s lower boundary, read off the
    /// frozen final tables: `(relegated, promoted)` as `(team, lower
    /// league)` pairs — the league a relegated side drops into, or the one a
    /// promoted side leaves.
    ///
    /// Each lower league promotes its share from `LeagueLadder::
    /// relegation_split` and takes back exactly as many relegated sides, so
    /// a tier split into groups keeps every group at constant size. The
    /// relegated sides are dealt out worst-first, one per group in turn.
    ///
    /// Reserve sides never go up into their first team's tier (Jong Ajax
    /// below Ajax, Castilla below Real Madrid); the next eligible side in
    /// the table takes the place.
    #[allow(clippy::type_complexity)]
    fn boundary_swap(
        leagues: &[crate::league::League],
        tier1_id: u32,
        reserve_ceilings: &FxHashMap<u32, u8>,
    ) -> Option<(Vec<(u32, u32)>, Vec<(u32, u32)>)> {
        let upper = leagues.iter().find(|l| l.id == tier1_id)?;
        let split: Vec<(u32, usize)> = LeagueLadder::new(leagues)
            .relegation_split(upper)
            .into_iter()
            .map(|(lower, places)| (lower.id, places))
            .collect();
        let nominal_swap: usize = split.iter().map(|&(_, places)| places).sum();
        if nominal_swap == 0 {
            return None;
        }

        // Worst first.
        let relegated_candidates: Vec<u32> = upper
            .final_table
            .as_ref()
            .map(|table| {
                table
                    .iter()
                    .rev()
                    .take(nominal_swap)
                    .map(|r| r.team_id)
                    .collect()
            })
            .unwrap_or_default();

        // Champions first across the lower leagues, then runners-up, so a
        // truncated swap drops the lowest-placed promotions.
        let lower_tops: Vec<(u32, Vec<u32>)> = split
            .iter()
            .map(|&(lower_id, places)| {
                let top = leagues
                    .iter()
                    .find(|l| l.id == lower_id)
                    .and_then(|l| l.final_table.as_ref())
                    .map(|t| {
                        t.iter()
                            .map(|r| r.team_id)
                            .filter(|&team| {
                                Self::may_go_up(reserve_ceilings, team, upper.settings.tier)
                            })
                            .take(places)
                            .collect()
                    })
                    .unwrap_or_default();
                (lower_id, top)
            })
            .collect();
        let deepest = lower_tops
            .iter()
            .map(|(_, top)| top.len())
            .max()
            .unwrap_or(0);
        let mut promoted: Vec<(u32, u32)> = Vec::new();
        for rank in 0..deepest {
            for (lower_id, top) in &lower_tops {
                if let Some(&team) = top.get(rank) {
                    promoted.push((team, *lower_id));
                }
            }
        }

        // Must balance: never relegate more than we promote (or vice versa)
        // or the top league silently shrinks each season.
        let swap_count = relegated_candidates.len().min(promoted.len());
        if swap_count == 0 {
            return None;
        }
        if swap_count < nominal_swap {
            info!(
                "⚠️ Promotion/relegation below {} truncated: wanted {}, got {} (missing final_table entries)",
                tier1_id, nominal_swap, swap_count
            );
        }
        promoted.truncate(swap_count);

        // Each lower league takes back as many sides as it sent up.
        let mut vacancies: Vec<(u32, usize)> = split
            .iter()
            .map(|&(lower_id, _)| {
                let sent = promoted
                    .iter()
                    .filter(|&&(_, from)| from == lower_id)
                    .count();
                (lower_id, sent)
            })
            .collect();
        let lowers = vacancies.len();
        let mut relegated: Vec<(u32, u32)> = Vec::with_capacity(swap_count);
        let mut next = 0;
        for team in relegated_candidates.into_iter().take(swap_count) {
            while vacancies[next % lowers].1 == 0 {
                next += 1;
            }
            let slot = &mut vacancies[next % lowers];
            slot.1 -= 1;
            relegated.push((team, slot.0));
            next += 1;
        }

        Some((relegated, promoted))
    }

    /// Every reserve side in a league (B team, U23, Jong) mapped to the tier
    /// its club's first team plays in.
    fn reserve_ceilings(country: &Country) -> FxHashMap<u32, u8> {
        let tier_of: FxHashMap<u32, u8> = country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, l.settings.tier))
            .collect();
        let mut ceilings = FxHashMap::default();
        for club in &country.clubs {
            let Some(main_tier) = club
                .teams
                .teams
                .iter()
                .find(|t| t.team_type == TeamType::Main)
                .and_then(|t| t.league_id)
                .and_then(|id| tier_of.get(&id).copied())
            else {
                continue;
            };
            for team in club.teams.teams.iter() {
                if team.team_type != TeamType::Main && team.league_id.is_some() {
                    ceilings.insert(team.id, main_tier);
                }
            }
        }
        ceilings
    }

    /// Whether `team_id` may be promoted into `tier`: a reserve side must
    /// stay strictly below its first team.
    fn may_go_up(reserve_ceilings: &FxHashMap<u32, u8>, team_id: u32, tier: u8) -> bool {
        reserve_ceilings
            .get(&team_id)
            .is_none_or(|&main_tier| tier > main_tier)
    }

    /// Swap pairs for split-season grouped competitions (Argentine
    /// Primera): relegation reads the ANNUAL cross-zone table, so the
    /// dropped sides may both come from one zone. Each returned pair is
    /// `(zone_id, group_id, relegated, promoted)` where the promoted side
    /// fills the exact vacancy the relegated side left (keeping both the
    /// zones and the lower groups at constant size).
    /// The second value lists every zone id the split path claimed, so
    /// the generic per-league loop skips them.
    #[allow(clippy::type_complexity)]
    fn split_competition_swap_pairs(
        leagues: &[crate::league::League],
    ) -> (Vec<(u32, u32, Vec<u32>, Vec<u32>)>, FxHashSet<u32>) {
        use std::collections::BTreeMap;

        // competition name → zone leagues (tier-1, split, grouped).
        let mut competitions: BTreeMap<&str, Vec<&crate::league::League>> = BTreeMap::new();
        for l in leagues {
            if !l.settings.split_season || l.settings.relegation_spots == 0 {
                continue;
            }
            if let Some(group) = &l.settings.league_group {
                competitions
                    .entry(group.competition.as_str())
                    .or_default()
                    .push(l);
            }
        }

        let mut pairs: Vec<(u32, u32, Vec<u32>, Vec<u32>)> = Vec::new();
        let mut handled: FxHashSet<u32> = FxHashSet::default();

        for (_, mut zones) in competitions {
            if zones.len() < 2 {
                continue;
            }
            zones.sort_by_key(|l| l.id);

            // Merged annual table across zones: (team_id, zone_id) rows
            // sorted best-first on the frozen annual final tables.
            let mut annual: Vec<(u32, u32, u8, i32, i32)> = Vec::new(); // team, zone, pts, gd, gs
            for z in &zones {
                let Some(table) = z.final_table.as_ref() else {
                    annual.clear();
                    break;
                };
                for r in table {
                    annual.push((
                        r.team_id,
                        z.id,
                        r.effective_points(),
                        r.goal_difference(),
                        r.goal_scored,
                    ));
                }
            }
            if annual.is_empty() {
                continue; // final tables not frozen — leave to the generic path
            }
            annual.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then(b.3.cmp(&a.3))
                    .then(b.4.cmp(&a.4))
                    .then(a.0.cmp(&b.0))
            });

            // Paired lower-division groups, one per zone (same mapping the
            // generic path uses).
            let ladder = LeagueLadder::new(leagues);
            let groups: Vec<u32> = zones
                .iter()
                .filter_map(|z| ladder.lower_partner(z.id))
                .map(|l| l.id)
                .collect();
            if groups.len() != zones.len() {
                continue;
            }

            // Bottom K of the annual table go down, worst first.
            let total_spots: usize = zones
                .iter()
                .map(|z| z.settings.relegation_spots as usize)
                .sum();
            let k = total_spots.min(groups.len());
            let relegated: Vec<(u32, u32)> = annual
                .iter()
                .rev()
                .take(k)
                .map(|&(team, zone, ..)| (team, zone))
                .collect();

            // Pair k-th relegated side with the k-th group: its champion
            // is promoted straight into the vacated zone.
            for (i, &(rel_team, rel_zone)) in relegated.iter().enumerate() {
                let group_id = groups[i];
                let promoted: Vec<u32> = leagues
                    .iter()
                    .find(|l| l.id == group_id)
                    .and_then(|l| l.final_table.as_ref())
                    .map(|t| t.iter().take(1).map(|r| r.team_id).collect())
                    .unwrap_or_default();
                if promoted.is_empty() {
                    continue;
                }
                pairs.push((rel_zone, group_id, vec![rel_team], promoted));
            }
            for z in &zones {
                handled.insert(z.id);
            }
        }

        (pairs, handled)
    }

    /// Apply one promotion/relegation swap between a top league and the
    /// league(s) below it: move the teams, fire the season-outcome events
    /// and contract clauses, move sub-teams to the matching youth leagues,
    /// and record the promoted club ids for transfer-clause settlement.
    ///
    /// `relegated` pairs each side with the lower league it drops into and
    /// `promoted` each side with the lower league it leaves — one call
    /// covers every lower group, so survival bonuses are paid once.
    fn apply_promotion_relegation_swap(
        country: &mut Country,
        date: NaiveDate,
        tier1_id: u32,
        relegated: &[(u32, u32)],
        promoted: &[(u32, u32)],
        promoted_club_ids: &mut Vec<u32>,
    ) {
        let swap = PromotionRelegationSwap::new(country, date, tier1_id, relegated, promoted);

        promoted_club_ids.extend(
            country
                .clubs
                .iter()
                .filter(|club| {
                    club.teams
                        .teams
                        .iter()
                        .any(|team| swap.promoted_team_ids.contains(&team.id))
                })
                .map(|club| club.id),
        );

        for club in &mut country.clubs {
            swap.swap_club(club);
        }

        for &team_id in &swap.promoted_team_ids {
            if let Some(club) = country
                .clubs
                .iter_mut()
                .find(|club| club.teams.iter().any(|team| team.id == team_id))
            {
                Self::award_trophy(club, team_id, AchievementType::Promotion, date);
            }
            Self::apply_team_squad_event(
                country,
                team_id,
                HappinessEventType::PromotionCelebration,
                365,
                1.0,
                date,
            );
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
    pub(crate) fn process_relegation_fear_audit(country: &mut Country, date: NaiveDate) {
        // Collect (team_id, in_zone, near_zone) from each non-friendly
        // league with a meaningful relegation slot count and enough
        // season progress to register pressure.
        let mut at_risk_teams: FxHashMap<u32, (u32, u8, u16, i16, u8)> = FxHashMap::default();

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
    use crate::TrophyKind;
    use crate::academy::ClubAcademy;
    use crate::club::board::mandate::MandatePurpose;
    use crate::club::finance::ParachuteEntitlement;
    use crate::club::mind::organs::memory::EpisodeKind;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::mind::CareerArc;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{
        DayMonthPeriod, DomesticCup, League, LeagueCollection, LeagueSettings, LeagueTableRow,
        ScheduleItem, ScheduleTour,
    };
    use crate::r#match::{Score, TeamScore};
    use crate::shared::Location;
    use crate::shared::{Currency, CurrencyValue};
    use crate::{
        Club, ClubColors, ClubFinances, ClubFinancialBalance, ClubStatus, LoanSpellVerdict,
        PersonAttributes, PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
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
                split_season: false,
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

    fn league_with_group(
        id: u32,
        tier: u8,
        promo: u8,
        releg: u8,
        competition: Option<&str>,
        group_name: &str,
    ) -> League {
        League::new(
            id,
            format!("L{id}"),
            format!("l-{id}"),
            1,
            5000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier,
                promotion_spots: promo,
                relegation_spots: releg,
                league_group: competition.map(|c| crate::league::LeagueGroup {
                    name: group_name.to_string(),
                    competition: c.to_string(),
                    total_groups: 2,
                    level: 0,
                    playoff: None,
                }),
                split_season: false,
            },
            false,
        )
    }

    fn table_row_for(team_id: u32, points: u8) -> crate::league::LeagueTableRow {
        crate::league::LeagueTableRow {
            team_id,
            played: 32,
            win: points / 3,
            draft: points % 3,
            lost: 0,
            goal_scored: 0,
            goal_concerned: 0,
            points,
            points_deduction: 0,
        }
    }

    /// Split-zone league with a frozen (annual) final table.
    fn split_zone_league(
        id: u32,
        competition: &str,
        group_name: &str,
        rows: Vec<(u32, u8)>,
    ) -> League {
        let mut league = league_with_group(id, 1, 0, 1, Some(competition), group_name);
        league.settings.split_season = true;
        league.final_table = Some(
            rows.into_iter()
                .map(|(team, pts)| table_row_for(team, pts))
                .collect(),
        );
        league
    }

    fn lower_group_league(id: u32, competition: &str, rows: Vec<(u32, u8)>) -> League {
        let mut league = league_with_group(id, 2, 1, 2, Some(competition), "Lower");
        league.final_table = Some(
            rows.into_iter()
                .map(|(team, pts)| table_row_for(team, pts))
                .collect(),
        );
        league
    }

    #[test]
    fn split_relegation_reads_annual_table_across_zones() {
        // Worst annual records sit one per zone: team 13 (Zona A, 5 pts)
        // and team 23 (Zona B, 8 pts).
        let leagues = vec![
            split_zone_league(100, "Primera", "Zona A", vec![(11, 40), (12, 30), (13, 5)]),
            split_zone_league(101, "Primera", "Zona B", vec![(21, 38), (22, 28), (23, 8)]),
            lower_group_league(200, "Primera", vec![(31, 60), (32, 50)]),
            lower_group_league(201, "Primera", vec![(41, 58), (42, 48)]),
        ];

        let (pairs, handled) = CountryResult::split_competition_swap_pairs(&leagues);
        assert!(handled.contains(&100) && handled.contains(&101));
        assert_eq!(pairs.len(), 2);
        // Worst overall (13, from zone 100) pairs with the first group;
        // its champion is promoted into that exact zone.
        assert_eq!(pairs[0].0, 100, "zone the vacancy opened in");
        assert_eq!(pairs[0].1, 200);
        assert_eq!(pairs[0].2, vec![13]);
        assert_eq!(pairs[0].3, vec![31]);
        assert_eq!(pairs[1].0, 101);
        assert_eq!(pairs[1].1, 201);
        assert_eq!(pairs[1].2, vec![23]);
        assert_eq!(pairs[1].3, vec![41]);
    }

    #[test]
    fn split_relegation_can_drop_both_teams_from_one_zone() {
        // Zona A holds BOTH worst annual records (teams 12 and 13); both
        // promoted champions land in Zona A, Zona B is untouched.
        let leagues = vec![
            split_zone_league(100, "Primera", "Zona A", vec![(11, 40), (12, 6), (13, 5)]),
            split_zone_league(101, "Primera", "Zona B", vec![(21, 38), (22, 28), (23, 20)]),
            lower_group_league(200, "Primera", vec![(31, 60), (32, 50)]),
            lower_group_league(201, "Primera", vec![(41, 58), (42, 48)]),
        ];

        let (pairs, _) = CountryResult::split_competition_swap_pairs(&leagues);
        assert_eq!(pairs.len(), 2);
        assert!(pairs.iter().all(|p| p.0 == 100), "both vacancies in Zona A");
        let mut relegated: Vec<u32> = pairs.iter().flat_map(|p| p.2.clone()).collect();
        relegated.sort_unstable();
        assert_eq!(relegated, vec![12, 13]);
        let mut promoted: Vec<u32> = pairs.iter().flat_map(|p| p.3.clone()).collect();
        promoted.sort_unstable();
        assert_eq!(promoted, vec![31, 41]);
    }

    #[test]
    fn non_split_competitions_are_left_to_the_generic_path() {
        let leagues = vec![
            league_with_group(100, 1, 0, 1, Some("Primera"), "Zona A"),
            league_with_group(101, 1, 0, 1, Some("Primera"), "Zona B"),
        ];
        let (pairs, handled) = CountryResult::split_competition_swap_pairs(&leagues);
        assert!(pairs.is_empty());
        assert!(handled.is_empty());
    }

    #[test]
    fn zones_pair_to_matching_second_division_groups() {
        // Two-zone top flight above a two-group second division: each zone
        // must relegate into its own group, not both into the first.
        let leagues = vec![
            league_with_group(100, 1, 0, 1, Some("Primera"), "Zona A"),
            league_with_group(101, 1, 0, 1, Some("Primera"), "Zona B"),
            league_with_group(200, 2, 1, 2, Some("Second"), "A"),
            league_with_group(201, 2, 1, 2, Some("Second"), "B"),
        ];
        // zone 0 → group 0, zone 1 → group 1 (ordered by id).
        assert_eq!(
            LeagueLadder::new(&leagues)
                .lower_partner(100)
                .map(|l| (l.id, l.settings.promotion_spots)),
            Some((200, 1))
        );
        assert_eq!(
            LeagueLadder::new(&leagues)
                .lower_partner(101)
                .map(|l| (l.id, l.settings.promotion_spots)),
            Some((201, 1))
        );
    }

    #[test]
    fn ungrouped_pairing_falls_back_to_first_match() {
        // A plain single-league tier boundary keeps the original behaviour.
        let leagues = vec![
            league_with_group(10, 1, 0, 3, None, ""),
            league_with_group(20, 2, 2, 0, None, ""),
        ];
        assert_eq!(
            LeagueLadder::new(&leagues)
                .lower_partner(10)
                .map(|l| (l.id, l.settings.promotion_spots)),
            Some((20, 2))
        );
    }

    #[test]
    fn ladder_moves_only_what_both_sides_of_a_boundary_allow() {
        let leagues = vec![
            league_with_group(10, 1, 0, 3, None, ""),
            league_with_group(20, 2, 2, 0, None, ""),
        ];
        let ladder = LeagueLadder::new(&leagues);
        assert_eq!(ladder.relegated_from_table(&leagues[0]), 2);
        assert_eq!(ladder.promoted_from_table(&leagues[1]), 2);
        assert_eq!(ladder.promoted_from_table(&leagues[0]), 0);
    }

    #[test]
    fn ladder_relegates_nobody_without_a_division_below() {
        let leagues = vec![league_with_group(10, 1, 0, 3, None, "")];
        assert_eq!(
            LeagueLadder::new(&leagues).relegated_from_table(&leagues[0]),
            0
        );
    }

    #[test]
    fn single_division_relegates_into_every_group_below() {
        // Segunda above Primera Federación's two groups: four go down,
        // two come up from each group.
        let leagues = vec![
            league_with_group(10, 2, 3, 4, None, ""),
            league_with_group(20, 3, 2, 0, Some("Primera Federación"), "Grupo 1"),
            league_with_group(21, 3, 2, 0, Some("Primera Federación"), "Grupo 2"),
        ];
        let ladder = LeagueLadder::new(&leagues);
        let split: Vec<(u32, usize)> = ladder
            .relegation_split(&leagues[0])
            .into_iter()
            .map(|(l, places)| (l.id, places))
            .collect();
        assert_eq!(split, vec![(20, 2), (21, 2)]);
        assert_eq!(ladder.relegated_from_table(&leagues[0]), 4);
        assert_eq!(ladder.promoted_from_table(&leagues[1]), 2);
        assert_eq!(ladder.promoted_from_table(&leagues[2]), 2);
    }

    #[test]
    fn group_promotion_places_go_champions_first() {
        // Three places over two groups that promote two each: both
        // champions, then the first group's runner-up.
        let mut leagues = vec![
            league_with_group(10, 2, 3, 3, None, ""),
            league_with_group(20, 3, 2, 0, Some("Primera Federación"), "Grupo 1"),
            league_with_group(21, 3, 2, 0, Some("Primera Federación"), "Grupo 2"),
        ];
        let ladder = LeagueLadder::new(&leagues);
        assert_eq!(ladder.relegated_from_table(&leagues[0]), 3);
        assert_eq!(ladder.promoted_from_table(&leagues[1]), 2);
        assert_eq!(ladder.promoted_from_table(&leagues[2]), 1);

        // A single place goes to the first group's champion alone.
        leagues[0].settings.relegation_spots = 1;
        let ladder = LeagueLadder::new(&leagues);
        assert_eq!(ladder.promoted_from_table(&leagues[1]), 1);
        assert_eq!(ladder.promoted_from_table(&leagues[2]), 0);
    }

    /// Russia's lower pyramid: First League (10) above Division A Gold
    /// (20) and Silver (21, ranked one level below Gold inside tier 3),
    /// above Division B's four zones (30..=33).
    fn ranked_group_pyramid() -> Vec<League> {
        let mut silver = league_with_group(21, 3, 2, 4, Some("Division A"), "Silver");
        if let Some(group) = silver.settings.league_group.as_mut() {
            group.level = 1;
        }
        let mut leagues = vec![
            league_with_group(10, 2, 0, 2, None, ""),
            league_with_group(20, 3, 2, 2, Some("Division A"), "Gold"),
            silver,
        ];
        for (id, zone) in (30u32..).zip(["South", "Northwest", "Center", "Ural-Volga"]) {
            leagues.push(league_with_group(id, 4, 1, 0, Some("Division B"), zone));
        }
        leagues
    }

    #[test]
    fn ranked_groups_are_rungs_inside_their_tier() {
        let leagues = ranked_group_pyramid();
        let ladder = LeagueLadder::new(&leagues);
        let split = |idx: usize| -> Vec<(u32, usize)> {
            ladder
                .relegation_split(&leagues[idx])
                .into_iter()
                .map(|(l, places)| (l.id, places))
                .collect()
        };
        // First League drops into Gold only; Gold into Silver; Silver
        // feeds every Division B zone.
        assert_eq!(split(0), vec![(20, 2)]);
        assert_eq!(split(1), vec![(21, 2)]);
        assert_eq!(split(2), vec![(30, 1), (31, 1), (32, 1), (33, 1)]);

        assert_eq!(ladder.promoted_from_table(&leagues[1]), 2);
        assert_eq!(ladder.promoted_from_table(&leagues[2]), 2);
        assert_eq!(ladder.relegated_from_table(&leagues[2]), 4);
        for zone in &leagues[3..] {
            assert_eq!(ladder.promoted_from_table(zone), 1, "zone {}", zone.id);
            assert_eq!(ladder.relegated_from_table(zone), 0, "zone {}", zone.id);
        }
    }

    #[test]
    fn ranked_groups_swap_sides_every_season() {
        // Gold 20..=25, Silver 40..=47, zones 60.., 70.., 80.., 90..
        let gold_ids: Vec<u32> = (20..=25).collect();
        let silver_ids: Vec<u32> = (40..=47).collect();
        let zone_ids: Vec<Vec<u32>> = [60u32, 70, 80, 90]
            .iter()
            .map(|&base| (base..base + 3).collect())
            .collect();
        let table = |ids: &[u32]| -> Vec<(u32, u8, u8)> {
            ids.iter()
                .enumerate()
                .map(|(i, &id)| (id, 30, 90 - (i as u8) * 5))
                .collect()
        };

        let mut clubs: Vec<Club> = Vec::new();
        let mut add = |ids: &[u32], league_id: u32| {
            clubs.extend(
                ids.iter()
                    .map(|&id| make_club(id, vec![make_simple_team(id, id, league_id)])),
            );
        };
        add(&gold_ids, 20);
        add(&silver_ids, 21);
        for (ids, league_id) in zone_ids.iter().zip(30u32..) {
            add(ids, league_id);
        }

        let mut leagues = Vec::new();
        for template in ranked_group_pyramid().into_iter().skip(1) {
            let ids: &[u32] = match template.id {
                20 => &gold_ids,
                21 => &silver_ids,
                id => &zone_ids[(id - 30) as usize],
            };
            let s = &template.settings;
            let mut league = make_league_with_settings(
                template.id,
                s.tier,
                s.promotion_spots,
                s.relegation_spots,
                table(ids),
            );
            league.settings.league_group = s.league_group.clone();
            leagues.push(league);
        }
        let mut country = build_country(clubs, leagues);
        // Two Gold sides drop with a parachute already running from an
        // earlier fall out of the First League.
        for id in [24u32, 25] {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == id) {
                club.finance.parachute = Some(ParachuteEntitlement {
                    from_tier: 2,
                    seasons_elapsed: 1,
                });
            }
        }

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        let league_of = |club_id: u32| {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.teams.iter().next())
                .and_then(|t| t.league_id)
        };
        // Gold ↔ Silver: two each way.
        assert_eq!((league_of(24), league_of(25)), (Some(21), Some(21)));
        assert_eq!((league_of(40), league_of(41)), (Some(20), Some(20)));
        // Silver's bottom four go one to each zone; every zone champion
        // goes up.
        for id in [44u32, 45, 46, 47] {
            assert!(
                (30..=33).contains(&league_of(id).unwrap_or(0)),
                "silver {id} relegated"
            );
        }
        for ids in &zone_ids {
            assert_eq!(league_of(ids[0]), Some(21), "zone champion {}", ids[0]);
        }
        // Every division keeps its size.
        let size = |league_id: u32| {
            country
                .clubs
                .iter()
                .filter(|c| c.teams.iter().next().and_then(|t| t.league_id) == Some(league_id))
                .count()
        };
        assert_eq!([20u32, 21, 30, 31, 32, 33].map(size), [6, 8, 3, 3, 3, 3]);
        // A same-tier drop neither grants nor resets a parachute.
        let parachute = |club_id: u32| {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.finance.parachute)
                .map(|p| (p.from_tier, p.seasons_elapsed))
        };
        assert_eq!(parachute(24), Some((2, 1)));
        assert_eq!(parachute(40), None);
        // Falling out of tier 3 still grants one.
        assert_eq!(parachute(47), Some((3, 0)));
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

    /// A midfielder whose skills read as `level` to anybody watching him.
    fn skilled_midfielder(id: u32, level: u8) -> Player {
        let mut p = make_player_with_position(id);
        p.player_attributes.current_ability = level;
        p.skills = PlayerSkills::flat_for_ability(level);
        p.contract = Some(PlayerClubContract::new(30_000, d(2028, 6, 30)));
        p
    }

    /// A month of income behind a club, so its board has a revenue to
    /// price a purchase against.
    fn with_income(club: &mut Club) {
        let mut month = ClubFinancialBalance::new(0);
        month.income = 3_000_000;
        club.finance.history.add(d(2027, 5, 1), month);
    }

    /// A borrower whose own midfielders all sit at level 100.
    fn hearing_borrower() -> Club {
        let own = (60..63).map(|id| skilled_midfielder(id, 100)).collect();
        let mut club = make_club(200, vec![make_team(20, 200, 1, own)]);
        with_income(&mut club);
        club
    }

    /// A loanee of `level` who held `role` at the borrower over `apps`
    /// games at an average of `rating`.
    fn hearing_loanee(level: u8, role: PlayerSquadStatus, apps: u16, rating: f32) -> Player {
        let mut p = skilled_midfielder(55, level);
        if let Some(contract) = p.contract.as_mut() {
            contract.squad_status = role;
        }
        p.statistics.played = apps;
        p.statistics.rating_points = rating * apps as f32;
        p.statistics.rating_weight = apps as f32;
        p
    }

    #[test]
    fn loan_obligation_always_converts() {
        // No appearances at all — an obligation is binding regardless.
        let mut p = make_player_with_position(1);
        p.statistics.played = 0;
        let decision = CountryResult::decide_loan_buyout(
            &make_club(200, vec![]),
            &p,
            &loan_deal(Some(2_000_000), true),
            d(2026, 6, 30),
        );
        assert!(
            matches!(
                decision,
                Some(LoanBuyoutDecision::Buy(LoanBuyout {
                    fee: 2_000_000,
                    obligation: true,
                    ..
                }))
            ),
            "{decision:?}"
        );
    }

    #[test]
    fn thriving_loanee_option_is_exercised() {
        // A season of starts at a borrower whose own midfield is forty
        // points short of him: its board buys the starter it cannot
        // replace.
        let decision = CountryResult::decide_loan_buyout(
            &hearing_borrower(),
            &hearing_loanee(140, PlayerSquadStatus::FirstTeamRegular, 22, 6.9),
            &loan_deal(Some(2_000_000), false),
            d(2027, 5, 28),
        );
        assert!(
            matches!(
                decision,
                Some(LoanBuyoutDecision::Buy(buyout))
                    if !buyout.obligation && buyout.mandate.purpose == MandatePurpose::Starter
            ),
            "a loan that worked is heard and bought as a starter: {decision:?}"
        );
    }

    #[test]
    fn failed_loan_option_lapses() {
        // A barely-used body for the bench, no better than the men already
        // there, at a fee no board pays for cover.
        let decision = CountryResult::decide_loan_buyout(
            &hearing_borrower(),
            &hearing_loanee(100, PlayerSquadStatus::MainBackupPlayer, 6, 6.2),
            &loan_deal(Some(20_000_000), false),
            d(2027, 5, 28),
        );
        assert!(
            matches!(
                decision,
                Some(LoanBuyoutDecision::TurnedDown(
                    BoardTransferConcern::FinancialDiscipline
                ))
            ),
            "the board turns the option down on price: {decision:?}"
        );
    }

    #[test]
    fn plain_loan_has_no_buyout() {
        let decision = CountryResult::decide_loan_buyout(
            &hearing_borrower(),
            &make_player_with_position(1),
            &loan_deal(None, false),
            d(2026, 6, 30),
        );
        assert!(decision.is_none());
    }

    /// A season of starts behind an option to buy, at a borrower in the
    /// division `borrower_league` carries, owned by a top-flight giant.
    fn option_world(borrower_league: u32, borrower_rep: u16) -> SimulatorData {
        let team_with_rep = |id: u32, club_id: u32, league_id: u32, rep: u16, players| {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(league_id))
                .club_id(club_id)
                .name(format!("Team{}", id))
                .slug(format!("team{}", id))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(rep, rep, rep))
                .training_schedule(make_training_schedule())
                .build()
                .unwrap()
        };
        let parent_team = team_with_rep(10, 100, 1, 8_900, vec![make_player_with_position(11)]);
        let parent_club = make_club(100, vec![parent_team]);

        // A fifty-cap international the giant has just bought: a name
        // worth a top-flight division, whatever his club form says. He
        // started the season at the borrower, which is the role its squad
        // pass wrote on his contract.
        let mut loanee = hearing_loanee(140, PlayerSquadStatus::FirstTeamRegular, 22, 6.9);
        loanee.player_attributes.current_reputation = 7_000;
        loanee.player_attributes.home_reputation = 7_000;
        loanee.player_attributes.world_reputation = 7_000;
        loanee.player_attributes.international_apps = 51;
        // On an international's wage, which is what a third-tier club's
        // offer is measured against.
        if let Some(contract) = loanee.contract.as_mut() {
            contract.salary = 4_000_000;
        }
        let mut loan = PlayerClubContract::new(30_000, d(2027, 5, 28));
        loan.loan_from_club_id = Some(100);
        loan.started = Some(d(2026, 9, 1));
        loan.loan_future_fee = Some(2_000_000);
        loanee.contract_loan = Some(loan);
        // The borrower's own midfield is forty points short of him, so its
        // board hears the option as a starter it cannot replace.
        let mut borrower_squad = vec![loanee];
        borrower_squad.extend((60..63).map(|id| skilled_midfielder(id, 100)));
        let borrower_team = team_with_rep(20, 200, borrower_league, borrower_rep, borrower_squad);
        let mut borrower_club = make_club(200, vec![borrower_team]);
        with_income(&mut borrower_club);

        let top_flight = make_league_with_table(1, 8_750, vec![]);
        let third_tier = make_league_with_table(2, 3_500, vec![]);
        let country = build_country(
            vec![parent_club, borrower_club],
            vec![top_flight, third_tier],
        );
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            d(2027, 5, 28).and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn holds_player(data: &SimulatorData, club_id: u32) -> Option<&Player> {
        data.country(1)
            .unwrap()
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams[0].players.players.iter().find(|p| p.id == 55))
    }

    /// The borrower exercises its option; the player will not sign for
    /// a division under his floor, so the option lapses and he goes
    /// home with his own refusal on the record.
    #[test]
    fn an_international_declines_the_option_of_a_third_tier_club() {
        let mut data = option_world(2, 3_500);
        CountryResult::process_loan_returns(&mut data, 1, d(2027, 5, 28));

        let returned = holds_player(&data, 100).expect("home with the parent");
        assert!(returned.contract_loan.is_none());
        assert!(
            returned
                .decision_history
                .items
                .iter()
                .any(|row| row.movement == "dec_loan_buyout_declined"),
            "his refusal is a decision in his name"
        );
        assert!(holds_player(&data, 200).is_none());
    }

    /// The same season, the same option, at a club in his own division:
    /// he signs, and ownership flips in place.
    #[test]
    fn the_same_option_is_signed_at_a_club_in_his_own_division() {
        let mut data = option_world(1, 7_000);
        CountryResult::process_loan_returns(&mut data, 1, d(2027, 5, 28));

        let bought = holds_player(&data, 200).expect("stays where he played");
        assert!(bought.contract_loan.is_none(), "the loan became a purchase");
        assert!(holds_player(&data, 100).is_none());
    }

    fn loanee_mut(data: &mut SimulatorData) -> &mut Player {
        data.country_mut(1)
            .unwrap()
            .clubs
            .iter_mut()
            .find(|c| c.id == 200)
            .and_then(|c| {
                c.teams.teams[0]
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == 55)
            })
            .unwrap()
    }

    /// A season on the bench and an option priced like a starter: the
    /// borrower's board turns it down, and he goes home with the board's
    /// refusal on his record.
    #[test]
    fn a_board_does_not_buy_a_body_for_its_bench() {
        let mut data = option_world(1, 7_000);
        let loanee = loanee_mut(&mut data);
        if let Some(contract) = loanee.contract.as_mut() {
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
        }
        if let Some(loan) = loanee.contract_loan.as_mut() {
            loan.loan_future_fee = Some(400_000_000);
        }
        CountryResult::process_loan_returns(&mut data, 1, d(2027, 5, 28));

        let returned = holds_player(&data, 100).expect("home with the parent");
        assert!(returned.contract_loan.is_none());
        assert!(
            returned.decision_history.items.iter().any(|row| {
                row.movement == "dec_loan_option_turned_down"
                    && row.decision == "board_concern_financial_discipline"
                    && row.decided_by == "dec_decided_board"
            }),
            "the refusal is a decision in the board's name"
        );
        assert!(holds_player(&data, 200).is_none());
    }

    /// An option the borrower cannot pay for lapses, and he goes home with
    /// the board's refusal on his record.
    #[test]
    fn an_option_the_borrower_cannot_afford_lapses() {
        let mut data = option_world(1, 7_000);
        if let Some(borrower) = data
            .country_mut(1)
            .unwrap()
            .clubs
            .iter_mut()
            .find(|c| c.id == 200)
        {
            borrower.finance.transfer_budget = Some(CurrencyValue::new(0.0, Currency::Usd));
        }
        CountryResult::process_loan_returns(&mut data, 1, d(2027, 5, 28));

        let returned = holds_player(&data, 100).expect("home with the parent");
        assert!(returned.contract_loan.is_none());
        assert!(
            returned.decision_history.items.iter().any(|row| {
                row.movement == "dec_loan_option_turned_down"
                    && row.decision == "board_concern_exceeds_transfer_budget"
            }),
            "the lapse is the board's, on his record"
        );
        assert!(holds_player(&data, 200).is_none());
    }

    /// He spent the season as the borrower's rotation man: the board hears
    /// a rotation purchase, and that is the purpose his plan carries once
    /// he signs.
    #[test]
    fn the_hearing_and_the_plan_agree() {
        let date = d(2027, 5, 28);
        let mut data = option_world(1, 7_000);
        if let Some(contract) = loanee_mut(&mut data).contract.as_mut() {
            contract.squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
        }
        let heard = {
            let borrower = data
                .country(1)
                .unwrap()
                .clubs
                .iter()
                .find(|c| c.id == 200)
                .unwrap();
            let loanee = borrower.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == 55)
                .unwrap();
            borrower
                .hear_option_to_buy(loanee, 2_000_000.0, date)
                .0
                .mandate
                .purpose
        };
        assert_eq!(heard, MandatePurpose::Rotation);

        CountryResult::process_loan_returns(&mut data, 1, date);

        let bought = holds_player(&data, 200).expect("he signs and stays");
        let plan = bought.plan.as_ref().expect("the borrower writes his plan");
        assert_eq!(plan.mandate.purpose, heard);
        assert_eq!(plan.started, date);
    }

    /// An obligation is not asked again at the end: he agreed to it with
    /// the loan, so the parent did not sell him against his will — even
    /// at a club under his floor.
    #[test]
    fn an_executed_obligation_is_his_own_move() {
        let mut data = option_world(2, 3_500);
        let loanee = data
            .country_mut(1)
            .unwrap()
            .clubs
            .iter_mut()
            .find(|c| c.id == 200)
            .and_then(|c| {
                c.teams.teams[0]
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == 55)
            })
            .unwrap();
        if let Some(loan) = loanee.contract_loan.as_mut() {
            loan.loan_future_fee_obligation = true;
        }
        CountryResult::process_loan_returns(&mut data, 1, d(2027, 5, 28));

        let bought = holds_player(&data, 200).expect("the obligation binds");
        assert!(bought.contract_loan.is_none());
        assert!(
            !bought
                .mind
                .memory()
                .episodes
                .iter()
                .any(|e| e.kind == EpisodeKind::SoldAgainstWill),
            "no grudge against the club that owned him"
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
                split_season: false,
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

    #[test]
    fn a_reserve_side_never_goes_up_into_its_first_teams_tier() {
        // Club 10's B team (100) tops tier 2 while its first team plays in
        // tier 1, so the runner-up (21) takes the one promotion place.
        let b_team = TeamBuilder::new()
            .id(100)
            .league_id(Some(2))
            .club_id(10)
            .name("Team100".to_string())
            .slug("team100".to_string())
            .team_type(TeamType::B)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap();
        let mut clubs = vec![make_club(10, vec![make_simple_team(10, 10, 1), b_team])];
        clubs.extend((11u32..=13).map(|id| make_club(id, vec![make_simple_team(id, id, 1)])));
        clubs.extend((21u32..=23).map(|id| make_club(id, vec![make_simple_team(id, id, 2)])));
        let tier1 = make_league_with_settings(
            1,
            1,
            0,
            1,
            vec![(10, 30, 70), (11, 30, 60), (12, 30, 30), (13, 30, 20)],
        );
        let tier2 = make_league_with_settings(
            2,
            2,
            1,
            0,
            vec![(100, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
        );
        let mut country = build_country(clubs, vec![tier1, tier2]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        let league_of = |team_id: u32| {
            country
                .clubs
                .iter()
                .flat_map(|c| c.teams.iter())
                .find(|t| t.id == team_id)
                .and_then(|t| t.league_id)
        };
        assert_eq!(
            league_of(100),
            Some(2),
            "the B team stays below its first team"
        );
        assert_eq!(league_of(21), Some(1), "the next eligible side goes up");
        assert_eq!(league_of(13), Some(2));
        assert_eq!(league_of(10), Some(1));
    }

    fn make_sub_team(
        id: u32,
        club_id: u32,
        league_id: u32,
        team_type: TeamType,
        players: Vec<Player>,
    ) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team{}", id))
            .team_type(team_type)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn league_of_team(country: &Country, team_id: u32) -> Option<u32> {
        country
            .clubs
            .iter()
            .flat_map(|c| c.teams.iter())
            .find(|t| t.id == team_id)
            .and_then(|t| t.league_id)
    }

    fn team_by_id(country: &Country, team_id: u32) -> &crate::Team {
        country
            .clubs
            .iter()
            .flat_map(|c| c.teams.iter())
            .find(|t| t.id == team_id)
            .unwrap()
    }

    fn celebrations(country: &Country, team_id: u32) -> usize {
        team_by_id(country, team_id)
            .players
            .players
            .iter()
            .map(|p| happiness_event_count(p, &HappinessEventType::PromotionCelebration))
            .sum()
    }

    /// Castilla tops the third tier while Real Madrid plays in the first:
    /// it goes up into the second, and stays there.
    #[test]
    fn a_promoted_b_team_stays_in_the_division_it_went_up_into() {
        let mut clubs = vec![make_club(
            10,
            vec![
                make_simple_team(10, 10, 1),
                make_sub_team(100, 10, 3, TeamType::B, Vec::new()),
            ],
        )];
        clubs.extend((11u32..=12).map(|id| make_club(id, vec![make_simple_team(id, id, 1)])));
        clubs.extend((21u32..=23).map(|id| make_club(id, vec![make_simple_team(id, id, 2)])));
        clubs.extend((31u32..=33).map(|id| make_club(id, vec![make_simple_team(id, id, 3)])));
        let tier1 =
            make_league_with_settings(1, 1, 0, 0, vec![(10, 30, 70), (11, 30, 60), (12, 30, 30)]);
        let tier2 =
            make_league_with_settings(2, 2, 0, 1, vec![(21, 30, 70), (22, 30, 60), (23, 30, 30)]);
        let tier3 = make_league_with_settings(
            3,
            3,
            1,
            0,
            vec![(100, 30, 80), (31, 30, 70), (32, 30, 40), (33, 30, 25)],
        );
        let mut country = build_country(clubs, vec![tier1, tier2, tier3]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        assert_eq!(league_of_team(&country, 100), Some(2));
        assert_eq!(league_of_team(&country, 23), Some(3));
        let size = |league_id: u32| {
            country
                .clubs
                .iter()
                .flat_map(|c| c.teams.iter())
                .filter(|t| t.league_id == Some(league_id))
                .count()
        };
        assert_eq!([1u32, 2, 3].map(size), [3, 3, 4]);
    }

    /// The first team drops a division; its B team, two divisions below,
    /// is not dragged after it — only the youth side follows.
    #[test]
    fn a_first_teams_relegation_leaves_its_b_team_where_it_plays() {
        let mut clubs = vec![make_club(
            10,
            vec![
                make_simple_team(10, 10, 1),
                make_sub_team(100, 10, 3, TeamType::B, Vec::new()),
                make_sub_team(101, 10, 1 + 110_000, TeamType::U19, Vec::new()),
            ],
        )];
        clubs.extend((11u32..=12).map(|id| make_club(id, vec![make_simple_team(id, id, 1)])));
        clubs.extend((21u32..=23).map(|id| make_club(id, vec![make_simple_team(id, id, 2)])));
        clubs.extend((31u32..=33).map(|id| make_club(id, vec![make_simple_team(id, id, 3)])));
        let tier1 =
            make_league_with_settings(1, 1, 0, 1, vec![(11, 30, 70), (12, 30, 60), (10, 30, 30)]);
        let tier2 =
            make_league_with_settings(2, 2, 1, 0, vec![(21, 30, 70), (22, 30, 60), (23, 30, 30)]);
        let tier3 = make_league_with_settings(
            3,
            3,
            0,
            0,
            vec![(100, 30, 80), (31, 30, 70), (32, 30, 40), (33, 30, 25)],
        );
        let mut country = build_country(clubs, vec![tier1, tier2, tier3]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        assert_eq!(league_of_team(&country, 10), Some(2));
        assert_eq!(
            league_of_team(&country, 100),
            Some(3),
            "the B team plays on in tier 3"
        );
        assert_eq!(league_of_team(&country, 101), Some(2 + 110_000));
    }

    /// A B team champion below its first team stays down; the two sides
    /// that do go up are the two that celebrate it.
    #[test]
    fn promotion_is_celebrated_by_the_sides_that_go_up() {
        let mut clubs = vec![make_club(
            10,
            vec![
                make_simple_team(10, 10, 1),
                make_sub_team(100, 10, 2, TeamType::B, vec![make_player(1001)]),
            ],
        )];
        clubs.extend((11u32..=13).map(|id| make_club(id, vec![make_simple_team(id, id, 1)])));
        clubs.extend(
            (21u32..=23)
                .map(|id| make_club(id, vec![make_team(id, id, 2, vec![make_player(id * 100)])])),
        );
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
            vec![(100, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
        );
        let mut country = build_country(clubs, vec![tier1, tier2]);

        CountryResult::process_season_awards(&mut country, &[], d(2032, 5, 31));
        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        let b_player = &team_by_id(&country, 100).players.players[0];
        assert_eq!(
            happiness_event_count(b_player, &HappinessEventType::TrophyWon),
            1
        );
        assert_eq!(celebrations(&country, 100), 0);
        for id in [21u32, 22] {
            assert_eq!(league_of_team(&country, id), Some(1));
            assert_eq!(celebrations(&country, id), 1, "team {id} celebrates");
            assert!(
                team_by_id(&country, id).reputation.home > 100,
                "team {id} carries the promotion achievement"
            );
        }
        assert_eq!(celebrations(&country, 23), 0);
        assert_eq!(team_by_id(&country, 23).reputation.home, 100);
    }

    /// Three places over two groups that each declare two: the second
    /// group's runner-up stays down and has nothing to celebrate.
    #[test]
    fn a_group_allotted_fewer_places_celebrates_only_its_movers() {
        let mut clubs: Vec<Club> = (10u32..=14)
            .map(|id| make_club(id, vec![make_simple_team(id, id, 1)]))
            .collect();
        for (base, league_id) in [(20u32, 2u32), (30, 3)] {
            clubs.extend((base..base + 4).map(|id| {
                make_club(
                    id,
                    vec![make_team(id, id, league_id, vec![make_player(id * 100)])],
                )
            }));
        }
        let upper = make_league_with_settings(
            1,
            2,
            0,
            3,
            vec![
                (10, 30, 70),
                (11, 30, 60),
                (12, 30, 50),
                (13, 30, 40),
                (14, 30, 30),
            ],
        );
        let mut groups = vec![
            make_league_with_settings(
                2,
                3,
                2,
                0,
                vec![(20, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
            ),
            make_league_with_settings(
                3,
                3,
                2,
                0,
                vec![(30, 30, 80), (31, 30, 70), (32, 30, 40), (33, 30, 25)],
            ),
        ];
        for (group, name) in groups.iter_mut().zip(["Grupo 1", "Grupo 2"]) {
            group.settings.league_group = Some(crate::league::LeagueGroup {
                name: name.to_string(),
                competition: "Primera Federación".to_string(),
                total_groups: 2,
                level: 0,
                playoff: None,
            });
        }
        let mut leagues = vec![upper];
        leagues.extend(groups);
        let mut country = build_country(clubs, leagues);

        CountryResult::process_season_awards(&mut country, &[], d(2032, 5, 31));
        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        for id in [20u32, 21, 30] {
            assert_eq!(league_of_team(&country, id), Some(1), "team {id} goes up");
            assert_eq!(celebrations(&country, id), 1, "team {id} celebrates");
        }
        assert_eq!(league_of_team(&country, 31), Some(3));
        assert_eq!(
            celebrations(&country, 31),
            0,
            "the runner-up who stays down"
        );
    }

    /// A B team falling out of the third tier is not the club falling out
    /// of anything: the first team's revenue is untouched.
    #[test]
    fn a_relegated_b_team_grants_its_club_no_parachute() {
        let mut clubs = vec![make_club(
            10,
            vec![
                make_simple_team(10, 10, 1),
                make_sub_team(100, 10, 3, TeamType::B, Vec::new()),
            ],
        )];
        clubs.extend((31u32..=33).map(|id| make_club(id, vec![make_simple_team(id, id, 3)])));
        clubs.extend((41u32..=43).map(|id| make_club(id, vec![make_simple_team(id, id, 4)])));
        let tier1 = make_league_with_settings(1, 1, 0, 0, vec![(10, 30, 70)]);
        let tier3 = make_league_with_settings(
            3,
            3,
            0,
            1,
            vec![(31, 30, 80), (32, 30, 70), (33, 30, 40), (100, 30, 25)],
        );
        let tier4 =
            make_league_with_settings(4, 4, 1, 0, vec![(41, 30, 70), (42, 30, 60), (43, 30, 30)]);
        let mut country = build_country(clubs, vec![tier1, tier3, tier4]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        assert_eq!(league_of_team(&country, 100), Some(4));
        let club = country.clubs.iter().find(|c| c.id == 10).unwrap();
        assert_eq!(club.finance.parachute, None);
    }

    /// A first team living on a parachute keeps it when its B team goes
    /// up two divisions below.
    #[test]
    fn a_promoted_b_team_leaves_its_clubs_parachute_running() {
        let mut parent = make_club(
            10,
            vec![
                make_simple_team(10, 10, 2),
                make_sub_team(100, 10, 4, TeamType::B, Vec::new()),
            ],
        );
        parent.finance.parachute = Some(ParachuteEntitlement {
            from_tier: 1,
            seasons_elapsed: 0,
        });
        let mut clubs = vec![parent];
        clubs.extend((31u32..=33).map(|id| make_club(id, vec![make_simple_team(id, id, 3)])));
        clubs.extend((41u32..=43).map(|id| make_club(id, vec![make_simple_team(id, id, 4)])));
        let tier2 = make_league_with_settings(2, 2, 0, 0, vec![(10, 30, 70)]);
        let tier3 =
            make_league_with_settings(3, 3, 0, 1, vec![(31, 30, 80), (32, 30, 70), (33, 30, 40)]);
        let tier4 = make_league_with_settings(
            4,
            4,
            1,
            0,
            vec![(100, 30, 80), (41, 30, 70), (42, 30, 60), (43, 30, 30)],
        );
        let mut country = build_country(clubs, vec![tier2, tier3, tier4]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        assert_eq!(league_of_team(&country, 100), Some(3));
        let club = country.clubs.iter().find(|c| c.id == 10).unwrap();
        assert_eq!(
            club.finance
                .parachute
                .map(|p| (p.from_tier, p.seasons_elapsed)),
            Some((1, 0))
        );
    }

    #[test]
    fn single_division_swaps_with_every_group_below() {
        // League 1 (teams 10..=15) drops its bottom four into two groups
        // (leagues 2 and 3) that promote two each.
        let mut clubs: Vec<Club> = (10u32..=15)
            .map(|id| make_club(id, vec![make_simple_team(id, id, 1)]))
            .collect();
        clubs.extend((20u32..=23).map(|id| make_club(id, vec![make_simple_team(id, id, 2)])));
        clubs.extend((30u32..=33).map(|id| make_club(id, vec![make_simple_team(id, id, 3)])));
        let upper = make_league_with_settings(
            1,
            2,
            0,
            4,
            vec![
                (10, 30, 70),
                (11, 30, 60),
                (12, 30, 50),
                (13, 30, 40),
                (14, 30, 30),
                (15, 30, 20),
            ],
        );
        let mut groups = vec![
            make_league_with_settings(
                2,
                3,
                2,
                0,
                vec![(20, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
            ),
            make_league_with_settings(
                3,
                3,
                2,
                0,
                vec![(30, 30, 80), (31, 30, 70), (32, 30, 40), (33, 30, 25)],
            ),
        ];
        for (group, name) in groups.iter_mut().zip(["Grupo 1", "Grupo 2"]) {
            group.settings.league_group = Some(crate::league::LeagueGroup {
                name: name.to_string(),
                competition: "Primera Federación".to_string(),
                total_groups: 2,
                level: 0,
                playoff: None,
            });
        }
        let mut leagues = vec![upper];
        leagues.extend(groups);
        let mut country = build_country(clubs, leagues);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        let league_of = |club_id: u32| {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.teams.iter().next())
                .and_then(|t| t.league_id)
        };
        // The top two of BOTH groups go up.
        for id in [20, 21, 30, 31] {
            assert_eq!(league_of(id), Some(1), "team {id} promoted");
        }
        // The bottom four are dealt out worst-first, one group at a time.
        assert_eq!(league_of(15), Some(2));
        assert_eq!(league_of(14), Some(3));
        assert_eq!(league_of(13), Some(2));
        assert_eq!(league_of(12), Some(3));
        assert_eq!(league_of(11), Some(1));
        // Every division keeps its size.
        let size = |league_id: u32| {
            [10u32, 11, 12, 13, 14, 15, 20, 21, 22, 23, 30, 31, 32, 33]
                .into_iter()
                .filter(|&id| league_of(id) == Some(league_id))
                .count()
        };
        assert_eq!((size(1), size(2), size(3)), (6, 4, 4));
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

    /// Every player who comes home brings his record with him. The
    /// spell is read off the borrower-season statistics, which
    /// `on_loan_return` freezes and resets moments later — if the report
    /// were filed any later than it is, the numbers would already be
    /// gone and the feed would say "returned from loan" and nothing else.
    #[test]
    fn a_returning_loanee_brings_the_record_of_his_spell_home() {
        let mut data = recall_world(None);
        // A season of it: in the team every week, and playing well.
        {
            let country = data.continents[0].countries.get_mut(0).unwrap();
            let borrower = country.clubs.iter_mut().find(|c| c.id == 200).unwrap();
            let loanee = borrower.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == 55)
                .unwrap();
            loanee.statistics.played = 28;
            loanee.statistics.played_subs = 3;
            loanee.statistics.goals = 9;
            loanee.statistics.assists = 4;
            // Raw 7.45 over a full season's weight; the sample-size
            // regression the record reads through lands it near 7.2.
            loanee.statistics.rating_points = 7.45 * 31.0;
            loanee.statistics.rating_weight = 31.0;
            let loan = loanee.contract_loan.as_mut().unwrap();
            // A season of it, not the three months the recall fixture
            // defaults to: 28 starts inside 92 days is not a loan, it is
            // a fixture list nobody plays.
            loan.started = Some(d(2025, 8, 1));
            // Expired on the day the return sweep runs.
            loan.expiration = d(2026, 6, 1);
        }

        CountryResult::process_loan_returns(&mut data, 1, d(2026, 6, 1));

        let country = data.country(1).unwrap();
        let parent = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let returnee = parent.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 55)
            .expect("the loanee must be home");

        let review = returnee
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LoanSpellReviewed)
            .expect("a finished spell files a report");
        let spell = review
            .context
            .as_ref()
            .and_then(|c| c.loan_context.as_ref())
            .and_then(|l| l.spell)
            .expect("the report carries the record it is a report of");

        assert_eq!(spell.appearances, 31);
        assert_eq!(spell.starts, 28);
        assert_eq!(spell.goals, 9);
        assert_eq!(spell.assists, 4);
        assert_eq!(spell.verdict, LoanSpellVerdict::Standout);
        assert_eq!(
            review.magnitude, 0.0,
            "the report states the record; the return moods carry the feeling"
        );
    }

    /// A spell worth something buys him a claim on a shirt, and the
    /// claim has to survive the weeks before he is next picked.
    ///
    /// The drive home resets the rolling start share to its neutral 0.5
    /// and the count behind it to zero. Read as football, that placeholder
    /// says he is playing, which answers the very claim the spell earned
    /// — on the first weekly think, before the parent has named him in a
    /// squad.
    #[test]
    fn a_standout_returnee_keeps_his_claim_until_somebody_plays_him() {
        let mut data = recall_world(None);
        {
            let country = data.continents[0].countries.get_mut(0).unwrap();
            let borrower = country.clubs.iter_mut().find(|c| c.id == 200).unwrap();
            let loanee = borrower.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == 55)
                .unwrap();
            loanee.statistics.played = 28;
            loanee.statistics.played_subs = 3;
            loanee.statistics.rating_points = 7.45 * 31.0;
            loanee.statistics.rating_weight = 31.0;
            let loan = loanee.contract_loan.as_mut().unwrap();
            loan.started = Some(d(2025, 8, 1));
            loan.expiration = d(2026, 6, 1);
        }

        CountryResult::process_loan_returns(&mut data, 1, d(2026, 6, 1));

        let arc_after = |data: &SimulatorData| {
            data.country(1)
                .unwrap()
                .clubs
                .iter()
                .find(|c| c.id == 100)
                .unwrap()
                .teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == 55)
                .expect("he is home")
                .mind
                .career
                .plan
                .map(|plan| plan.arc)
        };
        assert_eq!(
            arc_after(&data),
            Some(CareerArc::ClaimMyPlace),
            "a standout spell is a claim on the shirt"
        );

        let think = |data: &mut SimulatorData, date: NaiveDate| {
            for club in data.continents[0].countries[0].clubs.iter_mut() {
                for team in club.teams.teams.iter_mut() {
                    let Some(player) = team.players.players.iter_mut().find(|p| p.id == 55) else {
                        continue;
                    };
                    let ctx = player.mind_context(date, Some(club.id));
                    let situation = player.mind_situation(date, 1, "");
                    player.mind.tick_with(&ctx, &situation);
                }
            }
        };

        for week in 1..=4 {
            think(&mut data, d(2026, 6, 1) + chrono::Duration::days(7 * week));
        }
        assert_eq!(
            arc_after(&data),
            Some(CareerArc::ClaimMyPlace),
            "nobody has picked him yet, so nothing has answered him"
        );

        // Two months of it, and he is in the side. That answers it.
        {
            let parent = data.continents[0].countries[0]
                .clubs
                .iter_mut()
                .find(|c| c.id == 100)
                .unwrap();
            let returnee = parent.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == 55)
                .unwrap();
            returnee.happiness.starter_ratio = 0.6;
            returnee.happiness.appearances_tracked = 8;
        }
        think(&mut data, d(2026, 8, 1));
        assert_ne!(
            arc_after(&data),
            Some(CareerArc::ClaimMyPlace),
            "the club gave him the look his record earned"
        );
    }

    /// The same record, for the one position it did not describe.
    ///
    /// Goals and assists are the whole of what a returning player used
    /// to bring home, and a goalkeeper brings home none of either — so
    /// every keeper's loan, however well he kept, was reported as a
    /// season of nought goals. The columns his job is actually kept in
    /// are frozen with the rest of the borrowing season, so they have to
    /// be read here or not at all.
    #[test]
    fn a_returning_goalkeeper_brings_home_the_record_he_was_judged_on() {
        let mut data = recall_world(None);
        {
            let country = data.continents[0].countries.get_mut(0).unwrap();
            let borrower = country.clubs.iter_mut().find(|c| c.id == 200).unwrap();
            let loanee = borrower.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == 55)
                .unwrap();
            loanee.positions = PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 20,
                }],
            };
            loanee.statistics.played = 30;
            loanee.statistics.played_subs = 0;
            loanee.statistics.goals = 0;
            loanee.statistics.assists = 0;
            loanee.statistics.clean_sheets = 11;
            loanee.statistics.conceded = 27;
            loanee.statistics.rating_points = 7.20 * 30.0;
            loanee.statistics.rating_weight = 30.0;
            let loan = loanee.contract_loan.as_mut().unwrap();
            loan.started = Some(d(2025, 8, 1));
            loan.expiration = d(2026, 6, 1);
        }

        CountryResult::process_loan_returns(&mut data, 1, d(2026, 6, 1));

        let country = data.country(1).unwrap();
        let parent = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let returnee = parent.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 55)
            .expect("the keeper must be home");
        let spell = returnee
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LoanSpellReviewed)
            .and_then(|e| e.context.as_ref())
            .and_then(|c| c.loan_context.as_ref())
            .and_then(|l| l.spell)
            .expect("a finished spell files a report");

        assert!(spell.is_goalkeeper, "the report must know he kept goal");
        assert_eq!(spell.clean_sheets, 11);
        assert_eq!(spell.conceded, 27);
        assert_eq!(spell.appearances, 30);
        // The verdict is read off minutes and marks, which mean the
        // same thing in either job — a keeper's spell is not judged more
        // harshly for the empty goals column.
        assert!(
            spell.verdict.is_positive(),
            "a season of 11 shut-outs read as {:?}",
            spell.verdict
        );
    }

    /// A spell nobody would write home about still gets written up. The
    /// verdict is what changes, not whether there is one — a return that
    /// produced no mood event used to leave no trace at all.
    #[test]
    fn a_loanee_who_never_played_still_gets_a_verdict() {
        let mut data = recall_world(None);
        {
            let country = data.continents[0].countries.get_mut(0).unwrap();
            let borrower = country.clubs.iter_mut().find(|c| c.id == 200).unwrap();
            let loanee = borrower.teams.teams[0]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == 55)
                .unwrap();
            // Three months of it (started 1 March), two substitute
            // appearances to show for the trip.
            loanee.statistics.played = 0;
            loanee.statistics.played_subs = 2;
            loanee.contract_loan.as_mut().unwrap().expiration = d(2026, 6, 1);
        }

        CountryResult::process_loan_returns(&mut data, 1, d(2026, 6, 1));

        let country = data.country(1).unwrap();
        let parent = country.clubs.iter().find(|c| c.id == 100).unwrap();
        let returnee = parent.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 55)
            .expect("the loanee must be home");

        let spell = returnee
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LoanSpellReviewed)
            .and_then(|e| e.context.as_ref())
            .and_then(|c| c.loan_context.as_ref())
            .and_then(|l| l.spell)
            .expect("a spell with nothing in it is still a spell");

        assert_eq!(spell.verdict, LoanSpellVerdict::Peripheral);
        assert_eq!(spell.appearances, 2);
        assert!(
            spell.average_rating.is_none(),
            "nobody rated him, which is not the same as rating him 0.00"
        );
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
            split_season: false,
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

        country.on_domestic_cup_winner_awards(d(2027, 5, 20));

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

        country.on_domestic_cup_winner_awards(d(2027, 5, 20));

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
        country.on_domestic_cup_winner_awards(d(2027, 5, 20));
        country.on_domestic_cup_winner_awards(d(2027, 5, 21));

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
            split_season: false,
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

        country.on_domestic_cup_winner_awards(d(2027, 5, 20));

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

        country.on_domestic_cup_winner_awards(d(2027, 5, 20));

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
        country.on_domestic_cup_winner_awards(d(2027, 5, 20));
        country.on_domestic_cup_winner_awards(d(2027, 5, 21));

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

        country.on_domestic_cup_winner_awards(final_date);

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

        country.on_domestic_cup_winner_awards(d(2027, 5, 20));

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
