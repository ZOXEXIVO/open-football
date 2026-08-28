//! **The spacing census** — how much room the twenty-two are giving each
//! other, and who ends up standing in the same square of grass.
//!
//! `paths` already reports a nearest-team-mate mean and a "standing knot"
//! share off the replay track, and both say the engine bunches. Neither
//! can say **who** bunches or **where**, because the track carries
//! positions and nothing else: a knot of four is four dots, and the fix
//! for four defenders collapsing onto a carrier is the opposite of the
//! fix for four attackers sharing one box slot.
//!
//! This runs on the field, so it sees the team, the line and the ball.
//! Three questions, each with a real-football reference:
//!
//! * **The clump.** The largest set of outfielders joined by links of
//!   [`CLUMP_LINK`] or less. Real football has clumps — a corner, a
//!   goalmouth scramble, a challenge — so the number that matters is how
//!   often one of four or more exists in *settled* play, and what it is
//!   made of.
//! * **The spread of the side with the ball.** Nearest team-mate per
//!   outfielder, and how many of them are inside [`BALL_RING`] of the
//!   ball. A team whose players average 15-20 m apart is playing
//!   positionally; one averaging 5 m is chasing.
//! * **Who is free.** Outfielders with no opponent inside
//!   [`FREE_RADIUS`], split by whether they are ahead of the ball. This
//!   is the supply of passes the carrier actually has, and it is the
//!   thing a coach means by "somebody move into that space".
//!
//! Everything is bucketed twice — over every sample, and over the samples
//! with the ball in the final third — because the final third is where
//! the picture is and an all-pitch mean is dominated by build-up.

use crate::r#match::engine::engine::*;
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Two players this close are in the same clump. 24u = 3 m — close
/// enough that on a replay they read as one group of bodies.
const CLUMP_LINK: f32 = 24.0;

/// Nobody within this of a player and he is free to receive. 64u = 8 m,
/// about the distance a defender covers in the time a pass travels.
const FREE_RADIUS: f32 = 64.0;

/// The ring around the ball the swarm is counted in. 120u = 15 m.
const BALL_RING: f32 = 120.0;

/// Sampled every quarter-second of match time, like its neighbours in
/// [`census`](super::census).
const SAMPLE_INTERVAL_TICKS: u64 = 25;

/// Largest squad we walk. Both sides, keepers included in the roster and
/// filtered out of the geometry.
const MAX_OUTFIELD: usize = 22;

/// One bucket of the census. Two of these exist: everything, and the
/// final third alone.
pub struct SpacingBucket {
    samples: AtomicU64,
    /// Size of the largest clump, x10 so the mean keeps a decimal.
    clump_size_x10: AtomicU64,
    clump_ge4: AtomicU64,
    clump_ge6: AtomicU64,
    /// How far that clump's centre is from the ball, in units x10.
    clump_ball_gap_x10: AtomicU64,
    /// Members of the largest clump by line — 0 GK, 1 DEF, 2 MID, 3 FWD.
    clump_by_line: [AtomicU64; 4],
    /// …and by side: the team with the ball, and the team without it.
    clump_attacking: AtomicU64,
    clump_defending: AtomicU64,

    /// Per-outfielder nearest-team-mate distance for the side in
    /// possession, in units x10, and how many of those were under 5 m.
    mate_gap_x10: AtomicU64,
    mate_n: AtomicU64,
    mate_under5: AtomicU64,

    /// Outfielders inside [`BALL_RING`] of the ball, x10.
    swarm_attacking_x10: AtomicU64,
    swarm_defending_x10: AtomicU64,

    /// Attacking outfielders with no opponent inside [`FREE_RADIUS`],
    /// and the subset of those that are ahead of the ball, x10.
    free_x10: AtomicU64,
    free_ahead_x10: AtomicU64,

    /// Bounding box of the side in possession, in units x10.
    att_width_x10: AtomicU64,
    att_depth_x10: AtomicU64,
}

impl SpacingBucket {
    const fn new() -> Self {
        SpacingBucket {
            samples: AtomicU64::new(0),
            clump_size_x10: AtomicU64::new(0),
            clump_ge4: AtomicU64::new(0),
            clump_ge6: AtomicU64::new(0),
            clump_ball_gap_x10: AtomicU64::new(0),
            clump_by_line: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            clump_attacking: AtomicU64::new(0),
            clump_defending: AtomicU64::new(0),
            mate_gap_x10: AtomicU64::new(0),
            mate_n: AtomicU64::new(0),
            mate_under5: AtomicU64::new(0),
            swarm_attacking_x10: AtomicU64::new(0),
            swarm_defending_x10: AtomicU64::new(0),
            free_x10: AtomicU64::new(0),
            free_ahead_x10: AtomicU64::new(0),
            att_width_x10: AtomicU64::new(0),
            att_depth_x10: AtomicU64::new(0),
        }
    }
}

static ALL: SpacingBucket = SpacingBucket::new();
static FINAL_THIRD: SpacingBucket = SpacingBucket::new();

/// One sampled instant, before it is folded into the buckets.
pub struct SpacingSample {
    pub clump_size: usize,
    pub clump_ball_gap: f32,
    pub clump_by_line: [u32; 4],
    pub clump_attacking: u32,
    pub clump_defending: u32,
    pub mate_gap_sum: f32,
    pub mate_n: u32,
    pub mate_under5: u32,
    pub swarm_attacking: u32,
    pub swarm_defending: u32,
    pub free: u32,
    pub free_ahead: u32,
    pub att_width: f32,
    pub att_depth: f32,
}

/// What one bucket holds, in metres and per-sample means.
#[derive(Default, Clone, Copy)]
pub struct SpacingReport {
    pub samples: u64,
    pub clump_size: f32,
    pub clump_ge4_share: f32,
    pub clump_ge6_share: f32,
    pub clump_ball_gap_m: f32,
    /// Share of clump members from each line — GK, DEF, MID, FWD.
    pub clump_line_share: [f32; 4],
    pub clump_attacking: f32,
    pub clump_defending: f32,
    pub mate_gap_m: f32,
    pub mate_under5_share: f32,
    pub swarm_attacking: f32,
    pub swarm_defending: f32,
    pub free: f32,
    pub free_ahead: f32,
    pub att_width_m: f32,
    pub att_depth_m: f32,
}

pub struct SpacingCensus;

impl SpacingCensus {
    fn fold(bucket: &SpacingBucket, s: &SpacingSample) {
        let x10 = |v: f32| (v.max(0.0) * 10.0) as u64;
        bucket.samples.fetch_add(1, Ordering::Relaxed);
        bucket
            .clump_size_x10
            .fetch_add(s.clump_size as u64 * 10, Ordering::Relaxed);
        if s.clump_size >= 4 {
            bucket.clump_ge4.fetch_add(1, Ordering::Relaxed);
        }
        if s.clump_size >= 6 {
            bucket.clump_ge6.fetch_add(1, Ordering::Relaxed);
        }
        bucket
            .clump_ball_gap_x10
            .fetch_add(x10(s.clump_ball_gap), Ordering::Relaxed);
        for (slot, n) in bucket.clump_by_line.iter().zip(s.clump_by_line) {
            slot.fetch_add(n as u64, Ordering::Relaxed);
        }
        bucket
            .clump_attacking
            .fetch_add(s.clump_attacking as u64, Ordering::Relaxed);
        bucket
            .clump_defending
            .fetch_add(s.clump_defending as u64, Ordering::Relaxed);
        bucket
            .mate_gap_x10
            .fetch_add(x10(s.mate_gap_sum), Ordering::Relaxed);
        bucket.mate_n.fetch_add(s.mate_n as u64, Ordering::Relaxed);
        bucket
            .mate_under5
            .fetch_add(s.mate_under5 as u64, Ordering::Relaxed);
        bucket
            .swarm_attacking_x10
            .fetch_add(s.swarm_attacking as u64 * 10, Ordering::Relaxed);
        bucket
            .swarm_defending_x10
            .fetch_add(s.swarm_defending as u64 * 10, Ordering::Relaxed);
        bucket.free_x10.fetch_add(s.free as u64 * 10, Ordering::Relaxed);
        bucket
            .free_ahead_x10
            .fetch_add(s.free_ahead as u64 * 10, Ordering::Relaxed);
        bucket
            .att_width_x10
            .fetch_add(x10(s.att_width), Ordering::Relaxed);
        bucket
            .att_depth_x10
            .fetch_add(x10(s.att_depth), Ordering::Relaxed);
    }

    /// Fold one sampled instant into the all-play bucket, and into the
    /// final-third bucket when the ball was there.
    pub fn note(sample: &SpacingSample, final_third: bool) {
        Self::fold(&ALL, sample);
        if final_third {
            Self::fold(&FINAL_THIRD, sample);
        }
    }

    fn read(bucket: &SpacingBucket) -> SpacingReport {
        let n = bucket.samples.load(Ordering::Relaxed);
        if n == 0 {
            return SpacingReport::default();
        }
        let per = |v: &AtomicU64| v.load(Ordering::Relaxed) as f32 / 10.0 / n as f32;
        let share = |v: &AtomicU64| v.load(Ordering::Relaxed) as f32 / n as f32;
        let mate_n = bucket.mate_n.load(Ordering::Relaxed).max(1) as f32;
        let members: u64 = bucket
            .clump_by_line
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .sum();
        let members_f = members.max(1) as f32;
        let mut clump_line_share = [0.0f32; 4];
        for (slot, c) in clump_line_share.iter_mut().zip(&bucket.clump_by_line) {
            *slot = c.load(Ordering::Relaxed) as f32 / members_f;
        }
        SpacingReport {
            samples: n,
            clump_size: per(&bucket.clump_size_x10),
            clump_ge4_share: share(&bucket.clump_ge4),
            clump_ge6_share: share(&bucket.clump_ge6),
            clump_ball_gap_m: per(&bucket.clump_ball_gap_x10) * 0.125,
            clump_line_share,
            clump_attacking: bucket.clump_attacking.load(Ordering::Relaxed) as f32 / n as f32,
            clump_defending: bucket.clump_defending.load(Ordering::Relaxed) as f32 / n as f32,
            mate_gap_m: bucket.mate_gap_x10.load(Ordering::Relaxed) as f32 / 10.0 / mate_n * 0.125,
            mate_under5_share: bucket.mate_under5.load(Ordering::Relaxed) as f32 / mate_n,
            swarm_attacking: per(&bucket.swarm_attacking_x10),
            swarm_defending: per(&bucket.swarm_defending_x10),
            free: per(&bucket.free_x10),
            free_ahead: per(&bucket.free_ahead_x10),
            att_width_m: per(&bucket.att_width_x10) * 0.125,
            att_depth_m: per(&bucket.att_depth_x10) * 0.125,
        }
    }

    /// `(all play, ball in the final third)`.
    pub fn snapshot() -> (SpacingReport, SpacingReport) {
        (Self::read(&ALL), Self::read(&FINAL_THIRD))
    }
}

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Sample the spacing of both sides. Only while somebody is actually
    /// carrying the ball: a loose ball is a race and everybody converging
    /// on it is football, not a positional failure.
    pub(in crate::r#match::engine::engine) fn sample_spacing(
        field: &MatchField,
        context: &MatchContext,
    ) {
        if context.current_tick() % SAMPLE_INTERVAL_TICKS != 0 {
            return;
        }
        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let attacking_team = carrier.team_id;
        let ball = field.ball.position;

        // One pass to gather the outfielders, so every measure below
        // reads the same view. The keeper is excluded from the geometry
        // entirely — he is 40 m behind everybody and would flatten every
        // spread this file reports.
        let mut pos = [Vector3::zeros(); MAX_OUTFIELD];
        let mut ours = [false; MAX_OUTFIELD];
        let mut line = [0u8; MAX_OUTFIELD];
        let mut n = 0usize;
        for p in field.players.iter() {
            if n == MAX_OUTFIELD || p.is_sent_off {
                continue;
            }
            let position = p.tactical_position.current_position;
            if position.is_goalkeeper() {
                continue;
            }
            pos[n] = p.position;
            ours[n] = p.team_id == attacking_team;
            line[n] = if position.is_forward() {
                3
            } else if position.is_midfielder() {
                2
            } else {
                1
            };
            n += 1;
        }
        if n < 6 {
            return;
        }

        let Some(side) = carrier.side else { return };
        // "Ahead of the ball" is toward the goal this side is attacking,
        // and nothing else. Deriving it from the carrier's own body — the
        // ball sits a stride from his feet, so `ball - carrier` is a
        // dribbling offset — made the free-man count read as noise.
        let forward = Vector3::new(side.forward_dir_x(), 0.0, 0.0);
        let sample = Self::spacing_sample(&pos[..n], &ours[..n], &line[..n], ball, forward);
        let progress = side.attacking_progress_x(ball.x, field.size.width as f32);
        SpacingCensus::note(&sample, progress >= 2.0 / 3.0);
    }

    /// The geometry itself, split out so it is a pure function of a set
    /// of bodies and can be reasoned about (and tested) without a match.
    fn spacing_sample(
        pos: &[Vector3<f32>],
        ours: &[bool],
        line: &[u8],
        ball: Vector3<f32>,
        forward: Vector3<f32>,
    ) -> SpacingSample {
        let n = pos.len();

        // ── The clump ────────────────────────────────────────────────
        // Single-linkage at `CLUMP_LINK`, by flood fill. Twenty-one
        // bodies four times a second: the quadratic is cheaper than the
        // bookkeeping a union-find would need.
        let mut label = [usize::MAX; MAX_OUTFIELD];
        let mut stack = [0usize; MAX_OUTFIELD];
        let mut best_size = 0usize;
        let mut best_label = usize::MAX;
        let mut sizes = [0usize; MAX_OUTFIELD];
        for seed in 0..n {
            if label[seed] != usize::MAX {
                continue;
            }
            let mut depth = 0usize;
            label[seed] = seed;
            stack[depth] = seed;
            depth += 1;
            let mut size = 0usize;
            while depth > 0 {
                depth -= 1;
                let i = stack[depth];
                size += 1;
                for j in 0..n {
                    if label[j] != usize::MAX {
                        continue;
                    }
                    if (pos[j] - pos[i]).magnitude() <= CLUMP_LINK {
                        label[j] = seed;
                        stack[depth] = j;
                        depth += 1;
                    }
                }
            }
            sizes[seed] = size;
            if size > best_size {
                best_size = size;
                best_label = seed;
            }
        }

        let mut clump_by_line = [0u32; 4];
        let mut clump_attacking = 0u32;
        let mut clump_defending = 0u32;
        let mut centre = Vector3::zeros();
        for i in 0..n {
            if label[i] != best_label {
                continue;
            }
            clump_by_line[line[i] as usize] += 1;
            if ours[i] {
                clump_attacking += 1;
            } else {
                clump_defending += 1;
            }
            centre += pos[i];
        }
        let clump_ball_gap = if best_size > 0 {
            ((centre / best_size as f32) - ball).magnitude()
        } else {
            0.0
        };

        // ── Spread of the side in possession, and who is free ────────
        let mut mate_gap_sum = 0.0f32;
        let mut mate_n = 0u32;
        let mut mate_under5 = 0u32;
        let mut swarm_attacking = 0u32;
        let mut swarm_defending = 0u32;
        let mut free = 0u32;
        let mut free_ahead = 0u32;
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
        for i in 0..n {
            let to_ball = (pos[i] - ball).magnitude();
            if !ours[i] {
                if to_ball <= BALL_RING {
                    swarm_defending += 1;
                }
                continue;
            }
            min_x = min_x.min(pos[i].x);
            max_x = max_x.max(pos[i].x);
            min_y = min_y.min(pos[i].y);
            max_y = max_y.max(pos[i].y);
            if to_ball <= BALL_RING {
                swarm_attacking += 1;
            }
            // The carrier is not measured: he is where the ball is by
            // definition, and his nearest team-mate is a pressing
            // question, not a spacing one.
            if to_ball < 1.0 {
                continue;
            }
            let mut nearest_mate = f32::MAX;
            let mut nearest_opp = f32::MAX;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let d = (pos[j] - pos[i]).magnitude();
                if ours[j] {
                    nearest_mate = nearest_mate.min(d);
                } else {
                    nearest_opp = nearest_opp.min(d);
                }
            }
            if nearest_mate.is_finite() {
                mate_gap_sum += nearest_mate;
                mate_n += 1;
                if nearest_mate < 40.0 {
                    mate_under5 += 1;
                }
            }
            if nearest_opp > FREE_RADIUS {
                free += 1;
                if (pos[i] - ball).dot(&forward) > 0.0 {
                    free_ahead += 1;
                }
            }
        }

        SpacingSample {
            clump_size: best_size,
            clump_ball_gap,
            clump_by_line,
            clump_attacking,
            clump_defending,
            mate_gap_sum,
            mate_n,
            mate_under5,
            swarm_attacking,
            swarm_defending,
            free,
            free_ahead,
            att_width: if min_y <= max_y { max_y - min_y } else { 0.0 },
            att_depth: if min_x <= max_x { max_x - min_x } else { 0.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vector3<f32> {
        Vector3::new(x, y, 0.0)
    }

    /// A line of players spaced further apart than `CLUMP_LINK` is not a
    /// clump, however many of them there are.
    #[test]
    fn a_spread_side_has_no_clump() {
        let pos: Vec<_> = (0..8).map(|i| v(i as f32 * 60.0, 100.0)).collect();
        let ours = vec![true; 8];
        let line = vec![2u8; 8];
        let s = FootballEngine::<840, 545>::spacing_sample(
            &pos,
            &ours,
            &line,
            v(0.0, 100.0),
            v(1.0, 0.0),
        );
        assert_eq!(s.clump_size, 1, "spaced players joined a clump");
        assert_eq!(s.mate_under5, 0, "60u apart counted as under 5 m");
    }

    /// …and four bodies inside three metres of each other are one,
    /// whichever side they are on.
    #[test]
    fn a_pile_is_one_clump() {
        let pos = vec![
            v(400.0, 270.0),
            v(410.0, 272.0),
            v(405.0, 285.0),
            v(415.0, 288.0),
            v(700.0, 100.0),
        ];
        let ours = vec![true, false, true, false, true];
        let line = vec![3u8, 1, 2, 1, 2];
        let s = FootballEngine::<840, 545>::spacing_sample(
            &pos,
            &ours,
            &line,
            v(400.0, 270.0),
            v(1.0, 0.0),
        );
        assert_eq!(s.clump_size, 4);
        assert_eq!(s.clump_attacking, 2);
        assert_eq!(s.clump_defending, 2);
    }

    /// A man with nobody near him is free, and he is only "ahead" when
    /// he is on the far side of the ball from the carrier's own goal.
    #[test]
    fn free_counts_only_unmarked_men() {
        // Carrier at x=300 playing toward +x; one team-mate upfield in
        // space, one marked.
        let pos = vec![
            v(300.0, 270.0), // carrier
            v(600.0, 200.0), // free, ahead
            v(500.0, 400.0), // marked
            v(505.0, 400.0), // his marker
            v(100.0, 270.0), // free, behind
        ];
        let ours = vec![true, true, true, false, true];
        let line = vec![2u8, 3, 3, 1, 1];
        let s = FootballEngine::<840, 545>::spacing_sample(
            &pos,
            &ours,
            &line,
            v(300.0, 270.0),
            v(1.0, 0.0),
        );
        assert_eq!(s.free, 2, "wrong number of free men");
        assert_eq!(s.free_ahead, 1, "the man behind the ball counted as ahead");
    }
}
