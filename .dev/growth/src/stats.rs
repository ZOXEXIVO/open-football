//! Summary statistics over one cell of a table.

pub struct Stats;

impl Stats {
    /// The `p` quantile of an ascending slice, nearest rank.
    pub fn pct(sorted: &[f32], p: f32) -> f32 {
        if sorted.is_empty() {
            return f32::NAN;
        }
        let idx = ((sorted.len() as f32 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
        sorted[idx]
    }

    pub fn mean(v: &[f32]) -> f32 {
        if v.is_empty() {
            f32::NAN
        } else {
            v.iter().sum::<f32>() / v.len() as f32
        }
    }

    pub fn sd(v: &[f32]) -> f32 {
        if v.len() < 2 {
            return f32::NAN;
        }
        let m = Self::mean(v);
        (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / (v.len() - 1) as f32).sqrt()
    }

    pub fn sorted(mut v: Vec<f32>) -> Vec<f32> {
        v.sort_by(|a, b| a.total_cmp(b));
        v
    }
}
