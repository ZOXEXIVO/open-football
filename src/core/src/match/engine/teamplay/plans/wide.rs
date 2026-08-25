//! **Width** — the two men on the touchlines, and the run beyond them.
//!
//! # The problem this exists to solve
//!
//! [`AttackPlan`](super::attack::AttackPlan) assigns four box slots, a
//! short outlet, a runner in behind and a rest-defence group. Every one
//! of those jobs is *central*: the four slots sit within 7 m of the goal
//! centre, the outlet is simply the nearest man, and the far runner is
//! picked on pace alone. Nothing in the model ever told anybody to stand
//! on a touchline.
//!
//! The consequence is measurable, and it is exactly what a spectator
//! reports as "they never attack down the wings". `TeamShape` spreads the
//! side over a rectangle whose width is `team_width_target`-scaled, which
//! in a settled attack lands at ~43 m of a 68 m pitch — so the widest man
//! on the plan stands **12 m infield of the touchline**, and
//! `CrossModel::is_in_wide_position` (the entry guard on every crossing
//! state) needs the outer 20%. It was therefore false essentially always,
//! and the engine struck **2.2 open-play crosses per team per match
//! against a real 16-18**, with **5 cutbacks in 200 matches**.
//!
//! Two earlier attempts widened the *rectangle* and measured nothing:
//! occupied width stayed at 40.1 m, because a rectangle is a description
//! of where everybody is, not an instruction to anybody in particular.
//! Width is not an average. It is **two named players standing where
//! nobody else is**, and it has to be assigned the way a box slot is.
//!
//! # What is assigned
//!
//! * [`WidePlan::holder`] — one man per [`Flank`], holding the touchline
//!   for as long as we have the ball. He is what a back four has to
//!   answer: hold width and the line has to spread, and the spreading is
//!   what opens the half-spaces everything else in the attack uses.
//! * [`WidePlan::overlap_runner`] — the man licensed to run *beyond* the
//!   ball-side holder. In football that is normally the full-back behind
//!   him, and it is the commonest source of a byline cross in the game.
//!
//! Both are exclusive and both carry an incumbency bonus, for the same
//! reason the box slots do: a winger who is told to hold the touchline
//! and then told to tuck in every ten ticks holds nothing.
//!
//! # Why the far-side holder is deliberately NOT held back from the box
//!
//! The blind-side winger arriving at the far post is one of the most
//! productive runs in the game. So the far-side holder is left eligible
//! for the far-post box slot, and `TeamOperationsImpl::my_anchor` ranks a
//! box slot above a width anchor — meaning he holds the touchline until
//! the attack reaches the opposite flank and then attacks the back post,
//! with no extra machinery. Only the *ball-side* holder is reserved.

use crate::r#match::{MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// Which touchline something is nearest.
///
/// Pitch-absolute — decided by `y` alone, never by the team's attacking
/// direction. The half-time swap mirrors the pitch along the goal-to-goal
/// axis and rewrites every player's `side` and formation dot; a
/// team-relative flank would flip with it and every width assignment
/// would change hands at the interval for no footballing reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flank {
    /// The low-`y` touchline.
    Left,
    /// The high-`y` touchline.
    Right,
}

impl Flank {
    pub const ALL: [Flank; 2] = [Flank::Left, Flank::Right];

    /// Doubles as the array index, like `BoxSlot::index`.
    #[inline]
    pub fn index(self) -> usize {
        match self {
            Flank::Left => 0,
            Flank::Right => 1,
        }
    }

    /// Which flank a pitch `y` belongs to.
    #[inline]
    pub fn of(y: f32, field_height: f32) -> Self {
        if y < field_height * 0.5 {
            Flank::Left
        } else {
            Flank::Right
        }
    }

    #[inline]
    pub fn opposite(self) -> Self {
        match self {
            Flank::Left => Flank::Right,
            Flank::Right => Flank::Left,
        }
    }

    /// `-1` toward the low-`y` touchline, `+1` toward the high-`y` one.
    /// Lets a lateral offset be written once and used on either side.
    #[inline]
    pub fn sign(self) -> f32 {
        match self {
            Flank::Left => -1.0,
            Flank::Right => 1.0,
        }
    }

    /// The `y` a player holding this flank stands on, `inset` off the
    /// paint.
    #[inline]
    pub fn touchline_y(self, field_height: f32, inset: f32) -> f32 {
        match self {
            Flank::Left => inset,
            Flank::Right => field_height - inset,
        }
    }
}

/// How a side is holding its width this possession. Plain POD, embedded
/// in [`AttackPlan`](super::attack::AttackPlan) and copied with it.
#[derive(Debug, Clone, Copy)]
pub struct WidePlan {
    holders: [Option<u32>; 2],
    /// The man running beyond the ball-side holder, if the plan can
    /// afford to send him.
    pub overlap_runner: Option<u32>,
    /// The flank the ball is on right now — the side the overlap is on,
    /// and the side a switch is played away from.
    pub ball_flank: Flank,
    /// True whenever this side has the ball, including in build-up —
    /// deliberately a longer life than `AttackPlan::active`, which needs
    /// a phase that commits bodies forward. Holding a touchline commits
    /// nobody, and a full-back going wide to stretch the first press IS
    /// the build-up.
    pub active: bool,
}

impl WidePlan {
    pub const fn idle() -> Self {
        WidePlan {
            holders: [None; 2],
            overlap_runner: None,
            ball_flank: Flank::Left,
            active: false,
        }
    }

    /// How far off the touchline a width holder stands, in game units
    /// (1u = 0.125 m). 34u ≈ 4.3 m — a winger hugging the line without
    /// putting himself out of play every time he is found.
    pub const TOUCHLINE_INSET: f32 = 34.0;

    /// The man holding `flank`, if anybody is.
    #[inline]
    pub fn holder(&self, flank: Flank) -> Option<u32> {
        self.active.then(|| self.holders[flank.index()])?
    }

    /// The flank this player has been told to hold, if any.
    #[inline]
    pub fn flank_of(&self, player_id: u32) -> Option<Flank> {
        if !self.active {
            return None;
        }
        Flank::ALL
            .into_iter()
            .find(|f| self.holders[f.index()] == Some(player_id))
    }

    #[inline]
    pub fn is_overlap_runner(&self, player_id: u32) -> bool {
        self.active && self.overlap_runner == Some(player_id)
    }

    /// The holder on the side the ball is NOT on — the target of a
    /// switch, and the man who should be arriving at the far post.
    #[inline]
    pub fn far_holder(&self) -> Option<u32> {
        self.holder(self.ball_flank.opposite())
    }

    /// Where the man holding `flank` should be standing.
    ///
    /// Depth is whatever the block already wanted from him — holding
    /// width is a lateral instruction, not a licence to leave your line.
    /// The lateral is the point: he is pulled from his block anchor
    /// toward the touchline by `hug`, which is how committed the side is
    /// to playing wide.
    ///
    /// `hug` is a fraction rather than a fixed `y` so a compact,
    /// narrow-playing side still keeps its shape while a side told to
    /// stretch the pitch actually reaches the paint. At `hug = 1` he is
    /// [`Self::TOUCHLINE_INSET`] off the line.
    pub fn width_anchor(
        block_anchor: Vector3<f32>,
        flank: Flank,
        field_height: f32,
        hug: f32,
    ) -> Vector3<f32> {
        let touchline = flank.touchline_y(field_height, Self::TOUCHLINE_INSET);
        let hug = hug.clamp(0.0, 1.0);
        Vector3::new(
            block_anchor.x,
            block_anchor.y + (touchline - block_anchor.y) * hug,
            0.0,
        )
    }
}

/// One side's width assignment pass. Runs inside
/// [`AttackPlan::refresh`](super::attack::AttackPlan::refresh), after
/// rest defence and before the box slots, so a man told to hold the
/// touchline can never also be told to attack the near post.
pub(in crate::r#match::engine::teamplay::plans) struct WideBuilder<'a> {
    pub(in crate::r#match::engine::teamplay::plans) field: &'a MatchField,
    pub(in crate::r#match::engine::teamplay::plans) team_id: u32,
    pub(in crate::r#match::engine::teamplay::plans) side: PlayerSide,
    pub(in crate::r#match::engine::teamplay::plans) ball_pos: Vector3<f32>,
    /// The formation's own lateral extent, so "wide" means wide *for
    /// this shape* — a back three's wing-backs are its widest men even
    /// though a back four's full-backs start further out.
    pub(in crate::r#match::engine::teamplay::plans) min_lat: f32,
    pub(in crate::r#match::engine::teamplay::plans) lat_span: f32,
    /// …and its depth extent, so "advanced" means advanced for this
    /// shape. This is the front-line RANK the three-way position group
    /// never gave us: in a 4-4-2 the wide midfielders come out top of
    /// their flank, in a 4-2-3-1 the wide attacking midfielders do, and
    /// neither needs a per-formation special case.
    pub(in crate::r#match::engine::teamplay::plans) min_depth: f32,
    pub(in crate::r#match::engine::teamplay::plans) depth_span: f32,
}

impl WideBuilder<'_> {
    /// How far from the shape's lateral centre a formation slot has to
    /// be drawn before its occupant is a candidate to hold that flank.
    /// 0.5 of the half-width — the outer quarter of the shape on each
    /// side, which is the full-back / winger column in every formation
    /// the game ships.
    const WIDE_SLOT: f32 = 0.5;

    /// A slot this far up the shape or better is a *winger*; below it he
    /// is a full-back, and full-backs hold width only when there is
    /// nobody in front of them to do it. 0.45 puts the bar just below a
    /// 4-4-2's midfield band.
    const WINGER_DEPTH: f32 = 0.45;

    /// Incumbency, in the same units as the fit score. Matches the box
    /// slots' bonus: enough to stop two men trading the job every
    /// refresh, not enough to keep a man who has genuinely been
    /// displaced.
    const INCUMBENCY: f32 = 0.15;

    /// How far behind the ball an overlapping runner may start (~34 m).
    /// Beyond this he is not overlapping, he is chasing the move.
    const OVERLAP_REACH: f32 = 270.0;

    /// Assign the two touchlines.
    ///
    /// Runs FIRST in the plan, before rest defence: how wide a side
    /// plays is a structural decision it takes at the start of a
    /// possession, not what is left over once everybody else has a job.
    /// `eligible` answers "is this man still free".
    pub(in crate::r#match::engine::teamplay::plans) fn holders<F>(
        &self,
        plan: &mut WidePlan,
        previous: &WidePlan,
        mut eligible: F,
    ) where
        F: FnMut(u32) -> bool,
    {
        let field_height = self.field.size.height as f32;
        plan.ball_flank = Flank::of(self.ball_pos.y, field_height);

        // ── The touchlines ────────────────────────────────────────────
        for flank in Flank::ALL {
            let incumbent = previous.holder(flank);
            let best = self
                .outfielders()
                .filter(|p| eligible(p.id))
                .filter(|p| Flank::of(p.start_position.y, field_height) == flank)
                .filter_map(|p| {
                    let (lat, depth) = self.slot_fractions(p);
                    if (lat - 0.5).abs() * 2.0 < Self::WIDE_SLOT {
                        return None;
                    }
                    let mut score = self.width_fit(p, depth);
                    if incumbent == Some(p.id) {
                        score += Self::INCUMBENCY;
                    }
                    Some((p.id, score))
                })
                // Ties break by id so the assignment is reproducible.
                .max_by(|a, b| {
                    a.1.partial_cmp(&b.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(b.0.cmp(&a.0))
                });
            plan.holders[flank.index()] = best.map(|(id, _)| id);
        }

    }

    /// …and the run beyond the ball-side holder.
    ///
    /// Runs AFTER rest defence, so the man sent is by construction one
    /// the plan could afford to send — which is the whole safety
    /// argument, made once by the team instead of independently by every
    /// full-back's own eight-condition gate.
    ///
    /// Only on the ball's flank, and only from behind the man already
    /// there: an overlap is a run PAST somebody. Both halves matter —
    /// without the first, both full-backs vacate on the same possession
    /// and a single pass lands behind the team; without the second, the
    /// "overlap" is just a second player standing in the same channel.
    pub(in crate::r#match::engine::teamplay::plans) fn overlap<F>(
        &self,
        plan: &mut WidePlan,
        mut eligible: F,
    ) where
        F: FnMut(u32) -> bool,
    {
        let field_height = self.field.size.height as f32;
        let ball_flank = plan.ball_flank;
        let holder = plan.holder(ball_flank);
        let holder_depth = holder
            .and_then(|id| self.outfielders().find(|p| p.id == id))
            .map(|p| self.slot_fractions(p).1);
        let field_width = self.field.size.width as f32;
        let ball_progress = self.side.attacking_progress_x(self.ball_pos.x, field_width);
        plan.overlap_runner = self
            .outfielders()
            .filter(|p| eligible(p.id))
            .filter(|p| Some(p.id) != holder)
            .filter(|p| Flank::of(p.start_position.y, field_height) == ball_flank)
            .filter_map(|p| {
                let (lat, depth) = self.slot_fractions(p);
                if (lat - 0.5).abs() * 2.0 < Self::WIDE_SLOT {
                    return None;
                }
                // Behind the man he is overlapping. With nobody holding
                // the flank at all, any wide man on it may go — that is a
                // full-back attacking an empty channel, which is correct
                // football and the main way a back three creates width.
                if holder_depth.is_some_and(|h| depth >= h - 0.05) {
                    return None;
                }
                // He has to be able to get there: an overlap that starts
                // 40 m behind the ball arrives after the move is over.
                let progress = self.side.attacking_progress_x(p.position.x, field_width);
                if ball_progress - progress > Self::OVERLAP_REACH / field_width {
                    return None;
                }
                Some((p.id, self.overlap_fit(p)))
            })
            .max_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.cmp(&a.0))
            })
            .map(|(id, _)| id);
    }

    /// (lateral, depth) of this player's formation slot as fractions of
    /// the shape's own bounding box — the same normalisation
    /// [`ShapeBuilder`](super::block::ShapeBuilder) projects anchors
    /// with, so "wide" and "advanced" mean the same thing to both.
    fn slot_fractions(&self, p: &MatchPlayer) -> (f32, f32) {
        let field_width = self.field.size.width as f32;
        let lat = (p.start_position.y - self.min_lat) / self.lat_span;
        let depth = (self
            .side
            .attacking_progress_x(p.start_position.x, field_width)
            - self.min_depth)
            / self.depth_span;
        (lat.clamp(0.0, 1.0), depth.clamp(0.0, 1.0))
    }

    /// How well this player suits holding a touchline.
    ///
    /// The slot decides most of it — width is a positional job, and the
    /// man drawn highest in the wide column is the one football gives it
    /// to. Attributes decide how much the occupancy is worth once the
    /// ball arrives, and they are the attributes a wide player is
    /// actually judged on: can he deliver, can he beat the full-back,
    /// can he get to the byline and back.
    fn width_fit(&self, p: &MatchPlayer, depth: f32) -> f32 {
        let s = &p.skills;
        let delivery = (s.technical.crossing / 20.0) * 0.55 + (s.technical.technique / 20.0) * 0.45;
        let one_v_one =
            (s.technical.dribbling / 20.0) * 0.6 + (s.physical.acceleration / 20.0) * 0.4;
        let engine = (s.physical.stamina / 20.0) * 0.5 + (s.physical.pace / 20.0) * 0.5;

        // A winger is preferred over the full-back behind him, smoothly
        // rather than by a cut-off: a 4-3-3's front three win their
        // flanks outright, a 3-5-2's wing-backs win theirs because there
        // is nobody in front of them, and a 4-4-2's wide midfielder
        // edges his full-back by a comfortable margin.
        let advancement =
            ((depth - Self::WINGER_DEPTH) / (1.0 - Self::WINGER_DEPTH)).clamp(0.0, 1.0);

        advancement * 0.45 + delivery * 0.20 + one_v_one * 0.20 + engine * 0.15
    }

    /// How well this player suits the run beyond the holder. An overlap
    /// is 40 m at pace and then a delivery, so it is legs first and
    /// technique second — which is why it is a full-back's run.
    fn overlap_fit(&self, p: &MatchPlayer) -> f32 {
        let s = &p.skills;
        let legs = (s.physical.stamina / 20.0) * 0.35
            + (s.physical.pace / 20.0) * 0.35
            + (s.mental.work_rate / 20.0) * 0.30;
        let end_product =
            (s.technical.crossing / 20.0) * 0.6 + (s.mental.off_the_ball / 20.0) * 0.4;
        legs * 0.65 + end_product * 0.35
    }

    fn outfielders(&self) -> impl Iterator<Item = &MatchPlayer> {
        self.field
            .players
            .iter()
            .filter(move |p| p.team_id == self.team_id && !p.is_sent_off)
            .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flank_is_pitch_absolute() {
        let h = 545.0;
        assert_eq!(Flank::of(10.0, h), Flank::Left);
        assert_eq!(Flank::of(535.0, h), Flank::Right);
        assert_eq!(Flank::Left.opposite(), Flank::Right);
        assert_ne!(Flank::Left.index(), Flank::Right.index());
    }

    /// The whole point of the assignment: a man holding width must end
    /// up somewhere `CrossModel::is_in_wide_position` calls wide — the
    /// outer 20% of the pitch. A rectangle-derived anchor never did.
    #[test]
    fn a_full_hug_reaches_the_crossing_channel() {
        let h = 545.0f32;
        let margin = h * 0.2;
        for flank in Flank::ALL {
            // A block anchor 16 m infield of the touchline, which is
            // roughly where the widest man on the plan measured.
            let block = Vector3::new(400.0, h * 0.5 + flank.sign() * 130.0, 0.0);
            let anchor = WidePlan::width_anchor(block, flank, h, 1.0);
            assert!(
                anchor.y < margin || anchor.y > h - margin,
                "{flank:?} full hug landed at {} — still not wide",
                anchor.y
            );
            // …and it must stay on the pitch.
            assert!(anchor.y > 0.0 && anchor.y < h);
        }
    }

    /// No hug means no change — a side told to play narrow keeps its
    /// block, so a zero-width tactic is bit-identical to the engine
    /// before this module existed.
    #[test]
    fn no_hug_is_the_block_anchor() {
        let block = Vector3::new(400.0, 200.0, 0.0);
        let anchor = WidePlan::width_anchor(block, Flank::Left, 545.0, 0.0);
        assert!((anchor - block).magnitude() < 1e-3);
    }

    #[test]
    fn an_idle_plan_holds_nobody() {
        let plan = WidePlan::idle();
        assert!(plan.holder(Flank::Left).is_none());
        assert!(plan.holder(Flank::Right).is_none());
        assert!(plan.flank_of(7).is_none());
        assert!(!plan.is_overlap_runner(7));
    }
}
