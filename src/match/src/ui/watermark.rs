//! **Whose replay this is**: the project's mark, in the corner furthest from
//! the score.
//!
//! It began as a second row on the score plate, under the two clubs, with the
//! address beside it. That put a signature inside the one piece of furniture on
//! the screen that has to be read at a glance, and made the plate two rows tall
//! to carry something that never changes — so it is its own label now, diagonally
//! across the picture from the score and above the transport bar, which is the
//! one corner nothing else wants: the score has the top left, the frame counter
//! the top right, and the flight stick the bottom left.
//!
//! **The address is not on it.** `open-football.org` set beside the mark is a
//! line of type nobody reads during a match and the only thing on the label
//! that could not survive being made faint: the mark carries its own ground and
//! stays crisp at any transparency, where lettering over a stand full of people
//! does not. The mark alone says the same thing and asks for a fifth of the
//! corner to say it.
//!
//! Nothing drives it. It is the same fact for all ninety minutes, so it is
//! built once at startup and never looked at again: no system, no per-frame
//! cost, and nothing to get out of step with the replay. Furniture, at the
//! default depth with the transport bar, so a cut never dims it.

use crate::art::typeface::Faces;
use crate::ui::timeline::Timeline;
use bevy::prelude::*;
use bevy::text::FontSource;

/// The label in the corner.
pub struct Watermark;

impl Watermark {
    /// Where it sits, and how much of the corner it takes. Square, now that
    /// there is one square thing on it.
    const MARGIN: f32 = 12.0;
    const HEIGHT: f32 = 24.0;
    const CORNER: f32 = 6.0;
    /// Air between the mark and the edge of the plate it sits on.
    const BREATH: f32 = 5.0;

    /// The score plate's own white, at not quite half of it: the score has to
    /// be read, this only has to be there. See
    /// [`Scoreboard::PANEL`](crate::ui::scoreboard::Scoreboard).
    const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.4);

    /// **The mark, as the favicon draws it**: a rounded square in `#0e637f`
    /// with the initials in white. Set as nodes rather than cut as a texture —
    /// it is two rectangles and two letters, and the crate already carries the
    /// face.
    const MARK: f32 = 14.0;
    /// `favicon.svg` rounds a 64 box by 10; this is the same share of a smaller
    /// one. The hoardings round their own copy by it too.
    const MARK_CORNER: f32 = Self::MARK * 10.0 / 64.0;
    const MARK_TEXT: f32 = 7.5;
    const MARK_GROUND: Color = Color::srgb(0.055, 0.388, 0.498);

    pub fn spawn(mut commands: Commands, faces: Res<Faces>) {
        commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: px(Self::MARGIN),
                    // Clear of the transport bar rather than over it, the same
                    // way the flight stick clears it in the other corner.
                    bottom: px(Timeline::BAR_HEIGHT + Self::MARGIN),
                    width: px(Self::MARK + Self::BREATH * 2.0),
                    height: px(Self::HEIGHT),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border_radius: BorderRadius::all(px(Self::CORNER)),
                    ..default()
                },
                BackgroundColor(Self::PANEL),
            ))
            .with_children(|label| {
                label
                    .spawn((
                        Node {
                            width: px(Self::MARK),
                            height: px(Self::MARK),
                            flex_shrink: 0.0,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border_radius: BorderRadius::all(px(Self::MARK_CORNER)),
                            ..default()
                        },
                        BackgroundColor(Self::MARK_GROUND),
                    ))
                    .with_child((
                        Text::new("OF"),
                        TextFont {
                            font: FontSource::Handle(faces.face_for("OF")),
                            font_size: FontSize::Px(Self::MARK_TEXT),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
            });
    }
}
