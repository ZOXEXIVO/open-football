//! Each bullet of "A world keeps the database's shape", read off a census
//! report: pass, fail, or no view when the report holds no figure for it.

use std::collections::BTreeMap;

/// The figure lines of a report by section id.
struct Sections<'a> {
    by_id: BTreeMap<&'a str, Vec<&'a str>>,
}

impl<'a> Sections<'a> {
    fn of(text: &'a str) -> Self {
        let mut by_id: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut current = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("--- ") {
                current = rest.split(':').next();
                if let Some(id) = current {
                    by_id.entry(id).or_default();
                }
                continue;
            }
            if line.trim().is_empty() || line.starts_with('#') || line.starts_with('=') {
                continue;
            }
            if let Some(id) = current {
                by_id.entry(id).or_default().push(line);
            }
        }
        Sections { by_id }
    }

    fn get(&self, id: &str) -> &[&'a str] {
        self.by_id.get(id).map_or(&[], Vec::as_slice)
    }
}

/// Numbers inside a cell of a report line.
struct Cell;

impl Cell {
    /// The number a token holds after `prefix`, without a trailing `%`.
    fn value(token: &str, prefix: &str) -> Option<f32> {
        token
            .strip_prefix(prefix)?
            .trim_end_matches('%')
            .parse()
            .ok()
    }

    /// The first token of `cell` that carries `prefix` and a number.
    fn find(cell: &str, prefix: &str) -> Option<f32> {
        cell.split_whitespace().find_map(|t| Self::value(t, prefix))
    }

    /// The number after the word `word` in `cell`.
    fn after(cell: &str, word: &str) -> Option<f32> {
        let mut tokens = cell.split_whitespace();
        tokens.find(|t| *t == word)?;
        tokens.next()?.parse().ok().filter(|v: &f32| v.is_finite())
    }

    /// The group and age a table row opens with: `"  GK  22 | ..."`.
    fn group_age(line: &str) -> Option<(&str, u8)> {
        let mut head = line.split('|').next()?.split_whitespace();
        let group = head.next()?;
        let age = head.next()?.parse().ok()?;
        Some((group, age))
    }
}

struct Verdict {
    bullet: &'static str,
    pass: Option<bool>,
    detail: String,
}

impl Verdict {
    fn judged(bullet: &'static str, failures: Vec<String>, checked: usize, note: String) -> Self {
        let pass = (checked > 0).then_some(failures.is_empty());
        let detail = if checked == 0 {
            "no figure in this report".to_string()
        } else if failures.is_empty() {
            format!("{checked} checked{note}")
        } else {
            format!(
                "{} of {checked} out{note}: {}",
                failures.len(),
                failures.join(", ")
            )
        };
        Verdict {
            bullet,
            pass,
            detail,
        }
    }
}

pub struct ShapeCheck;

impl ShapeCheck {
    /// Cells holding fewer players than this are too noisy to judge.
    const MIN_N: f32 = 50.0;

    /// Print a verdict per bullet; true when every judged bullet passes and
    /// none is without a view.
    pub fn run(text: &str) -> bool {
        let sections = Sections::of(text);
        let verdicts = [
            Self::ratio_by_age(
                &sections,
                22..=33,
                0.03,
                "median CA/PA at 22-33 within 0.03 of the db",
            ),
            Self::ratio_by_age(
                &sections,
                34..=36,
                0.05,
                "median CA/PA at 34-36 within 0.05 of the db",
            ),
            Self::maxed_share(&sections),
            Self::profiles(&sections),
            Self::top_club_gain(&sections),
            Self::football_shows(&sections),
            Self::first_season(&sections),
        ];
        let mut all = true;
        for v in &verdicts {
            let mark = match v.pass {
                Some(true) => "PASS",
                Some(false) => "FAIL",
                None => "n/a ",
            };
            all &= v.pass == Some(true);
            println!("{mark}  {} | {}", v.bullet, v.detail);
        }
        let passed = verdicts.iter().filter(|v| v.pass == Some(true)).count();
        println!("# {passed} of {} bullets pass", verdicts.len());
        all
    }

    fn ratio_by_age(
        sections: &Sections,
        ages: std::ops::RangeInclusive<u8>,
        tolerance: f32,
        bullet: &'static str,
    ) -> Verdict {
        let (mut failures, mut checked, mut thin) = (Vec::new(), 0, 0);
        for line in sections.get("R1") {
            let Some((group, age)) = Cell::group_age(line) else {
                continue;
            };
            let Some(last) = line.split('|').next_back() else {
                continue;
            };
            if !ages.contains(&age) || !last.contains("-db") {
                continue;
            }
            let (Some(n), Some(diff)) = (
                Cell::find(last, "n"),
                last.split_whitespace()
                    .last()
                    .and_then(|t| t.parse::<f32>().ok()),
            ) else {
                continue;
            };
            if n < Self::MIN_N {
                thin += 1;
                continue;
            }
            checked += 1;
            if diff.abs() > tolerance {
                failures.push(format!("{group} {age} {diff:+.3}"));
            }
        }
        let note = if thin > 0 {
            format!(", {thin} cells under n{}", Self::MIN_N)
        } else {
            String::new()
        };
        Verdict::judged(bullet, failures, checked, note)
    }

    fn maxed_share(sections: &Sections) -> Verdict {
        let (mut failures, mut checked) = (Vec::new(), 0);
        for line in sections.get("R2") {
            let mut cells = line.split('|');
            let Some(head) = cells.next() else {
                continue;
            };
            if !head.contains("age 25-29") {
                continue;
            }
            let band = head.split_whitespace().next().unwrap_or("?");
            let Some(share) = cells
                .next_back()
                .and_then(|c| c.split_whitespace().nth(1))
                .and_then(|t| Cell::value(t, ""))
            else {
                continue;
            };
            checked += 1;
            if !(5.0..=15.0).contains(&share) {
                failures.push(format!("{band} {share:.1}%"));
            }
        }
        Verdict::judged(
            "5-15% of 25-29s at 0.98 PA or more, each potential band",
            failures,
            checked,
            String::new(),
        )
    }

    fn profiles(sections: &Sections) -> Verdict {
        #[derive(Default)]
        struct Kind {
            spread: Option<f32>,
            worst: f32,
            over_one: usize,
        }
        let mut kinds: BTreeMap<&str, Kind> = BTreeMap::new();
        for line in sections.get("R5") {
            let cells: Vec<&str> = line.split('|').collect();
            let mut head = cells[0].split_whitespace();
            let (Some(kind), Some(name)) = (head.next(), head.next()) else {
                continue;
            };
            let entry = kinds.entry(kind).or_default();
            if name == "all" && cells.len() >= 3 {
                if let (Some(db), Some(now)) =
                    (Cell::find(cells[1], "sd"), Cell::find(cells[2], "sd"))
                {
                    entry.spread = Some(now / db);
                }
            } else if let Some(moved) = cells.last().and_then(|c| Cell::after(c, "moved")) {
                entry.worst = entry.worst.max(moved.abs());
                entry.over_one += usize::from(moved.abs() > 1.0);
            }
        }
        let mut failures = Vec::new();
        for (name, kind) in &kinds {
            let mut out = Vec::new();
            if kind.worst > 1.5 {
                out.push(format!("one moved {:.1}", kind.worst));
            }
            if kind.over_one > 3 {
                out.push(format!("{} moved over 1.0", kind.over_one));
            }
            if let Some(spread) = kind.spread
                && (spread - 1.0).abs() > 0.10
            {
                out.push(format!("spread {spread:.2} of the db's"));
            }
            if !out.is_empty() {
                failures.push(format!("{name} {}", out.join(" / ")));
            }
        }
        Verdict::judged(
            "regulars 24-29 keep each kind's attribute means and spread",
            failures,
            kinds.len(),
            String::new(),
        )
    }

    fn top_club_gain(sections: &Sections) -> Verdict {
        let (mut n, mut sum) = (0.0f32, 0.0f32);
        for line in sections.get("R12b") {
            let Some(cell) = line.split('|').nth(1) else {
                continue;
            };
            if let (Some(count), Some(gain)) = (Cell::find(cell, "n"), Cell::find(cell, "dCA")) {
                n += count;
                sum += count * gain;
            }
        }
        let checked = usize::from(n > 0.0);
        let gain = sum / n.max(1.0);
        let failures = if checked == 1 && !(3.0..=8.0).contains(&gain) {
            vec![format!("dCA {gain:+.1}")]
        } else {
            Vec::new()
        };
        let note = if checked == 1 {
            format!(", dCA {gain:+.1} n{n}")
        } else {
            String::new()
        };
        Verdict::judged(
            "22-27s in a top-500 first team gain 3-8 CA over four seasons",
            failures,
            checked,
            note,
        )
    }

    fn football_shows(sections: &Sections) -> Verdict {
        let Some(line) = sections.get("R15").first() else {
            return Verdict::judged(
                "at 19-22 regulars gain twice the unused, the clubless a quarter",
                Vec::new(),
                0,
                String::new(),
            );
        };
        let cells: Vec<Option<f32>> = ["regular", "none", "free"]
            .iter()
            .map(|name| line.split('|').find_map(|c| Cell::after(c, name)))
            .collect();
        let (Some(regular), Some(none), Some(free)) = (cells[0], cells[1], cells[2]) else {
            return Verdict::judged(
                "at 19-22 regulars gain twice the unused, the clubless a quarter",
                Vec::new(),
                0,
                String::new(),
            );
        };
        let mut failures = Vec::new();
        if regular < 2.0 * none {
            failures.push(format!("regular {regular:+.1} against unused {none:+.1}"));
        }
        if free > 0.25 * regular {
            failures.push(format!("clubless {free:+.1} against regular {regular:+.1}"));
        }
        Verdict::judged(
            "at 19-22 regulars gain twice the unused, the clubless a quarter",
            failures,
            2,
            format!(", regular {regular:+.1} unused {none:+.1} clubless {free:+.1}"),
        )
    }

    fn first_season(sections: &Sections) -> Verdict {
        let regulars = |id: &str| -> BTreeMap<(String, u8), f32> {
            sections
                .get(id)
                .iter()
                .filter_map(|line| {
                    let (group, age) = Cell::group_age(line)?;
                    let cell = line.split('|').nth(1)?;
                    Some(((group.to_string(), age), Cell::after(cell, "regular")?))
                })
                .collect()
        };
        let (first, later) = (regulars("R3a"), regulars("R3b"));
        let (mut failures, mut checked) = (Vec::new(), 0);
        for ((group, age), a) in &first {
            if !(22..=30).contains(age) {
                continue;
            }
            let Some(b) = later.get(&(group.clone(), *age)) else {
                continue;
            };
            checked += 1;
            if (a - b).abs() > 1.5 {
                failures.push(format!("{group} {age} {a:+.1} vs {b:+.1}"));
            }
        }
        Verdict::judged(
            "regulars 22-30: first season's CA change within 1.5 of later ones",
            failures,
            checked,
            String::new(),
        )
    }
}
