//! **When the board goes up.**
//!
//! The rest of the substitution layer decides *who* comes off and *who*
//! replaces him. This module decides the other half — the half a spectator
//! actually notices — which is *when* a manager reaches for his bench at all.
//!
//! # What was wrong
//!
//! Timing used to be three fixed clocks stacked on each other: a per-slot
//! window (`55 / 65 / 75 / 85`) that no situation could move, a period timer
//! drawn once at the interval, and an urgency ramp keyed to the minute. None
//! of the three could see the match. Measured over 400 fixtures, the first
//! change of a side landed at 71.9' with a standard deviation of **4.2
//! minutes**, and — the part that gives it away — a team that finished the
//! game *behind* made that change at 71.9', a team that finished *ahead* at
//! 72.0'. The scoreline was worth a tenth of a minute. Eighty-eight percent
//! of sides used all five changes, and the fourth and fifth were both piled
//! onto 85-89' because that is where the last band opened.
//!
//! # The model
//!
//! One continuous quantity — [`BenchPressure::total`] — standing for how
//! badly this bench wants using *right now*. It is a sum of terms, each of
//! which is a smooth curve rather than a threshold, so nothing in it can
//! produce a cliff at a round minute:
//!
//! | term | what it reads | worth, at most |
//! |---|---|---|
//! | [`clock`](BenchPressure::clock) | the match running out | 1.00 |
//! | [`chase`](BenchPressure::chase) | goals behind × how little time is left | 0.42 |
//! | [`close_out`](BenchPressure::close_out) | a lead worth protecting, late | 0.30 |
//! | [`legs`](BenchPressure::legs) | the worst-drained starters, and how many | 0.35 |
//! | [`trouble`](BenchPressure::trouble) | a booking, an error, a collapsed game | 0.30 |
//! | [`temperament`](BenchPressure::temperament) | this manager, this afternoon | ±0.17 |
//!
//! Against it stands a bar, and what the bar prices is not the situation but
//! the **opportunity** — see [`ChangeOpportunity`]. Each change already made
//! raises it; the interval lowers it; a stoppage a team-mate has already
//! interrupted lowers it a lot; and a window or a change still in hand
//! raises it, but only while there is match left to need it in. Between them
//! the changes space themselves out, cluster into two or three real moments
//! the way they do on a Saturday, and leave the last one for the closing
//! minutes.
//!
//! The clock is deliberately the largest single term: most substitutions in
//! football really are made because the game is running out. What the other
//! terms buy is *displacement* — the ramp is worth about 0.013 of pressure
//! per minute, so a term worth 0.25 moves a change nineteen minutes earlier
//! than the same side would have made it in a quiet, level game. That is the
//! whole point: two managers in the same fixture, one chasing and one
//! protecting, should not reach for the bench in the same minute, and a
//! manager who is chasing in one match and protecting in the next should not
//! reach for it in the same minute either.
//!
//! # What it measures
//!
//! Same census, same 400 fixtures, `dev_match subs 400 14`:
//!
//! | | before | after | real |
//! |---|---|---|---|
//! | first change of a side, mean | 71.9' | 59.8' | ~62' |
//! | first change, **standard deviation** | **4.2'** | **10.4'** | ~13' |
//! | distinct minutes it lands on | 25 | 46 | — |
//! | first change: behind vs ahead | 71.9' / 72.0' | **54.7' / 64.8'** | large |
//! | changes at the interval | none possible | 7% | 10-14% |
//! | separate moments a side interrupts | 5 | 2-3 | 2-3 |
//! | changes per side | 4.83 | 4.63 | ~4.5 |
//!
//! # Temperament, and why it is not a die roll
//!
//! [`temperament`](BenchPressure::temperament) is derived, not drawn: from
//! the coach's own `risk_tolerance` / `conservatism` where a real manager is
//! attached to the side, and otherwise from a hash of the match seed and the
//! team. Both are *stable for the whole match* — a decisive manager is
//! decisive at 50' and still decisive at 80', which is what makes him
//! recognisable — and neither touches
//! `MatchContext::rng`, so adding this
//! model does not move a single roll in a fixture that has no bench to use.

use crate::club::staff::CoachProfile;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::sub_scoring::LiveSubstitutionStats;
use crate::PlayerPositionType;

/// One side's appetite for a change, at one moment.
///
/// Built by [`SubstitutionUrgency::read`]; the terms are kept separately
/// rather than pre-summed so a census can attribute a change to the thing
/// that caused it, and so a test can pin one curve without standing up the
/// other five.
#[derive(Debug, Clone, Copy)]
pub struct BenchPressure {
    /// The match running out. Zero until the fifteenth minute, one from the
    /// ninety-second — the only term that reaches the full unit on its own,
    /// and the reason a quiet, level, uninjured game still empties most of
    /// its bench by the end.
    pub clock: f32,
    /// Goals behind, weighted by how little time is left to retrieve them.
    /// The term the old model had no equivalent of at all.
    pub chase: f32,
    /// A lead big enough to want killing, late enough to want it killed.
    /// Fresh legs to run the corners, and a rest for whoever is on a
    /// booking.
    pub close_out: f32,
    /// How far into the tank the worst-off starters are, and how many of
    /// them are there.
    pub legs: f32,
    /// The single most alarming thing happening to a man on the pitch: a
    /// booking he might turn into a red, an error that cost a goal, a game
    /// that has fallen apart under him.
    pub trouble: f32,
    /// This manager, this afternoon. Signed: negative for a coach who sits
    /// on his hands, positive for one who acts. Worth roughly ±13 minutes.
    pub temperament: f32,
    /// The sum, floored at zero. Compared against [`Self::bar`].
    pub total: f32,
}

/// What it would cost this side to make a change *at this moment* —
/// everything about the opportunity rather than about the situation.
///
/// The distinction matters because the two move independently: a side can
/// want a change badly and still not make it (its last window is precious
/// and there is half an hour left), and a side can want one barely and make
/// it anyway (a team-mate is already walking off, so the interruption is
/// paid for).
#[derive(Debug, Clone, Copy, Default)]
pub struct ChangeOpportunity {
    /// The interval, where a change costs nothing at all.
    pub at_half_time: bool,
    /// A team-mate is already crossing the line on this stoppage. The
    /// second man costs nothing the first has not already paid.
    pub already_open: bool,
    /// Stoppages this side has already spent, out of the three the Law
    /// allows it.
    pub windows_spent: u8,
    /// Changes this side still has in hand. A manager down to his last one
    /// holds it back while there is still match left to need it in — for
    /// an injury, for a man on a booking, for the last twenty minutes — and
    /// spends it freely once there is not.
    pub changes_left: u8,
    /// Share of the match still to be played, from the interval on: 1 at
    /// half-time, 0 at the whistle. What makes an unspent window precious
    /// early and worthless late.
    pub match_left: f32,
    /// How much of the interval's cheapness is still in the air. One at
    /// half-time itself, decaying through the opening of the second half.
    ///
    /// A manager who spent the interval deciding that his left winger has
    /// to come off does not un-decide it because the whistle went; he waits
    /// for the first throw-in and makes the change. Without this the whole
    /// 46-60' band emptied out — every change the interval could take, it
    /// took, and nothing was left to happen until the ramp had climbed
    /// another quarter of an hour.
    pub interval_lingers: f32,
}

impl ChangeOpportunity {
    /// Build from the match clock and the side's ledgers.
    pub fn new(
        minute: u32,
        at_half_time: bool,
        already_open: bool,
        windows_spent: u8,
        changes_left: u8,
    ) -> Self {
        // Only *after* the interval, never before it. A first half has no
        // interval behind it to be cheap from — and a manager who can see
        // the break coming holds the change FOR it rather than making it on
        // 42', which is the opposite of a discount.
        let since_restart = minute as f32 - 45.0;
        ChangeOpportunity {
            at_half_time,
            already_open,
            windows_spent,
            changes_left,
            match_left: ((90.0 - minute as f32) / 45.0).clamp(0.0, 1.0),
            interval_lingers: if at_half_time {
                1.0
            } else if since_restart < 0.0 {
                0.0
            } else {
                BenchPressure::INTERVAL_CARRY
                    * (-since_restart / BenchPressure::INTERVAL_DECAY_MINUTES).exp()
            },
        }
    }
}

impl BenchPressure {
    /// Pressure the *n*-th change of a side has to clear, counting from
    /// zero, in the absence of anything special about the moment.
    ///
    /// The first change is the cheap one. Each after it costs more, which is
    /// what spaces a side's changes out across the match without anybody
    /// scheduling them, and what leaves the fifth needing a reason beyond
    /// the clock: the ramp alone tops out at 1.0, so a side that is neither
    /// chasing, protecting, tired nor in trouble finishes with a change or
    /// two unused — which is how a comfortable afternoon really goes, and
    /// which the old fixed windows could not express.
    ///
    /// Kept deliberately close together. The gap between consecutive bars is
    /// about six and a half minutes of ramp, so a double change is a normal
    /// event whenever anything else is pushing, and three at once is what a
    /// side two down at the hour looks like.
    pub fn bar(slot: u8) -> f32 {
        Self::FIRST_BAR + Self::SLOT_STEP * slot as f32
    }

    /// The bar as this particular moment prices it.
    ///
    /// Three adjustments, and each is a real cost a manager weighs:
    ///
    /// * **The interval is free.** No stoppage to cause, no walk to make,
    ///   none of his three windows, and the whole squad in front of him. So
    ///   the base drops.
    /// * **A window already open is free.** The interruption has been paid
    ///   for by the first man; the second and third cross the line on the
    ///   same whistle. This is what a double change *is*, and without it a
    ///   side that gets three windows makes three changes — which measured
    ///   3.39 changes a side against a real 4.3.
    /// * **An unspent window is worth hoarding, and only while there is
    ///   match left to need it in.** A manager with one window left on 55'
    ///   sits on it; the same manager with one left on 85' spends it,
    ///   because a window carried past the whistle is worth nothing. The
    ///   `match_left` factor is what turns the same cost from prohibitive
    ///   to free across the last half-hour, and it is what puts changes
    ///   back into the closing minutes.
    ///
    /// The same reasoning, applied to the changes themselves rather than to
    /// the stoppages, is [`Self::LAST_CHANGE_HOARD`]: a manager keeps one
    /// back for an injury, and stops keeping it back once there is nothing
    /// left to keep it for. Between them these two are the whole of why a
    /// side's fifth change lands at 85' rather than at 74' — which is where
    /// it landed while the only thing spacing changes out was a fixed
    /// ladder.
    pub fn bar_at(slot: u8, opportunity: &ChangeOpportunity) -> f32 {
        let base = Self::FIRST_BAR
            - (Self::FIRST_BAR - Self::HALF_TIME_BAR) * opportunity.interval_lingers;
        let mut bar = base + Self::SLOT_STEP * slot as f32;
        if opportunity.already_open {
            bar -= Self::BUNDLE_DISCOUNT;
        } else if !opportunity.at_half_time {
            let scarcity = (opportunity.windows_spent as f32 / Self::WINDOWS_PER_TEAM).min(1.0);
            bar += Self::WINDOW_COST * scarcity * opportunity.match_left;
        }
        // What is still in hand, and how much match is left to need it in.
        // Down to one change with half an hour to play, a manager sits on
        // it; down to one with five minutes to play, he has nothing to sit
        // on it for.
        let held = 1.0 / opportunity.changes_left.max(1) as f32;
        bar += Self::LAST_CHANGE_HOARD * held * opportunity.match_left;
        bar
    }

    /// True when this side would make its `slot`-th change now.
    #[inline]
    pub fn clears(&self, slot: u8) -> bool {
        self.total >= Self::bar(slot)
    }

    /// True when this side would make its `slot`-th change at this moment,
    /// priced.
    #[inline]
    pub fn clears_at(&self, slot: u8, opportunity: &ChangeOpportunity) -> bool {
        self.total >= Self::bar_at(slot, opportunity)
    }

    /// How much of the bar is already covered, for the *next* change — a
    /// 0..1 reading used to relax the pair-scoring threshold rather than to
    /// gate anything.
    ///
    /// A manager who badly wants a change is not only earlier, he is less
    /// fussy about which change it is: a side three down at 80' will send on
    /// a striker who does not really fit the shape, and the same side at 1-1
    /// on the hour would not. Returns 0 when the bar is out of reach and
    /// saturates once it is comfortably cleared.
    pub fn conviction(&self, slot: u8) -> f32 {
        self.conviction_against(Self::bar(slot))
    }

    /// [`Self::conviction`], against the bar as this moment prices it.
    pub fn conviction_at(&self, slot: u8, opportunity: &ChangeOpportunity) -> f32 {
        self.conviction_against(Self::bar_at(slot, opportunity))
    }

    fn conviction_against(&self, bar: f32) -> f32 {
        ((self.total - bar + Self::CONVICTION_SPAN) / (2.0 * Self::CONVICTION_SPAN)).clamp(0.0, 1.0)
    }

    /// [`Self::close_out`] as a 0..1 share of everything it could be — how
    /// far into "this game is won, start managing it" the side is.
    ///
    /// Kept separate from [`Self::conviction`] because the two answer
    /// different questions and only this one may touch star protection: a
    /// side can want a change desperately (chasing) without being at all
    /// willing to take off the man who is winning it for them.
    #[inline]
    pub fn close_out_share(&self) -> f32 {
        (self.close_out / SubstitutionUrgency::CLOSE_OUT_GAIN).clamp(0.0, 1.0)
    }

    /// Pressure the first change has to clear.
    ///
    /// Sited so that a level, uninjured, unremarkable game reaches it around
    /// the hour: the ramp is at 0.67 on 60', the drained-legs term is worth
    /// a few hundredths by then, and temperament swings it either side. That
    /// is the median first substitution in the real game, and everything
    /// interesting is displacement from it.
    const FIRST_BAR: f32 = 0.72;

    /// Added to the bar for each change already made.
    const SLOT_STEP: f32 = 0.11;

    /// Taken off the bar for a man crossing the line on a stoppage a
    /// team-mate has already interrupted. Large, because the honest cost of
    /// a second change in the same window really is close to nothing.
    const BUNDLE_DISCOUNT: f32 = 0.13;

    /// What an unspent window is worth to a manager who still has match left
    /// to need it in — at most, with every window gone and the whole second
    /// half to play.
    const WINDOW_COST: f32 = 0.55;

    /// How much of the interval's discount survives the restart whistle.
    /// Not all of it — the change really is cheaper at the break than five
    /// minutes into the half — but most, because the decision was already
    /// taken and only the throw-in is being waited for.
    const INTERVAL_CARRY: f32 = 0.60;

    /// Minutes over which what is left of that decays. An e-fold every six
    /// or seven minutes puts it at a third by 53' and at nothing by the
    /// hour, which is where the ordinary ramp has taken over anyway.
    const INTERVAL_DECAY_MINUTES: f32 = 10.0;

    /// What holding the *last* change in reserve is worth to a manager with
    /// the whole second half still to play. Scales down with each further
    /// change he has in hand — five spare is not a reserve, it is a bench —
    /// and with the match remaining, so it disappears entirely by the
    /// closing minutes.
    const LAST_CHANGE_HOARD: f32 = 0.28;

    /// Stoppages a side may interrupt over normal time, mirroring
    /// [`SubstitutionWindows::PER_TEAM`]. Held as a float because it is only
    /// ever used as a denominator.
    ///
    /// [`SubstitutionWindows::PER_TEAM`]: crate::r#match::SubstitutionWindows::PER_TEAM
    const WINDOWS_PER_TEAM: f32 = 3.0;

    /// Half-width of the band over which [`Self::conviction`] climbs from
    /// nothing to everything.
    const CONVICTION_SPAN: f32 = 0.25;

    /// The bar at half-time, which is its own kind of moment.
    ///
    /// A change at the interval costs a manager nothing — no walk, no
    /// stoppage, no window off his three, and a full dressing room to
    /// explain it in — so he makes ones he would not make on 50'. It is the
    /// single most common substitution minute in real football and the old
    /// model could not produce it at all, discretionary changes being gated
    /// to the second half.
    ///
    /// Sited just above what a *featureless* first half hands a side — the
    /// ramp is at 0.42 on 45' and temperament swings ±0.13 — so what
    /// reaches it is a side with a reason: behind, carrying a booking, or
    /// with a man already gone in the legs. Every side every week would be
    /// wrong; one in eight is about right.
    pub const HALF_TIME_BAR: f32 = 0.56;
}

/// Builds a [`BenchPressure`]. A namespace, not a value: the read is a pure
/// function of the match state and holds nothing between calls.
pub struct SubstitutionUrgency;

impl SubstitutionUrgency {
    // ── clock ───────────────────────────────────────────────────────────
    /// Minute the clock term starts climbing from zero. Before this a
    /// change is a reaction to something, never a matter of the time.
    const CLOCK_FROM: f32 = 15.0;
    /// Minute the clock term reaches one. Past it the match running out
    /// stops being news and the situational terms carry any remaining
    /// changes.
    const CLOCK_TO: f32 = 92.0;

    // ── chase ───────────────────────────────────────────────────────────
    /// How sharply the chase term saturates in the deficit. At 0.85 a
    /// one-goal deficit is already 57% of the term and a three-goal one is
    /// 92%: the difference between losing and losing badly is real but
    /// small, because the bench is finite either way.
    const CHASE_SATURATION: f32 = 0.85;
    /// What a deficit is worth at kickoff, as a fraction of what it is
    /// worth at the final whistle. Being two down matters at 40'; it
    /// matters more at 80'.
    const CHASE_FLOOR: f32 = 0.35;
    /// Ceiling on the chase term — about twenty-five minutes of ramp.
    const CHASE_GAIN: f32 = 0.42;

    // ── close-out ───────────────────────────────────────────────────────
    const LEAD_SATURATION: f32 = 0.70;
    /// Fraction of the match before which a lead is not yet worth spending
    /// a change to protect.
    const CLOSE_OUT_FROM: f32 = 0.55;
    const CLOSE_OUT_TO: f32 = 0.95;
    const CLOSE_OUT_GAIN: f32 = 0.30;

    // ── legs ────────────────────────────────────────────────────────────
    /// Condition, as a fraction of full, at which a player stops being
    /// worth taking off for tiredness alone.
    const FRESH_ENOUGH: f32 = 0.55;
    /// Condition at which he is finished, and the term saturates.
    const SPENT: f32 = 0.10;
    /// Weight on the single worst-off starter versus the squad as a whole.
    /// One man visibly gone is a more common reason for a change than
    /// eleven slightly tired ones.
    const WORST_WEIGHT: f32 = 0.60;
    const LEGS_GAIN: f32 = 0.35;

    // ── trouble ─────────────────────────────────────────────────────────
    /// What a booking is worth at kickoff. Scaled down as the match runs
    /// out — a yellow card in the 15th minute is an hour and a quarter of
    /// exposure, one in the 85th is nearly nothing.
    const BOOKING_WEIGHT: f32 = 0.55;
    /// What an error leading to a goal is worth, per error, capped at two.
    const ERROR_WEIGHT: f32 = 0.50;
    /// Live rating at which a player's game has not gone wrong at all.
    const RATING_FINE: f32 = 6.05;
    /// Live rating at which it has gone as wrong as the term can see.
    const RATING_GONE: f32 = 4.85;
    /// A bad game is a softer signal than a card or an error — a manager
    /// gives a struggling player longer than he gives a booked one.
    const COLLAPSE_WEIGHT: f32 = 0.80;
    /// Minutes a player needs on the pitch before his live rating is read
    /// as a verdict rather than as noise.
    const RATING_SETTLES_BY: u16 = 25;
    const TROUBLE_GAIN: f32 = 0.30;

    // ── temperament ─────────────────────────────────────────────────────
    /// Ceiling on the manager term, in pressure. About thirteen minutes of
    /// ramp either side of the mean, which is roughly the spread between a
    /// habitually early changer and a habitually late one.
    const TEMPERAMENT_GAIN: f32 = 0.17;
    /// How much of the manager term comes from the man rather than from the
    /// afternoon, when there is a real manager attached to the side. The
    /// remainder keeps two fixtures under the same coach from being
    /// identical.
    const PROFILE_SHARE: f32 = 0.65;

    /// Read one side's appetite for a change.
    ///
    /// `goal_diff` is from this side's perspective and has already passed
    /// whatever visibility gate the caller applies. `starters` and `live`
    /// are the same outfield players in the same order — `live` may be
    /// shorter (or empty) when the caller has not built the snapshots, in
    /// which case the trouble term simply reads zero.
    pub fn read(
        minute: u32,
        goal_diff: i32,
        starters: &[&MatchPlayer],
        live: &[LiveSubstitutionStats],
        profile: Option<&CoachProfile>,
        seed: u64,
        team_id: u32,
    ) -> BenchPressure {
        let m = minute as f32;
        let progress = (m / 90.0).clamp(0.0, 1.2);

        let clock =
            ((m - Self::CLOCK_FROM) / (Self::CLOCK_TO - Self::CLOCK_FROM)).clamp(0.0, 1.0);

        let chase = {
            let deficit = (-goal_diff).max(0) as f32;
            let depth = 1.0 - (-Self::CHASE_SATURATION * deficit).exp();
            let bite = Self::CHASE_FLOOR + (1.0 - Self::CHASE_FLOOR) * progress.min(1.0);
            depth * bite * Self::CHASE_GAIN
        };

        let close_out = {
            let lead = goal_diff.max(0) as f32;
            let depth = 1.0 - (-Self::LEAD_SATURATION * lead).exp();
            let window = ((progress - Self::CLOSE_OUT_FROM)
                / (Self::CLOSE_OUT_TO - Self::CLOSE_OUT_FROM))
                .clamp(0.0, 1.0);
            depth * window * Self::CLOSE_OUT_GAIN
        };

        let legs = Self::legs(starters);
        let trouble = Self::trouble(starters, live, progress);
        let temperament = Self::temperament(profile, seed, team_id);

        let total = (clock + chase + close_out + legs + trouble + temperament).max(0.0);

        BenchPressure {
            clock,
            chase,
            close_out,
            legs,
            trouble,
            temperament,
            total,
        }
    }

    /// How drained this side is: the worst-off starter, blended with the
    /// squad mean so eleven tired men outweigh one.
    ///
    /// The keeper is excluded — he is not the one who has been running, and
    /// including him diluted the mean by a ninth of a fresh player every
    /// time.
    fn legs(starters: &[&MatchPlayer]) -> f32 {
        let mut worst: f32 = 0.0;
        let mut sum: f32 = 0.0;
        let mut n: u32 = 0;
        for p in starters {
            if p.tactical_position.current_position == PlayerPositionType::Goalkeeper {
                continue;
            }
            let cond = (p.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
            let drain =
                ((Self::FRESH_ENOUGH - cond) / (Self::FRESH_ENOUGH - Self::SPENT)).clamp(0.0, 1.0);
            worst = worst.max(drain);
            sum += drain;
            n += 1;
        }
        if n == 0 {
            return 0.0;
        }
        let spread = sum / n as f32;
        (Self::WORST_WEIGHT * worst + (1.0 - Self::WORST_WEIGHT) * spread) * Self::LEGS_GAIN
    }

    /// The most alarming thing happening to a man on the pitch.
    ///
    /// A maximum rather than a sum, and deliberately so: a manager reacts to
    /// the one player he is worried about, and two separate worries do not
    /// make him twice as likely to act — they make him pick which one to
    /// solve. Summing them was what turned a scrappy first half into three
    /// changes on the hour in an early draft.
    fn trouble(starters: &[&MatchPlayer], live: &[LiveSubstitutionStats], progress: f32) -> f32 {
        // How much of the match a booked player still has to survive.
        let exposure = (1.0 - progress).clamp(0.0, 1.0);
        let mut worst: f32 = 0.0;
        for (idx, p) in starters.iter().enumerate() {
            let Some(l) = live.get(idx) else { break };
            if p.tactical_position.current_position == PlayerPositionType::Goalkeeper {
                continue;
            }
            let booking = if l.yellow_carded() {
                Self::BOOKING_WEIGHT * exposure
            } else {
                0.0
            };
            let error = (l.errors_leading_to_goal.min(2) as f32) * Self::ERROR_WEIGHT;
            // A rating needs a game behind it before it means anything. A
            // man who has been on for ten minutes has not had a bad game,
            // he has had ten minutes — and without this the term read as a
            // standing complaint against everybody in the opening half-hour
            // of every match, which is not a signal, it is an offset.
            let settled =
                (l.minutes_played as f32 / Self::RATING_SETTLES_BY as f32).clamp(0.0, 1.0);
            let collapse = ((Self::RATING_FINE - l.live_rating)
                / (Self::RATING_FINE - Self::RATING_GONE))
                .clamp(0.0, 1.0)
                * settled
                * Self::COLLAPSE_WEIGHT;
            worst = worst.max(booking).max(error).max(collapse);
        }
        worst.min(1.0) * Self::TROUBLE_GAIN
    }

    /// The manager term: how early this man, in this match, goes to his
    /// bench.
    ///
    /// Two sources, both stable for the whole ninety minutes. The coach's
    /// own profile supplies the part that should be recognisable across a
    /// season — a cautious manager is late every week — and a hash of the
    /// match seed and the team supplies the part that keeps the same
    /// manager from making his change on the same minute in every fixture.
    /// With no manager attached (a national side, a test fixture, the
    /// calibration harness) the whole term comes from the hash, which is
    /// the honest reading: an unknown manager is a draw from the
    /// population, not the population mean.
    fn temperament(profile: Option<&CoachProfile>, seed: u64, team_id: u32) -> f32 {
        let afternoon = Self::hash_axis(seed, team_id);
        let axis = match profile {
            Some(p) => {
                // Decisiveness, centred: willing to act on incomplete
                // information, and not wedded to the plan he started with.
                let decisive = 0.5 * p.risk_tolerance + 0.5 * (1.0 - p.conservatism);
                let man = (decisive.clamp(0.0, 1.0) - 0.5) * 2.0;
                Self::PROFILE_SHARE * man + (1.0 - Self::PROFILE_SHARE) * afternoon
            }
            None => afternoon,
        };
        axis.clamp(-1.0, 1.0) * Self::TEMPERAMENT_GAIN
    }

    /// A stable value in `[-1, 1]` from the match seed and the team.
    ///
    /// splitmix64, the same finalizer `MatchRng::from_seed` uses to fan a
    /// seed out, chosen so that neighbouring seeds — which is what a batch
    /// of fixtures generated in a loop produces — do not give neighbouring
    /// managers. Draws nothing from the match RNG, so a fixture with no
    /// bench consumes exactly the stream it consumed before this model
    /// existed.
    fn hash_axis(seed: u64, team_id: u32) -> f32 {
        // The odd constant is not decoration: splitmix64 maps zero to zero,
        // so without it a match seeded 0 — every fixture in every test that
        // does not bother to pick one — would hand team 0 the single most
        // cautious manager in the population rather than an average one.
        let mut z = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((team_id as u64).wrapping_mul(0xD1B5_4A32_D192_ED03))
            .wrapping_add(0x2545_F491_4F6C_DD1D);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Top 24 bits into [0, 1), then centred.
        let unit = (z >> 40) as f32 / (1u64 << 24) as f32;
        unit * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A side with a completely unremarkable afternoon: level, unhurt,
    /// nobody booked, and a manager of exactly average temperament. The
    /// baseline every other case is a displacement from.
    fn pressure(minute: u32, goal_diff: i32) -> BenchPressure {
        let mut p = SubstitutionUrgency::read(minute, goal_diff, &[], &[], None, 0, 0);
        p.total -= p.temperament;
        p.temperament = 0.0;
        p
    }

    #[test]
    fn clock_is_the_only_term_in_a_quiet_level_game() {
        let p = pressure(60, 0);
        assert_eq!(p.chase, 0.0);
        assert_eq!(p.close_out, 0.0);
        assert_eq!(p.legs, 0.0);
        assert_eq!(p.trouble, 0.0);
        let expected = (60.0 - SubstitutionUrgency::CLOCK_FROM)
            / (SubstitutionUrgency::CLOCK_TO - SubstitutionUrgency::CLOCK_FROM);
        assert!((p.clock - expected).abs() < 1e-5);
    }

    #[test]
    fn a_deficit_brings_the_first_change_forward() {
        // The minute a level side clears the first bar, versus a side two
        // goals down. The gap is the whole point of the model.
        let first_clear = |gd: i32| {
            (20..=95)
                .find(|&m| pressure(m, gd).clears(0))
                .expect("bar cleared at some point")
        };
        let level = first_clear(0);
        let two_down = first_clear(-2);
        assert!(
            (58..=72).contains(&level),
            "an average manager in a level game should reach the first bar \
             around the hour, got {level}'"
        );
        assert!(
            level - two_down >= 10,
            "two goals down should be at least ten minutes earlier: {two_down}' vs {level}'"
        );
    }

    #[test]
    fn a_lead_does_not_bring_it_forward_as_far_as_a_deficit() {
        let at = |gd: i32| pressure(70, gd).total;
        assert!(
            at(-2) > at(2),
            "chasing two goals must press harder than protecting two"
        );
        assert!(
            at(2) > at(0),
            "a lead worth killing should still press harder than a level game"
        );
    }

    #[test]
    fn the_fifth_change_needs_more_than_the_clock() {
        // Ramp saturates at 1.0; the fifth bar sits above it, so a side with
        // nothing else pushing finishes with a change unused.
        let quiet = pressure(95, 0);
        assert!(quiet.clears(0) && quiet.clears(1));
        assert!(
            !quiet.clears(4),
            "a quiet game should not empty the bench on the clock alone"
        );
    }

    #[test]
    fn temperament_is_stable_within_a_match_and_varies_across_them() {
        let a = SubstitutionUrgency::temperament(None, 12345, 7);
        let b = SubstitutionUrgency::temperament(None, 12345, 7);
        assert_eq!(a, b, "the same manager must not change his mind mid-match");
        let other_team = SubstitutionUrgency::temperament(None, 12345, 8);
        let other_match = SubstitutionUrgency::temperament(None, 12346, 7);
        assert_ne!(a, other_team);
        assert_ne!(a, other_match);
        assert!(a.abs() <= SubstitutionUrgency::TEMPERAMENT_GAIN + 1e-6);
    }

    #[test]
    fn temperament_spreads_the_first_change_by_several_minutes() {
        // Across a population of seeds, the minute the first bar is cleared
        // in an otherwise identical game must not be a single value.
        let mut minutes: Vec<u32> = (0..200u64)
            .map(|s| {
                let t = SubstitutionUrgency::temperament(None, s * 0x9E37, 1);
                (20..=95)
                    .find(|&m| {
                        let mut p = pressure(m, 0);
                        p.total = (p.clock + t).max(0.0);
                        p.clears(0)
                    })
                    .unwrap_or(95)
            })
            .collect();
        minutes.sort_unstable();
        let span = minutes[minutes.len() - 1] - minutes[0];
        assert!(
            span >= 12,
            "manager temperament should be worth a dozen minutes of spread, got {span}"
        );
    }

    #[test]
    fn a_cautious_coach_is_later_than_a_bold_one() {
        let mut bold = CoachProfile::neutral();
        bold.risk_tolerance = 0.95;
        bold.conservatism = 0.05;
        let mut cautious = CoachProfile::neutral();
        cautious.risk_tolerance = 0.05;
        cautious.conservatism = 0.95;
        let b = SubstitutionUrgency::temperament(Some(&bold), 99, 1);
        let c = SubstitutionUrgency::temperament(Some(&cautious), 99, 1);
        assert!(b > c, "bold {b} should out-press cautious {c}");
        assert!(b - c > 0.10, "and by a meaningful margin: {}", b - c);
    }

    #[test]
    fn conviction_climbs_across_the_bar() {
        let mut p = pressure(60, 0);
        p.total = BenchPressure::bar(0) - 1.0;
        assert_eq!(p.conviction(0), 0.0);
        p.total = BenchPressure::bar(0);
        assert!((p.conviction(0) - 0.5).abs() < 1e-5);
        p.total = BenchPressure::bar(0) + 1.0;
        assert_eq!(p.conviction(0), 1.0);
    }

    #[test]
    fn half_time_bar_sits_below_the_first_open_play_bar() {
        assert!(BenchPressure::HALF_TIME_BAR < BenchPressure::FIRST_BAR);
    }
}
