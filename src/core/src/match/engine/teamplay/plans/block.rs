//! **The rectangle itself** — the geometry the block is made of, and the
//! pass that rebuilds it each refresh.
//!
//! The constants below are the block: how long it is compact and
//! stretched, how wide narrow and full, how far off the ball it stands,
//! how far it slides toward the ball's flank, and how fast it may travel.
//! `ShapeBuilder` projects each player's kickoff pattern into whatever
//! rectangle those constants and the tactical state produce, and writes
//! the anchors onto [`TeamShape`].
//!
//! See [`shape`](super::shape) for why the block exists at all.

use crate::r#match::engine::teamplay::plans::shape::{MAX_ON_PITCH, TeamShape};
use crate::r#match::engine::teamplay::tactical::TeamTacticalState;
use crate::r#match::{MatchField, PlayerSide};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::ShapeCensus;
use nalgebra::Vector3;

/// Block length in game units (1u = 0.125 m) at full compactness and at
/// full stretch. 220u = 27.5 m for a compact low block, 330u = 41 m for a
/// side committed to an attack.
///
/// **The real-football reference of 35-45 m is an OBSERVED span, and the
/// observed span is always longer than the planned one.** Two measured
/// effects, both of which are themselves realistic:
///
/// * a systematic ladder — defenders settle ~4 m deeper than their
///   anchors and forwards ~9 m higher, because defending pulls one way
///   and attacking the other;
/// * scatter — each player carries ~12 m of ordinary lag in an arbitrary
///   direction, and the max-minus-min over ten of them adds most of that
///   again.
///
/// Together they add ~20 m, so a plan drawn at the real reference
/// measures ~58 m. The plan is therefore sized so the thing the reference
/// actually describes comes out right.
/// (Sized against the OBSERVED span, not the planned one — see below.)
pub(in crate::r#match::engine::teamplay::plans) const LENGTH_COMPACT: f32 = 220.0;
const LENGTH_STRETCHED: f32 = 330.0;

/// Block width at minimum and maximum `team_width_target`. 300u = 37.5 m
/// (a narrow low block), 450u = 56 m of a 68 m pitch (full width, wingers
/// on the touchline).
///
/// Raising the maximum to 510u (64 m) was tried and REVERTED: the measured
/// block width did not move off 40.1 m, because players do not occupy
/// their lateral anchors any more than they occupy their depth ones — the
/// same occupancy gap the length constants document. Widening the PLAN
/// achieves nothing on its own, and it matters beyond shape:
/// `CrossModel::is_in_wide_position` needs the outer 20% of the pitch
/// before anyone will cross, so a side that never reaches the flank never
/// crosses from open play (~2-3 a match against a real ~30).
pub(in crate::r#match::engine::teamplay::plans) const WIDTH_NARROW: f32 = 300.0;
const WIDTH_WIDE: f32 = 450.0;

/// How far goal-side of the ball the DEEPEST line sits, before the phase
/// cap applies. 200u = 25 m at a passive press, 130u = 16 m when hunting.
const STANDOFF_PASSIVE: f32 = 200.0;
const STANDOFF_AGGRESSIVE: f32 = 130.0;

/// How much of the ball's lateral displacement the block follows.
/// A defending side slides hard toward the ball; an attacking side keeps
/// more width to switch the play.
const SLIDE_DEFENDING: f32 = 0.55;
const SLIDE_ATTACKING: f32 = 0.30;

/// The block's rear never sits closer to the goal line than this, or the
/// whole team ends up inside its own six-yard box.
const REAR_FLOOR_PROGRESS: f32 = 0.055;

/// How far ahead of the ball the FRONT of an attacking block sits.
/// 130u ≈ 16 m — the striker plays on the shoulder of the last defender,
/// which is a stride or two beyond the man carrying the ball, not a third
/// of a pitch.
const FRONT_LEAD: f32 = 130.0;

/// …and never past this, or the front rank ends up standing on the goal
/// line. 0.93 of the pitch is ~7 m out, the near edge of the six-yard box.
const FRONT_CEILING: f32 = 0.93;

/// How high a back line may push while its own side has the ball. 0.62 is
/// ~10 m into the opponent half — a dominant side squeezing the game, and
/// about as far as a back four goes before it is a different tactic
/// rather than a compact shape.
const REAR_ATTACK_CEILING: f32 = 0.62;

/// Keep anchors this far off the touchlines and goal lines.
const PITCH_MARGIN: f32 = 14.0;

/// How fast the block itself may travel, in units per tick. 0.9 u/tick is
/// 9 u/s ≈ 1.1 m per 100 ms — a shade over 11 m/s, so the shape moves at
/// about the pace of a sprinting defensive line and no faster.
///
/// Without this the block is a pure function of the ball, and the ball
/// crosses 40 m in a single long pass: the whole shape teleported, every
/// player was instantly 40 m out of position, and they spent the next
/// several seconds chasing a plan they could never have reached. The
/// measured signature was a 38 m PLANNED block occupied over 58 m, with
/// the worst-placed player a mean 27 m from his anchor — a lag that is
/// not a discipline failure but an unreachable target.
///
/// A real team's shape is limited by how fast its players run, so this is
/// the constraint restored rather than a smoothing filter bolted on.
pub(in crate::r#match::engine::teamplay::plans) const BLOCK_SPEED: f32 = 0.9;

/// A jump larger than this is a restart, not movement — kickoff, the
/// half-time swap, a goal reset, a set piece on the far side. Snap
/// instead of walking the block across the pitch.
const BLOCK_SNAP: f32 = 240.0;

pub(in crate::r#match::engine::teamplay::plans) struct ShapeBuilder<'a> {
    pub(in crate::r#match::engine::teamplay::plans) field: &'a MatchField,
    pub(in crate::r#match::engine::teamplay::plans) team_id: u32,
    pub(in crate::r#match::engine::teamplay::plans) tactical: &'a TeamTacticalState,
    /// How far the block may move this refresh — `BLOCK_SPEED` scaled by
    /// however many ticks have elapsed since the last one.
    pub(in crate::r#match::engine::teamplay::plans) max_step: f32,
}

impl ShapeBuilder<'_> {
    /// Walk `previous` toward `target` by at most `max_step`, snapping
    /// outright when the gap is a restart rather than movement. `None`
    /// (no previous value) also snaps — that is the first refresh.
    fn approach(previous: Option<f32>, target: f32, max_step: f32, snap: f32) -> f32 {
        let Some(previous) = previous else {
            return target;
        };
        let delta = target - previous;
        if delta.abs() > snap || delta.abs() <= max_step {
            return target;
        }
        previous + delta.signum() * max_step
    }

    pub(in crate::r#match::engine::teamplay::plans) fn build(&self, shape: &mut TeamShape) {
        let field_width = self.field.size.width as f32;
        let field_height = self.field.size.height as f32;

        let Some(side) = self
            .field
            .players
            .iter()
            .find(|p| p.team_id == self.team_id && !p.is_sent_off)
            .and_then(|p| p.side)
        else {
            shape.active = false;
            return;
        };

        // ── The formation's own bounding box ─────────────────────────
        // The kickoff dots are a PATTERN, not a set of destinations. Read
        // their extent once so each player can be described as a fraction
        // of the shape he was drawn into, independent of which formation
        // it was or which end the team is defending. The keeper is
        // excluded: he is 40 m behind the deepest outfielder and would
        // absorb most of the depth range on his own.
        let mut min_depth = f32::MAX;
        let mut max_depth = f32::MIN;
        let mut min_lat = f32::MAX;
        let mut max_lat = f32::MIN;
        for p in self.outfielders() {
            let depth = side.attacking_progress_x(p.start_position.x, field_width);
            min_depth = min_depth.min(depth);
            max_depth = max_depth.max(depth);
            min_lat = min_lat.min(p.start_position.y);
            max_lat = max_lat.max(p.start_position.y);
        }
        if min_depth > max_depth {
            shape.active = false;
            return;
        }
        let depth_span = (max_depth - min_depth).max(0.01);
        let lat_span = (max_lat - min_lat).max(1.0);

        // ── Where the block is ───────────────────────────────────────
        let ball = self.field.ball.position;
        let ball_progress = side.attacking_progress_x(ball.x, field_width);

        // The deepest line sits goal-side of the ball by a standoff that
        // shrinks as the side presses harder, then is capped by the
        // phase's line height — so a high-pressing side still cannot hold
        // a line above where the coach's shape puts it, and a low block
        // still drops when the ball arrives even if the phase says
        // MidBlock. Taking the MINIMUM of the two is what makes the line
        // ball-reactive: `defensive_line_x` alone is a constant per phase
        // and gave the same height whether the ball was on the halfway
        // line or in the six-yard box.
        let standoff = STANDOFF_PASSIVE
            - (STANDOFF_PASSIVE - STANDOFF_AGGRESSIVE)
                * self.tactical.press_intensity.clamp(0.0, 1.0);
        let phase_cap = side.attacking_progress_x(self.tactical.defensive_line_x, field_width);

        // ── How big it is ────────────────────────────────────────────
        let compact = self.tactical.compactness_target.clamp(0.0, 1.0);
        let length = LENGTH_STRETCHED - (LENGTH_STRETCHED - LENGTH_COMPACT) * compact;
        let length_progress = length / field_width;

        // ── WHICH END the block hangs from ───────────────────────────
        //
        // A block is not always positioned by its back line. Defending,
        // the back line is the reference and the strikers end up wherever
        // the block's length leaves them. **Attacking, it is the other way
        // round**: the front line is at the opponent's box, and the back
        // line pushes up to whatever the length allows — which is exactly
        // how a team that attacks stays compact instead of stretching.
        //
        // Hanging both cases off the rear was measurably wrong, and it is
        // worth stating why, because the failure is invisible in the plan
        // and only shows in the players. In the `Attack` phase the rear is
        // capped at 0.55 of the pitch, so a side attacking with the ball
        // at 0.85 got a rear of 0.55 and a front of 0.94 — a front rank
        // parked 26 m from the goal it is attacking. The forwards
        // (correctly) went to the box anyway and sat a mean **+11.7 m in
        // front of their own anchors**, which is most of the block's
        // measured over-length. The plan was fighting the one thing the
        // players were right about.
        let rear_target = if self.tactical.in_possession {
            // Front line leads the ball into the box; the block hangs
            // back from it.
            let front = (ball_progress + FRONT_LEAD / field_width).min(FRONT_CEILING);
            (front - length_progress)
                // Capped by how high a back line may realistically push
                // while its own side attacks — NOT by `defensive_line_x`.
                // That is a DEFENDING line height (0.55 in the `Attack`
                // phase), and applying it here re-imposed exactly the
                // failure this branch exists to remove: the rear pinned
                // to 0.55, the front landed 30 m from the goal being
                // attacked, and the forwards stood 12 m beyond it. Who
                // stays home during an attack is a squad decision and is
                // already made, by name, in `AttackPlan::rest_defence`.
                .min(REAR_ATTACK_CEILING)
                .max(REAR_FLOOR_PROGRESS)
        } else {
            // Back line sits goal-side of the ball by the standoff, and
            // the phase caps how high that may be. Taking the MINIMUM is
            // what makes the line ball-reactive: `defensive_line_x` alone
            // is a constant per phase and gave the same height whether the
            // ball was on the halfway line or in the six-yard box.
            (ball_progress - standoff / field_width)
                .min(phase_cap)
                .max(REAR_FLOOR_PROGRESS)
        };
        // …and the block walks there at running pace rather than
        // teleporting. See `BLOCK_SPEED`.
        let rear = Self::approach(
            shape.active.then_some(shape.rear_progress),
            rear_target,
            self.max_step / field_width,
            BLOCK_SNAP / field_width,
        );

        // The block cannot extend through the opponent's goal. Without
        // this a high line plus a stretched shape puts the front rank
        // behind the goal line, and every forward's anchor pins to the
        // same clamped strip — the clustering this module exists to end.
        let room = (1.0 - rear) * field_width - PITCH_MARGIN;
        let length = length.min(room.max(LENGTH_COMPACT * 0.5));

        let width = WIDTH_NARROW
            + (WIDTH_WIDE - WIDTH_NARROW) * self.tactical.team_width_target.clamp(0.0, 1.0);

        // Lateral slide toward the ball's flank. A team defending its own
        // box shifts almost entirely to the ball side; a team building an
        // attack holds width so it can switch.
        let slide = if self.tactical.in_possession {
            SLIDE_ATTACKING
        } else {
            SLIDE_DEFENDING
        };
        let pitch_centre_y = field_height / 2.0;
        let centre_target = pitch_centre_y + (ball.y - pitch_centre_y) * slide;
        // Keep the whole block on the pitch rather than clamping each
        // anchor separately — clamping per player collapses everyone on
        // the far flank onto the same touchline strip.
        let half_w = width / 2.0;
        let centre_target = centre_target.clamp(
            PITCH_MARGIN + half_w.min(pitch_centre_y - PITCH_MARGIN),
            field_height - PITCH_MARGIN - half_w.min(pitch_centre_y - PITCH_MARGIN),
        );
        // The lateral slide is rate-limited for the same reason as the
        // rear line: a switch of play crosses the pitch far faster than
        // a team can shuffle across it, and the delay in getting over IS
        // why switching play works.
        let centre_y = Self::approach(
            shape.active.then_some(shape.centre_y),
            centre_target,
            self.max_step,
            BLOCK_SNAP,
        );

        // ── Project every player into it ─────────────────────────────
        let mut len = 0usize;
        for p in self.field.players.iter() {
            if p.team_id != self.team_id || p.is_sent_off || len == MAX_ON_PITCH {
                continue;
            }
            let anchor = if p.tactical_position.current_position.is_goalkeeper() {
                self.keeper_anchor(side, field_width, pitch_centre_y, ball, rear)
            } else {
                let depth_frac = (side.attacking_progress_x(p.start_position.x, field_width)
                    - min_depth)
                    / depth_span;
                let lat_frac = (p.start_position.y - min_lat) / lat_span;
                let progress = rear + depth_frac * (length / field_width);
                let x = side
                    .x_at_progress(progress, field_width)
                    .clamp(PITCH_MARGIN, field_width - PITCH_MARGIN);
                let y = (centre_y + (lat_frac - 0.5) * width)
                    .clamp(PITCH_MARGIN, field_height - PITCH_MARGIN);
                Vector3::new(x, y, 0.0)
            };
            shape.anchors[len] = (p.id, anchor);
            len += 1;
        }

        // Is the plan too spread out, or does nobody go to it? The two
        // have opposite fixes and the aggregate block length cannot tell
        // them apart.
        #[cfg(feature = "match-logs")]
        {
            let (mut a_min, mut a_max) = (f32::MAX, f32::MIN);
            let (mut p_min, mut p_max) = (f32::MAX, f32::MIN);
            let mut worst = 0.0f32;
            for p in self.outfielders() {
                let Some((_, anchor)) = shape.anchors[..len].iter().find(|(id, _)| *id == p.id)
                else {
                    continue;
                };
                a_min = a_min.min(anchor.x);
                a_max = a_max.max(anchor.x);
                p_min = p_min.min(p.position.x);
                p_max = p_max.max(p.position.x);
                worst = worst.max((p.position - anchor).magnitude());

                let pos = p.tactical_position.current_position;
                let role = if pos.is_forward() {
                    3
                } else if pos.is_midfielder() {
                    2
                } else {
                    1
                };
                ShapeCensus::note_axis_lag(role, side.forward_delta(anchor.x, p.position.x));
            }
            if a_min <= a_max && p_min <= p_max {
                // Split by phase: the defending block has a 35-45 m real
                // target, the attacking one 50-60 m, so the all-phase mean
                // answers neither question on its own.
                let defending = self
                    .field
                    .ball
                    .current_owner
                    .and_then(|id| self.field.players.iter().find(|p| p.id == id))
                    .is_some_and(|p| p.team_id != self.team_id);
                ShapeCensus::note_span(a_max - a_min, p_max - p_min, worst, defending);
            }
        }

        shape.len = len;
        shape.rear_progress = rear;
        shape.length = length;
        shape.centre_y = centre_y;
        shape.width = width;
        shape.active = len > 0;
    }

    /// The keeper's anchor. He is not part of the block — he holds his
    /// line and steps out as it rises, which is what makes a high line
    /// survivable. His own state machine overrides this whenever the ball
    /// is anywhere near him; this only decides where he idles.
    fn keeper_anchor(
        &self,
        side: PlayerSide,
        field_width: f32,
        pitch_centre_y: f32,
        ball: Vector3<f32>,
        rear: f32,
    ) -> Vector3<f32> {
        // Sweeper depth scales with how high the block is: a rear line on
        // the halfway line wants the keeper 20 m off his goal, a low block
        // wants him on it. Bounded well inside his own half either way.
        let sweep = (rear * 0.45).clamp(0.02, 0.16);
        let x = side.x_at_progress(sweep, field_width);
        // Shade toward the ball's side of the goal, but only a little —
        // the keeper covers the near post, he does not slide with play.
        let y = pitch_centre_y + (ball.y - pitch_centre_y) * 0.18;
        Vector3::new(x, y, 0.0)
    }

    fn outfielders(&self) -> impl Iterator<Item = &crate::r#match::MatchPlayer> {
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

    /// The PLANNED block, plus the drift players carry on top of it,
    /// must land on the real-football reference of 35-45 m.
    ///
    /// The reference describes an OBSERVED span, and the observed span
    /// runs about 21 m longer than the plan — a systematic ladder
    /// (defenders settle deeper, forwards higher) plus per-player scatter,
    /// both measured in `dev_match stats`. Asserting the plan against the
    /// reference directly is the mistake this test used to make: it
    /// passed while the thing the reference actually describes measured
    /// 58 m.
    ///
    /// The drift constant is deliberately part of the assertion. If state
    /// work later reduces it, this test fails and says so, which is the
    /// signal to re-open the plan rather than leave it sized for a
    /// discipline problem that no longer exists.
    #[test]
    fn observed_block_lands_in_the_real_range() {
        /// Measured excess of the observed span over the planned one.
        const DRIFT_METRES: f32 = 21.0;
        let observed = |compact: f32| {
            (LENGTH_STRETCHED - (LENGTH_STRETCHED - LENGTH_COMPACT) * compact) * 0.125
                + DRIFT_METRES
        };

        // `compute_compactness` cannot reach the endpoints in a real
        // match — a tactic sits near 0.5 and the phase bias moves it by
        // ±0.20 — so the band that has to hold the reference is the band
        // matches are actually played in.
        for compact in [0.35f32, 0.5, 0.75] {
            let o = observed(compact);
            assert!(
                (35.0..=58.0).contains(&o),
                "observed block {o} m out of the real range at compactness {compact}"
            );
        }

        // The endpoints are a side in full attack and a side parked on
        // its own box. Both are legitimately outside the settled range;
        // neither may be absurd.
        assert!(observed(0.0) <= 65.0, "full stretch is not a shape");
        assert!(observed(1.0) >= 45.0, "full compaction is not a team");
    }

    /// Whatever else changes, the plan must remain a BLOCK — the static
    /// kickoff dots it replaced spanned 85 m.
    #[test]
    fn plan_is_always_a_block() {
        assert!(LENGTH_COMPACT < LENGTH_STRETCHED, "compactness inverted");
        assert!(
            LENGTH_STRETCHED * 0.125 < 50.0,
            "the plan alone is already longer than a real block"
        );
    }

    #[test]
    fn width_stays_inside_the_pitch() {
        let field_height = 545.0f32;
        for w in [0.0f32, 0.5, 1.0] {
            let width = WIDTH_NARROW + (WIDTH_WIDE - WIDTH_NARROW) * w;
            assert!(
                width <= field_height - 2.0 * PITCH_MARGIN,
                "block width {width} exceeds the pitch"
            );
        }
    }

    /// A rear line derived from the ball must react to the ball. The
    /// phase cap alone is a constant per phase, which is what made the
    /// old line height unable to tell a siege from a settled midfield.
    #[test]
    fn rear_follows_the_ball_under_the_phase_cap() {
        let field_width = 840.0f32;
        let standoff = STANDOFF_PASSIVE;
        let cap = 0.55f32; // a high line the phase permits
        let deep_ball = (0.20 - standoff / field_width)
            .min(cap)
            .max(REAR_FLOOR_PROGRESS);
        let high_ball = (0.80 - standoff / field_width)
            .min(cap)
            .max(REAR_FLOOR_PROGRESS);
        assert!(
            high_ball > deep_ball,
            "rear line ignored the ball: {deep_ball} vs {high_ball}"
        );
        assert!(high_ball <= cap, "rear line broke the phase cap");
    }

    /// A phase that wants a low block must be able to hold one even when
    /// the ball is upfield — the cap is a ceiling, not a target.
    #[test]
    fn phase_cap_bounds_a_high_ball() {
        let field_width = 840.0f32;
        let cap = 0.18f32; // LowBlock
        let rear = (0.90 - STANDOFF_AGGRESSIVE / field_width)
            .min(cap)
            .max(REAR_FLOOR_PROGRESS);
        assert!(
            (rear - cap).abs() < 1e-6,
            "low block failed to cap at {rear}"
        );
    }

    /// The deepest possible block plus its longest possible length must
    /// still fit inside the pitch, or the front rank clamps onto the goal
    /// line and every forward's anchor collapses onto the same strip.
    #[test]
    fn deepest_block_at_full_stretch_still_fits() {
        let field_width = 840.0f32;
        let front = REAR_FLOOR_PROGRESS * field_width + LENGTH_STRETCHED;
        assert!(front < field_width, "block overruns the pitch: {front}");
    }
}
