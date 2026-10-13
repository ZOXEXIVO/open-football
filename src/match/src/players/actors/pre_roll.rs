use super::*;
use crate::players::aftermath::Aftermath;
use crate::recording::playback::RecordedSpans;
use bevy::ecs::system::SystemId;

/// **A cut that lands in stride.**
///
/// Every filter a man runs — his pace, his heading, the lean into his
/// acceleration, the bank into a turn — lags the recording by its own time
/// constant, and a seek used to put them all down afresh: on the first frame
/// after a jump everybody stood still, facing wherever he last faced, for the
/// half second the filters took to catch up. So the frame a seek lands on
/// re-runs the systems that integrate the men over the stretch of recording
/// just behind the new playhead, at the step playing through would have
/// taken, and the picture it shows is the one playing through would have
/// arrived at. A clip edge bounds it: there is nothing recorded to roll
/// through before one.
#[derive(Resource)]
pub struct PreRoll {
    integrators: Vec<SystemId>,
}

impl PreRoll {
    /// Match time re-run behind a cut: long enough for a man sprinting away
    /// from the ball, whom the seek turned to face it, to come round onto his
    /// run at the pivot rate. A second left one cut in twenty more than twenty
    /// degrees out.
    const LEAD_MS: f64 = 1500.0;
    /// A frame at 60 fps, the step playing through takes. Coarser steps were
    /// tried for the early part and arrived ten degrees out: what playing
    /// through reaches depends on the instants it read the recording at.
    const STEP_MS: f64 = 1000.0 / 60.0;

    /// The systems that integrate the men, in the order the frame runs them.
    pub fn new(integrators: Vec<SystemId>) -> PreRoll {
        PreRoll { integrators }
    }

    pub fn install(world: &mut World) {
        let integrators = vec![
            world.register_system(Actors::follow_playhead),
            world.register_system(Aftermath::follow_playhead),
            world.register_system(Actors::animate),
        ];
        world.insert_resource(PreRoll::new(integrators));
    }

    /// Behind `animate` and in front of the joints, so what the rest of the
    /// frame poses, carries and films is the rolled state — and everything
    /// after it still sees the seek it answers to.
    pub fn run(world: &mut World) {
        let playback = world.resource::<Playback>();
        if !playback.seeked || world.resource::<Lineup>().on() {
            return;
        }
        let landing = playback.time_ms;
        let advanced = playback.advanced_ms;
        let from =
            (landing - Self::LEAD_MS).max(world.resource::<RecordedSpans>().start_of(landing));
        if from >= landing {
            return;
        }
        let integrators = world.resource::<PreRoll>().integrators.clone();
        Self::frame(world, &integrators, from, 0.0, true);
        // Counted rather than stepped until it arrives, which can leave a
        // sliver of a step at the end that costs a whole frame to run.
        let frames = ((landing - from) / Self::STEP_MS - 1e-9).ceil() as u32;
        let mut at = from;
        for frame in 1..=frames {
            let next = (from + f64::from(frame) * Self::STEP_MS).min(landing);
            Self::frame(world, &integrators, next, next - at, false);
            at = next;
        }
        let mut playback = world.resource_mut::<Playback>();
        playback.time_ms = landing;
        playback.advanced_ms = advanced;
        playback.seeked = true;
    }

    fn frame(world: &mut World, integrators: &[SystemId], at: f64, advanced: f64, seeked: bool) {
        let mut playback = world.resource_mut::<Playback>();
        playback.time_ms = at;
        playback.advanced_ms = advanced;
        playback.seeked = seeked;
        for &system in integrators {
            world
                .run_system(system)
                .expect("registered when the pre-roll was made");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// What each integrator was shown: playhead, step and whether it was a
    /// seek.
    #[derive(Resource, Default)]
    struct Shown(Vec<(f64, f64, bool)>);

    fn note(playback: Res<Playback>, mut shown: ResMut<Shown>) {
        shown
            .0
            .push((playback.time_ms, playback.advanced_ms, playback.seeked));
    }

    fn rolled(spans: RecordedSpans, lineup: Lineup, seeked: bool) -> World {
        let mut world = World::new();
        let mut playback = Playback::new(100_000.0);
        playback.time_ms = 10_000.0;
        playback.seeked = seeked;
        world.insert_resource(playback);
        world.insert_resource(lineup);
        world.insert_resource(spans);
        world.init_resource::<Shown>();
        let note = world.register_system(note);
        world.insert_resource(PreRoll::new(vec![note]));
        world.run_system_once(PreRoll::run).unwrap();
        world
    }

    fn clip(start: f64) -> RecordedSpans {
        let mut spans = RecordedSpans::default();
        spans.set(vec![(start, 20_000.0)]);
        spans
    }

    #[test]
    fn a_cut_is_played_up_to_from_a_second_and_a_half_behind() {
        let world = rolled(RecordedSpans::default(), Lineup::default(), true);
        let shown = &world.resource::<Shown>().0;
        assert_eq!(shown[0], (8_500.0, 0.0, true), "it starts with a seek");
        assert_eq!(shown.len(), 91, "a second and a half of frames");
        for pair in shown.windows(2) {
            let ((was, _, _), (at, step, seeked)) = (pair[0], pair[1]);
            assert!(
                !seeked && (step - PreRoll::STEP_MS).abs() < 1e-9 && (at - was - step).abs() < 1e-9
            );
        }
        assert_eq!(shown[90].0, 10_000.0, "it lands where the seek did");
        let playback = world.resource::<Playback>();
        assert!(playback.seeked && playback.time_ms == 10_000.0 && playback.advanced_ms == 0.0);
    }

    #[test]
    fn a_clip_edge_bounds_it() {
        let world = rolled(clip(9_600.0), Lineup::default(), true);
        let shown = &world.resource::<Shown>().0;
        assert_eq!(shown[0], (9_600.0, 0.0, true));
        assert_eq!(shown.last().unwrap().0, 10_000.0);
    }

    #[test]
    fn a_cut_to_the_start_of_a_clip_has_nothing_to_play() {
        let world = rolled(clip(10_000.0), Lineup::default(), true);
        assert!(world.resource::<Shown>().0.is_empty());
    }

    #[test]
    fn an_ordinary_frame_and_the_line_up_are_left_alone() {
        let playing = rolled(RecordedSpans::default(), Lineup::default(), false);
        assert!(playing.resource::<Shown>().0.is_empty());
        let walking_out = rolled(RecordedSpans::default(), Lineup::walking_out(), true);
        assert!(walking_out.resource::<Shown>().0.is_empty());
    }
}
