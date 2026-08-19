//! Per-tick trace of the ball around a contact with the woodwork.
//!
//! # Why a dedicated trace and not the existing counters
//!
//! [`flight_diag`](super::flight_diag) answers "did any stage move the ball
//! further than its velocity allows", aggregated over a match. That is the
//! right shape for a census and the wrong shape for a woodwork report,
//! because what a viewer reports is a SEQUENCE — the ball comes off the
//! post, runs somewhere it should not, and reappears somewhere else — and a
//! sum over ninety minutes cannot show a sequence.
//!
//! So this records the ball itself: one sample a tick, a short window before
//! each frame contact and a long one after it, with the resolvers that fired
//! in between written in alongside. Read down a capture and the answer is in
//! it — a tick whose travel is larger than its own speed is a teleport, and
//! the note above it names what did it.
//!
//! Diagnostic infrastructure, compiled only under `match-logs` and inert
//! unless `OF_FRAME_TRACE` is set. Nothing in here may influence a simulated
//! value: every entry point takes copies and the store is touched once a
//! tick.

use nalgebra::Vector3;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Ticks of run-up kept before the contact.
const PRE: usize = 20;

/// Ticks followed afterwards. 4 s — long enough to cover the rebound, the
/// second ball, whatever restart it turns into, and the first touch after
/// that.
const POST: usize = 400;

/// Captures kept per process. The frame is struck perhaps once a match, so
/// this is several matches' worth.
const MAX_CAPTURES: usize = 40;

/// A relocation the ball's own velocity does not explain, in game units.
/// One unit is 12.5 cm — a tenth of that is below anything a viewer could
/// see, and above it something moved the ball.
const JUMP_TOLERANCE: f32 = 1.5;

/// A one-tick fall, in metres, past which the drop is not gravity. Gravity
/// is 9.81e-4 m/tick², and a ball entering the tick at rest cannot fall
/// more than that; anything near a metre was written, not integrated.
const DROP_TOLERANCE: f32 = 0.30;

/// The ball, as one tick left it.
#[derive(Clone, Copy)]
pub struct Sample {
    pub tick: u64,
    pub pos: Vector3<f32>,
    pub vel: Vector3<f32>,
    pub owner: Option<u32>,
    /// Position-group initial of the owner — `G`/`D`/`M`/`F`, or `-`.
    pub owner_role: char,
    pub in_net: bool,
    pub awaiting_restart: bool,
    pub held: bool,
}

/// One sample plus everything the engine reported during that tick.
struct Entry {
    sample: Sample,
    notes: Vec<String>,
}

/// A window around one contact.
struct Capture {
    header: String,
    entries: Vec<Entry>,
    remaining: usize,
}

#[derive(Default)]
struct Store {
    ring: VecDeque<Sample>,
    pending: Vec<String>,
    open: Vec<Capture>,
    done: Vec<Capture>,
    /// Contacts seen, whether or not they were captured.
    hits: u64,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// What the captures show, counted.
#[derive(Default, Clone, Copy)]
pub struct Summary {
    /// Contacts with the woodwork, whether or not they were captured.
    pub hits: u64,
    /// Relocations inside the goal that nothing claimed — the netting
    /// pulling the ball rather than the ball flying.
    pub mesh_jumps: u64,
    /// The same, on a loose ball in open play.
    pub loose_jumps: u64,
    /// The worst of them, in centimetres.
    pub worst_jump_cm: u64,
    /// One-tick falls too big to be gravity: the ball put on the floor.
    pub ground_snaps: u64,
    /// The worst of them, in centimetres.
    pub worst_drop_cm: u64,
    /// Captures whose window ran out with the ball still in the goal.
    pub rested_in_net: u64,
}

/// Accessors. Grouped on a struct so the module exposes no free functions.
pub struct FrameTrace;

impl FrameTrace {
    /// Whether the trace is armed. Off unless `OF_FRAME_TRACE` is set, so a
    /// `match-logs` build measuring something else pays one cached read a
    /// tick and nothing more.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| std::env::var("OF_FRAME_TRACE").is_ok())
    }

    /// Whether a goal opens a capture too (`OF_FRAME_TRACE=net`).
    ///
    /// What happens INSIDE the goal is most of what a viewer sees after a
    /// shot off the woodwork goes in, and the frame is struck under once a
    /// match — far too rare a trigger to measure the netting with. Every
    /// goal is the same passage of play from the netting's point of view.
    pub fn captures_goals() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_FRAME_TRACE")
                .map(|v| v.contains("net"))
                .unwrap_or(false)
        })
    }

    fn with<R>(f: impl FnOnce(&mut Store) -> R) -> Option<R> {
        if !Self::armed() {
            return None;
        }
        let mut guard = STORE.lock().ok()?;
        Some(f(guard.get_or_insert_with(Store::default)))
    }

    /// Drop everything and start again.
    pub fn reset() {
        Self::with(|store| *store = Store::default());
    }

    /// Record the ball as the previous tick left it, and flush the notes
    /// raised during that tick alongside it.
    pub fn note_tick(sample: Sample) {
        Self::with(|store| {
            let notes = std::mem::take(&mut store.pending);
            let mut finished = Vec::new();
            for index in (0..store.open.len()).rev() {
                let capture = &mut store.open[index];
                capture.entries.push(Entry {
                    sample,
                    notes: notes.clone(),
                });
                capture.remaining = capture.remaining.saturating_sub(1);
                if capture.remaining == 0 {
                    finished.push(store.open.remove(index));
                }
            }
            store.done.extend(finished);
            if store.ring.len() >= PRE {
                store.ring.pop_front();
            }
            store.ring.push_back(sample);
        });
    }

    /// Note something the engine did during the tick now in progress.
    pub fn note(text: impl Into<String>) {
        Self::with(|store| {
            // With no capture open the notes are only the run-up to one that
            // may never come; bound them so a ninety-minute match with no
            // woodwork cannot accumulate a match's worth of strings.
            if store.open.is_empty() && store.pending.len() > 16 {
                store.pending.clear();
            }
            store.pending.push(text.into());
        });
    }

    /// Count a contact with the woodwork. Separate from [`Self::open`]
    /// because a goal opens a capture too, and the contact rate has to stay
    /// a rate of contacts.
    pub fn note_hit() {
        Self::with(|store| store.hits += 1);
    }

    /// Open a capture on whatever is happening now.
    pub fn open(header: String) {
        Self::with(|store| {
            store.pending.push(header.clone());
            if store.done.len() + store.open.len() >= MAX_CAPTURES {
                return;
            }
            let entries = store
                .ring
                .iter()
                .map(|sample| Entry {
                    sample: *sample,
                    notes: Vec::new(),
                })
                .collect();
            store.open.push(Capture {
                header,
                entries,
                remaining: POST,
            });
        });
    }

    /// What the captures show, counted. Read this before reading the
    /// tables: it says whether the ball is being MOVED by something other
    /// than its own flight, and the tables say by what.
    pub fn summary() -> Summary {
        Self::with(|store| {
            let mut summary = Summary {
                hits: store.hits,
                ..Summary::default()
            };
            for capture in store.done.iter().chain(store.open.iter()) {
                Self::tally(capture, &mut summary);
            }
            summary
        })
        .unwrap_or_default()
    }

    /// Every capture recorded so far, rendered, and the total contact count.
    /// Renders still-open windows too, so a trace cut short by the final
    /// whistle is reported rather than lost.
    pub fn report() -> (u64, Vec<String>) {
        Self::with(|store| {
            let rendered = store
                .done
                .iter()
                .chain(store.open.iter())
                .map(Self::render)
                .collect();
            (store.hits, rendered)
        })
        .unwrap_or((0, Vec::new()))
    }

    /// Count one capture's anomalies into `summary`.
    ///
    /// A tick that carries a NOTE is exempt from the jump test: a restart
    /// moving the ball is that restart's job, and counting it here would
    /// bury the ones nobody owns.
    fn tally(capture: &Capture, summary: &mut Summary) {
        let mut previous: Option<Sample> = None;
        for entry in &capture.entries {
            let sample = entry.sample;
            if let Some(p) = previous {
                let gap = sample.tick.saturating_sub(p.tick).max(1) as f32;
                let speed = (p.vel.x * p.vel.x + p.vel.y * p.vel.y)
                    .sqrt()
                    .max((sample.vel.x * sample.vel.x + sample.vel.y * sample.vel.y).sqrt());
                let moved =
                    ((sample.pos.x - p.pos.x).powi(2) + (sample.pos.y - p.pos.y).powi(2)).sqrt();
                let explained = entry.notes.is_empty() && sample.owner.is_none();
                if explained && moved > speed * gap + JUMP_TOLERANCE {
                    if sample.in_net || p.in_net {
                        summary.mesh_jumps += 1;
                    } else {
                        summary.loose_jumps += 1;
                    }
                    let unexplained = moved - speed * gap;
                    summary.worst_jump_cm = summary.worst_jump_cm.max((unexplained * 12.5) as u64);
                }
                // A drop the ball's own vertical speed cannot account for.
                let fell = p.pos.z - sample.pos.z;
                if explained && fell > DROP_TOLERANCE && -p.vel.z * gap < fell * 0.5 {
                    summary.ground_snaps += 1;
                    summary.worst_drop_cm = summary.worst_drop_cm.max((fell * 100.0) as u64);
                }
            }
            previous = Some(sample);
        }
        if let Some(last) = capture.entries.last() {
            if last.sample.in_net {
                summary.rested_in_net += 1;
            }
        }
    }

    /// One capture as a table.
    ///
    /// `|v|` is the horizontal speed the ball carried out of the tick and `d`
    /// how far it actually travelled since the previous sample. The two agree
    /// on every tick of honest physics; a row marked `*` moved further than
    /// its own velocity can explain, which is a relocation somebody applied.
    fn render(capture: &Capture) -> String {
        let mut out = String::with_capacity(8192);
        out.push_str(&capture.header);
        out.push('\n');
        out.push_str(
            "      tick        x       y      z       vx      vy      vz     |v|      d  owner\n",
        );
        let mut previous: Option<Sample> = None;
        for entry in &capture.entries {
            let sample = entry.sample;
            let speed = (sample.vel.x * sample.vel.x + sample.vel.y * sample.vel.y).sqrt();
            let moved = previous
                .map(|p| {
                    ((sample.pos.x - p.pos.x).powi(2) + (sample.pos.y - p.pos.y).powi(2)).sqrt()
                })
                .unwrap_or(0.0);
            // Tick gaps are real — the engine skips the post-goal window —
            // so the allowance scales with the gap.
            let gap = previous
                .map(|p| sample.tick.saturating_sub(p.tick).max(1))
                .unwrap_or(1);
            let mark = if moved > speed * gap as f32 + 1.5 {
                '*'
            } else {
                ' '
            };
            let owner = match sample.owner {
                Some(id) => format!("{}{}", sample.owner_role, id % 1000),
                None => "-".to_string(),
            };
            out.push_str(&format!(
                "{mark} {:>8} {:>8.2} {:>7.2} {:>6.2}  {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>6.2}  {:<6}{}{}{}\n",
                sample.tick,
                sample.pos.x,
                sample.pos.y,
                sample.pos.z,
                sample.vel.x,
                sample.vel.y,
                sample.vel.z,
                speed,
                moved,
                owner,
                if sample.in_net { " NET" } else { "" },
                if sample.awaiting_restart { " WAIT" } else { "" },
                if sample.held { " HELD" } else { "" },
            ));
            for note in &entry.notes {
                out.push_str(&format!("               | {note}\n"));
            }
            previous = Some(sample);
        }
        out
    }
}
