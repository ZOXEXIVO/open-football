//! **The two team sheets, over the walk-out.**
//!
//! The ceremony before the first whistle opens high over the centre spot and
//! flies down at a line of forty-odd men standing on the touchline (see
//! [`Lineup`]). From up there nobody is legible and nothing says who is
//! playing — which is exactly the stretch a broadcast fills with the two
//! sheets, because it is the only time in a match when the question "who is on
//! the pitch" has not yet been answered by watching.
//!
//! So the card is up for the aerial, and gone by the time the camera comes
//! round onto the faces. That hand-over is the whole of its timing and it is
//! [`Lineup::presenting`]'s to state; nothing in here has a clock of its own.
//!
//! ⚠ **It is hidden rather than faded.** The other two cards come up and go
//! down on a ramp, because they land on a still picture where a cut would read
//! as a glitch. This one leaves on the frame the camera reaches the first face
//! — the busiest moment of the whole ceremony, a shot coming out of a corner
//! and running down a line of men — and a card dissolving across that is one
//! more thing moving in a frame that already has plenty.
//!
//! It is the same plate the score and the full-time card are cut from — see
//! [`Plate`] — because a viewer should not have to learn two of them.

use crate::app::config::{PlayerInfo, ViewerConfig};
use crate::art::typeface::Faces;
use crate::broadcast::lineup::Lineup;
use crate::ui::plate::Plate;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::text::{FontSource, LineBreak};
use bevy::ui::FocusPolicy;

/// The sheet the card is centred on, and the only thing switched on and off.
#[derive(Component)]
pub struct SheetScreen;

/// The card the walk-out is watched over.
pub struct TeamSheet;

impl TeamSheet {
    /// The dark the picture goes down behind, and the panel over it. The
    /// full-time card's own two — see
    /// [`FullTime`](crate::ui::scoreboard::FullTime), which explains why a
    /// white card wants a dark scrim under it.
    const SCRIM: Color = Color::srgba(0.031, 0.047, 0.071, 0.62);
    const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);

    const PANEL_WIDTH: f32 = 728.0;
    const GUTTER: f32 = 18.0;

    /// **Set to be read at a glance**, and the panel widened to match so the
    /// names keep their room.
    ///
    /// Far larger than the other two cards, and deliberately. Those are read at
    /// leisure: the score plate sits still in the corner all match and the
    /// full-time card comes up on a stopped picture. This one is over a camera
    /// flying at the line and it is gone inside fifteen seconds, so it has to
    /// be taken in rather than studied — which is a bigger face, not a longer
    /// read.
    const CLUB: f32 = 20.3;
    const NAME: f32 = 18.6;
    const NUMBER: f32 = 16.9;
    /// Room for two figures, because shirt numbers run past ninety.
    const NUMBERS: f32 = 27.0;
    const LABEL: f32 = 15.2;

    /// The most substitutes listed under one side.
    ///
    /// ⚠ **This is what the type size costs.** A bench is seven in most
    /// competitions and nine in some, and a document is free to carry more —
    /// but the card is centred over a moving camera with nowhere to scroll,
    /// and at this size a ninth row runs it off the bottom of the picture.
    /// Measured against the canvas: eleven and seven comes to about 645 px of
    /// a 682 px frame, and every further man is another 29. The eleven above
    /// them are what a walk-out is about anyway.
    const MOST_SUBS: usize = 7;

    /// Builds the card, hidden, at startup.
    ///
    /// Everything on it is known before the first frame — a team sheet is the
    /// one thing about a match that is settled before it starts — so this is
    /// built once and never touched again, the same rule the full-time card
    /// keeps.
    pub fn spawn(mut commands: Commands, config: Res<ViewerConfig>, faces: Res<Faces>) {
        let home_shirt = config.home.background_color(Color::srgb(0.0, 0.19, 0.49));
        let away_shirt = config.away.background_color(Color::srgb(0.70, 0.25, 0.0));
        let bench = config.labels.substitutes.to_uppercase();

        commands
            .spawn((
                SheetScreen,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    padding: UiRect::all(px(Self::GUTTER)),
                    ..default()
                },
                BackgroundColor(Self::SCRIM),
                FocusPolicy::Pass,
                Visibility::Hidden,
            ))
            .with_children(|over| {
                over.spawn((
                    Node {
                        width: percent(100),
                        max_width: px(Self::PANEL_WIDTH),
                        flex_direction: FlexDirection::Column,
                        border_radius: BorderRadius::all(px(Plate::CORNER)),
                        ..default()
                    },
                    BackgroundColor(Self::PANEL),
                ))
                .with_children(|card| {
                    card.spawn(Node {
                        width: percent(100),
                        height: px(Plate::BAND),
                        flex_direction: FlexDirection::Row,
                        flex_shrink: 0.0,
                        padding: UiRect::axes(px(Plate::CORNER), px(0)),
                        ..default()
                    })
                    .with_children(|band| {
                        for shirt in [home_shirt, away_shirt] {
                            band.spawn((
                                Node {
                                    width: percent(50),
                                    height: percent(100),
                                    ..default()
                                },
                                BackgroundColor(shirt),
                            ));
                        }
                    });

                    card.spawn(Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Row,
                        column_gap: px(22),
                        padding: UiRect::axes(px(24), px(20)),
                        ..default()
                    })
                    .with_children(|body| {
                        for home in [true, false] {
                            Self::side(body, &faces, &config, home, &bench);
                        }
                    });
                });
            });
    }

    /// One club's sheet: its name, its eleven, and its bench under a rule.
    fn side(
        body: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        config: &ViewerConfig,
        home: bool,
        bench: &str,
    ) {
        let club = if home {
            config.home_name.to_uppercase()
        } else {
            config.away_name.to_uppercase()
        };
        let named: Vec<&PlayerInfo> = config
            .players
            .iter()
            .filter(|player| player.is_home == home)
            .collect();

        body.spawn(Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .with_children(|column| {
            column.spawn((
                Text::new(club.clone()),
                TextFont {
                    font: FontSource::Handle(faces.face_for(&club)),
                    font_size: FontSize::Px(Self::CLUB),
                    ..default()
                },
                TextColor(Plate::INK),
                TextLayout {
                    linebreak: LineBreak::NoWrap,
                    ..default()
                },
            ));
            column.spawn((
                Node {
                    width: percent(100),
                    height: px(1),
                    margin: UiRect::axes(px(0), px(7)),
                    ..default()
                },
                BackgroundColor(Plate::HAIRLINE),
            ));

            for player in named.iter().filter(|player| player.starting) {
                Self::man(column, faces, player);
            }

            // A bench with nobody on it says nothing worth a heading.
            let subs: Vec<&&PlayerInfo> = named
                .iter()
                .filter(|player| !player.starting)
                .take(Self::MOST_SUBS)
                .collect();
            if subs.is_empty() {
                return;
            }
            column.spawn((
                Text::new(bench.to_string()),
                TextFont {
                    font: FontSource::Handle(faces.face_for(bench)),
                    font_size: FontSize::Px(Self::LABEL),
                    ..default()
                },
                TextColor(Plate::INK_MUTED),
                Node {
                    margin: UiRect::new(px(0), px(0), px(9), px(5)),
                    ..default()
                },
            ));
            for player in subs {
                Self::man(column, faces, player);
            }
        });
    }

    /// One man: his shirt number, then his name.
    fn man(column: &mut RelatedSpawnerCommands<ChildOf>, faces: &Faces, player: &PlayerInfo) {
        column
            .spawn(Node {
                width: percent(100),
                flex_direction: FlexDirection::Row,
                column_gap: px(7),
                margin: UiRect::bottom(px(5)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new(player.shirt_number.to_string()),
                    TextFont {
                        font: FontSource::Handle(faces.face_for("0123456789")),
                        font_size: FontSize::Px(Self::NUMBER),
                        ..default()
                    },
                    TextColor(Plate::INK_MUTED),
                    TextLayout::justify(Justify::Right),
                    Node {
                        width: px(Self::NUMBERS),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
                row.spawn((
                    Text::new(player.last_name.clone()),
                    TextFont {
                        font: FontSource::Handle(faces.face_for(&player.last_name)),
                        font_size: FontSize::Px(Self::NAME),
                        ..default()
                    },
                    TextColor(Plate::INK_SOFT),
                    TextLayout {
                        linebreak: LineBreak::NoWrap,
                        ..default()
                    },
                    Node {
                        min_width: px(0),
                        ..default()
                    },
                ));
            });
    }

    /// Shows the card for the ceremony's opening beats and hides it for the
    /// rest of the match.
    ///
    /// Behind [`Lineup::hold`], which is what advances the act this reads — a
    /// frame's lag would leave the sheet over the first face.
    pub fn follow_ceremony(
        lineup: Res<Lineup>,
        screen: Single<&mut Visibility, With<SheetScreen>>,
    ) {
        screen.into_inner().set_if_neq(if lineup.presenting() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
