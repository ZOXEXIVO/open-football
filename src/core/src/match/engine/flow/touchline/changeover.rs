//! The window a substitution is played out in: who is walking where, how
//! fast, and what is left to tidy up when the referee waves play on.
//!
//! # The swap still happens on the tick it is decided
//!
//! Worth stating outright, because the obvious design is the other one. The
//! roster change — `field.substitute_player`, the stat snapshot, the ledger
//! entry, the emergency keeper stationed in goal — all runs exactly when and
//! where it always did, inside `Substitutions::execute_substitution`. This
//! window only decides where the two bodies are DRAWN while it is open.
//!
//! Deferring the swap to the end of the window was tried first and is a trap:
//! `Substitutions::process_inner` loops on
//! `best_discretionary_pair_with_coach`, which reads the roster to decide
//! whether there is another change worth making. Leave the roster untouched
//! and it picks the same pair again, and again, up to the per-team cap — one
//! decision, five substitutions. Every candidate scan in that file has the
//! same shape. So the roster leads and the picture follows, which also means
//! nothing in the substitution layer had to learn that a change now takes
//! time.

use super::bench::Bench;
use crate::r#match::{MATCH_TIME_INCREMENT_MS, MatchContext, MatchField};
use nalgebra::Vector3;

/// One change, as the picture sees it: two men and the three points between
/// them.
///
/// Resolved once, when the board goes up, so the two runs are stable for the
/// whole window instead of being re-aimed every tick at a formation that is
/// standing still anyway.
#[derive(Debug, Clone, Copy)]
pub struct Changeover {
    /// The man who has just come off. He is in `field.departed` from the
    /// instant the swap is made, and is drawn from his
    /// [`TouchlineStand`](super::TouchlineStand).
    pub player_out_id: u32,
    /// The man who has just come on. He is one of `field.players` already —
    /// standing on his own bench spot, which is where this window picks him
    /// up.
    pub player_in_id: u32,
    /// Which side made it. Only used to count how many of his own team are
    /// already waiting at the gate, which is what spaces them out.
    is_home: bool,
    /// Where the two men meet: the fourth official's shoulder, which is
    /// where the substitute has been standing since the board went up.
    ///
    /// **Not the nearest point on the touchline**, which is what Law 3 has
    /// said since 2019 and what this did first. A man leaving by the nearest
    /// point leaves alone, forty metres from the man replacing him, and the
    /// exchange — the half of a substitution anybody watches — never happens
    /// in one place. He runs to the halfway line and they cross there, which
    /// is the older Law and the only version of it that is a picture.
    meet: Vector3<f32>,
    /// And the slot the man coming on is taking over — the outgoing player's
    /// formation spot, which is where an instant swap would have written him.
    /// He runs there in one leg from the fourth official's shoulder, where
    /// `MatchField::substitute_player` stood him.
    slot: Vector3<f32>,
}

/// **Play has stopped and two men are crossing the line.**
///
/// Modelled on [`GoalCelebration`](super::super::celebration::GoalCelebration) and
/// running inside the same
/// [`dead_ball_until_ms`](super::super::context::MatchContext::dead_ball_until_ms)
/// window: no ball physics, no AI, no events, no coach evaluations, and
/// nobody on the pitch moving a centimetre except the man who has just been
/// taken off.
pub struct SubstitutionBreak {
    /// Match clock (ms) the board went up.
    opened_at_ms: u64,
    /// Match clock (ms) play resumes at. Exactly the beats — see
    /// [`Self::beats_ms`], which is the whole length of a window.
    resume_at_ms: u64,
    changes: Vec<Changeover>,
}

impl SubstitutionBreak {
    /// **How long a man coming on stands still while the camera looks at
    /// him**, in ms of match clock — his close-up, before he moves a step.
    ///
    /// ⚠ **The picture cannot show a man standing at the fourth official's
    /// shoulder who is already running onto the pitch.** The replay opens
    /// each change by cutting to the substitute's face, coming round him to
    /// his back so the name across his shoulders can be read, and only THEN
    /// letting him go — see `ChangeoverShot::PORTRAIT_MS` in the viewer, which
    /// is this same figure. Without the hold he is away at [`Self::ON`] from
    /// the tick the board goes up and the shot is a close-up of the grass he
    /// was standing on.
    ///
    /// A second and a half on his face, the swing round him, and a second on
    /// his back: 3.4 s a man. It stops the whole window rather than his own
    /// leg of it, because everybody else on the pitch is already standing
    /// still and one man moving is the only thing in frame that would.
    ///
    /// ⚠ **The viewer's `ChangeoverShot::PORTRAIT_MS` is this same figure and
    /// the two have to agree** — this crate cannot depend on that one, so if
    /// either moves the other moves with it. Under-hold and he walks out of
    /// his own close-up; over-hold and the picture is on a man who has already
    /// arrived while twenty-one others stand about.
    ///
    /// It is charged to [`Self::resume_at_ms`] rather than taken out of it, so
    /// a change with three men in it still gets the full walking allowance —
    /// and it is match clock, like the rest of the window. Two to four
    /// stoppages a match at six to eighteen seconds apiece, which is the same
    /// order as the 45-75 s a goal celebration already spends.
    pub const PORTRAIT_MS: u64 = 3_400;

    /// **And how long he then has the picture to himself while he runs on**,
    /// in ms of match clock.
    ///
    /// The camera has come round behind him and it stays exactly where it is:
    /// he runs away from it onto the field with his name across his shoulders
    /// and the whole ground laid out in front of him. Two seconds at
    /// [`Self::ON`] is sixteen metres, which reads as a man arriving.
    ///
    /// ⚠ **This is what makes a multiple change sequential.** The men are no
    /// longer released together the moment the last close-up is over — man two
    /// is still held while man one runs, so a triple change is three complete
    /// arrivals instead of one portrait each and then a scramble. The viewer's
    /// `ChangeoverShot::RUN_MS` is the same figure, and the pairing rule is
    /// [`Self::PORTRAIT_MS`]'s.
    ///
    /// ⚠⚠ **And since 2026-08-26 it is also the END of the window**, not just
    /// a beat inside it — see [`Self::advance`]. Two seconds of him running is
    /// what the change is worth; where he and the man he replaced have got to
    /// when it expires is the match's business, not the window's.
    pub const RUN_MS: u64 = 2_000;

    /// One man's whole turn in front of the camera: his close-up and then his
    /// run. The next man's close-up starts here.
    pub const BEAT_MS: u64 = Self::PORTRAIT_MS + Self::RUN_MS;

    /// Movement speeds, in game units per tick — one unit is 12.5 cm and one
    /// tick is 10 ms, so 0.08 u/tick is 1 m/s and these are real speeds
    /// rather than tuned ones.
    ///
    /// A man being taken off jogs briskly: the referee is waiting on him and
    /// the manager is waving. 8.75 m/s covers the longest walk anybody can be
    /// given — a corner-flag full-back to the halfway line, 86 m — in under
    /// ten seconds. Most of that now falls AFTER the window, with the match
    /// going on around him.
    ///
    /// Public because the window no longer walks him all the way to the line —
    /// it stops on its own beats and hands him to
    /// `MatchField::settle_touchline`, which carries on at this speed until he
    /// is over it. See [`Self::beats_ms`].
    pub const OFF: f32 = 0.70;
    /// And on: 8.25 m/s. He leaves on the same tick as the man he is
    /// replacing rather than after him, so the two runs overlap and the
    /// ceiling only has to hold the longer of them — but he is the one being
    /// watched, and [`Self::RUN_MS`] of camera is sized off this: 2.6 s puts
    /// him twenty-one metres onto the field.
    const ON: f32 = 0.66;
    /// The walk from the line back to the bench, once he is off.
    /// Deliberately the speed [`MatchField::settle_touchline`] carries on
    /// with after the window closes, so his pace does not step at the
    /// handover.
    pub const HOME: f32 = 0.18;

    /// Close enough to his slot to be standing on it.
    ///
    /// ⚠ **This tolerance is the size of the jump `land` has to close**, so it
    /// is not a comfort number: at a readable-looking 2 u the window ended
    /// with the man a quarter of a metre short and the tidy-up moved him
    /// there in one frame — 2.19 units in a tick against a 0.62 speed, caught
    /// by the step assertion in `substitution_break_tests`. 0.05 u is
    /// [`Self::step`]'s own dead zone, so a walk that reaches it has genuinely
    /// finished and `land` writes nothing.
    const ARRIVED: f32 = 0.05;

    /// Open a window at `now` with nothing in it yet — and so, until a change
    /// is staged into it, one that is already over.
    pub fn open(now: u64) -> Self {
        SubstitutionBreak {
            opened_at_ms: now,
            resume_at_ms: now,
            changes: Vec::with_capacity(2),
        }
    }

    /// When play resumes. Exactly, now — see [`Self::beats_ms`].
    pub fn resume_at_ms(&self) -> u64 {
        self.resume_at_ms
    }

    /// **How long the window lasts, in ms of match clock: one
    /// [`Self::BEAT_MS`] for every man coming on, and not a tick more.**
    ///
    /// ⚠ **It used to wait for the walking, and that is the 2026-08-26
    /// change** (maintainer: *"on subs — do not wait until replaced player run
    /// to field side; show 2 secs when new player run on field and stop"*).
    /// The old window ran until the last substitute was standing on his slot
    /// AND the last man coming off was over the touchline, under a
    /// twenty-second ceiling (`BREAK_MS`, now gone). Both of those are ground
    /// nobody is watching: the camera cuts away at the end of the last man's
    /// run, and everything after it was the match standing still for a walk
    /// that had already left the frame.
    ///
    /// So the window is a fixed, known length now — a single change costs 5.4
    /// s of match clock, a triple 16.2 — and where either walker has got to
    /// when it expires is the match's business rather than the window's:
    ///
    /// - the man coming ON is one of the eleven and keeps running to his slot
    ///   under his own AI once play resumes, which is what a substitute does;
    /// - the man coming OFF is handed to
    ///   `MatchField::settle_touchline`, which walks him the rest of the way
    ///   while the football goes on around him.
    ///
    /// ⚠ **And that is why [`Self::land`] no longer writes a position.** The
    /// old tidy-up put the substitute on his slot to close whatever ground a
    /// walk had left when the ceiling expired; against a window that stops
    /// two seconds into his run, the same write is a twenty-metre teleport.
    fn beats_ms(&self) -> u64 {
        self.changes.len() as u64 * Self::BEAT_MS
    }

    /// The match clock the `index`-th man of the window is let go at: the end
    /// of his own close-up, and not a tick before.
    ///
    /// His counterpart leaves on the same tick. The exchange is what a
    /// substitution is, and both of them standing still until the camera has
    /// finished with the man coming on is what keeps the close-up a picture of
    /// somebody standing rather than of somebody halfway out of frame.
    fn released_at(&self, index: usize) -> u64 {
        self.opened_at_ms + index as u64 * Self::BEAT_MS + Self::PORTRAIT_MS
    }

    /// The changes being played out. Both sides can change at once — that is
    /// one stoppage and one window, which is what a real double change is.
    pub fn changes(&self) -> &[Changeover] {
        &self.changes
    }

    /// How many of this side's substitutes are already waiting at the gate.
    /// [`Bench::entry_gate`] spaces the next one along from them.
    pub fn waiting_for(&self, is_home: bool) -> usize {
        self.changes.iter().filter(|c| c.is_home == is_home).count()
    }

    /// Add a swap that has *already been made* to the window, and say where
    /// its two men have to walk. `meet` is the gate the substitute is
    /// standing at — the same point `MatchField::substitute_player` put him
    /// on, so the man coming off runs to him rather than to somewhere near
    /// him.
    pub fn stage(
        &mut self,
        is_home: bool,
        player_out_id: u32,
        player_in_id: u32,
        meet: Vector3<f32>,
        slot: Vector3<f32>,
    ) {
        self.changes.push(Changeover {
            player_out_id,
            player_in_id,
            is_home,
            meet,
            slot,
        });
        // Every man added to the window buys another [`Self::BEAT_MS`] of the
        // camera's time, and the beats ARE the window — see
        // [`Self::beats_ms`].
        //
        // Recomputed here rather than in `open` because the window is opened
        // empty: the caller stages the pair straight afterwards and re-arms
        // `dead_ball_until_ms` off this same figure, which is also what
        // `close` compares against to pull the pause back.
        self.resume_at_ms = self.opened_at_ms + self.beats_ms();
    }

    /// One tick of the window. `true` while it is still running.
    ///
    /// ⚠ **It ends on the clock and on nothing else** — see
    /// [`Self::beats_ms`]. It used to end when the last substitute was
    /// standing on his slot AND the last man coming off was over the line,
    /// which meant the match stood still for a walk the camera had already
    /// cut away from.
    pub fn advance(&mut self, field: &mut MatchField, context: &MatchContext) -> bool {
        let now = context.total_match_time;
        if now >= self.resume_at_ms {
            return false;
        }
        for (index, change) in self.changes.iter().enumerate() {
            // ⚠ **Neither man of a pair moves while the picture is on the one
            // coming on.** The replay opens his beat with his face, comes
            // round him to the name across his shoulders, and only then lets
            // him go — see [`Self::PORTRAIT_MS`]. He cannot be shown standing
            // at the fourth official's shoulder and be running onto the pitch
            // at the same time.
            //
            // ⚠ **And it is per MAN, not per window.** Man two is still held
            // while man one runs on, which is what makes a triple change three
            // arrivals rather than one shot of three men setting off together.
            if now < self.released_at(index) {
                continue;
            }
            // The two of them go at once and cross at the gate, which is what
            // an exchange looks like. Neither of them is waited FOR: both of
            // these report whether they have arrived and neither answer is
            // read any more.
            Self::walk_on(field, change);
            Self::walk_off(field, change);
        }
        true
    }

    /// The man leaving: across the pitch to the man replacing him, then along
    /// the touchline towards the dugout at a walk. `true` once he is over the
    /// line, which is also the moment the two of them pass each other.
    ///
    /// **Nothing is watching him.** He sets off on the same tick as his
    /// replacement and runs to the same gate, so the crossing still happens in
    /// one place — but the camera is behind the man coming on, looking at the
    /// pitch, and this walk is background. It is here so a body does not
    /// vanish off the spot it was standing on, not because anybody is pointed
    /// at it.
    ///
    /// He moves on his [`TouchlineStand`](super::TouchlineStand) rather than
    /// on his `position`, because he is no longer one of the eleven and his
    /// `position` is the off-pitch sentinel every departed player is parked
    /// at — see that type's own note for why the two coordinates have to stay
    /// apart.
    fn walk_off(field: &mut MatchField, change: &Changeover) -> bool {
        let Some(player) = field
            .departed
            .iter_mut()
            .find(|p| p.id == change.player_out_id)
        else {
            // Nothing left to wait for — he has already walked home and been
            // dropped, or the fixture never gave him a stand.
            return true;
        };
        let Some(stand) = player.touchline.as_mut() else {
            return true;
        };
        if Bench::is_over_the_line(stand.at) {
            // **He is off, and the window's part in him is finished.**
            //
            // From the line to the dugout he belongs to
            // `MatchField::settle_touchline`, which walks him at the same
            // [`Self::HOME`] speed on the recorder's cadence. Releasing him
            // HERE rather than in `close` is what stops the two overlapping:
            // hold him to the end of the window and the frame it closes on
            // gets both steps, measured at 0.72 units in a tick against a
            // 0.18 speed. See `TouchlineStand::held`.
            stand.held = false;
            return true;
        }
        let mut ignored = Vector3::zeros();
        Self::step(&mut stand.at, &mut ignored, change.meet, Self::OFF);
        false
    }

    /// The man arriving: over the line at the fourth official's shoulder and
    /// out to the slot he is taking over. `true` once he is standing on it.
    ///
    /// He IS one of the eleven — the roster changed on the tick the decision
    /// was made — so he moves on his real `position`, which is safe for
    /// exactly as long as this window owns the tick: no proximity table is
    /// rebuilt, no state machine runs and no ball is in play while a man of
    /// theirs is standing in the run-off.
    fn walk_on(field: &mut MatchField, change: &Changeover) -> bool {
        let Some(player) = field
            .players
            .iter_mut()
            .find(|p| p.id == change.player_in_id)
        else {
            return true;
        };
        Self::step(
            &mut player.position,
            &mut player.velocity,
            change.slot,
            Self::ON,
        );
        let reach = change.slot - player.position;
        (reach.x * reach.x + reach.y * reach.y).sqrt() <= Self::ARRIVED
    }

    /// Move `position` toward `target` at `speed` units per tick, writing the
    /// step into `velocity` so anything that reads it sees a man moving
    /// rather than a man appearing.
    ///
    /// Unlike `GoalCelebration::steer` this does NOT hold the walker inside
    /// the touchline: crossing it is the whole point.
    fn step(
        position: &mut Vector3<f32>,
        velocity: &mut Vector3<f32>,
        target: Vector3<f32>,
        speed: f32,
    ) {
        let dx = target.x - position.x;
        let dy = target.y - position.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if speed <= 0.0 || distance < 0.05 {
            *velocity = Vector3::zeros();
            return;
        }
        let step = speed.min(distance);
        let (ux, uy) = (dx / distance, dy / distance);
        *velocity = Vector3::new(ux * step, uy * step, 0.0);
        position.x += ux * step;
        position.y += uy * step;
    }

    /// Hand both walkers back to the match.
    ///
    /// ⚠ **This used to PUT the man coming on exactly where an instant swap
    /// would have put him**, on the grounds that the window had run until he
    /// was all but standing there and the write was a few centimetres. Against
    /// a window that stops two seconds into his run — see [`Self::beats_ms`] —
    /// the same write is twenty metres of teleport, straight into the
    /// `substitution` row of the player-teleport census that was watching for
    /// exactly this.
    ///
    /// So nothing is landed. He is one of the eleven with a real position and
    /// a real velocity, and the moment play resumes his own AI carries him the
    /// rest of the way to his slot, which is what a substitute jogging into
    /// position looks like. The only thing this still does is let go of the
    /// man walking off.
    fn land(&self, field: &mut MatchField) {
        for change in &self.changes {
            // A man still held here has not reached the touchline —
            // `walk_off` releases everybody else the tick they cross. Let him
            // walk home from wherever he has got to rather than stand there
            // held forever; `MatchField::settle_touchline` takes him on, and
            // keeps his brisk pace until he is over the line.
            if let Some(stand) = field
                .departed
                .iter_mut()
                .find(|p| p.id == change.player_out_id)
                .and_then(|p| p.touchline.as_mut())
            {
                stand.held = false;
            }
        }
    }
}

/// One tick of the substitution window, or `false` when there is none.
///
/// Mirrors [`advance_goal_celebration`](super::super::arena::goal::advance_goal_celebration)
/// down to the take/put-back, and for the same reason: while it is open the
/// window owns the tick, and the caller only needs to know whether it still
/// does.
pub fn advance_substitution_break(field: &mut MatchField, context: &mut MatchContext) -> bool {
    let Some(mut window) = context.substitution_break.take() else {
        return false;
    };
    if window.advance(field, context) {
        context.substitution_break = Some(window);
        return true;
    }
    close(field, context, window);
    true
}

/// Force any pending window closed immediately.
///
/// The whistle for half time can go while two men are still walking. Play
/// must never resume — nor a period end — with a substitution half drawn, and
/// a period boundary re-forms both sides anyway, so whatever ground the
/// walkers had left is ground nobody was going to see.
pub fn finish_substitution_break(field: &mut MatchField, context: &mut MatchContext) {
    if let Some(window) = context.substitution_break.take() {
        close(field, context, window);
    }
}

/// Land the walkers, wave play on, and hand the waiting restart back the
/// patience the window spent for it.
fn close(field: &mut MatchField, context: &mut MatchContext, window: SubstitutionBreak) {
    window.land(field);
    // What the change actually cost, which is the figure the replay sizes its
    // shot from. Capped at the window's own length: a half-time whistle can
    // close a window early through `finish_substitution_break`, and a shot
    // held for longer than the footage behind it is a camera pointed at a
    // pitch nobody is standing on.
    let spent_ms = context
        .total_match_time
        .saturating_sub(window.opened_at_ms)
        .min(window.resume_at_ms.saturating_sub(window.opened_at_ms));

    // **Play resumes when the beats are done.** Ordinarily that is exactly
    // when `dead_ball_until_ms` was armed for and this changes nothing; it is
    // here for the early close above. Only if the pause is still this
    // window's, though: a goal celebration owns the same field and must never
    // have its own window cut short by a substitution's tidy-up. The two
    // cannot overlap, and this is what keeps that true even if they ever do.
    if context.dead_ball_until_ms == window.resume_at_ms {
        context.dead_ball_until_ms = context.total_match_time;
    }

    // **How long it actually took**, stamped on the ledger so the replay can
    // hold its shot for exactly as long as the change lasted rather than
    // guessing at a constant. Written here because it is not knowable when
    // the swap is recorded — the window had not been walked yet.
    for change in window.changes() {
        if let Some(record) = context
            .substitutions
            .iter_mut()
            .find(|r| r.player_in_id == change.player_in_id)
        {
            record.break_ms = spent_ms;
        }
    }

    // ⚠ **The restart the substitution interrupted has been waiting all this
    // time.**
    //
    // `AwaitedRestart` bounds how long it holds for its taker
    // (`patience_ticks`, counted from `awarded_tick`) and falls back to
    // teleporting him onto the ball when the bound expires — the one teleport
    // the whole walked-restart mechanism exists to remove. Ten seconds is
    // 1 000 ticks, which blows every patience in that file, so the ticks the
    // substitution spent are given back rather than charged to a man who was
    // standing still under orders.
    let spent = spent_ms / MATCH_TIME_INCREMENT_MS;
    if let Some(restart) = field.ball.awaiting_restart.as_mut() {
        restart.awarded_tick = restart.awarded_tick.saturating_add(spent);
        if let Some(settled) = restart.settled_tick.as_mut() {
            *settled = settled.saturating_add(spent);
        }
    }
}
