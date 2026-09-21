use super::SimulatorData;
use crate::transfers::ScoutingRegion;
use crate::transfers::market::knowledge::{PlacementReach, PlacementReachIndex};
use crate::transfers::market::map::{
    CountryTransferProfile, MarketCountryFacts, MarketMap, RegionPrestigeTable,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

impl SimulatorData {
    /// Rebuild the world's transfer geography from the current
    /// `country_info` and publish the region-prestige table it implies.
    ///
    /// Called at construction, and again at each transfer-window boundary so
    /// the wage axis of `import_capacity` follows a world whose economy has
    /// moved. The corridor cards themselves are shipped data and never
    /// change; only the facts around them do.
    pub fn rebuild_market_map(&mut self) {
        let wages = self.median_top_flight_wages();
        let facts: HashMap<u32, MarketCountryFacts> = self
            .country_info
            .values()
            .map(|info| {
                (
                    info.id,
                    MarketCountryFacts {
                        id: info.id,
                        code: info.code.clone(),
                        continent_id: info.continent_id,
                        region: ScoutingRegion::from_country(info.continent_id, &info.code),
                        reputation: info.reputation,
                        top_flight_reputation: info.top_flight_reputation,
                        median_top_flight_wage: wages.get(&info.id).copied().unwrap_or(0),
                    },
                )
            })
            .collect();
        let profiles: HashMap<u32, CountryTransferProfile> = self
            .country_info
            .values()
            .map(|info| (info.id, info.transfer_profile.clone()))
            .collect();

        self.market_map = Arc::new(MarketMap::new(profiles, facts));
        // Region prestige is read from two dozen gates that sit inside
        // per-country borrows and cannot reach here, so the world's answer
        // is published for them. See `RegionPrestigeTable`.
        RegionPrestigeTable::publish(self.market_map.region_prestige_table());
    }

    /// Seed every club's market ledger from the squad it was shipped with.
    ///
    /// A foreign player on the books IS a signing the club made from that
    /// market at some point before the save began — the shipped world is
    /// already the real corridor map, so this is the strictest possible day-0
    /// condition and it needs no authoring at all. Galatasaray starts knowing
    /// Brazil because Galatasaray has eight Brazilians.
    ///
    /// Only the senior squad counts. An academy is a domestic institution,
    /// and counting its intake would make every club look like an importer of
    /// its own country.
    pub fn bootstrap_market_ledgers(&mut self) {
        let today = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let country_id = country.id;
                for club in &mut country.clubs {
                    let mut counts: HashMap<u32, u16> = HashMap::new();
                    for team in club.teams.teams.iter().filter(|t| !t.team_type.is_youth()) {
                        for player in &team.players.players {
                            if player.country_id != country_id {
                                *counts.entry(player.country_id).or_insert(0) += 1;
                            }
                        }
                    }
                    for (source_country, signings) in counts {
                        club.market_ledger
                            .bootstrap(source_country, signings, today);
                    }
                }
            });
    }

    /// Seed every club's placement ledger from the loanees it already has
    /// out in the world.
    ///
    /// The shipped world carries every current loan, so a boy sitting at a
    /// club in another country IS a placement his owner made before the
    /// save began — the same evidence [`Self::bootstrap_market_ledgers`]
    /// reads from the other end of the deal. Collected first and written
    /// after, because the parent is routinely in a different country from
    /// the club the player is standing in.
    pub fn bootstrap_loan_placements(&mut self) {
        let today = self.date.date();
        let mut club_country: HashMap<u32, u32> = HashMap::new();
        let mut counts: HashMap<(u32, u32), u16> = HashMap::new();
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    club_country.insert(club.id, country.id);
                    for team in &club.teams.teams {
                        for player in &team.players.players {
                            if let Some(parent) = player
                                .contract_loan
                                .as_ref()
                                .and_then(|loan| loan.loan_from_club_id)
                            {
                                *counts.entry((parent, country.id)).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }

        for ((parent_club_id, borrower_country_id), placements) in counts {
            if club_country.get(&parent_club_id) == Some(&borrower_country_id) {
                continue;
            }
            if let Some(parent) = self.club_mut(parent_club_id) {
                parent
                    .loan_placements
                    .bootstrap(borrower_country_id, placements, today);
            }
        }
    }

    /// Stage every club's placement map for the parallel pass.
    ///
    /// A borrowing country's borrow cannot reach the club that owns the
    /// player, so the lender's half of the geography travels beside the
    /// world pool exactly as the player summaries do.
    pub fn collect_placement_reach(&self) -> PlacementReachIndex {
        PlacementReachIndex::from_clubs(
            self.continents
                .par_iter()
                .flat_map(|continent| continent.countries.par_iter())
                .flat_map_iter(|country| {
                    country
                        .clubs
                        .iter()
                        .map(|club| (club.id, PlacementReach::of(club, country.id)))
                })
                .collect::<Vec<(u32, PlacementReach)>>(),
        )
    }

    /// Median annual salary in each country's strongest division. The money
    /// axis of `import_capacity`: what a league PAYS is what decides whether
    /// it can sign a name from outside its own corridors, and it is a fact
    /// about the live world rather than anything the data files could state.
    fn median_top_flight_wages(&self) -> HashMap<u32, u32> {
        self.continents
            .iter()
            .flat_map(|continent| &continent.countries)
            .map(|country| {
                let top_league = country.leagues.leagues.iter().max_by_key(|l| l.reputation);
                let mut salaries: Vec<u32> = country
                    .clubs
                    .iter()
                    .flat_map(|club| club.teams.teams.iter())
                    .filter(|team| match (team.league_id, top_league) {
                        (Some(id), Some(league)) => id == league.id,
                        _ => false,
                    })
                    .flat_map(|team| team.players.players.iter())
                    .filter_map(|player| player.contract.as_ref().map(|c| c.salary))
                    .filter(|salary| *salary > 0)
                    .collect();
                salaries.sort_unstable();
                let median = salaries.get(salaries.len() / 2).copied().unwrap_or(0);
                (country.id, median)
            })
            .collect()
    }
}
