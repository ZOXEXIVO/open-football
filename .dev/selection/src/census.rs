//! Reads every first-team sheet of the top two divisions as the season is
//! played, and turns the season into report rows.

use crate::report::{Mean, Report, Row, Unit};
use crate::world::{ClubFacts, DayFacts, Kind, PitchLine, PlayerFacts, WorldIndex};
use chrono::NaiveDate;
use core::club::staff::PlannedRole;
use core::r#match::{FieldSquad, MatchResult, MatchResultRaw};
use core::{HappinessEventType, PlayerPositionType, SimulationResult, SimulatorData};
use std::collections::{HashMap, HashSet};

/// One competitive sheet, kept so rotation can be read against the fixtures
/// either side of it.
struct SheetNote {
    date: NaiveDate,
    kind: Kind,
    home: bool,
    importance: f32,
    xi: Vec<u32>,
    quarter: usize,
    established: u8,
    young: u8,
}

impl SheetNote {
    fn changes_from(&self, earlier: &SheetNote) -> f64 {
        let kept = self
            .xi
            .iter()
            .filter(|id| earlier.xi.binary_search(id).is_ok())
            .count();
        (self.xi.len().max(11) - kept) as f64
    }

    fn days_after(&self, earlier: &SheetNote) -> i64 {
        (self.date - earlier.date).num_days()
    }
}

/// What the census knows of a fixture besides the sheet itself.
struct FixtureNote {
    kind: Kind,
    home: bool,
    quarter: usize,
    season: i32,
}

#[derive(Default)]
struct TeamBook {
    tier: u8,
    club_name: String,
    sheets: Vec<SheetNote>,
    league_matches: u32,
    league_starts: HashMap<u32, u32>,
    league_appearances: HashMap<u32, u32>,
    keeper_starts: [HashMap<u32, u32>; 3],
    /// Starts by the keeper declared number one that morning, and sheets
    /// picked while a declaration stood.
    declared_starts: [(u64, u64); 3],
    deputy_starts: [u64; 3],
    borrowed_keepers: HashSet<u32>,
    last_declared: Option<u32>,
    last_friendly: Option<Vec<u32>>,
    friendly_starters: HashSet<u32>,
    friendlies: u32,
}

#[derive(Default)]
struct Totals {
    sheets: [u64; 3],
    told_importance: [Mean; 3],
    told_room: [Mean; 3],
    starts: [u64; 3],
    starts_by_age: [[u64; 6]; 3],
    starts_by_status: [[u64; 9]; 3],
    starts_by_role: [[u64; 10]; 3],
    starts_listed: [u64; 3],
    starts_shop_window: [u64; 3],
    starts_first_choice: [u64; 3],
    plan_shares: Mean,
    plan_first_choices: Mean,
    starts_other_squad: [u64; 3],
    starts_unknown: u64,
    starts_against_parent: u64,
    starts_cup_tied: u64,
    team_days: u64,
    double_match_days: u64,
    league_appearances: u64,
    league_appearances_young: u64,
    league_appearances_old: u64,
    borrowed_best_sheets: u64,
    borrowed_best_undeclared: u64,
    outfielder_in_goal: [u64; 3],
    outfielder_with_free_keeper: u64,
    outfielder_details: Vec<String>,
    bench_size: [Mean; 3],
    subs_used: Mean,
    bench_keepers: [u64; 3],
    bench_keepers_small: [u64; 3],
    uncovered_lines: Mean,
    line_fielded: [u64; 5],
    line_uncovered: [u64; 5],
    plan_fixtures: [u64; 10],
    plan_planned: [f64; 10],
    plan_played: [f64; 10],
    preseason_sheets: u64,
    preseason_changes: Mean,
    preseason_starts: u64,
    preseason_by_role: [u64; 10],
    preseason_listed: u64,
    preseason_young: u64,
    complaints: [u64; 10],
    role_days: [u64; 10],
    young_days: u64,
    young_first_team: u64,
    young_first_team_friendly: u64,
    young_other_sides: u64,
}

pub struct Census {
    pub index: WorldIndex,
    books: HashMap<u32, TeamBook>,
    totals: [Totals; 2],
    complaints_seen: HashMap<u32, usize>,
    /// The club each player first appeared for in a cup's edition.
    cup_record: HashMap<(u32, u32, i32), u32>,
    days: u32,
}

impl Census {
    /// The department reviews its room monthly, so a keeper is given this
    /// long to be declared.
    const REVIEW_LAG_DAYS: i64 = 35;
    /// Levels clear of the declared number one at which the department
    /// changes its order.
    const CLEARLY_BETTER: u8 = 6;

    const AGE_BANDS: [&'static str; 6] = ["le17", "18_19", "20_21", "22_23", "24_29", "ge30"];
    const QUARTERS: [&'static str; 4] = ["0_25", "25_50", "50_70", "70_100"];
    const STAKES: [&'static str; 5] = ["lt30", "30_50", "50_70", "70_85", "ge85"];
    const SIDES: [&'static str; 2] = ["away", "home"];
    /// A club's season is read once it has this many league sheets.
    const SEASON_SHEETS: u32 = 20;
    /// A fixture "matters more" when its importance is this far above.
    const BIGGER_BY: f32 = 0.05;
    const CLOSE_DAYS: i64 = 4;
    const DETAIL_LINES: usize = 40;

    pub fn new(data: &SimulatorData) -> Self {
        Census {
            index: WorldIndex::build(data),
            books: HashMap::new(),
            totals: [Totals::default(), Totals::default()],
            complaints_seen: HashMap::new(),
            cup_record: HashMap::new(),
            days: 0,
        }
    }

    fn age_band(age: u8) -> usize {
        match age {
            0..=17 => 0,
            18..=19 => 1,
            20..=21 => 2,
            22..=23 => 3,
            24..=29 => 4,
            _ => 5,
        }
    }

    fn stakes_band(importance: f32) -> usize {
        match importance {
            i if i < 0.30 => 0,
            i if i < 0.50 => 1,
            i if i < 0.70 => 2,
            i if i < 0.85 => 3,
            _ => 4,
        }
    }

    /// Football played in the match by each man who took the field, in
    /// matches: a full ninety minutes is 1.0.
    fn football_played(sheet: &FieldSquad, details: &MatchResultRaw) -> HashMap<u32, f32> {
        let mut played: HashMap<u32, f32> = sheet.main.iter().map(|id| (*id, 1.0)).collect();
        for change in details
            .substitutions
            .iter()
            .filter(|s| s.team_id == sheet.team_id)
        {
            let minute = (change.match_time_ms as f32 / 60_000.0).min(90.0);
            if let Some(share) = played.get_mut(&change.player_out_id) {
                *share = (*share - (90.0 - minute) / 90.0).max(0.0);
            }
            played.insert(change.player_in_id, (90.0 - minute) / 90.0);
        }
        played
    }

    pub fn record_day(&mut self, data: &SimulatorData, day: &DayFacts, result: &SimulationResult) {
        self.days += 1;
        let playing: HashSet<u32> = result
            .match_results
            .iter()
            .flat_map(|m| [m.home_team_id, m.away_team_id])
            .collect();
        for m in &result.match_results {
            let Some(details) = m.details.as_ref() else {
                continue;
            };
            let sides = [
                (&details.left_team_players, &details.right_team_players, true),
                (&details.right_team_players, &details.left_team_players, false),
            ];
            for (sheet, opponent, home) in sides {
                self.record_young(day, m, sheet);
                let cup_tied = self.record_cup_appearances(data, m, sheet);
                let Some(club) = day.clubs.get(&sheet.team_id) else {
                    continue;
                };
                self.totals[usize::from(club.tier == 2)].starts_cup_tied += cup_tied;
                if sheet.main.len() < 11 {
                    continue;
                }
                match self.index.kind(m) {
                    Some(kind) => {
                        let fixture = FixtureNote {
                            kind,
                            home,
                            quarter: Self::season_quarter(data, &self.index, sheet.team_id, m),
                            season: self.index.season_year(m.league_id, day.date),
                        };
                        self.record_sheet(day, club, fixture, sheet, opponent, details, &playing);
                    }
                    None if m.friendly => self.record_friendly(club, sheet),
                    None => {}
                }
            }
        }
        self.read_dressing_rooms(data, day);
    }

    /// Books every man who took the field in a cup tie against the club he
    /// played for, and returns the starters who had already appeared for
    /// another club in this edition.
    fn record_cup_appearances(
        &mut self,
        data: &SimulatorData,
        m: &MatchResult,
        sheet: &FieldSquad,
    ) -> u64 {
        if self.index.kind(m) != Some(Kind::Cup) {
            return 0;
        }
        let (Some(club_id), Some(edition)) = (
            self.index.club_of_team(sheet.team_id),
            self.index.cup_edition(data, m.league_id),
        ) else {
            return 0;
        };
        let mut cup_tied = 0;
        for (i, id) in sheet
            .main
            .iter()
            .chain(sheet.substitutes_used.iter())
            .enumerate()
        {
            let first = *self
                .cup_record
                .entry((*id, m.league_id, edition))
                .or_insert(club_id);
            cup_tied += u64::from(first != club_id && i < sheet.main.len());
        }
        cup_tied
    }

    fn record_young(&mut self, day: &DayFacts, m: &MatchResult, sheet: &FieldSquad) {
        for id in sheet.main.iter().chain(sheet.substitutes_used.iter()) {
            let Some(first_team) = day.young_first_team.get(id) else {
                continue;
            };
            let Some(club) = day.clubs.get(first_team) else {
                continue;
            };
            let totals = &mut self.totals[usize::from(club.tier == 2)];
            if sheet.team_id != *first_team {
                totals.young_other_sides += 1;
            } else if m.friendly {
                totals.young_first_team_friendly += 1;
            } else {
                totals.young_first_team += 1;
            }
        }
    }

    fn record_friendly(&mut self, club: &ClubFacts, sheet: &FieldSquad) {
        let totals = &mut self.totals[usize::from(club.tier == 2)];
        let book = self.books.entry(sheet.team_id).or_default();
        let mut xi = sheet.main.clone();
        xi.sort_unstable();
        totals.preseason_sheets += 1;
        if let Some(last) = book.last_friendly.as_ref() {
            let kept = xi.iter().filter(|id| last.binary_search(id).is_ok()).count();
            totals.preseason_changes.add((11 - kept.min(11)) as f64);
        }
        for id in &sheet.main {
            totals.preseason_starts += 1;
            book.friendly_starters.insert(*id);
            let Some(player) = club.players.get(id) else {
                continue;
            };
            totals.preseason_by_role[PlayerFacts::role_slot(player.role)] += 1;
            totals.preseason_listed += u64::from(player.listed);
            totals.preseason_young += u64::from(player.age <= DayFacts::YOUNG_AGE);
        }
        book.friendlies += 1;
        book.last_friendly = Some(xi);
    }

    #[allow(clippy::too_many_arguments)]
    fn record_sheet(
        &mut self,
        day: &DayFacts,
        club: &ClubFacts,
        fixture: FixtureNote,
        sheet: &FieldSquad,
        opponent: &FieldSquad,
        details: &MatchResultRaw,
        playing: &HashSet<u32>,
    ) {
        let FixtureNote {
            kind,
            home,
            quarter,
            season,
        } = fixture;
        let k = kind.slot();
        let totals = &mut self.totals[usize::from(club.tier == 2)];
        let book = self.books.entry(sheet.team_id).or_default();
        book.tier = club.tier;
        if book.club_name.is_empty() {
            book.club_name = club.club_name.clone();
        }
        totals.sheets[k] += 1;
        totals.told_importance[k].add(f64::from(sheet.selection.importance));
        totals.told_room[k].add(f64::from(sheet.selection.room));

        // A borrowed keeper clearly the best the club has, a month into
        // his loan, against the order the department declares.
        if kind == Kind::League
            && let Some(declared) = club.declared_number_one
        {
            let best = club
                .players
                .iter()
                .filter(|(_, p)| p.keeper && p.first_team)
                .max_by_key(|(id, p)| (p.level, std::cmp::Reverse(**id)));
            if let Some((id, best)) = best
                && best.borrowed
                && best.borrowed_days.is_some_and(|days| days > Self::REVIEW_LAG_DAYS)
            {
                let declared_level = club.players.get(&declared).map_or(0, |p| p.level);
                if *id == declared || best.level >= declared_level.saturating_add(Self::CLEARLY_BETTER) {
                    totals.borrowed_best_sheets += 1;
                    totals.borrowed_best_undeclared += u64::from(*id != declared);
                }
            }
        }

        // Who wears the gloves.
        let keeper_id = sheet
            .starter_slots
            .iter()
            .find(|(_, slot)| *slot == PlayerPositionType::Goalkeeper)
            .map(|(id, _)| *id);
        if let Some(id) = keeper_id {
            let facts = club.players.get(&id);
            if facts.is_some_and(|p| !p.keeper) {
                totals.outfielder_in_goal[k] += 1;
                let (elsewhere, free) =
                    club.keepers_elsewhere(sheet.team_id, &|team| playing.contains(&team));
                totals.outfielder_with_free_keeper += u64::from(free > 0);
                if totals.outfielder_details.len() < Self::DETAIL_LINES {
                    let own: Vec<String> = club
                        .players
                        .iter()
                        .filter(|(_, p)| p.keeper && p.team_id == sheet.team_id)
                        .map(|(id, p)| {
                            format!(
                                "{}{}{}",
                                PlayerFacts::role_key(p.role),
                                if p.fit { "" } else { " unfit" },
                                if sheet.substitutes.contains(id) { " on the bench" } else { "" },
                            )
                        })
                        .collect();
                    totals.outfielder_details.push(format!(
                        "{} {} {}: own keepers [{}], fit keepers in the club's other squads {elsewhere}, free that day {free}",
                        day.date,
                        club.club_name,
                        kind.key(),
                        own.join(", ")
                    ));
                }
            }
            if facts.is_some_and(|p| p.borrowed) {
                book.borrowed_keepers.insert(id);
            }
            *book.keeper_starts[k].entry(id).or_default() += 1;
            if let Some(declared) = club.declared_number_one {
                book.declared_starts[k].0 += u64::from(declared == id);
                book.declared_starts[k].1 += 1;
                book.deputy_starts[k] += u64::from(club.declared_deputy == Some(id));
            }
        }
        book.last_declared = club.declared_number_one;

        // Starters by age, status, plan, market state and squad of origin.
        let opponent_club = day
            .team_owner
            .get(&opponent.team_id)
            .and_then(|first_team| day.clubs.get(first_team))
            .map(|c| c.club_id)
            .or_else(|| self.index.club_of_team(opponent.team_id));
        let mut established = 0u8;
        let mut young = 0u8;
        for id in &sheet.main {
            totals.starts[k] += 1;
            let Some(player) = club.players.get(id) else {
                totals.starts_unknown += 1;
                continue;
            };
            totals.starts_by_age[k][Self::age_band(player.age)] += 1;
            totals.starts_by_status[k][player.status] += 1;
            totals.starts_by_role[k][PlayerFacts::role_slot(player.role)] += 1;
            established += u8::from(player.status <= 1);
            young += u8::from(player.age <= DayFacts::YOUNG_AGE);
            totals.starts_listed[k] += u64::from(player.listed);
            totals.starts_shop_window[k] += u64::from(player.role == Some(PlannedRole::ShopWindow));
            totals.starts_first_choice[k] += u64::from(player.first_choice);
            totals.starts_other_squad[k] += u64::from(player.team_id != sheet.team_id);
            totals.starts_against_parent +=
                u64::from(player.parent_club.is_some() && player.parent_club == opponent_club);
        }

        // Football played against football planned, over the side's own fit
        // players.
        let played = Self::football_played(sheet, details);
        let side = || club.players.values().filter(|p| p.team_id == sheet.team_id);
        if side().any(|p| p.role.is_some()) {
            totals.plan_shares.add(side().map(|p| f64::from(p.share)).sum());
            totals.plan_first_choices.add(side().filter(|p| p.first_choice).count() as f64);
        }
        for (id, player) in club
            .players
            .iter()
            .filter(|(_, p)| p.team_id == sheet.team_id && p.fit)
        {
            let slot = PlayerFacts::role_slot(player.role);
            totals.plan_fixtures[slot] += 1;
            totals.plan_planned[slot] += f64::from(player.share);
            totals.plan_played[slot] += f64::from(played.get(id).copied().unwrap_or(0.0));
        }

        // The bench.
        totals.bench_size[k].add(sheet.substitutes.len() as f64);
        totals.subs_used.add(sheet.substitutes_used.len() as f64);
        let bench: Vec<&PlayerFacts> = sheet
            .substitutes
            .iter()
            .filter_map(|id| club.players.get(id))
            .collect();
        let bench_keepers = bench.iter().filter(|p| p.keeper).count().min(2);
        totals.bench_keepers[bench_keepers] += 1;
        if sheet.substitutes.len() <= 9 {
            totals.bench_keepers_small[bench_keepers] += 1;
        }
        let cover = bench
            .iter()
            .filter(|p| !p.keeper)
            .fold(0u8, |mask, p| mask | p.lines);
        let fielded = sheet
            .starter_slots
            .iter()
            .filter_map(|(_, slot)| PitchLine::of(*slot))
            .fold(0u8, |mask, line| mask | line.bit());
        let mut uncovered = 0u32;
        for line in PitchLine::ALL {
            if fielded & line.bit() == 0 {
                continue;
            }
            totals.line_fielded[line as usize] += 1;
            if cover & line.bit() == 0 {
                totals.line_uncovered[line as usize] += 1;
                uncovered += 1;
            }
        }
        totals.uncovered_lines.add(f64::from(uncovered));

        if kind == Kind::League {
            book.league_matches += 1;
            for id in &sheet.main {
                *book.league_starts.entry(*id).or_default() += 1;
            }
            for id in sheet.main.iter().chain(sheet.substitutes_used.iter()) {
                *book.league_appearances.entry(*id).or_default() += 1;
                totals.league_appearances += 1;
                if let Some(player) = club.players.get(id) {
                    let season_age = season - player.born;
                    totals.league_appearances_young += u64::from(season_age <= 21);
                    totals.league_appearances_old += u64::from(season_age >= 30);
                }
            }
        }

        let mut xi = sheet.main.clone();
        xi.sort_unstable();
        book.sheets.push(SheetNote {
            date: day.date,
            kind,
            home,
            importance: sheet.selection.importance,
            xi,
            quarter,
            established,
            young,
        });
    }

    /// Playing-time complaints raised today, by the role the plan gave the
    /// player this morning.
    fn read_dressing_rooms(&mut self, data: &SimulatorData, day: &DayFacts) {
        for (team_id, team_ref) in self.index.tracked() {
            let (Some(club), Some(facts)) = (self.index.club(data, team_ref), day.clubs.get(team_id))
            else {
                continue;
            };
            let totals = &mut self.totals[usize::from(facts.tier == 2)];
            totals.team_days += 1;
            totals.double_match_days += u64::from(facts.double_match_week);
            totals.young_days += facts
                .players
                .values()
                .filter(|p| p.first_team && p.age <= DayFacts::YOUNG_AGE)
                .count() as u64;
            let Some(team) = club.teams.teams.iter().find(|t| t.id == *team_id) else {
                continue;
            };
            for player in team.players.iter() {
                let slot = PlayerFacts::role_slot(facts.players.get(&player.id).and_then(|p| p.role));
                totals.role_days[slot] += 1;
                let complaints = player
                    .happiness
                    .recent_events
                    .iter()
                    .filter(|e| e.event_type == HappinessEventType::LackOfPlayingTime)
                    .count();
                let seen = self.complaints_seen.insert(player.id, complaints).unwrap_or(0);
                totals.complaints[slot] += complaints.saturating_sub(seen) as u64;
            }
        }
    }

    fn season_quarter(data: &SimulatorData, index: &WorldIndex, team_id: u32, m: &MatchResult) -> usize {
        let Some(rounds) = index.league_rounds(m.league_id) else {
            return 0;
        };
        let Some(team) = index.main_teams.get(&team_id) else {
            return 0;
        };
        let country = &data.continents[team.continent].countries[team.country];
        let played = country
            .leagues
            .leagues
            .iter()
            .find(|l| l.id == m.league_id)
            .and_then(|l| l.table.rows.iter().find(|row| row.team_id == team_id))
            .map(|row| f64::from(row.played))
            .unwrap_or(0.0);
        match played / f64::from(rounds) {
            p if p < 0.25 => 0,
            p if p < 0.50 => 1,
            p if p < 0.70 => 2,
            _ => 3,
        }
    }

    pub fn report(&self) -> Report {
        let mut report = Report::default();
        for tier in [1u8, 2] {
            let totals = &self.totals[usize::from(tier == 2)];
            let books: Vec<&TeamBook> = self.books.values().filter(|b| b.tier == tier).collect();
            Self::rotation_rows(&mut report, tier, totals, &books);
            Self::keeper_rows(&mut report, tier, totals, &books);
            Self::spread_rows(&mut report, tier, totals, &books);
            Self::starter_rows(&mut report, tier, totals);
            self.plan_rows(&mut report, tier, totals);
            Self::bench_rows(&mut report, tier, totals);
            Self::preseason_rows(&mut report, tier, totals, &books);
            self.young_rows(&mut report, tier, totals);
        }
        report
    }

    fn rotation_rows(report: &mut Report, tier: u8, totals: &Totals, books: &[&TeamBook]) {
        const S: &str = "S1 rotation";
        let mut league_to_league = Mean::default();
        let mut histogram = [0u64; 7];
        let mut by_rest = [Mean::default(); 3];
        let mut into = [Mean::default(); 3];
        let mut by_stakes = [[Mean::default(); 2]; 5];
        let mut bigger_ahead = [Mean::default(); 2];
        let mut league_before_other = Mean::default();
        let mut league_after_other = Mean::default();
        let mut league_clear = Mean::default();
        let mut quarter_changes = [Mean::default(); 4];
        let mut quarter_established = [Mean::default(); 4];
        let mut quarter_young = [Mean::default(); 4];

        for book in books {
            let mut last_league: Option<&SheetNote> = None;
            for (i, sheet) in book.sheets.iter().enumerate() {
                let previous = i.checked_sub(1).map(|j| &book.sheets[j]);
                let next = book.sheets.get(i + 1);
                if let Some(previous) = previous {
                    let changes = sheet.changes_from(previous);
                    let rest = sheet.days_after(previous);
                    by_rest[if rest <= 4 {
                        0
                    } else if rest <= 8 {
                        1
                    } else {
                        2
                    }]
                    .add(changes);
                    into[sheet.kind.slot()].add(changes);
                    by_stakes[Self::stakes_band(sheet.importance)][usize::from(sheet.home)].add(changes);
                    let bigger = next.is_some_and(|n| {
                        n.days_after(sheet) <= Self::CLOSE_DAYS
                            && n.importance > sheet.importance + Self::BIGGER_BY
                    });
                    bigger_ahead[usize::from(bigger)].add(changes);
                }
                if sheet.kind != Kind::League {
                    continue;
                }
                quarter_established[sheet.quarter].add(f64::from(sheet.established));
                quarter_young[sheet.quarter].add(f64::from(sheet.young));
                if let Some(last) = last_league {
                    let changes = sheet.changes_from(last);
                    league_to_league.add(changes);
                    histogram[(changes as usize).min(6)] += 1;
                    quarter_changes[sheet.quarter].add(changes);
                    let before_other = next.is_some_and(|n| {
                        n.kind != Kind::League && n.days_after(sheet) <= Self::CLOSE_DAYS
                    });
                    let after_other = previous.is_some_and(|p| {
                        p.kind != Kind::League && sheet.days_after(p) <= Self::CLOSE_DAYS
                    });
                    if before_other {
                        league_before_other.add(changes);
                    }
                    if after_other {
                        league_after_other.add(changes);
                    }
                    if !before_other && !after_other {
                        league_clear.add(changes);
                    }
                }
                last_league = Some(sheet);
            }
        }

        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::count(
                    format!("sheets.{}", kind.key()),
                    format!("{} sheets read", kind.key()),
                    totals.sheets[kind.slot()],
                    totals.sheets[kind.slot()],
                ),
            );
        }
        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::mean(
                    format!("told.{}.importance", kind.key()),
                    format!("importance a side was told, {} sheets", kind.key()),
                    &totals.told_importance[kind.slot()],
                ),
            );
            report.push(
                tier,
                S,
                Row::mean(
                    format!("told.{}.room", kind.key()),
                    format!("room a side was left, {} sheets", kind.key()),
                    &totals.told_room[kind.slot()],
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.league_to_league",
                "XI changes, one league match to the next",
                &league_to_league,
            ),
        );
        let histogram_total: u64 = histogram.iter().sum();
        for (changes, count) in histogram.iter().enumerate() {
            report.push(
                tier,
                S,
                Row::share(
                    format!("rotation.league_to_league.share_{changes}"),
                    format!(
                        "league sheets with {changes}{} changes",
                        if changes == 6 { " or more" } else { "" }
                    ),
                    *count,
                    histogram_total,
                ),
            );
        }
        for (i, (key, label)) in [
            ("rotation.rest_le4", "XI changes after four days' rest or fewer"),
            ("rotation.rest_5_8", "XI changes after five to eight days' rest"),
            ("rotation.rest_ge9", "XI changes after nine days' rest or more"),
        ]
        .iter()
        .enumerate()
        {
            report.push(tier, S, Row::mean(*key, *label, &by_rest[i]));
        }
        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::mean(
                    format!("rotation.into_{}", kind.key()),
                    format!("XI changes going into a {} sheet", kind.key()),
                    &into[kind.slot()],
                ),
            );
        }
        for (band, name) in Self::STAKES.iter().enumerate() {
            for (side, side_name) in Self::SIDES.iter().enumerate() {
                report.push(
                    tier,
                    S,
                    Row::mean(
                        format!("rotation.stakes.{name}.{side_name}"),
                        format!("XI changes, {side_name} side told importance {name}"),
                        &by_stakes[band][side],
                    ),
                );
            }
        }
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.bigger_fixture_within_4d",
                "XI changes when a fixture that matters more follows within four days",
                &bigger_ahead[1],
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.no_bigger_fixture_within_4d",
                "XI changes when none does",
                &bigger_ahead[0],
            ),
        );
        report.push(
            tier,
            S,
            Row::share(
                "week.training_double_match_days",
                "team-days on which training reads two or more fixtures in the week",
                totals.double_match_days,
                totals.team_days,
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.league_before_other_within_4d",
                "changes from the last league XI, another competition's fixture within four days after",
                &league_before_other,
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.league_after_other_within_4d",
                "changes from the last league XI, another competition's fixture within four days before",
                &league_after_other,
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "rotation.league_clear_of_others",
                "changes from the last league XI, no other competition within four days",
                &league_clear,
            ),
        );
        for (quarter, name) in Self::QUARTERS.iter().enumerate() {
            report.push(
                tier,
                S,
                Row::mean(
                    format!("rotation.quarter_{name}.changes"),
                    format!("changes from the last league XI, {name}% of the season"),
                    &quarter_changes[quarter],
                ),
            );
            report.push(
                tier,
                S,
                Row::mean(
                    format!("rotation.quarter_{name}.key_regular"),
                    format!("Key and Regular starters in a league XI, {name}% of the season"),
                    &quarter_established[quarter],
                ),
            );
            report.push(
                tier,
                S,
                Row::mean(
                    format!("rotation.quarter_{name}.aged_le21"),
                    format!("starters aged 21 or under in a league XI, {name}% of the season"),
                    &quarter_young[quarter],
                ),
            );
        }
    }

    fn keeper_rows(report: &mut Report, tier: u8, totals: &Totals, books: &[&TeamBook]) {
        const S: &str = "S2 goalkeepers";
        let league = Kind::League.slot();
        let mut top_share = Mean::default();
        let mut keepers_used = Mean::default();
        let mut share_bands = [0u64; 5];
        let mut most_used_elsewhere = [(0u64, 0u64); 3];
        let mut declared = [(0u64, 0u64); 3];
        let mut deputy = [0u64; 3];
        let mut declared_clubs = (0u64, 0u64);
        let mut borrowed_first_choice = (0u64, 0u64);
        let mut clubs = 0u64;
        for book in books
            .iter()
            .filter(|b| b.league_matches >= Self::SEASON_SHEETS)
        {
            let total: u32 = book.keeper_starts[league].values().sum();
            let Some((&most_used, &most)) = book.keeper_starts[league]
                .iter()
                .max_by_key(|(id, n)| (**n, std::cmp::Reverse(**id)))
            else {
                continue;
            };
            clubs += 1;
            let share = f64::from(most) / f64::from(total.max(1));
            top_share.add(100.0 * share);
            keepers_used.add(book.keeper_starts[league].len() as f64);
            share_bands[match share {
                s if s >= 0.95 => 0,
                s if s >= 0.85 => 1,
                s if s >= 0.70 => 2,
                s if s >= 0.55 => 3,
                _ => 4,
            }] += 1;
            for kind in Kind::ALL {
                let k = kind.slot();
                most_used_elsewhere[k].0 +=
                    u64::from(book.keeper_starts[k].get(&most_used).copied().unwrap_or(0));
                most_used_elsewhere[k].1 +=
                    book.keeper_starts[k].values().map(|n| u64::from(*n)).sum::<u64>();
                declared[k].0 += book.declared_starts[k].0;
                declared[k].1 += book.declared_starts[k].1;
                deputy[k] += book.deputy_starts[k];
            }
            let (by_declared, with_declaration) = book.declared_starts[league];
            if with_declaration >= u64::from(Self::SEASON_SHEETS) {
                declared_clubs.1 += 1;
                declared_clubs.0 += u64::from(by_declared as f64 >= 0.85 * with_declaration as f64);
            }
            if book.borrowed_keepers.contains(&most_used) {
                borrowed_first_choice.1 += 1;
                borrowed_first_choice.0 += u64::from(book.last_declared != Some(most_used));
            }
        }
        report.push(
            tier,
            S,
            Row::mean(
                "keepers.most_used_share",
                "most-used keeper's share of his club's league starts, %",
                &top_share,
            ),
        );
        report.push(
            tier,
            S,
            Row::share(
                "keepers.clubs_most_used_ge85",
                "clubs whose most-used keeper starts 85% of league matches or more",
                share_bands[0] + share_bands[1],
                clubs,
            ),
        );
        for (band, name) in ["ge95", "85_95", "70_85", "55_70", "lt55"].iter().enumerate() {
            report.push(
                tier,
                S,
                Row::share(
                    format!("keepers.clubs_by_share.{name}"),
                    format!("clubs whose most-used keeper's share of league starts is {name}%"),
                    share_bands[band],
                    clubs,
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::mean(
                "keepers.given_league_start",
                "keepers given a league start, per club",
                &keepers_used,
            ),
        );
        for kind in [Kind::Cup, Kind::Continental] {
            let (part, whole) = most_used_elsewhere[kind.slot()];
            report.push(
                tier,
                S,
                Row::share(
                    format!("keepers.most_used_share_of_{}_starts", kind.key()),
                    format!("the league's most-used keeper's share of {} starts", kind.key()),
                    part,
                    whole,
                ),
            );
        }
        for kind in Kind::ALL {
            let (part, whole) = declared[kind.slot()];
            report.push(
                tier,
                S,
                Row::share(
                    format!("keepers.declared_one_share_of_{}_starts", kind.key()),
                    format!("the declared number one's share of {} starts", kind.key()),
                    part,
                    whole,
                ),
            );
        }
        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::share(
                    format!("keepers.declared_deputy_share_of_{}_starts", kind.key()),
                    format!("the declared deputy's share of {} starts", kind.key()),
                    deputy[kind.slot()],
                    declared[kind.slot()].1,
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::share(
                "keepers.clubs_declared_one_ge85",
                "clubs whose declared number one starts 85% of league matches or more",
                declared_clubs.0,
                declared_clubs.1,
            ),
        );
        report.push(
            tier,
            S,
            Row::count(
                "keepers.borrowed_first_choice_not_declared",
                "clubs whose most-used keeper is borrowed and is not the declared number one",
                borrowed_first_choice.0,
                borrowed_first_choice.1,
            ),
        );
        report.push(
            tier,
            S,
            Row::count(
                "keepers.borrowed_best_not_declared",
                "league sheets where a borrowed keeper clearly the club's best, a month into his loan, is not the declared number one",
                totals.borrowed_best_undeclared,
                totals.borrowed_best_sheets,
            ),
        );
        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::count(
                    format!("keepers.outfielder_in_goal.{}", kind.key()),
                    format!("{} matches started with an outfielder in goal", kind.key()),
                    totals.outfielder_in_goal[kind.slot()],
                    totals.sheets[kind.slot()],
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::count(
                "keepers.outfielder_in_goal.free_fit_keeper",
                "of those, the club had a fit keeper free in another squad",
                totals.outfielder_with_free_keeper,
                totals.outfielder_in_goal.iter().sum(),
            ),
        );
        for line in &totals.outfielder_details {
            report.detail(tier, S, line.clone());
        }
    }

    fn spread_rows(report: &mut Report, tier: u8, totals: &Totals, books: &[&TeamBook]) {
        const S: &str = "S3 how football is spread";
        let mut spread = [[Mean::default(); 3]; 2];
        for book in books
            .iter()
            .filter(|b| b.league_matches >= Self::SEASON_SHEETS)
        {
            for (i, ledger) in [&book.league_starts, &book.league_appearances]
                .iter()
                .enumerate()
            {
                let mut counts: Vec<u32> = ledger.values().copied().collect();
                counts.sort_unstable_by(|a, b| b.cmp(a));
                let total: u32 = counts.iter().sum();
                let threshold = (f64::from(book.league_matches) * 0.8) as u32;
                spread[i][0].add(counts.len() as f64);
                spread[i][1].add(
                    100.0 * f64::from(counts.iter().take(11).sum::<u32>()) / f64::from(total.max(1)),
                );
                spread[i][2].add(counts.iter().filter(|n| **n >= threshold).count() as f64);
            }
        }
        let rows = [
            (
                "spread.players_given_league_start",
                "players given a league start, per club",
                &spread[0][0],
            ),
            (
                "spread.top11_share_of_league_starts",
                "share of a club's league starts taken by its eleven most-used, %",
                &spread[0][1],
            ),
            (
                "spread.players_starting_80pct",
                "players starting 80% of league matches or more, per club",
                &spread[0][2],
            ),
            (
                "spread.players_given_league_appearance",
                "players given a league appearance, per club",
                &spread[1][0],
            ),
            (
                "spread.top11_share_of_league_appearances",
                "share of a club's league appearances taken by its eleven most-used, %",
                &spread[1][1],
            ),
            (
                "spread.players_appearing_80pct",
                "players appearing in 80% of league matches or more, per club",
                &spread[1][2],
            ),
        ];
        for (key, label, mean) in rows {
            report.push(tier, S, Row::mean(key, label, mean));
        }
        report.push(
            tier,
            S,
            Row::share(
                "appearances.league.season_age_le21",
                "league appearances by players of 21 or under, age as season year minus birth year",
                totals.league_appearances_young,
                totals.league_appearances,
            ),
        );
        report.push(
            tier,
            S,
            Row::share(
                "appearances.league.season_age_ge30",
                "league appearances by players of 30 or over, age as season year minus birth year",
                totals.league_appearances_old,
                totals.league_appearances,
            ),
        );
    }

    fn starter_rows(report: &mut Report, tier: u8, totals: &Totals) {
        const S: &str = "S4 who starts";
        for kind in Kind::ALL {
            let k = kind.slot();
            let starts = totals.starts[k];
            if starts == 0 {
                continue;
            }
            let name = kind.key();
            for (band, age) in Self::AGE_BANDS.iter().enumerate() {
                report.push(
                    tier,
                    S,
                    Row::share(
                        format!("starts.{name}.age.{age}"),
                        format!("{name} starts by players aged {age}"),
                        totals.starts_by_age[k][band],
                        starts,
                    ),
                );
            }
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.age_le21"),
                    format!("{name} starts by players aged 21 or under"),
                    totals.starts_by_age[k][..3].iter().sum(),
                    starts,
                ),
            );
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.age_ge30"),
                    format!("{name} starts by players aged 30 or over"),
                    totals.starts_by_age[k][5],
                    starts,
                ),
            );
            for (slot, status) in PlayerFacts::STATUS.iter().enumerate() {
                report.push(
                    tier,
                    S,
                    Row::share(
                        format!("starts.{name}.status.{status}"),
                        format!("{name} starts by squad status {status}"),
                        totals.starts_by_status[k][slot],
                        starts,
                    ),
                );
            }
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.key_regular"),
                    format!("{name} starts by Key and Regular players"),
                    totals.starts_by_status[k][..2].iter().sum(),
                    starts,
                ),
            );
            for slot in 0..10 {
                let role = PlayerFacts::role_key(PlayerFacts::role_of_slot(slot));
                report.push(
                    tier,
                    S,
                    Row::share(
                        format!("starts.{name}.role.{role}"),
                        format!("{name} starts by planned role {role}"),
                        totals.starts_by_role[k][slot],
                        starts,
                    ),
                );
            }
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.listed"),
                    format!("{name} starts by transfer-listed players"),
                    totals.starts_listed[k],
                    starts,
                ),
            );
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.first_choice"),
                    format!("{name} starts by the plan's first choice for a shirt"),
                    totals.starts_first_choice[k],
                    starts,
                ),
            );
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.shop_window"),
                    format!("{name} starts by players in the shop window"),
                    totals.starts_shop_window[k],
                    starts,
                ),
            );
            report.push(
                tier,
                S,
                Row::share(
                    format!("starts.{name}.other_squad"),
                    format!("{name} starts by players named from another of the club's squads"),
                    totals.starts_other_squad[k],
                    starts,
                ),
            );
        }
        let starts: u64 = totals.starts.iter().sum();
        report.push(
            tier,
            S,
            Row::count(
                "starts.against_parent_club",
                "starts by a borrowed player against the club that owns him",
                totals.starts_against_parent,
                starts,
            ),
        );
        report.push(
            tier,
            S,
            Row::count(
                "starts.cup_tied",
                "cup starts by a player who had appeared for another club in the same edition",
                totals.starts_cup_tied,
                totals.starts[Kind::Cup.slot()],
            ),
        );
        report.push(
            tier,
            S,
            Row::count(
                "starts.not_at_the_club",
                "starters who were at the club in no squad that morning",
                totals.starts_unknown,
                starts,
            ),
        );
    }

    fn plan_rows(&self, report: &mut Report, tier: u8, totals: &Totals) {
        const S: &str = "S5 planned against played";
        let seasons = f64::from(self.days.max(1)) / 365.0;
        report.push(
            tier,
            S,
            Row::mean(
                "plan.shares_per_side",
                "football the plan gives out across a side that has one, in shirts",
                &totals.plan_shares,
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "plan.first_choices_per_side",
                "players a side's plan names first choice for a shirt",
                &totals.plan_first_choices,
            ),
        );
        for slot in 0..10 {
            let role = PlayerFacts::role_key(PlayerFacts::role_of_slot(slot));
            let fixtures = totals.plan_fixtures[slot];
            let share = |sum: f64| sum / (fixtures.max(1) as f64);
            let error = |p: f64| 100.0 * (p * (1.0 - p).max(0.0) / fixtures.max(1) as f64).sqrt();
            let planned = share(totals.plan_planned[slot]);
            let played = share(totals.plan_played[slot]);
            report.push(
                tier,
                S,
                Row::figure(
                    format!("plan.{role}.planned_share"),
                    format!("football the plan gives a {role}, % of the fixtures he was fit for"),
                    100.0 * planned,
                    0.0,
                    fixtures,
                    Unit::Percent,
                ),
            );
            report.push(
                tier,
                S,
                Row::figure(
                    format!("plan.{role}.played_share"),
                    format!("football a {role} played, % of the fixtures he was fit for"),
                    100.0 * played,
                    error(played),
                    fixtures,
                    Unit::Percent,
                ),
            );
        }
        for slot in 0..10 {
            let role = PlayerFacts::role_key(PlayerFacts::role_of_slot(slot));
            let players = totals.role_days[slot] as f64 / f64::from(self.days.max(1));
            let scale = 100.0 / players.max(1.0) / seasons;
            let complaints = totals.complaints[slot] as f64;
            report.push(
                tier,
                S,
                Row::figure(
                    format!("complaints.{role}"),
                    format!(
                        "playing-time complaints a season per 100 first-team players planned as {role}"
                    ),
                    complaints * scale,
                    complaints.sqrt() * scale,
                    players.round() as u64,
                    Unit::Mean,
                ),
            );
        }
    }

    fn bench_rows(report: &mut Report, tier: u8, totals: &Totals) {
        const S: &str = "S6 bench";
        for kind in Kind::ALL {
            report.push(
                tier,
                S,
                Row::mean(
                    format!("bench.size.{}", kind.key()),
                    format!("bench size, {}", kind.key()),
                    &totals.bench_size[kind.slot()],
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::mean("bench.subs_used", "substitutes used a match", &totals.subs_used),
        );
        let benches: u64 = totals.bench_keepers.iter().sum();
        for (count, (name, label)) in [
            ("none", "benches naming no keeper"),
            ("one", "benches naming one keeper"),
            ("two_plus", "benches naming two or more keepers"),
        ]
        .iter()
        .enumerate()
        {
            report.push(
                tier,
                S,
                Row::share(
                    format!("bench.keepers.{name}"),
                    *label,
                    totals.bench_keepers[count],
                    benches,
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::share(
                "bench.keepers.two_plus.le9",
                "benches of nine or fewer naming two or more keepers",
                totals.bench_keepers_small[2],
                totals.bench_keepers_small.iter().sum(),
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "bench.uncovered_lines",
                "lines of the XI with no cover on the bench, per sheet",
                &totals.uncovered_lines,
            ),
        );
        for line in PitchLine::ALL {
            report.push(
                tier,
                S,
                Row::share(
                    format!("bench.uncovered.{}", line.key()),
                    format!("sheets fielding {} with no cover for it on the bench", line.key()),
                    totals.line_uncovered[line as usize],
                    totals.line_fielded[line as usize],
                ),
            );
        }
    }

    fn preseason_rows(report: &mut Report, tier: u8, totals: &Totals, books: &[&TeamBook]) {
        const S: &str = "S7 first-team friendlies";
        report.push(
            tier,
            S,
            Row::count(
                "preseason.sheets",
                "first-team friendly sheets read",
                totals.preseason_sheets,
                totals.preseason_sheets,
            ),
        );
        report.push(
            tier,
            S,
            Row::mean(
                "preseason.changes",
                "XI changes, one friendly to the next",
                &totals.preseason_changes,
            ),
        );
        let mut distinct = Mean::default();
        for book in books.iter().filter(|b| b.friendlies >= 3) {
            distinct.add(book.friendly_starters.len() as f64);
        }
        report.push(
            tier,
            S,
            Row::mean(
                "preseason.distinct_starters",
                "players given a friendly start, per club with three friendlies or more",
                &distinct,
            ),
        );
        for slot in 0..10 {
            let role = PlayerFacts::role_key(PlayerFacts::role_of_slot(slot));
            report.push(
                tier,
                S,
                Row::share(
                    format!("preseason.starts.role.{role}"),
                    format!("friendly starts by planned role {role}"),
                    totals.preseason_by_role[slot],
                    totals.preseason_starts,
                ),
            );
        }
        report.push(
            tier,
            S,
            Row::share(
                "preseason.starts.listed",
                "friendly starts by transfer-listed players",
                totals.preseason_listed,
                totals.preseason_starts,
            ),
        );
        report.push(
            tier,
            S,
            Row::share(
                "preseason.starts.age_le21",
                "friendly starts by players aged 21 or under",
                totals.preseason_young,
                totals.preseason_starts,
            ),
        );
    }

    fn young_rows(&self, report: &mut Report, tier: u8, totals: &Totals) {
        const S: &str = "S8 first-team players aged 21 or under";
        let days = f64::from(self.days.max(1));
        let players = totals.young_days as f64 / days;
        let seasons = days / 365.0;
        let rows = [
            (
                "young.first_team_appearances",
                "competitive first-team appearances a season, per player",
                totals.young_first_team,
            ),
            (
                "young.first_team_friendly_appearances",
                "first-team friendly appearances a season, per player",
                totals.young_first_team_friendly,
            ),
            (
                "young.other_side_appearances",
                "appearances for another of the club's sides a season, per player",
                totals.young_other_sides,
            ),
            (
                "young.appearances",
                "appearances for any side of the club a season, per player",
                totals.young_first_team + totals.young_first_team_friendly + totals.young_other_sides,
            ),
        ];
        let scale = 1.0 / players.max(1.0) / seasons;
        for (key, label, appearances) in rows {
            let appearances = appearances as f64;
            report.push(
                tier,
                S,
                Row::figure(
                    key,
                    label,
                    appearances * scale,
                    appearances.sqrt() * scale,
                    players.round() as u64,
                    Unit::Mean,
                ),
            );
        }
    }
}
