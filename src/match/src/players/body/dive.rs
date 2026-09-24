//! **A keeper's dive, as his limbs make it**: the push off one leg with the
//! other knee driven through, the reach toward where the ball is rather than
//! over his head whatever the flight, and the give of the landing.
use super::*;

impl Joint {
    /// **The arm reaching along the grass in front of him**, as the shoulder
    /// pitch of a smother — see [`Gait::smother`]. Straight out from the
    /// chest, which on a body lying on its side is along the turf toward the
    /// ball rather than past the head.
    const SMOTHER_SHOULDER: f32 = -1.62;
    /// …and how far the two hands come in toward each other doing it: a
    /// smother is both gloves on one ball, not a span across a goal.
    const SMOTHER_GATHER: f32 = 0.55;
    /// **The push**, as the hip and knee the two legs leave the ground with:
    /// the one he drives off straight out behind him, the other thigh
    /// thrown forward and up across him with the knee bent under it. What a
    /// diving keeper's legs look like for the first tenth of a second, and
    /// the whole of what makes a dive a jump rather than a fall.
    const PUSH_HIP: f32 = 0.40;
    const PUSH_KNEE: f32 = 0.04;
    const DRIVEN_HIP: f32 = -1.05;
    const DRIVEN_KNEE: f32 = 1.75;
    /// **The landing's give**, per unit of [`Gait::thud`]: the trunk curling
    /// harder over the impact and the knees and elbows folding with it, then
    /// all of it springing back.
    const THUD_CURL: f32 = 0.30;
    const THUD_KNEE: f32 = 0.45;
    const THUD_ELBOW: f32 = -0.40;
    /// How far the trunk leans into a take-off he is loading for, in radians
    /// at a full coil — see [`Gait::coil`].
    const COIL_LEAN: f32 = 0.32;

    /// The leading arm's shoulder at full stretch: over his head for a dive
    /// across his goal, out in front of his chest for one at a man's feet.
    pub(super) fn reach_shoulder(gait: Gait) -> f32 {
        Self::REACH_SHOULDER + (Self::SMOTHER_SHOULDER - Self::REACH_SHOULDER) * gait.smother
    }

    /// …and how much of the span between the gloves a smother keeps.
    pub(super) fn reach_span(gait: Gait) -> f32 {
        1.0 - Self::SMOTHER_GATHER * gait.smother
    }

    /// **This leg in the air**, as `(hip, knee)`: out of the push and into
    /// the trail. `leading` is +1 for the leg on the side he is going to,
    /// which is the one he drives off, and −1 for the other.
    pub(super) fn dive_leg(gait: Gait, leading: f32, trailing: f32) -> (f32, f32) {
        let trail = (
            Self::DIVE_HIP + Self::DIVE_SCISSOR_HIP * leading,
            Self::DIVE_KNEE + Self::DIVE_SCISSOR_KNEE * trailing,
        );
        let near = leading.max(0.0);
        let far = (-leading).max(0.0);
        let push = (
            Self::PUSH_HIP * near + Self::DRIVEN_HIP * far,
            Self::PUSH_KNEE * near + Self::DRIVEN_KNEE * far,
        );
        let flown = 1.0 - gait.push;
        (
            push.0 + (trail.0 - push.0) * flown,
            push.1 + (trail.1 - push.1) * flown,
        )
    }

    /// How much of the flight pose his legs are in: the push from the first
    /// frame, the trail as the extension opens.
    pub(super) fn diving_legs(gait: Gait) -> f32 {
        (gait.dive * gait.stretch).max(gait.push) * (1.0 - gait.jump)
    }

    /// **The elbow opens after the shoulder has**, which is what makes the
    /// forearm whip out rather than swing out on a straight arm: the same
    /// extension, arriving later down the chain.
    pub(super) fn unfolding(gait: Gait) -> f32 {
        gait.stretch.max(0.0).powf(1.6)
    }

    /// The landing, as the curl it adds to the trunk.
    pub(super) fn thud_curl(gait: Gait) -> f32 {
        Self::THUD_CURL * gait.thud
    }

    pub(super) fn thud_knee(gait: Gait) -> f32 {
        Self::THUD_KNEE * gait.thud
    }

    pub(super) fn thud_elbow(gait: Gait) -> f32 {
        Self::THUD_ELBOW * gait.thud
    }

    /// The trunk leaning out over the take-off he is loading for, as a
    /// rotation: rolled toward the side it goes and pitched along it.
    pub(super) fn coiling(gait: Gait) -> Quat {
        Quat::from_rotation_z(-Self::COIL_LEAN * gait.coil.x)
            * Quat::from_rotation_x(0.5 * Self::COIL_LEAN * gait.coil.y)
    }
}
