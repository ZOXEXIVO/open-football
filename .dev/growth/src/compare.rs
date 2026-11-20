//! Two census reports side by side: every row holding a figure that moved by
//! more than two runs of one build disagree by.

use std::collections::BTreeMap;

/// One number of a report and how finely it is printed, which says what
/// kind of figure it is.
#[derive(Clone, Copy, Debug)]
enum Figure {
    /// A share in percent.
    Share(f32),
    /// Three decimals: CA/PA and its changes.
    Ratio(f32),
    /// Two decimals: spreads and banked shares.
    Fine(f32),
    /// One decimal: mean CA, attribute and minute figures.
    Mean(f32),
    /// `nN`: how many players a cell holds.
    Count(f32),
    /// Any other whole number.
    Whole(f32),
}

impl Figure {
    fn parse(token: &str) -> Option<Figure> {
        if token.starts_with('y') && token[1..].chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let bytes = token.as_bytes();
        let mut end = bytes.len();
        while end > 0 && !bytes[end - 1].is_ascii_digit() {
            end -= 1;
        }
        if end == 0 {
            return None;
        }
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if start > 0 && matches!(bytes[start - 1], b'-' | b'+') {
            start -= 1;
        }
        let number = &token[start..end];
        let value: f32 = number.parse().ok()?;
        let decimals = number.split('.').nth(1).map_or(0, str::len);
        let prefix = &token[..start];
        Some(if token[end..].starts_with('%') {
            Figure::Share(value)
        } else if decimals == 0 && (prefix == "n" || prefix == "n=") {
            Figure::Count(value)
        } else {
            match decimals {
                0 => Figure::Whole(value),
                1 => Figure::Mean(value),
                2 => Figure::Fine(value),
                _ => Figure::Ratio(value),
            }
        })
    }

    /// Whether `self` and `other` differ by more than two runs of one build
    /// do, in a cell of `n` players. Sampling noise falls with the square
    /// root of a cell's size, so a figure of a small cell needs a larger
    /// move to count. Set from the two control runs of
    /// `measurements/section-1/`.
    fn differs(self, other: Figure, n: f32) -> bool {
        let scale = (Self::REFERENCE_N / n.max(1.0)).sqrt().max(1.0);
        let count = |a: f32, b: f32| {
            let size = a.abs().max(b.abs());
            (a - b).abs() > (size * 0.10).max(4.0 * size.sqrt()).max(10.0)
        };
        match (self, other) {
            (Figure::Share(a), Figure::Share(b)) => {
                let p = ((a + b) / 200.0).clamp(0.01, 0.99);
                (a - b).abs() > (300.0 * (2.0 * p * (1.0 - p) / n.max(1.0)).sqrt()).max(1.0)
            }
            (Figure::Ratio(a), Figure::Ratio(b)) => (a - b).abs() > 0.02 * scale,
            (Figure::Fine(a), Figure::Fine(b)) => (a - b).abs() > 0.15 * scale,
            (Figure::Mean(a), Figure::Mean(b)) => {
                (a - b).abs() > (0.5 * scale).max(a.abs().max(b.abs()) * 0.04 * scale)
            }
            (Figure::Count(a), Figure::Count(b)) | (Figure::Whole(a), Figure::Whole(b)) => {
                count(a, b)
            }
            _ => true,
        }
    }

    /// The cell size at which the base tolerances hold.
    const REFERENCE_N: f32 = 400.0;
}

struct Row {
    line: String,
    cells: Vec<Vec<Figure>>,
    /// Players behind a row that prints no count of its own: those of the
    /// last counted row above it in the same table.
    inherited: f32,
}

impl Row {
    /// A cell of fewer players than this is too small to read a change from.
    const MIN_N: f32 = 50.0;

    fn parse(line: &str) -> Option<(String, Row)> {
        if line.starts_with('#') || line.starts_with('-') || line.starts_with('=') {
            return None;
        }
        let mut parts = line.split('|');
        let label = parts
            .next()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let cells: Vec<Vec<Figure>> = parts
            .map(|cell| cell.split_whitespace().filter_map(Figure::parse).collect())
            .collect();
        if cells.is_empty() {
            return None;
        }
        Some((
            label,
            Row {
                line: line.trim_end().to_string(),
                cells,
                inherited: f32::MAX,
            },
        ))
    }

    fn small(cell: &[Figure]) -> bool {
        cell.iter()
            .any(|f| matches!(f, Figure::Count(n) if *n < Self::MIN_N))
    }

    fn count(cell: &[Figure]) -> Option<f32> {
        cell.iter().find_map(|f| match f {
            Figure::Count(n) => Some(*n),
            _ => None,
        })
    }

    /// The players behind one of the row's cells: its own count, else the
    /// row's, else the row above's.
    fn size(&self, cell: &[Figure]) -> f32 {
        Self::count(cell)
            .or_else(|| self.counted())
            .unwrap_or(self.inherited)
    }

    /// The smallest count the row prints, if it prints one.
    fn counted(&self) -> Option<f32> {
        self.cells
            .iter()
            .filter_map(|c| Self::count(c))
            .reduce(f32::min)
    }

    fn differs(&self, other: &Row) -> bool {
        self.cells.iter().zip(&other.cells).any(|(a, b)| {
            if Self::small(a) || Self::small(b) || a.len() != b.len() {
                return false;
            }
            let n = self.size(a).min(other.size(b));
            a.iter().zip(b).any(|(x, y)| x.differs(*y, n))
        })
    }
}

pub struct ReportComparison;

impl ReportComparison {
    fn rows(text: &str) -> BTreeMap<String, Row> {
        let mut table = String::new();
        let mut rows = BTreeMap::new();
        let mut last_count = f32::MAX;
        for line in text.lines() {
            if let Some(header) = line.strip_prefix("--- ") {
                table = header.split(':').next().unwrap_or("").trim().to_string();
                last_count = f32::MAX;
                continue;
            }
            if let Some((label, mut row)) = Row::parse(line) {
                match row.counted() {
                    Some(n) => last_count = n,
                    None => row.inherited = last_count,
                }
                rows.insert(format!("{table} {label}"), row);
            }
        }
        rows
    }

    /// Prints every row present in both reports whose figures differ beyond
    /// the noise, and returns how many there were.
    pub fn run(before: &str, after: &str) -> usize {
        let (a, b) = (Self::rows(before), Self::rows(after));
        let mut moved = 0;
        let mut unmatched = 0;
        for (key, row) in &a {
            let Some(other) = b.get(key) else {
                unmatched += 1;
                continue;
            };
            if row.differs(other) {
                moved += 1;
                println!("{key}\n  a: {}\n  b: {}", row.line, other.line);
            }
        }
        unmatched += b.keys().filter(|k| !a.contains_key(*k)).count();
        println!(
            "# {moved} rows moved beyond the noise; {} rows compared; {unmatched} rows in one report only",
            a.len().min(b.len())
        );
        moved
    }
}
