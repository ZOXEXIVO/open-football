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
    /// Match clock (ms) play resumes at IF nobody has finished walking by
    /// then — see [`Self::BREAK_MS`], which is a ceiling.
    resume_at_ms: u64,
    changes: Vec<Changeover>,
}

impl SubstitutionBreak {
    /// **How long a substitution stops the match for, in ms of match clock.**
    ///
    /// A televised change takes twenty to forty seconds; this is the part of
    /// it worth drawing — the man coming off reaching the line and the man
    /// coming on reaching his slot — with nothing padded either side.
    ///
    /// It is also the number every speed below is sized against, and the two
    /// legs run END TO END now that the man coming off has to reach the man
    /// coming on. Worst case: a corner-flag full-back to the halfway line
    /// (86 m at [`Self::OFF`], 9.8 s) and then the gate to a far-corner slot
    /// (62 m at [`Self::ON`], 7.5 s) — 17.3 s, with the ceiling two and a
    /// half seconds clear of it. That margin is what keeps the tidy-up at the
    /// end from being a teleport.
    ///
    /// **The longest a substitution may stop the match for, in ms of match
    /// clock — a ceiling, not a length.**
    ///
    /// The window closes on the tick the last man reaches his slot, so what a
    /// change actually costs is the two runs: the man coming off reaching the
    /// nearest point on the bench touchline, and only THEN the man coming on
    /// covering the ground to the slot he is inheriting. Typically ~34 m at
    /// [`Self::OFF`] plus ~35 m at [`Self::ON`] — nine or ten seconds. The
    /// worst case is a full-back on the far touchline (68 m across) followed
    /// by a run to the far corner (~55 m), which is seventeen; twenty leaves
    /// margin without letting a failed walk stall the match.
    ///
    /// A fixed window would have to be as long as the worst case and would
    /// then stand everybody still through the ordinary one.
    ///
    /// ⚠ Whatever it comes to is **match clock that used to be football.**
    /// The engine has always credited a substitution with 30 s of stoppage
    /// time while consuming none of it; now it consumes the window.
    ///
    /// Per STOPPAGE, not per change — a double or triple change on the same
    /// whistle shares one window, which is the whole reason a window holds a
    /// list. Measured on real recordings, ten changes a match arrive in two
    /// to four stoppages, so the bill is well under a percent of the playing
    /// time — an order below the 45-75 s a goal celebration already costs,
    /// and in the same direction, since a match with the ball out of play has
    /// less football in it. The calibration harness cannot see it either way:
    /// `dev_match stats` builds squads with an empty bench, so the
    /// substitution pass returns before it ever reaches here. `dev_match
    /// bench` can — see [`MatchContext::sub_walk_off`].
    ///
    /// [`MatchContext::sub_walk_off`]: super::super::context::MatchContext::sub_walk_off
    pub const BREAK_MS: u64 = 20_000;

    /// **How long everybody stands still at the start of the window, per man
    /// being replaced**, in ms of match clock.
    ///
    /// ⚠ **The picture cannot show a man on the pitch who has already left
    /// it.** The replay opens a change by looking at the back of each man
    /// coming off, where he is standing, so his name can be read — see
    /// `ChangeoverShot::PORTRAIT_MS` in the viewer, which is this same figure.
    /// Without the hold he is running for the touchline at [`Self::OFF`] from
    /// the tick the board goes up, 8.75 m/s, and the second and third men of a
    /// triple change are over the line before the camera reaches them.
    ///
    /// A second on his back and then the camera comes round to his face, which
    /// is 3.4 s a man; and it stops the whole window rather than his own leg
    /// of it, because everybody else on the pitch is already standing still
    /// and a man jogging off behind a shot of somebody else is the one thing
    /// in frame that moves.
    ///
    /// ⚠ **The viewer's `ChangeoverShot::PORTRAIT_MS` is this same figure and
    /// the two have to agree** — this crate cannot depend on that one, so if
    /// either moves the other moves with it. Under-hold and the man walks out
    /// of his own close-up; over-hold and the picture is on the touchline
    /// while the pitch stands still.
    ///
    /// It is charged to [`Self::resume_at_ms`] rather than taken out of it, so
    /// a change with three men in it still gets the full walking allowance —
    /// and it is match clock, like the rest of the window. Two to four
    /// stoppages a match at three to ten seconds apiece, which is the same
    /// order as the 45-75 s a goal celebration already spends.
    pub const PORTRAIT_MS: u64 = 3_400;

    /// Movement speeds, in game units per tick — one unit is 12.5 cm and one
    /// tick is 10 ms, so 0.08 u/tick is 1 m/s and these are real speeds
    /// rather than tuned ones.
    ///
    /// A man being taken off jogs briskly: the referee is waiting on him and
    /// the manager is waving. 8.75 m/s covers the longest walk anybody can be
    /// given — a corner-flag full-back to the halfway line, 86 m — in under
    /// ten seconds, which is what leaves room inside [`Self::BREAK_MS`] for
    /// the second leg.
    const OFF: f32 = 0.70;
    /// And on: 8.25 m/s. He is the second half of a sequence rather than the
    /// first, so his ground and the other man's have to fit inside the
    /// ceiling together — worst case 86 m off then 62 m on, about eighteen
    /// seconds.
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

    /// Open a window at `now` with nothing in it yet.
    pub fn open(now: u64) -> Self {
        SubstitutionBreak {
            opened_at_ms: now,
            resume_at_ms: now + Self::BREAK_MS,
            changes: Vec::with_capacity(2),
        }
    }

    /// The latest play can resume. It usually resumes sooner — see
    /// [`Self::BREAK_MS`].
    pub fn resume_at_ms(&self) -> u64 {
        self.resume_at_ms
    }

    /// How long nobody moves at the start of the window: one
    /// [`Self::PORTRAIT_MS`] for every man being replaced.
    fn beats_ms(&self) -> u64 {
        self.changes.len() as u64 * Self::PORTRAIT_MS
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
        // Every man added to the window buys another [`Self::PORTRAIT_MS`] of
        // everybody standing still, so the ceiling has to move with him — the
        // walking allowance is `BREAK_MS` and the beats are on top of it.
        //
        // Recomputed here rather than in `open` because the window is opened
        // empty: the caller stages the pair straight afterwards and re-arms
        // `dead_ball_until_ms` off this same figure, which is also what
        // `close` compares against to pull the pause back.
        self.resume_at_ms = self.opened_at_ms + Self::BREAK_MS + self.beats_ms();
    }

    /// One tick of the window. `true` while it is still running.
    ///
    /// It ends when the last man is on, not when the clock says so:
    /// [`Self::resume_at_ms`] is a CEILING. A change where nobody had far to
    /// walk costs the match nine or ten seconds; one where a full-back on the
    /// far touchline has to cross the whole width costs nearly twice that,
    /// and a fixed window would have to be long enough for the second while
    /// standing everybody still through the first.
    pub fn advance(&mut self, field: &mut MatchField, context: &MatchContext) -> bool {
        if context.total_match_time >= self.resume_at_ms {
            return false;
        }
        // ⚠ **Nobody moves while the picture is on the men being replaced.**
        // The replay opens the change with a second on the back of each of
        // them, standing where he was when the board went up — see
        // [`Self::PORTRAIT_MS`]. He cannot be shown standing on the pitch and
        // be running off it at the same time.
        if context.total_match_time < self.opened_at_ms + self.beats_ms() {
            return true;
        }
        let mut done = true;
        for change in &self.changes {
            // **He does not move until the other man is off.** A referee
            // waits for the exchange, and without this gate the two runs are
            // simultaneous and unrelated — which is what it looked like when
            // it was watched, and the first thing reported back.
            let off = Self::walk_off(field, change);
            done &= off && Self::walk_on(field, change);
        }
        !done
    }

    /// The man leaving: across the pitch to the man replacing him, then along
    /// the touchline towards the dugout at a walk. `true` once he is over the
    /// line, which is also the moment the two of them pass each other.
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

    /// Put every man who came on exactly where an instant swap would have put
    /// him, and stop him.
    ///
    /// The distance this closes is the window's own error — how much ground a
    /// walker had left when the referee waved play on — and it is booked
    /// against the `substitution` row of the player-teleport census, which is
    /// where the whole-pitch write this replaces used to be booked. The row
    /// must read near zero; anything else is a walk that ran out of
    /// [`Self::BREAK_MS`] before it ran out of ground.
    fn land(&self, field: &mut MatchField) {
        for change in &self.changes {
            // A man still held here never got over the line before the
            // ceiling expired — `walk_off` releases everybody else the tick
            // they cross. Let him walk home from wherever he is rather than
            // stand there held forever.
            if let Some(stand) = field
                .departed
                .iter_mut()
                .find(|p| p.id == change.player_out_id)
                .and_then(|p| p.touchline.as_mut())
            {
                stand.held = false;
            }

            let Some(player) = field
                .players
                .iter_mut()
                .find(|p| p.id == change.player_in_id)
            else {
                continue;
            };
            #[cfg(feature = "match-logs")]
            {
                use crate::r#match::engine::ball::ball::teleport as tc;
                tc::PlayerTeleportCensus::note_firing(tc::PSITE_SUBSTITUTION);
                tc::PlayerTeleportCensus::note(
                    tc::PSITE_SUBSTITUTION,
                    player.position,
                    change.slot,
                );
            }
            player.position = change.slot;
            player.velocity = Vector3::zeros();
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
    // Capped at the window's own ceiling rather than at `BREAK_MS`, because
    // the beats at the top of it are on top of that — see
    // `SubstitutionBreak::PORTRAIT_MS`. The figure is what the replay sizes
    // its shot from, and the shot includes the beats.
    let spent_ms = context
        .total_match_time
        .saturating_sub(window.opened_at_ms)
        .min(window.resume_at_ms.saturating_sub(window.opened_at_ms));

    // **Play resumes the moment the last man is on**, not when the ceiling
    // says so — see [`SubstitutionBreak::BREAK_MS`]. The pause was armed for
    // the ceiling, so it has to be pulled back here, and only if it is still
    // this window's: a goal celebration owns the same field and must never
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
