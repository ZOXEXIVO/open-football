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
//! White team panels with club-coloured banners, prominent surnames and
//! smaller first names. The bench is a compact two-column grid.
//!
//! It is laid out once at one size and then scaled into whatever picture the
//! page gave the canvas — see [`TeamSheet::fit_frame`], which is what keeps the
//! whole of it on a laptop's canvas and off the transport bar on a phone's.

use crate::app::config::{PlayerInfo, ViewerConfig};
use crate::art::typeface::Faces;
use crate::broadcast::lineup::Lineup;
use crate::ui::plate::Plate;
use crate::ui::timeline::Timeline;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::text::{FontSource, LineBreak};
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;

/// The sheet the card is centred on, and the only thing switched on and off.
#[derive(Component)]
pub struct SheetScreen;

/// The two panels together — the card, as opposed to the tint behind it.
///
/// The scrim is the frame and this is what stands in it, which is why they are
/// two entities: the tint covers the whole picture whatever size it is, and the
/// card is only ever as big as the picture can hold ([`TeamSheet::fit_frame`]).
#[derive(Component)]
pub struct SheetCard;

/// The card the walk-out is watched over.
pub struct TeamSheet;

impl TeamSheet {
    const SCRIM: Color = Color::srgba(0.025, 0.035, 0.050, 0.30);
    const PANEL: Color = Color::srgb(0.99, 0.99, 0.985);
    const BENCH: Color = Color::srgb(0.945, 0.952, 0.958);
    const INK: Color = Plate::INK;
    const MUTED: Color = Color::srgb(0.39, 0.43, 0.47);
    const RULE: Color = Color::srgba(0.10, 0.15, 0.20, 0.075);

    const PANEL_WIDTH: f32 = 760.0;
    const GUTTER: f32 = 18.0;

    /// **Set to be read at a glance**, with the panel sized to match so the
    /// names keep their room.
    ///
    /// A size up from the other two cards, and deliberately. Those are read at
    /// leisure: the score plate sits still in the corner all match and the
    /// full-time card comes up on a stopped picture. This one is over a camera
    /// flying at the line and it is gone inside fifteen seconds, so it has to
    /// be taken in rather than studied — which is a bigger face, not a longer
    /// read.
    const CLUB: f32 = 19.0;
    const NAME: f32 = 15.0;
    const NUMBER: f32 = 19.0;
    /// Room for two figures, because shirt numbers run past ninety.
    const NUMBERS: f32 = 28.0;
    const LABEL: f32 = 10.6;

    /// The most substitutes listed under one side.
    ///
    /// A bench is seven in most competitions and nine in some, and a document
    /// is free to carry more — but the card is centred over a moving camera
    /// with nowhere to scroll, so it stops at the deepest bench a competition
    /// actually names. Eleven starters and nine substitutes stand about 610 px
    /// tall, with the substitutes in two columns: that and [`Self::PANEL_WIDTH`]
    /// are the card's full size, and [`Self::fit_frame`] is what gets it into a
    /// picture smaller than that.
    const MOST_SUBS: usize = 9;

    /// Builds the card, hidden, at startup.
    ///
    /// Everything on it is known before the first frame — a team sheet is the
    /// one thing about a match that is settled before it starts — so this is
    /// built once and never touched again, the same rule the full-time card
    /// keeps.
    pub fn spawn(mut commands: Commands, config: Res<ViewerConfig>, faces: Res<Faces>) {
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
                    // The transport bar is furniture the card may not cover,
                    // so the box it is centred in is the picture above the bar
                    // rather than the whole canvas.
                    padding: UiRect::all(px(Self::GUTTER))
                        .with_bottom(px(Self::GUTTER + Timeline::BAR_HEIGHT)),
                    ..default()
                },
                BackgroundColor(Self::SCRIM),
                FocusPolicy::Pass,
                Visibility::Hidden,
            ))
            .with_children(|over| {
                over.spawn((
                    SheetCard,
                    UiTransform::IDENTITY,
                    Node {
                        // The width it was drawn at, held against a frame of
                        // any size: a card that shrank its own columns would
                        // squeeze the names into a clip rather than set them
                        // smaller, and every measurement below is a fixed
                        // pixel figure that assumes this one. What a narrow
                        // canvas gets instead is the whole card, scaled.
                        width: px(Self::PANEL_WIDTH),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Row,
                        column_gap: px(12),
                        ..default()
                    },
                ))
                .with_children(|card| {
                    for home in [true, false] {
                        Self::side(card, &faces, &config, home, &bench);
                    }
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
        let kit = if home { &config.home } else { &config.away };
        let shirt = kit.background_color(if home {
            Color::srgb(0.0, 0.19, 0.49)
        } else {
            Color::srgb(0.70, 0.25, 0.0)
        });
        let ink = kit.foreground_color(Color::WHITE);
        let named: Vec<&PlayerInfo> = config
            .players
            .iter()
            .filter(|player| player.is_home == home)
            .collect();

        body.spawn((
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::bottom(px(8)),
                border_radius: BorderRadius::all(px(6)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Self::PANEL),
            BoxShadow(vec![ShadowStyle {
                color: Color::srgba(0.0, 0.0, 0.0, 0.22),
                y_offset: px(10),
                blur_radius: px(24),
                ..default()
            }]),
        ))
        .with_children(|column| {
            column
                .spawn((
                    Node {
                        width: percent(100),
                        height: px(58),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(px(18), px(8)),
                        margin: UiRect::bottom(px(8)),
                        overflow: Overflow::clip(),
                        border: UiRect::bottom(px(2)),
                        ..default()
                    },
                    BackgroundColor(shirt),
                    BorderColor::all(ink.with_alpha(0.25)),
                ))
                .with_children(|header| {
                    header.spawn((
                        Text::new(club.clone()),
                        TextFont {
                            font: FontSource::Handle(faces.face_for(&club)),
                            font_size: FontSize::Px(Self::CLUB),
                            ..default()
                        },
                        TextColor(ink),
                        TextLayout {
                            linebreak: LineBreak::NoWrap,
                            ..default()
                        },
                        Node {
                            width: percent(100),
                            min_width: px(0),
                            ..default()
                        },
                    ));
                });

            for player in named.iter().filter(|player| player.starting) {
                Self::man(column, faces, player, false);
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
            column
                .spawn((
                    Node {
                        width: percent(100),
                        height: px(28),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(px(18)),
                        margin: UiRect::top(px(4)),
                        border: UiRect::top(px(1)),
                        ..default()
                    },
                    BorderColor::all(Self::RULE),
                    BackgroundColor(Self::BENCH),
                ))
                .with_children(|heading| {
                    heading.spawn((
                        Text::new(bench.to_string()),
                        TextFont {
                            font: FontSource::Handle(faces.face_for(bench)),
                            font_size: FontSize::Px(Self::LABEL),
                            ..default()
                        },
                        TextColor(Self::MUTED),
                    ));
                });
            column
                .spawn((
                    Node {
                        width: percent(100),
                        display: Display::Grid,
                        grid_template_columns: RepeatedGridTrack::flex(2, 1.0),
                        padding: UiRect::horizontal(px(6)),
                        ..default()
                    },
                    BackgroundColor(Self::BENCH),
                ))
                .with_children(|bench| {
                    for player in subs {
                        Self::man(bench, faces, player, true);
                    }
                });
        });
    }

    /// One man: a number beside a small first name and a prominent surname.
    fn man(
        column: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        player: &PlayerInfo,
        substitute: bool,
    ) {
        column
            .spawn((
                Node {
                    width: percent(100),
                    min_width: px(0),
                    height: px(32),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(if substitute { 6.0 } else { 12.0 }),
                    padding: UiRect::horizontal(px(if substitute { 6.0 } else { 18.0 })),
                    border: UiRect::bottom(px(1)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BorderColor::all(Self::RULE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(player.shirt_number.to_string()),
                    TextFont {
                        font: FontSource::Handle(faces.face_for("0123456789")),
                        font_size: FontSize::Px(if substitute { 14.0 } else { Self::NUMBER }),
                        ..default()
                    },
                    TextColor(Self::MUTED),
                    TextLayout::justify(Justify::Center),
                    Node {
                        width: px(if substitute { 22.0 } else { Self::NUMBERS }),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
                row.spawn(Node {
                    min_width: px(0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_children(|name| {
                    if !player.first_name.trim().is_empty() {
                        Self::name(name, faces, &player.first_name, 9.0, Self::MUTED);
                    }
                    Self::name(
                        name,
                        faces,
                        &player.last_name.to_uppercase(),
                        if substitute { 11.5 } else { Self::NAME },
                        Self::INK,
                    );
                });
            });
    }

    fn name(
        parent: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        text: &str,
        size: f32,
        color: Color,
    ) {
        parent.spawn((
            Text::new(text),
            TextFont {
                font: FontSource::Handle(faces.face_for(text)),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
            TextLayout {
                linebreak: LineBreak::NoWrap,
                ..default()
            },
            Node {
                min_width: px(0),
                ..default()
            },
        ));
    }

    /// Scales the card down into whatever picture the page gave the canvas.
    ///
    /// Every measurement on the sheet is a fixed pixel size settled against a
    /// frame of around eight hundred by seven hundred, and anything smaller —
    /// a laptop window, a phone in portrait — had the card running off the top
    /// and the bottom of the picture and sitting over the transport bar. One
    /// factor off the tighter axis keeps the proportions it was drawn with,
    /// where a second set of sizes would be a second set of numbers to keep in
    /// step. A [`UiTransform`] rather than those sizes for the reason the
    /// full-time card's lift is one: it moves nothing inside the card, where a
    /// written size has taffy re-solve two panels and forty lines of text.
    /// Never scaled UP: past its own size there is nothing to gain.
    pub fn fit_frame(
        window: Single<&Window, With<PrimaryWindow>>,
        card: Single<(&ComputedNode, &mut UiTransform), With<SheetCard>>,
    ) {
        let (measured, mut sizing) = card.into_inner();
        // Logical pixels: the unit the window reports its size in, and the one
        // the layout above is written in.
        let full = measured.size * measured.inverse_scale_factor;
        let canvas = Vec2::new(window.width(), window.height());
        let fit = Vec2::splat(Self::factor(full, canvas));
        if sizing.scale != fit {
            sizing.scale = fit;
        }
    }

    /// How much of its full size the card is drawn at on a canvas this big.
    ///
    /// Zero before the first layout and zero on a canvas with no room under the
    /// bar: nothing is drawn, which is the honest answer to how much of it fits.
    fn factor(full: Vec2, canvas: Vec2) -> f32 {
        if full.cmple(Vec2::ZERO).any() {
            return 0.0;
        }
        let frame = Vec2::new(
            canvas.x - 2.0 * Self::GUTTER,
            canvas.y - Timeline::BAR_HEIGHT - 2.0 * Self::GUTTER,
        );
        (frame / full).min_element().clamp(0.0, 1.0)
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

/// The card's one measurement that is arithmetic rather than layout, asked
/// without a screen to draw on.
#[cfg(test)]
mod tests {
    use super::*;

    /// The full size the card is built at: two panels of eleven and nine, the
    /// deepest sheet a competition names. The height is what those rows add up
    /// to — a banner, eleven men, a bench heading and five paired rows.
    const FULL: Vec2 = Vec2::new(TeamSheet::PANEL_WIDTH, 618.0);

    #[test]
    fn full_size_on_a_frame_that_holds_it() {
        assert_eq!(TeamSheet::factor(FULL, Vec2::new(1280.0, 720.0)), 1.0);
    }

    /// A phone in portrait: the page gives the replay a 16:9 strip the width of
    /// the screen, which is a quarter of the height the card is drawn at. It is
    /// the height that binds, and what comes back has to leave the transport
    /// bar showing.
    #[test]
    fn a_phone_gets_the_whole_card_inside_the_picture() {
        let canvas = Vec2::new(366.0, 206.0);
        let drawn = FULL * TeamSheet::factor(FULL, canvas);

        assert!(drawn.x <= canvas.x - 2.0 * TeamSheet::GUTTER);
        assert!(drawn.y <= canvas.y - Timeline::BAR_HEIGHT - 2.0 * TeamSheet::GUTTER);
    }

    /// One factor for both axes, or the names come out stretched.
    #[test]
    fn the_tighter_axis_sets_it() {
        let canvas = Vec2::new(900.0, 500.0);
        let fit = TeamSheet::factor(FULL, canvas);

        let across = (canvas.x - 2.0 * TeamSheet::GUTTER) / FULL.x;
        let down = (canvas.y - Timeline::BAR_HEIGHT - 2.0 * TeamSheet::GUTTER) / FULL.y;
        assert!(down < across);
        assert_eq!(fit, down);
    }

    /// Nothing to scale yet, and nothing to scale into.
    #[test]
    fn no_card_and_no_room_both_draw_nothing() {
        assert_eq!(TeamSheet::factor(Vec2::ZERO, Vec2::new(1280.0, 720.0)), 0.0);
        assert_eq!(TeamSheet::factor(FULL, Vec2::new(320.0, 60.0)), 0.0);
    }
}
