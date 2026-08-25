//! **The score-reactive regime**, and the A/B switches that measure it.
//!
//! When teams are allowed to read the scoreline at all
//! ([`SCORE_REACTION_FROM_MINUTE`](MatchContext::SCORE_REACTION_FROM_MINUTE)),
//! how hard it pulls when they do
//! ([`SCORE_REACTION_GAIN`](MatchContext::SCORE_REACTION_GAIN)), and the
//! four env-var switches that turn whole engine layers off so a
//! calibration run can attribute an effect to one of them.
//!
//! All of it is debug/calibration infrastructure as much as it is match
//! behaviour, which is why it lives apart from the context's own state:
//! every item here is read once per PROCESS, not once per match.

use super::match_context::MatchContext;

impl MatchContext {
    /// Diagnostic switch: when the `OF_SCORE_BLIND` env var is set, all
    /// BEHAVIORAL reads of the scoreline return neutral (0-0) — coach
    /// instructions, tactical game management, chasing/protect lifts
    /// and desperation all act as if the match were level, while the
    /// real score still accumulates for the result. Used by the dev
    /// harness to measure how much of the engine's draw-correlation
    /// surplus is carried by the score-reactive regime versus emergent
    /// match state. Read once per process; keep for future calibration
    /// rounds (debug infrastructure — do not remove).
    pub fn score_blind() -> bool {
        use std::sync::OnceLock;
        static BLIND: OnceLock<bool> = OnceLock::new();
        *BLIND.get_or_init(|| std::env::var("OF_SCORE_BLIND").is_ok())
    }

    /// Diagnostic switch: when the `OF_SHAPE_OFF` env var is set, the
    /// team-shape layer is inert — `TeamShape` stops handing out
    /// anchors and `ShapeDiscipline` stops pulling on anybody, so every
    /// off-ball consumer falls back to the kickoff formation dot exactly
    /// as it did before the layer existed.
    ///
    /// This is the A/B control for the whole positional system. Its
    /// effects reach every player on every tick, so "did the shape work
    /// cause this?" cannot be answered by reading the diff — and it must
    /// not be answered by checking out an older revision either, because
    /// the working tree moves under you. Same pattern and same purpose as
    /// [`score_blind`](Self::score_blind); read once per process. Debug
    /// infrastructure — do not remove.
    pub fn shape_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_SHAPE_OFF").is_ok())
    }

    /// Diagnostic switch: with `OF_MID_CLEAR_OFF` set, the midfielder
    /// Tier-1 "clear chance" shot in `midfielders/states/running` stops
    /// being a DETERMINISTIC bypass and the same look goes through the
    /// ordinary appetite-vs-bar decision with every other shot.
    ///
    /// It exists because that tier is the engine's single largest
    /// quality-coupled channel into the SCORELINE, and nothing in the
    /// aggregate stats says so — the tier's only skill-sensitive term is
    /// "no opponent within 3 m", so it fires whenever the defending is
    /// poor enough to leave that space, without a probability anywhere in
    /// it to bound how often. Measured over 300 matches a level, uniform
    /// squads, it is **44% of every shot in the game at level 6 and 2% at
    /// level 18** — the mechanism behind "lower divisions play 3-2 and
    /// the top flight plays 0-0".
    ///
    /// Same pattern and purpose as [`shape_off`](Self::shape_off): the
    /// effect reaches every attacking tick in the box, so the question
    /// "how much of the goals-per-division spread is this one path?"
    /// cannot be answered from a diff. Debug infrastructure — do not
    /// remove.
    pub fn mid_clear_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_MID_CLEAR_OFF").is_ok())
    }

    /// Diagnostic switch: with `OF_PRESS_OFF` set, `DefensivePlan`
    /// nominates a presser only when an opponent is actually CARRYING
    /// the ball, and only from the back line and midfield — the model as
    /// it stood before 2026-08-17.
    ///
    /// The two halves of that are what the switch exists to A/B, because
    /// between them they decide whether anybody closes the ball down at
    /// all: `carrier` is `None` for every pass in flight and every loose
    /// ball, which is most of a match, and the front line was not in the
    /// pool. Measured with this on: press 0.25 duties per refresh,
    /// `Forward: Pressing` 0.3% of AI ticks, ball stuck 18.0 s/match.
    ///
    /// Same pattern and same purpose as [`shape_off`](Self::shape_off) —
    /// the effect reaches every defensive tick, so the question "did the
    /// press work cause this?" cannot be answered from the diff, and must
    /// not be answered by checking out an older revision either.
    /// Debug infrastructure — do not remove.
    pub fn press_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_PRESS_OFF").is_ok())
    }

    /// Match minute before which BEHAVIORAL score reactions stay off —
    /// teams play their football regardless of the scoreline until the
    /// final quarter, exactly like real sides do (managers don't park
    /// the bus at minute 30 or go all-out at minute 40; reactive
    /// game-state football is a post-~65' phenomenon, which is also
    /// where real substitution/instruction activity clusters).
    ///
    /// Why this gate is load-bearing: a score-blind A/B run measured
    /// the engine at rho = −0.05 / 23.5% draws (real: ~0 / 25%) with
    /// reactions off, versus rho = +0.51 / 43-46% draws with them on —
    /// the score-reactive regime, running from minute 1, carried the
    /// ENTIRE equal-strength draw surplus (trailing teams scored 2.35
    /// goals/90 vs leaders' 1.08; real football keeps game-state rates
    /// nearly equal). Bounding the regime to the final ~28 minutes
    /// keeps its realistic late-game drama while capping its
    /// correlation budget.
    pub const SCORE_REACTION_FROM_MINUTE: u32 = 62;

    /// Match minute from which the **bench** may read the scoreline, which
    /// is earlier than the minute the eleven on the pitch may.
    ///
    /// The two are separate because the thing the gate above is rationing
    /// is not knowledge, it is *play*. What carried the draw surplus was
    /// trailing sides attacking harder and leading sides sitting deeper —
    /// eleven men changing how they play, on every tick, for the rest of
    /// the match. A manager deciding at 40' that his left winger is coming
    /// off at the interval changes nothing about how anybody plays until
    /// the change is made, and then it changes one player.
    ///
    /// It is also simply what managers do. Nobody watches his side go two
    /// down on the half-hour and thinks about the bench for the first time
    /// twenty-two minutes later; the whole point of a half-time change is
    /// that it was decided during the first half. Holding the substitution
    /// engine to the play gate meant the earliest a deficit could reach
    /// the bench at all was 62' — by which time the first change had
    /// usually already been made, blind, and the scoreline could no longer
    /// move it.
    ///
    /// Set at the half-hour: late enough that a fluke early goal does not
    /// empty a bench, early enough to reach the interval with a view.
    ///
    /// **Measured, both arms, 400 fixtures on `dev_match subs 400 14` —
    /// the only harness that fields a bench, so the only one that can see
    /// this at all:**
    ///
    /// | | gate at 62' | gate at 30' |
    /// |---|---|---|
    /// | team-goal rho | +0.301 | **+0.184** |
    /// | draws | 25.5% | 25.0% |
    /// | first change, mean | 64.0' | 59.6' |
    /// | first change, sd | 4.0 | **8.3** |
    /// | modal minute's share of the slot | 30% | 11% |
    /// | first change, behind vs ahead | 63.2' / 64.6' | **55.4' / 63.9'** |
    ///
    /// It does not spend correlation budget — it *returns* some, because a
    /// trailing side that changes earlier changes into a weaker player and
    /// the comeback dynamic that carries rho gets damped rather than fed.
    ///
    /// The other half of that table is the more interesting half, and it
    /// is why this constant is part of the timing fix rather than a
    /// concession to it: with the bench blind until 62', every side's read
    /// of its own match was *identical* — the clock and nothing else — so
    /// every side crossed the bar in the same minute. The old gate was
    /// itself one of the reasons substitutions all happened at once.
    pub const BENCH_SCORE_FROM_MINUTE: u32 = 30;

    /// Whether the substitution layer may read the real scoreline yet.
    ///
    /// Shares the `OF_SCORE_BLIND` control with
    /// [`behavioral_score_visible`](Self::behavioral_score_visible) so the
    /// score-blind A/B arm stays genuinely blind everywhere.
    pub fn bench_score_visible(&self) -> bool {
        if Self::score_blind() {
            return false;
        }
        (self.total_match_time / 60_000) as u32 >= Self::BENCH_SCORE_FROM_MINUTE
    }

    /// **How hard the score-reactive regime pulls, as one number.**
    ///
    /// The gate above decides WHEN teams start reading the scoreline;
    /// this decides HOW MUCH. It scales every continuous score-reactive
    /// magnitude in the engine — game-management intensity, the chasing
    /// risk lift, defensive urgency, the desperation conversion penalty —
    /// and shifts the coach's escalation ladder later in the match, so the
    /// whole regime moves together.
    ///
    /// # Why it has to be ONE number
    ///
    /// The regime is a dozen small channels that compound, and the
    /// previous calibration round tuned them one at a time: halving the
    /// instruction coefficients, the chasing-risk lift, the
    /// game-management risk slope and the line drop each moved the
    /// measured draw surplus by **under 10%**, because the other eleven
    /// channels were still at full strength. Nothing short of a common
    /// factor moves the regime as a whole.
    ///
    /// # What it was titrated against
    ///
    /// `OF_SCORE_BLIND` is the regime's own A/B and it lands on real
    /// football almost exactly — rho +0.03, variance/mean 1.12/1.05,
    /// 22.5% draws at equal strength — while the regime at full strength
    /// ran rho +0.36, variance/mean **0.65/0.84** and 37.5% draws. The
    /// under-dispersion is the part a viewer actually sees: a team that
    /// goes two up stops scoring, so 3-0 and 4-0 essentially never happen
    /// (1.2% and 0.5% against a real 5.5% and 2%) and every match ends
    /// 1-0, 1-1 or 2-1. Post-62' the leader was scoring at 0.55 goals/90
    /// against the trailer's 2.77 — a five-fold swing where real football
    /// has a mild one.
    ///
    /// # The sweep, 400 equal-strength matches a point
    ///
    /// | gain | goals | draws | correlation surplus | post-62' lead / trail |
    /// |---|---|---|---|---|
    /// | 1.0 | 2.81 | 36.8% | **+11.2pp** | 0.75 / 2.67 |
    /// | 0.6 | 2.70 | 35.0% | +8.6pp | 0.87 / 2.57 |
    /// | 0.4 | 2.47 | 33.0% | +5.2pp | 0.75 / 2.03 |
    /// | 0.2 | 2.50 | **27.0%** | **+0.4pp** | 1.10 / 1.80 |
    /// | blind | 2.62 | 22.5% | −2.3pp | 1.57 / 1.23 |
    ///
    /// The surplus is the metric to read — it is the observed draw rate
    /// against the rate the same two goal counts would produce if they
    /// were independent, so it isolates the correlation from the goal
    /// level. It falls cleanly with the gain where `rho` and the
    /// equalizer share are too noisy at n=400 to rank neighbouring points.
    ///
    /// 0.25 is the setting: real football's surplus is zero, and the
    /// regime keeps a quarter of its amplitude — a leader still slows
    /// down and a trailer still pushes, visibly, without the leader
    /// scoring at a third of the trailer's rate. The sweep above was run
    /// BEFORE the situational shape swap joined the gain, so a given
    /// number now carries slightly more of the regime than it did there.
    ///
    /// `OF_SCORE_GAIN` overrides it for a sweep; read once per process.
    /// Calibration infrastructure — do not remove.
    pub const SCORE_REACTION_GAIN: f32 = 0.25;

    /// The gain in force, honouring `OF_SCORE_GAIN` and `OF_SCORE_BLIND`.
    pub fn score_reaction_gain() -> f32 {
        use std::sync::OnceLock;
        static GAIN: OnceLock<f32> = OnceLock::new();
        *GAIN.get_or_init(|| {
            if Self::score_blind() {
                return 0.0;
            }
            std::env::var("OF_SCORE_GAIN")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .map(|v| v.clamp(0.0, 2.0))
                .unwrap_or(Self::SCORE_REACTION_GAIN)
        })
    }

    /// A progress threshold moved later in proportion as the gain falls.
    ///
    /// The coach's escalation ladder is DISCRETE — a trailing side is on
    /// `AllOutAttack` or it is not — so the gain cannot scale it. What it
    /// can do is decide how much of the match is spent past each rung:
    /// at gain 1.0 the ladder is untouched, at 0.5 a threshold sits
    /// halfway between where it was and the final whistle, and at 0 the
    /// coach never escalates at all.
    pub fn score_reaction_threshold(progress: f32) -> f32 {
        1.0 - (1.0 - progress) * Self::score_reaction_gain()
    }

    /// Diagnostic switch: with `OF_SUB_WALK_OFF` set, a substitution goes
    /// back to being the instant body swap it was before August 2026 — no
    /// waiting for the ball to go dead, no
    /// [`SubstitutionBreak`](super::super::touchline::SubstitutionBreak), the man
    /// coming on written straight onto his slot, and nothing marked on the
    /// timeline.
    ///
    /// It is the A/B control for the one thing the walk costs that nothing
    /// else in the engine can give back: **twelve seconds of match clock per
    /// STOPPAGE that a change is made at** — two to four a match once
    /// simultaneous changes are counted once, so 24-48 s, or 0.4-0.9% of the
    /// football in it. Whether that shows up in goals, shots or saves is a
    /// question about the whole population and cannot be answered from a
    /// diff — nor by checking out an older revision, for the reason
    /// [`shape_off`](Self::shape_off) gives.
    ///
    /// ⚠ **And it needs a very large n.** At 300 matches an arm the standard
    /// error on the difference is ~0.15 goals/match, which is a third of a
    /// percent of the goal rate away from being able to see an effect this
    /// size at all. Two arms that come out a tenth of a goal apart have said
    /// nothing.
    ///
    /// The off arm is the old engine exactly, including the medical pass's
    /// old scheduling, so a checksum comparison is meaningful. Same pattern
    /// and same purpose as the switches above; read once per process. Debug
    /// infrastructure — do not remove.
    pub fn sub_walk_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_SUB_WALK_OFF").is_ok())
    }

    /// The scoreline as BEHAVIOR is allowed to see it: 0-0 (level)
    /// before `SCORE_REACTION_FROM_MINUTE`, the real difference after.
    /// All tactical / coach / desperation score reads route through
    /// the three aggregation points that consume this.
    pub fn behavioral_score_visible(&self) -> bool {
        if Self::score_blind() {
            return false;
        }
        (self.total_match_time / 60_000) as u32 >= Self::SCORE_REACTION_FROM_MINUTE
    }
}
