//! Every number the dossier layer turns on, in one place.
//!
//! The house pattern (see `academy/tuning.rs`): an associated const with the
//! sentence that justifies it, so a tuning pass is a diff in one file rather
//! than a hunt through five.

/// Constants governing what a coach keeps about a player after they part,
/// and what he does with it when they meet again.
pub struct DossierTuning;

impl DossierTuning {
    // ── The store ───────────────────────────────────────────────

    /// How many players a coach holds a dossier on. A thirty-year career at
    /// three clubs a decade passes through perhaps four hundred players; he
    /// remembers the ones who mattered, and this is roughly how many that is.
    pub const CAPACITY: usize = 192;

    /// Eviction weights. Years together outrank everything, because the thing
    /// a manager actually retains is the man he worked with daily for three
    /// seasons — not the one who had a memorable night.
    pub const SIGNIFICANCE_W_MATCHES: f32 = 0.45;
    pub const SIGNIFICANCE_W_WARMTH: f32 = 0.25;
    pub const SIGNIFICANCE_W_MARKS: f32 = 0.15;
    pub const SIGNIFICANCE_W_RECENCY: f32 = 0.15;

    /// A protected scar or medal — a red card in a final, a refusal, a
    /// captaincy — is expensive to learn and never makes way for a passing
    /// acquaintance.
    pub const SIGNIFICANCE_PROTECTED_BONUS: f32 = 0.50;

    /// Matches at which the depth term saturates for eviction ranking.
    pub const SIGNIFICANCE_MATCHES_FULL: f32 = 60.0;

    /// Marks (scars + medals) at which that term saturates.
    pub const SIGNIFICANCE_MARKS_FULL: f32 = 4.0;

    /// Half-life shape of the recency term, in years.
    pub const SIGNIFICANCE_RECENCY_TAU_YEARS: f32 = 8.0;

    // ── Warmth at parting ───────────────────────────────────────

    pub const WARMTH_W_STANDING: f32 = 0.40;
    pub const WARMTH_W_PROFESSIONALISM: f32 = 0.20;
    pub const WARMTH_W_MEDALS: f32 = 0.15;
    pub const WARMTH_W_SCARS: f32 = 0.25;
    /// Warmth is reciprocal: a coach warms to a player who liked him.
    pub const WARMTH_W_HIS_STANCE: f32 = 0.10;

    /// Medals and scars at which their warmth terms saturate.
    pub const WARMTH_MEDALS_FULL: f32 = 3.0;

    // ── How it ended ────────────────────────────────────────────
    //
    // A parting is the last thing that happens between two people and it
    // colours everything before it. Signed, applied to warmth.

    pub const PARTING_SOLD_BY_BOARD: f32 = 0.05;
    pub const PARTING_SOLD_ON_MY_CALL: f32 = -0.10;
    pub const PARTING_RELEASED_ON_MY_CALL: f32 = -0.20;
    pub const PARTING_RELEASED_BY_BOARD: f32 = 0.0;
    pub const PARTING_HE_REQUESTED_OUT: f32 = -0.25;
    /// A player the coach rated walking out is the sharper version of the
    /// same thing.
    pub const PARTING_HE_WALKED_OUT_ON_ME: f32 = -0.40;
    pub const PARTING_HE_RAN_DOWN_HIS_CONTRACT: f32 = -0.30;
    pub const PARTING_HE_RETIRED: f32 = 0.10;

    /// Standing above which "he asked to leave" reads as desertion rather
    /// than a squad player taking his chance.
    pub const WALKED_OUT_STANDING: f32 = 0.30;

    /// A loyal man takes desertion harder. Scales the negative parting terms
    /// by the coach's own `loyalty` attribute (0–20).
    pub const LOYALTY_PARTING_BASE: f32 = 0.6;
    pub const LOYALTY_PARTING_SPAN: f32 = 0.8;

    /// A hot-tempered man carries a grievance further. Scales the scar term.
    pub const TEMPERAMENT_SCAR_BASE: f32 = 0.7;
    pub const TEMPERAMENT_SCAR_SPAN: f32 = 0.6;

    /// How wrong a coach can be about how the player felt. A high
    /// man-management coach reads the room; a low one guesses.
    pub const HIS_STANCE_NOISE: f32 = 0.3;

    // ── Scars over time ─────────────────────────────────────────

    /// A quarter of a grievance goes every year.
    pub const SCAR_DECAY_PER_YEAR: f32 = 0.75;

    /// Below which a protected scar never falls, as a fraction of what it
    /// weighed at the time. A red card in a final is still a red card in a
    /// final a decade later.
    pub const SCAR_PROTECTED_FLOOR: f32 = 0.35;

    /// What an old grievance still has to weigh before a coach carries it
    /// into a new dressing room as something he is actively holding against
    /// the man, rather than merely something he remembers.
    ///
    /// An absolute, unlike [`Self::SCAR_PROTECTED_FLOOR`], which is a
    /// fraction — the two are easy to confuse and they are not the same
    /// quantity.
    pub const SCAR_REARM: f32 = 0.20;

    /// A public record can soften a private grievance: a man whose
    /// big-match football since has been good halves the flop scar.
    pub const PUBLIC_RECORD_SOFTENING: f32 = 0.5;
    /// Big-match games needed before that public record counts.
    pub const PUBLIC_RECORD_MIN_GAMES: u16 = 8;
    /// And the average he has to have kept over them.
    pub const PUBLIC_RECORD_RATING: f32 = 7.0;

    // ── The reunion prior ───────────────────────────────────────

    /// Years at which the time term falls to 1/e. One year → 0.78, three →
    /// 0.47, six → 0.22: he remembers, but he no longer trusts the detail.
    pub const TAU_REUNION_YEARS: f32 = 4.0;

    /// He never forgets a man he coached, and he never skips looking again.
    pub const PRIOR_MIN: f32 = 0.10;
    pub const PRIOR_MAX: f32 = 0.85;

    /// Matches together at which the depth term saturates.
    pub const DEPTH_FULL_AT_MATCHES: f32 = 20.0;
    pub const DEPTH_MIN: f32 = 0.25;

    /// A player who has crossed thirty, or who was a boy when the coach last
    /// saw him, is a different footballer.
    pub const AGE_BAND_PENALTY: f32 = 0.6;
    pub const AGE_BAND_OLD: u8 = 30;
    pub const AGE_BAND_BOY: u8 = 21;
    pub const AGE_BAND_GROWN: u8 = 24;

    /// A good judge trusts his own old read further.
    pub const EYE_PRIOR_BASE: f32 = 0.85;
    pub const EYE_PRIOR_SPAN: f32 = 0.30;

    /// Warmth fades slower than detail — you forget what a player could do
    /// long before you forget whether you liked him.
    pub const WARMTH_FADE_FLOOR: f32 = 0.6;

    // ── Seeding a reunion ───────────────────────────────────────

    /// Observations the coach credits himself with on day one of a reunion,
    /// scaled by the prior. At a full prior he is well-observed immediately.
    pub const SEED_OBSERVATIONS: f32 = 6.0;

    /// Character reads stick harder than ability reads: whether a man is a
    /// professional is not something you re-open.
    pub const SEED_PROFESSIONALISM_MIN_PRIOR: f32 = 0.6;

    /// Warmth alone is worth a little standing before a ball is kicked.
    pub const SEED_STANDING_WARMTH_BONUS: f32 = 0.10;

    /// How long a reunion floor (or ceiling) holds the plan before the
    /// evidence of this spell takes over.
    pub const REUNION_PLAN_DAYS: i64 = 90;
    /// Above this age the old role is no longer a floor worth honouring.
    pub const REUNION_PLAN_MAX_AGE: u8 = 31;
    /// Warmth at which the coach hands a returning player his old role back.
    pub const REUNION_PLAN_WARMTH: f32 = 0.30;
    /// Stubbornness at which he holds an old grievance against a man
    /// rather than letting the new spell speak for itself.
    pub const REUNION_STUBBORN: f32 = 0.6;

    /// A manager reads loan reports; he does not watch every game. One loan
    /// appearance is worth this much of a match he saw himself.
    pub const LOAN_REPORT_WEIGHT: f32 = 0.4;

    // ── Affinity: whether he wants him again ────────────────────

    /// Multiplier on a shortlist score: `1 + SCALE · affinity`.
    pub const AFFINITY_SCALE: f32 = 0.18;
    /// Below this he will not have the player at any price short of an
    /// emergency.
    pub const AFFINITY_VETO: f32 = -0.50;
    /// Above this he asks for him by name.
    pub const AFFINITY_REQUEST_MIN: f32 = 0.45;

    pub const AFFINITY_W_WARMTH: f32 = 0.7;
    pub const AFFINITY_W_LEVEL: f32 = 0.3;
    pub const AFFINITY_W_SCAR: f32 = 0.8;

    pub const AFFINITY_CONVICTION_WORTH: f32 = 0.15;
    pub const AFFINITY_CONVICTION_LET_DOWN: f32 = -0.25;
    pub const AFFINITY_CONVICTION_WRONG: f32 = -0.10;

    // ── And whether the player will have him ────────────────────

    pub const PLAYER_AFFINITY_W_MADE: f32 = 0.3;
    pub const PLAYER_AFFINITY_W_BACKED: f32 = 0.2;
    pub const PLAYER_AFFINITY_W_CLASHED: f32 = -0.4;
    pub const PLAYER_AFFINITY_W_NEVER_TRUSTED: f32 = -0.3;
    pub const PLAYER_AFFINITY_W_WORD: f32 = -0.5;

    /// How far a player's feeling about the manager moves his willingness.
    pub const PLAYER_AFFINITY_WILLINGNESS: f32 = 0.12;
    /// Below which he will not work for the man at all.
    pub const PLAYER_AFFINITY_REFUSAL: f32 = -0.60;

    /// How hard a sacked manager's favourite wants to follow him.
    pub const FOLLOW_MY_MANAGER_STRENGTH: f32 = 0.25;
    /// And how fast that want fades when no move materialises.
    pub const FOLLOW_MY_MANAGER_EASE_PER_MONTH: f32 = 0.05;

    /// Base magnitude of the reunion happiness event, scaled and signed by
    /// how the player feels about the man.
    pub const REUNION_EVENT_MAGNITUDE: f32 = 6.0;

    // ── Helpers ─────────────────────────────────────────────────

    /// Scale a negative parting term by the coach's loyalty (0–20).
    #[inline]
    pub fn loyalty_scale(loyalty: f32) -> f32 {
        Self::LOYALTY_PARTING_BASE + (loyalty / 20.0).clamp(0.0, 1.0) * Self::LOYALTY_PARTING_SPAN
    }

    /// Scale the scar term by the coach's temperament (0–20).
    #[inline]
    pub fn temperament_scale(temperament: f32) -> f32 {
        Self::TEMPERAMENT_SCAR_BASE
            + (temperament / 20.0).clamp(0.0, 1.0) * Self::TEMPERAMENT_SCAR_SPAN
    }
}
