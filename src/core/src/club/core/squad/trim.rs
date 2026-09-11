//! Season-start positional trim: the depth a club is actually allowed to
//! carry, and what happens to the bodies above it.

use chrono::NaiveDate;
use log::debug;

use crate::club::player::calculators::{
    AutomaticReleaseEligibility, FreeAgentReleaseReason, ReleaseEligibilityContext,
};
use crate::club::staff::perception::AbilityEstimator;
use crate::club::team::squad::SquadAssetContext;
use crate::transfers::pipeline::TransferTrace;
use crate::{Club, PlayerFieldPositionGroup, PlayerStatusType};

use super::depth::SquadSize;

impl Club {
    /// Trim excess players at over-represented positions. Caps scale
    /// per-team so a club with main + reserve + U18 is allowed the depth a
    /// real club carries; previously a flat cross-club cap deleted
    /// legitimate squad members every season start.
    ///
    /// Surplus players are no longer all released outright: only those
    /// passing `AutomaticReleaseEligibility` (clearly below team level,
    /// negligible market value, affordable severance) become free agents
    /// on the roster (contract cleared, Frt set) for the country-level
    /// free-agent pipeline. Everyone else is flagged for sale instead —
    /// the country listing pass picks `is_transfer_listed` up and creates
    /// the market listing with a real asking price.
    pub(in crate::club::core) fn trim_positional_surplus(&mut self, date: NaiveDate) {
        // Per-team caps. Total ~30 per team covers a realistic first-team
        // squad + cover depth; multiplied across main + reserve + U18
        // (typically 3 teams) this is ~90, in line with real clubs.
        let team_count = self.teams.teams.len().max(1);
        let limits: [(PlayerFieldPositionGroup, usize); 4] = [
            (PlayerFieldPositionGroup::Goalkeeper, 4 * team_count),
            (PlayerFieldPositionGroup::Defender, 10 * team_count),
            (PlayerFieldPositionGroup::Midfielder, 10 * team_count),
            (PlayerFieldPositionGroup::Forward, 7 * team_count),
        ];

        // Club-level context for the release-eligibility gate, computed
        // once. League reputation isn't reachable from club scope — use
        // the main team's world reputation as the pricing proxy, the same
        // trade-off the renewal pass documents in `simulate`.
        let (squad_avg_ability, club_reputation, league_reputation_proxy) = match self.teams.main()
        {
            Some(main) => (
                main.players.current_ability_avg(),
                main.reputation.market_value_score(),
                main.reputation.world,
            ),
            None => return,
        };
        let annual_wage_bill: u32 = self.teams.iter().map(|t| t.get_annual_salary()).sum();

        // Central squad-asset classification, built once against the pre-trim
        // squad. Threaded into the release gate so a player who is core /
        // first-team / recognised / merely-unevaluated is never walked for
        // free — only genuine surplus is. Owns its data, so the mutable trim
        // loop below holds no borrow on it.
        let asset_ctx = SquadAssetContext::build(self, date);

        for (group, max_count) in &limits {
            // Active players only: if a player has already been released
            // (no contract, Frt) they shouldn't count against the cap, or
            // we'd re-release them every season until someone signs.
            // Rank by the coach-observable level (visible skill + results +
            // training), not the hidden CA digit: who is "surplus to squad
            // requirements" is a judgement on how a player performs and
            // applies himself, so the worst-first order the trim walks is his
            // assessed standing. Each candidate still passes the observable
            // `classify` + release gate below before anything happens to him.
            let mut players_at_pos: Vec<(usize, u32, u8)> = Vec::new();
            for (ti, team) in self.teams.iter().enumerate() {
                for p in team.players.iter() {
                    if p.contract.is_none() || p.is_on_loan() {
                        continue;
                    }
                    if p.position().position_group() == *group {
                        players_at_pos.push((ti, p.id, AbilityEstimator::observable_level(p)));
                    }
                }
            }

            if players_at_pos.len() <= *max_count {
                continue;
            }

            // Sort by observable level ascending — move the worst out first
            players_at_pos.sort_by_key(|&(_, _, level)| level);

            // Walk worst-first until enough players are actually on the
            // market to close the overshoot. A protected candidate (youth
            // squad at minimum size, signing still in its evaluation
            // window) must NOT consume a trim slot — with `.take(to_trim)`
            // every protected skip silently shrank the enforcement, so a
            // club whose surplus sat in protected pockets never trimmed
            // at all. Already-listed players DO count: they are actioned
            // surplus awaiting a buyer, and counting them keeps the walk
            // from marching past the genuine overshoot into useful players.
            let to_trim = players_at_pos.len() - max_count;
            let mut actioned = 0usize;
            for &(team_idx, player_id, _) in players_at_pos.iter() {
                if actioned >= to_trim {
                    break;
                }
                if self.teams.teams[team_idx].team_type.max_age().is_some()
                    && self.teams.teams[team_idx].players.players.len() <= SquadSize::MIN_YOUTH
                {
                    continue;
                }
                let team_name = self.teams.teams[team_idx].name.clone();
                let squad_tier = self.teams.teams[team_idx].team_type;
                if let Some(player) = self.teams.teams[team_idx]
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == player_id)
                {
                    // Already on the market from an earlier pass — the
                    // transfer pipeline owns this player now. Counts as
                    // actioned surplus (see the walk comment above).
                    let already_listed = player.statuses.has(PlayerStatusType::Lst)
                        || player
                            .contract
                            .as_ref()
                            .map(|c| c.is_transfer_listed)
                            .unwrap_or(false);
                    if already_listed {
                        actioned += 1;
                        continue;
                    }
                    // A signing still inside its evaluation window is not
                    // trimmable surplus — this season-start walk used to
                    // free-release players bought weeks earlier in the same
                    // summer window.
                    if player.signing_protection_active(date) {
                        continue;
                    }
                    let market_value = player.value(date, league_reputation_proxy, club_reputation);
                    let termination_cost = player
                        .contract
                        .as_ref()
                        .map(|c| c.termination_cost(date))
                        .unwrap_or(0);
                    let release_ctx = ReleaseEligibilityContext {
                        date,
                        squad_avg_ability,
                        market_value,
                        annual_wage_bill,
                        asset_class: asset_ctx.classify_in_squad(player, date, squad_tier),
                        early_season: asset_ctx.is_early_season(),
                        squad_tier,
                    };
                    match AutomaticReleaseEligibility::assess(player, &release_ctx) {
                        None => {
                            debug!(
                                "positional surplus release: {} (id={}, {:?}, CA={} vs avg {}, \
                                 value={:.0}, severance={}) from {}",
                                player.full_name,
                                player.id,
                                group,
                                player.player_attributes.current_ability,
                                squad_avg_ability,
                                market_value,
                                termination_cost,
                                team_name
                            );
                            player.contract = None;
                            if !player.statuses.has(PlayerStatusType::Frt) {
                                player.statuses.add(date, PlayerStatusType::Frt);
                            }
                            // Stamp the explicit exit so the free-agent
                            // sweep records a "squad surplus" free release,
                            // distinct from a negotiated mutual termination.
                            player.set_release_reason(FreeAgentReleaseReason::SurplusFreeRelease);
                            player.decision_history.add(
                                date,
                                "dec_free_transfer_listed".to_string(),
                                FreeAgentReleaseReason::SurplusFreeRelease
                                    .history_reason()
                                    .to_string(),
                                "dec_decided_board".to_string(),
                            );
                        }
                        Some(block) => {
                            // Worth keeping under contract: flag for sale. Only
                            // the contract flag is set — the country listing
                            // pass turns `is_transfer_listed` into a market
                            // listing + `Lst` status; stamping `Lst` here would
                            // trip its already-listed guard and strand the
                            // player off-market. This entry is the single
                            // decision-history record for the listing — the
                            // country pass deliberately skips writing one for
                            // pre-flagged players.
                            if let Some(contract) = player.contract.as_mut() {
                                contract.is_transfer_listed = true;
                            }
                            TransferTrace::list(
                                player,
                                date,
                                "trim_positional_surplus",
                                "surplus_squad",
                            );
                            player.decision_history.add(
                                date,
                                "dec_transfer_listed".to_string(),
                                "dec_reason_surplus_squad".to_string(),
                                "dec_decided_board".to_string(),
                            );
                            debug!(
                                "positional surplus listed instead of released: {} (id={}, {:?}, \
                                 CA={} vs avg {}, value={:.0}, severance={}) from {} — blocked: {:?}",
                                player.full_name,
                                player.id,
                                group,
                                player.player_attributes.current_ability,
                                squad_avg_ability,
                                market_value,
                                termination_cost,
                                team_name,
                                block
                            );
                        }
                    }
                    actioned += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::country::result::transfers::ListingPass;
    use crate::country::result::transfers::types::TransferActivitySummary;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPlan, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{Datelike, Duration};

    /// Fixtures for the surplus-trim gate. A single Main team means the
    /// goalkeeper cap is 4, so a fifth keeper is always the trim
    /// candidate; ability/salary/length of that candidate then steers
    /// which side of the release-vs-list decision fires.
    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        }

        fn goalkeeper(id: u32, ability: u8, age: u8, salary: u32, contract_months: u32) -> Player {
            let date = Self::date();
            let expiration = date + Duration::days(contract_months as i64 * 30);
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("Keeper{}", id)))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(salary, expiration)))
                .build()
                .unwrap()
        }

        fn training_schedule() -> TrainingSchedule {
            use chrono::NaiveTime;
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn club(players: Vec<Player>) -> Club {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        fn find_player(club: &Club, id: u32) -> &Player {
            club.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == id)
                .expect("trimmed player must stay on the roster")
        }

        /// Two-squad club for the cross-system tests: Main (id 10, league
        /// 1) plus a Reserve roster (id 20, no league of its own).
        fn club_with_reserve(main_players: Vec<Player>, reserve_players: Vec<Player>) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(main_players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            let reserve = TeamBuilder::new()
                .id(20)
                .league_id(None)
                .club_id(100)
                .name("Reserve".to_string())
                .slug("reserve".to_string())
                .team_type(TeamType::Reserve)
                .players(PlayerCollection::new(reserve_players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main, reserve]),
                ClubFacilities::default(),
            )
        }

        fn country(club: Club) -> Country {
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            );
            Country::builder()
                .id(1)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap()
        }
    }

    #[test]
    fn near_level_surplus_is_listed_not_released() {
        // Fifth keeper at CA 95 vs squad avg ~99 — surplus, but nowhere
        // near the -25 release gap. He must be flagged for sale, not
        // walked for free.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 95, 28, 50_000, 12),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        let contract = trimmed
            .contract
            .as_ref()
            .expect("near-level surplus must keep his contract");
        assert!(
            contract.is_transfer_listed,
            "near-level surplus must be flagged for the transfer market"
        );
        assert!(
            !trimmed.statuses.has(PlayerStatusType::Frt),
            "near-level surplus must not be marked released"
        );
        assert!(
            trimmed
                .decision_history
                .items
                .iter()
                .any(|d| d.movement == "dec_transfer_listed"),
            "listing must be explained in decision history"
        );
        // The four keepers inside the cap are untouched.
        for id in 1..=4 {
            let keeper = Fixture::find_player(&club, id);
            assert!(keeper.contract.is_some());
            assert!(!keeper.contract.as_ref().unwrap().is_transfer_listed);
        }
    }

    #[test]
    fn recent_signing_with_active_plan_is_not_trimmed() {
        // Same near-level fifth keeper as above, but the club bought him
        // three weeks ago — his signing plan is still active, so the
        // season-start trim must leave him alone instead of flagging the
        // player it just paid for.
        let mut new_signing = Fixture::goalkeeper(5, 95, 28, 50_000, 12);
        new_signing.plan = Some(PlayerPlan::from_signing(
            28,
            1_000_000.0,
            Fixture::date() - Duration::days(21),
        ));
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            new_signing,
        ]);

        club.trim_positional_surplus(Fixture::date());

        let protected = Fixture::find_player(&club, 5);
        let contract = protected
            .contract
            .as_ref()
            .expect("protected signing must keep his contract");
        assert!(
            !contract.is_transfer_listed,
            "a signing inside his evaluation window must not be flagged for sale"
        );
        assert!(
            !protected.statuses.has(PlayerStatusType::Frt),
            "a signing inside his evaluation window must not be released"
        );
    }

    #[test]
    fn cheap_fringe_veteran_is_released() {
        // CA 40 at age 36 on a tiny, nearly-expired deal — every gate
        // passes, so the classic mutual release fires.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 40, 36, 15_000, 3),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        assert!(
            trimmed.contract.is_none(),
            "cheap fringe veteran must have the contract cleared"
        );
        assert!(
            trimmed.statuses.has(PlayerStatusType::Frt),
            "release must stamp Frt for the free-agent sweep"
        );
        assert!(
            trimmed
                .decision_history
                .items
                .iter()
                .any(|d| d.movement == "dec_free_transfer_listed"
                    && d.decision == "dec_reason_released_surplus"),
            "release must be explained in decision history as a squad-surplus free release"
        );
        assert_eq!(
            trimmed.release_reason(),
            Some(FreeAgentReleaseReason::SurplusFreeRelease),
            "trimmed surplus player must carry the explicit surplus release reason"
        );
    }

    #[test]
    fn blocked_reserve_surplus_reaches_market_with_single_decision_entry() {
        // End-to-end across the two systems sharing the surplus flow: the
        // season-start trim flags a reserve keeper it must not release
        // (near team level → `is_transfer_listed` only), and the country
        // listing pass then turns the flag into a real market listing
        // carrying the reserve team's id. The trim's decision-history
        // entry must stay the only one — the listing pass deliberately
        // writes none for pre-flagged players.
        let date = Fixture::date();
        // GK cap is 4 per team × 2 teams = 8; nine keepers under contract
        // put the worst one (id 5, CA 95, rostered on the Reserve squad)
        // over the cap.
        let main: Vec<Player> = (1..=4)
            .map(|id| Fixture::goalkeeper(id, 100, 28, 50_000, 12))
            .collect();
        let reserve: Vec<Player> = (5..=9)
            .map(|id| Fixture::goalkeeper(id, 90 + id as u8, 28, 50_000, 12))
            .collect();
        let club = Fixture::club_with_reserve(main, reserve);
        let mut country = Fixture::country(club);

        country.clubs[0].trim_positional_surplus(date);

        {
            let flagged = country.clubs[0].teams.teams[1]
                .players
                .players
                .iter()
                .find(|p| p.id == 5)
                .expect("trimmed reserve keeper must stay on the roster");
            assert!(
                flagged
                    .contract
                    .as_ref()
                    .expect("near-level surplus keeps his contract")
                    .is_transfer_listed,
                "near-level reserve surplus must be flagged for sale"
            );
            assert!(
                !flagged.statuses.has(PlayerStatusType::Frt),
                "blocked release candidate must not be marked released"
            );
        }

        let mut summary = TransferActivitySummary::new();
        ListingPass::list_players_from_pipeline(&mut country, date, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 5)
            .expect("flagged reserve player must reach the country transfer market");
        assert_eq!(
            listing.team_id, 20,
            "the market listing must carry the player's real (reserve) team"
        );

        let player = country.clubs[0].teams.teams[1]
            .players
            .players
            .iter()
            .find(|p| p.id == 5)
            .unwrap();
        assert!(
            player.statuses.has(PlayerStatusType::Lst),
            "the listing pass must stamp Lst once the listing is live"
        );
        assert_eq!(
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.movement == "dec_transfer_listed")
                .count(),
            1,
            "the trim owns the decision entry — the listing pass must not duplicate it"
        );
    }

    #[test]
    fn expensive_full_time_surplus_is_listed_not_released() {
        // Same fringe quality, but two years of a 2M salary left —
        // severance is far beyond the club's tolerance, so the player is
        // listed instead of paid off.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 40, 36, 2_000_000, 24),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        let contract = trimmed
            .contract
            .as_ref()
            .expect("expensive contract must not be torn up automatically");
        assert!(contract.is_transfer_listed);
        assert!(!trimmed.statuses.has(PlayerStatusType::Frt));
    }
}
