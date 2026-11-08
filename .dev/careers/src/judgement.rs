//! Do clubs judge a man by what he did? Among regulars of similar ability at
//! clubs of similar size, what a season's rating does to the next season's
//! role, contract and departure.

use crate::reading::{Snap, Tier};
use core::{Player, SimulatorData};
use std::collections::{BTreeMap, HashMap};

const MIN_SEASON_APPEARANCES: u16 = 15;
const ABILITY_BAND_WIDTH: u8 = 15;
const RENEWAL_SIGNED: &str = "dec_contract_renewal_signed";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum SeasonVerdict {
    Poor,
    Middling,
    Good,
}

impl SeasonVerdict {
    const ALL: [SeasonVerdict; 3] = [
        SeasonVerdict::Poor,
        SeasonVerdict::Middling,
        SeasonVerdict::Good,
    ];
}

struct Watch {
    verdict: SeasonVerdict,
    role_rank: u8,
    club_id: u32,
    listed: bool,
    sold: bool,
    gone: bool,
    renewals_before: u32,
}

#[derive(Default, Clone, Copy)]
struct Outcomes {
    men: u32,
    promoted: u32,
    demoted: u32,
    renewed: u32,
    listed: u32,
    sold: u32,
    left_for_nothing: u32,
}

#[derive(Default)]
pub struct ClubJudgement {
    watch: HashMap<u32, Watch>,
    outcomes: BTreeMap<(Tier, SeasonVerdict), Outcomes>,
    rating_sum: BTreeMap<SeasonVerdict, (f64, u32)>,
}

impl ClubJudgement {
    fn renewals(p: &Player) -> u32 {
        p.decision_history
            .items
            .iter()
            .filter(|d| d.decision == RENEWAL_SIGNED)
            .count() as u32
    }

    /// While the season runs: what has happened to the men being watched.
    pub fn on_player(&mut self, p: &Player, before: Option<&Snap>, now: &Snap, sold_today: bool) {
        if let Some(watch) = self.watch.get_mut(&p.id) {
            watch.listed |= now.listed && before.is_some_and(|b| !b.listed);
            watch.sold |= sold_today;
            watch.gone |= now.club_id != watch.club_id && !now.on_loan;
        }
    }

    /// A year on: close last year's watch, and start this year's from the
    /// season just finished. A man's verdict is his rating against other
    /// regulars of his ability at clubs his size.
    pub fn on_season(&mut self, data: &SimulatorData, snaps: &HashMap<u32, Snap>) {
        let mut players: Vec<&Player> = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        players.extend(team.players.players.iter());
                    }
                }
            }
        }
        players.extend(data.free_agents.iter());

        let watch = std::mem::take(&mut self.watch);
        let mut renewals: HashMap<u32, u32> = HashMap::with_capacity(watch.len());
        for p in &players {
            if watch.contains_key(&p.id) {
                renewals.insert(p.id, Self::renewals(p));
            }
        }
        for (id, watched) in watch {
            let Some(now) = snaps.get(&id) else { continue };
            let tier = Tier::of_rank(now.rank);
            let row = self.outcomes.entry((tier, watched.verdict)).or_default();
            row.men += 1;
            let role_now = Snap::role_rank(now.status);
            if !watched.gone && role_now > watched.role_rank {
                row.promoted += 1;
            }
            if !watched.gone && role_now < watched.role_rank {
                row.demoted += 1;
            }
            if renewals.get(&id).copied().unwrap_or(0) > watched.renewals_before {
                row.renewed += 1;
            }
            row.listed += watched.listed as u32;
            row.sold += watched.sold as u32;
            row.left_for_nothing += (watched.gone && !watched.sold) as u32;
        }

        // Cell means first, so a verdict is relative to his peers.
        let mut seasons: Vec<(&Player, &Snap, f32, (Tier, u8))> = Vec::new();
        let mut cells: HashMap<(Tier, u8), (f32, f32, u32)> = HashMap::new();
        for p in &players {
            let Some(snap) = snaps.get(&p.id) else {
                continue;
            };
            if !snap.regular || snap.on_loan {
                continue;
            }
            let Some(last) = p.statistics_history.items.last() else {
                continue;
            };
            let stats = &last.statistics;
            if stats.played + stats.played_subs < MIN_SEASON_APPEARANCES {
                continue;
            }
            let cell = (snap.tier(), snap.ca / ABILITY_BAND_WIDTH);
            let rating = stats.mean_rating();
            if rating <= 0.0 {
                continue;
            }
            let entry = cells.entry(cell).or_insert((0.0, 0.0, 0));
            entry.0 += rating;
            entry.1 += rating * rating;
            entry.2 += 1;
            seasons.push((p, snap, rating, cell));
        }
        for (p, snap, rating, cell) in seasons {
            let (sum, squares, n) = cells[&cell];
            let mean = sum / n as f32;
            let sd = (squares / n as f32 - mean * mean).max(0.0).sqrt();
            let verdict = if rating < mean - 0.5 * sd {
                SeasonVerdict::Poor
            } else if rating > mean + 0.5 * sd {
                SeasonVerdict::Good
            } else {
                SeasonVerdict::Middling
            };
            let total = self.rating_sum.entry(verdict).or_insert((0.0, 0));
            total.0 += rating as f64;
            total.1 += 1;
            self.watch.insert(
                p.id,
                Watch {
                    verdict,
                    role_rank: Snap::role_rank(snap.status),
                    club_id: snap.club_id,
                    listed: false,
                    sold: false,
                    gone: false,
                    renewals_before: Self::renewals(p),
                },
            );
        }
    }

    pub fn report(&self) {
        println!("\n-- does a season matter? regulars of similar ability at clubs of similar size, next season's outcome --");
        for verdict in SeasonVerdict::ALL {
            if let Some((sum, n)) = self.rating_sum.get(&verdict) {
                println!(
                    "  {verdict:?} seasons: {n} men-seasons, mean rating {:.2}",
                    sum / (*n).max(1) as f64
                );
            }
        }
        println!(
            "{:<14} {:<9} {:>7} | {:>9} {:>8} {:>8} {:>7} {:>6} {:>10}",
            "club size", "season", "men", "promoted%", "demoted%", "renewed%", "listed%", "sold%", "for nothing%"
        );
        for tier in Tier::CLUBS {
            for verdict in SeasonVerdict::ALL {
                let Some(row) = self.outcomes.get(&(tier, verdict)) else {
                    continue;
                };
                let n = row.men.max(1) as f64;
                println!(
                    "{:<14} {:<9} {:>7} | {:>8.1}% {:>7.1}% {:>7.1}% {:>6.1}% {:>5.1}% {:>9.1}%",
                    tier.label(),
                    format!("{verdict:?}"),
                    row.men,
                    row.promoted as f64 * 100.0 / n,
                    row.demoted as f64 * 100.0 / n,
                    row.renewed as f64 * 100.0 / n,
                    row.listed as f64 * 100.0 / n,
                    row.sold as f64 * 100.0 / n,
                    row.left_for_nothing as f64 * 100.0 / n,
                );
            }
        }
    }
}
