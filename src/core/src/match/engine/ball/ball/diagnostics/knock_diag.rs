//! **The KEEPER KNOCK-CHAIN census** — the instrument for *"sometimes he
//! kicks the ball around with his hands and runs after it himself, and
//! sometimes the ball even rolls out of bounds"*.
//!
//! # Why a chain and not a counter
//!
//! Every mechanism that puts a ball back into play off a goalkeeper is
//! already counted somewhere: [`KeeperBodyDiag`] counts the capsule,
//! `SaveContactDiag` counts the spill, `KeeperActionDiag` counts the
//! smother and the punch. None of those counters can see the reported
//! behaviour, because the behaviour is not *a* contact — it is the SECOND
//! one. A keeper who parries a shot and gathers the rebound has done his
//! job; a keeper who touches the same ball three times in two seconds and
//! ends up chasing it over the byline has done the thing the report
//! describes, and the two are indistinguishable in any per-contact total.
//!
//! So this census links contacts. Every touch that leaves the ball LOOSE
//! off a goalkeeper extends a chain belonging to that keeper; a gap longer
//! than [`KnockChain::LINK_TICKS`] starts a new one. When the chain ends —
//! somebody controls the ball, or it leaves the pitch, or three seconds
//! pass with it lying there — the length and the ending are booked, and
//! the two numbers that matter fall straight out of the table:
//!
//! * **chains of length ≥ 2 per match** — the knock-chase loop itself;
//! * **chains that ended out of play with nobody near him** — the
//!   ball rolling out off his own touch, which has no reading at all.
//!
//! # What is deliberately NOT a knock
//!
//! A punt, a throw, a goal kick and a clearance are all a keeper putting
//! the ball where he means to put it, and every one of them starts from a
//! ball he OWNS — so the chain has already closed as
//! [`KnockEnd::Gloves`] or [`KnockEnd::Feet`] before he strikes it. The one
//! deliberate release that never passes through ownership is the parry
//! tipped round the post, and that closes its chain explicitly as
//! [`KnockEnd::Cleared`] at the site: it is a save, not a fumble, and the
//! ball is meant to go out.
//!
//! # Coverage
//!
//! Two sites book a knock by name — the capsule and the spilled parry —
//! and everything else is caught by [`Ball::settle_keeper_knock`], which
//! reads the ball's own `last_touch_*` bookkeeping once a tick and opens a
//! chain for any UNCONTROLLED touch by a goalkeeper that is not a release
//! he made himself. So a route into a loose keeper contact that nobody
//! thought of is still seen, which is the property a census needs and a
//! hand-placed counter never has.
//!
//! ⚠ **The punch and the smother's knock-away are the known gap**, and
//! deliberately: both leave the ball through `ClearBall` on a ball nobody
//! owns, which records no touch at all, and both are aimed 15-25 m
//! clearances rather than a fumble at his feet. `KeeperActionDiag` counts
//! them.

use crate::PlayerFieldPositionGroup;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::ball::ball::Ball;
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Where the ball came off him.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KnockSource {
    /// The swept capsule — a ball that ran into a man who was not
    /// gathering it. See
    /// [`KeeperBody`](crate::r#match::engine::ball::ball::contest::body::KeeperBody).
    Body = 0,
    /// A spilled parry: the save model's dangerous outcome.
    Spill = 1,
    /// The 1-v-1 he won with his body rather than his gloves —
    /// `KeeperSmother`'s `Blocked`.
    Smother = 2,
    /// A fist.
    Punch = 3,
    /// Anything else that left the ball loose off a goalkeeper.
    Other = 4,
}

/// How a chain ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KnockEnd {
    /// He got it into his gloves. The outcome every other one is measured
    /// against.
    Gloves = 0,
    /// …or under his boot, which is the same thing under Law 12.
    Feet = 1,
    /// Somebody on his own side took it away — including him putting it
    /// out on purpose.
    Cleared = 2,
    /// An opponent did.
    ToOpponent = 3,
    /// It left the pitch.
    OutOfPlay = 4,
    /// Nobody did anything with it for [`KnockChain::LINK_TICKS`].
    Lapsed = 5,
}

/// One goalkeeper's run of loose contacts with the same ball.
///
/// Lives on the [`Ball`] rather than in a static because a chain is a
/// property of one ball in one match, and the counters below are shared by
/// every match in a harness run.
#[derive(Clone, Debug)]
pub struct KnockChain {
    keeper: u32,
    contacts: u16,
    /// Engine tick of the most recent contact…
    last_tick: u64,
    /// …and of the first, so the chain has a duration as well as a count.
    opened_tick: u64,
    /// Where the first contact happened, so it has a length on the grass.
    from: Vector3<f32>,
    /// An opponent was inside [`Self::PRESSED`] of him at one of the
    /// contacts. A keeper who spills a ball with a striker on his shoulder
    /// is playing football; one who does it alone in his six-yard box is
    /// the report.
    pressed: bool,
    /// Which mechanisms it is made of, as a bit per [`KnockSource`].
    sources: u8,
}

impl KnockChain {
    /// How long a gap between two contacts still counts as the same
    /// passage of play. 300 engine ticks = 3 s: long enough for him to
    /// knock a ball five metres and run onto it, short enough that two
    /// unrelated saves in the same attack are two chains.
    pub const LINK_TICKS: u64 = 300;

    /// An opponent this close makes the contact a pressed one. 40u = 5 m,
    /// the same radius `KeeperFeetDecision::PRESS_RANGE` calls pressure.
    const PRESSED: f32 = 40.0;

    fn opened(
        keeper: u32,
        tick: u64,
        at: Vector3<f32>,
        pressed: bool,
        source: KnockSource,
    ) -> Self {
        KnockChain {
            keeper,
            contacts: 1,
            last_tick: tick,
            opened_tick: tick,
            from: at,
            pressed,
            sources: 1 << (source as u8),
        }
    }

    /// Is there an opponent close enough to him for this contact to be a
    /// pressed one?
    fn under_pressure(keeper: &MatchPlayer, players: &[MatchPlayer]) -> bool {
        players.iter().any(|p| {
            p.team_id != keeper.team_id
                && !p.is_sent_off
                && (p.position.x - keeper.position.x).hypot(p.position.y - keeper.position.y)
                    <= Self::PRESSED
        })
    }
}

/// **The knock-chain table.**
///
/// ```text
///  0        chains closed
///  1        loose contacts booked into them
///  2..=7    chain-length histogram: 1, 2, 3, 4, 5, 6 or more
///  8..=13   how they ended, indexed by `KnockEnd`
/// 14..=19   the same, for chains of length >= 2 only
/// 20..=24   contacts by `KnockSource`
/// 25..=29   the same, in chains of length >= 2
/// 30        chains that ended out of play with nobody near him at any
///           contact — the number that has no defensible reading
/// 31        Σ of how far the ball travelled from the first contact to the
///           close, in game units
/// 32/33     of the out-of-play endings, the ones that left BEHIND the
///           goal line and the ones that left over a TOUCHLINE
/// 34        Σ of how long they lasted, first contact to close, in engine
///           ticks — three touches over two seconds and three over a
///           fifth of one are not the same passage of play
/// ```
///
/// Slots 32 and 33 are what tell the two readings of an out-of-play ending
/// apart, and they are opposite verdicts: a shot deflecting off a keeper
/// and running behind for a corner is football and every match has a
/// couple, while a ball trickling over a touchline off his own hands is the
/// report.
///
/// Slot 30 is the acceptance criterion and slot 3 (chains of exactly two)
/// plus 4..=7 are the loop itself. Read them per match.
pub static KNOCK: [AtomicU64; 35] = [const { AtomicU64::new(0) }; 35];

pub struct KnockDiag;

impl KnockDiag {
    fn note(slot: usize) {
        KNOCK[slot].fetch_add(1, Ordering::Relaxed);
    }

    fn add(slot: usize, n: u64) {
        KNOCK[slot].fetch_add(n, Ordering::Relaxed);
    }

    /// Book a closed chain. `behind` says which line an out-of-play
    /// ending crossed, and is `None` for every other ending.
    fn close(chain: &KnockChain, end: KnockEnd, travelled: f32, held: u64, behind: Option<bool>) {
        Self::note(0);
        Self::add(1, chain.contacts as u64);
        let length = (chain.contacts as usize).clamp(1, 6);
        Self::note(1 + length);
        Self::note(8 + end as usize);
        let long = chain.contacts >= 2;
        if long {
            Self::note(14 + end as usize);
        }
        for source in 0..5usize {
            if chain.sources & (1 << source) != 0 {
                Self::note(20 + source);
                if long {
                    Self::note(25 + source);
                }
            }
        }
        if end == KnockEnd::OutOfPlay && !chain.pressed {
            Self::note(30);
        }
        Self::add(31, travelled.max(0.0) as u64);
        Self::add(34, held);
        if let Some(behind) = behind {
            Self::note(if behind { 32 } else { 33 });
        }
    }

    pub fn snapshot() -> [u64; 35] {
        let mut out = [0u64; 35];
        for (slot, c) in out.iter_mut().zip(KNOCK.iter()) {
            *slot = c.load(Ordering::Relaxed);
        }
        out
    }
}

impl Ball {
    /// **One loose contact off a goalkeeper**, extending his chain or
    /// starting a new one.
    ///
    /// Called from every site that leaves the ball travelling off a keeper
    /// without him controlling it. A contact by a DIFFERENT keeper, or one
    /// more than [`KnockChain::LINK_TICKS`] after the last, closes the open
    /// chain before opening its own — the first because a chain belongs to
    /// one man, the second because two touches three seconds apart are two
    /// passages of play.
    pub fn note_keeper_knock(
        &mut self,
        keeper: &MatchPlayer,
        source: KnockSource,
        players: &[MatchPlayer],
    ) {
        let pressed = KnockChain::under_pressure(keeper, players);
        self.extend_keeper_knock(keeper.id, source, pressed);
    }

    /// The body of the above, once the pressure question has been answered
    /// — shared with the catch-all sweep, which has already had to answer
    /// it for itself.
    fn extend_keeper_knock(&mut self, keeper: u32, source: KnockSource, pressed: bool) {
        let tick = self.current_tick_cached;
        match self.knock_chain.as_mut() {
            Some(chain)
                if chain.keeper == keeper
                    && tick.saturating_sub(chain.last_tick) <= KnockChain::LINK_TICKS =>
            {
                chain.contacts = chain.contacts.saturating_add(1);
                chain.last_tick = tick;
                chain.pressed |= pressed;
                chain.sources |= 1 << (source as u8);
                return;
            }
            Some(_) => self.close_keeper_knock(KnockEnd::Lapsed),
            None => {}
        }
        self.knock_chain = Some(KnockChain::opened(
            keeper,
            tick,
            self.position,
            pressed,
            source,
        ));
    }

    /// Close the open chain, if there is one, and book it.
    pub fn close_keeper_knock(&mut self, end: KnockEnd) {
        let Some(chain) = self.knock_chain.take() else {
            return;
        };
        // Which line it crossed, for an out-of-play ending. Read off the
        // ball where it came to rest: outside the goal lines on the long
        // axis is a corner or a goal kick, anything else is a throw-in.
        let behind = (end == KnockEnd::OutOfPlay)
            .then(|| self.position.x <= 0.0 || self.position.x >= self.field_width);
        KnockDiag::close(
            &chain,
            end,
            (self.position - chain.from).magnitude(),
            self.current_tick_cached.saturating_sub(chain.opened_tick),
            behind,
        );
    }

    /// **Has the open chain ended?** — asked once a tick, at the top of
    /// [`Ball::update`].
    ///
    /// One site rather than a close at every release, because the endings
    /// are all statements about the ball rather than about the keeper: he
    /// has it, somebody else has it, it is off the pitch, or it is lying
    /// there and nobody has come. Reading them here is what makes the
    /// census complete by construction — a route into any of those four
    /// that nobody thought of is still seen.
    pub fn settle_keeper_knock(&mut self, players: &[MatchPlayer]) {
        self.sweep_keeper_touch(players);
        let Some(chain) = self.knock_chain.as_ref() else {
            return;
        };
        let keeper = chain.keeper;
        let stale =
            self.current_tick_cached.saturating_sub(chain.last_tick) > KnockChain::LINK_TICKS;

        if self.awaiting_restart.is_some() || self.in_net.is_some() {
            self.close_keeper_knock(KnockEnd::OutOfPlay);
            return;
        }
        if let Some(owner) = self.current_owner {
            let end = if owner == keeper {
                if self.held_in_hands {
                    KnockEnd::Gloves
                } else {
                    KnockEnd::Feet
                }
            } else {
                let same_side = players
                    .iter()
                    .find(|p| p.id == owner)
                    .zip(players.iter().find(|p| p.id == keeper))
                    .map(|(o, k)| o.team_id == k.team_id)
                    .unwrap_or(false);
                if same_side {
                    KnockEnd::Cleared
                } else {
                    KnockEnd::ToOpponent
                }
            };
            self.close_keeper_knock(end);
            return;
        }
        if stale {
            self.close_keeper_knock(KnockEnd::Lapsed);
        }
    }

    /// **The catch-all.** Every uncontrolled touch by a goalkeeper that the
    /// named sites did not book.
    ///
    /// Read off the ball's own `last_touch_*` bookkeeping, which every
    /// route to the ball already goes through, so this covers mechanisms
    /// nobody enumerated. Two exclusions, both exact rather than
    /// heuristic:
    ///
    /// * a touch the same tick as the open chain's last one has already
    ///   been booked by its own site;
    /// * a touch on the tick he RELEASED the ball is him putting it into
    ///   play, which is the opposite of a knock.
    fn sweep_keeper_touch(&mut self, players: &[MatchPlayer]) {
        if self.last_touch_tick == self.knock_seen_touch {
            return;
        }
        self.knock_seen_touch = self.last_touch_tick;
        if self.last_touch_was_controlled {
            return;
        }
        let Some(toucher) = self.last_touch_player_id else {
            return;
        };
        if self.last_release_player_id == Some(toucher)
            && self.last_release_tick == self.last_touch_tick
        {
            return;
        }
        if self
            .knock_chain
            .as_ref()
            .is_some_and(|c| c.keeper == toucher && c.last_tick == self.last_touch_tick)
        {
            return;
        }
        let Some(keeper) = players.iter().find(|p| {
            p.id == toucher
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
        }) else {
            return;
        };
        // Cloned out of the slice because the note needs `&mut self` and
        // the whole slice at once, and `MatchPlayer` is not `Copy`. One
        // shallow read of an id, a position and a team — the note itself
        // is the only thing that uses it.
        let (id, position, team_id) = (keeper.id, keeper.position, keeper.team_id);
        let pressed = players.iter().any(|p| {
            p.team_id != team_id
                && !p.is_sent_off
                && (p.position.x - position.x).hypot(p.position.y - position.y)
                    <= KnockChain::PRESSED
        });
        self.extend_keeper_knock(id, KnockSource::Other, pressed);
    }
}
