//! **The mid-air census.** Who is given a ball at what height, who
//! strikes one at what height, how often a ball is taken out of the air,
//! and what a refused strike costs in ticks before it is struck for
//! real.
//!
//! Added, never replacing anything: the whole-tick relocation census
//! ([`teleport`](crate::r#match::engine::ball::ball::teleport)) already
//! books a `vertical` column and must not grow, and
//! [`reception_diag`](crate::r#match::engine::ball::ball::ownership::reception_diag)
//! already books the horizontal half of every grant. Neither of them can
//! see the vertical axis of a GRANT or of a STRIKE, which is where the
//! reported artefact lived: *"sometimes I see a ball pass event when the
//! ball is flying, and it looks weird. It feels like it's flying and
//! being hit."*
//!
//! Every counter here is a plain process-wide atomic, in the same shape
//! as its neighbours, so a parallel `dev_match` run aggregates by
//! addition and `reset()` clears a campaign between arms.

use std::sync::atomic::{AtomicU64, Ordering};

/// Height bands, in metres, that the whole census is cut by.
///
/// The edges are the model's own numbers rather than round ones:
/// `DECK` is where a ball stops being airborne at all (the same 0.5 m
/// the report's own census used), [`Band::BOOT`] tops out at
/// `AerialReach::VOLLEY`, and [`Band::HEAD`] tops out at the highest
/// jump anybody has (`AerialReach::HIGHEST`, 3.1 m). Anything above
/// that is a ball nobody may touch, and the row must read zero.
pub struct Band;

impl Band {
    pub const COUNT: usize = 4;
    pub const NAMES: [&'static str; Self::COUNT] =
        ["deck <=0.5", "boot 0.5-1.45", "head 1.45-3.1", "OVER >3.1"];

    /// Which band this height falls in.
    #[inline]
    pub fn of(height: f32) -> usize {
        if height <= 0.5 {
            0
        } else if height <= 1.45 {
            1
        } else if height <= 3.1 {
            2
        } else {
            3
        }
    }
}

/// The four handlers that write a velocity onto the ball, plus the
/// grant paths that hand it to somebody.
pub struct Handler;

impl Handler {
    pub const COUNT: usize = 4;
    pub const PASS: usize = 0;
    pub const SHOOT: usize = 1;
    pub const CLEAR: usize = 2;
    pub const MOVE: usize = 3;
    pub const NAMES: [&'static str; Self::COUNT] = ["PassTo", "Shoot", "ClearBall", "MoveBall"];
}

/// Where a grant came from. The event path is the one that had no
/// ceiling at all; the others each had their own.
pub struct GrantPath;

impl GrantPath {
    pub const COUNT: usize = 4;
    /// `check_ball_ownership`'s per-tick claim scan.
    pub const SCAN: usize = 0;
    /// The intended receiver of a pass taking control of it.
    pub const RECEIVER: usize = 1;
    /// A `PlayerEvent` — `ClaimBall`, `GainBall`, `TacklingBall`,
    /// `MoveBall`, `BallOwnerChange`, `CaughtBall`.
    pub const EVENT: usize = 2;
    /// `try_intercept` / `try_block_shot` / the keeper's save.
    pub const CONTEST: usize = 3;
    pub const NAMES: [&'static str; Self::COUNT] = ["scan", "receiver", "event", "contest"];
}

const ZERO: AtomicU64 = AtomicU64::new(0);

/// Strikes by handler × height band.
static STRIKES: [AtomicU64; Handler::COUNT * Band::COUNT] = [ZERO; Handler::COUNT * Band::COUNT];
/// …the outfield subset struck above [`AerialReach::VOLLEY`], split by
/// whether the striker's state had declared itself aerial. The second of
/// those is a header; the first is the artefact.
static HEAD_BAND_STRIKES: [AtomicU64; 2] = [ZERO; 2];
/// Grants by path × height band.
static GRANTS: [AtomicU64; GrantPath::COUNT * Band::COUNT] = [ZERO; GrantPath::COUNT * Band::COUNT];
/// Balls taken out of the air: count, and the height each was taken at
/// summed × 1000 so a mean can be read back.
static CONTROLS: [AtomicU64; Band::COUNT] = [ZERO; Band::COUNT];
static CONTROL_HEIGHT_MM: AtomicU64 = AtomicU64::new(0);
/// Controls that reached the deck with nobody under them — a loose ball,
/// the same shape as a failed first touch.
static CONTROLS_LOST: AtomicU64 = AtomicU64::new(0);
/// Strikes refused for being out of reach, by handler.
static REFUSED: [AtomicU64; Handler::COUNT] = [ZERO; Handler::COUNT];
/// …and how many of those the same player went on to strike for real,
/// with the ticks it took him summed against them. This is the
/// deadlock proof: a refusal that is never re-struck is a state stuck
/// asking for a ball it cannot have.
static RESTRUCK: [AtomicU64; Handler::COUNT] = [ZERO; Handler::COUNT];
static RESTRIKE_TICKS: [AtomicU64; Handler::COUNT] = [ZERO; Handler::COUNT];

/// Pending refusals, so a re-strike can be timed without a map.
///
/// A fixed table indexed by `player_id % SLOTS`: two players colliding
/// costs the census one measurement and nothing else, which is the right
/// trade for a counter that runs inside the event dispatcher. Each slot
/// holds `(handler << 56) | tick`, or 0 for empty.
const SLOTS: usize = 64;
static PENDING: [AtomicU64; SLOTS] = [ZERO; SLOTS];

/// How long a refusal stays open, in ticks.
///
/// The question this census answers is "did the refused strike happen a
/// moment later, once the ball was down?" — so the window has to be the
/// length of that moment and not the length of the match. A ball
/// controlled at chest height reaches the feet in about half a second
/// under gravity; 3 s covers that with room for the state to come back
/// round. Beyond it the same man striking the ball again is a different
/// action entirely, and counting it would report a 20-second "re-strike
/// delay" that means nothing.
const RESTRIKE_WINDOW: u64 = 300;

/// The census itself. All associated functions — there is no instance
/// state, the counters are process-wide, and the shape matches every
/// other diagnostic in this directory.
pub struct StrikeCensus;

impl StrikeCensus {
    /// A strike that actually happened, at the height the ball was at
    /// when the handler ran.
    pub fn note_strike(handler: usize, height: f32, aerial_state: bool, keeper: bool) {
        STRIKES[handler * Band::COUNT + Band::of(height)].fetch_add(1, Ordering::Relaxed);
        if !keeper && height > 1.45 {
            HEAD_BAND_STRIKES[aerial_state as usize].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A grant of possession, at the height the ball was at.
    pub fn note_grant(path: usize, height: f32) {
        GRANTS[path * Band::COUNT + Band::of(height)].fetch_add(1, Ordering::Relaxed);
    }

    /// The touch: a ball taken out of the air by the man who has just
    /// been given it. This is what replaced the string-pull — the count
    /// is the same population the old `vz = -10, no acceleration` census
    /// found, and the heights should be the same heights.
    pub fn note_control(height: f32) {
        CONTROLS[Band::of(height)].fetch_add(1, Ordering::Relaxed);
        CONTROL_HEIGHT_MM.fetch_add((height * 1000.0).max(0.0) as u64, Ordering::Relaxed);
    }

    /// …and the ones that landed with the man no longer under them.
    pub fn note_control_lost() {
        CONTROLS_LOST.fetch_add(1, Ordering::Relaxed);
    }

    /// A strike refused because the striker could not reach the ball.
    pub fn note_refusal(handler: usize, player_id: u32, tick: u64) {
        REFUSED[handler].fetch_add(1, Ordering::Relaxed);
        PENDING[player_id as usize % SLOTS].store(
            ((handler as u64) << 56) | (tick & 0x00FF_FFFF_FFFF_FFFF),
            Ordering::Relaxed,
        );
    }

    /// A strike that landed. If this player had a refusal outstanding,
    /// close it and book how long it took him.
    pub fn note_restrike(player_id: u32, tick: u64) {
        let slot = &PENDING[player_id as usize % SLOTS];
        let pending = slot.swap(0, Ordering::Relaxed);
        if pending == 0 {
            return;
        }
        let handler = (pending >> 56) as usize;
        let refused_at = pending & 0x00FF_FFFF_FFFF_FFFF;
        if handler >= Handler::COUNT {
            return;
        }
        let waited = tick.saturating_sub(refused_at);
        if waited > RESTRIKE_WINDOW {
            // Too long ago to be the same attempt — see
            // [`RESTRIKE_WINDOW`]. The refusal lapses uncounted, which is
            // what makes `refused` minus `restruck` mean "the state gave
            // up or moved on" rather than "the match ended".
            return;
        }
        RESTRUCK[handler].fetch_add(1, Ordering::Relaxed);
        RESTRIKE_TICKS[handler].fetch_add(waited, Ordering::Relaxed);
    }

    /// `(strikes, head-band [accidental, headed], grants, controls,
    /// control height mm, controls lost, refused, restruck,
    /// restrike ticks)`
    #[allow(clippy::type_complexity)]
    pub fn snapshot() -> (
        [u64; Handler::COUNT * Band::COUNT],
        [u64; 2],
        [u64; GrantPath::COUNT * Band::COUNT],
        [u64; Band::COUNT],
        u64,
        u64,
        [u64; Handler::COUNT],
        [u64; Handler::COUNT],
        [u64; Handler::COUNT],
    ) {
        (
            std::array::from_fn(|i| STRIKES[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| HEAD_BAND_STRIKES[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| GRANTS[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| CONTROLS[i].load(Ordering::Relaxed)),
            CONTROL_HEIGHT_MM.load(Ordering::Relaxed),
            CONTROLS_LOST.load(Ordering::Relaxed),
            std::array::from_fn(|i| REFUSED[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| RESTRUCK[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| RESTRIKE_TICKS[i].load(Ordering::Relaxed)),
        )
    }

    pub fn reset() {
        for c in STRIKES.iter().chain(GRANTS.iter()) {
            c.store(0, Ordering::Relaxed);
        }
        for c in HEAD_BAND_STRIKES
            .iter()
            .chain(CONTROLS.iter())
            .chain(REFUSED.iter())
            .chain(RESTRUCK.iter())
            .chain(RESTRIKE_TICKS.iter())
            .chain(PENDING.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
        CONTROL_HEIGHT_MM.store(0, Ordering::Relaxed);
        CONTROLS_LOST.store(0, Ordering::Relaxed);
    }
}
