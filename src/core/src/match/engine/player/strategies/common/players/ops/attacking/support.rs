//! **Offering yourself to the man on the ball** — where an off-ball
//! team-mate stands so that there is a pass to him.
//!
//! # The problem this exists to solve
//!
//! Two states owned a "go and support the carrier" movement, and both
//! sent the supporting player to stand more or less on top of him:
//!
//! * `ForwardRunningState` — "SUPPORT PRESSURED TEAMMATE" placed the
//!   forward `22u * 0.7` off the carrier plus a 10u goal-ward nudge,
//!   which is **two to three metres**, and it fired for every forward
//!   within 10 m of the ball whenever the carrier had an opponent inside
//!   1.9 m. The pressure census says that is 60% of the time in the
//!   attacking third.
//! * `MidfielderAttackSupportingState` — `AttackingRunType::SupportRun`
//!   used a `support_distance` of 40u, **five metres**, measured from the
//!   ball rather than from the carrier.
//!
//! Those two states are 7.2% and 10.3% of every AI tick in the match —
//! between them the two largest off-ball behaviours in the engine — and
//! what they describe is not support. A team-mate two metres away is not
//! a passing option: the man pressing the carrier is already goal-side of
//! both of them, the pass does not beat anybody, and the two of them plus
//! their markers are four bodies inside a three-metre circle. Measured
//! over twelve fixtures at level 14 (`dev_match stats`, SPACING CENSUS)
//! with the ball in the final third: a clump of four-plus bodies inside
//! 3 m existed on **55% of ticks**, and the side in possession had
//! **0.18 unmarked team-mates ahead of the ball** against a real one to
//! three.
//!
//! # The football
//!
//! Support is an ANGLE at a DISTANCE, and the distance has a floor. Ten
//! to fifteen metres is the range a coach asks for: near enough that the
//! pass is safe and the return is on, far enough that one defender cannot
//! press the ball and cover the pass at the same time. Under pressure the
//! supporting player comes shorter — but to ten metres, not to two.
//!
//! The angle is the other half. The presser's own body casts a shadow
//! through the carrier, and the whole skill of supporting is standing
//! outside it, which is why a player who has to move at all moves sideways
//! rather than closer.
//!
//! # Why the bearing is the player's own
//!
//! Every off-ball destination in this engine that was derived from a
//! shared global — the best free channel, the widest gap, the arriving
//! runner's target — handed the same answer to everybody who asked, and
//! that is how bodies came to converge in the first place (see
//! [`AttackPlan`](crate::r#match::AttackPlan)). So the offer below is
//! computed from **where the supporting player already is**: it keeps his
//! bearing from the carrier and corrects only the radius and, a little,
//! the angle. Two players approaching from different sides stay on
//! different sides, and the team fans out around the ball instead of
//! collapsing onto it.

use crate::r#match::StateProcessingContext;
use nalgebra::Vector3;

pub struct SupportOffer;

impl SupportOffer {
    /// Closest a supporting player offers himself, in game units.
    /// 76u = 9.5 m.
    ///
    /// The floor is the point of the whole model. Inside it the
    /// supporting player is behind the same defender as the carrier, so
    /// the pass beats nobody, and he is close enough that the two of them
    /// are one target — which is what turns a possession into a huddle.
    const NEAR: f32 = 76.0;

    /// …and the furthest a SHORT outlet is still short. 160u = 20 m.
    /// Beyond this he is not supporting the carrier, he is occupying a
    /// different part of the pitch, and the plan has other names for
    /// that.
    const FAR: f32 = 160.0;

    /// Where an unpressured offer settles, as a fraction of the band. A
    /// carrier with time wants his options spread; one under pressure
    /// wants them closer, and [`Self::pressure`] moves the offer down the
    /// band toward [`Self::NEAR`].
    const SETTLED: f32 = 0.45;

    /// A carrier with an opponent this close is under real pressure.
    /// 44u = 5.5 m — inside this a defender can commit to the ball.
    const CONTEST: f32 = 44.0;

    /// How much of the offer's bearing is the supporting player's own
    /// existing angle to the carrier, how much is the lane away from the
    /// man pressing him, and how much is a pull up the pitch so support
    /// is progressive rather than purely lateral.
    ///
    /// Own bearing dominates on purpose — see the module note on why a
    /// shared destination is what produced the clustering.
    const W_OWN: f32 = 0.62;
    const W_SHADOW: f32 = 0.26;
    const W_FORWARD: f32 = 0.12;

    /// Where this player should stand to be an option for `carrier`.
    ///
    /// Returns `None` when he IS the carrier, or when the geometry is
    /// degenerate (the two are standing on the same point) — the caller
    /// then keeps whatever it was doing.
    pub fn target(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> Option<Vector3<f32>> {
        let me = ctx.player.position;
        let bearing = me - carrier;
        let own = bearing.try_normalize(0.01)?;

        // The lane away from the man pressing the ball. His body casts a
        // shadow through the carrier, and standing outside it is most of
        // what supporting means.
        let presser = Self::nearest_presser(ctx, carrier);
        let shadow = presser
            .and_then(|p| (carrier - p).try_normalize(0.01))
            .unwrap_or(own);

        // …and a pull toward the goal we are attacking, so the outlet is
        // progressive. Small: a support pass is often square or backward,
        // and a supporting player who always runs forward is a supporting
        // player the carrier cannot reach.
        let forward = (ctx.player().opponent_goal_position() - carrier)
            .try_normalize(0.01)
            .unwrap_or(own);

        let dir = (own * Self::W_OWN + shadow * Self::W_SHADOW + forward * Self::W_FORWARD)
            .try_normalize(0.01)
            .unwrap_or(own);

        let target = carrier + dir * Self::distance(ctx, carrier);
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        Some(Vector3::new(
            target.x.clamp(20.0, field_width - 20.0),
            target.y.clamp(20.0, field_height - 20.0),
            0.0,
        ))
    }

    /// How far off the carrier this offer sits.
    ///
    /// The player's current distance, held inside the band — so a man
    /// already at a sensible range barely moves, and only the two errors
    /// the band exists to catch are corrected: standing on top of him,
    /// and being too far away to be an outlet at all.
    fn distance(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> f32 {
        let held = (ctx.player.position - carrier)
            .magnitude()
            .clamp(Self::NEAR, Self::FAR);
        let settled = Self::NEAR + (Self::FAR - Self::NEAR) * Self::SETTLED;
        // Under pressure the offer comes shorter, toward the floor and
        // never through it.
        let pressure = Self::pressure(ctx, carrier);
        let want = settled + (Self::NEAR - settled) * pressure;
        // Blend rather than snap: he is correcting his position, not
        // teleporting to a coordinate.
        (held * 0.45 + want * 0.55).clamp(Self::NEAR, Self::FAR)
    }

    /// How hard the carrier is being pressed, 0..1.
    fn pressure(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> f32 {
        match Self::nearest_presser(ctx, carrier) {
            Some(p) => (1.0 - (p - carrier).magnitude() / Self::CONTEST).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// The opponent nearest the carrier, whoever he is marking.
    fn nearest_presser(ctx: &StateProcessingContext, carrier: Vector3<f32>) -> Option<Vector3<f32>> {
        ctx.players()
            .opponents()
            .all()
            .map(|o| (o.position, (o.position - carrier).magnitude()))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(pos, _)| pos)
    }

    /// The band, for callers that need to know whether a player is
    /// already offering — a gate written against a different number is
    /// how the two states drifted apart in the first place.
    pub const fn near() -> f32 {
        Self::NEAR
    }

    pub const fn far() -> f32 {
        Self::FAR
    }
}
