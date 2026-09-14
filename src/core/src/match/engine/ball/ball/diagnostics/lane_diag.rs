//! **The lane census** — how close the nearest opponent came to a pass
//! while it was in flight, and whether the pass still got through.
//!
//! Reported from the viewer: *"defenders fail to intercept passes, the
//! ball simply gets past them."* The interception counter cannot see
//! that, because it counts the passes that were taken and not the ones
//! that rolled through somebody's feet. This books every pass once, at
//! its next touch, against the closest any opponent got to the ball on
//! the way — so the read-out is "of the passes that went within a metre
//! of an opponent, this many were completed anyway". `match-logs` only.

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use std::sync::atomic::{AtomicU64, Ordering};

/// Band edges in game units (1u = 0.125 m): half a metre, a metre, a
/// metre and a half, two metres, and clear.
const EDGES: [f32; 4] = [4.0, 8.0, 12.0, 16.0];
const BANDS: usize = EDGES.len() + 1;

/// Passes booked, by the band of their nearest opponent.
static PASSES: [AtomicU64; BANDS] = [const { AtomicU64::new(0) }; BANDS];
/// The same passes by what the PASSER priced the lane at when he struck
/// it (`PassEvaluator::calculate_interception_risk`, recomputed at the
/// strike): passes, and the ones an opponent then won. Interceptions
/// piling into the low bands mean the passer could not see them coming;
/// into the high bands, that he played them anyway.
const PRED_EDGES: [f32; 3] = [0.1, 0.3, 0.6];
const PREDS: usize = PRED_EDGES.len() + 1;
static PRED_PASSES: [AtomicU64; PREDS] = [const { AtomicU64::new(0) }; PREDS];
static PRED_WON: [AtomicU64; PREDS] = [const { AtomicU64::new(0) }; PREDS];
static PRED_SUM_X10000: [AtomicU64; PREDS] = [const { AtomicU64::new(0) }; PREDS];
/// …whose next touch was the passer's own side.
static THROUGH: [AtomicU64; BANDS] = [const { AtomicU64::new(0) }; BANDS];
/// …whose next touch was an opponent.
static WON: [AtomicU64; BANDS] = [const { AtomicU64::new(0) }; BANDS];
/// The interception rolls themselves, by how long after the strike they
/// were made: how many, at what mean chance, from what mean distance,
/// and how many fired.
const AGE_EDGES: [f32; 3] = [5.0, 20.0, 60.0];
const AGES: usize = AGE_EDGES.len() + 1;
static ROLLS: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static ROLL_CHANCE_X10000: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static ROLL_GAP_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static FIRED: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
/// Where the crossing was: the ball's distance to the man it was played
/// to, how far it had travelled since the strike, the interceptor's
/// distance to that man, and the ball's speed — all x100, summed.
static TO_TARGET_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static TRAVELLED_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static MAN_TO_TARGET_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static SPEED_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
/// Where the man rolled for stood when the pass was STRUCK: his distance
/// off the lane then (x100, over the rolls it is known for), and how
/// many of them were outside a stride and a leg of it — men who moved
/// into the lane during the flight, whom no passer could have seen.
static STRIKE_PERP_X100: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static STRIKE_KNOWN: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
static STRIKE_FROM_OUTSIDE: [AtomicU64; AGES] = [const { AtomicU64::new(0) }; AGES];
/// What the man rolled for was DOING — which behaviour carried him to
/// the lane. Indexed by [`LaneDiag::STATES`].
const STATES: usize = 8;
static ROLL_STATE: [[AtomicU64; STATES]; AGES] =
    [const { [const { AtomicU64::new(0) }; STATES] }; AGES];

/// The open book on one pass: the side that played it and the nearest
/// any opponent has come to the ball so far. Lives on the ball from the
/// strike to the next touch.
#[derive(Clone, Copy, Debug)]
pub struct LaneCensus {
    pub passer_team: u32,
    pub min_gap: f32,
    /// What the passer priced the lane at as he struck it.
    pub predicted: f32,
    /// Every opponent's distance off the lane at the strike, by id.
    pub at_strike: [(u32, f32); 16],
    pub at_strike_len: usize,
}

impl LaneCensus {
    pub fn open(passer_team: u32, predicted: f32) -> Self {
        Self {
            passer_team,
            min_gap: f32::MAX,
            predicted,
            at_strike: [(0, f32::MAX); 16],
            at_strike_len: 0,
        }
    }

    /// One opponent's distance off the lane as the ball was struck.
    pub fn note_at_strike(&mut self, id: u32, perp: f32) {
        if self.at_strike_len < self.at_strike.len() {
            self.at_strike[self.at_strike_len] = (id, perp);
            self.at_strike_len += 1;
        }
    }

    /// Where he stood at the strike, if he was booked.
    pub fn perp_at_strike(&self, id: u32) -> Option<f32> {
        self.at_strike[..self.at_strike_len]
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, p)| *p)
    }

    /// One opponent, one tick: the distance across the grass.
    #[inline]
    pub fn note_gap(&mut self, gap: f32) {
        if gap < self.min_gap {
            self.min_gap = gap;
        }
    }

    /// The next touch. A touch by the passer's own side means the ball
    /// got through whoever it passed; anything else and it did not.
    pub fn close(self, toucher_team: u32) {
        let band = EDGES
            .iter()
            .position(|&e| self.min_gap < e)
            .unwrap_or(BANDS - 1);
        PASSES[band].fetch_add(1, Ordering::Relaxed);
        let pred = PRED_EDGES
            .iter()
            .position(|&e| self.predicted < e)
            .unwrap_or(PREDS - 1);
        PRED_PASSES[pred].fetch_add(1, Ordering::Relaxed);
        PRED_SUM_X10000[pred].fetch_add((self.predicted * 10_000.0) as u64, Ordering::Relaxed);
        if toucher_team == self.passer_team {
            THROUGH[band].fetch_add(1, Ordering::Relaxed);
        } else {
            WON[band].fetch_add(1, Ordering::Relaxed);
            PRED_WON[pred].fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct LaneDiag;

impl LaneDiag {
    pub const LABELS: [&'static str; BANDS] = ["≤0.5 m", "≤1 m", "≤1.5 m", "≤2 m", "clear"];

    pub const AGE_LABELS: [&'static str; AGES] = ["<5 ticks", "<20", "<60", "60+"];

    pub const STATES: [&'static str; STATES] = [
        "TakeBall",
        "Intercepting",
        "Pressing",
        "Marking",
        "Covering/HoldingLine",
        "Running/Returning",
        "Standing/Walking",
        "other",
    ];

    fn state_slot(state: &PlayerState) -> usize {
        match state {
            PlayerState::Defender(DefenderState::TakeBall)
            | PlayerState::Midfielder(MidfielderState::TakeBall)
            | PlayerState::Forward(ForwardState::TakeBall)
            | PlayerState::Goalkeeper(GoalkeeperState::TakeBall) => 0,
            PlayerState::Defender(DefenderState::Intercepting)
            | PlayerState::Midfielder(MidfielderState::Intercepting)
            | PlayerState::Forward(ForwardState::Intercepting) => 1,
            PlayerState::Defender(DefenderState::Pressing)
            | PlayerState::Midfielder(MidfielderState::Pressing)
            | PlayerState::Forward(ForwardState::Pressing) => 2,
            PlayerState::Defender(DefenderState::Marking)
            | PlayerState::Midfielder(MidfielderState::Guarding) => 3,
            PlayerState::Defender(DefenderState::Covering | DefenderState::HoldingLine) => 4,
            PlayerState::Defender(DefenderState::Running | DefenderState::Returning)
            | PlayerState::Midfielder(MidfielderState::Running | MidfielderState::Returning)
            | PlayerState::Forward(ForwardState::Running | ForwardState::Returning) => 5,
            PlayerState::Defender(DefenderState::Standing | DefenderState::Walking)
            | PlayerState::Midfielder(MidfielderState::Standing | MidfielderState::Walking)
            | PlayerState::Forward(ForwardState::Standing | ForwardState::Walking) => 6,
            _ => 7,
        }
    }

    /// `(label, rolls)` per state for one age band, heaviest first.
    pub fn states(age: usize) -> Vec<(&'static str, u64)> {
        let mut rows: Vec<(&'static str, u64)> = Self::STATES
            .iter()
            .enumerate()
            .map(|(i, l)| (*l, ROLL_STATE[age][i].load(Ordering::Relaxed)))
            .filter(|(_, n)| *n > 0)
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        rows
    }

    /// One interception roll, at the distance it was rolled from and the
    /// time since the strike.
    #[allow(clippy::too_many_arguments)]
    pub fn note_roll(
        chance: f32,
        gap: f32,
        since_strike: f32,
        fired: bool,
        to_target: f32,
        travelled: f32,
        man_to_target: f32,
        speed: f32,
        perp_at_strike: Option<f32>,
        reach: f32,
        state: &PlayerState,
    ) {
        let a = AGE_EDGES
            .iter()
            .position(|&e| since_strike < e)
            .unwrap_or(AGES - 1);
        ROLLS[a].fetch_add(1, Ordering::Relaxed);
        ROLL_STATE[a][Self::state_slot(state)].fetch_add(1, Ordering::Relaxed);
        if let Some(perp) = perp_at_strike {
            STRIKE_KNOWN[a].fetch_add(1, Ordering::Relaxed);
            STRIKE_PERP_X100[a].fetch_add((perp.min(200.0) * 100.0) as u64, Ordering::Relaxed);
            if perp > reach {
                STRIKE_FROM_OUTSIDE[a].fetch_add(1, Ordering::Relaxed);
            }
        }
        TO_TARGET_X100[a].fetch_add((to_target * 100.0) as u64, Ordering::Relaxed);
        TRAVELLED_X100[a].fetch_add((travelled * 100.0) as u64, Ordering::Relaxed);
        MAN_TO_TARGET_X100[a].fetch_add((man_to_target * 100.0) as u64, Ordering::Relaxed);
        SPEED_X100[a].fetch_add((speed * 100.0) as u64, Ordering::Relaxed);
        ROLL_CHANCE_X10000[a].fetch_add((chance * 10_000.0) as u64, Ordering::Relaxed);
        ROLL_GAP_X100[a].fetch_add((gap * 100.0) as u64, Ordering::Relaxed);
        if fired {
            FIRED[a].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub const PRED_LABELS: [&'static str; PREDS] = ["<10%", "<30%", "<60%", "60%+"];

    /// `(label, passes, won, mean predicted)` per predicted-risk band.
    pub fn by_prediction() -> Vec<(&'static str, u64, u64, f32)> {
        (0..PREDS)
            .map(|b| {
                let n = PRED_PASSES[b].load(Ordering::Relaxed);
                (
                    Self::PRED_LABELS[b],
                    n,
                    PRED_WON[b].load(Ordering::Relaxed),
                    PRED_SUM_X10000[b].load(Ordering::Relaxed) as f32 / 10_000.0 / n.max(1) as f32,
                )
            })
            .collect()
    }

    /// `(label, passes, through, won)` per band.
    pub fn by_band() -> Vec<(&'static str, u64, u64, u64)> {
        (0..BANDS)
            .map(|b| {
                (
                    Self::LABELS[b],
                    PASSES[b].load(Ordering::Relaxed),
                    THROUGH[b].load(Ordering::Relaxed),
                    WON[b].load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// `(label, rolls, fired, mean chance, mean gap u, mean ball-to-target
    /// u, mean travelled u, mean man-to-target u, mean speed, mean
    /// off-lane distance at the strike u, share that came from outside
    /// reach)` per age band.
    #[allow(clippy::type_complexity)]
    pub fn rolls() -> Vec<(
        &'static str,
        u64,
        u64,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    )> {
        (0..AGES)
            .map(|a| {
                let n = ROLLS[a].load(Ordering::Relaxed);
                let f = n.max(1) as f32;
                let per = |c: &AtomicU64| c.load(Ordering::Relaxed) as f32 / 100.0 / f;
                let known = STRIKE_KNOWN[a].load(Ordering::Relaxed).max(1) as f32;
                (
                    Self::AGE_LABELS[a],
                    n,
                    FIRED[a].load(Ordering::Relaxed),
                    ROLL_CHANCE_X10000[a].load(Ordering::Relaxed) as f32 / 10_000.0 / f,
                    per(&ROLL_GAP_X100[a]),
                    per(&TO_TARGET_X100[a]),
                    per(&TRAVELLED_X100[a]),
                    per(&MAN_TO_TARGET_X100[a]),
                    per(&SPEED_X100[a]),
                    STRIKE_PERP_X100[a].load(Ordering::Relaxed) as f32 / 100.0 / known,
                    STRIKE_FROM_OUTSIDE[a].load(Ordering::Relaxed) as f32 / known,
                )
            })
            .collect()
    }

    pub fn reset() {
        for c in PASSES
            .iter()
            .chain(THROUGH.iter())
            .chain(WON.iter())
            .chain(PRED_PASSES.iter())
            .chain(PRED_WON.iter())
            .chain(PRED_SUM_X10000.iter())
            .chain(ROLLS.iter())
            .chain(ROLL_CHANCE_X10000.iter())
            .chain(ROLL_GAP_X100.iter())
            .chain(FIRED.iter())
            .chain(TO_TARGET_X100.iter())
            .chain(TRAVELLED_X100.iter())
            .chain(MAN_TO_TARGET_X100.iter())
            .chain(SPEED_X100.iter())
            .chain(STRIKE_PERP_X100.iter())
            .chain(STRIKE_KNOWN.iter())
            .chain(STRIKE_FROM_OUTSIDE.iter())
            .chain(ROLL_STATE.iter().flatten())
        {
            c.store(0, Ordering::Relaxed);
        }
    }
}
