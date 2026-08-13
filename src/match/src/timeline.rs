use crate::actors::BallState;
use crate::camera::{CameraFlight, CameraOrbit, CameraRig, CameraZoom};
use crate::config::ViewerConfig;
use crate::loader::ChunkLoader;
use crate::playback::Playback;
use bevy::prelude::*;
use bevy::text::LineBreak;
use bevy::ui::RelativeCursorPosition;

#[derive(Component)]
pub struct SeekTrack;

#[derive(Component)]
pub struct SeekFill;

#[derive(Component)]
pub struct SeekKnob;

#[derive(Component)]
pub struct ClockLabel;

#[derive(Component)]
pub struct PlayToggle;

#[derive(Component)]
pub struct PlayToggleIcon;

#[derive(Component)]
pub struct LoadingNotice;

#[derive(Component)]
pub struct SpeedButton;

#[derive(Component)]
pub struct SpeedLabel;

#[derive(Component)]
pub struct StatesButton;

#[derive(Component)]
pub struct CameraResetButton;

#[derive(Component)]
pub struct ZoomLabel;

/// What the debug overlay is showing. Only meaningful when the page asked for
/// it; in the game itself nothing ever reads or flips this.
#[derive(Resource)]
pub struct DebugOverlay {
    pub states: bool,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        DebugOverlay { states: true }
    }
}

/// The transport bar along the bottom of the canvas: play/pause, a scrub track
/// with goal markers, a camera reset and the match clock. It lives inside the
/// viewer rather than in the page so that the recording UI travels with the
/// renderer.
///
/// In debug mode it also carries the match harness's controls — playback speed,
/// a state-label toggle and the ball's engine coordinates.
pub struct Timeline;

impl Timeline {
    const BAR_HEIGHT: f32 = 46.0;
    const TRACK_HEIGHT: f32 = 8.0;
    const KNOB_SIZE: f32 = 14.0;
    const ACCENT: Color = Color::srgb(0.29, 0.68, 0.98);
    const CHIP_BACKGROUND: Color = Color::srgba(1.0, 1.0, 1.0, 0.14);
    const CHIP_ACTIVE: Color = Color::srgba(0.35, 0.78, 0.48, 0.55);

    pub fn spawn(mut commands: Commands, config: Res<ViewerConfig>) {
        let home = config.home.background_color(Color::srgb(0.0, 0.19, 0.49));
        let away = config.away.background_color(Color::srgb(0.70, 0.25, 0.0));

        commands
            .spawn(Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_children(|root| {
                root.spawn((
                    Node {
                        width: percent(100),
                        height: px(Self::BAR_HEIGHT),
                        align_items: AlignItems::Center,
                        column_gap: px(12),
                        padding: UiRect::axes(px(14), px(0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.05, 0.08, 0.72)),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        PlayToggle,
                        Interaction::default(),
                        Node {
                            width: px(26),
                            height: px(26),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            flex_shrink: 0.0,
                            border_radius: BorderRadius::all(px(13)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.14)),
                    ))
                    .with_children(|toggle| {
                        toggle.spawn((
                            PlayToggleIcon,
                            Text::new(">"),
                            TextFont {
                                font_size: FontSize::Px(13.0),
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                    });

                    bar.spawn((
                        SeekTrack,
                        RelativeCursorPosition::default(),
                        Node {
                            flex_grow: 1.0,
                            height: px(Self::TRACK_HEIGHT),
                            border_radius: BorderRadius::all(px(Self::TRACK_HEIGHT * 0.5)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.16)),
                    ))
                    .with_children(|track| {
                        track.spawn((
                            SeekFill,
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(0),
                                top: px(0),
                                width: percent(0),
                                height: percent(100),
                                border_radius: BorderRadius::all(px(Self::TRACK_HEIGHT * 0.5)),
                                ..default()
                            },
                            BackgroundColor(Self::ACCENT),
                        ));

                        // Half-time.
                        track.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: percent(50),
                                top: px(-3),
                                width: px(2),
                                height: px(Self::TRACK_HEIGHT + 6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.55)),
                        ));

                        for goal in &config.goals {
                            if config.match_time_ms <= 0.0 {
                                continue;
                            }
                            let at = (goal.time / config.match_time_ms).clamp(0.0, 1.0) as f32;
                            let color = if config.goal_belongs_to_home(goal) {
                                home
                            } else {
                                away
                            };
                            track.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: percent(at * 100.0),
                                    top: px(-5),
                                    margin: UiRect::left(px(-2)),
                                    width: px(4),
                                    height: px(Self::TRACK_HEIGHT + 10.0),
                                    border_radius: BorderRadius::all(px(2)),
                                    ..default()
                                },
                                BackgroundColor(color),
                            ));
                        }

                        track.spawn((
                            SeekKnob,
                            Node {
                                position_type: PositionType::Absolute,
                                left: percent(0),
                                top: px((Self::TRACK_HEIGHT - Self::KNOB_SIZE) * 0.5),
                                margin: UiRect::left(px(-Self::KNOB_SIZE * 0.5)),
                                width: px(Self::KNOB_SIZE),
                                height: px(Self::KNOB_SIZE),
                                border_radius: BorderRadius::all(px(Self::KNOB_SIZE * 0.5)),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                        ));
                    });

                    // Puts the camera back on the gantry. The rig can now be
                    // flown clear of the ground (`CameraFlight`), and a free
                    // camera without a way home is a way to lose the match —
                    // so this is the one camera control that is not a gesture
                    // over the canvas but a button that is always in view.
                    //
                    // Labelled in English rather than through `ViewerLabels`:
                    // the viewer draws with Bevy's built-in font, which
                    // carries ASCII and nothing else, so a translated string
                    // would come out as blank boxes in most of the locales
                    // that asked for it.
                    bar.spawn((
                        CameraResetButton,
                        Interaction::default(),
                        Self::chip(84.0),
                        BackgroundColor(Self::CHIP_BACKGROUND),
                    ))
                    .with_child((
                        Text::new("Reset view"),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                    ));

                    // Playback speed. NOT debug-only: slow motion is what
                    // a replay is for. The cycle runs 0.25x through 16x
                    // (see `Playback::SPEEDS`) — the slow half to watch a
                    // decision actually happen, the fast half to skim.
                    bar.spawn((
                        SpeedButton,
                        Interaction::default(),
                        // Wide enough for the longest label the cycle can
                        // produce ("0.25x"); 40 px fitted only the
                        // whole-number speeds it used to carry.
                        Self::chip(52.0),
                        BackgroundColor(Self::CHIP_BACKGROUND),
                    ))
                    .with_child((
                        SpeedLabel,
                        Text::new("1x"),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));

                    if config.debug {
                        bar.spawn((
                            StatesButton,
                            Interaction::default(),
                            Self::chip(58.0),
                            BackgroundColor(Self::CHIP_ACTIVE),
                        ))
                        .with_child((
                            Text::new("States"),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));

                        // Camera zoom readout. The chips that used to sit
                        // in front of it are gone: the wheel does this
                        // now (`CameraZoom::handle_wheel`), which is where
                        // a hand already is while watching, and two
                        // buttons to nudge a lens are a poor substitute
                        // for turning it. The number stays — it is the
                        // only feedback that the wheel did anything when
                        // the shot is of open pitch.
                        bar.spawn((
                            ZoomLabel,
                            Text::new("1.00x"),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.64, 0.72, 0.80)),
                            TextLayout {
                                linebreak: LineBreak::NoWrap,
                                ..default()
                            },
                            Node {
                                width: px(52),
                                flex_shrink: 0.0,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                        ));
                    }

                    bar.spawn((
                        ClockLabel,
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.88, 0.91, 0.96)),
                        // The half label is a translated string, and some
                        // locales are long — let it run rather than wrap the
                        // clock onto two lines.
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                        Node {
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::FlexEnd,
                            ..default()
                        },
                    ));
                });
            });

        commands.spawn((
            LoadingNotice,
            Text::new(config.labels.loading.clone()),
            TextFont {
                font_size: FontSize::Px(15.0),
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.89, 0.95)),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: percent(46),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ));
    }

    /// Shared shape of the small debug buttons that sit either side of the
    /// scrub track.
    fn chip(width: f32) -> Node {
        Node {
            width: px(width),
            height: px(22),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_shrink: 0.0,
            border_radius: BorderRadius::all(px(4)),
            ..default()
        }
    }

    /// Click or drag anywhere on the track to scrub. Dragging works because the
    /// seek reads the held button rather than a press edge.
    pub fn handle_seek(
        track: Single<&RelativeCursorPosition, With<SeekTrack>>,
        mouse: Res<ButtonInput<MouseButton>>,
        mut playback: ResMut<Playback>,
    ) {
        if !mouse.pressed(MouseButton::Left) || !track.cursor_over() {
            return;
        }
        // `normalized` runs -0.5 .. 0.5 across the node.
        if let Some(position) = track.normalized {
            playback.seek_to(position.x + 0.5);
        }
    }

    pub fn handle_toggle(
        toggle: Query<&Interaction, (Changed<Interaction>, With<PlayToggle>)>,
        mut playback: ResMut<Playback>,
    ) {
        for interaction in &toggle {
            if *interaction != Interaction::Pressed {
                continue;
            }
            let finished = playback.time_ms >= playback.duration_ms;
            if finished {
                playback.seek_to(0.0);
                playback.playing = true;
            } else {
                playback.playing = !playback.playing;
            }
        }
    }

    /// Puts the camera back where the replay opened: on the gantry, at rest
    /// zoom, following the ball again.
    ///
    /// Runs ahead of the camera systems so a click lands on the frame it
    /// happened rather than the next one — the same reason the orbit drag
    /// sits where it does.
    pub fn handle_camera_reset(
        button: Query<&Interaction, (Changed<Interaction>, With<CameraResetButton>)>,
        mut orbit: ResMut<CameraOrbit>,
        mut zoom: ResMut<CameraZoom>,
        mut flight: ResMut<CameraFlight>,
    ) {
        if button.iter().any(|i| *i == Interaction::Pressed) {
            CameraRig::reset(&mut orbit, &mut zoom, &mut flight);
        }
    }

    /// Cycles playback speed.
    ///
    /// Registered unconditionally, unlike the debug controls below: the
    /// speed chip is part of the transport bar in the game as well, so
    /// its handler has to run there. Leaving it in `handle_debug_controls`
    /// would draw a button that does nothing.
    pub fn handle_speed(
        speed: Query<&Interaction, (Changed<Interaction>, With<SpeedButton>)>,
        mut playback: ResMut<Playback>,
    ) {
        if speed.iter().any(|i| *i == Interaction::Pressed) {
            playback.cycle_speed();
        }
    }

    /// Keeps the speed chip's label in step. Also unconditional — see
    /// [`Self::handle_speed`].
    pub fn refresh_speed(
        playback: Res<Playback>,
        mut speed: Query<&mut Text, With<SpeedLabel>>,
    ) {
        if let Ok(mut text) = speed.single_mut() {
            let wanted = playback.speed_label();
            if text.as_str() != wanted {
                **text = wanted;
            }
        }
    }

    /// Flips the state labels. Debug overlay only — the button does not
    /// exist otherwise.
    pub fn handle_debug_controls(
        states: Query<&Interaction, (Changed<Interaction>, With<StatesButton>)>,
        mut overlay: ResMut<DebugOverlay>,
    ) {
        if states.iter().any(|i| *i == Interaction::Pressed) {
            overlay.states = !overlay.states;
        }
    }

    pub fn refresh_debug(
        overlay: Res<DebugOverlay>,
        _ball: Res<BallState>,
        mut states: Query<&mut BackgroundColor, With<StatesButton>>,
        zoom: Res<CameraZoom>,
        mut readout: Query<&mut Text, With<ZoomLabel>>,
    ) {
        if let Ok(mut background) = states.single_mut() {
            let wanted = if overlay.states {
                Self::CHIP_ACTIVE
            } else {
                Self::CHIP_BACKGROUND
            };
            background.set_if_neq(BackgroundColor(wanted));
        }
        if let Ok(mut text) = readout.single_mut() {
            let wanted = format!("{:.2}x", zoom.factor);
            if text.as_str() != wanted {
                **text = wanted;
            }
        }
    }

    pub fn refresh(
        playback: Res<Playback>,
        loader: Res<ChunkLoader>,
        config: Res<ViewerConfig>,
        mut fill: Query<&mut Node, (With<SeekFill>, Without<SeekKnob>)>,
        mut knob: Query<&mut Node, (With<SeekKnob>, Without<SeekFill>)>,
        mut clock: Query<&mut Text, (With<ClockLabel>, Without<PlayToggleIcon>)>,
        mut icon: Query<&mut Text, (With<PlayToggleIcon>, Without<ClockLabel>)>,
        mut notice: Query<&mut Visibility, With<LoadingNotice>>,
    ) {
        let progress = playback.progress() * 100.0;
        if let Ok(mut node) = fill.single_mut() {
            node.width = percent(progress);
        }
        if let Ok(mut node) = knob.single_mut() {
            node.left = percent(progress);
        }
        if let Ok(mut text) = clock.single_mut() {
            **text = playback.clock_label(&config.labels);
        }
        if let Ok(mut text) = icon.single_mut() {
            let wanted = if playback.playing { "II" } else { ">" };
            if text.as_str() != wanted {
                **text = wanted.to_string();
            }
        }
        if let Ok(mut visibility) = notice.single_mut() {
            *visibility = if loader.ready {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }

    /// Lights the reset chip whenever the shot is not the one the replay
    /// opened on.
    ///
    /// On a canvas of open pitch there is nothing else that says the camera
    /// has been moved, and someone who has flown behind a stand and lost the
    /// ball needs telling which button gets it back.
    pub fn refresh_camera_reset(
        orbit: Res<CameraOrbit>,
        zoom: Res<CameraZoom>,
        flight: Res<CameraFlight>,
        mut reset: Query<&mut BackgroundColor, With<CameraResetButton>>,
    ) {
        if let Ok(mut background) = reset.single_mut() {
            let wanted = if CameraRig::moved(&orbit, &zoom, &flight) {
                Self::CHIP_ACTIVE
            } else {
                Self::CHIP_BACKGROUND
            };
            background.set_if_neq(BackgroundColor(wanted));
        }
    }
}
