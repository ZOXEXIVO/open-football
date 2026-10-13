use super::*;

/// **A sole held on the spot it came down on**: where that is, which way he
/// faced as it landed, the last turn of the leg that kept it there, how much
/// of the hold is drawn, whether it is still holding, and — once it lets go
/// of a foot the stride still has on the grass — that the foot is coming up
/// into a step rather than sliding across.
#[derive(Clone, Copy)]
pub(crate) struct Plant {
    at: Vec3,
    yaw: f32,
    turn: Vec3,
    weight: f32,
    holding: bool,
    stepping: bool,
}

impl Plant {
    /// Under this the stride has a sole on the grass: a standing man's boot
    /// rests 17 mm up, a hair over the 15 mm `census_contact` reads.
    const GROUND: f32 = 0.02;
    /// How high the stride has a foot it is lifting before the spot it
    /// stood on is let go of, in metres: up to here the foot rises off the
    /// spot, and above it the swing takes it, the last turn fading out.
    const CLEAR: f32 = 0.05;
    /// Seconds a hold takes to let go as the stride lifts the foot. It comes
    /// on whole: at touchdown the sole is where the stride has it, so
    /// holding it there takes nothing.
    const LETS_GO: f32 = 0.1;
    /// …and a step takes, when the foot cannot be kept and the stride still
    /// has it down.
    const STEP: f32 = 0.25;
    /// How far round he may turn over a held foot, in radians, before it has
    /// to come up: about what a hip and a knee give.
    const TWIST: f32 = 0.8;
    /// How fast the knee may fold or open to keep a spot, in radians a
    /// second: about a loading knee's rate. Shortening a nearly straight leg
    /// by two centimetres takes a third of a radian, and taken in one frame
    /// that is the knee snapping.
    const FOLDS: f32 = 8.0;
    /// How far a standing man's held foot may stray from where his stance
    /// would put it, in metres, before he steps it back under him…
    const STRAY: f32 = 0.25;
    /// …and how far his leg may fall short of it, which it does where the
    /// knee would have to straighten or fold past what a knee does.
    const SLACK: f32 = 0.02;
    /// How far a striding man's leg may fall short of a held spot, in
    /// metres: a foot rolling up off it into toe-off. Past that the stride is
    /// not carrying the ground he covers — a keeper shuffling faster than a
    /// side-step reaches — and holding on would hang the foot in the air.
    const ROLL: f32 = 0.05;

    /// How high a step has the foot, 0..1: up and down again across the
    /// letting go, and nothing for a hold.
    fn lift(&self) -> f32 {
        if self.stepping {
            (PI * (1.0 - self.weight)).sin()
        } else {
            0.0
        }
    }
}

impl PlayerActor {
    /// **Holds each sole the stride puts on the grass where it came down**,
    /// until the stride lifts it — a leg that cannot quite reach it any more
    /// rolls up off the grass toward it, as a foot does going into toe-off,
    /// rather than sliding back under him, and lets it go past
    /// [`Plant::ROLL`]. A standing man has no stride to lift it: a foot he
    /// has turned too far over, drifted too far from or can no longer reach
    /// comes up into a step, one foot at a time. The striking foot is never
    /// held, and neither foot of a man off them. What it leaves in
    /// [`Gait::hold`] the hips and knees draw.
    pub(super) fn plant_feet(&mut self, root: &Transform, match_delta: f32, reset: bool) {
        if reset {
            self.plants = [None; 2];
        }
        let (pitch, roll) = self.topple();
        let carriage = Transform::from_translation(root.translation).with_rotation(root.rotation)
            * Carriage::placed(pitch, roll, self.lift_in(self.pose));
        let posture = self.posture;
        let legs = Physique::legs(&self.pose, &posture);
        let off_his_feet = self.height > Actors::AIRBORNE_FEET || self.pose.dive > 0.02;
        let standing = self.tread < Actors::STEPPING * 0.5;
        for (index, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let leg = legs[index];
            let striking = self.pose.power + self.pose.trap + self.pose.header > 0.02
                && side * self.pose.foot > 0.0;
            let sole = carriage.transform_point(leg.sole);
            let down = sole.y < Plant::GROUND && !striking && !off_his_feet;
            let other_stepping = self.plants[1 - index].is_some_and(|plant| plant.stepping);
            // The spot rises with the stride's own lift, so a foot it lets go
            // of comes up off the grass where it stood and only then crosses
            // to the swing, instead of being drawn there along the turf.
            let reached = self.plants[index].map(|plant| {
                let spot = Vec3::new(plant.at.x, plant.at.y.max(sole.y), plant.at.z);
                let most = Plant::FOLDS * match_delta;
                leg.reach(
                    &posture,
                    carriage.rotation.inverse() * (spot - carriage.translation),
                    (plant.turn.z - most, plant.turn.z + most),
                )
            });
            let misses = reached.map_or(0.0, |(_, misses)| misses);
            let kept = |plant: &Plant| {
                if !standing {
                    return misses < Plant::ROLL;
                }
                if other_stepping {
                    return true;
                }
                let twist = (self.heading - plant.yaw + PI).rem_euclid(TAU) - PI;
                let strayed =
                    Vec2::new(plant.at.x - sole.x, plant.at.z - sole.z).length() > Plant::STRAY;
                twist.abs() < Plant::TWIST && !strayed && misses < Plant::SLACK
            };
            let plant = match self.plants[index] {
                Some(plant) if plant.holding && down && kept(&plant) => Some(plant),
                // A standing man has no stride to lift the foot, so it is
                // stepped up; a striding one's stride lifts it.
                Some(plant) if plant.holding => Some(Plant {
                    holding: false,
                    stepping: down && standing,
                    ..plant
                }),
                Some(plant) => {
                    let lasts = if plant.stepping {
                        Plant::STEP
                    } else {
                        Plant::LETS_GO
                    };
                    let weight = plant.weight - match_delta / lasts;
                    (weight > 0.0).then_some(Plant { weight, ..plant })
                }
                None => None,
            };
            self.plants[index] = plant
                // Chased into the swing, the spot behind him wrenches the
                // leg back across it as the stride brings it through.
                .map(|plant| Plant {
                    turn: match reached {
                        Some((turn, _)) if plant.holding || sole.y < Plant::CLEAR => turn,
                        _ => plant.turn,
                    },
                    ..plant
                })
                .or_else(|| {
                    down.then_some(Plant {
                        at: sole,
                        yaw: self.heading,
                        turn: Vec3::ZERO,
                        weight: 1.0,
                        holding: true,
                        stepping: false,
                    })
                });
            self.pose.hold[index] = self.plants[index].map_or(Hold::default(), |plant| Hold {
                flex: plant.turn.x,
                splay: plant.turn.y,
                fold: plant.turn.z,
                lift: plant.lift(),
                weight: Actors::ease(plant.weight),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(actor: &mut PlayerActor, root: &Transform, reset: bool) {
        actor.pose = actor.gait();
        actor.posture = Posture::of(&actor.pose);
        actor.plant_feet(root, 1.0 / 60.0, reset);
    }

    fn soles(actor: &PlayerActor, root: &Transform) -> [Vec3; 2] {
        let world = *root * Carriage::placed(0.0, 0.0, actor.lift_in(actor.pose));
        Physique::legs(&actor.pose, &actor.posture).map(|leg| world.transform_point(leg.sole))
    }

    /// The recording rocking a standing man a few centimetres sways him over
    /// his feet; it does not slide them.
    #[test]
    fn a_standing_man_sways_over_his_feet() {
        let mut actor = PlayerActor::new(7, false, true);
        let mut root = Transform::IDENTITY;
        frame(&mut actor, &root, true);
        let landed = soles(&actor, &root);
        for x in [0.02, -0.03, 0.01, 0.0] {
            root.translation.x = x;
            frame(&mut actor, &root, false);
            for (was, now) in landed.iter().zip(soles(&actor, &root)) {
                assert!(
                    Vec2::new(was.x - now.x, was.z - now.z).length() < 0.005,
                    "slid from {was:?} to {now:?}"
                );
            }
        }
    }

    /// …and drifted half a metre, he steps a foot back under him rather than
    /// dragging it.
    #[test]
    fn a_foot_left_behind_is_stepped_back_under_him() {
        let mut actor = PlayerActor::new(7, false, true);
        let mut root = Transform::IDENTITY;
        frame(&mut actor, &root, true);
        root.translation.x = 0.5;
        let mut lifted = 0.0f32;
        for _ in 0..30 {
            frame(&mut actor, &root, false);
            lifted = lifted.max(actor.pose.hold[0].lift.max(actor.pose.hold[1].lift));
        }
        assert!(lifted > 0.9, "never stepped: the lift reached {lifted}");
    }
}
