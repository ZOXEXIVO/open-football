//! The census as rows: one figure a line, each with its sampling error, so
//! two reports can be compared row by row.

use crate::reference::{Band, RealFootball};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

#[derive(Default, Clone, Copy)]
pub struct Mean {
    sum: f64,
    squares: f64,
    pub n: u64,
}

impl Mean {
    pub fn add(&mut self, v: f64) {
        self.sum += v;
        self.squares += v * v;
        self.n += 1;
    }

    pub fn get(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.sum / self.n as f64
        }
    }

    pub fn standard_error(&self) -> f64 {
        if self.n < 2 {
            return 0.0;
        }
        let n = self.n as f64;
        let variance = (self.squares - self.sum * self.sum / n) / (n - 1.0);
        (variance.max(0.0) / n).sqrt()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Mean,
    Percent,
    Count,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub tier: u8,
    pub section: &'static str,
    pub key: String,
    pub label: String,
    pub value: f64,
    pub error: f64,
    pub n: u64,
    pub unit: Unit,
}

impl Row {
    pub fn mean(key: impl Into<String>, label: impl Into<String>, mean: &Mean) -> Self {
        Row {
            tier: 0,
            section: "",
            key: key.into(),
            label: label.into(),
            value: mean.get(),
            error: mean.standard_error(),
            n: mean.n,
            unit: Unit::Mean,
        }
    }

    pub fn share(key: impl Into<String>, label: impl Into<String>, part: u64, whole: u64) -> Self {
        let p = if whole == 0 {
            0.0
        } else {
            part as f64 / whole as f64
        };
        let error = if whole == 0 {
            0.0
        } else {
            100.0 * (p * (1.0 - p) / whole as f64).sqrt()
        };
        Row {
            tier: 0,
            section: "",
            key: key.into(),
            label: label.into(),
            value: 100.0 * p,
            error,
            n: whole,
            unit: Unit::Percent,
        }
    }

    pub fn count(key: impl Into<String>, label: impl Into<String>, count: u64, n: u64) -> Self {
        Row {
            tier: 0,
            section: "",
            key: key.into(),
            label: label.into(),
            value: count as f64,
            error: (count as f64).sqrt(),
            n,
            unit: Unit::Count,
        }
    }

    pub fn figure(
        key: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        error: f64,
        n: u64,
        unit: Unit,
    ) -> Self {
        Row {
            tier: 0,
            section: "",
            key: key.into(),
            label: label.into(),
            value,
            error,
            n,
            unit,
        }
    }

    fn band(&self) -> Option<&'static Band> {
        RealFootball::band(self.tier, &self.key)
    }

    fn line(&self) -> String {
        let value = match self.unit {
            Unit::Mean => format!("{:.2}", self.value),
            Unit::Percent => format!("{:.2}%", self.value),
            Unit::Count => format!("{:.0}", self.value),
        };
        let reference = match self.band() {
            Some(band) => format!("{:<18} {:<8}", band.range(), band.verdict(self.value)),
            None => " ".repeat(27),
        };
        format!(
            "{:<46} {:>9} se={:<7.3} n={:<8} {} | {}",
            self.key, value, self.error, self.n, reference, self.label
        )
    }

    /// Reads back a line [`Self::line`] wrote.
    fn parse(tier: u8, line: &str) -> Option<Row> {
        let mut tokens = line.split_whitespace();
        let key = tokens.next()?;
        if !key.contains('.') || !key.starts_with(|c: char| c.is_ascii_lowercase()) {
            return None;
        }
        let raw = tokens.next()?;
        let unit = if raw.ends_with('%') {
            Unit::Percent
        } else if raw.contains('.') {
            Unit::Mean
        } else {
            Unit::Count
        };
        let value: f64 = raw.trim_end_matches('%').parse().ok()?;
        let error: f64 = tokens.next()?.strip_prefix("se=")?.parse().ok()?;
        let n: u64 = tokens.next()?.strip_prefix("n=")?.parse().ok()?;
        Some(Row {
            tier,
            section: "",
            key: key.to_string(),
            label: line
                .split_once("| ")
                .map(|(_, label)| label.to_string())
                .unwrap_or_default(),
            value,
            error,
            n,
            unit,
        })
    }
}

#[derive(Default)]
pub struct Report {
    pub rows: Vec<Row>,
    /// Lines listed under a section for reading, never compared.
    details: Vec<(u8, &'static str, String)>,
}

impl Report {
    const TIER_HEADER: &'static str = "================ TIER ";

    pub fn push(&mut self, tier: u8, section: &'static str, mut row: Row) {
        row.tier = tier;
        row.section = section;
        self.rows.push(row);
    }

    pub fn detail(&mut self, tier: u8, section: &'static str, line: String) {
        self.details.push((tier, section, line));
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut tiers: Vec<u8> = self.rows.iter().map(|r| r.tier).collect();
        tiers.dedup();
        for tier in tiers {
            let _ = writeln!(out, "\n{}{tier} ================", Self::TIER_HEADER);
            let mut section = "";
            for row in self.rows.iter().filter(|r| r.tier == tier) {
                if row.section != section {
                    self.render_details(&mut out, tier, section);
                    section = row.section;
                    let _ = writeln!(out, "\n-- {section} --");
                }
                let _ = writeln!(out, "{}", row.line());
            }
            self.render_details(&mut out, tier, section);
        }
        out
    }

    fn render_details(&self, out: &mut String, tier: u8, section: &str) {
        for (_, _, line) in self
            .details
            .iter()
            .filter(|(t, s, _)| *t == tier && *s == section)
        {
            let _ = writeln!(out, "    {line}");
        }
    }

    /// The headline rows: those a reference band stands beside.
    pub fn render_verdicts(&self) -> String {
        let mut out = String::from("\n================ VERDICTS ================\n");
        for row in &self.rows {
            if let Some(band) = row.band() {
                let _ = writeln!(
                    out,
                    "T{} {:<46} {:>9.2} {:<18} {:<8} {}",
                    row.tier,
                    row.key,
                    row.value,
                    band.range(),
                    band.verdict(row.value),
                    band.source.label()
                );
            }
        }
        out
    }

    pub fn load(path: &str) -> Result<Report, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let mut report = Report::default();
        let mut tier = 0u8;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix(Self::TIER_HEADER) {
                tier = rest
                    .split_whitespace()
                    .next()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0);
                continue;
            }
            if tier == 0 || line.starts_with(' ') {
                continue;
            }
            if let Some(row) = Row::parse(tier, line) {
                report.rows.push(row);
            }
        }
        if report.rows.is_empty() {
            return Err(format!("{path}: no census rows"));
        }
        Ok(report)
    }

    fn by_key(&self) -> BTreeMap<(u8, &str), &Row> {
        self.rows
            .iter()
            .map(|row| ((row.tier, row.key.as_str()), row))
            .collect()
    }
}

/// Two reports side by side, against the run-to-run noise of one build.
pub struct Comparison {
    before: Report,
    after: Report,
    /// Two runs of the same build, when the caller has them.
    control: Option<(Report, Report)>,
}

impl Comparison {
    /// A row is listed when it moved by more than this many times its noise.
    const NOISE_MULTIPLE: f64 = 2.0;

    pub fn new(before: Report, after: Report, control: Option<(Report, Report)>) -> Self {
        Comparison {
            before,
            after,
            control,
        }
    }

    /// The noise of one row: the larger of its sampling error across the two
    /// reports and what two runs of one build differed by.
    fn noise(&self, key: (u8, &str), before: &Row, after: &Row) -> f64 {
        let sampling = (before.error * before.error + after.error * after.error).sqrt();
        let measured = self
            .control
            .as_ref()
            .and_then(|(a, b)| {
                let (a, b) = (a.by_key(), b.by_key());
                Some((a.get(&key)?.value - b.get(&key)?.value).abs())
            })
            .unwrap_or(0.0);
        sampling.max(measured)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let before = self.before.by_key();
        let after = self.after.by_key();
        let mut moved = 0usize;
        for row in &self.after.rows {
            let key = (row.tier, row.key.as_str());
            let Some(old) = before.get(&key) else {
                let _ = writeln!(out, "T{} {:<46} new row: {:.2}", row.tier, row.key, row.value);
                moved += 1;
                continue;
            };
            let noise = self.noise(key, old, row);
            let delta = row.value - old.value;
            if delta.abs() <= Self::NOISE_MULTIPLE * noise || delta == 0.0 {
                continue;
            }
            moved += 1;
            let verdict = match row.band() {
                Some(band) => format!(
                    "{} -> {}",
                    band.verdict(old.value),
                    band.verdict(row.value)
                ),
                None => String::new(),
            };
            let _ = writeln!(
                out,
                "T{} {:<46} {:>9.2} -> {:>9.2}  ({:+.2}, noise {:.2})  {}",
                row.tier, row.key, old.value, row.value, delta, noise, verdict
            );
        }
        for row in &self.before.rows {
            if !after.contains_key(&(row.tier, row.key.as_str())) {
                let _ = writeln!(out, "T{} {:<46} row gone (was {:.2})", row.tier, row.key, row.value);
                moved += 1;
            }
        }
        let _ = writeln!(
            out,
            "# {moved} of {} rows moved by more than {} x noise ({})",
            self.after.rows.len(),
            Self::NOISE_MULTIPLE,
            if self.control.is_some() {
                "sampling error and two runs of one build"
            } else {
                "sampling error only"
            }
        );
        out
    }

    /// Per-row difference between the two control runs, for the notes.
    pub fn render_noise(a: &Report, b: &Report) -> String {
        let mut out = String::new();
        let other = b.by_key();
        for row in &a.rows {
            if RealFootball::band(row.tier, &row.key).is_none() {
                continue;
            }
            if let Some(second) = other.get(&(row.tier, row.key.as_str())) {
                let _ = writeln!(
                    out,
                    "T{} {:<46} {:>9.2} {:>9.2}  diff {:.2}  sampling {:.2}",
                    row.tier,
                    row.key,
                    row.value,
                    second.value,
                    (row.value - second.value).abs(),
                    (row.error * row.error + second.error * second.error).sqrt()
                );
            }
        }
        out
    }
}
