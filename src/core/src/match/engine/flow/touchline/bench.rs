//! Where the men involved in a change are standing, and the two points on
//! the line one crosses.
//!
//! All of it is geometry and none of it is a decision, which is why it sits
//! apart from [`changeover`](super::changeover): the choreography asks this
//! module where to walk and does not otherwise know what a dugout is.

use crate::r#match::MatchFieldSize;
use nalgebra::Vector3;

/// Where a man off the pitch is standing, and where he is heading.
///
/// **A drawing coordinate, and deliberately not `MatchPlayer::position`.**
///
/// Everybody who is not one of the eleven is parked at the off-pitch
/// sentinel `(-500, -500)`, and that is not decoration either:
/// `PlayerFieldData` chains `field.substitutes` into the same proximity
/// table the on-pitch players are scanned in, so a substitute standing two
/// metres beyond the touchline would win "who is closest to this loose ball"
/// for anything rolling near the line, and the man actually on the pitch
/// would yield to him. The sentinel is what makes that impossible.
///
/// So a man off the pitch carries two coordinates: one the engine can never
/// mistake for a player, and this one, which nothing reads but the recorder.
///
/// ⚠ **Only a man in the middle of a change ever has one.** A substitute who
/// is not coming on has `touchline: None` and is not drawn at all — a row of
/// twelve figures a side standing there for ninety minutes is scenery nobody
/// asked for, and it was the first thing to go when this was watched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchlineStand {
    /// Where he is this instant.
    pub at: Vector3<f32>,
    /// Where he is heading — the technical area, for a man who has just come
    /// off. He stops being drawn the moment he gets there.
    pub seat: Vector3<f32>,
    /// **Somebody else is walking him.**
    ///
    /// `MatchField::settle_touchline` walks every off-pitch man toward his
    /// seat once per recorded frame, and while a substitution window is open
    /// [`SubstitutionBreak`] is walking one of them itself — first out to the
    /// line, which is not toward his seat at all. Without this the man coming
    /// off got BOTH steps every frame, in two different directions: measured
    /// on a real recording, 0.74 units a tick against a 0.55 speed, and the
    /// path bent as the two pulls traded off.
    ///
    /// Set when a change opens and cleared when it closes, so the answer to
    /// "who is moving this man" is always exactly one thing.
    ///
    /// [`SubstitutionBreak`]: super::SubstitutionBreak
    pub held: bool,
}

impl TouchlineStand {
    /// Walking from `at` to the technical area, under the substitution
    /// window's control until it closes — see [`Self::held`].
    pub fn walking(at: Vector3<f32>, seat: Vector3<f32>) -> Self {
        TouchlineStand {
            at,
            seat,
            held: true,
        }
    }

    /// True once he is standing on the spot he was heading for.
    pub fn arrived(&self) -> bool {
        self.at == self.seat
    }

    /// One step of the walk home, `distance` game units long. Returns true
    /// while he is still moving.
    ///
    /// Refuses to move a [held](Self::held) man: while a change is being
    /// played out, the window is the only thing allowed to move him.
    pub fn settle(&mut self, distance: f32) -> bool {
        if self.held {
            return true;
        }
        if self.arrived() {
            return false;
        }
        let delta = self.seat - self.at;
        let remaining = (delta.x * delta.x + delta.y * delta.y).sqrt();
        if remaining <= distance.max(0.05) {
            self.at = self.seat;
            return false;
        }
        let step = distance / remaining;
        self.at.x += delta.x * step;
        self.at.y += delta.y * step;
        true
    }
}

/// The two dugouts and the ground in front of them.
pub struct Bench;

impl Bench {
    /// How far beyond the touchline the technical area is, in game units.
    ///
    /// The run-off is `RunOff::SIDE` — 27.2 u, 3.4 m — deep and the
    /// hoardings stand at the end of it, with `RunOff::PLAYER_INSET` (8 u)
    /// as the closest a body is ever put to them. 17 u = 2.1 m leaves a man
    /// clear of the line he must not stand on and clear of the boards he
    /// cannot stand inside.
    pub const DEPTH: f32 = 17.0;

    /// How far out a man is once he has crossed the line, in game units.
    /// 6 u = 75 cm: over the touchline, out of play, and not yet at the
    /// dugout.
    pub const GATE_DEPTH: f32 = 6.0;

    /// **How far along the touchline a man who has come off walks before he
    /// stops being drawn**, in game units, measured from the halfway line.
    ///
    /// 152 u = 19 m, and it is a TIMING number rather than a plan of the
    /// ground. He is dropped the moment he gets there — there is no bench
    /// drawn for him to stand beside, and a lone figure in the run-off for
    /// the rest of the match is the scenery this feature exists without. So
    /// the walk has to outlast the shot that is watching it: 19 m at
    /// [`SubstitutionBreak::HOME`] is about eight seconds, and the
    /// substitution camera has cut back to the ball inside seven. He goes
    /// off-picture and then he goes.
    ///
    /// At 3 m — a real technical area — the walk took a second and he
    /// vanished in the middle of the frame the camera was pointed at.
    ///
    /// [`SubstitutionBreak::HOME`]: super::SubstitutionBreak::HOME
    const DUGOUT_ALONG: f32 = 152.0;

    /// How far to the team's own side of the halfway line a substitute steps
    /// on. 8 u = 1 m, so the two sides' gates do not sit on top of each
    /// other when both change at once.
    const GATE_OFFSET: f32 = 8.0;

    /// And how far apart two of the SAME side's substitutes stand while they
    /// wait, in game units.
    ///
    /// ⚠ **Without it a double change puts both men on one coordinate.** A
    /// side is allowed three changes at one stoppage and the gate was a
    /// single point, so two substitutes were drawn inside each other:
    /// z-fighting shirts, two names printed over one back, and — the thing
    /// that made it visible — a camera brought round specifically to look at
    /// them. 14 u = 1.75 m is a man's width and then some.
    const GATE_PITCH: f32 = 14.0;

    /// How close to a goal line an exit may be taken, in game units.
    /// 40 u = 5 m: a man does not leave the pitch through the corner flag.
    const EXIT_MARGIN: f32 = 40.0;

    /// **Which touchline the dugouts are on, and why it is not a coin flip.**
    ///
    /// `y = 0`: the near one, the side the replay's broadcast rig stands
    /// behind. That is where dugouts are in a real ground — the main stand —
    /// and it is what makes the substitution shot possible at all: a camera
    /// behind the near touchline is a camera behind the man coming on, so
    /// what it sees is his back as he runs onto the pitch.
    ///
    /// It was on the FAR line first, on the argument that `AIM_NEAR_BIAS`
    /// keeps the near run-off below the bottom edge of the broadcast frame
    /// for most of a match. That argument dies with the row it was protecting
    /// — nobody stands out here except during a change, and a change gets its
    /// own shot (`ChangeoverShot` in the viewer).
    fn touchline() -> f32 {
        0.0
    }

    /// The technical area a man who has just come off walks to — and stops
    /// being drawn at.
    ///
    /// `is_home` is keyed to the TEAM, never to `PlayerSide`: the two sides
    /// change ends at half time and the dugouts do not. Home takes the area
    /// to the left of the halfway line, away the one to the right, exactly as
    /// they are laid out on a real touchline.
    pub fn dugout(size: &MatchFieldSize, is_home: bool) -> Vector3<f32> {
        let half = size.width as f32 * 0.5;
        let x = if is_home {
            half - Self::DUGOUT_ALONG
        } else {
            half + Self::DUGOUT_ALONG
        };
        Vector3::new(
            x.clamp(Self::EXIT_MARGIN, size.width as f32 - Self::EXIT_MARGIN),
            Self::touchline() - Self::DEPTH,
            0.0,
        )
    }

    /// The point on the line a substitute steps over to come on: the halfway
    /// line, his own side of it, where the fourth official is standing.
    ///
    /// `waiting` is how many of his own side are already standing there — see
    /// [`Self::GATE_PITCH`], which is what keeps a double change from putting
    /// two men on one patch of grass.
    pub fn entry_gate(size: &MatchFieldSize, is_home: bool, waiting: usize) -> Vector3<f32> {
        let half = size.width as f32 * 0.5;
        let along = Self::GATE_OFFSET + waiting as f32 * Self::GATE_PITCH;
        let x = if is_home { half - along } else { half + along };
        Vector3::new(x, Self::touchline() - Self::GATE_DEPTH, 0.0)
    }

    /// Is this coordinate off the playing surface, on the dugout side?
    ///
    /// What the man coming off is walking towards, and the test that says his
    /// leg of the change is finished: past the line he stops belonging to the
    /// substitution window and starts belonging to the recorder, which walks
    /// him the rest of the way to his seat.
    pub fn is_over_the_line(at: Vector3<f32>) -> bool {
        at.y <= Self::touchline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> MatchFieldSize {
        MatchFieldSize::new(840, 545)
    }

    #[test]
    fn the_two_dugouts_flank_the_halfway_line() {
        let size = size();
        let half = size.width as f32 * 0.5;
        assert!(Bench::dugout(&size, true).x < half, "home is left of half");
        assert!(
            Bench::dugout(&size, false).x > half,
            "away is right of half"
        );
        assert!(Bench::entry_gate(&size, true, 0).x < half);
        assert!(Bench::entry_gate(&size, false, 0).x > half);
    }

    #[test]
    fn everything_out_here_is_off_the_pitch_but_short_of_the_boards() {
        use crate::r#match::engine::ball::ball::RunOff;
        let size = size();
        for is_home in [true, false] {
            for spot in [
                Bench::dugout(&size, is_home),
                Bench::entry_gate(&size, is_home, 0),
            ] {
                assert!(
                    Bench::is_over_the_line(spot),
                    "a man standing ON the pitch is a phantom player: {spot:?}"
                );
                assert!(
                    spot.y > -(RunOff::SIDE - RunOff::PLAYER_INSET),
                    "a man standing inside the advertising hoardings: {spot:?}"
                );
                assert!(spot.x > 0.0 && spot.x < size.width as f32);
            }
        }
    }

    #[test]
    fn both_sides_change_at_the_halfway_line() {
        // The two men meet where the substitute is standing, and he is at the
        // fourth official's shoulder whichever end his team is attacking —
        // so the exchange is always in the same place, which is what makes it
        // one picture instead of two.
        let size = size();
        let half = size.width as f32 * 0.5;
        for is_home in [true, false] {
            let gate = Bench::entry_gate(&size, is_home, 0);
            assert!(
                (gate.x - half).abs() < Bench::DEPTH,
                "the exchange is nowhere near the halfway line: {gate:?}"
            );
            assert!(Bench::is_over_the_line(gate));
        }
    }

    #[test]
    fn a_double_change_does_not_stand_two_men_on_one_spot() {
        // A side may change three at one stoppage, and with a single gate all
        // three stood inside each other: two shirts z-fighting and two names
        // printed across one back, in front of a camera brought round
        // specifically to look at them.
        let size = size();
        for is_home in [true, false] {
            let gates: Vec<Vector3<f32>> = (0..3)
                .map(|waiting| Bench::entry_gate(&size, is_home, waiting))
                .collect();
            for (index, gate) in gates.iter().enumerate() {
                for other in gates.iter().skip(index + 1) {
                    assert!(
                        (gate.x - other.x).abs() > 8.0,
                        "two substitutes a metre apart: {gate:?} and {other:?}"
                    );
                }
                assert!(Bench::is_over_the_line(*gate));
            }
        }
    }

    #[test]
    fn a_held_man_is_not_walked_by_anybody_else() {
        let seat = Vector3::new(300.0, -17.0, 0.0);
        let mut stand = TouchlineStand::walking(Vector3::new(400.0, -6.0, 0.0), seat);
        let before = stand.at;
        assert!(stand.held, "a change owns his feet until it closes");
        stand.settle(2.0);
        assert_eq!(
            stand.at, before,
            "the recorder walked a man the window owns"
        );
    }

    #[test]
    fn the_walk_home_arrives_and_then_stops() {
        let seat = Vector3::new(300.0, -17.0, 0.0);
        let mut stand = TouchlineStand::walking(Vector3::new(400.0, -6.0, 0.0), seat);
        stand.held = false;
        let mut steps = 0;
        while stand.settle(2.0) {
            steps += 1;
            assert!(steps < 1_000, "the walk never converged");
        }
        assert_eq!(stand.at, seat);
        assert!(stand.arrived());
        assert!(!stand.settle(2.0), "an arrived man keeps arriving");
    }
}
