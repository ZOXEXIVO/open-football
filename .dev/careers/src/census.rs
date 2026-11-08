//! The moves a reader would question, counted across the whole world, each
//! with what it should be read against.

use crate::reading::{AgeBand, MoveKind, MoveRow, Place, Snap, Tier};
use chrono::{Datelike, NaiveDate};
use core::club::player::mind::{CareerArc, CareerDreamReview, ChapterKind, DreamKind, GoalKind};
use core::r#match::engine::player::injury::InjuryGrade;
use core::transfers::market::TransferListingOrigin;
use core::{Person, Player, SimulationResult, SimulatorData, TeamType};
use std::collections::{BTreeMap, HashMap, HashSet};

const RENEWAL_SIGNED: &str = "dec_contract_renewal_signed";
const WORST_CASES: usize = 15;
/// A year, for "within a year" rows.
const YEAR: u32 = 365;
/// The biggest fall the baseline census counted as a division or more.
const TOP_TIER_RANK: u32 = 250;
const CLUB_STEP_DOWN: f32 = 0.03;
const DIVISION_DROP: i32 = 2000;
const SHORT_REQUEST_DAYS: u32 = 14;
/// How long after his own request ended a listing reads as outliving it.
const LISTING_OUTLIVED_DAYS: u32 = 30;
const LISTING_REASON_WAIT_DAYS: u32 = 7;

type Cell = (Tier, bool, u8);

#[derive(Clone, Copy)]
enum RequestOutcome {
    Sold,
    Loaned,
    Free,
    Withdrawn,
    Retired,
    Open,
}

#[derive(Default)]
struct Rates {
    expiry: u32,
    pay_off: u32,
    other_free: u32,
    paid_off_starting: u32,
    sold: u32,
    dropped: u32,
    requests: u32,
    retired: u32,
}

#[derive(Clone, Copy)]
struct WantSpan {
    first_day: u32,
    last_seen: u32,
    club: u32,
    club_rep: f32,
    peak_pressure: f32,
}

#[derive(Default)]
struct WantStat {
    weeks: Vec<u32>,
    same_club: u32,
    moved_up: u32,
    moved_other: u32,
    voiced: u32,
}

#[derive(Default)]
struct ArcStat {
    weeks: Vec<u32>,
    same_club: u32,
    after_move: u32,
    same_club_then: HashMap<&'static str, u32>,
}

#[derive(Clone)]
struct RequestSpan {
    started: u32,
    club: u32,
    tier: Tier,
    regular: bool,
    first: String,
    month: u32,
}

#[derive(Clone, Copy, Default)]
struct PermanentMove {
    day: u32,
    from: u32,
}

/// One year's market and wage bill, by club size.
#[derive(Default, Clone)]
struct EconomyRow {
    day: u32,
    men: [u32; 4],
    reputation_sum: [f64; 4],
    wage_sum: [f64; 4],
    top_reputation: (i16, String),
    top_wage: (u32, String),
    fee_sum: [f64; 4],
    fee_count: [u32; 4],
    top_fee: (f64, String),
}

#[derive(Default)]
struct SeasonMoves {
    paid: u32,
    free_signings: u32,
    released: u32,
    loans: u32,
}

/// A line naming a man, ranked by how good he was.
struct Named {
    ca: u8,
    text: String,
}

/// Competitive results, against the bands the stub is calibrated to.
#[derive(Default)]
struct ResultsRow {
    matches: u32,
    goals: u32,
    home_goals: u32,
    draws: u32,
    goalless: u32,
    player_of_the_match_named: u32,
    yellows: u32,
    reds: u32,
    fouls: u32,
    hurt_or_worse: u32,
}

#[derive(Default)]
pub struct Census {
    pub snaps: HashMap<u32, Snap>,
    results: ResultsRow,
    exposure: HashMap<Cell, u64>,
    rates: BTreeMap<Cell, Rates>,
    named_exits: Vec<Named>,
    named_pay_offs_starting: Vec<Named>,
    named_drops: Vec<Named>,
    named_listed_after_request: Vec<Named>,
    named_retired_regulars: Vec<Named>,
    named_resigned: Vec<Named>,
    named_rival_moves: Vec<Named>,

    left_for_pool: HashMap<u32, (u32, u32)>,
    pool_entry: HashMap<u32, u32>,
    pool_stay_days: u64,
    pool_stays: u32,
    resigned_by_same_club: u32,
    last_permanent: HashMap<u32, PermanentMove>,
    two_moves_in_a_year: u32,
    /// Regulars of 29 or under at the top 250 clubs, a division down and to a
    /// smaller club too. Without the second half this also counts men joining
    /// a bigger club in a smaller league.
    fell_a_division_and_a_club: u32,
    fell_though_starting: u32,
    back_to_seller: u32,
    rival_moves: u32,
    moves_with_a_favourite: u32,

    open_requests: HashMap<u32, RequestSpan>,
    request_starts: u32,
    request_men: HashSet<u32>,
    request_spans: Vec<u32>,
    requests_no_reason: u32,
    request_reasons: BTreeMap<(Tier, bool, String), u32>,
    request_ended: HashMap<u32, (u32, u32)>,
    request_outcomes: BTreeMap<(Tier, bool, String), [u32; 6]>,
    refused_by_month: BTreeMap<u32, [u32; 6]>,
    refused_men: HashSet<u32>,
    refused_starts: u32,
    listed_after_request: u32,
    listed_by_the_club_after_request: u32,
    listed_while_still_unhappy: u32,
    listed_with_no_market_row: u32,
    listing_reasons: BTreeMap<(Tier, u8, String), u32>,
    listings_awaiting_reason: HashMap<u32, (u32, Tier, u8)>,
    listings_of_others: u32,
    renewals_signed: u32,

    retirements: u32,
    retired_regulars: u32,

    wants: HashMap<(u32, GoalKind), WantSpan>,
    want_stats: HashMap<GoalKind, WantStat>,
    arc_spans: HashMap<u32, (CareerArc, u32, u32)>,
    arc_stats: HashMap<&'static str, ArcStat>,

    economy: Vec<EconomyRow>,
    current_fees: EconomyRow,
    season_moves: BTreeMap<u32, SeasonMoves>,
    free_signing_reasons: BTreeMap<String, u32>,
    short_squads: Vec<(u32, u32, u32)>,
    first_day: Option<u32>,
}

impl Census {
    pub fn on_results(&mut self, result: &SimulationResult) {
        let row = &mut self.results;
        for m in result.match_results.iter().filter(|m| !m.friendly) {
            let (home, away) = (m.score.home_team.get() as u32, m.score.away_team.get() as u32);
            row.matches += 1;
            row.goals += home + away;
            row.home_goals += home;
            row.draws += (home == away) as u32;
            row.goalless += (home + away == 0) as u32;
            let Some(details) = &m.details else { continue };
            row.player_of_the_match_named += details.player_of_the_match_id.is_some() as u32;
            for line in details.player_stats.values() {
                row.yellows += line.yellow_cards as u32;
                row.reds += line.red_cards as u32;
                row.fouls += line.fouls as u32;
            }
            row.hurt_or_worse += details
                .physical_snapshots
                .values()
                .filter(|s| s.injury.is_some_and(|g| g != InjuryGrade::Knock))
                .count() as u32;
        }
    }

    pub fn on_market_rows(&mut self, day: u32, rows: &HashMap<u32, Vec<MoveRow>>, ranks: &crate::reading::ClubRanks) {
        let season = day / YEAR;
        let moves = self.season_moves.entry(season).or_default();
        for row in rows.values().flatten() {
            match row.kind {
                MoveKind::Loan => moves.loans += 1,
                _ if row.fee > 0.0 => moves.paid += 1,
                _ if row.to_club_id == 0 => moves.released += 1,
                _ => {
                    moves.free_signings += 1;
                    *self
                        .free_signing_reasons
                        .entry(row.reason.clone())
                        .or_insert(0) += 1;
                }
            }
            if row.fee > 0.0 {
                let tier = Tier::of_rank(ranks.of(row.to_club_id));
                if let Some(slot) = Tier::CLUBS.iter().position(|t| *t == tier) {
                    self.current_fees.fee_sum[slot] += row.fee;
                    self.current_fees.fee_count[slot] += 1;
                }
                if row.fee > self.current_fees.top_fee.0 {
                    self.current_fees.top_fee = (row.fee, row.player_name.clone());
                }
            }
        }
    }

    pub fn on_player(
        &mut self,
        p: &Player,
        now: &Snap,
        place: &Place,
        day: u32,
        date: NaiveDate,
        rows: Option<&Vec<MoveRow>>,
        data: &SimulatorData,
    ) {
        self.first_day.get_or_insert(day);
        if !now.on_loan && now.club_id != 0 {
            *self
                .exposure
                .entry((now.tier(), now.regular, AgeBand::of(now.age)))
                .or_insert(0) += 1;
        }
        let Some(before) = self.snaps.insert(p.id, *now) else {
            return;
        };

        self.on_request(p, &before, now, day, date);
        self.on_listing(p, &before, now, day, date, data);
        if p.decision_history
            .items
            .iter()
            .rev()
            .take_while(|d| d.date == date)
            .any(|d| d.decision == RENEWAL_SIGNED)
        {
            self.renewals_signed += 1;
        }
        if before.club_id != now.club_id {
            self.on_move(p, &before, now, place, day, date, rows, data);
        }
    }

    fn rate(&mut self, snap: &Snap) -> &mut Rates {
        self.rates
            .entry((snap.tier(), snap.regular, AgeBand::of(snap.age)))
            .or_default()
    }

    fn on_request(&mut self, p: &Player, before: &Snap, now: &Snap, day: u32, date: NaiveDate) {
        if now.req && !before.req {
            self.request_starts += 1;
            self.request_men.insert(p.id);
            self.rate(now).requests += 1;
            self.open_requests.insert(
                p.id,
                RequestSpan {
                    started: day,
                    club: now.club_id,
                    tier: now.tier(),
                    regular: now.regular,
                    first: p
                        .transfer_request_reasons
                        .first()
                        .map_or_else(|| "(none)".to_string(), |r| format!("{r:?}")),
                    month: date.month(),
                },
            );
            if self.open_requests[&p.id].first == "RefusedPlayingTime" {
                self.refused_starts += 1;
                self.refused_men.insert(p.id);
            }
            self.request_ended.remove(&p.id);
            if p.transfer_request_reasons.is_empty() {
                self.requests_no_reason += 1;
            }
            for reason in &p.transfer_request_reasons {
                *self
                    .request_reasons
                    .entry((now.tier(), now.regular, format!("{reason:?}")))
                    .or_insert(0) += 1;
            }
        } else if !now.req
            && before.req
            && before.club_id == now.club_id
            && let Some(span) = self.open_requests.remove(&p.id)
        {
            self.request_spans.push(day - span.started);
            if span.club == now.club_id {
                self.request_ended.insert(p.id, (day, now.club_id));
                self.on_request_outcome(span, RequestOutcome::Withdrawn);
            }
        }
    }

    fn on_listing(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        day: u32,
        date: NaiveDate,
        data: &SimulatorData,
    ) {
        if now.listed && !before.listed && before.club_id == now.club_id {
            if now.regular {
                self.listings_awaiting_reason.insert(p.id, (day, now.tier(), AgeBand::of(now.age)));
            } else {
                self.listings_of_others += 1;
            }
        }
        // The register's row can follow the flag by a day or more.
        if let Some((listed_on, tier, band)) = self.listings_awaiting_reason.get(&p.id).copied() {
            let waited = day - listed_on;
            let row = p
                .decision_history
                .items
                .iter()
                .rev()
                .take_while(|d| (date - d.date).num_days() <= waited as i64)
                .find(|d| d.decision.starts_with("dec_reason"));
            if let Some(row) = row {
                *self
                    .listing_reasons
                    .entry((tier, band, row.decision.clone()))
                    .or_insert(0) += 1;
                self.listings_awaiting_reason.remove(&p.id);
            } else if waited >= LISTING_REASON_WAIT_DAYS || !now.listed {
                *self
                    .listing_reasons
                    .entry((tier, band, "(flag with no register row)".to_string()))
                    .or_insert(0) += 1;
                self.listings_awaiting_reason.remove(&p.id);
            }
        }
        if let Some((ended, club)) = self.request_ended.get(&p.id).copied() {
            if club != now.club_id || !now.listed || now.req {
                self.request_ended.remove(&p.id);
            } else if day - ended >= LISTING_OUTLIVED_DAYS {
                self.request_ended.remove(&p.id);
                let row = data
                    .continents
                    .iter()
                    .flat_map(|continent| &continent.countries)
                    .find_map(|country| country.transfer_market.get_listing_by_player(p.id));
                let Some(row) = row else {
                    self.listed_with_no_market_row += 1;
                    return;
                };
                if row.origin != TransferListingOrigin::PlayerAsked {
                    self.listed_by_the_club_after_request += 1;
                    return;
                }
                if p.is_asking_to_leave(date) {
                    self.listed_while_still_unhappy += 1;
                    return;
                }
                self.listed_after_request += 1;
                self.named_listed_after_request.push(Named {
                    ca: now.ca,
                    text: format!(
                        "{} {}y CA{} {} start {:.2}, top want {:?}",
                        p.full_name, now.age, now.ca, now.status, now.starter, now.top_goal
                    ),
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_move(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        place: &Place,
        day: u32,
        date: NaiveDate,
        rows: Option<&Vec<MoveRow>>,
        data: &SimulatorData,
    ) {
        let fee = rows.map_or(0.0, |r| r.iter().map(|x| x.fee).fold(0.0, f64::max));
        let loan_leg = now.on_loan != before.on_loan
            || rows.is_some_and(|r| r.iter().any(|x| x.kind == MoveKind::Loan));
        if let Some(span) = self.open_requests.remove(&p.id) {
            self.request_spans.push(day - span.started);
            let outcome = if loan_leg {
                RequestOutcome::Loaned
            } else if fee > 0.0 {
                RequestOutcome::Sold
            } else {
                RequestOutcome::Free
            };
            self.on_request_outcome(span, outcome);
        }
        let reason = rows
            .and_then(|r| r.iter().find(|x| x.kind != MoveKind::Loan))
            .map_or("", |x| x.reason.as_str());

        let to_pool = before.club_id != 0 && now.club_id == 0;
        let direct = before.club_id != 0 && now.club_id != 0 && !loan_leg;

        if (to_pool || (direct && fee <= 0.0)) && !before.on_loan {
            self.on_left_for_nothing(p, before, place, to_pool, day, date, reason);
        }
        if to_pool {
            self.pool_entry.insert(p.id, day);
        }
        if before.club_id == 0 && now.club_id != 0 {
            if let Some(entered) = self.pool_entry.remove(&p.id) {
                self.pool_stay_days += (day - entered) as u64;
                self.pool_stays += 1;
            }
            if let Some((club, left_day)) = self.left_for_pool.remove(&p.id)
                && club == now.club_id
                && day - left_day <= YEAR
            {
                self.resigned_by_same_club += 1;
                self.named_resigned.push(Named {
                    ca: now.ca,
                    text: format!(
                        "{date} {} {}y CA{} back at {} after {}d",
                        p.full_name,
                        now.age,
                        now.ca,
                        place.club_name,
                        day - left_day
                    ),
                });
            }
        }

        if direct && !before.on_loan {
            if fee > 0.0 {
                self.rate(before).sold += 1;
            }
            let drop = before.league_rep as i32 - now.league_rep as i32;
            if before.regular && drop >= DIVISION_DROP {
                self.rate(before).dropped += 1;
                if before.rank < TOP_TIER_RANK
                    && before.age <= 29
                    && now.club_rep + CLUB_STEP_DOWN < before.club_rep
                {
                    self.fell_a_division_and_a_club += 1;
                    self.fell_though_starting += before.starting as u32;
                }
                self.named_drops.push(Named {
                    ca: before.ca,
                    text: format!(
                        "{date} {} {}y CA{} {} lg{} -> {} lg{} fee {fee:.0} wage {:.1}M -> {:.1}M ({} start {:.2}, lst{} req{} {:?}, top want {:?}) {reason}",
                        p.full_name,
                        before.age,
                        before.ca,
                        Self::club_name(data, before.club_id),
                        before.league_rep,
                        place.club_name,
                        now.league_rep,
                        before.salary as f64 / 1_000_000.0,
                        now.salary as f64 / 1_000_000.0,
                        before.status,
                        before.starter,
                        before.listed as u8,
                        before.req as u8,
                        before.req_reason,
                        before.top_goal,
                    ),
                });
            }
            if let Some(last) = self.last_permanent.get(&p.id).copied()
                && day - last.day <= YEAR
            {
                self.two_moves_in_a_year += 1;
                if now.club_id == last.from {
                    self.back_to_seller += 1;
                }
            }
            self.last_permanent.insert(
                p.id,
                PermanentMove {
                    day,
                    from: before.club_id,
                },
            );
            if !p.favorite_clubs.is_empty() {
                self.moves_with_a_favourite += 1;
                let goes_to_a_rival = p.favorite_clubs.iter().any(|favourite| {
                    data.continents
                        .iter()
                        .flat_map(|c| &c.countries)
                        .flat_map(|c| &c.clubs)
                        .find(|c| c.id == *favourite)
                        .is_some_and(|club| club.is_rival(now.club_id))
                });
                if goes_to_a_rival {
                    self.rival_moves += 1;
                    self.named_rival_moves.push(Named {
                        ca: before.ca,
                        text: format!(
                            "{date} {} {}y CA{} joins {}, a rival of a favourite",
                            p.full_name, before.age, before.ca, place.club_name
                        ),
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_left_for_nothing(
        &mut self,
        p: &Player,
        before: &Snap,
        place: &Place,
        to_pool: bool,
        day: u32,
        date: NaiveDate,
        reason: &str,
    ) {
        if to_pool {
            self.left_for_pool.insert(p.id, (before.club_id, day));
        }
        let expired = before.days_left <= 30 || reason == "dec_reason_contract_expired";
        let paid_off = !expired && to_pool;
        {
            let rates = self.rate(before);
            if expired {
                rates.expiry += 1;
            } else if paid_off {
                rates.pay_off += 1;
                if before.starting {
                    rates.paid_off_starting += 1;
                }
            } else {
                rates.other_free += 1;
            }
        }
        if !before.regular || !matches!(before.tier(), Tier::Elite | Tier::Top) {
            return;
        }
        let route = if expired {
            "expiry"
        } else if paid_off {
            "pay-off"
        } else {
            "other"
        };
        let text = format!(
            "{date} {} {}y CA{} rep{} rank {} {} start {:.2} left {}d lst{} req{} top want {:?}@{:.2} plan {:?} [{route} {reason}] -> {}",
            p.full_name,
            before.age,
            before.ca,
            before.rep,
            before.rank,
            before.status,
            before.starter,
            before.days_left,
            before.listed as u8,
            before.req as u8,
            before.top_goal,
            before.top_pressure,
            before.arc.map(|a| a.as_token()),
            place.club_name,
        );
        if paid_off && before.starting {
            self.named_pay_offs_starting.push(Named {
                ca: before.ca,
                text: text.clone(),
            });
        }
        self.named_exits.push(Named { ca: before.ca, text });
    }

    fn club_name(data: &SimulatorData, club_id: u32) -> String {
        data.continents
            .iter()
            .flat_map(|c| &c.countries)
            .flat_map(|c| &c.clubs)
            .find(|c| c.id == club_id)
            .map_or_else(|| "?".to_string(), |c| c.name.clone())
    }

    fn on_request_outcome(&mut self, span: RequestSpan, outcome: RequestOutcome) {
        if span.first == "RefusedPlayingTime" {
            self.refused_by_month.entry(span.month).or_insert([0; 6])[outcome as usize] += 1;
        }
        self.request_outcomes
            .entry((span.tier, span.regular, span.first))
            .or_insert([0; 6])[outcome as usize] += 1;
    }

    pub fn on_retired(&mut self, p: &Player, date: NaiveDate) {
        if let Some(span) = self.open_requests.remove(&p.id) {
            self.on_request_outcome(span, RequestOutcome::Retired);
        }
        let before = self.snaps.remove(&p.id);
        self.retirements += 1;
        let Some(before) = before else { return };
        self.rate(&before).retired += 1;
        if before.regular {
            self.retired_regulars += 1;
            self.named_retired_regulars.push(Named {
                ca: before.ca,
                text: format!(
                    "{date} {} {}y CA{} rep{} rank {} {} start {:.2}",
                    p.full_name,
                    before.age,
                    before.ca,
                    before.rep,
                    before.rank,
                    before.status,
                    before.starter
                ),
            });
        }
    }

    /// Weekly: how long a want or a plan lasts, and what ended it.
    pub fn observe_wants(&mut self, data: &SimulatorData, day: u32) {
        let mut wants = std::mem::take(&mut self.wants);
        let mut arc_spans = std::mem::take(&mut self.arc_spans);
        let mut arc_stats = std::mem::take(&mut self.arc_stats);
        let snaps = &self.snaps;
        let mut visit = |p: &Player| {
            let snap = snaps.get(&p.id);
            let club = snap.map_or(0, |s| s.club_id);
            let club_rep = snap.map_or(0.0, |s| s.club_rep);
            for goal in p.mind.goals().live() {
                let span = wants.entry((p.id, goal.kind)).or_insert(WantSpan {
                    first_day: day,
                    last_seen: day,
                    club,
                    club_rep,
                    peak_pressure: 0.0,
                });
                span.last_seen = day;
                span.peak_pressure = span.peak_pressure.max(goal.pressure());
            }
            let arc = p.mind.career.plan.map(|plan| plan.arc);
            match (arc_spans.get(&p.id).copied(), arc) {
                (Some((old, start, start_club)), new) if new != Some(old) => {
                    let stat: &mut ArcStat = arc_stats.entry(old.as_token()).or_default();
                    stat.weeks.push((day - start) / 7);
                    if start_club == club {
                        stat.same_club += 1;
                        *stat
                            .same_club_then
                            .entry(new.map_or("(no plan)", |next| next.as_token()))
                            .or_insert(0) += 1;
                    } else {
                        stat.after_move += 1;
                    }
                    match new {
                        Some(next) => {
                            arc_spans.insert(p.id, (next, day, club));
                        }
                        None => {
                            arc_spans.remove(&p.id);
                        }
                    }
                }
                (None, Some(next)) => {
                    arc_spans.insert(p.id, (next, day, club));
                }
                _ => {}
            }
        };
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            visit(p);
                        }
                    }
                }
            }
        }
        for p in &data.free_agents {
            visit(p);
        }

        let mut finished = Vec::new();
        wants.retain(|key, span| {
            if span.last_seen == day {
                true
            } else {
                finished.push((*key, *span));
                false
            }
        });
        for ((id, kind), span) in finished {
            let stat = self.want_stats.entry(kind).or_default();
            stat.weeks.push((span.last_seen - span.first_day) / 7);
            if span.peak_pressure >= 0.55 {
                stat.voiced += 1;
            }
            match self.snaps.get(&id) {
                Some(s) if s.club_id == span.club => stat.same_club += 1,
                Some(s) if s.club_rep > span.club_rep + 0.03 => stat.moved_up += 1,
                _ => stat.moved_other += 1,
            }
        }
        self.wants = wants;
        self.arc_spans = arc_spans;
        self.arc_stats = arc_stats;
    }

    /// Yearly: the world's reputation, wages and fees by club size, and how
    /// many squads are short of a position.
    pub fn observe_year(&mut self, data: &SimulatorData, ranks: &crate::reading::ClubRanks, day: u32) {
        let mut row = std::mem::take(&mut self.current_fees);
        row.day = day;
        let (mut main_teams, mut short) = (0, 0);
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let tier = Tier::of_rank(ranks.of(club.id));
                    let slot = Tier::CLUBS.iter().position(|t| *t == tier);
                    for team in &club.teams.teams {
                        if team.team_type != TeamType::Main {
                            continue;
                        }
                        main_teams += 1;
                        let mut positions = [0u32; 4];
                        for p in &team.players.players {
                            let position = p.position();
                            let index = if position.is_goalkeeper() {
                                0
                            } else if position.is_defender() {
                                1
                            } else if position.is_midfielder() {
                                2
                            } else {
                                3
                            };
                            positions[index] += 1;
                            let Some(slot) = slot else { continue };
                            row.men[slot] += 1;
                            let reputation = p.player_attributes.current_reputation;
                            let wage = p.contract.as_ref().map_or(0, |c| c.salary);
                            row.reputation_sum[slot] += reputation as f64;
                            row.wage_sum[slot] += wage as f64;
                            if reputation > row.top_reputation.0 {
                                row.top_reputation = (reputation, p.full_name.to_string());
                            }
                            if wage > row.top_wage.0 {
                                row.top_wage = (wage, p.full_name.to_string());
                            }
                        }
                        if positions[0] < 1 || positions[1] < 3 || positions[2] < 3 || positions[3] < 1 {
                            short += 1;
                        }
                    }
                }
            }
        }
        self.short_squads.push((day, short, main_teams));
        self.economy.push(row);
    }

    fn named(title: &str, rows: &[Named]) {
        if rows.is_empty() {
            return;
        }
        let mut order: Vec<&Named> = rows.iter().collect();
        order.sort_by_key(|n| std::cmp::Reverse(n.ca));
        println!("  worst cases, {title}:");
        for row in order.iter().take(WORST_CASES) {
            println!("    {}", row.text);
        }
    }

    fn percentiles(values: &[u32]) -> (u32, u32) {
        if values.is_empty() {
            return (0, 0);
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        (sorted[sorted.len() / 2], sorted[sorted.len() * 9 / 10])
    }

    pub fn report(&self, data: &SimulatorData, day: u32) {
        let date = data.date.date();
        let seasons = |cell: &Cell| self.exposure.get(cell).copied().unwrap_or(0) as f64 / 365.0;
        let per_100 = |count: u32, seasons: f64| {
            if seasons > 0.0 {
                count as f64 / seasons * 100.0
            } else {
                0.0
            }
        };

        println!("\n-- moves and exits per 100 player-seasons, by club size, role and age --");
        println!(
            "{:<14} {:<8} {:<6} {:>9} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6}",
            "club size", "role", "age", "seasons", "expiry", "/100", "payoff", "/100", "other", "/100", "sold", "/100", "dropdv", "/100", "reqs", "/100", "retire", "/100"
        );
        for tier in Tier::CLUBS {
            for regular in [true, false] {
                for band in 0..AgeBand::COUNT {
                    let cell = (tier, regular, band);
                    let s = seasons(&cell);
                    if s < 1.0 {
                        continue;
                    }
                    let empty = Rates::default();
                    let r = self.rates.get(&cell).unwrap_or(&empty);
                    println!(
                        "{:<14} {:<8} {:<6} {:>9.0} | {:>6} {:>6.1} | {:>6} {:>6.1} | {:>6} {:>6.1} | {:>6} {:>6.1} | {:>6} {:>6.1} | {:>6} {:>6.1} | {:>6} {:>6.1}",
                        tier.label(),
                        if regular { "regular" } else { "other" },
                        AgeBand::label(band),
                        s,
                        r.expiry,
                        per_100(r.expiry, s),
                        r.pay_off,
                        per_100(r.pay_off, s),
                        r.other_free,
                        per_100(r.other_free, s),
                        r.sold,
                        per_100(r.sold, s),
                        r.dropped,
                        per_100(r.dropped, s),
                        r.requests,
                        per_100(r.requests, s),
                        r.retired,
                        per_100(r.retired, s),
                    );
                }
            }
        }

        // Rolled up over the sizes a reader cares about most.
        let roll = |tiers: &[Tier], read: fn(&Rates) -> u32| -> (u32, f64) {
            let mut count = 0;
            let mut at_risk = 0.0;
            for tier in tiers {
                for band in 0..AgeBand::COUNT {
                    let cell = (*tier, true, band);
                    at_risk += seasons(&cell);
                    count += self.rates.get(&cell).map_or(0, read);
                }
            }
            (count, at_risk)
        };
        println!("\n-- regulars, by club size: count, player-seasons at risk, rate per 100 --");
        for (label, tiers) in [
            ("elite", vec![Tier::Elite]),
            ("elite+top", vec![Tier::Elite, Tier::Top]),
            ("all clubs", Tier::CLUBS.to_vec()),
        ] {
            let rows: [(&str, fn(&Rates) -> u32); 6] = [
                ("left at expiry", |r| r.expiry),
                ("paid off", |r| r.pay_off),
                ("paid off while starting", |r| r.paid_off_starting),
                ("other free exit", |r| r.other_free),
                ("dropped a division or more", |r| r.dropped),
                ("retired while regular", |r| r.retired),
            ];
            for (name, read) in rows {
                let (count, at_risk) = roll(&tiers, read);
                println!(
                    "  {label:<10} {name:<28} {count:>6} of {at_risk:>8.0} seasons = {:>5.1}/100",
                    per_100(count, at_risk)
                );
            }
        }
        Self::named("regulars at big clubs leaving for nothing", &self.named_exits);
        Self::named("paid off while still starting", &self.named_pay_offs_starting);
        Self::named("regulars dropping a division or more", &self.named_drops);

        println!(
            "\n  regulars <=29 at top-250 clubs dropping a division and to a smaller club: {} ({} of them starting)",
            self.fell_a_division_and_a_club, self.fell_though_starting
        );
        println!("\n-- odd journeys (all clubs) --");
        let career_seasons: f64 = self.exposure.values().sum::<u64>() as f64 / 365.0;
        for (name, count) in [
            ("released and re-signed by the same club within a year", self.resigned_by_same_club),
            ("two permanent moves within a year", self.two_moves_in_a_year),
            ("back within a year to the club that sold him", self.back_to_seller),
        ] {
            println!(
                "  {name:<56} {count:>6} of {career_seasons:>8.0} seasons = {:>5.2}/100",
                per_100(count, career_seasons)
            );
        }
        Self::named("released and re-signed by the same club", &self.named_resigned);
        println!(
            "  moves to a rival of a favourite club {} of {} moves by men holding a favourite",
            self.rival_moves, self.moves_with_a_favourite
        );
        Self::named("moves to a rival of a favourite", &self.named_rival_moves);

        println!("\n-- transfer requests --");
        let men = self.request_men.len().max(1);
        let short = self
            .request_spans
            .iter()
            .filter(|s| **s < SHORT_REQUEST_DAYS)
            .count();
        let (p50, p90) = Self::percentiles(&self.request_spans);
        println!(
            "  started {} by {} men = {:.2} per asking man, {:.2}/100 player-seasons",
            self.request_starts,
            men,
            self.request_starts as f64 / men as f64,
            per_100(self.request_starts, career_seasons),
        );
        println!(
            "  ended {}: {short} under a fortnight ({:.0}%), p50 {p50}d p90 {p90}d",
            self.request_spans.len(),
            short as f64 * 100.0 / self.request_spans.len().max(1) as f64,
        );
        println!(
            "  with no reason on their first day {} of {} ({:.0}%)",
            self.requests_no_reason,
            self.request_starts,
            self.requests_no_reason as f64 * 100.0 / self.request_starts.max(1) as f64
        );
        let mut reasons: Vec<_> = self.request_reasons.iter().collect();
        reasons.sort_by_key(|(key, count)| (key.0, !key.1, std::cmp::Reverse(**count)));
        for ((tier, regular, reason), count) in reasons.iter().take(40) {
            println!(
                "    {:<14} {:<8} {reason:<30} {count:>7}",
                tier.label(),
                if *regular { "regular" } else { "other" }
            );
        }

        println!("  how requests ended, by the tier and role he asked from and his first reason:");
        println!(
            "    {:<14} {:<8} {:<26} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "club size", "role", "first reason", "sold", "loaned", "free", "withdrn", "retired", "open"
        );
        let mut outcomes: BTreeMap<(Tier, bool, String), [u32; 6]> = self.request_outcomes.clone();
        for span in self.open_requests.values() {
            outcomes
                .entry((span.tier, span.regular, span.first.clone()))
                .or_insert([0; 6])[RequestOutcome::Open as usize] += 1;
        }
        let mut outcomes: Vec<_> = outcomes
            .into_iter()
            .filter(|((tier, _, _), counts)| {
                matches!(tier, Tier::Elite | Tier::Top) && counts.iter().sum::<u32>() >= 50
            })
            .collect();
        outcomes.sort_by_key(|((tier, regular, _), counts)| {
            (*tier, !*regular, std::cmp::Reverse(counts.iter().sum::<u32>()))
        });
        for ((tier, regular, reason), c) in &outcomes {
            println!(
                "    {:<14} {:<8} {reason:<26} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
                tier.label(),
                if *regular { "regular" } else { "other" },
                c[0], c[1], c[2], c[3], c[4], c[5]
            );
        }

        let mut by_month = self.refused_by_month.clone();
        for span in self.open_requests.values().filter(|s| s.first == "RefusedPlayingTime") {
            by_month.entry(span.month).or_insert([0; 6])[RequestOutcome::Open as usize] += 1;
        }
        println!(
            "  refused-talk requests, every club: {} by {} men; how they ended, by the month he asked:",
            self.refused_starts,
            self.refused_men.len()
        );
        for (month, c) in &by_month {
            println!(
                "    month {month:>2}                                       {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
                c[0], c[1], c[2], c[3], c[4], c[5]
            );
        }

        println!("\n-- listings --");
        let regular_listings: u32 = self.listing_reasons.values().sum();
        println!(
            "  of regulars {regular_listings} ({:.1}/100 regular seasons), of others {}",
            per_100(regular_listings, roll(&Tier::CLUBS, |_| 0).1),
            self.listings_of_others
        );
        let mut listings: Vec<_> = self.listing_reasons.iter().collect();
        listings.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for ((tier, band, reason), count) in listings.iter().take(25) {
            println!(
                "    {:<14} {:<6} {reason:<44} {count:>6}",
                tier.label(),
                AgeBand::label(*band)
            );
        }
        println!(
            "  still listed {LISTING_OUTLIVED_DAYS} days after their own request ended: {} of {} requests that ended at the club, and {} listed by the club by then, {} still long unhappy, {} with no market row",
            self.listed_after_request,
            self.request_spans.len(),
            self.listed_by_the_club_after_request,
            self.listed_while_still_unhappy,
            self.listed_with_no_market_row,
        );
        Self::named("still listed after their own request ended", &self.named_listed_after_request);

        println!("\n-- retirements --");
        println!(
            "  {} retired, {} of them regulars",
            self.retirements, self.retired_regulars
        );
        Self::named("retired while regular", &self.named_retired_regulars);

        self.report_plans(data, date);
        self.report_dreams(data, date);

        println!("\n-- how long a want lasts (finished spans) and what ended it --");
        println!(
            "{:<28} {:>7} {:>6} {:>6} {:>6} | {:>9} {:>9} {:>9} {:>7}",
            "want", "n", "p50w", "p90w", ">=1y", "sameClub%", "movedUp%", "movedOth%", "voiced%"
        );
        let mut kinds: Vec<(&GoalKind, &WantStat)> = self.want_stats.iter().collect();
        kinds.sort_by_key(|(_, s)| std::cmp::Reverse(s.weeks.len()));
        for (kind, stat) in kinds {
            let (p50, p90) = Self::percentiles(&stat.weeks);
            let n = stat.weeks.len().max(1) as f64;
            let over_year = stat.weeks.iter().filter(|w| **w >= 52).count() as f64 / n * 100.0;
            println!(
                "{:<28} {:>7} {p50:>6} {p90:>6} {over_year:>5.0}% | {:>8.0}% {:>8.0}% {:>8.0}% {:>6.0}%",
                format!("{kind:?}"),
                stat.weeks.len(),
                stat.same_club as f64 / n * 100.0,
                stat.moved_up as f64 / n * 100.0,
                stat.moved_other as f64 / n * 100.0,
                stat.voiced as f64 / n * 100.0
            );
        }
        println!("\n-- how long a plan lasts --");
        let mut arcs: Vec<(&&str, &ArcStat)> = self.arc_stats.iter().collect();
        arcs.sort_by_key(|(_, s)| std::cmp::Reverse(s.weeks.len()));
        for (arc, stat) in arcs {
            let (p50, p90) = Self::percentiles(&stat.weeks);
            let open: Vec<u32> = self
                .arc_spans
                .values()
                .filter(|(held, _, _)| held.as_token() == *arc)
                .map(|(_, start, _)| (day - start) / 7)
                .collect();
            println!(
                "  {arc:<22} n{:>7} p50w {p50:>4} p90w {p90:>4} median with the {:>6} still held {:>4} ended at the same club {:>6} after a move {:>6}",
                stat.weeks.len(),
                open.len(),
                Self::median_with_those_still_held(&stat.weeks, &open),
                stat.same_club,
                stat.after_move
            );
            if *arc == "step_up" {
                let mut then: Vec<_> = stat.same_club_then.iter().collect();
                then.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
                let line: Vec<String> = then.iter().map(|(next, count)| format!("{next} {count}")).collect();
                println!("    at the same club it gave way to: {}", line.join(", "));
            }
        }

        println!("\n-- reputation, wages and fees by club size, by year --");
        for row in &self.economy {
            print!("  day {:>5}:", row.day);
            for (slot, tier) in Tier::CLUBS.iter().enumerate() {
                let men = row.men[slot].max(1) as f64;
                let fees = row.fee_count[slot].max(1) as f64;
                print!(
                    " {} rep {:.0} wage ${:.2}M fee ${:.1}M |",
                    tier.label(),
                    row.reputation_sum[slot] / men,
                    row.wage_sum[slot] / men / 1_000_000.0,
                    row.fee_sum[slot] / fees / 1_000_000.0,
                );
            }
            println!(
                " highest rep {} ({}), wage ${:.1}M ({}), fee ${:.1}M ({})",
                row.top_reputation.0,
                row.top_reputation.1,
                row.top_wage.0 as f64 / 1_000_000.0,
                row.top_wage.1,
                row.top_fee.0 / 1_000_000.0,
                row.top_fee.1,
            );
        }

        println!("\n-- guards: what a fix must not break --");
        let matches = self.results.matches.max(1) as f64;
        println!(
            "  results over {} matches: {:.2} goals (2.6-2.9), {:.1}% draws (22-28), {:.1}% goalless (5-8), {:.1}% home goals (53-58)",
            self.results.matches,
            self.results.goals as f64 / matches,
            self.results.draws as f64 * 100.0 / matches,
            self.results.goalless as f64 * 100.0 / matches,
            self.results.home_goals as f64 * 100.0 / self.results.goals.max(1) as f64,
        );
        println!(
            "  per match: {:.1} fouls, {:.2} yellows, {:.3} reds, {:.3} hurt or worse; player of the match named in {:.0}%",
            self.results.fouls as f64 / matches,
            self.results.yellows as f64 / matches,
            self.results.reds as f64 / matches,
            self.results.hurt_or_worse as f64 / matches,
            self.results.player_of_the_match_named as f64 * 100.0 / matches,
        );
        for (season, moves) in &self.season_moves {
            println!(
                "  season {season:>2}: paid moves {:>6}, free signings {:>6}, released {:>6}, loans {:>6}",
                moves.paid, moves.free_signings, moves.released, moves.loans
            );
        }
        let mut free_reasons: Vec<_> = self.free_signing_reasons.iter().collect();
        free_reasons.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        let free_reasons: Vec<String> = free_reasons
            .iter()
            .take(8)
            .map(|(reason, count)| format!("{reason} {count}"))
            .collect();
        println!("  free signings by reason: {}", free_reasons.join(", "));
        for (when, short, teams) in &self.short_squads {
            println!(
                "  day {when:>5}: squads short of a position {short} of {teams} ({:.1}%)",
                *short as f64 * 100.0 / (*teams).max(1) as f64
            );
        }
        println!(
            "  free-agent pool {} today; finished stays {} averaging {:.0}d; renewals signed {}",
            data.free_agents.len(),
            self.pool_stays,
            self.pool_stay_days as f64 / self.pool_stays.max(1) as f64,
            self.renewals_signed,
        );
        let mut pool: Vec<&Player> = data.free_agents.iter().collect();
        pool.sort_by_key(|p| std::cmp::Reverse(p.player_attributes.current_ability));
        println!("  strongest in the pool:");
        for p in pool.iter().take(8) {
            println!(
                "    {:<26} {}y CA{} rep{} top want {:?}",
                p.full_name.to_string(),
                p.age(date),
                p.player_attributes.current_ability,
                p.player_attributes.current_reputation,
                p.mind.strongest_goal().map(|g| g.kind)
            );
        }
        let _ = day;
    }

    /// The dreams held today, by club size, and what has become of them over
    /// the run. Everybody counts: under contract, in the pool or retired.
    /// Kaplan-Meier: a plan still held has lasted at least this long, and
    /// counting only the ended ones undersells the longest.
    fn median_with_those_still_held(ended: &[u32], open: &[u32]) -> u32 {
        let mut rows: Vec<(u32, bool)> = ended
            .iter()
            .map(|weeks| (*weeks, true))
            .chain(open.iter().map(|weeks| (*weeks, false)))
            .collect();
        rows.sort_unstable();
        let mut at_risk = rows.len() as f64;
        let mut surviving = 1.0;
        for (weeks, did_end) in rows {
            if did_end {
                surviving *= 1.0 - 1.0 / at_risk;
                if surviving <= 0.5 {
                    return weeks;
                }
            }
            at_risk -= 1.0;
        }
        u32::MAX
    }

    fn report_dreams(&self, data: &SimulatorData, date: NaiveDate) {
        let mut seniors: HashMap<Tier, u32> = HashMap::new();
        let mut holders: HashMap<Tier, u32> = HashMap::new();
        let mut arrived: HashMap<Tier, u32> = HashMap::new();
        let mut summits: HashMap<Tier, u32> = HashMap::new();
        let mut by_kind: HashMap<(Tier, DreamKind), u32> = HashMap::new();
        let mut arrived_planning_to_step_up = 0;
        let mut turns: HashMap<ChapterKind, u32> = HashMap::new();
        let mut read_turns = |p: &Player| {
            for chapter in p.mind.career.anchors.chapters.iter() {
                *turns.entry(chapter.kind).or_insert(0) += 1;
            }
        };
        for continent in &data.continents {
            for country in &continent.countries {
                for p in &country.retired_players.players {
                    read_turns(p);
                }
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            read_turns(p);
                            if p.is_on_loan() || p.age(date) < CareerDreamReview::MIN_AGE {
                                continue;
                            }
                            let Some(snap) = self.snaps.get(&p.id) else {
                                continue;
                            };
                            let tier = snap.tier();
                            *seniors.entry(tier).or_insert(0) += 1;
                            let anchors = &p.mind.career.anchors;
                            *summits.entry(tier).or_insert(0) += anchors.summit.is_some() as u32;
                            if let Some(dream) = anchors.dream {
                                *holders.entry(tier).or_insert(0) += 1;
                                *arrived.entry(tier).or_insert(0) += dream.arrived as u32;
                                if dream.arrived
                                    && p.mind.career.plan.is_some_and(|plan| plan.arc == CareerArc::StepUp)
                                {
                                    arrived_planning_to_step_up += 1;
                                }
                                *by_kind.entry((tier, dream.kind)).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        for p in &data.free_agents {
            read_turns(p);
        }

        println!("\n-- dreams, today, by club size --");
        println!("  arrived and still planning to step up: {arrived_planning_to_step_up}");
        for tier in Tier::CLUBS {
            let n = seniors.get(&tier).copied().unwrap_or(0).max(1);
            let share = |count: u32| count as f64 * 100.0 / n as f64;
            let kinds: Vec<String> = DreamKind::ALL
                .iter()
                .map(|kind| {
                    format!(
                        "{kind:?} {:.0}%",
                        share(by_kind.get(&(tier, *kind)).copied().unwrap_or(0))
                    )
                })
                .collect();
            println!(
                "  {:<14} n{n:>7}  holds a dream {:.1}%, arrived {:.1}%, holds a summit {:.1}% | {}",
                tier.label(),
                share(holders.get(&tier).copied().unwrap_or(0)),
                share(arrived.get(&tier).copied().unwrap_or(0)),
                share(summits.get(&tier).copied().unwrap_or(0)),
                kinds.join(", "),
            );
        }
        println!("  turns over careers (chapters still held):");
        for kind in ChapterKind::ALL {
            println!(
                "    {kind:?}: {}",
                turns.get(&kind).copied().unwrap_or(0)
            );
        }
    }

    /// Plans and wants held today, by club size.
    fn report_plans(&self, data: &SimulatorData, date: NaiveDate) {
        println!("\n-- today, by club size: seniors (20+, not on loan) holding a plan to step up, and each want --");
        let mut seniors: HashMap<Tier, u32> = HashMap::new();
        let mut step_ups: HashMap<Tier, u32> = HashMap::new();
        let mut holders: HashMap<(Tier, GoalKind), u32> = HashMap::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for p in &team.players.players {
                            if p.is_on_loan() || p.age(date) < 20 {
                                continue;
                            }
                            let Some(snap) = self.snaps.get(&p.id) else {
                                continue;
                            };
                            let tier = snap.tier();
                            *seniors.entry(tier).or_insert(0) += 1;
                            if snap.arc == Some(CareerArc::StepUp) {
                                *step_ups.entry(tier).or_insert(0) += 1;
                            }
                            for goal in p.mind.goals().live() {
                                *holders.entry((tier, goal.kind)).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        for tier in Tier::CLUBS {
            let n = seniors.get(&tier).copied().unwrap_or(0).max(1);
            println!(
                "  {:<14} n{n:>7}  plan to step up {:>5} = {:.1}%",
                tier.label(),
                step_ups.get(&tier).copied().unwrap_or(0),
                step_ups.get(&tier).copied().unwrap_or(0) as f64 * 100.0 / n as f64,
            );
            let mut rows: Vec<(GoalKind, u32)> = holders
                .iter()
                .filter(|((t, _), _)| *t == tier)
                .map(|((_, kind), count)| (*kind, *count))
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            let line: Vec<String> = rows
                .iter()
                .take(12)
                .map(|(kind, count)| format!("{kind:?} {:.1}%", *count as f64 * 100.0 / n as f64))
                .collect();
            println!("     {}", line.join(", "));
        }
    }
}
