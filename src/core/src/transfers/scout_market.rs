//! How a corridor OPENS, and how it closes.
//!
//! Everything else in the geography model reads state: the country cards are
//! shipped, the club ledgers are bootstrapped from shipped squads, and the
//! scouts' markets are seeded at generation. Read alone, that is a lookup
//! table with a decay curve on it — the 2026 world stays the 2026 world, and
//! in 2040 Galatasaray is still signing Brazilians because Galatasaray signed
//! Brazilians in 2026.
//!
//! Real corridors move because PEOPLE move. A club hires a man who knows
//! Venezuela and starts signing Venezuelans within one to three windows; the
//! corridor persists while he and his successes do, and fades within four or
//! five years of his leaving. This module is that channel, and it is the only
//! one by which a corridor absent from the shipped data can appear in a save.
//!
//! Deliberately small and slow: one pass a year, per club, budget-gated, at
//! most one hire. The knowledge census counts what it produces (WI-0), and
//! the acceptance band is 3–8 % of clubs opening a NEW source country per
//! season — drift, not churn.

use chrono::{Datelike, NaiveDate};

use crate::club::staff::model::attributes::{
    StaffAttributes, StaffCoaching, StaffDataAnalysis, StaffGoalkeeperCoaching, StaffKnowledge,
    StaffMedical, StaffMental,
};
use crate::club::staff::model::contract::{StaffClubContract, StaffPosition, StaffStatus};
use crate::country::Country;
use crate::shared::FullName;
use crate::transfers::market_knowledge::ClubMarketKnowledge;
use crate::transfers::market_map::MarketMap;
use crate::utils::{FloatUtils, IntegerUtils};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::transfers::scouting_region::ScoutingRegion;
use crate::{Club, PersonAttributes, Staff, StaffLicenseType};

/// Ids for staff minted after world generation.
///
/// The world generator has its own counter starting at 1; this one starts
/// far above anything a generated world reaches (a full world carries tens
/// of thousands of staff) and is additionally pushed past the real maximum
/// at load by [`seed_staff_id_sequence`]. Same discipline as the player
/// sequence: two counters handing out the same id is a bug that surfaces as
/// one person being two people.
static STAFF_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(500_000_000));

/// Next id for a staff member minted at runtime.
pub fn next_staff_id() -> u32 {
    STAFF_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

/// Push the runtime staff-id counter past every id the loaded world uses.
/// Called once after world generation, exactly like the player sequence.
pub fn seed_staff_id_sequence(min_exclusive: u32) {
    let target = min_exclusive.saturating_add(1);
    let mut current = STAFF_ID_SEQUENCE.load(Ordering::SeqCst);
    while current < target {
        match STAFF_ID_SEQUENCE.compare_exchange(
            current,
            target,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

/// The scouting-department market: who a club wants covering which country,
/// and what it does about the gap.
pub struct ScoutMarketDesk;

impl ScoutMarketDesk {
    /// The one day a year the desk sits. Pre-season, after the world has
    /// rolled over and before the summer window's business.
    const REVIEW_MONTH: u32 = 6;
    const REVIEW_DAY: u32 = 15;

    /// Age past which a scout retires rather than being renewed.
    const RETIREMENT_AGE: u32 = 62;
    /// A club never strips its whole department: it retires a man only when
    /// somebody is left to do the work.
    const MIN_SCOUTS: usize = 2;

    /// Share of the club's wage bill a new scout's salary must fit inside.
    /// Small — a scout is a rounding error against a squad — but it is the
    /// gate that stops a broke club hiring its way to a global network.
    const WAGE_HEADROOM_SHARE: f64 = 0.02;

    /// True on the day the desk sits.
    pub fn is_review_day(date: NaiveDate) -> bool {
        date.month() == Self::REVIEW_MONTH && date.day() == Self::REVIEW_DAY
    }

    /// Run the yearly review for every club in one country.
    pub fn run(country: &mut Country, market_map: &MarketMap, date: NaiveDate) {
        if !Self::is_review_day(date) {
            return;
        }
        let country_id = country.id;
        let year = date.year();
        // The country's own import priors, resolved once: this is the menu
        // every club in the league picks its desired markets from, because a
        // league's clubs shop where its agents and its friendly clubs
        // already are.
        let priors: Vec<(u32, f32)> = market_map
            .profile(country_id)
            .import
            .iter()
            .map(|c| (c.country_id, c.weight))
            .collect();
        if priors.is_empty() {
            return;
        }
        let names = country.generator_data.people_names.clone();

        for club in &mut country.clubs {
            Self::review_corridor_outcomes(club, country_id);
            Self::retire_aged_scouts(club, date);

            let reputation = club
                .teams
                .main()
                .or_else(|| club.teams.teams.first())
                .map(|t| t.reputation.world as i16)
                .unwrap_or(0);

            let wanted = Self::desired_markets(club.id, year, reputation, &priors);
            let Some(target_country) = wanted.into_iter().find(|country_id| {
                Self::best_scout_level(club, *country_id) < ClubMarketKnowledge::COVERED_LEVEL
            }) else {
                continue;
            };

            let salary = Self::scout_salary(reputation);
            if !Self::can_afford(club, salary) {
                continue;
            }
            let Some(region) = market_map.facts(target_country).map(|facts| facts.region) else {
                continue;
            };

            let scout = ScoutFactory::hire(
                country_id,
                target_country,
                region,
                reputation,
                salary,
                date,
                &names,
            );
            if let Some(team) = club
                .teams
                .teams
                .iter_mut()
                .find(|t| t.team_type == crate::TeamType::Main)
            {
                team.staffs.staffs.push(scout);
            }
        }
    }

    /// The markets this club would like covered this year, best first.
    ///
    /// Its country's import priors, weighted by a per-club, per-year belief
    /// noise. The noise is what stops twenty clubs in one league hiring the
    /// same specialist in the same summer — gates read truth, rankings read
    /// belief — and it is stable within a year, so a club does not change
    /// its mind between two passes.
    fn desired_markets(
        club_id: u32,
        year: i32,
        reputation: i16,
        priors: &[(u32, f32)],
    ) -> Vec<u32> {
        let breadth = if reputation >= 7000 {
            6
        } else if reputation >= 5000 {
            4
        } else if reputation >= 3000 {
            2
        } else {
            1
        };
        let mut scored: Vec<(u32, f32)> = priors
            .iter()
            .map(|(country_id, weight)| {
                (
                    *country_id,
                    weight * Self::belief(club_id, *country_id, year),
                )
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(breadth);
        scored
            .into_iter()
            .map(|(country_id, _)| country_id)
            .collect()
    }

    /// Stable per-(club, market, year) opinion in 0.5..1.5. A hash, not a
    /// draw: two passes in the same year must agree, and no RNG stream is
    /// consumed by a decision most clubs will not act on.
    fn belief(club_id: u32, country_id: u32, year: i32) -> f32 {
        let mut hash = club_id
            .wrapping_mul(2_654_435_761)
            .wrapping_add(country_id.wrapping_mul(40_503))
            .wrapping_add((year as u32).wrapping_mul(2_246_822_519));
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        0.5 + (hash % 1000) as f32 / 1000.0
    }

    fn best_scout_level(club: &Club, country_id: u32) -> u8 {
        club.teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .filter(|s| Self::is_scout(s))
            .map(|s| s.staff_attributes.knowledge.country_level(country_id))
            .max()
            .unwrap_or(0)
    }

    fn is_scout(staff: &Staff) -> bool {
        staff
            .contract
            .as_ref()
            .map(|c| matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout))
            .unwrap_or(false)
    }

    /// How each market the club works has actually served it.
    ///
    /// A corridor that delivers starters strengthens; one that delivers men
    /// who watch does not. Read off the players themselves — the share of
    /// the side's football each foreign import is getting — rather than off
    /// a bookkeeping trail, so it stays true after loans, sales and injuries
    /// have moved things around.
    ///
    /// Once a year is the right cadence: a signing needs a season before
    /// anyone at the club has an opinion about the market he came from.
    fn review_corridor_outcomes(club: &mut Club, club_country_id: u32) {
        let mut shares: Vec<(u32, f32, usize)> = Vec::new();
        for team in club.teams.teams.iter().filter(|t| !t.team_type.is_youth()) {
            for player in &team.players.players {
                if player.country_id == club_country_id || player.is_on_loan() {
                    continue;
                }
                match shares
                    .iter_mut()
                    .find(|(country, _, _)| *country == player.country_id)
                {
                    Some((_, total, count)) => {
                        *total += player.happiness.starter_ratio;
                        *count += 1;
                    }
                    None => shares.push((player.country_id, player.happiness.starter_ratio, 1)),
                }
            }
        }
        for (country_id, total, count) in shares {
            if count == 0 {
                continue;
            }
            club.market_ledger
                .record_outcome(country_id, total / count as f32);
        }
    }

    /// A scout past retirement age leaves, and his knowledge leaves with
    /// him — the club keeps only its own half, the ledger. Never below a
    /// working department.
    fn retire_aged_scouts(club: &mut Club, date: NaiveDate) {
        for team in club.teams.teams.iter_mut() {
            let scouts = team
                .staffs
                .staffs
                .iter()
                .filter(|s| Self::is_scout(s))
                .count();
            if scouts <= Self::MIN_SCOUTS {
                continue;
            }
            let mut allowed_removals = scouts - Self::MIN_SCOUTS;
            team.staffs.staffs.retain(|staff| {
                if allowed_removals == 0 || !Self::is_scout(staff) {
                    return true;
                }
                let age = (date.year() - staff.birth_date.year()) as u32;
                if age > Self::RETIREMENT_AGE {
                    allowed_removals -= 1;
                    return false;
                }
                true
            });
        }
    }

    /// What this club would pay a specialist, on the same reputation curve
    /// the world generator prices staff with.
    fn scout_salary(reputation: i16) -> u32 {
        let rep_factor = (reputation.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        (8_000.0 + rep_factor * 90_000.0) as u32
    }

    /// Does the club have room in its wage bill for one more scout?
    fn can_afford(club: &Club, salary: u32) -> bool {
        if club.finance.balance.balance < salary as i64 {
            return false;
        }
        let wage_bill: u64 = club
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.players.iter())
            .filter_map(|p| p.contract.as_ref().map(|c| c.salary as u64))
            .sum();
        // A club with no wage bill at all (a fixture, a shell) is not in the
        // market for staff.
        wage_bill > 0 && (salary as f64) <= wage_bill as f64 * Self::WAGE_HEADROOM_SHARE
    }
}

/// Mints one scout. The core-side counterpart of the world generator's
/// staff factory — the same shape of person, hired mid-save.
///
/// Deliberately narrow: a specialist scout's job is to know a market and
/// judge a player, so only the attributes those two things read are given
/// any spread. Everything else sits at a plain professional baseline rather
/// than being invented.
pub struct ScoutFactory;

impl ScoutFactory {
    #[allow(clippy::too_many_arguments)]
    pub fn hire(
        club_country_id: u32,
        market_country_id: u32,
        market_region: ScoutingRegion,
        club_reputation: i16,
        salary: u32,
        date: NaiveDate,
        names: &crate::PeopleNameGeneratorData,
    ) -> Staff {
        let rep_factor = (club_reputation.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        let judging = (8.0 + rep_factor * 9.0 + FloatUtils::random(-2.0, 2.0)).clamp(1.0, 20.0);

        let mut knowledge = StaffKnowledge {
            judging_player_ability: judging as u8,
            judging_player_potential: (judging - 1.0).max(1.0) as u8,
            tactical_knowledge: (6.0 + rep_factor * 8.0) as u8,
            known_regions: vec![market_region],
            region_familiarity: Vec::new(),
            known_countries: Vec::new(),
        };
        // He was hired FOR this market: he arrives already knowing it, which
        // is the whole point of the hire and the reason a corridor can open
        // within a window or two rather than a decade.
        knowledge.seed_country(market_country_id, IntegerUtils::random(40, 61) as u8);
        // And he knows the country that employs him, at a working level.
        knowledge.seed_country(club_country_id, 60);

        let birth_year = date.year() - IntegerUtils::random(32, 58);

        Staff::new(
            next_staff_id(),
            FullName::new(
                Self::pick(&names.first_names, "Scout"),
                Self::pick(&names.last_names, "Anonymous"),
            ),
            club_country_id,
            NaiveDate::from_ymd_opt(birth_year, 6, 1).unwrap_or(date),
            StaffAttributes {
                coaching: StaffCoaching {
                    attacking: 6,
                    defending: 6,
                    fitness: 6,
                    mental: 8,
                    tactical: 8,
                    technical: 7,
                    working_with_youngsters: 8,
                },
                goalkeeping: StaffGoalkeeperCoaching {
                    distribution: 5,
                    handling: 5,
                    shot_stopping: 5,
                },
                mental: StaffMental {
                    adaptability: (10.0 + rep_factor * 6.0) as u8,
                    determination: 12,
                    discipline: 12,
                    man_management: 10,
                    motivating: 10,
                },
                knowledge,
                data_analysis: StaffDataAnalysis {
                    judging_player_data: (6.0 + rep_factor * 8.0) as u8,
                    judging_team_data: (6.0 + rep_factor * 6.0) as u8,
                    presenting_data: 9,
                },
                medical: StaffMedical {
                    physiotherapy: 3,
                    sports_science: 4,
                    non_player_tendencies: 8,
                },
            },
            Some(StaffClubContract::new(
                salary,
                NaiveDate::from_ymd_opt(date.year() + IntegerUtils::random(2, 5), 6, 30)
                    .unwrap_or(date),
                StaffPosition::Scout,
                StaffStatus::Active,
            )),
            PersonAttributes {
                adaptability: FloatUtils::random(8.0, 18.0),
                ambition: FloatUtils::random(6.0, 16.0),
                controversy: FloatUtils::random(0.0, 10.0),
                loyalty: FloatUtils::random(8.0, 18.0),
                pressure: FloatUtils::random(8.0, 16.0),
                professionalism: FloatUtils::random(10.0, 19.0),
                sportsmanship: FloatUtils::random(8.0, 18.0),
                temperament: FloatUtils::random(8.0, 18.0),
                consistency: FloatUtils::random(8.0, 18.0),
                dirtiness: FloatUtils::random(0.0, 8.0),
                important_matches: FloatUtils::random(8.0, 16.0),
            },
            StaffLicenseType::NationalC,
            None,
        )
    }

    fn pick(names: &[String], fallback: &str) -> String {
        if names.is_empty() {
            return fallback.to_string();
        }
        let index = IntegerUtils::random(0, names.len() as i32) as usize;
        names
            .get(index)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn the_desk_sits_once_a_year() {
        assert!(ScoutMarketDesk::is_review_day(day(2026, 6, 15)));
        assert!(!ScoutMarketDesk::is_review_day(day(2026, 6, 16)));
        assert!(!ScoutMarketDesk::is_review_day(day(2026, 7, 15)));
    }

    #[test]
    fn belief_is_stable_within_a_year_and_moves_between_them() {
        let a = ScoutMarketDesk::belief(11, 22, 2026);
        assert_eq!(a, ScoutMarketDesk::belief(11, 22, 2026));
        assert_ne!(a, ScoutMarketDesk::belief(11, 22, 2027));
        assert_ne!(a, ScoutMarketDesk::belief(12, 22, 2026));
        assert!((0.5..=1.5).contains(&a));
    }

    #[test]
    fn two_clubs_in_one_league_want_different_markets() {
        // Same priors, same reputation, different clubs: the belief noise
        // has to separate them, or every club in a league hires the same
        // specialist in the same summer.
        let priors: Vec<(u32, f32)> = (1..=12).map(|id| (id, 1.0 - id as f32 * 0.05)).collect();
        let a = ScoutMarketDesk::desired_markets(101, 2026, 6000, &priors);
        let b = ScoutMarketDesk::desired_markets(202, 2026, 6000, &priors);
        assert_eq!(a.len(), 4);
        assert_ne!(a, b, "clubs must diverge on which markets they want");
    }

    #[test]
    fn a_bigger_club_wants_more_markets() {
        let priors: Vec<(u32, f32)> = (1..=12).map(|id| (id, 1.0)).collect();
        let giant = ScoutMarketDesk::desired_markets(1, 2026, 8000, &priors);
        let minnow = ScoutMarketDesk::desired_markets(1, 2026, 1500, &priors);
        assert!(giant.len() > minnow.len());
        assert_eq!(minnow.len(), 1);
    }

    #[test]
    fn the_hired_scout_arrives_knowing_the_market_he_was_hired_for() {
        let names = crate::PeopleNameGeneratorData {
            first_names: vec!["Ivan".to_string()],
            last_names: vec!["Petrov".to_string()],
            nicknames: Vec::new(),
        };
        let scout = ScoutFactory::hire(
            10,
            77,
            ScoutingRegion::SouthAmerica,
            7000,
            50_000,
            day(2026, 6, 15),
            &names,
        );
        let knowledge = &scout.staff_attributes.knowledge;
        assert!(
            knowledge.country_level(77) >= ClubMarketKnowledge::COVERED_LEVEL,
            "a specialist must cover the market he was hired for"
        );
        assert!(
            knowledge.country_level(10) >= 30,
            "he knows his employer's own market"
        );
        assert_eq!(knowledge.country_level(999), 0, "and nothing else");
        assert!(
            knowledge
                .known_regions
                .contains(&ScoutingRegion::SouthAmerica)
        );
    }
}
