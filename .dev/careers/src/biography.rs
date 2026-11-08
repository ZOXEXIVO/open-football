//! What someone following one man would read: a line a year and a line for
//! everything that happened to him.

use crate::cohort::Cohort;
use crate::reading::{MoveKind, MoveRow, Place, Snap};
use chrono::NaiveDate;
use core::club::player::mind::{GoalKind, MindClock};
use core::club::staff::perception::AbilityEstimator;
use core::{Player, SimulatorData};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

const RENEWAL_REJECTED: &str = "dec_contract_renewal_rejected";
const RENEWAL_SIGNED: &str = "dec_contract_renewal_signed";

pub struct Bio {
    id: u32,
    name: String,
    cohort: Cohort,
    birth: NaiveDate,
    ca0: u8,
    pa0: u8,
    ambition: f32,
    loyalty: f32,
    start_club_id: u32,
    start_club_name: String,
    start_club_rep: f32,
    last: Option<Snap>,
    last_club_name: String,
    lines: Vec<String>,
    best_league_regular: u16,
    best_rank_regular: u32,
    sold: u32,
    free_moves: u32,
    loans: u32,
    days_unemployed: u32,
    request_spells: u32,
    renewals: u32,
    refusals: u32,
    arc_changes: u32,
    wants_seen: HashSet<GoalKind>,
    retired_age: Option<u8>,
    flags: Vec<String>,
    final_ca: u8,
    last_arc: Option<&'static str>,
    last_rejects: u32,
    last_signed: u32,
}

#[derive(Default)]
pub struct Biographies {
    bios: HashMap<u32, Bio>,
}

impl Biographies {
    /// Everyone chosen, from one walk of the world.
    pub fn follow(data: &SimulatorData, ranks: &crate::reading::ClubRanks, chosen: &HashMap<u32, Cohort>) -> Self {
        let mut bios = HashMap::with_capacity(chosen.len());
        crate::reading::WorldWalk::players(data, ranks, |p, place| {
            if let Some(cohort) = chosen.get(&p.id) {
                bios.insert(p.id, Bio::new(p, *cohort, place));
            }
        });
        Biographies { bios }
    }

    pub fn men(&self) -> usize {
        self.bios.len()
    }

    pub fn on_day(
        &mut self,
        p: &Player,
        now: &Snap,
        place: &Place,
        date: NaiveDate,
        yearly: bool,
        rows: Option<&Vec<MoveRow>>,
    ) {
        if let Some(bio) = self.bios.get_mut(&p.id) {
            bio.on_day(p, now, place.club_name, date, yearly, rows);
        }
    }

    pub fn on_retired(&mut self, p: &Player, date: NaiveDate) {
        if let Some(bio) = self.bios.get_mut(&p.id)
            && bio.retired_age.is_none()
        {
            let age = core::Person::age(p, date);
            bio.retired_age = Some(age);
            bio.final_ca = p.player_attributes.current_ability;
            let status = bio.last.map_or("-", |s| s.status);
            bio.lines.push(format!(
                "{date} RETIRED at {age}, CA{} after {} ({status}), wants {}",
                p.player_attributes.current_ability,
                bio.last_club_name,
                Self::wants_line(p),
            ));
        }
    }

    pub fn wants_line(p: &Player) -> String {
        let mut wants: Vec<(GoalKind, f32, &'static str)> = p
            .mind
            .goals()
            .live()
            .map(|g| {
                let rung = if g.status.is_public() { "!" } else { "" };
                (g.kind, g.pressure(), rung)
            })
            .collect();
        wants.sort_by(|a, b| b.1.total_cmp(&a.1));
        let parts: Vec<String> = wants
            .iter()
            .take(3)
            .map(|(kind, pressure, rung)| format!("{kind:?}{rung}{pressure:.1}"))
            .collect();
        if parts.is_empty() {
            "-".into()
        } else {
            parts.join(" ")
        }
    }

    pub fn report(&self, per_cohort: usize) {
        for cohort in Cohort::ALL {
            let mut group: Vec<&Bio> = self.bios.values().filter(|b| b.cohort == cohort).collect();
            if group.is_empty() {
                println!("\n#### {cohort:?}: nobody followed");
                continue;
            }
            group.sort_by_key(|b| (std::cmp::Reverse(b.ca0), b.id));
            Bio::report_cohort(cohort, &group, per_cohort);
        }
    }
}

impl Bio {
    fn new(p: &Player, cohort: Cohort, place: &Place) -> Self {
        Bio {
            id: p.id,
            name: p.full_name.to_string(),
            cohort,
            birth: p.birth_date,
            ca0: p.player_attributes.current_ability,
            pa0: p.player_attributes.potential_ability,
            ambition: p.attributes.ambition,
            loyalty: p.attributes.loyalty,
            start_club_id: place.club_id,
            start_club_name: place.club_name.to_string(),
            start_club_rep: place.club_rep,
            last: None,
            last_club_name: place.club_name.to_string(),
            lines: Vec::new(),
            best_league_regular: 0,
            best_rank_regular: u32::MAX,
            sold: 0,
            free_moves: 0,
            loans: 0,
            days_unemployed: 0,
            request_spells: 0,
            renewals: 0,
            refusals: 0,
            arc_changes: 0,
            wants_seen: HashSet::new(),
            retired_age: None,
            flags: Vec::new(),
            final_ca: p.player_attributes.current_ability,
            last_arc: None,
            last_rejects: 0,
            last_signed: 0,
        }
    }

    fn rank_label(rank: u32) -> String {
        if rank == u32::MAX {
            "-".into()
        } else {
            rank.to_string()
        }
    }

    fn on_day(
        &mut self,
        p: &Player,
        now: &Snap,
        club_name: &str,
        date: NaiveDate,
        yearly: bool,
        rows: Option<&Vec<MoveRow>>,
    ) {
        let age = now.age;
        self.final_ca = now.ca;
        if now.club_id == 0 {
            self.days_unemployed += 1;
        }
        if now.regular && !now.on_loan {
            self.best_league_regular = self.best_league_regular.max(now.league_rep);
            self.best_rank_regular = self.best_rank_regular.min(now.rank);
        }
        for goal in p.mind.goals().live() {
            self.wants_seen.insert(goal.kind);
        }
        self.read_chapters(p, date);

        let before = self.last.unwrap_or(*now);
        let first_day = self.last.is_none();

        if now.req && (!before.req || first_day) {
            self.request_spells += 1;
            self.lines.push(format!(
                "{date} ({age}) asks to leave {club_name} ({}), reasons {:?}; wants {}",
                now.status,
                p.transfer_request_reasons,
                Biographies::wants_line(p)
            ));
        } else if !now.req && before.req && before.club_id == now.club_id {
            self.lines
                .push(format!("{date} ({age}) withdraws his request at {club_name}"));
        }
        if now.listed && !before.listed && before.club_id == now.club_id {
            self.lines.push(format!(
                "{date} ({age}) listed by {club_name} ({}, {}d left) {}",
                now.status,
                now.days_left,
                p.decision_history
                    .items
                    .iter()
                    .rev()
                    .take_while(|d| (date - d.date).num_days() <= 1)
                    .find(|d| d.decision.starts_with("dec_reason"))
                    .map_or("(no register row today)", |d| d.decision.as_str()),
            ));
        } else if !now.listed && before.listed && before.club_id == now.club_id {
            self.lines
                .push(format!("{date} ({age}) comes off the list at {club_name}"));
        }

        let arc = now.arc.map(|a| a.as_token());
        if arc != self.last_arc {
            self.arc_changes += 1;
            self.last_arc = arc;
        }

        let rejects = Self::decisions(p, RENEWAL_REJECTED);
        if rejects > self.last_rejects {
            self.refusals += rejects - self.last_rejects;
            self.lines.push(format!(
                "{date} ({age}) turns down new terms at {club_name} ({}d left); wants {}",
                now.days_left,
                Biographies::wants_line(p)
            ));
            self.last_rejects = rejects;
        }
        let signed = Self::decisions(p, RENEWAL_SIGNED);
        if signed > self.last_signed {
            self.renewals += signed - self.last_signed;
            self.lines.push(format!(
                "{date} ({age}) re-signs at {club_name} ({})",
                now.status
            ));
            self.last_signed = signed;
        }

        if !first_day && before.club_id != now.club_id {
            self.on_move(p, &before, now, club_name, date, rows);
        }
        if yearly {
            self.on_year(p, now, club_name, date, arc);
        }
        self.last = Some(*now);
        if before.club_id != now.club_id || first_day {
            self.last_club_name = club_name.to_string();
        }
    }

    /// The turns his career has written since yesterday, and the ones it
    /// came into the census with.
    fn read_chapters(&mut self, p: &Player, date: NaiveDate) {
        let today = MindClock::day(date);
        for chapter in p.mind.career.anchors.chapters.iter() {
            let is_new = if self.last.is_none() {
                chapter.on <= today
            } else {
                chapter.on == today
            };
            if is_new {
                self.lines.push(format!(
                    "{} chapter {:?} ({:?}) club {} lg {}",
                    MindClock::date(chapter.on),
                    chapter.kind,
                    chapter.dream,
                    chapter.club_id,
                    chapter.league_reputation,
                ));
            }
        }
    }

    fn decisions(p: &Player, decision: &str) -> u32 {
        p.decision_history
            .items
            .iter()
            .filter(|d| d.decision == decision)
            .count() as u32
    }

    fn on_move(
        &mut self,
        p: &Player,
        before: &Snap,
        now: &Snap,
        club_name: &str,
        date: NaiveDate,
        rows: Option<&Vec<MoveRow>>,
    ) {
        let age = now.age;
        let row = rows.and_then(|r| {
            r.iter()
                .find(|x| x.fee > 0.0)
                .or_else(|| r.iter().find(|x| x.kind != MoveKind::Loan))
                .or_else(|| r.first())
        });
        let fee = row.map_or(0.0, |r| r.fee);
        let reason = row.map_or("", |r| r.reason.as_str());
        let word = if now.on_loan && !before.on_loan {
            self.loans += 1;
            "LOAN"
        } else if before.on_loan && !now.on_loan && now.club_id != 0 {
            "back from loan"
        } else if now.club_id == 0 {
            self.free_moves += 1;
            "RELEASED"
        } else if before.club_id == 0 {
            "signs from the pool"
        } else if fee > 0.0 {
            self.sold += 1;
            "SOLD"
        } else {
            self.free_moves += 1;
            "FREE"
        };
        self.lines.push(format!(
            "{date} ({age}) {word}: {} [rank {}, lg {}] -> {club_name} [rank {}, lg {}] fee {fee:.0} {reason} ({} {}d left; lst{} req{} arc {:?}) wants {}",
            self.last_club_name,
            Self::rank_label(before.rank),
            before.league_rep,
            Self::rank_label(now.rank),
            now.league_rep,
            before.status,
            before.days_left,
            before.listed as u8,
            before.req as u8,
            now.arc.map(|a| a.as_token()),
            Biographies::wants_line(p),
        ));

        if before.regular && before.rank < 250 && age <= 30 && matches!(word, "RELEASED" | "FREE") {
            self.flags.push(format!(
                "left for nothing from rank {} at {age} ({})",
                before.rank, self.last_club_name
            ));
        }
        let drop = before.league_rep as i32 - now.league_rep as i32;
        if word != "LOAN"
            && before.club_id != 0
            && now.club_id != 0
            && before.regular
            && age <= 29
            && drop >= 2000
        {
            self.flags
                .push(format!("dropped {drop} league reputation at {age} as a regular"));
        }
        if word == "LOAN" && age >= 27 {
            self.flags.push(format!("loaned out at {age}"));
        }
    }

    fn on_year(&mut self, p: &Player, now: &Snap, club_name: &str, date: NaiveDate, arc: Option<&'static str>) {
        let anchors = &p.mind.career.anchors;
        let dream = anchors.dream.map_or("-".to_string(), |d| {
            format!(
                "{:?} target {:.2} club {} conviction {:.2} {:?}",
                d.kind,
                d.target_standing,
                d.target_club,
                d.conviction,
                d.state()
            )
        });
        let summit = anchors.summit.map_or("-".to_string(), |s| {
            format!(
                "lg {} held {}",
                s.league_reputation,
                s.held(
                    MindClock::day(date),
                    now.age,
                    AbilityEstimator::observable_level(p)
                )
            )
        });
        let last_season = p
            .statistics_history
            .items
            .last()
            .map_or("-".to_string(), |i| {
                format!(
                    "{}+{}apps {}g r{:.2}",
                    i.statistics.played,
                    i.statistics.played_subs,
                    i.statistics.goals,
                    i.statistics.mean_rating()
                )
            });
        self.lines.push(format!(
            "{date} ({}) CA{} rep{} ${:.1}M {club_name} [rank {}, lg {}] {} start {:.2} {}d left | last {last_season} | plan {:?} | dream {dream} | summit {summit} | wants {} | morale {:.0}",
            now.age,
            now.ca,
            now.rep,
            now.salary as f64 / 1_000_000.0,
            Self::rank_label(now.rank),
            now.league_rep,
            now.status,
            now.starter,
            now.days_left,
            arc,
            Biographies::wants_line(p),
            p.happiness.morale,
        ));
    }

    fn report_cohort(cohort: Cohort, group: &[&Bio], per_cohort: usize) {
        let n = group.len() as f64;
        let retired = group.iter().filter(|b| b.retired_age.is_some()).count();
        let unemployed = group
            .iter()
            .filter(|b| b.retired_age.is_none() && b.last.is_some_and(|l| l.club_id == 0))
            .count();
        let total = |read: fn(&Bio) -> u32| group.iter().map(|b| read(b)).sum::<u32>() as f64 / n;
        let mut retire_ages: Vec<u8> = group.iter().filter_map(|b| b.retired_age).collect();
        retire_ages.sort_unstable();

        let (mut same_club, mut bigger, mut smaller) = (0, 0, 0);
        for b in group {
            let Some(last) = b.last else { continue };
            if last.club_id == b.start_club_id {
                same_club += 1;
            } else if last.club_rep > b.start_club_rep + 0.03 {
                bigger += 1;
            } else if last.club_rep + 0.03 < b.start_club_rep {
                smaller += 1;
            }
        }
        let mut unemployed_days: Vec<u32> = group.iter().map(|b| b.days_unemployed).collect();
        unemployed_days.sort_unstable();

        let mut want_counts: BTreeMap<String, u32> = BTreeMap::new();
        for b in group {
            for kind in &b.wants_seen {
                *want_counts.entry(format!("{kind:?}")).or_insert(0) += 1;
            }
        }
        let mut want_rows: Vec<(String, u32)> = want_counts.into_iter().collect();
        want_rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        let wants: Vec<String> = want_rows
            .iter()
            .take(8)
            .map(|(kind, count)| format!("{kind} {:.0}%", *count as f64 / n * 100.0))
            .collect();

        println!("\n#### {cohort:?} (n={})", group.len());
        println!(
            "  per man: sold {:.2}, free or released {:.2}, loans {:.2}, requests {:.2}, renewals {:.2}, refusals {:.2}, plan changes {:.1}",
            total(|b| b.sold),
            total(|b| b.free_moves),
            total(|b| b.loans),
            total(|b| b.request_spells),
            total(|b| b.renewals),
            total(|b| b.refusals),
            total(|b| b.arc_changes),
        );
        println!(
            "  now: retired {retired} (median age {}), unemployed {unemployed}, same club {same_club}, bigger club {bigger}, smaller club {smaller}, days unemployed p50 {} p90 {}, men flagged {}",
            retire_ages.get(retire_ages.len() / 2).copied().unwrap_or(0),
            unemployed_days[unemployed_days.len() / 2],
            unemployed_days[unemployed_days.len() * 9 / 10],
            group.iter().filter(|b| !b.flags.is_empty()).count(),
        );
        println!("  wants ever held: {}", wants.join(", "));

        let mut printed: HashSet<u32> = HashSet::new();
        let mut by_flags = group.to_vec();
        by_flags.sort_by_key(|b| (std::cmp::Reverse(b.flags.len()), std::cmp::Reverse(b.ca0), b.id));
        for b in by_flags
            .iter()
            .take(per_cohort / 2)
            .chain(group.iter().take(per_cohort - per_cohort / 2))
        {
            if printed.insert(b.id) {
                b.print();
            }
        }
    }

    fn print(&self) {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "\n  -- {} (b.{}) CA{} PA{} amb {:.0} loy {:.0}, from {}, final CA{}, best as a regular: rank {} lg {}",
            self.name,
            self.birth,
            self.ca0,
            self.pa0,
            self.ambition,
            self.loyalty,
            self.start_club_name,
            self.final_ca,
            Self::rank_label(self.best_rank_regular),
            self.best_league_regular
        );
        if !self.flags.is_empty() {
            let _ = writeln!(out, "     FLAGS: {:?}", self.flags);
        }
        for line in &self.lines {
            let _ = writeln!(out, "     {line}");
        }
        print!("{out}");
    }
}
