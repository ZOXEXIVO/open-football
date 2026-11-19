//! **The score-reactive regime**, and the A/B switches that measure it.
//!
//! How hard the score pulls on play: each coach's stance — how much his
//! side should commit, from what the next goal is worth — reaches the
//! plan's levers as its move since kickoff, scaled by
//! [`SCORE_REACTION_GAIN`](MatchContext::SCORE_REACTION_GAIN)
//! ([`stance_gain`](MatchContext::stance_gain)). Also whether anything may
//! see the score at all (`OF_SCORE_BLIND`), and the env-var switches that
//! turn whole engine layers off so a calibration run can attribute an
//! effect to one of them.
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

    /// A/B control: with `OF_TEAM_STANCE=off` each coach still reads his
    /// stance, but it moves none of the levers. Read once per process —
    /// debug infrastructure, do not remove.
    pub fn team_stance_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_TEAM_STANCE").is_ok_and(|v| v == "off"))
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

    /// Diagnostic switch: with `OF_BOX_DEFENCE_OFF` set, the whole
    /// box-defending layer reverts to what it was before 2026-09-06:
    ///
    /// * `Ball::try_block_pass` never runs, so the only thing that can
    ///   touch a pass is `try_intercept`'s 69 cm standing radius;
    /// * every one of [`KeeperVoice`]'s three calls returns neutral, so
    ///   the goalkeeper organises nothing in front of him;
    /// * `TackleDecision::BOX_RESTRAINT` goes back to 0.32.
    ///
    /// The A/B control for all three at once. They are deliberately on
    /// ONE switch: each is small on its own and they are meant to be
    /// judged as one answer to one report, and a per-channel arm would
    /// tempt exactly the sort of single-lever tuning the calibration
    /// memos warn about. Read once per process. Same pattern and purpose
    /// as [`shape_off`](Self::shape_off) — debug infrastructure, do not
    /// remove.
    pub fn box_defence_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_BOX_DEFENCE_OFF").is_ok())
    }

    /// Diagnostic switch: with `OF_CHARGE_DOWN_OFF` set, a pass block is a
    /// lunge only — no man is ever too late to react
    /// (`BlockGeometry::reaction` reads zero) — and a wide man refuses to
    /// cross past a man stood in front of him, as he did before the
    /// crosser priced the block. The A/B control for the charge-down, one
    /// switch for both halves because the striker's price and the
    /// contest's roll are one model. Read once per process — debug
    /// infrastructure, do not remove.
    pub fn charge_down_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CHARGE_DOWN_OFF").is_ok())
    }

    /// Diagnostic switch: with `OF_TAKEOVER_DUEL=off`, an opponent at a
    /// carrier's feet takes the ball by comparison, as before, instead of
    /// by a challenge decided on the block tackle's odds. The A/B control
    /// for `BlockChallenge` at the ball. Read once per process — debug
    /// infrastructure, do not remove.
    pub fn takeover_duel_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_TAKEOVER_DUEL").is_ok_and(|v| v == "off"))
    }

    /// Diagnostic switch: with `OF_MIDFIELDER_DUEL=off`, a midfielder the
    /// carrier has gone past has no challenge left to him, as before the
    /// duel was split into closing, containing and challenging. The A/B
    /// control for the midfielder's duel. Read once per process.
    pub fn midfielder_duel_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_MIDFIELDER_DUEL").is_ok_and(|v| v == "off"))
    }

    /// Diagnostic switch: with `OF_MIDFIELDER_MARK_PACE=off`, a midfielder
    /// marking a man arrives flat out at his point and stops on it, as before
    /// he moved with his man. The A/B control for the marker's pace. Read
    /// once per process.
    pub fn midfielder_mark_pace_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_MIDFIELDER_MARK_PACE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_CROSS_OWN_GOAL=off` a goal is credited as it
    /// was before the football rule: a defending touch of under 0.3 s hands
    /// the goal to the most recent opposing passer. Read once per process —
    /// debug infrastructure, do not remove.
    pub fn cross_own_goal_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_OWN_GOAL").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_CROSS_HEAD_ARRIVAL=off` a won delivery arrives
    /// within a boot's reach of its winner and a midfielder or a defender
    /// plays the header he won by his role's old rule, as before. Read once
    /// per process — debug infrastructure, do not remove.
    pub fn cross_head_arrival_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_HEAD_ARRIVAL").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_CROSS_BOOKING=off` a pass is booked as a cross
    /// by the old geometry (from a wide channel of the final third to the
    /// box or within 10 u of its edge) and its completion is credited when
    /// a team-mate receives it, as before. Read once per process — debug
    /// infrastructure, do not remove.
    pub fn cross_booking_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_BOOKING").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_CROSS_BYLINE=off` the wide carrier's drive ends
    /// at the corner of the penalty area, the wide area stops at the outer
    /// fifth of the pitch, a ball from the byline is typed by depth alone,
    /// and a full-back carries at the goal, as before. Read once per
    /// process — debug infrastructure, do not remove.
    pub fn cross_byline_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_BYLINE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_CROSS_TOUCHLINE=off` nobody pinned at his own
    /// touchline puts the ball out of play on purpose, as before. Read once
    /// per process — debug infrastructure, do not remove.
    pub fn cross_touchline_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_TOUCHLINE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_FORWARD_RUN_PACE=off` the runner in behind
    /// sprints along the line he waits on, as before he kept station on it.
    /// Read once per process — debug infrastructure, do not remove.
    pub fn forward_run_pace_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_FORWARD_RUN_PACE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_FORWARD_BOX_PACE=off` a forward attacking a
    /// place in the box declares the `Moderate` tier, as before. Read once
    /// per process — debug infrastructure, do not remove.
    pub fn forward_box_pace_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_FORWARD_BOX_PACE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_FORWARD_DUEL=off` a containing forward is not
    /// capped below a sprint, his challenge does not read how he arrived,
    /// and a forward the carrier has gone past has no challenge left to him.
    /// Read once per process — debug infrastructure, do not remove.
    pub fn forward_duel_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_FORWARD_DUEL").is_ok_and(|v| v == "off"))
    }

    /// Diagnostic switch: with `OF_SECOND_BALL_DUEL=off`, a ball dropping
    /// with nobody named for it is not contested: whoever reaches it plays
    /// it, as before. The A/B control for the second ball. Read once per
    /// process.
    pub fn second_ball_duel_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_SECOND_BALL_DUEL").is_ok_and(|v| v == "off"))
    }

    /// Diagnostic switch: with `OF_CROSS_LINE_OFF` set, every man closing
    /// the ball carrier steers for the point he did before
    /// `ClosingPoint` — on the carrier's line to the goal centre, with
    /// each state's old stand-off. The A/B control for the closer's
    /// position on the crossing line. Read once per process — debug
    /// infrastructure, do not remove.
    pub fn cross_line_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_CROSS_LINE_OFF").is_ok())
    }

    /// Diagnostic switch: with `OF_BLOCK_CONTACT_FLAT` set, both block
    /// channels resolve their deferred contact the way they did before
    /// 2026-09-06 — on the distance across the grass alone, with no
    /// height asked and no commitment ever released.
    ///
    /// The A/B control for [`PlayerReach::block_contact`](crate::r#match::engine::ball::ball::PlayerReach::block_contact).
    /// It exists because the fix is a geometry change on a channel that
    /// was itself calibrated three weeks ago: "did the blocks that
    /// stopped happening cost anything" is a population question about
    /// goals, shots and pass accuracy, and the only honest way to answer
    /// it is the same binary run twice. Read once per process. Debug
    /// infrastructure — do not remove.
    pub fn block_contact_flat() -> bool {
        use std::sync::OnceLock;
        static FLAT: OnceLock<bool> = OnceLock::new();
        *FLAT.get_or_init(|| std::env::var("OF_BLOCK_CONTACT_FLAT").is_ok())
    }

    /// Diagnostic switch: with `OF_AERIAL_ARRIVAL_FLAT` set,
    /// `Ball::tick_aerial_delivery` applies a decided contest's outcome the
    /// way it did before 2026-09-06 — the moment the ball reaches the SPOT
    /// it was aimed at, with no question about whether the man who won it
    /// got there.
    ///
    /// The A/B control for the header landing on a body. It moves the
    /// aerial supply — contests, headers on goal, corners — so "did the
    /// deliveries this drops cost anything" is a population question, and
    /// the only honest way to answer it is the same binary run twice. Read
    /// once per process. Debug infrastructure — do not remove.
    pub fn aerial_arrival_flat() -> bool {
        use std::sync::OnceLock;
        static FLAT: OnceLock<bool> = OnceLock::new();
        *FLAT.get_or_init(|| std::env::var("OF_AERIAL_ARRIVAL_FLAT").is_ok())
    }

    /// Diagnostic switch: with `OF_DELIVERY_RELAUNCH` set,
    /// `deliver_to_winner` solves a fresh ballistic arc for EVERY armed
    /// delivery, the way it did before 2026-09-07 — including for a ball
    /// already descending through the heading band a stride from the man,
    /// where the solved arc is a metres-high pop written onto a ball
    /// nobody touched.
    ///
    /// The A/B control for the cross contest riding the flight the ball
    /// already has. It changes how many deliveries reach their man, which
    /// is a population question about headers, corners and goals — so the
    /// only honest way to answer it is the same binary run twice. Read
    /// once per process. Debug infrastructure — do not remove.
    pub fn delivery_relaunch_flat() -> bool {
        use std::sync::OnceLock;
        static FLAT: OnceLock<bool> = OnceLock::new();
        *FLAT.get_or_init(|| std::env::var("OF_DELIVERY_RELAUNCH").is_ok())
    }

    /// Diagnostic switch: with `OF_CROSS_CLEAR_FLAT` set,
    /// `resolve_cross_contest` applies its DEFENSIVE outcomes the way it
    /// did before 2026-09-07 — the clearance and the hook written onto the
    /// ball wherever it happens to be, with no defender required to be
    /// near it and, when none is contesting at all, no defender required
    /// to exist.
    ///
    /// The A/B control for the clearance happening at a man. It is the
    /// majority outcome of every open-play cross, so what it does to
    /// possession, second balls and the corner supply is a population
    /// question. Read once per process. Debug infrastructure — do not
    /// remove.
    pub fn cross_clear_flat() -> bool {
        use std::sync::OnceLock;
        static FLAT: OnceLock<bool> = OnceLock::new();
        *FLAT.get_or_init(|| std::env::var("OF_CROSS_CLEAR_FLAT").is_ok())
    }

    /// Diagnostic switch: with `OF_HOME_FLAT` set, EVERY home-advantage
    /// channel is neutral — `crowd_arousal` is 1.0 for both sides, the
    /// tactical press/risk/tempo lift is not applied, and the referee's
    /// marginal-call bias is zeroed.
    ///
    /// This is the decomposition arm, and it exists because the home
    /// split is the one distribution in the engine that has been
    /// re-titrated three times without ever being decomposed. Home wins
    /// measured 60.5% against a real 45%, and there are two very
    /// different explanations: the crowd constant is too large, or
    /// something in the engine is not side-swap-aware and hands the home
    /// side an edge no constant controls. With every declared channel off
    /// the remaining split answers that question directly — anything
    /// materially above 50/25/25 with this set is a bug, not a constant.
    ///
    /// Read once per process. Debug infrastructure — do not remove.
    pub fn home_flat() -> bool {
        use std::sync::OnceLock;
        static FLAT: OnceLock<bool> = OnceLock::new();
        *FLAT.get_or_init(|| std::env::var("OF_HOME_FLAT").is_ok())
    }

    /// Titration override for the two `crowd_arousal` constants:
    /// `OF_HOME_AROUSAL=<home>,<away>`, e.g. `0.06,0.035`.
    ///
    /// This constant has now needed re-titrating three times, always for
    /// the same reason — it is not a property of home advantage, it is a
    /// property of how strongly THIS engine converts an effective-skill
    /// edge into goals, so it moves whenever the duel model does. Each
    /// re-titration previously cost a rebuild per arm. It does not any
    /// more.
    ///
    /// Read once per process. Debug infrastructure — do not remove.
    pub fn home_arousal_override() -> Option<(f32, f32)> {
        use std::sync::OnceLock;
        static OVERRIDE: OnceLock<Option<(f32, f32)>> = OnceLock::new();
        *OVERRIDE.get_or_init(|| {
            let raw = std::env::var("OF_HOME_AROUSAL").ok()?;
            let (h, a) = raw.split_once(',')?;
            Some((h.trim().parse().ok()?, a.trim().parse().ok()?))
        })
    }

    /// Diagnostic switch: with `OF_BLOCK_RIGID` set, the positional block
    /// goes back to keeping its whole rectangle on the pitch by clamping
    /// its own CENTRE, instead of sliding freely and narrowing to fit.
    ///
    /// This is the A/B control for the lateral half of the shape layer.
    /// The old clamp was the binding constraint on the entire side's
    /// sideways behaviour and nothing else in the engine said so: on a
    /// 68 m pitch a 47 m block left the centre ±8.9 m of travel, so a
    /// `SLIDE_DEFENDING` of 0.55 arrived at the players as about 0.30.
    /// Measured with the thermal map (`dev_match heat`, 10 × 90 min,
    /// level 14): the ball's own lateral spread is sd 15.3 m and the
    /// centre-backs' was sd 4.6 m.
    ///
    /// Read once per process. Same pattern and purpose as
    /// [`shape_off`](Self::shape_off) — its effect reaches every player
    /// on every tick, so what it costs cannot be read off the diff.
    /// Debug infrastructure — do not remove.
    pub fn block_rigid() -> bool {
        use std::sync::OnceLock;
        static RIGID: OnceLock<bool> = OnceLock::new();
        *RIGID.get_or_init(|| std::env::var("OF_BLOCK_RIGID").is_ok() || Self::shape_fix_off())
    }

    /// Diagnostic switch: with `OF_SHAPE_FIX_OFF` set, the whole
    /// 2026-08-30 positional pass reverts and the engine behaves as it did
    /// before it — the back line's width reference goes back to the
    /// kick-off slot, the block clamps its centre again, a width holder
    /// takes his depth straight from the block, a full-back needs
    /// `rest_defense_count + 1` behind the ball and the ball on his own
    /// half, the keeper's depth curve is squared again, and every
    /// midfielder inside 20 m offers the same pass.
    ///
    /// One switch rather than six because the seven changes were made
    /// together and their effects interleave: the thermal map's
    /// possession split changed at the same time, and separating "the
    /// engine moved" from "the measurement moved" needs both arms read by
    /// the same census in the same run of the harness. Read once per
    /// process. Same pattern and purpose as
    /// [`shape_off`](Self::shape_off) — debug infrastructure, do not
    /// remove.
    pub fn shape_fix_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_SHAPE_FIX_OFF").is_ok())
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

    /// Whether the substitution layer may read the real scoreline: from the
    /// goal, unless `OF_SCORE_BLIND` keeps every reader of it blind.
    pub fn bench_score_visible(&self) -> bool {
        !Self::score_blind()
    }

    /// How hard the score moves play, as one number: the share of each
    /// coach's stance that reaches the levers and the plans he changes to.
    ///
    /// Re-fitted with the score open from the first minute, 300 equal
    /// matches a point: 0.25 holds the goal correlation (+0.05), the draw
    /// surplus (+0.5 pp) and the leader/trailer rate ratios (0.87 before
    /// 62', 1.06 after); 0.5 already carries a +5.2 pp surplus and 1.0 the
    /// old draw machine (rho +0.38). `OF_SCORE_GAIN` overrides it for a
    /// sweep; read once per process.
    pub const SCORE_REACTION_GAIN: f32 = 0.25;

    /// A/B control: with `OF_TEAM_ADHERENCE=off` every man reads the
    /// neutral adherence, so the shape pull, the press and the jobs treat
    /// the whole squad alike. Read once per process.
    pub fn adherence_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_TEAM_ADHERENCE").is_ok_and(|v| v == "off"))
    }

    /// A/B control: with `OF_TEAM_TALK=off` the coach still chooses and
    /// records his half-time talk, and nobody reacts to it. Read once per
    /// process.
    pub fn team_talk_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_TEAM_TALK").is_ok_and(|v| v == "off"))
    }

    /// The gain the stance reaches the plan and its levers through: the
    /// regime's, or none under `OF_TEAM_STANCE=off`.
    pub fn stance_gain() -> f32 {
        if Self::team_stance_off() {
            0.0
        } else {
            Self::score_reaction_gain()
        }
    }

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

    /// Whether behaviour may see the real scoreline: from the goal, unless
    /// `OF_SCORE_BLIND` keeps every reader of it level.
    pub fn behavioral_score_visible(&self) -> bool {
        !Self::score_blind()
    }
}
