//! **The dip between one episode and the next.**
//!
//! A clipped recording is not a film. It is a dozen or so clips scattered
//! across ninety minutes (see `HighlightSelector` and `RecordingScope::Goals`),
//! and playback jumps the holes between them — [`Playback::advance`] moves the
//! playhead to the start of the next clip and the camera cuts with it.
//!
//! Before this, that jump was invisible. One frame the ball was in one goalmouth
//! in the twelfth minute; the next it was in the other one in the sixty-first,
//! with twenty-two men standing somewhere else, and nothing on the screen said
//! a cut had happened rather than the replay having gone wrong.
//!
//! So the picture comes up through a vignette: dark at the instant of the cut,
//! clear about half a second later. It is the same dark ring the match page
//! wears while the viewer is still loading (`.match-loader`'s radial ground,
//! and the frame `.match-stage::after` keeps around the replay afterwards) —
//! reused rather than invented, so a cut looks like the picture settling into
//! the box it already sits in.
//!
//! **A fade IN and not a fade out**, which is not the asymmetry it looks like:
//! there is nothing to fade out of. The playhead leaves the old clip on the
//! very frame it arrives at the new one, and dimming the seconds BEFORE a cut
//! would mean spending the end of every clip — the celebration, the rebound,
//! the man walking off — on a transition.
//!
//! ## What dips, and what does not
//!
//! Drawn over the replay and under the transport bar, which is a statement
//! about what belongs to the picture. The name plates over the players' heads
//! go down with the football they are attached to; the bar, the speed chip and
//! the flight stick do not, because furniture that dimmed every time the replay
//! cut would read as a fault rather than as a transition. That order is held by
//! three numbers and nothing else — see [`Backdrop::PICTURE`].

use crate::playback::Playback;
use crate::stage::Backdrop;
use crate::textures::Textures;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;

/// The sheet the dip is painted on: one node over the whole window, dark at the
/// edges and thinner in the middle, showing only while a fade is running.
#[derive(Component)]
pub struct CutVeil;

/// How far through the dip we are.
#[derive(Resource, Default)]
pub struct CutFade {
    /// Seconds of fade left. Zero means the picture is clear and the veil is
    /// not being drawn at all.
    left: f32,
    /// How long THIS fade was armed for.
    ///
    /// Carried rather than recomputed from [`Self::FADE`], because the ramp is
    /// a fraction of its own length and the playback speed it was scaled by can
    /// be changed halfway through one.
    span: f32,
}

impl CutFade {
    /// How long the picture takes to come up, in seconds of real time at 1x.
    ///
    /// Long enough to read as a transition and short enough that nobody waits
    /// through it. Either side of that was tried by eye: a quarter of a second
    /// reads as a flicker — the panel's own refresh — and a whole one spends a
    /// tenth of a ten-second clip looking at a dark screen.
    const FADE: f32 = 0.55;

    /// The dark the picture comes up out of.
    ///
    /// `#080c12` — the ground the match page paints behind the stage, and the
    /// colour of its own loading vignette. The dip and the frame around the
    /// replay are then the same dark rather than two nearby ones.
    const DIP: Srgba = Srgba::new(0.031, 0.047, 0.071, 1.0);

    /// Builds the veil, hidden, at startup.
    ///
    /// Built here rather than on the frame it is first wanted, for the reason
    /// everything else in this viewer is (see `bringup`): a node assembled at
    /// the moment of a cut is a texture uploaded in the middle of one.
    ///
    /// An `ImageNode` for a second reason of the same kind. `BackgroundGradient`
    /// would draw this ring with no texture at all — and out of its own shader,
    /// which is a program the browser would have to link on the frame the ring
    /// first appears, for the better part of four seconds. The backdrop under
    /// this is already a stretched `ImageNode`, so this one queues nothing the
    /// first frame did not.
    pub fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
        commands.spawn((
            CutVeil,
            ImageNode {
                image: Textures::vignette(&mut images),
                // The node's size decides the image's: a square ring stretched
                // across a 16:9 window, which is what a lens does anyway.
                image_mode: NodeImageMode::Stretch,
                color: Self::DIP.with_alpha(0.0).into(),
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            GlobalZIndex(Backdrop::DIP),
            // A full-screen node captures the hover and the press of everything
            // under it by default — `FocusPolicy::Block` is what a node with no
            // opinion gets. Nothing interactive is under this one today, and the
            // day something is, it must not stop working for half a second every
            // time the replay cuts.
            FocusPolicy::Pass,
            Visibility::Hidden,
        ));
    }

    /// Arms the dip on the frame the playhead was cut, and runs it out.
    ///
    /// Registered behind [`Playback::advance`], which is what sets the flag,
    /// and ahead of [`Playback::end_frame`], which clears it. It reads `cut`
    /// and never `seeked`: see [`Playback::cut`] for why the two are separate.
    pub fn follow_playhead(
        mut fade: ResMut<CutFade>,
        playback: Res<Playback>,
        time: Res<Time>,
        veil: Single<(&mut ImageNode, &mut Visibility), With<CutVeil>>,
    ) {
        let Some(strength) = fade.advance(playback.cut, playback.speed, time.delta_secs()) else {
            return;
        };

        let (mut veil, mut visibility) = veil.into_inner();
        veil.color = Self::DIP.with_alpha(strength).into();
        // Taken off the screen rather than left at zero alpha: a transparent
        // full-window quad is still a quad, drawn for the rest of the match.
        visibility.set_if_neq(if strength > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }

    /// Runs the dip on by `elapsed` seconds, arming it first if the playhead
    /// was cut. `None` when there is nothing on the screen and nothing to put
    /// there, which is almost every frame of a match.
    fn advance(&mut self, cut: bool, speed: f32, elapsed: f32) -> Option<f32> {
        if cut {
            // Real seconds at 1x and less than that above it. At 8x a
            // ten-second clip is over in a second and a quarter, and a fade
            // that took half a second of that would be most of what was seen
            // of it. Below 1x it stays put: slowing the replay down is a way
            // of looking at the football, not at the transitions.
            self.span = Self::FADE / speed.max(1.0);
            self.left = self.span;
        } else if self.left <= 0.0 {
            return None;
        } else {
            self.left = (self.left - elapsed).max(0.0);
        }
        Some(Self::ease(self.left / self.span.max(f32::EPSILON)))
    }

    /// The ramp: full dark at the cut, clear at the end, and no corner at
    /// either. A linear release lets go the instant it starts and then stops
    /// dead, and the eye reads both of those as the picture stepping rather
    /// than coming up.
    fn ease(through: f32) -> f32 {
        let through = through.clamp(0.0, 1.0);
        through * through * (3.0 - 2.0 * through)
    }
}

/// The dip is half a second long and lives between two frames of a replay, so
/// none of these rules can be checked by looking at one: what they are about is
/// the SHAPE of the half second — that it starts dark, that it only ever gets
/// lighter, that it ends, and that it is not still running when the next clip
/// is well under way.
#[cfg(test)]
mod tests {
    use super::*;

    /// A millisecond, so a fade measured in these is not being measured in
    /// frames of whatever the machine happened to manage.
    const TICK: f32 = 0.001;

    /// Arms a dip, runs it to the end, and says how long that took in seconds.
    fn run_out(speed: f32) -> f32 {
        let mut fade = CutFade::default();
        fade.advance(true, speed, 0.0);
        let mut ticks = 0;
        while fade.advance(false, speed, TICK).is_some() {
            ticks += 1;
            assert!(ticks < 100_000, "the dip never cleared");
        }
        ticks as f32 * TICK
    }

    /// The whole point of the thing: the frame a cut lands on is the darkest
    /// one, and every frame after it is lighter, until it is gone.
    #[test]
    fn a_cut_opens_dark_and_the_picture_comes_up() {
        let mut fade = CutFade::default();
        assert_eq!(
            fade.advance(true, 1.0, 0.0),
            Some(1.0),
            "the frame the cut landed on was not the darkest one"
        );

        let mut previous = 1.0;
        let mut seen = 0;
        while let Some(strength) = fade.advance(false, 1.0, TICK) {
            assert!(
                strength <= previous,
                "the picture went back down: {previous} then {strength}"
            );
            previous = strength;
            seen += 1;
            assert!(seen < 100_000, "the dip never cleared");
        }
        assert_eq!(previous, 0.0, "the dip stopped short of a clear picture");
    }

    /// And nothing is drawn at all otherwise. The veil is over the whole
    /// window, so an idle state that leaves it at anything but gone is a match
    /// watched through a filter.
    #[test]
    fn nothing_is_drawn_until_something_is_cut() {
        let mut fade = CutFade::default();
        for _ in 0..600 {
            assert_eq!(
                fade.advance(false, 1.0, TICK),
                None,
                "the veil was painted over a replay that never cut"
            );
        }
    }

    /// A dip is spent in REAL seconds but it is covering match seconds, and at
    /// speed there are far fewer of those to spend. Half a second of a clip
    /// skimmed at 8x is most of the clip.
    #[test]
    fn the_dip_is_over_sooner_when_the_replay_is_faster() {
        let real = run_out(1.0);
        assert!(
            (real - CutFade::FADE).abs() < 0.01,
            "a dip at 1x ran {real}s against the {}s it is set to",
            CutFade::FADE
        );

        let fast = run_out(8.0);
        assert!(
            (fast - CutFade::FADE / 8.0).abs() < 0.01,
            "a dip at 8x ran {fast}s against the {}s eight times as fast",
            CutFade::FADE / 8.0
        );

        // …and never longer than it does at real time. Watching in slow motion
        // is a way of looking at the football, not at the transitions.
        let slow = run_out(0.25);
        assert!(
            (slow - real).abs() < 0.01,
            "a dip at quarter speed ran {slow}s against {real}s"
        );
    }

    /// Two clips can end and begin a few seconds apart — the shortlist spaces
    /// its markers two minutes apart, but a goal off a rebound merges with the
    /// chance before it and a change can be made at the restart. A dip that
    /// carried on from wherever the last one had got to would come up out of a
    /// half-dark screen.
    #[test]
    fn a_second_cut_starts_its_own_dip() {
        let mut fade = CutFade::default();
        fade.advance(true, 1.0, 0.0);
        let half_way = fade.advance(false, 1.0, CutFade::FADE * 0.5);
        assert!(
            half_way.is_some_and(|strength| strength < 1.0 && strength > 0.0),
            "expected a dip in progress, got {half_way:?}"
        );

        assert_eq!(
            fade.advance(true, 1.0, TICK),
            Some(1.0),
            "the second cut inherited the first one's fade instead of starting over"
        );
    }
}
