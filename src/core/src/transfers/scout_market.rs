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

use chrono::{Datelike, Duration, NaiveDate};

use crate::club::staff::model::attributes::{
    StaffAttributes, StaffCoaching, StaffDataAnalysis, StaffGoalkeeperCoaching, StaffKnowledge,
    StaffMedical, StaffMental,
};
use crate::club::staff::model::contract::{StaffClubContract, StaffPosition, StaffStatus};
use crate::country::Country;
use crate::shared::FullName;
use crate::transfers::market_knowledge::ClubMarketKnowledge;
use crate::transfers::market_map::MarketMap;
use crate::transfers::window::TransferCalendar;
use crate::utils::{FloatUtils, IntegerUtils};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::transfers::scouting_region::ScoutingRegion;
use crate::club::board::ChairmanAmbition;
use crate::{Club, PersonAttributes, Staff, StaffLicenseType};

/// Ids for staff minted after world generation.
///
/// The world generator has its own counter starting at 1; this one starts
/// far above anything a generated world reaches (a full world carries tens
/// of thousands of staff) and is additionally pushed past the real maximum
/// at load by [`StaffIdSequence::seed`]. Same discipline as the player
/// sequence: two counters handing out the same id is a bug that surfaces as
/// one person being two people.
static STAFF_ID_SEQUENCE: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(500_000_000));

/// The runtime staff-id counter.
pub struct StaffIdSequence;

impl StaffIdSequence {
    /// Next id for a staff member minted at runtime.
    pub fn next() -> u32 {
        STAFF_ID_SEQUENCE.fetch_add(1, Ordering::SeqCst)
    }

    /// Push the counter past every id the loaded world uses. Called once
    /// after world generation, exactly like the player sequence.
    pub fn seed(min_exclusive: u32) {
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
}

/// The scouting-department market: who a club wants covering which country,
/// and what it does about the gap.
pub struct ScoutMarketDesk;

impl ScoutMarketDesk {
    /// Age past which a scout retires rather than being renewed.
    const RETIREMENT_AGE: u32 = 62;
    /// A club never strips its whole department: it retires a man only when
    /// somebody is left to do the work.
    const MIN_SCOUTS: usize = 2;

    /// Share of the club's wage bill a new scout's salary must fit inside.
    /// Small — a scout is a rounding error against a squad — but it is the
    /// gate that stops a broke club hiring its way to a global network.
    const WAGE_HEADROOM_SHARE: f64 = 0.02;

    /// Years back a ledger row still counts as a market the club is
    /// currently working. Two: a corridor with no business in two years is
    /// one the club is letting go, not one it wants a specialist for.
    const LEDGER_RECENCY_YEARS: i64 = 2;

    /// Chance a big club sends someone to a market NOBODY in its country
    /// works, scaled by its chairman's ambition and its own standing.
    ///
    /// Small on purpose. Without it the world is a lookup table with a
    /// decay curve on it: the scout desk could only desire markets its
    /// country's card already named, so an off-card corridor was never
    /// hired for, never opened, and the 2040 world was the 2026 world.
    /// With it, a handful of giants a season take a punt, most of which
    /// produce nothing.
    const EXPLORATION_CHANCE: f32 = 0.03;
    /// World exporters an exploring club draws from.
    const EXPLORATION_POOL: usize = 20;

    /// True on the day this country's desk sits — the day before its main
    /// transfer window opens.
    ///
    /// A single hard-coded 15 June sat the desk mid-season for MLS, Brazil,
    /// Argentina, Japan and the Nordics: those leagues were reviewing their
    /// scouting departments in the middle of a season and hiring for a
    /// window that had already closed. Every other transfer pass reads the
    /// country's own calendar, and so does this one.
    pub fn is_review_day(country_code: &str, date: NaiveDate) -> bool {
        Self::review_day(country_code, date) == Some(date)
    }

    /// The review date for this country in `date`'s season year.
    fn review_day(country_code: &str, date: NaiveDate) -> Option<NaiveDate> {
        let windows = TransferCalendar::for_country(country_code, date);
        // The LONGEST window is the main one everywhere the calendar table
        // models: the European summer, the MLS and Asian winter, the
        // Latin-American December break, the Nordic pre-season February.
        let (summer_open, summer_close) = windows.summer_window;
        let (winter_open, winter_close) = windows.winter_window;
        let main_open = if (summer_close - summer_open) >= (winter_close - winter_open) {
            summer_open
        } else {
            winter_open
        };
        main_open.checked_sub_signed(Duration::days(1))
    }

    /// Run the yearly review for every club in one country.
    pub fn run(country: &mut Country, market_map: &MarketMap, date: NaiveDate) {
        if !Self::is_review_day(&country.code, date) {
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
        // The world's heaviest exporters, for the exploration draw. Read
        // once per country per year rather than per club.
        let world_exporters = market_map.top_exporters(Self::EXPLORATION_POOL);
        let names = country.generator_data.people_names.clone();

        for club in &mut country.clubs {
            // The two housekeeping passes are NOT conditional on the
            // country having a card: a club still learns how its corridors
            // served it and still loses men to age in a country the data
            // does not describe. Only the HIRE needs a menu to pick from.
            Self::review_corridor_outcomes(club, country_id);
            Self::retire_aged_scouts(club, date);

            let reputation = club
                .teams
                .main()
                .or_else(|| club.teams.teams.first())
                .map(|t| t.reputation.world as i16)
                .unwrap_or(0);

            let wanted = Self::desired_markets(
                club,
                country_id,
                year,
                reputation,
                &priors,
                &world_exporters,
                date,
            );
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
            let club_region = market_map
                .facts(country_id)
                .map(|facts| facts.region)
                .unwrap_or(region);

            let scout = ScoutFactory::hire(
                country_id,
                club_region,
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
                // Through the collection, not the bare Vec: it owns whatever
                // bookkeeping a staff insert carries.
                team.staffs.push(scout);
            }
        }
    }

    /// The markets this club would like covered this year, best first.
    ///
    /// Four sources, all weighted by the same per-club, per-year belief
    /// noise. The noise is what stops twenty clubs in one league hiring the
    /// same specialist in the same summer — gates read truth, rankings read
    /// belief — and it is stable within a year, so a club does not change
    /// its mind between two passes.
    ///
    ///   * its **country's import priors** — where its league's agents and
    ///     friendly clubs already are;
    ///   * its **own recent business** — a club that has just signed its
    ///     first Colombian wants someone who knows Colombia. This is the
    ///     source that lets an off-card corridor become a worked one: the
    ///     ledger records the signing, the desk hires for it, and the
    ///     corridor is open for as long as the man and his successes last;
    ///   * the **markets it is already looking at** — open shortlists and
    ///     scout monitoring rows abroad;
    ///   * **exploration** — rarely, a giant sends someone somewhere nobody
    ///     in its country works.
    ///
    /// The last three carry a discount against the country prior, because a
    /// league's established corridors really are the likeliest place a club
    /// hires into; they are not so discounted that they cannot win.
    #[allow(clippy::too_many_arguments)]
    fn desired_markets(
        club: &Club,
        club_country_id: u32,
        year: i32,
        reputation: i16,
        priors: &[(u32, f32)],
        world_exporters: &[u32],
        date: NaiveDate,
    ) -> Vec<u32> {
        /// Weight a market the club itself has done business in carries
        /// against its country's own top corridor.
        const OWN_LEDGER_WEIGHT: f32 = 0.9;
        /// Weight a market it is merely looking at carries.
        const WATCHING_WEIGHT: f32 = 0.6;
        /// Weight an exploratory punt carries. Below the other three, so it
        /// wins only when the club's own map is thin.
        const EXPLORATION_WEIGHT: f32 = 0.5;

        let mut wanted: HashMap<u32, f32> = HashMap::new();
        let mut want = |country_id: u32, weight: f32| {
            if country_id == 0 || country_id == club_country_id {
                return;
            }
            let slot = wanted.entry(country_id).or_insert(0.0);
            *slot = slot.max(weight);
        };

        for (country_id, weight) in priors {
            want(*country_id, *weight);
        }
        for entry in club.market_ledger.entries() {
            if entry.bootstrapped {
                continue;
            }
            let years = (date - entry.last_signing).num_days() as f32 / 365.0;
            if years > Self::LEDGER_RECENCY_YEARS as f32 {
                continue;
            }
            want(entry.country_id, OWN_LEDGER_WEIGHT);
        }
        for country_id in Self::markets_being_watched(club, club_country_id) {
            want(country_id, WATCHING_WEIGHT);
        }
        if let Some(country_id) = Self::exploration_draw(club, reputation, world_exporters) {
            want(country_id, EXPLORATION_WEIGHT);
        }
        drop(want);

        let candidates: Vec<(u32, f32)> = wanted.into_iter().collect();
        Self::rank_markets(club.id, year, reputation, &candidates)
    }

    /// The pure half of [`Self::desired_markets`]: candidate markets in, the
    /// club's shortlist of them out.
    ///
    /// The belief noise lives here. It is what stops twenty clubs in one
    /// league hiring the same specialist in the same summer, and it is a
    /// hash rather than a draw so two passes in the same year agree and no
    /// RNG stream is consumed by a decision most clubs will not act on.
    fn rank_markets(
        club_id: u32,
        year: i32,
        reputation: i16,
        candidates: &[(u32, f32)],
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
        let mut scored: Vec<(u32, f32)> = candidates
            .iter()
            .map(|(country_id, weight)| {
                (*country_id, weight * Self::belief(club_id, *country_id, year))
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

    /// Countries this club is already looking in — the foreign players on
    /// its live monitoring rows and open shortlists, resolved through the
    /// club's own [`KnownPlayerMemory`], which is the only record inside a
    /// per-country borrow that says where a foreign target actually plays.
    ///
    /// A club watching three Uruguayans has already decided Uruguay is
    /// interesting; the specialist is what turns that into a corridor.
    fn markets_being_watched(club: &Club, club_country_id: u32) -> Vec<u32> {
        let plan = &club.transfer_plan;
        let watching: HashSet<u32> = plan
            .scout_monitoring
            .iter()
            .filter(|row| row.is_active_interest())
            .map(|row| row.player_id)
            .chain(
                plan.shortlists
                    .iter()
                    .flat_map(|list| list.candidates.iter())
                    .map(|candidate| candidate.player_id),
            )
            .collect();
        plan.known_players
            .iter()
            .filter(|known| watching.contains(&known.player_id))
            .map(|known| known.last_known_country_id)
            .filter(|id| *id != 0 && *id != club_country_id)
            .collect()
    }

    /// One exploratory market, or `None` — the rare punt outside everything
    /// the club and its country already know.
    ///
    /// Gated on standing and on the chairman's ambition, because this is a
    /// discretionary spend nobody can justify from results: a small club
    /// does not fly a man to a country its league has never signed from.
    fn exploration_draw(club: &Club, reputation: i16, world_exporters: &[u32]) -> Option<u32> {
        if world_exporters.is_empty() {
            return None;
        }
        let standing = (reputation.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        let ambition = match club.board.chairman.ambition {
            ChairmanAmbition::Reckless => 1.0,
            ChairmanAmbition::Ambitious => 0.75,
            ChairmanAmbition::Balanced => 0.4,
            ChairmanAmbition::Conservative => 0.15,
        };
        let chance = Self::EXPLORATION_CHANCE * standing * ambition;
        if FloatUtils::random(0.0, 1.0) >= chance {
            return None;
        }
        let index = IntegerUtils::random(0, world_exporters.len() as i32) as usize;
        world_exporters.get(index).copied()
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

    fn is_chief_scout(staff: &Staff) -> bool {
        staff
            .contract
            .as_ref()
            .map(|c| matches!(c.position, StaffPosition::ChiefScout))
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
        let mut retired: HashSet<u32> = HashSet::new();
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
            // A department with one chief scout keeps him whatever his age:
            // retiring the only man who can sign off a report while three
            // plain scouts stay is not a department losing someone to age,
            // it is a department losing its head.
            let chiefs = team
                .staffs
                .staffs
                .iter()
                .filter(|s| Self::is_chief_scout(s))
                .count();
            let mut allowed_removals = scouts - Self::MIN_SCOUTS;
            team.staffs.staffs.retain(|staff| {
                if allowed_removals == 0 || !Self::is_scout(staff) {
                    return true;
                }
                if chiefs <= 1 && Self::is_chief_scout(staff) {
                    return true;
                }
                let age = (date.year() - staff.birth_date.year()) as u32;
                if age > Self::RETIREMENT_AGE {
                    allowed_removals -= 1;
                    retired.insert(staff.id);
                    return false;
                }
                true
            });
        }
        if retired.is_empty() {
            return;
        }
        // His open assignments leave with him. An assignment still naming a
        // man who is no longer at the club silently fell back to default
        // judging — the club kept "scouting" Colombia with nobody on it —
        // so the row is released instead, and the next `assign_scouts` pass
        // re-fills it from the department that actually exists.
        for assignment in club.transfer_plan.scouting_assignments.iter_mut() {
            if let Some(scout_id) = assignment.scout_staff_id
                && retired.contains(&scout_id)
            {
                assignment.scout_staff_id = None;
            }
        }
        club.transfer_plan
            .scout_monitoring
            .retain(|row| !retired.contains(&row.scout_staff_id));
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
        club_region: ScoutingRegion,
        market_country_id: u32,
        market_region: ScoutingRegion,
        club_reputation: i16,
        salary: u32,
        date: NaiveDate,
        names: &crate::PeopleNameGeneratorData,
    ) -> Staff {
        let rep_factor = (club_reputation.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        let judging = (8.0 + rep_factor * 9.0 + FloatUtils::random(-2.0, 2.0)).clamp(1.0, 20.0);

        // His own patch AND the one he was hired for. The first cut gave him
        // only the market, so a Brazil specialist hired by a Turkish club
        // could not see a Turkish reserve-team player — a scout who does not
        // know the country he works in is not a person who exists.
        let mut known_regions = vec![market_region];
        if club_region != market_region {
            known_regions.push(club_region);
        }
        let mut knowledge = StaffKnowledge {
            judging_player_ability: judging as u8,
            judging_player_potential: (judging - 1.0).max(1.0) as u8,
            tactical_knowledge: (6.0 + rep_factor * 8.0) as u8,
            known_regions,
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
            StaffIdSequence::next(),
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
    fn the_desk_sits_on_each_countrys_own_pre_season() {
        // England: the European summer window opens 1 June, so the desk
        // sits on 31 May — before the summer's business, not during it.
        assert!(ScoutMarketDesk::is_review_day("gb", day(2026, 5, 31)));
        assert!(!ScoutMarketDesk::is_review_day("gb", day(2026, 6, 15)));

        // The MLS and Japanese primary windows are the winter ones, and
        // 15 June sat their desks in the middle of a season.
        for code in ["us", "jp"] {
            let review = ScoutMarketDesk::review_day(code, day(2026, 6, 15))
                .expect("every calendar yields a review day");
            assert_ne!(review, day(2026, 6, 15), "{code} must not sit mid-season");
            assert!(ScoutMarketDesk::is_review_day(code, review));
        }

        // And it is still exactly one day a year, wherever it falls.
        for code in ["gb", "us", "br", "ru", "no", "jp"] {
            let sittings = (0..365)
                .filter_map(|offset| day(2026, 1, 1).checked_add_signed(Duration::days(offset)))
                .filter(|d| ScoutMarketDesk::is_review_day(code, *d))
                .count();
            assert_eq!(sittings, 1, "{code} sat {sittings} times in 2026");
        }
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
        let a = ScoutMarketDesk::rank_markets(101, 2026, 6000, &priors);
        let b = ScoutMarketDesk::rank_markets(202, 2026, 6000, &priors);
        assert_eq!(a.len(), 4);
        assert_ne!(a, b, "clubs must diverge on which markets they want");
    }

    #[test]
    fn a_bigger_club_wants_more_markets() {
        let priors: Vec<(u32, f32)> = (1..=12).map(|id| (id, 1.0)).collect();
        let giant = ScoutMarketDesk::rank_markets(1, 2026, 8000, &priors);
        let minnow = ScoutMarketDesk::rank_markets(1, 2026, 1500, &priors);
        assert!(giant.len() > minnow.len());
        assert_eq!(minnow.len(), 1);
    }

    #[test]
    fn a_market_the_club_itself_works_can_outrank_its_countrys_card() {
        // The whole point of the ledger source: a club that has just made
        // its first signing out of a country nobody in its league works
        // must be able to want a specialist for it. Country 99 is off the
        // card entirely and enters at the ledger weight; with the card's
        // own tail sitting at 0.2 it has to be able to win.
        let mut candidates: Vec<(u32, f32)> = (1..=6).map(|id| (id, 0.2)).collect();
        candidates.push((99, 0.9));
        let wanted = ScoutMarketDesk::rank_markets(7, 2026, 8000, &candidates);
        assert!(
            wanted.contains(&99),
            "an off-card market the club has business in must be desirable: {wanted:?}"
        );
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
            ScoutingRegion::MiddleEastEurope,
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
        assert!(
            knowledge
                .known_regions
                .contains(&ScoutingRegion::MiddleEastEurope),
            "and the region his employer sits in: {:?}",
            knowledge.known_regions
        );
    }
}
