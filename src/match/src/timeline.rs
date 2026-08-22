use crate::actors::BallState;
use crate::camera::{CameraFlight, CameraOrbit, CameraRig, CameraZoom};
use crate::config::ViewerConfig;
use crate::loader::ChunkLoader;
use crate::perf::FrameCost;
use crate::playback::{Playback, RecordedSpans};
use crate::quality::{Quality, Tier};
use crate::stage::Stage;
use crate::textures::Textures;
use crate::typeface::Faces;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::text::{FontSource, LineBreak};
use bevy::ui::RelativeCursorPosition;

/// The two faces of the play button, rasterised once at startup.
///
/// Held as a resource rather than looked up per frame because the toggle
/// swaps between them on every press, and re-rasterising a texture to change
/// a button's icon would be a strange way to spend a frame.
#[derive(Resource)]
pub struct TransportIcons {
    play: Handle<Image>,
    pause: Handle<Image>,
}

#[derive(Component)]
pub struct SeekTrack;

#[derive(Component)]
pub struct SeekFill;

/// Holds the grey bars covering the stretches of match that were never
/// recorded. A container rather than loose children so the bars can be
/// rebuilt when the metadata lands without disturbing the goal markers and
/// the knob, which are spawned after it and have to stay on top.
#[derive(Component)]
pub struct GapOverlay;

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

/// One of the small labelled buttons flanking the scrub track.
///
/// `armed` is the button's own opinion of itself — the camera reset lights up
/// once the shot has been moved, the states toggle while the labels are on —
/// and it is kept here rather than painted directly by those systems because
/// hover has to compose with it. Two systems writing one `BackgroundColor`
/// means whichever ran last wins, and the button stops answering the mouse.
#[derive(Component, Default, PartialEq)]
pub struct Chip {
    pub armed: bool,
}

#[derive(Component)]
pub struct ZoomLabel;

/// The frame-cost readout that sits beside the zoom. Debug overlay only, and
/// the reason it is on the bar at all rather than in the console: the numbers
/// that matter are the ones you can watch change while flying the camera.
#[derive(Component)]
pub struct CostLabel;

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
    /// Read next door as well: [`crate::touch`] lays its controls out clear of
    /// the bar, and cuts this band out of the canvas so that a finger reaching
    /// for the scrub rail does not also swing the camera.
    pub const BAR_HEIGHT: f32 = 48.0;
    const TRACK_HEIGHT: f32 = 8.0;
    /// Height of the invisible band around the rail that actually takes the
    /// clicks. Tall enough to hit without looking, and exactly as tall as a
    /// goal marker so nothing pokes out of it.
    const TRACK_BAND: f32 = 20.0;
    const KNOB_SIZE: f32 = 13.0;
    /// A goal's pin on the rail, and a chance's. Both stay inside
    /// [`Self::TRACK_BAND`], which is the click target and is not allowed to
    /// grow: anything poking out of it is a thing you can see and cannot hit.
    /// The chance is smaller because it is the lesser event, and that is the
    /// only difference between them that the eye has to hold — the glyph says
    /// the rest.
    const MARKER_SIZE: f32 = 18.0;
    const MARKER_SIZE_CHANCE: f32 = 15.0;
    /// How much of the pin the glyph takes. Short of the edge so the shirt
    /// colour survives as a ring around it — a glyph filling its pin would
    /// leave nothing to say whose moment it was.
    const MARKER_GLYPH: f32 = 0.66;
    /// What the knob grows to while the pointer is over the track. The only
    /// thing that says the rail can be scrubbed at all — everything else in
    /// the bar is a button, and this one is a control you have to notice is
    /// draggable.
    const KNOB_SIZE_HOVER: f32 = 17.0;
    /// The play button. Same height as the chips so the bar keeps one
    /// baseline, wider than any of them so it still reads as the lead control
    /// — it earns that by being the only solid fill on the bar rather than by
    /// being a different shape or size.
    const PLAY_WIDTH: f32 = 40.0;
    const CHIP_HEIGHT: f32 = 24.0;

    /// Progress along the rail.
    const ACCENT: Color = Color::srgb(0.29, 0.68, 0.98);

    /// The play button, flat. Three steps of one green rather than a gradient
    /// and a shadow: the only feedback is the fill getting lighter under the
    /// pointer and darker under a press.
    const GO: Color = Color::srgb(0.15, 0.72, 0.40);
    const GO_HOVER: Color = Color::srgb(0.22, 0.82, 0.48);
    const GO_PRESSED: Color = Color::srgb(0.10, 0.56, 0.31);

    const BAR: Color = Color::srgba(0.035, 0.055, 0.085, 0.88);
    /// The bar's top edge. Without it the strip has no border against a pitch
    /// that is itself dark, and it reads as a smudge rather than as furniture
    /// laid over the picture.
    const HAIRLINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.10);
    const TRACK: Color = Color::srgba(1.0, 1.0, 1.0, 0.13);

    const CHIP: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
    const CHIP_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.15);
    const CHIP_PRESSED: Color = Color::srgba(1.0, 1.0, 1.0, 0.23);
    const CHIP_EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.13);
    const CHIP_EDGE_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.30);
    /// A chip that is holding something on.
    ///
    /// Blue rather than the green it used to be, now that green is the play
    /// button: one bar cannot have a colour that means both "this is the
    /// action" and "this toggle is on". Green is the thing you press, blue is
    /// the state the viewer is in — which is also what the progress along the
    /// rail is.
    const ARMED: Color = Color::srgba(0.20, 0.48, 0.80, 0.80);
    const ARMED_EDGE: Color = Color::srgba(0.46, 0.76, 1.0, 0.75);

    const INK: Color = Color::srgb(0.93, 0.95, 0.98);
    const INK_MUTED: Color = Color::srgb(0.56, 0.63, 0.73);

    /// Stretches of the match with no recording behind them. Flat, dull and
    /// opaque enough to cover the progress fill underneath — the playhead has
    /// not "watched" a gap, it has jumped it.
    const GAP: Color = Color::srgb(0.31, 0.34, 0.39);

    pub fn spawn(
        mut commands: Commands,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
        faces: Res<Faces>,
    ) {
        let home = config.home.background_color(Color::srgb(0.0, 0.19, 0.49));
        let away = config.away.background_color(Color::srgb(0.70, 0.25, 0.0));
        // The ink inside a marker. A club's foreground is the colour it prints
        // its own numbers in, so it is already the one thing guaranteed to read
        // against its shirt — which is what a marker filled with that shirt
        // needs and what a fixed white or black cannot promise across a league
        // that wears everything.
        let home_ink = config.home.foreground_color(Color::WHITE);
        let away_ink = config.away.foreground_color(Color::WHITE);
        let goal_glyph = Textures::goal_icon(&mut images);
        let chance_glyph = Textures::chance_icon(&mut images);
        let icons = TransportIcons {
            play: Textures::play_icon(&mut images),
            pause: Textures::pause_icon(&mut images),
        };
        let play_face = icons.play.clone();

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
                        padding: UiRect::axes(px(16), px(0)),
                        border: UiRect::top(px(1)),
                        ..default()
                    },
                    BackgroundColor(Self::BAR),
                    BorderColor::all(Self::HAIRLINE),
                ))
                .with_children(|bar| {
                    // The one control on the bar that is not optional, and the
                    // only one with a solid fill behind it — which is the
                    // whole of how it says so. No gradient, no shadow, no
                    // rounding to speak of.
                    bar.spawn((
                        PlayToggle,
                        Interaction::default(),
                        Node {
                            width: px(Self::PLAY_WIDTH),
                            height: px(Self::CHIP_HEIGHT),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            flex_shrink: 0.0,
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BackgroundColor(Self::GO),
                    ))
                    .with_children(|toggle| {
                        toggle.spawn((
                            PlayToggleIcon,
                            ImageNode {
                                image: play_face,
                                ..default()
                            },
                            Node {
                                width: px(12),
                                height: px(12),
                                ..default()
                            },
                        ));
                    });

                    // The rail is eight pixels tall, which is a fine thing to
                    // look at and a poor thing to hit. `SeekTrack` — what the
                    // scrub and the hover both read — is therefore this band
                    // around it rather than the rail itself. Both span the
                    // same width, so the fraction across is the same number
                    // either way and nothing downstream has to know.
                    bar.spawn((
                        SeekTrack,
                        RelativeCursorPosition::default(),
                        Node {
                            flex_grow: 1.0,
                            height: px(Self::TRACK_BAND),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_children(|band| {
                        band.spawn((
                            Node {
                                width: percent(100),
                                height: px(Self::TRACK_HEIGHT),
                                border_radius: BorderRadius::all(px(Self::TRACK_HEIGHT * 0.5)),
                                ..default()
                            },
                            BackgroundColor(Self::TRACK),
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

                            // Above the fill and below everything else: a gap has
                            // no progress worth showing through it, but it must
                            // not bury a goal marker or the knob.
                            track.spawn((
                                GapOverlay,
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(0),
                                    top: px(0),
                                    width: percent(100),
                                    height: percent(100),
                                    // The bars inside are plain rectangles; the
                                    // track is a lozenge, and a gap at either end
                                    // would otherwise square its cap off.
                                    overflow: Overflow::clip(),
                                    border_radius: BorderRadius::all(px(Self::TRACK_HEIGHT * 0.5)),
                                    ..default()
                                },
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

                            // What the recording is FOR, laid out along it: the
                            // goals, and the two or three moments a side that
                            // came closest without being one. On a goals-only
                            // recording these ARE the recording — the grey
                            // between them is the rest of the match — so the
                            // marker has to say both when something happened
                            // and what, or every clip looks alike from here.
                            //
                            // Chances first, goals over the top of them: two
                            // markers can land close enough to overlap and a
                            // goal is never the one that should go under.
                            if config.match_time_ms > 0.0 {
                                for chance in &config.chances {
                                    Self::marker(
                                        track,
                                        (chance.time / config.match_time_ms).clamp(0.0, 1.0) as f32,
                                        config.chance_belongs_to_home(chance),
                                        (home, home_ink),
                                        (away, away_ink),
                                        chance_glyph.clone(),
                                        false,
                                    );
                                }
                                for goal in &config.goals {
                                    Self::marker(
                                        track,
                                        (goal.time / config.match_time_ms).clamp(0.0, 1.0) as f32,
                                        config.goal_belongs_to_home(goal),
                                        (home, home_ink),
                                        (away, away_ink),
                                        goal_glyph.clone(),
                                        true,
                                    );
                                }
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
                                // Lifts the playhead off the rail. Without it the
                                // knob and the fill it sits at the end of read as
                                // one shape.
                                BoxShadow(vec![ShadowStyle {
                                    color: Color::srgba(0.0, 0.0, 0.0, 0.55),
                                    x_offset: px(0),
                                    y_offset: px(1),
                                    spread_radius: px(0),
                                    blur_radius: px(4),
                                }]),
                            ));
                        });
                    });

                    // Playback speed. NOT debug-only: slow motion is what
                    // a replay is for. The cycle runs 0.25x through 16x
                    // (see `Playback::SPEEDS`) — the slow half to watch a
                    // decision actually happen, the fast half to skim.
                    //
                    // Sits next to the rail, ahead of the camera control: it
                    // is part of the transport, and the two of them belong to
                    // different jobs.
                    bar.spawn((
                        SpeedButton,
                        Chip::default(),
                        Interaction::default(),
                        // Wide enough for the longest label the cycle can
                        // produce ("0.25x"); 40 px fitted only the
                        // whole-number speeds it used to carry.
                        Self::chip(52.0),
                        BackgroundColor(Self::CHIP),
                        BorderColor::all(Self::CHIP_EDGE),
                    ))
                    .with_child((
                        SpeedLabel,
                        Text::new("1x"),
                        Self::chip_font(),
                        TextColor(Self::INK),
                    ));

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
                    // that asked for it. Set upper-case, which is both the
                    // vernacular of the broadcast graphics this bar is
                    // pretending to be and the more legible of the two at
                    // eleven pixels.
                    bar.spawn((
                        CameraResetButton,
                        Chip::default(),
                        Interaction::default(),
                        Self::chip(60.0),
                        BackgroundColor(Self::CHIP),
                        BorderColor::all(Self::CHIP_EDGE),
                    ))
                    .with_child((
                        Text::new("RESET"),
                        Self::chip_font(),
                        TextColor(Self::INK),
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                    ));

                    if config.debug {
                        bar.spawn((
                            StatesButton,
                            Chip::default(),
                            Interaction::default(),
                            Self::chip(62.0),
                            BackgroundColor(Self::CHIP),
                            BorderColor::all(Self::CHIP_EDGE),
                        ))
                        .with_child((
                            Text::new("STATES"),
                            Self::chip_font(),
                            TextColor(Self::INK),
                            TextLayout {
                                linebreak: LineBreak::NoWrap,
                                ..default()
                            },
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
                            TextColor(Self::INK_MUTED),
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

                        // What the frame costs, and where. Reads
                        // `<fps> <frame ms> u<our systems> m<all of Bevy's
                        // main world> o<render + browser> <drawn>/<meshes>`
                        // — see `perf`, which explains what each of those
                        // three points at.
                        bar.spawn((
                            CostLabel,
                            Text::new(""),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                            TextColor(Self::INK_MUTED),
                            TextLayout {
                                linebreak: LineBreak::NoWrap,
                                ..default()
                            },
                            Node {
                                width: px(216),
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
                            // Chosen once from both half names rather than per
                            // frame: the text under this label changes every
                            // tick, and re-picking would rebuild the atlas with
                            // the seconds.
                            font: FontSource::Handle(faces.face_for_all([
                                config.labels.first_half.as_str(),
                                config.labels.second_half.as_str(),
                            ])),
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Self::INK),
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

        commands.insert_resource(icons);

        commands.spawn((
            LoadingNotice,
            Text::new(config.labels.loading.clone()),
            TextFont {
                font: FontSource::Handle(faces.face_for_all([
                    config.labels.loading.as_str(),
                    config.labels.no_recording.as_str(),
                ])),
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

    /// One event on the rail: a round pin in the shirt of the side it belongs
    /// to, with a glyph in that shirt's own ink saying what it was — a ball for
    /// a goal, an exclamation for a chance that stayed out.
    ///
    /// `at` is the fraction along the match. A goal's pin is the larger of the
    /// two and sits under a brighter hairline, which is the whole of how the
    /// bar ranks them: same shape, same place, one of them louder.
    ///
    /// The hairline is not trim. A kit colour is whatever the club wears and
    /// half the league wears something dark: a navy pin on a dark rail over a
    /// dark pitch is an invisible marker. A ring of white puts every kit on the
    /// same footing without touching the colour itself.
    fn marker(
        track: &mut RelatedSpawnerCommands<ChildOf>,
        at: f32,
        is_home: bool,
        home: (Color, Color),
        away: (Color, Color),
        glyph: Handle<Image>,
        scored: bool,
    ) {
        let (shirt, ink) = if is_home { home } else { away };
        let size = if scored {
            Self::MARKER_SIZE
        } else {
            Self::MARKER_SIZE_CHANCE
        };
        let edge = if scored { 0.75 } else { 0.45 };

        track
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(at * 100.0),
                    // Centred on the rail rather than hung off its top edge, so
                    // the pin marks the instant with its middle — which is
                    // where the eye reads a round shape as being.
                    top: px((Self::TRACK_HEIGHT - size) * 0.5),
                    margin: UiRect::left(px(-size * 0.5)),
                    width: px(size),
                    height: px(size),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(size * 0.5)),
                    ..default()
                },
                BackgroundColor(shirt),
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, edge)),
            ))
            .with_children(|pin| {
                pin.spawn((
                    ImageNode {
                        image: glyph,
                        color: ink,
                        ..default()
                    },
                    Node {
                        width: px(size * Self::MARKER_GLYPH),
                        height: px(size * Self::MARKER_GLYPH),
                        ..default()
                    },
                ));
            });
    }

    /// Shared shape of the small labelled buttons that sit after the scrub
    /// track.
    fn chip(width: f32) -> Node {
        Node {
            width: px(width),
            height: px(Self::CHIP_HEIGHT),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        }
    }

    fn chip_font() -> TextFont {
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        }
    }

    /// Paints every chip from its interaction and its own armed state.
    ///
    /// One system for both because they compose: an armed chip still has to
    /// answer the pointer, and a hover that simply overwrote the armed colour
    /// would make the reset button forget it was lit the moment you moved
    /// over it.
    pub fn paint_chips(
        mut chips: Query<
            (&Interaction, &Chip, &mut BackgroundColor, &mut BorderColor),
            Or<(Changed<Interaction>, Changed<Chip>)>,
        >,
    ) {
        for (interaction, chip, mut background, mut border) in &mut chips {
            let (fill, edge) = match (chip.armed, interaction) {
                (_, Interaction::Pressed) => (Self::CHIP_PRESSED, Self::CHIP_EDGE_HOVER),
                (true, Interaction::Hovered) => (Self::ARMED, Self::CHIP_EDGE_HOVER),
                (true, Interaction::None) => (Self::ARMED, Self::ARMED_EDGE),
                (false, Interaction::Hovered) => (Self::CHIP_HOVER, Self::CHIP_EDGE_HOVER),
                (false, Interaction::None) => (Self::CHIP, Self::CHIP_EDGE),
            };
            background.set_if_neq(BackgroundColor(fill));
            border.set_if_neq(BorderColor::all(edge));
        }
    }

    /// The play button's own hover and press. Separate from the chips because
    /// it is not one — it is a solid fill rather than a tint, and it has no
    /// armed state to compose with.
    pub fn paint_play(
        mut toggle: Query<
            (&Interaction, &mut BackgroundColor),
            (With<PlayToggle>, Changed<Interaction>),
        >,
    ) {
        for (interaction, mut fill) in &mut toggle {
            fill.set_if_neq(BackgroundColor(match interaction {
                Interaction::Pressed => Self::GO_PRESSED,
                Interaction::Hovered => Self::GO_HOVER,
                Interaction::None => Self::GO,
            }));
        }
    }

    /// Click or drag anywhere on the track to scrub. Dragging works because the
    /// seek reads the held button rather than a press edge.
    ///
    /// A finger counts as the left button, and has to: `bevy_ui` writes
    /// [`RelativeCursorPosition`] off the touch position when there is no
    /// cursor to read (its own buttons work that way), so the rail was already
    /// being told where a thumb was — and then ignoring it, because a
    /// touchscreen never presses `MouseButton::Left`. That made the scrub the
    /// one control on the bar a phone could not work.
    pub fn handle_seek(
        track: Single<&RelativeCursorPosition, With<SeekTrack>>,
        mouse: Res<ButtonInput<MouseButton>>,
        touches: Res<Touches>,
        spans: Res<RecordedSpans>,
        mut playback: ResMut<Playback>,
    ) {
        let held = mouse.pressed(MouseButton::Left) || touches.iter().next().is_some();
        if !held || !track.cursor_over() {
            return;
        }
        // `normalized` runs -0.5 .. 0.5 across the node.
        if let Some(position) = track.normalized {
            playback.seek_to(position.x + 0.5);
            // Dropped in a grey stretch, the knob catches on the nearest clip
            // edge instead. There is nothing to show in there, and a scrub
            // that lands on a blank pitch reads as a broken replay.
            playback.time_ms = spans.snap(playback.time_ms);
        }
    }

    /// Draws the stretches of the match with no recording behind them.
    ///
    /// Rebuilt from scratch whenever the spans change, which in practice is
    /// once: the metadata arrives, and it is the same recording for the rest
    /// of the session.
    pub fn refresh_gaps(
        mut commands: Commands,
        spans: Res<RecordedSpans>,
        config: Res<ViewerConfig>,
        overlay: Query<(Entity, Option<&Children>), With<GapOverlay>>,
        mut drawn: Local<Option<u32>>,
    ) {
        if !spans.partial() || *drawn == Some(spans.revision()) {
            return;
        }
        let duration = config.match_time_ms;
        if duration <= 0.0 {
            return;
        }
        let Ok((overlay, children)) = overlay.single() else {
            return;
        };
        *drawn = Some(spans.revision());

        if let Some(children) = children {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }

        // Walk the clips in order and draw what is between them — plus the run
        // up to the first and the run out from the last.
        let mut cursor = 0.0f64;
        let mut gaps: Vec<(f64, f64)> = Vec::new();
        for (start, end) in spans.spans() {
            if *start > cursor {
                gaps.push((cursor, *start));
            }
            cursor = cursor.max(*end);
        }
        if cursor < duration {
            gaps.push((cursor, duration));
        }

        commands.entity(overlay).with_children(|parent| {
            for (start, end) in gaps {
                let left = (start / duration).clamp(0.0, 1.0) as f32;
                let width = ((end - start) / duration).clamp(0.0, 1.0) as f32;
                if width <= 0.0 {
                    continue;
                }
                parent.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: percent(left * 100.0),
                        top: px(0),
                        width: percent(width * 100.0),
                        height: percent(100),
                        ..default()
                    },
                    BackgroundColor(Self::GAP),
                ));
            }
        });
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
    pub fn refresh_speed(playback: Res<Playback>, mut speed: Query<&mut Text, With<SpeedLabel>>) {
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
        mut states: Query<&mut Chip, With<StatesButton>>,
        zoom: Res<CameraZoom>,
        mut readout: Query<&mut Text, With<ZoomLabel>>,
        cost: Res<FrameCost>,
        quality: Res<Quality>,
        stage: Res<Stage>,
        time: Res<Time>,
        mut due: Local<f32>,
        mut costs: Query<&mut Text, (With<CostLabel>, Without<ZoomLabel>)>,
    ) {
        if let Ok(mut chip) = states.single_mut() {
            // `set_if_neq` on the component, not the colour: `paint_chips`
            // watches `Changed<Chip>`, and writing an unchanged value every
            // frame would repaint every chip on the bar every frame.
            chip.set_if_neq(Chip {
                armed: overlay.states,
            });
        }
        if let Ok(mut text) = readout.single_mut() {
            let wanted = format!("{:.2}x", zoom.factor);
            if text.as_str() != wanted {
                **text = wanted;
            }
        }

        // Four times a second, not every frame. A rolling median moves on
        // every sample, so writing it out unconditionally would re-shape a
        // line of text on each frame — which is a real cost, on the very
        // measurement it would then be inflating.
        *due -= time.delta_secs();
        if *due > 0.0 {
            return;
        }
        *due = 0.25;
        if let Ok(mut text) = costs.single_mut() {
            // The tier rides along on the same line, because the first
            // question asked of a slow frame here is which of the two
            // pictures produced it — and the answer may have changed since
            // the replay started. See `Quality::relent`.
            let wanted = format!(
                "{} {} {}",
                cost.strip(),
                match quality.tier() {
                    Tier::Multisampled => "msaa4",
                    Tier::PostProcessed => "fxaa",
                },
                stage.readout(),
            );
            if text.as_str() != wanted {
                **text = wanted;
            }
        }
    }

    pub fn refresh(
        playback: Res<Playback>,
        loader: Res<ChunkLoader>,
        spans: Res<RecordedSpans>,
        config: Res<ViewerConfig>,
        icons: Res<TransportIcons>,
        // A plain query rather than a `Single`: this system also drives the
        // clock, the fill and the loading notice, and a `Single` that failed
        // to match would skip the lot of them to save a hover effect.
        pointer: Query<&RelativeCursorPosition, With<SeekTrack>>,
        mut fill: Query<&mut Node, (With<SeekFill>, Without<SeekKnob>)>,
        mut knob: Query<&mut Node, (With<SeekKnob>, Without<SeekFill>)>,
        mut clock: Query<&mut Text, With<ClockLabel>>,
        mut icon: Query<&mut ImageNode, With<PlayToggleIcon>>,
        mut notice: Query<(&mut Visibility, &mut Text), (With<LoadingNotice>, Without<ClockLabel>)>,
    ) {
        let progress = playback.progress() * 100.0;
        if let Ok(mut node) = fill.single_mut()
            && node.width != percent(progress)
        {
            node.width = percent(progress);
        }
        if let Ok(mut node) = knob.single_mut() {
            // Guarded for the same reason as the knob's size below: a paused
            // replay, or one waiting on its first chunk, is a replay whose bar
            // does not move, and it should not be relaying out the UI to say
            // so.
            if node.left != percent(progress) {
                node.left = percent(progress);
            }
            // The knob swells under the pointer. Sized here rather than in a
            // hover system because the track carries the cursor position and
            // the knob is a sibling with no interaction of its own — it is the
            // rail that is being pointed at, not the handle.
            let over = pointer.single().is_ok_and(|track| track.cursor_over());
            let size = if over {
                Self::KNOB_SIZE_HOVER
            } else {
                Self::KNOB_SIZE
            };
            // Only when it actually changes, which is when the pointer arrives
            // and when it leaves. `Node` is change-detected and the layout
            // pass reruns for the whole tree when any node in it is touched,
            // so five unconditional writes here put the knob's hover state on
            // the bill of every frame of the match.
            if node.width != px(size) {
                node.width = px(size);
                node.height = px(size);
                node.top = px((Self::TRACK_HEIGHT - size) * 0.5);
                node.margin = UiRect::left(px(-size * 0.5));
                node.border_radius = BorderRadius::all(px(size * 0.5));
            }
        }
        if let Ok(mut text) = clock.single_mut() {
            // Compared before writing, like every other label on this bar.
            // The clock reads to the minute and this system runs on every
            // frame, so an unconditional write re-shaped a line of text three
            // thousand times for each time it changed — and text shaping is
            // the most expensive thing the UI does.
            let wanted = playback.clock_label(&config.labels);
            if text.as_str() != wanted {
                **text = wanted;
            }
        }
        if let Ok(mut node) = icon.single_mut() {
            let wanted = if playback.playing {
                &icons.pause
            } else {
                &icons.play
            };
            if node.image != *wanted {
                node.image = wanted.clone();
            }
        }
        if let Ok((mut visibility, mut text)) = notice.single_mut() {
            // A recording with no clips in it is not still loading — nothing
            // was kept and nothing ever will be, so say that instead of
            // spinning forever on "Loading…". It used to mean "goalless"; now
            // that a near miss is kept too it means a match in which neither
            // side had a shot worth the name, which is rarer and duller still.
            let empty = spans.nothing_recorded();
            *visibility = if loader.ready && !empty {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            let wanted = if empty {
                &config.labels.no_recording
            } else {
                &config.labels.loading
            };
            if text.as_str() != wanted {
                **text = wanted.clone();
            }
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
        mut reset: Query<&mut Chip, With<CameraResetButton>>,
    ) {
        if let Ok(mut chip) = reset.single_mut() {
            chip.set_if_neq(Chip {
                armed: CameraRig::moved(&orbit, &zoom, &flight),
            });
        }
    }
}
