//! What a club does after it sells a man it was picking.
//!
//! Every other census here measures a move. This one follows the hole a
//! move leaves: how long until the club asks for his shirt, how long until
//! somebody arrives to wear it and how he compares, who inside the club
//! moved up on the day, what the fee did to the transfer budget, and what
//! the dressing room said. Running, because every reading is a difference
//! between one morning and the next.

use chrono::{Datelike, NaiveDate};
use core::club::staff::PlannedRole;
use core::club::staff::perception::AbilityEstimator;
use core::r#match::squad::selection::PitchLine;
use core::transfers::{TransferCalendar, TransferNeedReason, TransferType};
use core::{
    Club, ClubDirectionEvidence, HappinessEventType, Person, Player, PlayerFieldPositionGroup,
    PlayerPositionType, PlayerStatusType, SimulatorData,
};
use std::collections::{HashMap, HashSet};

/// A first-team player and where his manager's plan had him.
#[derive(Clone, Copy)]
struct PlannedMan {
    club_id: u32,
    role: PlannedRole,
    /// The plan's first choice for a shirt, whatever role he holds: a
    /// listed first choice is in the shop window and is still the man the
    /// side is built on.
    first_choice: bool,
    /// He had asked to go, as of the morning.
    asked: bool,
    group: PlayerFieldPositionGroup,
}

/// The man who arrived to wear the shirt.
struct Arrival {
    day: u32,
    date: NaiveDate,
    fee: f64,
    level: u8,
    age: u8,
    shirt: PlayerPositionType,
}

struct KeySale {
    day: u32,
    date: NaiveDate,
    club_id: u32,
    player_id: u32,
    cornerstone: bool,
    /// The club had put him on the market, or he had asked to go.
    let_go: bool,
    fee: f64,
    shirt: PlayerPositionType,
    group: PlayerFieldPositionGroup,
    level: u8,
    age: u8,
    /// The close of the window the sale fell in.
    window_close: Option<NaiveDate>,
    budget_before: f64,
    budget_day_after: Option<f64>,
    budget_next_month: Option<f64>,
    /// A search for his shirt was open before he left.
    search_already_open: bool,
    requests_before: HashSet<u32>,
    first_request_day: Option<u32>,
    promotions: u32,
    concerned: u32,
    /// The first day a team-mate held "sold and not replaced" against it.
    concern_day: Option<u32>,
    arrival: Option<Arrival>,
}

#[derive(Default)]
pub struct ReplacementCensus {
    planned: HashMap<u32, PlannedMan>,
    budgets: HashMap<u32, f64>,
    requests: HashMap<u32, Vec<(u32, PlayerFieldPositionGroup, bool)>>,
    history_seen: HashMap<u32, usize>,
    sales: Vec<KeySale>,
    /// Every fee a club paid, by the day it paid it.
    purchases: HashMap<u32, Vec<(u32, f64)>>,
    last_day: u32,
}

impl ReplacementCensus {
    /// A sale needs this much of the run after it for "never" to mean it.
    const FOLLOWED_DAYS: u32 = 60;
    /// A signing this soon after a concern says the concern was premature.
    const GRACE_DAYS: u32 = 30;
    /// Perceived levels within which a signing is like for like.
    const ABOUT_HIS_LEVEL: u8 = 8;
    /// The window after the one a sale fell in closes within this long.
    const NEXT_WINDOW_DAYS: i64 = 215;

    fn first_team_plan(club: &Club) -> impl Iterator<Item = (&Player, Option<PlannedRole>)> {
        club.teams.main().into_iter().flat_map(|team| {
            let plan = &team.staffs.head_coach().squad_plan;
            team.players.iter().map(|p| (p, plan.role_of(p.id)))
        })
    }

    fn is_first_choice(club: &Club, player_id: u32) -> bool {
        club.teams.main().is_some_and(|team| {
            team.staffs
                .head_coach()
                .squad_plan
                .entry(player_id)
                .is_some_and(|entry| entry.shirt.is_some() && entry.place == 0)
        })
    }

    fn find_player(club: &Club, player_id: u32) -> Option<&Player> {
        club.teams
            .teams
            .iter()
            .find_map(|team| team.players.iter().find(|p| p.id == player_id))
    }

    fn budget(club: &Club) -> f64 {
        club.finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or(0.0)
    }

    /// Team-mates who became more central to the plan than they were this
    /// morning, in the sold man's line.
    fn promotions(&self, club: &Club, group: PlayerFieldPositionGroup) -> u32 {
        Self::first_team_plan(club)
            .filter(|(player, _)| player.position().position_group() == group)
            .filter(|(player, role)| {
                match (self.planned.get(&player.id), role) {
                    (Some(before), Some(now)) => now.as_u8() < before.role.as_u8(),
                    _ => false,
                }
            })
            .count() as u32
    }

    fn concerned(club: &Club, sold_player: u32) -> u32 {
        club.teams
            .teams
            .iter()
            .flat_map(|team| team.players.iter())
            .filter(|player| {
                player.happiness.recent_events.iter().any(|event| {
                    event.event_type == HappinessEventType::ConcernedByClubDirection
                        && event
                            .context
                            .as_ref()
                            .and_then(|c| c.club_direction_context.as_ref())
                            .is_some_and(|direction| {
                                direction.focal_player_id == Some(sold_player)
                                    && direction
                                        .evidence
                                        .contains(&ClubDirectionEvidence::KeyPlayerSoldUnreplaced)
                            })
                })
            })
            .count() as u32
    }

    pub fn observe(&mut self, data: &SimulatorData, day: u32) {
        self.last_day = day;
        let today = data.date.date();
        let clubs: HashMap<u32, (&Club, &str)> = data
            .continents
            .iter()
            .flat_map(|continent| continent.countries.iter())
            .flat_map(|country| {
                country
                    .clubs
                    .iter()
                    .map(move |club| (club.id, (club, country.code.as_str())))
            })
            .collect();

        for country in data.continents.iter().flat_map(|c| c.countries.iter()) {
            let history = &country.transfer_market.transfer_history;
            let seen = self.history_seen.insert(country.id, history.len()).unwrap_or(0);
            for row in history.iter().skip(seen.min(history.len())) {
                if row.fee.amount > 0.0 {
                    self.purchases
                        .entry(row.to_club_id)
                        .or_default()
                        .push((day, row.fee.amount));
                }
                self.read_arrival(&clubs, row.to_club_id, row.player_id, row.fee.amount, day, today);
                if !matches!(row.transfer_type, TransferType::Permanent) || row.fee.amount <= 0.0 {
                    continue;
                }
                let Some(planned) = self.planned.get(&row.player_id).copied() else {
                    continue;
                };
                let key = planned.role.is_at_least(PlannedRole::Starter)
                    || (planned.role == PlannedRole::ShopWindow && planned.first_choice);
                if planned.club_id != row.from_club_id
                    || !key
                    || self.sales.iter().any(|s| s.player_id == row.player_id && s.day == day)
                {
                    continue;
                }
                let (Some((seller, code)), Some((buyer, _))) =
                    (clubs.get(&row.from_club_id), clubs.get(&row.to_club_id))
                else {
                    continue;
                };
                let Some(player) = Self::find_player(buyer, row.player_id) else {
                    continue;
                };
                let before = self.requests.get(&seller.id).cloned().unwrap_or_default();
                self.sales.push(KeySale {
                    day,
                    date: row.transfer_date,
                    club_id: seller.id,
                    player_id: row.player_id,
                    cornerstone: planned.role == PlannedRole::Cornerstone,
                    let_go: planned.role == PlannedRole::ShopWindow || planned.asked,
                    fee: row.fee.amount,
                    shirt: player.position(),
                    group: planned.group,
                    level: AbilityEstimator::observable_level(player),
                    age: player.age(today),
                    window_close: TransferCalendar::for_country(code, row.transfer_date)
                        .open_on(row.transfer_date)
                        .map(|(_, close)| close),
                    budget_before: self.budgets.get(&seller.id).copied().unwrap_or(0.0),
                    budget_day_after: None,
                    budget_next_month: None,
                    search_already_open: before.iter().any(|(_, group, _)| *group == planned.group),
                    requests_before: before
                        .iter()
                        .filter(|(_, _, replacement)| !replacement)
                        .map(|(id, _, _)| *id)
                        .collect(),
                    first_request_day: None,
                    promotions: self.promotions(seller, planned.group),
                    concerned: Self::concerned(seller, row.player_id),
                    concern_day: None,
                    arrival: None,
                });
            }
        }

        for sale in &mut self.sales {
            let Some((club, _)) = clubs.get(&sale.club_id) else {
                continue;
            };
            if sale.budget_day_after.is_none() && day > sale.day {
                sale.budget_day_after = Some(Self::budget(club));
            }
            if sale.budget_next_month.is_none() && day > sale.day && today.day() == 2 {
                sale.budget_next_month = Some(Self::budget(club));
            }
            if sale.concern_day.is_none() {
                let concerned = Self::concerned(club, sale.player_id);
                if concerned > 0 {
                    sale.concerned = concerned;
                    sale.concern_day = Some(day);
                }
            }
            if sale.first_request_day.is_none()
                && club.transfer_plan.transfer_requests.iter().any(|request| {
                    // A search raised for him, or one already open for his
                    // line that his departure re-shaped into a replacement.
                    request.position.position_group() == sale.group
                        && (!sale.requests_before.contains(&request.id)
                            || request.reason == TransferNeedReason::Replacement)
                })
            {
                sale.first_request_day = Some(day);
            }
        }

        self.planned.clear();
        self.budgets.clear();
        self.requests.clear();
        for (club, _) in clubs.values() {
            self.budgets.insert(club.id, Self::budget(club));
            self.requests.insert(
                club.id,
                club.transfer_plan
                    .transfer_requests
                    .iter()
                    .map(|r| {
                        (
                            r.id,
                            r.position.position_group(),
                            r.reason == TransferNeedReason::Replacement,
                        )
                    })
                    .collect(),
            );
            for (player, role) in Self::first_team_plan(club) {
                if let Some(role) = role {
                    self.planned.insert(
                        player.id,
                        PlannedMan {
                            club_id: club.id,
                            role,
                            first_choice: Self::is_first_choice(club, player.id),
                            asked: player.statuses.has(PlayerStatusType::Req),
                            group: player.position().position_group(),
                        },
                    );
                }
            }
        }
    }

    /// A player joining a club that still has a sold man's shirt to fill.
    fn read_arrival(
        &mut self,
        clubs: &HashMap<u32, (&Club, &str)>,
        to_club_id: u32,
        player_id: u32,
        fee: f64,
        day: u32,
        today: NaiveDate,
    ) {
        let Some((club, _)) = clubs.get(&to_club_id) else {
            return;
        };
        let Some(player) = Self::find_player(club, player_id) else {
            return;
        };
        let group = player.position().position_group();
        let Some(sale) = self
            .sales
            .iter_mut()
            .find(|s| s.club_id == to_club_id && s.arrival.is_none() && s.group == group && s.day < day)
        else {
            return;
        };
        sale.arrival = Some(Arrival {
            day,
            date: today,
            fee,
            level: AbilityEstimator::observable_level(player),
            age: player.age(today),
            shirt: player.position(),
        });
    }

    fn median(mut values: Vec<f64>) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }

    fn quartiles(values: Vec<f64>) -> String {
        if values.is_empty() {
            return "none".to_string();
        }
        let mut sorted = values;
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
        format!(
            "median {:.0} (quartiles {:.0} to {:.0}, n={})",
            at(0.5),
            at(0.25),
            at(0.75),
            sorted.len()
        )
    }

    fn share(part: usize, whole: usize) -> f64 {
        if whole == 0 {
            0.0
        } else {
            100.0 * part as f64 / whole as f64
        }
    }

    pub fn print(&self) {
        let sales = &self.sales;
        let followed: Vec<&KeySale> = sales
            .iter()
            .filter(|s| s.day + Self::FOLLOWED_DAYS <= self.last_day)
            .collect();
        println!();
        println!("=== key sales and what followed (day {}) ===", self.last_day);
        println!(
            "key sales (a permanent sale for a fee of a player planned as starter or cornerstone): {}  \
             [cornerstone {}, starter {}]",
            sales.len(),
            sales.iter().filter(|s| s.cornerstone).count(),
            sales.iter().filter(|s| !s.cornerstone).count(),
        );
        println!(
            "followed for {} days or more: {}",
            Self::FOLLOWED_DAYS,
            followed.len()
        );
        if followed.is_empty() {
            return;
        }
        let n = followed.len();

        let already = followed.iter().filter(|s| s.search_already_open).count();
        let requested: Vec<f64> = followed
            .iter()
            .filter_map(|s| s.first_request_day.map(|d| f64::from(d - s.day)))
            .collect();
        println!(
            "a search for his line already open on the day: {already} ({:.1}%)",
            Self::share(already, n)
        );
        println!(
            "days to the first new request for his line: {}; none raised: {} ({:.1}%)",
            Self::quartiles(requested.clone()),
            n - requested.len(),
            Self::share(n - requested.len(), n)
        );
        println!(
            "  raised within a day {:.1}% | a week {:.1}% | a month {:.1}%",
            Self::share(requested.iter().filter(|d| **d <= 1.0).count(), n),
            Self::share(requested.iter().filter(|d| **d <= 7.0).count(), n),
            Self::share(requested.iter().filter(|d| **d <= 30.0).count(), n),
        );

        let arrivals: Vec<(&KeySale, &Arrival)> = followed
            .iter()
            .filter_map(|s| s.arrival.as_ref().map(|a| (*s, a)))
            .collect();
        println!(
            "days to a signing for his line: {}",
            Self::quartiles(arrivals.iter().map(|(s, a)| f64::from(a.day - s.day)).collect())
        );
        let same_window = arrivals
            .iter()
            .filter(|(s, a)| s.window_close.is_some_and(|close| a.date <= close))
            .count();
        let next_window = arrivals
            .iter()
            .filter(|(s, a)| {
                s.window_close.is_none_or(|close| {
                    a.date > close && (a.date - close).num_days() <= Self::NEXT_WINDOW_DAYS
                })
            })
            .count();
        println!(
            "replaced in the same window {:.1}% | the next {:.1}% | later {:.1}% | not by the end of the run {:.1}%",
            Self::share(same_window, n),
            Self::share(next_window, n),
            Self::share(arrivals.len() - same_window - next_window, n),
            Self::share(n - arrivals.len(), n),
        );
        println!(
            "the replacement against the man who left (n={}): perceived level {:+.0} median, age {:+.0} median, \
             same shirt {:.1}%, level within five of his {:.1}%",
            arrivals.len(),
            Self::median(
                arrivals
                    .iter()
                    .map(|(s, a)| f64::from(a.level) - f64::from(s.level))
                    .collect()
            ),
            Self::median(
                arrivals
                    .iter()
                    .map(|(s, a)| f64::from(a.age) - f64::from(s.age))
                    .collect()
            ),
            Self::share(arrivals.iter().filter(|(s, a)| a.shirt == s.shirt).count(), arrivals.len()),
            Self::share(
                arrivals
                    .iter()
                    .filter(|(s, a)| a.level.saturating_add(5) >= s.level)
                    .count(),
                arrivals.len()
            ),
        );
        println!(
            "share of the fee spent on the replacement: median {:.0}%; arrived for no fee {:.1}%",
            Self::median(arrivals.iter().map(|(s, a)| 100.0 * a.fee / s.fee).collect()),
            Self::share(arrivals.iter().filter(|(_, a)| a.fee <= 0.0).count(), arrivals.len()),
        );

        let promoted = followed.iter().filter(|s| s.promotions > 0).count();
        println!(
            "promotions in his line inside the club on the day of the sale: {} over {} sales ({:.1}% of sales)",
            followed.iter().map(|s| s.promotions).sum::<u32>(),
            promoted,
            Self::share(promoted, n)
        );

        let against_fee = |read: fn(&KeySale) -> Option<f64>| -> String {
            let moved: Vec<f64> = followed
                .iter()
                .filter_map(|s| read(s).map(|budget| 100.0 * (budget - s.budget_before) / s.fee))
                .collect();
            format!("median {:+.0}% of the fee (n={})", Self::median(moved.clone()), moved.len())
        };
        println!(
            "transfer budget against the day before the sale: the day after {}; on the first of the next month {}",
            against_fee(|s| s.budget_day_after),
            against_fee(|s| s.budget_next_month),
        );

        // Before the sale: what the seller's readiness changed.
        let windows = (f64::from(self.last_day) / 365.0 * 2.0).max(1.0);
        println!(
            "first-choice sales per window: {:.0} ({} over {:.1} windows)",
            sales.len() as f64 / windows,
            sales.len(),
            windows
        );
        let late: Vec<&&KeySale> = followed
            .iter()
            .filter(|s| {
                s.window_close.is_some_and(|close| {
                    let left = (close - s.date).num_days();
                    (0..=7).contains(&left)
                })
            })
            .collect();
        let late_unreplaced = late
            .iter()
            .filter(|s| {
                !s.arrival
                    .as_ref()
                    .is_some_and(|a| s.window_close.is_some_and(|close| a.date <= close))
            })
            .count();
        let late_let_go = late.iter().filter(|s| s.let_go).count();
        println!(
            "  in the last week of a window: {} sales ({} the club was selling or he had asked to go),              left unreplaced in that window {} ({:.1}%)",
            late.len(),
            late_let_go,
            late_unreplaced,
            Self::share(late_unreplaced, late.len())
        );
        let mut fees: Vec<f64> = followed.iter().map(|s| s.fee).collect();
        fees.sort_by(f64::total_cmp);
        let large = fees.get(fees.len() * 9 / 10).copied().unwrap_or(f64::MAX);
        let after_large: Vec<f64> = followed
            .iter()
            .filter(|s| s.fee >= large)
            .map(|s| {
                self.purchases
                    .get(&s.club_id)
                    .map(|paid| {
                        paid.iter()
                            .filter(|(day, _)| *day > s.day && *day <= s.day + 14)
                            .map(|(_, fee)| fee)
                            .sum::<f64>()
                    })
                    .unwrap_or(0.0)
            })
            .collect();
        println!(
            "  fees a club paid in the fortnight after a large sale (top tenth, {:.1}M or more): mean {:.2}M over {} sales",
            large / 1e6,
            after_large.iter().sum::<f64>() / after_large.len().max(1) as f64 / 1e6,
            after_large.len()
        );

        let concerned: Vec<&&KeySale> = followed.iter().filter(|s| s.concern_day.is_some()).collect();
        let on_the_day = concerned.iter().filter(|s| s.concern_day == Some(s.day)).count();
        let waited: Vec<f64> = concerned
            .iter()
            .filter_map(|s| s.concern_day.map(|day| f64::from(day - s.day)))
            .collect();
        let answered = concerned
            .iter()
            .filter(|s| {
                s.arrival.as_ref().is_some_and(|a| {
                    s.concern_day.is_some_and(|day| a.day <= day)
                        && a.level.saturating_add(Self::ABOUT_HIS_LEVEL) >= s.level
                        && PitchLine::of(a.shirt) == PitchLine::of(s.shirt)
                })
            })
            .count();
        let premature = concerned
            .iter()
            .filter(|s| {
                s.arrival
                    .as_ref()
                    .is_some_and(|a| a.day - s.day <= Self::GRACE_DAYS)
            })
            .count();
        println!(
            "team-mates recorded \"sold and not replaced\": {} sales ({:.1}%), {} team-mates; on the day of the sale {}; days from the sale to the concern: median {:.0}",
            concerned.len(),
            Self::share(concerned.len(), n),
            concerned.iter().map(|s| s.concerned).sum::<u32>(),
            on_the_day,
            Self::median(waited),
        );
        println!(
            "  a signing on his line of the pitch within {} of his level had already arrived when it was recorded: {}; a signing for his line of any level arrived within {} days of the sale: {}",
            Self::ABOUT_HIS_LEVEL,
            answered,
            Self::GRACE_DAYS,
            premature
        );
    }
}
