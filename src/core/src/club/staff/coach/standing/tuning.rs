//! Every number the standing ladder turns on.

/// Constants governing where a player stands with his manager, how he gets
/// there, and how hard it is to come back.
pub struct StandingTuning;

impl StandingTuning {
    // ── Impulses ────────────────────────────────────────────────
    //
    // Signed nudges to the continuous score. Sized so that an ordinary
    // league season of ordinary performances moves a man a rung or two,
    // and one night can move him further than a month.

    /// Per point of match rating above or below what the coach expected of
    /// *this* player. A 6.0 from a man he has down as a 7.0 is worth −0.04.
    pub const RATING_PER_POINT: f32 = 0.04;
    pub const RATING_CLAMP: f32 = 0.12;

    /// The nights that tell you something about a footballer.
    pub const BIG_MATCH_GOOD: f32 = 0.10;
    pub const BIG_MATCH_BAD: f32 = -0.14;

    pub const COSTLY_ERROR: f32 = -0.12;
    pub const RED_CARD: f32 = -0.15;
    /// On the one occasion that counted, both land twice as hard.
    pub const OCCASION_MULTIPLIER: f32 = 2.0;
    /// Match importance at which a fixture counts as *the* occasion.
    pub const OCCASION_IMPORTANCE: f32 = 0.90;

    pub const EARLY_HOOK: f32 = -0.05;
    pub const CLEAN_FULL_MATCH: f32 = 0.03;
    pub const DISCIPLINE_EVENT: f32 = -0.08;

    /// He would not play for me.
    pub const REFUSED: f32 = -0.60;
    /// He asked out while I was picking him.
    pub const WANTS_OUT: f32 = -0.30;
    /// A loyal man takes that harder.
    pub const WANTS_OUT_LOYAL: f32 = -0.40;
    /// Coach `loyalty` attribute at which he does.
    pub const LOYAL_ATTRIBUTE: f32 = 14.0;
    /// He took it to the press.
    pub const WENT_PUBLIC: f32 = -0.35;

    pub const TALK_POSITIVE: f32 = 0.08;
    pub const TALK_NEGATIVE: f32 = -0.04;

    /// Per week, once a run of weeks at one extreme of training has built
    /// up. Training is a slow signal and this is what makes it one.
    pub const TRAINING_WEEK: f32 = 0.04;
    /// Consecutive weeks before the run registers at all.
    pub const TRAINING_RUN_WEEKS: u8 = 4;
    /// Training impression above / below which a week counts.
    pub const TRAINING_HIGH: f32 = 0.70;
    pub const TRAINING_LOW: f32 = 0.35;
    /// And the most a month of it can be worth either way.
    pub const TRAINING_MONTHLY_CAP: f32 = 0.16;

    /// He was promised minutes and got them.
    pub const PROMISE_KEPT: f32 = 0.05;
    /// He was promised minutes and did not. That is the coach's failure,
    /// so it buys the player a debt rather than costing him standing.
    pub const DEBT_PROMISE_BROKEN: f32 = 0.35;
    /// I picked him against the evidence and he proved me right.
    pub const REPAID_FAITH: f32 = 0.25;

    // ── How hard he feels it ────────────────────────────────────

    /// Base gain on every impulse, before personality.
    pub const GAIN_BASE: f32 = 0.5;
    /// How far emotional volatility moves it.
    pub const GAIN_VOLATILITY_SPAN: f32 = 0.5;

    /// A negativity-biased coach feels the bad more and the good less —
    /// the same asymmetry `CoachMemory`'s trust delta already uses.
    pub const NEGATIVITY_DOWN_SPAN: f32 = 0.5;
    pub const NEGATIVITY_UP_SPAN: f32 = 0.2;

    /// A coach defends his own signing. Hardest in the first year, when
    /// admitting the mistake would also be admitting he made it.
    pub const SUNK_COST_YEAR_ONE_SPAN: f32 = 0.4;
    pub const SUNK_COST_AFTER_SPAN: f32 = 0.15;
    pub const SUNK_COST_YEAR_DAYS: i64 = 365;

    /// Confirmation bias: evidence that agrees with his first impression
    /// counts for more, evidence that contradicts it for less.
    pub const CONFIRMATION_SPAN: f32 = 0.3;

    // ── Time ────────────────────────────────────────────────────

    /// A standing nobody is adding to drifts back toward nothing.
    pub const IDLE_DRIFT_PER_MONTH: f32 = 0.02;
    pub const HEAT_DECAY_PER_MATCH: f32 = 0.85;
    pub const HEAT_DECAY_PER_IDLE_MONTH: f32 = 0.70;
    /// A single impulse adds this much of its magnitude to the heat.
    pub const HEAT_PER_IMPULSE: f32 = 1.5;

    // ── The rungs ───────────────────────────────────────────────
    //
    // Entering is easier than leaving, in both directions, and that
    // asymmetry is the whole point: a manager who reverses himself weekly
    // has not made a decision, he has had a mood.

    pub const ENTER_UNDROPPABLE: f32 = 0.70;
    pub const LEAVE_UNDROPPABLE: f32 = 0.50;
    /// Matches watched before he will call anyone undroppable.
    pub const UNDROPPABLE_MATCHES: u16 = 20;

    pub const ENTER_TRUSTED: f32 = 0.35;
    pub const LEAVE_TRUSTED: f32 = 0.15;

    pub const ENTER_IN_FAVOUR: f32 = 0.12;
    pub const LEAVE_IN_FAVOUR: f32 = 0.02;

    pub const ENTER_UNDER_REVIEW: f32 = -0.20;
    pub const LEAVE_UNDER_REVIEW: f32 = -0.05;
    /// Heat that puts a man under review on a milder score — the coach is
    /// thinking about him, which is itself the danger.
    pub const HEAT_UNDER_REVIEW: f32 = 0.60;
    pub const HEAT_UNDER_REVIEW_SCORE: f32 = -0.10;

    pub const ENTER_OUT_OF_FAVOUR: f32 = -0.45;
    pub const LEAVE_OUT_OF_FAVOUR: f32 = -0.25;
    /// A settled review that keeps going one way ends the same place.
    pub const REVIEW_TO_OUT_OF_FAVOUR_MATCHES: u8 = 4;
    pub const REVIEW_TO_OUT_OF_FAVOUR_SCORE: f32 = -0.30;

    pub const ENTER_FROZEN_OUT: f32 = -0.75;
    pub const LEAVE_FROZEN_OUT: f32 = -0.50;
    /// A stubborn coach lets a long spell out of favour harden.
    pub const OUT_OF_FAVOUR_TO_FROZEN_DAYS: i64 = 60;
    pub const OUT_OF_FAVOUR_TO_FROZEN_SCORE: f32 = -0.55;
    pub const OUT_OF_FAVOUR_TO_FROZEN_STUBBORNNESS: f32 = 0.6;

    // ── Minimum stays ───────────────────────────────────────────

    pub const MIN_STAY_UNDER_REVIEW_MATCHES: u8 = 2;
    pub const MIN_STAY_OUT_OF_FAVOUR_DAYS: i64 = 21;
    pub const MIN_STAY_OUT_OF_FAVOUR_MATCHES: u8 = 3;
    /// A stubborn coach needs more than that.
    pub const MIN_STAY_OUT_OF_FAVOUR_MATCHES_STUBBORN: u8 = 5;
    pub const STUBBORN: f32 = 0.6;
    pub const MIN_STAY_FROZEN_DAYS: i64 = 45;
    /// A warm coach will let a frozen-out man back in on time alone.
    pub const THAW_MAN_MANAGEMENT: f32 = 0.6;
    pub const THAW_DAYS: i64 = 30;

    // ── What a rung does ────────────────────────────────────────
    //
    // Shifts on `start_preference` / `bench_preference`, which the
    // assessment then scales into slot points. Deliberately inside the
    // envelope the coach lens already had, with one exception.

    pub const SHIFT_UNDROPPABLE: f32 = 0.18;
    pub const SHIFT_TRUSTED: f32 = 0.10;
    pub const SHIFT_IN_FAVOUR: f32 = 0.04;
    pub const SHIFT_UNDER_REVIEW: f32 = -0.08;
    pub const SHIFT_OUT_OF_FAVOUR: f32 = -0.22;
    /// The exception. A frozen-out player is not being rotated, he is not
    /// being picked, and the number has to say so.
    pub const SHIFT_FROZEN_OUT: f32 = -0.45;

    pub const BENCH_UNDROPPABLE: f32 = 0.10;
    pub const BENCH_TRUSTED: f32 = 0.06;
    pub const BENCH_IN_FAVOUR: f32 = 0.02;
    pub const BENCH_UNDER_REVIEW: f32 = 0.0;
    pub const BENCH_OUT_OF_FAVOUR: f32 = -0.10;
    pub const BENCH_FROZEN_OUT: f32 = -0.30;

    /// A coach backs a man he trusts through a slump.
    pub const TRUSTED_FORM_DAMPENER: f32 = 0.6;
    pub const UNDROPPABLE_FORM_DAMPENER: f32 = 0.4;

    /// Fit senior outfielders below which nobody gets frozen out, whatever
    /// the coach thinks of him. Needs must.
    pub const FROZEN_SQUAD_FLOOR: usize = 14;

    // ── Getting back in ─────────────────────────────────────────

    /// Cameos at this rating, in the last five appearances, and the coach
    /// looks again.
    pub const RECOVERY_CAMEOS: u8 = 2;
    pub const RECOVERY_CAMEO_RATING: f32 = 7.0;
    /// Or a run of weeks working like a man who wants his place back.
    pub const RECOVERY_TRAINING_WEEKS: u8 = 6;
    pub const RECOVERY_TRAINING_TRUST: f32 = 0.65;

    /// A debt this size, on a coach who pays them, buys a start.
    pub const SECOND_CHANCE_DEBT: f32 = 0.5;
    pub const SECOND_CHANCE_MAN_MANAGEMENT: f32 = 0.6;
    /// And it is paid in a fixture where a mistake is affordable.
    pub const SECOND_CHANCE_IMPORTANCE: f32 = 0.6;

    /// After a man repays the coach's faith he is protected for a while.
    pub const PROTECTED_DAYS: i64 = 90;
    pub const PROTECTED_FORM_DAMPENER: f32 = 0.5;

    /// A patient coach gives a young player a run before judging him.
    pub const YOUTH_GRACE_MATCHES: u16 = 8;
    pub const YOUTH_GRACE_AGE: u8 = 22;
    pub const YOUTH_GRACE_PATIENCE: f32 = 0.3;

    /// A captain has a floor under him. A manager who wants to freeze out
    /// his own captain takes the armband off him first.
    pub const CAPTAIN_FLOOR: f32 = -0.20;

    // ── The big-match window ────────────────────────────────────

    /// Big-match starts the coach holds in mind.
    pub const BIG_MATCH_WINDOW: u8 = 4;
    pub const BIG_MATCH_WINDOW_MASK: u8 = (1 << Self::BIG_MATCH_WINDOW) - 1;
    /// Failures inside it before he stops picking the man for them.
    pub const BIG_MATCH_FAILURES: u32 = 2;
    /// A volatile coach needs one; a good judge wants three.
    pub const BIG_MATCH_FAILURES_VOLATILE: u32 = 1;
    pub const BIG_MATCH_VOLATILITY: f32 = 0.7;
    pub const BIG_MATCH_FAILURES_PATIENT: u32 = 3;
    pub const BIG_MATCH_JUDGING: f32 = 0.8;
    /// Rating below which a big-match start counts as a failure, and above
    /// which it counts as an answer.
    pub const BIG_MATCH_FAILURE_RATING: f32 = 5.7;
    pub const BIG_MATCH_GOOD_RATING: f32 = 7.2;
    /// Consecutive big-match starts at this rating to win the trust back.
    pub const BIG_MATCH_REDEMPTION_RATING: f32 = 6.8;
    pub const BIG_MATCH_REDEMPTION_RUN: u8 = 2;
    /// How far distrust in the big matches moves selection — in the big
    /// matches only. He is fine on a wet Tuesday and the coach knows it.
    pub const BIG_MATCH_UNTRUSTED_SHIFT: f32 = -0.30;
    /// A warm coach gives a trusted man one more before the flag bites.
    pub const BIG_MATCH_GRACE_MAN_MANAGEMENT: f32 = 0.6;

    // ── The favourite ───────────────────────────────────────────

    /// Matches of real form pressure before even an undroppable man is
    /// dropped. A good judge sees it sooner.
    pub const FAVOURITE_CAP_MATCHES: u8 = 8;
    pub const FAVOURITE_CAP_MATCHES_SHARP: u8 = 5;
    pub const FAVOURITE_CAP_JUDGING: f32 = 0.75;
    pub const FAVOURITE_PRESSURE: f32 = 0.45;
    /// What the reckoning costs him when it comes.
    pub const FAVOURITE_CAP_PENALTY: f32 = -0.25;

    // ── Helpers ─────────────────────────────────────────────────

    /// How hard this coach feels things.
    #[inline]
    pub fn gain(volatility: f32) -> f32 {
        Self::GAIN_BASE + volatility.clamp(0.0, 1.0) * Self::GAIN_VOLATILITY_SPAN
    }

    /// Asymmetric personality scaling on one impulse.
    #[inline]
    pub fn negativity_scale(impulse: f32, negativity: f32) -> f32 {
        let negativity = negativity.clamp(0.0, 1.0);
        if impulse < 0.0 {
            1.0 + negativity * Self::NEGATIVITY_DOWN_SPAN
        } else {
            1.0 - negativity * Self::NEGATIVITY_UP_SPAN
        }
    }
}
