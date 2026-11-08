//! The kinds of player the census follows, picked by facts about each man and
//! his club, shortly after clubs have first said who plays.

use crate::reading::{ClubRanks, Snap};
use core::club::player::statistics::StuckCareerScan;
use core::{Person, SimulatorData, TeamType};
use std::collections::{BTreeMap, HashMap};

pub const COHORT_SIZE: usize = 40;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Cohort {
    EliteStar,
    EliteProspect,
    SmallClubWonderkid,
    AmbitiousClimber,
    SettledMidRegular,
    BigClubFringe,
    ClubServant,
    UnsettledAbroad,
    TopClubVeteran,
    LowLeaguePro,
    BackupKeeper,
    FreeAgentAtStart,
}

impl Cohort {
    pub const ALL: [Cohort; 12] = [
        Cohort::EliteStar,
        Cohort::EliteProspect,
        Cohort::SmallClubWonderkid,
        Cohort::AmbitiousClimber,
        Cohort::SettledMidRegular,
        Cohort::BigClubFringe,
        Cohort::ClubServant,
        Cohort::UnsettledAbroad,
        Cohort::TopClubVeteran,
        Cohort::LowLeaguePro,
        Cohort::BackupKeeper,
        Cohort::FreeAgentAtStart,
    ];

    /// Each man in the first cohort that wants him, strongest first, with
    /// ties broken by id: no draw, so two runs follow the same men.
    pub fn select(data: &SimulatorData, ranks: &ClubRanks) -> HashMap<u32, Cohort> {
        let date = data.date.date();
        let mut pools: BTreeMap<Cohort, Vec<(i32, u32)>> = BTreeMap::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let rank = ranks.of(club.id);
                    for team in &club.teams.teams {
                        let is_main = team.team_type == TeamType::Main;
                        for p in &team.players.players {
                            if p.is_on_loan() {
                                continue;
                            }
                            let age = p.age(date);
                            let ca = p.player_attributes.current_ability as i32;
                            let pa = p.player_attributes.potential_ability as i32;
                            let status = p
                                .contract
                                .as_ref()
                                .map_or("-", |c| Snap::status_label(&c.squad_status));
                            let regular = is_main && matches!(status, "Key" | "Regular");
                            let keeper = p.position().is_goalkeeper();
                            let tenure = StuckCareerScan::club_tenure_days(p, date).unwrap_or(0);
                            let abroad = p.country_id != 0 && p.country_id != country.id;
                            let mut add = |cohort: Cohort, score: i32| {
                                pools.entry(cohort).or_default().push((score, p.id));
                            };
                            if rank < 50 && regular && (24..=28).contains(&age) && !keeper {
                                add(Cohort::EliteStar, ca);
                            }
                            if rank < 50 && age <= 20 && pa >= 130 {
                                add(Cohort::EliteProspect, pa);
                            }
                            if rank >= 250 && age <= 21 && pa >= 130 {
                                add(Cohort::SmallClubWonderkid, pa);
                            }
                            if (250..1000).contains(&rank)
                                && regular
                                && (21..=25).contains(&age)
                                && p.attributes.ambition >= 15.0
                            {
                                add(Cohort::AmbitiousClimber, ca);
                            }
                            if (250..1000).contains(&rank)
                                && regular
                                && (25..=29).contains(&age)
                                && p.attributes.ambition <= 13.0
                            {
                                add(Cohort::SettledMidRegular, ca);
                            }
                            if rank < 250 && is_main && !regular && (24..=29).contains(&age) && !keeper
                            {
                                add(Cohort::BigClubFringe, ca);
                            }
                            if p.attributes.loyalty >= 15.0
                                && tenure >= 5 * 365
                                && (26..=31).contains(&age)
                                && is_main
                            {
                                add(Cohort::ClubServant, ca);
                            }
                            if abroad
                                && p.attributes.adaptability <= 9.0
                                && (20..=28).contains(&age)
                                && is_main
                            {
                                add(Cohort::UnsettledAbroad, ca);
                            }
                            if rank < 250 && regular && (31..=33).contains(&age) {
                                add(Cohort::TopClubVeteran, ca);
                            }
                            if rank >= 1000 && regular && (23..=28).contains(&age) {
                                add(Cohort::LowLeaguePro, ca);
                            }
                            if rank < 250 && keeper && !regular && (24..=32).contains(&age) && is_main
                            {
                                add(Cohort::BackupKeeper, ca);
                            }
                        }
                    }
                }
            }
        }
        for p in &data.free_agents {
            if (20..=30).contains(&p.age(date)) {
                pools
                    .entry(Cohort::FreeAgentAtStart)
                    .or_default()
                    .push((p.player_attributes.current_ability as i32, p.id));
            }
        }

        let mut chosen: HashMap<u32, Cohort> = HashMap::new();
        for cohort in Cohort::ALL {
            let mut pool = pools.remove(&cohort).unwrap_or_default();
            pool.sort_by_key(|(score, id)| (std::cmp::Reverse(*score), *id));
            let eligible = pool.len();
            let mut taken = 0;
            for (_, id) in pool {
                if taken == COHORT_SIZE {
                    break;
                }
                if let std::collections::hash_map::Entry::Vacant(slot) = chosen.entry(id) {
                    slot.insert(cohort);
                    taken += 1;
                }
            }
            println!("cohort {cohort:?}: {taken} of {eligible} eligible");
        }
        chosen
    }
}
