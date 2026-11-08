//! Why a man is on the list, whether a buyer can see him, how it ends, and
//! which loans of established men get past their club.

use crate::reading::{ClubRanks, MoveKind, MoveRow, Snap, Tier};
use chrono::{Datelike, NaiveDate};
use core::transfers::loan::guard::LoanAssetGuard;
use core::transfers::loan::ParentWillingness;
use core::transfers::market::{TransferListingStatus, TransferListingType};
use core::transfers::squad::ledger::{LedgerContext, LedgerPressure};
use core::{Person, Player, PlayerStatusType, SimulatorData, TeamType};
use std::collections::{BTreeMap, HashMap, HashSet};

const REASON_WAIT_DAYS: u32 = 7;
const LONG_ON_THE_LIST_DAYS: u32 = 365;
const STARTING_SHARE: f32 = 0.5;
const ESTABLISHED_AGE: u8 = 24;
const NAMED: usize = 25;
const RELISTED_WITHIN_DAYS: u32 = 180;

struct Stretch {
    since: u32,
    last_seen: u32,
    club: u32,
    tier: Tier,
    starting_at_start: bool,
    flag_at_start: bool,
    reason: Option<String>,
    days: u32,
    starting_days: u32,
    weeks_read: u32,
    weeks_no_row: u32,
    main_days: u32,
    name: String,
    ca: u8,
    age: u8,
    status: &'static str,
    days_left: i32,
}

#[derive(Clone, Copy)]
struct LoanRead {
    key_man: bool,
    first_choice: bool,
    standing: f32,
    score: f32,
    hold: f32,
    minutes: f32,
    req: bool,
    loa: bool,
    lst: bool,
}

#[derive(Default)]
pub struct ListingWatch {
    live_sale: HashSet<u32>,
    open: HashMap<u32, Stretch>,
    started: BTreeMap<(Tier, bool, String), u32>,
    ended: BTreeMap<(Tier, bool, &'static str), Vec<u32>>,
    loan_reads: HashMap<u32, LoanRead>,
    loans: BTreeMap<(Tier, &'static str, bool, bool, bool, bool), u32>,
    named_loans: Vec<(u8, String)>,
    came_off: HashMap<u32, (u32, bool)>,
    relisted: BTreeMap<(bool, String), u32>,
    pressure: BTreeMap<(Tier, &'static str, u8), u32>,
    loans_from: HashMap<(u32, u32), u8>,
    serial: BTreeMap<(Tier, u8, u8), u32>,
    named_serial: Vec<(u8, String)>,
    fame: BTreeMap<(i32, &'static str), (u32, u64, u32)>,
    bigger_stage_moves: BTreeMap<(Tier, &'static str), u32>,
    loan_levels: BTreeMap<(Tier, &'static str, &'static str), u32>,
    named_loans_down: Vec<(u8, String)>,
    named_bigger_stage_down: Vec<(u8, String)>,
}

impl ListingWatch {
    pub fn on_week(&mut self, data: &SimulatorData, ranks: &ClubRanks, date: NaiveDate) {
        self.live_sale.clear();
        for continent in &data.continents {
            for country in &continent.countries {
                for l in &country.transfer_market.listings {
                    if l.listing_type == TransferListingType::Transfer
                        && matches!(
                            l.status,
                            TransferListingStatus::Available | TransferListingStatus::InNegotiation
                        )
                    {
                        self.live_sale.insert(l.player_id);
                    }
                }
                for club in &country.clubs {
                    if Tier::of_rank(ranks.of(club.id)) > Tier::Top {
                        continue;
                    }
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            if p.is_on_loan() || p.age(date) < ESTABLISHED_AGE {
                                continue;
                            }
                            let Some(guard) = LoanAssetGuard::for_player(club, p, date, 0, 0)
                            else {
                                continue;
                            };
                            let w = LoanAssetGuard::willingness_for(club, p, date);
                            self.loan_reads.insert(
                                p.id,
                                LoanRead {
                                    key_man: guard.key_man(),
                                    first_choice: guard.first_choice(),
                                    standing: guard.standing(),
                                    score: w.score,
                                    hold: w.starter_hold,
                                    minutes: w.minutes,
                                    req: p.statuses.has(PlayerStatusType::Req),
                                    loa: p.statuses.has(PlayerStatusType::Loa),
                                    lst: p
                                        .contract
                                        .as_ref()
                                        .is_some_and(|c| c.is_transfer_listed)
                                        || p.statuses.has(PlayerStatusType::Lst),
                                },
                            );
                        }
                    }
                }
            }
        }
        for s in self.open.values_mut() {
            s.weeks_read += 1;
        }
        let live_sale = &self.live_sale;
        for (id, s) in self.open.iter_mut() {
            if !live_sale.contains(id) {
                s.weeks_no_row += 1;
            }
        }
    }

    pub fn on_player(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        day: u32,
        date: NaiveDate,
        rows: Option<&Vec<MoveRow>>,
        main_team: bool,
    ) {
        if before.club_id != now.club_id {
            self.on_club_change(p, before, now, day, date, rows);
        }
        if now.listed && now.club_id != 0 && !now.on_loan && !self.open.contains_key(&p.id) {
            if let Some((ended, starter)) = self.came_off.remove(&p.id)
                && day - ended <= RELISTED_WITHIN_DAYS
            {
                let reason = p
                    .decision_history
                    .items
                    .iter()
                    .rev()
                    .find(|d| d.decision.starts_with("dec_reason"))
                    .map_or("?".to_string(), |d| d.decision.clone());
                *self.relisted.entry((starter, reason)).or_insert(0) += 1;
            }
            self.open.insert(
                p.id,
                Stretch {
                    since: day,
                    last_seen: day,
                    club: now.club_id,
                    tier: now.tier(),
                    starting_at_start: now.starting,
                    flag_at_start: p.contract.as_ref().is_some_and(|c| c.is_transfer_listed),
                    reason: None,
                    days: 0,
                    starting_days: 0,
                    weeks_read: 0,
                    weeks_no_row: 0,
                    main_days: 0,
                    name: p.full_name.to_string(),
                    ca: now.ca,
                    age: now.age,
                    status: now.status,
                    days_left: now.days_left,
                },
            );
        }
        let Some(s) = self.open.get_mut(&p.id) else {
            return;
        };
        s.last_seen = day;
        s.days += 1;
        if now.starter >= STARTING_SHARE {
            s.starting_days += 1;
        }
        if main_team {
            s.main_days += 1;
        }
        s.ca = now.ca;
        s.days_left = now.days_left;
        if s.reason.is_none() {
            let waited = day - s.since;
            let row = p
                .decision_history
                .items
                .iter()
                .rev()
                .take_while(|d| (date - d.date).num_days() <= waited as i64)
                .find(|d| d.decision.starts_with("dec_reason"));
            if let Some(row) = row {
                s.reason = Some(row.decision.clone());
            } else if waited >= REASON_WAIT_DAYS || !now.listed {
                s.reason = Some(if s.flag_at_start {
                    "(flag, no register row)".to_string()
                } else {
                    "(badge, no register row)".to_string()
                });
            }
            if let Some(reason) = s.reason.clone() {
                *self
                    .started
                    .entry((s.tier, s.starting_at_start, reason))
                    .or_insert(0) += 1;
            }
        }
        if !now.listed {
            let finished = self.open.remove(&p.id).unwrap();
            self.came_off
                .insert(p.id, (day, finished.starting_days * 2 >= finished.days.max(1)));
            self.close(finished, "came off the list");
        }
    }

    fn on_club_change(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        day: u32,
        date: NaiveDate,
        rows: Option<&Vec<MoveRow>>,
    ) {
        let loan_row = rows.and_then(|r| r.iter().find(|x| x.kind == MoveKind::Loan));
        if let Some(s) = self.open.remove(&p.id) {
            let fee = rows.map_or(0.0, |r| r.iter().map(|x| x.fee).fold(0.0, f64::max));
            let how = if loan_row.is_some() || now.on_loan {
                "lent"
            } else if now.club_id == 0 {
                "left for nothing"
            } else if fee > 0.0 {
                "sold"
            } else {
                "free move"
            };
            self.close(s, how);
        }
        if now.on_loan && !before.on_loan {
            self.on_lent(p, before, day, date, loan_row);
            self.on_lent_at_level(p, before, now, date, loan_row);
        }
        let asked_for_a_bigger_stage = before.req
            && before
                .req_reason
                .is_some_and(|reason| reason.asks_for_a_bigger_stage());
        if asked_for_a_bigger_stage && !now.on_loan && !before.on_loan && now.club_id != 0 {
            let up_club = now.club_rep >= before.club_rep;
            let down_league = (now.league_rep as i32) < before.league_rep as i32 - 1_000;
            let kind = match (up_club, down_league, before.asks_only_bigger) {
                (true, false, _) => "to a bigger club",
                (true, true, _) => "bigger club, weaker league",
                (false, false, _) => "smaller club, same level",
                (false, true, true) => "smaller, weaker, asked only that",
                (false, true, false) => "smaller, weaker, other reasons",
            };
            *self.bigger_stage_moves.entry((before.tier(), kind)).or_insert(0) += 1;
            if !up_club && down_league {
                let fee = rows.map_or(0.0, |r| r.iter().map(|x| x.fee).fold(0.0, f64::max));
                self.named_bigger_stage_down.push((
                    before.ca,
                    format!(
                        "{date} {} {}y CA{} {} rank {} lg{} -> rank {} lg{} fee {:.1}M wage {:.1}M -> {:.1}M start {:.2} first reason {:?} only bigger {} arc {:?}",
                        p.full_name,
                        before.age,
                        before.ca,
                        before.status,
                        before.rank,
                        before.league_rep,
                        now.rank,
                        now.league_rep,
                        fee / 1e6,
                        before.salary as f64 / 1e6,
                        now.salary as f64 / 1e6,
                        before.starter,
                        before.req_reason,
                        before.asks_only_bigger,
                        before.arc,
                    ),
                ));
            }
        }
        if !now.on_loan || before.on_loan || before.age < ESTABLISHED_AGE {
            return;
        }
        let tier = before.tier();
        if tier > Tier::Top || !matches!(before.status, "Key" | "Regular") {
            return;
        }
        let read = self.loan_reads.get(&p.id).copied();
        let key_man = read.is_some_and(|r| r.key_man);
        let at_key_level = read.is_some_and(|r| r.standing >= 1.0);
        *self
            .loans
            .entry((
                tier,
                before.status,
                key_man,
                at_key_level,
                before.req || read.is_some_and(|r| r.req),
                before.listed || read.is_some_and(|r| r.loa),
            ))
            .or_insert(0) += 1;
        let reason = loan_row.map_or("?", |r| r.reason.as_str());
        let mut text = format!(
            "{date} {} {}y CA{} rank {} {} start {:.2} lst{} req{} arc {:?} via {reason} day {day}",
            p.full_name,
            before.age,
            before.ca,
            before.rank,
            before.status,
            before.starter,
            before.listed as u8,
            before.req as u8,
            before.arc,
        );
        if let Some(r) = read {
            text.push_str(&format!(
                " | key_man {} first {} standing {:.2} score {:.2} hold {:.2} minutes {:.2} loa {} lst {} req {}",
                r.key_man, r.first_choice, r.standing, r.score, r.hold, r.minutes, r.loa, r.lst, r.req
            ));
        }
        let history: Vec<String> = p
            .decision_history
            .items
            .iter()
            .rev()
            .take(4)
            .map(|d| format!("{} {} {}", d.date, d.movement, d.decision))
            .collect();
        text.push_str(&format!(" | stage {:?} | {}", p.plan.as_ref().map(|plan| plan.stage), history.join(" ; ")));
        self.named_loans.push((before.ca, text));
    }

    /// Every loan out, counted against the club that lent him and his career.
    fn on_lent(
        &mut self,
        p: &Player,
        before: &Snap,
        day: u32,
        date: NaiveDate,
        loan_row: Option<&MoveRow>,
    ) {
        let from_parent = self.loans_from.entry((p.id, before.club_id)).or_insert(0);
        *from_parent = from_parent.saturating_add(1);
        let from_parent = *from_parent;
        let career = p.statistics_history.loan_spells_at_current_club();
        let band = crate::reading::AgeBand::of(before.age);
        *self
            .serial
            .entry((before.tier(), band, from_parent.min(4)))
            .or_insert(0) += 1;
        if from_parent >= 3 || (career >= 3 && before.age >= ESTABLISHED_AGE) {
            let plan = p.plan.as_ref();
            let history: Vec<String> = p
                .decision_history
                .items
                .iter()
                .rev()
                .take(5)
                .map(|d| format!("{} {} {}", d.date, d.movement, d.decision))
                .collect();
            self.named_serial.push((
                before.ca,
                format!(
                    "{date} {} {}y CA{} {} rank {} {} start {:.2} | loan {from_parent} from this club, {career} in his record, plan loans_used {:?} stage {:?} purpose {:?} | via {} arc {:?} req{} lst{} day {day} | {}",
                    p.full_name,
                    before.age,
                    before.ca,
                    before.tier().label(),
                    before.rank,
                    before.status,
                    before.starter,
                    plan.map(|plan| plan.loans_used),
                    plan.map(|plan| plan.stage),
                    plan.and_then(|plan| plan.loan_purpose),
                    loan_row.map_or("?", |r| r.reason.as_str()),
                    before.arc,
                    before.req as u8,
                    before.listed as u8,
                    history.join(" ; "),
                ),
            ));
        }
    }

    fn on_lent_at_level(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        date: NaiveDate,
        loan_row: Option<&MoveRow>,
    ) {
        if before.age < ESTABLISHED_AGE || !matches!(before.tier(), Tier::Elite | Tier::Top) {
            return;
        }
        let drop = before.league_rep as i32 - now.league_rep as i32;
        let level = if drop >= 2_000 {
            "a division or more down"
        } else if drop >= 1_000 {
            "a weaker league"
        } else {
            "same level or up"
        };
        let ability = match before.ca {
            0..=120 => "CA <=120",
            121..=140 => "CA 121-140",
            141..=160 => "CA 141-160",
            _ => "CA 161+",
        };
        *self.loan_levels.entry((before.tier(), ability, level)).or_insert(0) += 1;
        if drop >= 2_000 && before.ca > 140 {
            self.named_loans_down.push((
                before.ca,
                format!(
                    "{date} {} {}y CA{} {} rank {} lg{} -> rank {} lg{} start {:.2} via {} arc {:?} | read after the move: drought {:.2} floor {:.0}",
                    p.full_name,
                    before.age,
                    before.ca,
                    before.status,
                    before.rank,
                    before.league_rep,
                    now.rank,
                    now.league_rep,
                    before.starter,
                    loan_row.map_or("?", |r| r.reason.as_str()),
                    before.arc,
                    p.football_drought(date),
                    p.level_floor(date, before.league_rep).floor(),
                ),
            ));
        }
    }

    fn close(&mut self, s: Stretch, how: &'static str) {
        let was_starter = s.starting_days * 2 >= s.days.max(1);
        self.ended
            .entry((s.tier, was_starter, how))
            .or_default()
            .push(s.days);
    }

    pub fn report(&self, data: &SimulatorData, day: u32) {
        println!("\n-- the list --");
        self.report_flag_without_row(data);
        println!("  listed, by tier, starting when listed, reason (top-250 clubs and every starter):");
        let mut started: Vec<_> = self
            .started
            .iter()
            .filter(|((tier, starting, _), _)| *starting || *tier <= Tier::Top)
            .collect();
        started.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for ((tier, starting, reason), count) in started.iter().take(40) {
            println!(
                "    {:<14} {:<9} {reason:<40} {count:>6}",
                tier.label(),
                if *starting { "starting" } else { "not" },
            );
        }
        let mut relisted: Vec<_> = self.relisted.iter().collect();
        relisted.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        println!(
            "  back on the list within {RELISTED_WITHIN_DAYS}d of coming off it: {} (starter for most of the first stretch, reason):",
            relisted.iter().map(|(_, c)| **c).sum::<u32>()
        );
        for ((starter, reason), count) in relisted.iter().take(12) {
            println!("    {starter:<5} {reason:<40} {count:>6}");
        }
        println!("  loans of men aged 24+ from top-250 clubs, by ability and the borrower's level:");
        for ((tier, ability, level), count) in &self.loan_levels {
            println!("    {:<14} {ability:<11} {level:<26} {count:>6}", tier.label());
        }
        let mut down = self.named_loans_down.clone();
        down.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        for (_, text) in down.iter().take(15) {
            println!("    {text}");
        }
        println!("  permanent moves of men whose request asked for a bigger stage, by the tier he left:");
        for ((tier, kind), count) in &self.bigger_stage_moves {
            println!("    {:<14} {kind:<28} {count:>6}", tier.label());
        }
        let mut down = self.named_bigger_stage_down.clone();
        down.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        for (_, text) in down.iter().take(12) {
            println!("    {text}");
        }
        println!("  reputation on the first of August: mean current reputation, and men at 9,500 or more:");
        for ((year, band), (n, sum, capped)) in &self.fame {
            println!(
                "    {year} {band:<11} n {n:>6}  mean {:>5}  at 9500+ {capped:>5} ({:.1}%)",
                sum / (*n).max(1) as u64,
                *capped as f64 * 100.0 / (*n).max(1) as f64
            );
        }
        println!("  club pressure on the first of August, by tier, share of clubs at 0, up to .5, above .5:");
        for ((tier, kind, bucket), count) in &self.pressure {
            println!("    {:<14} {kind:<6} {bucket} {count:>6}", tier.label());
        }
        println!("  how a stretch on the list ended, by tier, starter for most of it:");
        for ((tier, starter, how), spans) in &self.ended {
            let mut spans = spans.clone();
            spans.sort_unstable();
            println!(
                "    {:<14} {:<9} {how:<18} {:>6}  median {:>4}d  p90 {:>4}d",
                tier.label(),
                if *starter { "starter" } else { "not" },
                spans.len(),
                spans[spans.len() / 2],
                spans[spans.len() * 9 / 10],
            );
        }
        let mut long: Vec<&Stretch> = self
            .open
            .values()
            .filter(|s| {
                day - s.last_seen <= 2
                    && s.days >= LONG_ON_THE_LIST_DAYS
                    && s.starting_days * 2 >= s.days
            })
            .collect();
        let long_top = long.iter().filter(|s| s.tier <= Tier::Top).count();
        println!(
            "  still listed after {LONG_ON_THE_LIST_DAYS}d while starting most of it: {} ({} at top-250 clubs)",
            long.len(),
            long_top,
        );
        long.sort_by_key(|s| std::cmp::Reverse(s.ca));
        self.report_long(data, &long);
        println!(
            "  loans of Key and Regular men aged {ESTABLISHED_AGE}+ at top-250 clubs (ENTERTAINS {:.2}):",
            ParentWillingness::ENTERTAINS
        );
        println!("    tier status key_man at-key-level asked listed  count");
        let mut loans: Vec<_> = self.loans.iter().collect();
        loans.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for ((tier, status, key_man, at_key, asked, listed), count) in loans {
            println!(
                "    {:<14} {status:<8} key_man={key_man:<5} at_key={at_key:<5} asked={asked:<5} listed={listed:<5} {count:>5}",
                tier.label(),
            );
        }
        println!("  every loan out, by the parent's tier, age band, and which loan from that club it was (4 = fourth or later):");
        for ((tier, band, nth), count) in &self.serial {
            println!(
                "    {:<14} {:<6} loan {nth} {count:>6}",
                tier.label(),
                crate::reading::AgeBand::label(*band)
            );
        }
        let mut serial = self.named_serial.clone();
        serial.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        println!("  third or later loans from one club, or a record of three, at 24+: {}", serial.len());
        for (_, text) in serial.iter().take(NAMED) {
            println!("    {text}");
        }
        let mut named = self.named_loans.clone();
        named.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        for (_, text) in named.iter().take(NAMED) {
            println!("    {text}");
        }
    }

    /// What a long stretch is standing on: the man now and his row.
    fn report_long(&self, data: &SimulatorData, long: &[&Stretch]) {
        let date = data.date.date();
        let ids: HashMap<u32, &Stretch> = self
            .open
            .iter()
            .filter(|(_, s)| long.iter().any(|l| std::ptr::eq(*l, *s)))
            .map(|(id, s)| (*id, s))
            .collect();
        let mut table: BTreeMap<(Tier, String, &'static str, &'static str, &'static str), u32> =
            BTreeMap::new();
        let mut lines: Vec<(u8, String)> = Vec::new();
        let mut no_row: Vec<(u8, String)> = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                let market = &country.transfer_market;
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            let Some(s) = ids.get(&p.id) else { continue };
                            let row = market.listings.iter().find(|l| {
                                l.player_id == p.id
                                    && l.listing_type == TransferListingType::Transfer
                                    && matches!(
                                        l.status,
                                        TransferListingStatus::Available
                                            | TransferListingStatus::InNegotiation
                                    )
                            });
                            let loan_row = market.listings.iter().any(|l| {
                                l.player_id == p.id
                                    && l.listing_type == TransferListingType::Loan
                                    && matches!(
                                        l.status,
                                        TransferListingStatus::Available
                                            | TransferListingStatus::InNegotiation
                                    )
                            });
                            let flag = p.contract.as_ref().is_some_and(|c| c.is_transfer_listed);
                            let asking = if p.statuses.has(PlayerStatusType::Req) {
                                "req"
                            } else if p.is_asking_to_leave(date) {
                                "unh"
                            } else {
                                "no"
                            };
                            let row_kind = match row {
                                Some(_) => "sale row",
                                None if loan_row => "loan row",
                                None => "no row",
                            };
                            let squad = if team.team_type == TeamType::Main { "main" } else { "reserve" };
                            if row_kind == "no row" && squad == "main" {
                                let history: Vec<String> = p
                                    .decision_history
                                    .items
                                    .iter()
                                    .rev()
                                    .take(10)
                                    .map(|d| format!("{} {} {}", d.date, d.movement, d.decision))
                                    .collect();
                                let team_index = club
                                    .teams
                                    .teams
                                    .iter()
                                    .position(|t| std::ptr::eq(t, team))
                                    .unwrap_or(99);
                                let candidate = club
                                    .transfer_plan
                                    .loan_out_candidates
                                    .iter()
                                    .find(|c| c.player_id == p.id)
                                    .map(|c| format!("{:?}/{}", c.status, c.from_pathway));
                                let entertains = LoanAssetGuard::willingness_for(club, p, date).score
                                    >= ParentWillingness::ENTERTAINS;
                                let plan = p.plan.as_ref();
                                no_row.push((
                                    s.ca,
                                    format!(
                                        "{} {}y CA{} {} {} {} {}d listed as {}, starting {}d, no row {}/{}w, {}d left | kept {} picked {} flag {} lst {} req {} unh {:?} loa {} team#{} window {} | protected {} season {:?} on_schedule {:?} candidate {:?} entertains {} moved {:?} stage {:?} | {}",
                                        s.name,
                                        s.age,
                                        s.ca,
                                        s.status,
                                        club.name,
                                        s.tier.label(),
                                        s.days,
                                        s.reason.as_deref().unwrap_or("?"),
                                        s.starting_days,
                                        s.weeks_no_row,
                                        s.weeks_read,
                                        s.days_left,
                                        club.transfer_plan.kept_in_the_side.contains(&p.id),
                                        p.is_picked(date),
                                        flag,
                                        p.statuses.has(PlayerStatusType::Lst),
                                        p.statuses.has(PlayerStatusType::Req),
                                        p.statuses.held_for_days(PlayerStatusType::Unh, date),
                                        p.statuses.has(PlayerStatusType::Loa),
                                        team_index,
                                        market.transfer_window_open,
                                        p.signing_protection_active(date),
                                        plan.map(|pl| pl.season_index(date)),
                                        plan.map(|pl| pl.on_schedule(
                                            date,
                                            p.happiness.starter_ratio,
                                            p.happiness.appearances_tracked
                                        )),
                                        candidate,
                                        entertains,
                                        p.last_transfer_date(),
                                        p.plan.as_ref().map(|plan| plan.stage),
                                        history.join(" ; "),
                                    ),
                                ));
                            }
                            *table
                                .entry((
                                    s.tier,
                                    s.reason.clone().unwrap_or_else(|| "?".to_string()),
                                    asking,
                                    row_kind,
                                    squad,
                                ))
                                .or_insert(0) += 1;
                            let value = p.value(date, 0, 0);
                            let row_text = row.map_or("no sale row".to_string(), |l| {
                                format!(
                                    "{:?} ask {:.1}M of {:.1}M exposure {}d best bid {:.1}M",
                                    l.origin,
                                    l.asking_price.amount / 1e6,
                                    l.original_asking_price.amount / 1e6,
                                    l.exposure_days,
                                    l.best_rejected_bid.unwrap_or(0.0) / 1e6,
                                )
                            });
                            lines.push((
                                s.ca,
                                format!(
                                    "{} {}y CA{} {} {} {} {squad} {}d listed, starting {}d, main {}d, no row {}/{}w, {}d left, {} | asking {asking} flag {flag} {row_kind} value {:.1}M {row_text} reasons {:?} arc {:?}",
                                    s.name,
                                    s.age,
                                    s.ca,
                                    s.status,
                                    club.name,
                                    s.tier.label(),
                                    s.days,
                                    s.starting_days,
                                    s.main_days,
                                    s.weeks_no_row,
                                    s.weeks_read,
                                    s.days_left,
                                    s.reason.as_deref().unwrap_or("?"),
                                    value / 1e6,
                                    p.transfer_request_reasons,
                                    p.mind.career.plan.map(|plan| plan.arc),
                                ),
                            ));
                        }
                    }
                }
            }
        }
        let mut table: Vec<_> = table.into_iter().collect();
        table.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        println!("    tier, reason listed, asking now, row, squad:");
        for ((tier, reason, asking, row, squad), count) in table.iter().take(40) {
            println!(
                "    {:<14} {reason:<36} {asking:<4} {row:<9} {squad:<8} {count:>5}",
                tier.label()
            );
        }
        lines.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        for (_, line) in lines.iter().take(NAMED) {
            println!("    {line}");
        }
        no_row.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
        println!(
            "    on the first team with no sale row: {}; kept, picked, last decisions:",
            no_row.len()
        );
        for (_, line) in no_row.iter().take(40) {
            println!("      {line}");
        }
    }

    pub fn on_day(&mut self, data: &SimulatorData, ranks: &ClubRanks, date: NaiveDate) {
        if date.month() == 8 && date.day() == 1 {
            println!("\n-- the list on {date}, with windows open --");
            self.report_flag_without_row(data);
            for continent in &data.continents {
                for country in &continent.countries {
                    for club in &country.clubs {
                        for team in &club.teams.teams {
                            for p in &team.players.players {
                                let ca = p.player_attributes.current_ability;
                                let age = p.age(date);
                                let ability = match ca {
                                    0..=100 => "CA <=100",
                                    101..=130 => "CA 101-130",
                                    131..=160 => "CA 131-160",
                                    _ => "CA 161+",
                                };
                                let years = match age {
                                    0..=29 => "age <=29",
                                    30..=33 => "age 30-33",
                                    _ => "age 34+",
                                };
                                let rep = p.player_attributes.current_reputation.max(0) as u64;
                                for band in [ability, years] {
                                    let row = self.fame.entry((date.year(), band)).or_insert((0, 0, 0));
                                    row.0 += 1;
                                    row.1 += rep;
                                    if rep >= 9_500 {
                                        row.2 += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            for continent in &data.continents {
                for country in &continent.countries {
                    for club in &country.clubs {
                        let tier = Tier::of_rank(ranks.of(club.id));
                        let pressure = LedgerPressure::of(&LedgerContext::of(club));
                        for (kind, value) in
                            [("wage", pressure.wage_pressure), ("cash", pressure.cash_need)]
                        {
                            let bucket = if value <= 0.0 {
                                0
                            } else if value <= 0.5 {
                                1
                            } else {
                                2
                            };
                            *self.pressure.entry((tier, kind, bucket)).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }

    fn report_flag_without_row(&self, data: &SimulatorData) {
        let date = data.date.date();
        let mut causes: BTreeMap<&'static str, (u32, u32, u32)> = BTreeMap::new();
        let mut named: BTreeMap<&'static str, Vec<(u8, String)>> = BTreeMap::new();
        let mut flags = 0u32;
        let mut badges_only = 0u32;
        for continent in &data.continents {
            for country in &continent.countries {
                let market = &country.transfer_market;
                let mut sale: HashSet<u32> = HashSet::new();
                let mut loan: HashSet<u32> = HashSet::new();
                for l in &market.listings {
                    if !matches!(
                        l.status,
                        TransferListingStatus::Available | TransferListingStatus::InNegotiation
                    ) {
                        continue;
                    }
                    match l.listing_type {
                        TransferListingType::Transfer => {
                            sale.insert(l.player_id);
                        }
                        TransferListingType::Loan => {
                            loan.insert(l.player_id);
                        }
                        TransferListingType::EndOfContract => {}
                    }
                }
                for club in &country.clubs {
                    let reviewing = club
                        .transfer_plan
                        .manager_review_until
                        .is_some_and(|until| date < until);
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            let Some(contract) = p.contract.as_ref() else {
                                continue;
                            };
                            if sale.contains(&p.id) {
                                continue;
                            }
                            let flag = contract.is_transfer_listed;
                            let badge = p.statuses.has(PlayerStatusType::Lst);
                            if !flag && !badge {
                                continue;
                            }
                            if flag {
                                flags += 1;
                            } else {
                                badges_only += 1;
                            }
                            let main = team.team_type == TeamType::Main;
                            let cause = if p.is_on_loan() {
                                "out on loan"
                            } else if loan.contains(&p.id) {
                                "on the loan market instead"
                            } else if !market.transfer_window_open {
                                if main { "window shut, main" } else if badge { "window shut, reserve badge" } else { "window shut, reserve flag" }
                            } else if p.is_force_match_selection {
                                "manager pinned"
                            } else if reviewing {
                                "new manager reviewing"
                            } else if main && p.signing_protection_active(date) {
                                "signing protected"
                            } else if !main && badge {
                                "reserve with a badge"
                            } else if !main {
                                "reserve, window open"
                            } else {
                                "main squad, window open"
                            };
                            if matches!(cause, "main squad, window open" | "signing protected") {
                                let team_index = club
                                    .teams
                                    .teams
                                    .iter()
                                    .position(|t| std::ptr::eq(t, team))
                                    .unwrap_or(99);
                                let candidate = club
                                    .transfer_plan
                                    .loan_out_candidates
                                    .iter()
                                    .find(|c| c.player_id == p.id)
                                    .map(|c| format!("{:?}/{}", c.status, c.from_pathway));
                                let entertains = LoanAssetGuard::willingness_for(club, p, date).score
                                    >= ParentWillingness::ENTERTAINS;
                                let plan = p.plan.as_ref();
                                let history: Vec<String> = p
                                    .decision_history
                                    .items
                                    .iter()
                                    .rev()
                                    .take(8)
                                    .map(|d| format!("{} {} {}", d.date, d.movement, d.decision))
                                    .collect();
                                named.entry(cause).or_default().push((
                                    p.player_attributes.current_ability,
                                    format!(
                                        "{} {}y CA{} {:?} {} team#{} | flag {} lst {} req {} unh {} loa {} pinned {} | protected {} season {:?} on_schedule {:?} | candidate {:?} entertains {} kept {} picked {} start {:.2}/{} | {}",
                                        p.full_name,
                                        p.age(date),
                                        p.player_attributes.current_ability,
                                        contract.squad_status,
                                        club.name,
                                        team_index,
                                        flag,
                                        badge,
                                        p.statuses.has(PlayerStatusType::Req),
                                        p.statuses.has(PlayerStatusType::Unh),
                                        p.statuses.has(PlayerStatusType::Loa),
                                        p.is_force_match_selection,
                                        p.signing_protection_active(date),
                                        plan.map(|pl| pl.season_index(date)),
                                        plan.map(|pl| pl.on_schedule(
                                            date,
                                            p.happiness.starter_ratio,
                                            p.happiness.appearances_tracked
                                        )),
                                        candidate,
                                        entertains,
                                        club.transfer_plan.kept_in_the_side.contains(&p.id),
                                        p.is_picked(date),
                                        p.happiness.starter_ratio,
                                        p.happiness.appearances_tracked,
                                        history.join(" ; "),
                                    ),
                                ));
                            }
                            let row = causes.entry(cause).or_insert((0, 0, 0));
                            row.0 += 1;
                            let seen = p.happiness.appearances_tracked >= 6;
                            if main && seen && p.happiness.starter_ratio >= STARTING_SHARE {
                                row.1 += 1;
                            }
                            if p.contract.as_ref().is_some_and(|c| {
                                matches!(
                                    c.squad_status,
                                    core::PlayerSquadStatus::KeyPlayer
                                        | core::PlayerSquadStatus::FirstTeamRegular
                                )
                            }) {
                                row.2 += 1;
                            }
                        }
                    }
                }
            }
        }
        println!(
            "  on the list with no sale row today: {flags} flagged, {badges_only} badge only; by cause (all, starting, Key or Regular):"
        );
        for (cause, (all, starting, key)) in causes {
            println!("    {cause:<28} {all:>6} {starting:>6} {key:>6}");
        }
        for (cause, mut lines) in named {
            lines.sort_by_key(|(ca, _)| std::cmp::Reverse(*ca));
            println!("    {cause}, named:");
            for (_, line) in lines.iter().take(30) {
                println!("      {line}");
            }
        }
    }
}
