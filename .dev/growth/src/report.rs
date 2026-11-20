//! The census tables.
//!
//! Every data row is `label | cell | cell ...`, so two reports can be laid
//! side by side by `--compare`. A count inside a cell is written `nN`; a cell
//! holding fewer than `ReportComparison::MIN_N` players is too small to read
//! a difference from. Lines starting with `#` or `-` are notes and headers.

use crate::snapshot::{SKILLS, Snap, YearlySnapshots};
use crate::stats::Stats;
use crate::tracker::Role;
use crate::world::{GROUPS, Kind, Place};
use std::collections::{BTreeMap, BTreeSet};

pub struct Report<'a> {
    snaps: &'a YearlySnapshots,
}

impl<'a> Report<'a> {
    const ROLES: [Role; 5] = [
        Role::Regular,
        Role::Rotation,
        Role::Fringe,
        Role::YouthFootball,
        Role::None,
    ];
    const VETERAN_WATCH: [&'static str; 10] = [
        "Pace",
        "Acceleration",
        "Agility",
        "Stamina",
        "Strength",
        "Jumping",
        "Passing",
        "Technique",
        "Decisions",
        "Positioning",
    ];

    pub fn new(snaps: &'a YearlySnapshots) -> Self {
        Report { snaps }
    }

    pub fn print(&self) {
        println!(
            "\n================ DEVELOPMENT CENSUS: {} yearly snapshots ================",
            self.snaps.years.len()
        );
        println!("# y0 is the opening world: the shipped database as loaded.");
        self.places();
        self.ratio_by_age();
        self.maxed_share();
        self.yearly_change();
        self.loan_question();
        self.profiles();
        self.same_men();
        self.who_plays();
        self.line_by_tier();
        self.the_young();
        self.youth_football();
        self.veterans();
        self.sources();
        self.fitting_football();
        self.football_shows();
        self.talent_supply();
        self.academy_levels();
        self.first_teams_by_kind();
        self.same_men_top_clubs();
        self.character();
    }

    /// Snapshots are hash maps: a table that walks players in order walks
    /// them by id, so float sums come out the same on every run.
    fn sorted(&self, y: usize) -> Vec<(&u32, &Snap)> {
        let mut v: Vec<(&u32, &Snap)> = self.snaps.years[y].iter().collect();
        v.sort_by_key(|(id, _)| **id);
        v
    }

    fn last(&self) -> usize {
        self.snaps.last()
    }

    fn picks(&self) -> Vec<usize> {
        let last = self.last();
        [0, last / 2, last]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn places(&self) {
        println!("\n--- R0: world by place, per year ---");
        for (y, snaps) in self.snaps.years.iter().enumerate() {
            let mut counts = [0u32; 5];
            for s in snaps.values() {
                counts[s.place as usize] += 1;
            }
            let cells: String = Place::ALL
                .iter()
                .map(|p| format!(" | {} n{}", p.name(), counts[*p as usize]))
                .collect();
            println!("year {y} | total n{}{cells}", snaps.len());
        }
    }

    fn ratio_by_age(&self) {
        println!(
            "\n--- R1: CA/PA by age and group, PA >= 110, in a club squad; db = the opening world ---"
        );
        println!(
            "# cells: year, n, p10 p25 p50 p75 p90; last: the last year's median less the db's"
        );
        let picks = self.picks();
        let last = self.last();
        for (g, group) in GROUPS.iter().enumerate() {
            for age in 16u8..=37 {
                let mut line = format!("{group:>4} {age:>3}");
                let mut medians = Vec::new();
                for &y in &picks {
                    let v = Stats::sorted(
                        self.snaps.years[y]
                            .values()
                            .filter(|s| {
                                s.kind.group() == g
                                    && s.age == age
                                    && s.pa >= 110
                                    && s.place.in_squad()
                            })
                            .map(|s| s.ratio())
                            .collect(),
                    );
                    let label = if y == 0 {
                        "db".to_string()
                    } else {
                        format!("y{y}")
                    };
                    line += &format!(
                        " | {label} n{:<5} {:.3} {:.3} {:.3} {:.3} {:.3}",
                        v.len(),
                        Stats::pct(&v, 0.10),
                        Stats::pct(&v, 0.25),
                        Stats::pct(&v, 0.50),
                        Stats::pct(&v, 0.75),
                        Stats::pct(&v, 0.90)
                    );
                    medians.push((y, v.len(), Stats::pct(&v, 0.50)));
                }
                if last > 0 {
                    let (_, n0, db) = medians[0];
                    let (_, n1, now) = medians[medians.len() - 1];
                    line += &format!(" | y{last}-db n{} {:+.3}", n0.min(n1), now - db);
                }
                println!("{line}");
            }
        }
    }

    fn maxed_share(&self) {
        println!(
            "\n--- R2: share of players at CA >= 0.98 PA, by age band and potential band, per year ---"
        );
        for (lo, hi, label) in [
            (0u8, 109u8, "PA<110"),
            (110, 139, "PA110-139"),
            (140, 200, "PA>=140"),
        ] {
            for (alo, ahi) in [(19u8, 21u8), (22, 24), (25, 29), (30, 33)] {
                let mut line = format!("{label:>10} age {alo}-{ahi}");
                for (y, snaps) in self.snaps.years.iter().enumerate() {
                    let pool: Vec<&Snap> = snaps
                        .values()
                        .filter(|s| {
                            s.pa >= lo
                                && s.pa <= hi
                                && s.age >= alo
                                && s.age <= ahi
                                && s.place.in_squad()
                        })
                        .collect();
                    let maxed = pool.iter().filter(|s| s.ratio() >= 0.98).count();
                    line += &format!(
                        " | y{y} {:.1}% n{}",
                        100.0 * maxed as f32 / pool.len().max(1) as f32,
                        pool.len()
                    );
                }
                println!("{line}");
            }
        }
    }

    fn yearly_change(&self) {
        let last = self.last();
        for (id, title, range) in [
            (
                "R3a",
                "first year only (the database meeting the model)",
                0..1usize,
            ),
            ("R3b", "later years", 1..last),
        ] {
            println!(
                "\n--- {id}: mean CA change over a year by age, group and football played: {title} ---"
            );
            println!(
                "# roles by official minutes: regular >= 1800, rotation >= 700, youthfb < 700 official and >= 15 friendly-flagged apps"
            );
            for (g, group) in GROUPS.iter().enumerate() {
                for age in 15u8..=36 {
                    let mut cells: Vec<Vec<f32>> = vec![Vec::new(); 5];
                    for y in range.clone() {
                        if y + 1 > last {
                            continue;
                        }
                        for (pid, a) in self.sorted(y) {
                            if a.kind.group() != g || a.age != age || a.place == Place::Free {
                                continue;
                            }
                            let Some(b) = self.snaps.years[y + 1].get(pid) else {
                                continue;
                            };
                            cells[b.track.role() as usize].push(b.ca as f32 - a.ca as f32);
                        }
                    }
                    let line: String = Self::ROLES
                        .iter()
                        .map(|role| {
                            let c = &cells[*role as usize];
                            if c.len() < 8 {
                                format!(" | {:>8} -", role.name())
                            } else {
                                format!(
                                    " | {:>8} {:+6.1} n{:<5}",
                                    role.name(),
                                    Stats::mean(c),
                                    c.len()
                                )
                            }
                        })
                        .collect();
                    println!("{group:>4} {age:>3}{line}");
                }
            }
        }
    }

    fn loan_question(&self) {
        let last = self.last();
        println!(
            "\n--- R4: the loan question. Players aged 17-22 at the start of a year, PA >= 110, years 1+ ---"
        );
        println!(
            "# path = where he spent the year; cells: n, official minutes, friendly apps, mean dCA, mean d(CA/PA), football banked at year end, mean fit of his minutes"
        );
        for (alo, ahi) in [(17u8, 18u8), (19, 20), (21, 22)] {
            for (g, group) in GROUPS.iter().enumerate() {
                let mut rows: BTreeMap<String, Vec<(f32, f32, f32, f32, f32, f32)>> =
                    BTreeMap::new();
                for y in 1..last {
                    for (pid, a) in self.sorted(y) {
                        if a.kind.group() != g || a.age < alo || a.age > ahi || a.pa < 110 {
                            continue;
                        }
                        let Some(b) = self.snaps.years[y + 1].get(pid) else {
                            continue;
                        };
                        let t = &b.track;
                        let path = if t.on_loan() {
                            format!("loan/{}", t.role().name())
                        } else {
                            format!("{}/{}", t.place().name(), t.role().name())
                        };
                        rows.entry(path).or_default().push((
                            t.minutes,
                            t.friendly_apps as f32,
                            b.ca as f32 - a.ca as f32,
                            b.ratio() - a.ratio(),
                            b.banked,
                            if b.minutes > 0.0 {
                                b.fitting_minutes / b.minutes
                            } else {
                                0.0
                            },
                        ));
                    }
                }
                for (path, v) in rows {
                    if v.len() < 10 {
                        continue;
                    }
                    let n = v.len() as f32;
                    let mean = |f: fn(&(f32, f32, f32, f32, f32, f32)) -> f32| {
                        v.iter().map(f).sum::<f32>() / n
                    };
                    println!(
                        "{group:>4} {alo}-{ahi} {path:>18} | n{:<5} min{:<6.0} fr{:<5.1} dCA{:+.1} dRatio{:+.3} banked{:.2} fit{:.2}",
                        v.len(),
                        mean(|x| x.0),
                        mean(|x| x.1),
                        mean(|x| x.2),
                        mean(|x| x.3),
                        mean(|x| x.4),
                        mean(|x| x.5),
                    );
                }
            }
        }
    }

    fn regulars_24_29(&self, y: usize, kind: Kind) -> Vec<&Snap> {
        self.snaps.years[y]
            .values()
            .filter(|s| {
                s.kind == kind
                    && s.age >= 24
                    && s.age <= 29
                    && s.place == Place::Main
                    && s.pa >= 100
                    && (y == 0 || s.track.role() == Role::Regular)
                    && (y != 0 || s.banked >= 0.7)
            })
            .collect()
    }

    /// The attributes a kind carries: every one but match readiness, and no
    /// goalkeeping for an outfielder.
    fn attributes_of(&self, kind: Kind) -> Vec<usize> {
        (0..SKILLS)
            .filter(|i| {
                let name = self.snaps.names[*i].as_str();
                name != "MatchReadiness" && (kind == Kind::GK || !name.starts_with("Gk"))
            })
            .collect()
    }

    fn profiles(&self) {
        let last = self.last();
        println!(
            "\n--- R5: attribute profile of main-team regulars aged 24-29: db against the last year ---"
        );
        println!("# cells: mean and sd across players; * marks a mean moved by 0.5 or more");
        for kind in Kind::ALL {
            let (a, b) = (
                self.regulars_24_29(0, kind),
                self.regulars_24_29(last, kind),
            );
            if a.len() < 20 || b.len() < 20 {
                continue;
            }
            let ca =
                |v: &Vec<&Snap>| Stats::mean(&v.iter().map(|s| s.ca as f32).collect::<Vec<_>>());
            let mut sd_sum = (0.0f32, 0.0f32, 0u32);
            let mut rows = Vec::new();
            for i in self.attributes_of(kind) {
                let va: Vec<f32> = a.iter().map(|s| s.skills[i]).collect();
                let vb: Vec<f32> = b.iter().map(|s| s.skills[i]).collect();
                let (ma, mb) = (Stats::mean(&va), Stats::mean(&vb));
                if ma < 1.5 && mb < 1.5 {
                    continue;
                }
                let (sa, sb) = (Stats::sd(&va), Stats::sd(&vb));
                sd_sum.0 += sa;
                sd_sum.1 += sb;
                sd_sum.2 += 1;
                let mover = if (mb - ma).abs() >= 0.5 { " *" } else { "" };
                rows.push(format!(
                    "{kind:?} {} | db n{} {ma:.1} sd{sa:.2} | y{last} n{} {mb:.1} sd{sb:.2} | n{} moved {:+.1}{mover}",
                    self.snaps.names[i],
                    a.len(),
                    b.len(),
                    a.len().min(b.len()),
                    mb - ma
                ));
            }
            println!(
                "{kind:?} all | db n{} CA{:.1} sd{:.2} | y{last} n{} CA{:.1} sd{:.2}",
                a.len(),
                ca(&a),
                sd_sum.0 / sd_sum.2 as f32,
                b.len(),
                ca(&b),
                sd_sum.1 / sd_sum.2 as f32
            );
            for row in rows {
                println!("{row}");
            }
        }
    }

    fn same_men(&self) {
        let last = self.last();
        println!(
            "\n--- R6: the same men. In a first team at year 0 and in a club at the last year, by age at year 0 and kind: mean attribute change ---"
        );
        for (alo, ahi) in [(18u8, 21u8), (22, 25), (26, 29), (30, 34)] {
            for kind in Kind::ALL {
                let attributes = self.attributes_of(kind);
                let mut deltas: Vec<Vec<f32>> = vec![Vec::new(); SKILLS];
                let mut dca = Vec::new();
                for (pid, a) in self.sorted(0) {
                    if a.kind != kind
                        || a.age < alo
                        || a.age > ahi
                        || a.pa < 100
                        || a.place != Place::Main
                    {
                        continue;
                    }
                    let Some(b) = self.snaps.years[last].get(pid) else {
                        continue;
                    };
                    if b.place == Place::Free {
                        continue;
                    }
                    dca.push(b.ca as f32 - a.ca as f32);
                    for i in 0..SKILLS {
                        deltas[i].push(b.skills[i] - a.skills[i]);
                    }
                }
                if dca.len() < 30 {
                    continue;
                }
                let changes: String = attributes
                    .iter()
                    .map(|i| format!(" {}{:+.1}", self.snaps.names[*i], Stats::mean(&deltas[*i])))
                    .collect();
                println!(
                    "{alo}-{ahi} {kind:?} | n{} dCA{:+.1} |{changes}",
                    dca.len(),
                    Stats::mean(&dca)
                );
            }
        }
    }

    fn who_plays(&self) {
        let last = self.last();
        println!(
            "\n--- R7: who plays. Regulars (>=1800 official minutes in the year) by group, years 1+ ---"
        );
        for (g, group) in GROUPS.iter().enumerate() {
            for y in 1..=last {
                let ages = Stats::sorted(
                    self.snaps.years[y]
                        .values()
                        .filter(|s| s.kind.group() == g && s.track.role() == Role::Regular)
                        .map(|s| s.age as f32)
                        .collect(),
                );
                let n = ages.len().max(1) as f32;
                let share = |f: &dyn Fn(f32) -> bool| {
                    100.0 * ages.iter().filter(|a| f(**a)).count() as f32 / n
                };
                let elite: Vec<f32> = self.snaps.years[y]
                    .values()
                    .filter(|s| {
                        s.kind.group() == g
                            && s.track.role() == Role::Regular
                            && s.rank < 100
                            && s.place == Place::Main
                    })
                    .map(|s| s.ca as f32)
                    .collect();
                println!(
                    "{group:>4} y{y} | n{} age{:.0} | u20 {:.1}% | u22 {:.1}% | o30 {:.1}% | o33 {:.1}% | o35 {:.1}% | top100 n{} CA{:.1}",
                    ages.len(),
                    Stats::pct(&ages, 0.5),
                    share(&|a| a <= 20.0),
                    share(&|a| a <= 22.0),
                    share(&|a| a >= 30.0),
                    share(&|a| a >= 33.0),
                    share(&|a| a >= 35.0),
                    elite.len(),
                    Stats::mean(&elite)
                );
            }
        }
    }

    fn line_by_tier(&self) {
        println!("\n--- R7b: main-team squads, mean CA by group and club tier, per year ---");
        for (tlo, thi, label) in [
            (0u32, 99u32, "top100"),
            (100, 499, "100-499"),
            (500, 1999, "500-1999"),
        ] {
            for (g, group) in GROUPS.iter().enumerate() {
                let line: String = self
                    .snaps
                    .years
                    .iter()
                    .enumerate()
                    .map(|(y, snaps)| {
                        let v: Vec<f32> = snaps
                            .values()
                            .filter(|s| {
                                s.kind.group() == g
                                    && s.place == Place::Main
                                    && s.rank >= tlo
                                    && s.rank <= thi
                            })
                            .map(|s| s.ca as f32)
                            .collect();
                        format!(" | y{y} {:.1} n{}", Stats::mean(&v), v.len())
                    })
                    .collect();
                println!("{label:>9} {group:>4}{line}");
            }
        }
    }

    fn the_young(&self) {
        println!("\n--- R8: the young. CA/PA (mean) and count by age and place, per year ---");
        for place in [Place::Academy, Place::Youth, Place::Reserve, Place::Main] {
            for age in 14u8..=21 {
                let line: String = self
                    .snaps
                    .years
                    .iter()
                    .enumerate()
                    .map(|(y, snaps)| {
                        let v: Vec<&Snap> = snaps
                            .values()
                            .filter(|s| s.place == place && s.age == age)
                            .collect();
                        if v.len() < 10 {
                            return format!(" | y{y} -");
                        }
                        let r: Vec<f32> = v.iter().map(|s| s.ratio()).collect();
                        let c: Vec<f32> = v.iter().map(|s| s.ca as f32).collect();
                        let p: Vec<f32> = v.iter().map(|s| s.pa as f32).collect();
                        format!(
                            " | y{y} {:.3} ca{:.1} pa{:.1} n{}",
                            Stats::mean(&r),
                            Stats::mean(&c),
                            Stats::mean(&p),
                            v.len()
                        )
                    })
                    .collect();
                println!("{:>8} {age}{line}", place.name());
            }
        }
    }

    fn youth_football(&self) {
        let last = self.last();
        println!(
            "\n--- R8b: football by place for players of 21 and under: official minutes, friendly apps, league reputation, last year ---"
        );
        for place in [Place::Main, Place::Reserve, Place::Youth, Place::Academy] {
            let v: Vec<&Snap> = self.snaps.years[last]
                .values()
                .filter(|s| s.place == place && s.age <= 21)
                .collect();
            if v.is_empty() {
                continue;
            }
            let n = v.len() as f32;
            let mean = |f: fn(&&Snap) -> f32| v.iter().map(f).sum::<f32>() / n;
            println!(
                "{:>8} u21 | n{} min{:.0} fr{:.1} rep{:.0} banked{:.2}",
                place.name(),
                v.len(),
                mean(|s| s.track.minutes),
                mean(|s| s.track.friendly_apps as f32),
                mean(|s| s.league_rep as f32),
                mean(|s| s.banked),
            );
        }
    }

    /// Mean CA and attributes of first-team players of a group and age in
    /// the opening world.
    fn opening_means(&self, g: usize, age: u8) -> Option<(usize, f32, [f32; SKILLS])> {
        let pool: Vec<&Snap> = self.snaps.years[0]
            .values()
            .filter(|s| {
                s.kind.group() == g && s.age == age && s.place == Place::Main && s.pa >= 100
            })
            .collect();
        if pool.len() < 30 {
            return None;
        }
        let n = pool.len() as f32;
        let mut skills = [0.0f32; SKILLS];
        for s in &pool {
            for (total, v) in skills.iter_mut().zip(s.skills) {
                *total += v / n;
            }
        }
        Some((
            pool.len(),
            pool.iter().map(|s| s.ca as f32).sum::<f32>() / n,
            skills,
        ))
    }

    fn veterans(&self) {
        let last = self.last();
        println!(
            "\n--- R9: veterans. In a main team at year 0, by age then: who is still in a club at the last year, and what he lost ---"
        );
        println!(
            "# db: the opening world's first-team players {last} years older less those of the age, the same attributes"
        );
        let watch: Vec<usize> = Self::VETERAN_WATCH
            .iter()
            .map(|n| self.snaps.skill(n))
            .collect();
        let short = |i: usize| -> String { self.snaps.names[i].chars().take(3).collect() };
        for (g, group) in GROUPS.iter().enumerate() {
            for age in 28u8..=36 {
                let pool: Vec<(&u32, &Snap)> = self
                    .sorted(0)
                    .into_iter()
                    .filter(|(_, s)| {
                        s.kind.group() == g && s.age == age && s.place == Place::Main && s.pa >= 100
                    })
                    .collect();
                if pool.len() < 30 {
                    continue;
                }
                let kept: Vec<(&Snap, &Snap)> = pool
                    .iter()
                    .filter_map(|(pid, a)| {
                        self.snaps.years[last]
                            .get(*pid)
                            .filter(|b| b.place != Place::Free)
                            .map(|b| (*a, b))
                    })
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                let dca: Vec<f32> = kept
                    .iter()
                    .map(|(a, b)| b.ca as f32 - a.ca as f32)
                    .collect();
                let attrs: String = watch
                    .iter()
                    .map(|&i| {
                        let d: Vec<f32> = kept
                            .iter()
                            .map(|(a, b)| b.skills[i] - a.skills[i])
                            .collect();
                        format!(" {}{:+.1}", short(i), Stats::mean(&d))
                    })
                    .collect();
                let regulars = kept
                    .iter()
                    .filter(|(_, b)| b.track.role() == Role::Regular)
                    .count();
                let db = match (
                    self.opening_means(g, age),
                    self.opening_means(g, age + last as u8),
                ) {
                    (Some((n0, ca0, s0)), Some((n1, ca1, s1))) if last > 0 => {
                        let attrs: String = watch
                            .iter()
                            .map(|&i| format!(" {}{:+.1}", short(i), s1[i] - s0[i]))
                            .collect();
                        format!(" | db n{} dCA{:+.1}{attrs}", n0.min(n1), ca1 - ca0)
                    }
                    _ => " | db -".to_string(),
                };
                println!(
                    "{group:>4} {age} | n{} kept{:.0}% regulars{:.0}% dCA{:+.1} | n{}{attrs}{db}",
                    pool.len(),
                    100.0 * kept.len() as f32 / pool.len() as f32,
                    100.0 * regulars as f32 / pool.len() as f32,
                    Stats::mean(&dca),
                    kept.len()
                );
            }
        }
    }

    fn sources(&self) {
        let last = self.last();
        println!(
            "\n--- R13: where a year's growth came from. Attribute points per player from his own ledger (July to June), years 1+ ---"
        );
        println!(
            "# cells: sessions, matches, a growing body, ageing, lay-off; total; matches' share of the gains"
        );
        for (alo, ahi) in [(17u8, 18u8), (19u8, 22u8), (23, 26), (27, 30), (31, 35)] {
            for (g, group) in GROUPS.iter().enumerate() {
                for role in Self::ROLES {
                    let v: Vec<&Snap> = (1..=last)
                        .flat_map(|y| self.snaps.years[y].values())
                        .filter(|s| {
                            s.kind.group() == g
                                && s.age >= alo
                                && s.age <= ahi
                                && s.place.in_squad()
                                && s.track.role() == role
                        })
                        .collect();
                    if v.len() < 30 {
                        continue;
                    }
                    let n = v.len() as f32;
                    let mean = |k: usize| v.iter().map(|s| s.sources[k]).sum::<f32>() / n;
                    let (ses, mat, grw, age, lay) = (mean(0), mean(1), mean(2), mean(3), mean(4));
                    let gains = ses + mat + grw;
                    println!(
                        "{group:>4} {alo}-{ahi} {:>8} | n{} ses{:+.2} mat{:+.2} grw{:+.2} age{:+.2} lay{:+.2} | total{:+.2} | mat{:.0}%",
                        role.name(),
                        v.len(),
                        ses,
                        mat,
                        grw,
                        age,
                        lay,
                        gains + age + lay,
                        if gains > 0.0 {
                            100.0 * mat / gains
                        } else {
                            0.0
                        }
                    );
                }
            }
        }
    }

    fn fitting_football(&self) {
        println!(
            "\n--- R14: football that fits. Players aged 18-21 in a club squad with 900 or more minutes at a fitting standard (minutes counted by their fit), per year ---"
        );
        for (y, snaps) in self.snaps.years.iter().enumerate().skip(1) {
            let pool: Vec<&Snap> = snaps
                .values()
                .filter(|s| s.age >= 18 && s.age <= 21 && s.place.in_squad())
                .collect();
            let fitting = pool.iter().filter(|s| s.fitting_minutes >= 900.0).count();
            println!(
                "year {y} | n{} {:.1}%",
                pool.len(),
                100.0 * fitting as f32 / pool.len().max(1) as f32
            );
        }
    }

    /// A year's CA change at 19 to 22, years 1+: regulars, club players who
    /// played no football, and players without a club all year.
    fn football_shows(&self) {
        let last = self.last();
        println!(
            "
--- R15: football shows. Mean CA change over a year at 19-22, years 1+: regulars, club players with no football, players without a club all year ---"
        );
        let mut cells: [Vec<f32>; 3] = Default::default();
        for y in 1..last {
            for (pid, a) in self.sorted(y) {
                if a.age < 19 || a.age > 22 {
                    continue;
                }
                let Some(b) = self.snaps.years[y + 1].get(pid) else {
                    continue;
                };
                let cell = match (a.place == Place::Free, b.place == Place::Free) {
                    (true, true) => 2,
                    (false, false) => match b.track.role() {
                        Role::Regular => 0,
                        Role::None => 1,
                        _ => continue,
                    },
                    _ => continue,
                };
                cells[cell].push(b.ca as f32 - a.ca as f32);
            }
        }
        let line: Vec<String> = ["regular", "none", "free"]
            .iter()
            .zip(&cells)
            .map(|(name, c)| format!("{name} {:+.1} n{}", Stats::mean(c), c.len()))
            .collect();
        println!("{}", line.join(" | "));
    }

    fn talent_supply(&self) {
        println!(
            "\n--- R11: talent supply. Players by age with potential >= 110 / 130 / 150 in any place, per year ---"
        );
        for age in 14u8..=22 {
            let line: String = self
                .snaps
                .years
                .iter()
                .enumerate()
                .map(|(y, snaps)| {
                    let v: Vec<&Snap> = snaps.values().filter(|s| s.age == age).collect();
                    let c = |min: u8| v.iter().filter(|s| s.pa >= min).count();
                    let pa = Stats::sorted(v.iter().map(|s| s.pa as f32).collect());
                    format!(
                        " | y{y} n{} pa110:{} pa130:{} pa150:{} p50:{:.0} p90:{:.0} p99:{:.0}",
                        v.len(),
                        c(110),
                        c(130),
                        c(150),
                        Stats::pct(&pa, 0.5),
                        Stats::pct(&pa, 0.9),
                        Stats::pct(&pa, 0.99)
                    )
                })
                .collect();
            println!("age {age}{line}");
        }
    }

    fn academy_levels(&self) {
        let last = self.last();
        println!(
            "\n--- R11b: academy outfielders by age: CA/PA and attribute level (mean of outfield attributes), last year ---"
        );
        for age in 13u8..=18 {
            let v: Vec<&Snap> = self.snaps.years[last]
                .values()
                .filter(|s| s.place == Place::Academy && s.age == age && s.kind != Kind::GK)
                .collect();
            if v.len() < 10 {
                continue;
            }
            let level = |s: &Snap| s.skills[..36].iter().sum::<f32>() / 36.0;
            let lv: Vec<f32> = v.iter().map(|s| level(s)).collect();
            let r: Vec<f32> = v.iter().map(|s| s.ratio()).collect();
            let hr: Vec<f32> = v
                .iter()
                .filter(|s| s.pa >= 110)
                .map(|s| s.ratio())
                .collect();
            println!(
                "academy {age} | n{} {:.3} level{:.2} | pa110 n{} {:.3}",
                v.len(),
                Stats::mean(&r),
                Stats::mean(&lv),
                hr.len(),
                Stats::mean(&hr)
            );
        }
    }

    fn first_teams_by_kind(&self) {
        println!("\n--- R12: main-team squads at the top 500 clubs, mean CA by kind, per year ---");
        for kind in Kind::ALL {
            let line: String = self
                .snaps
                .years
                .iter()
                .enumerate()
                .map(|(y, snaps)| {
                    let pool: Vec<&Snap> = snaps
                        .values()
                        .filter(|s| s.kind == kind && s.place == Place::Main && s.rank < 500)
                        .collect();
                    let ca: Vec<f32> = pool.iter().map(|s| s.ca as f32).collect();
                    let age: Vec<f32> = pool.iter().map(|s| s.age as f32).collect();
                    format!(
                        " | y{y} {:.1} n{} age{:.1}",
                        Stats::mean(&ca),
                        pool.len(),
                        Stats::mean(&age)
                    )
                })
                .collect();
            println!("{kind:?}{line}");
        }
    }

    fn same_men_top_clubs(&self) {
        let last = self.last();
        println!(
            "\n--- R12b: the same men at the top 500 clubs: in a main team at year 0 aged 22-27 and in a club at the last year, mean dCA by kind ---"
        );
        for kind in Kind::ALL {
            let d: Vec<f32> = self
                .sorted(0)
                .into_iter()
                .filter(|(_, s)| {
                    s.kind == kind
                        && s.place == Place::Main
                        && s.rank < 500
                        && s.age >= 22
                        && s.age <= 27
                })
                .filter_map(|(pid, a)| {
                    self.snaps.years[last]
                        .get(pid)
                        .filter(|b| b.place != Place::Free)
                        .map(|b| b.ca as f32 - a.ca as f32)
                })
                .collect();
            println!("{kind:?} | n{} dCA{:+.1}", d.len(), Stats::mean(&d));
        }
    }

    fn character(&self) {
        let last = self.last();
        println!(
            "\n--- R10: character attributes of main-team outfielders aged 24-29, db against the last year ---"
        );
        for name in [
            "Aggression",
            "Bravery",
            "Determination",
            "Flair",
            "Leadership",
            "Teamwork",
            "WorkRate",
            "Composure",
            "Decisions",
        ] {
            let i = self.snaps.skill(name);
            let stat = |y: usize| {
                let v: Vec<f32> = self.snaps.years[y]
                    .values()
                    .filter(|s| {
                        s.place == Place::Main
                            && s.age >= 24
                            && s.age <= 29
                            && s.pa >= 100
                            && s.kind != Kind::GK
                    })
                    .map(|s| s.skills[i])
                    .collect();
                (Stats::mean(&v), Stats::sd(&v))
            };
            let (a, b) = (stat(0), stat(last));
            println!(
                "{name:>14} | db {:.2} sd{:.2} | y{last} {:.2} sd{:.2}",
                a.0, a.1, b.0, b.1
            );
        }
    }
}
