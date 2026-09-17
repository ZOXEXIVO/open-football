//! **The score, as the replay reaches it** — in the corner while the match is
//! on, and on a card of its own once it is over.
//!
//! [`Scoreboard`] is the bug in the corner: two digits that tick over on the
//! frame the ball goes in. [`FullTime`] is what the replay ends on, and the two
//! are one module because they are one fact — the bug hands the screen to the
//! card at the final whistle, and neither may ever say a different score from
//! the other.
//!
//! # Why a replay needs one at all
//!
//! Nothing in the viewer used to say that a goal had been given. The net
//! rippled, a rustle came out of the speakers, and the match page above the
//! canvas carried the FULL-TIME score from the first frame — so a goal changed
//! no number anywhere on the screen, and the seconds after one read as play
//! continuing rather than as a goal being awarded. The pins on the seek rail
//! are no help: they are in the same places before the ball crosses the line as
//! after it, because they describe the whole match rather than the moment.
//!
//! So the bug in the corner counts the goals BEHIND the playhead, and its
//! digits tick over on the frame the ball goes in.
//!
//! # Named, at last
//!
//! Both of them say which club is which in words. The viewer spent its whole
//! life unable to: nothing in the document said what a side was CALLED, so two
//! blocks of shirt colour were the entire answer, and a viewer who did not
//! already know the fixture could not tell which of them was his.
//! `ViewerConfig::home_name` is what changed that. Nothing is truncated to make
//! it fit — the bug grows to hold the longest name it is given, because a club
//! called Borussia Mönchengladbach is not called Borussia Mönch.
//!
//! # A white panel rather than two shirts
//!
//! The bug used to BE the two kits: a block of each club's colour with its
//! figure printed on it in the club's own ink. It said whose score was whose
//! without a word, which was the whole point when there were no words to be
//! had — and once there are, two saturated blocks in the corner of a green
//! pitch are a lot of paint for a thing that now spells the answer out.
//!
//! So it is what a televised match puts there instead: one translucent white
//! plate, the names set on it in near-black, and the two figures meeting in the
//! middle. The kits have not gone anywhere — the pins on the seek rail, the men
//! on the pitch and the plates on [`FullTime`] are all still shirt-coloured.
//!
//! The project's own mark is NOT on it. It was, for a version: a second row
//! under the two clubs. That is a signature inside the one piece of furniture
//! on the screen that has to be read at a glance, and it made the plate twice
//! as tall to carry something that never changes. It has the opposite corner to
//! itself now — see [`Watermark`](crate::ui::watermark::Watermark).
//!
//! ⚠ **No colour tab either**, which was tried: a four-pixel strip of each
//! club's shirt down the ends of the plate. A club that plays in white then has
//! an invisible one, and a bug with a tab at one end and nothing at the other
//! reads as a fault rather than as a kit.
//!
//! # And it flashes
//!
//! **The figure of whoever has just scored takes his club's colour**, for as
//! long as the celebration runs — the one moment the kit is worth spending on
//! the corner of the screen, and it lands on exactly the number that has just
//! changed. The signal is [`Aftermath`]'s and not a clock of this module's own:
//! the bug lights for exactly the window the bodies are celebrating in and goes
//! out with them, so one goal is one event on the screen rather than two that
//! nearly agree.

use crate::app::config::ViewerConfig;
use crate::app::stage::Backdrop;
use crate::art::typeface::Faces;
use crate::broadcast::Grip;
use crate::broadcast::lineup::Lineup;
use crate::players::aftermath::Aftermath;
use crate::recording::playback::Playback;
use crate::ui::plate::Plate;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::text::{FontSource, LineBreak};
use bevy::ui::FocusPolicy;

/// The bug itself, so [`Scoreboard::refresh`] can take it off the screen for a
/// card that supersedes it. Two namings of the same fixture, one of them over
/// the other in the corner, is worse than either alone.
#[derive(Component)]
pub struct ScoreBug;

/// One side's figure on the plate: what it currently reads, and the kit it
/// takes while that side is celebrating.
///
/// The number is kept here rather than read back off the label because
/// [`Scoreboard::refresh`] runs every frame and the score changes three times
/// a match: an unconditional write would re-shape a line of text for every one
/// of those frames, and shaping text is the most expensive thing the interface
/// does. The same rule every label on the transport bar keeps.
///
/// The two colours are kept here for the same reason: the flash has to have
/// something to come back to, and a second copy of the config to look it up in
/// would be the more expensive half of the two.
#[derive(Component)]
pub struct ScoreDigit {
    is_home: bool,
    drawn: u32,
    /// The club's shirt, which the figure's own chip takes while he is
    /// celebrating, and its ink, which the figure takes with it.
    shirt: Color,
    ink: Color,
}

/// The score bug in the corner of the picture.
pub struct Scoreboard;

impl Scoreboard {
    /// Where it sits. Top left, which is where a televised match puts it, and
    /// clear of the transport bar along the bottom and the flight controls
    /// above that.
    const MARGIN: f32 = 12.0;
    /// How tall the plate is. Its WIDTH is whatever the two names need — see
    /// the module note on why nothing here is truncated.
    const HEIGHT: f32 = 28.0;
    const CORNER: f32 = 6.0;

    /// The plate itself: white, and see-through enough that the match goes on
    /// behind it rather than stopping at its edge.
    const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.88);
    /// What is printed on it. Near-black rather than black, which is what a
    /// broadcast caption is set in and what keeps the plate from reading as a
    /// hole punched in the picture.
    const INK: Color = Plate::INK;

    const NAME: f32 = 11.0;
    const DIGITS: f32 = 16.0;
    /// Room for two figures, because a match can finish 10-0 and a plate that
    /// reflowed at the tenth goal would move the other club's name across the
    /// screen.
    const FIGURES: f32 = 17.0;
    /// How far a name is kept from the end of the plate and from the figure
    /// beside it.
    const BREATH: f32 = 9.0;
    /// The rule between the two figures, which is what makes them a score
    /// rather than two numbers that happen to be next to each other.
    const DASH: (f32, f32) = (8.0, 2.0);
    const DASH_INK: Color = Color::srgb(0.545, 0.58, 0.62);

    pub fn spawn(mut commands: Commands, config: Res<ViewerConfig>, faces: Res<Faces>) {
        // The same two fallbacks the seek rail's markers take, so a fixture
        // with no kit on record still flashes two different colours here and
        // there rather than one colour in both places.
        let sides = [
            (
                true,
                config.home.background_color(Color::srgb(0.0, 0.19, 0.49)),
                config.home.foreground_color(Color::WHITE),
                config.home_name.as_str(),
            ),
            (
                false,
                config.away.background_color(Color::srgb(0.70, 0.25, 0.0)),
                config.away.foreground_color(Color::WHITE),
                config.away_name.as_str(),
            ),
        ];

        commands
            .spawn((
                ScoreBug,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(Self::MARGIN),
                    top: px(Self::MARGIN),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    height: px(Self::HEIGHT),
                    border_radius: BorderRadius::all(px(Self::CORNER)),
                    ..default()
                },
                BackgroundColor(Self::PANEL),
            ))
            .with_children(|line| {
                // **The two figures meet in the middle**, with a club's name on
                // the outside of each, so the plate reads outward from the
                // score the way a televised one does.
                let [home, away] = sides;
                Self::club(line, &faces, home.3);
                Self::figure(line, &faces, home);
                line.spawn((
                    Node {
                        width: px(Self::DASH.0),
                        height: px(Self::DASH.1),
                        flex_shrink: 0.0,
                        margin: UiRect::axes(px(5), px(0)),
                        border_radius: BorderRadius::all(px(Self::DASH.1 * 0.5)),
                        ..default()
                    },
                    BackgroundColor(Self::DASH_INK),
                ));
                Self::figure(line, &faces, away);
                Self::club(line, &faces, away.3);
            });
    }

    /// A club's name on the plate, or nothing at all when the document did not
    /// carry one — an empty label would still spend the air beside it and push
    /// the figure off the middle.
    fn club(line: &mut RelatedSpawnerCommands<ChildOf>, faces: &Faces, name: &str) {
        if name.is_empty() {
            return;
        }
        // Upper case, which is what a broadcast plate is set in and what the
        // page sets its own full-time mark in. Cased BEFORE the face is chosen:
        // a capital Outfit has no glyph for is a box on screen whatever the
        // lower-case letter was.
        let name = name.to_uppercase();
        line.spawn((
            Text::new(name.clone()),
            TextFont {
                font: FontSource::Handle(faces.face_for(&name)),
                font_size: FontSize::Px(Self::NAME),
                ..default()
            },
            TextColor(Self::INK),
            // The plate is as wide as its names; wrapping one would make it two
            // lines tall instead, over the corner of the picture.
            TextLayout {
                linebreak: LineBreak::NoWrap,
                ..default()
            },
            Node {
                padding: UiRect::axes(px(Self::BREATH), px(0)),
                ..default()
            },
        ));
    }

    /// …and a side's figure, which is also the thing that lights up when he
    /// scores: the chip behind it is the club's shirt, drawn at nothing until
    /// [`Scoreboard::refresh`] has a celebration to spend it on.
    fn figure(
        line: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        side: (bool, Color, Color, &str),
    ) {
        let (is_home, shirt, ink, _) = side;
        line.spawn((
            ScoreDigit {
                is_home,
                drawn: 0,
                shirt,
                ink,
            },
            Text::new("0"),
            TextFont {
                // Digits, in every locale — so the face is chosen once here
                // rather than per frame as the score changes.
                font: FontSource::Handle(faces.face_for("0123456789")),
                font_size: FontSize::Px(Self::DIGITS),
                ..default()
            },
            TextColor(Self::INK),
            TextLayout::justify(Justify::Center),
            Node {
                min_width: px(Self::FIGURES),
                padding: UiRect::axes(px(4), px(2)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(shirt.with_alpha(0.0)),
        ));
    }

    /// The tally at the playhead, and the flash on whoever just scored.
    ///
    /// Behind [`Aftermath::follow_playhead`], which is what resolves the
    /// window the flash is drawn over — reading last frame's would leave the
    /// bug a frame behind a scrub. And behind the two systems that decide
    /// whether there is a card on the screen for the bug to stand down for:
    /// [`Lineup::hold`] at the start and [`FullTime::follow_playhead`] at the
    /// end.
    pub fn refresh(
        config: Res<ViewerConfig>,
        playback: Res<Playback>,
        aftermath: Res<Aftermath>,
        card: Res<FullTime>,
        lineup: Res<Lineup>,
        bug: Single<&mut Visibility, With<ScoreBug>>,
        mut digits: Query<(
            &mut ScoreDigit,
            &mut Text,
            &mut TextColor,
            &mut BackgroundColor,
        )>,
    ) {
        // Whenever a card has the picture, the corner is quiet: the team sheets
        // over the walk-out and the full-time card both name the two clubs
        // larger than the bug does, and a second naming of the same fixture
        // underneath one of them reads as a fault in it. The bug is what says
        // the score when nothing else on the screen is saying it.
        bug.into_inner()
            .set_if_neq(if card.showing() || lineup.presenting() {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });

        let (home, away) = Self::tally(&config, playback.time_ms);

        for (mut digit, mut text, mut ink, mut chip) in &mut digits {
            let wanted = if digit.is_home { home } else { away };
            if digit.drawn != wanted {
                digit.drawn = wanted;
                **text = wanted.to_string();
            }

            // `elation` is the celebration read for this side — non-zero for
            // exactly the side the goal belongs to, own goals already resolved,
            // and it fades out at the end of the window on its own.
            let lit = aftermath.elation(digit.is_home);
            let ground = digit.shirt.with_alpha(lit.clamp(0.0, 1.0));
            if chip.0 != ground {
                chip.0 = ground;
            }
            let printed = Self::mix(Self::INK, digit.ink, lit);
            if ink.0 != printed {
                ink.0 = printed;
            }
        }
    }

    /// The goals either side had scored by `now`.
    ///
    /// A linear scan over the goal list, which is two or three entries and a
    /// dozen at the very worst — the same scan [`Aftermath`] makes of the same
    /// list, and for the same reason a cursor would be more code than the
    /// thing it saved.
    fn tally(config: &ViewerConfig, now: f64) -> (u32, u32) {
        let mut score = (0, 0);
        for goal in &config.goals {
            if goal.time > now {
                continue;
            }
            if config.goal_belongs_to_home(goal) {
                score.0 += 1;
            } else {
                score.1 += 1;
            }
        }
        score
    }

    /// `rest` carried `weight` of the way to `lit`.
    ///
    /// The figure's ink, which has to travel from the plate's near-black to
    /// whatever the club prints its own numbers in and back again — and to
    /// arrive exactly on both, because a flash that ended a shade off would
    /// leave the plate a different colour after every goal.
    fn mix(rest: Color, lit: Color, weight: f32) -> Color {
        if weight <= Aftermath::NOTHING {
            return rest;
        }
        let weight = weight.clamp(0.0, 1.0);
        let (rest, lit) = (rest.to_srgba(), lit.to_srgba());
        Color::from(Srgba {
            red: rest.red + (lit.red - rest.red) * weight,
            green: rest.green + (lit.green - rest.green) * weight,
            blue: rest.blue + (lit.blue - rest.blue) * weight,
            alpha: rest.alpha + (lit.alpha - rest.alpha) * weight,
        })
    }
}

/// The sheet the card sits on: one node over the whole window, and the only
/// thing on it whose visibility is switched.
#[derive(Component)]
pub struct FullTimeVeil;

/// The sheet the panel is centred on — everything the card is made of, at the
/// default depth with the rest of the interface. It is what rises the last few
/// pixels into place as the card comes up, and what is switched on and off.
#[derive(Component)]
pub struct FullTimeCard;

/// **Something the card writes WITH**, and the colour it settles at.
///
/// Bevy has no opacity a subtree can inherit, so a card that fades has to be
/// faded one node at a time — and a node that has been faded no longer knows
/// what it was. This is where that is kept, for the same reason [`ScoreDigit`]
/// keeps the kit it flashes into.
#[derive(Component)]
pub struct FullTimeInk(Color);

/// …and something it writes ON.
///
/// ⚠ **Two components rather than one, and the split is load-bearing.** `Node`
/// REQUIRES `BackgroundColor`, so every node on the card carries one — a line
/// of text every bit as much as a plate. A single marker covering both roles
/// therefore matched the text nodes in the background pass as well, and painted
/// each label's own box in its own ink: a solid rectangle exactly the size of
/// the node, in exactly the colour the letters were meant to be, drawn behind
/// them. Every word on the card came out a blank block, and it looked for all
/// the world like a font that had failed to load. Keeping the two roles in two
/// components is what makes the two passes disjoint by construction rather than
/// by a filter somebody has to remember to write.
#[derive(Component)]
pub struct FullTimeGround(Color);

/// A plate's edge, which fades with the rest of the card.
///
/// A marker rather than a third colour component because a chip already
/// carries one for its shirt, and one entity cannot hold two of a component.
/// It needs no colour of its own: every edge on the card is
/// [`FullTime::PLATE_EDGE`].
#[derive(Component)]
pub struct FullTimeEdge;

/// **The card the replay ends on**: the result, and the men who scored it.
///
/// # What the final whistle used to be
///
/// Nothing at all. [`Playback::advance`] parks the playhead on the duration and
/// clears `playing`; the twenty-two stop wherever the last sample left them and
/// the picture holds on a field of men standing still. On a goals-only
/// recording that arrives seconds after the last clip, so what a viewer is
/// shown is a replay FREEZING rather than a match ending — and the score, which
/// is the one thing anybody wants at that moment, is a bug the size of a
/// postage stamp in the corner.
///
/// So the replay ends the way a broadcast does: the picture goes down behind a
/// scrim and a card comes up over it carrying the result and the goalscorers.
/// The frozen stadium stays visible through it, which is what makes this read
/// as a caption over the match rather than as a page that has replaced it.
///
/// # The same plate the corner wears
///
/// White and see-through, with the names in near-black and each side's figure
/// on a chip of its own kit — which is [`Scoreboard`]'s own arrangement at
/// four times the size. That is the point: a viewer who has watched the corner
/// all match already knows how to read this, and two pieces of furniture that
/// said the same thing two different ways would be two things to learn rather
/// than one. The kit is on the chip and nowhere else, so a club that plays in
/// white gets an edge rather than a hole — see [`Self::PLATE_EDGE`].
///
/// # Nothing on it is worked out at full time
///
/// The card is built once at startup, hidden, off the document the page handed
/// over: the goals are fixed by the time a replay exists, so the final score
/// and every name on it are known before the first frame is drawn. Full time
/// only makes it VISIBLE. That is the rule the rest of this crate keeps about
/// anything that appears mid-replay — see
/// [`CutFade::spawn`](crate::broadcast::cut::CutFade::spawn) — and here it also
/// means the end of the match spends nothing on shaping a dozen lines of text.
///
/// # It is a caption, not a page
///
/// Drawn over the picture and the dip, and UNDER the transport bar, which stays
/// lit and answers the pointer through it: the whole point of a card at full
/// time is that somebody can watch the match again from it. Ordered by
/// [`Backdrop::CAPTION`] rather than by which startup system happened to run
/// first.
#[derive(Resource, Default)]
pub struct FullTime {
    /// How far the card has come up, 0..1.
    weight: f32,
}

impl FullTime {
    /// Seconds the card takes to come up, and to go back down.
    ///
    /// Long enough to read as an ending rather than as a graphic being
    /// switched on, and short enough that the score is legible before anybody
    /// has decided to reach for the rail.
    const RISE_TIME: f32 = 0.55;
    /// How far below its resting place the panel starts, in pixels. The lift
    /// is what separates the card from the scrim it comes up through; without
    /// it the two are one rectangle getting brighter.
    const RISE: f32 = 18.0;
    /// Below this there is nothing on the screen, and the sheet comes off it
    /// rather than being left at zero alpha — a transparent full-window quad is
    /// still a quad, drawn for the rest of the session.
    const NOTHING: f32 = 1e-3;

    /// The most scorers listed under one side before the rest become a count.
    ///
    /// The same five the page above the canvas shows before it folds the rest
    /// behind its own expander, so a hat-trick reads the same in both places
    /// and a 9-0 cannot grow the card off the bottom of a phone.
    const MOST: usize = 5;

    /// The dark the picture goes down behind. `#080c12` again — the ground the
    /// match page paints behind the stage and the colour the cut dips through,
    /// so the replay has one dark rather than three nearby ones.
    ///
    /// Heavy enough to put the picture behind the card and no heavier: a
    /// floodlit ground is most of what is on the screen at the final whistle,
    /// and a scrim that erased it would leave the replay ending on a dialog.
    const SCRIM: Color = Color::srgba(0.031, 0.047, 0.071, 0.62);
    /// …and the panel over it: the corner plate's own white, a little more of
    /// it because this one is the thing being looked at.
    const PANEL: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
    /// The clubs' own names, and the darkest thing on the card.
    const INK: Color = Plate::INK;
    /// The scorers, a step back from the clubs.
    const INK_SOFT: Color = Plate::INK_SOFT;
    /// The heading, a step back again.
    const INK_MUTED: Color = Plate::INK_MUTED;
    const HAIRLINE: Color = Plate::HAIRLINE;
    /// The edge round a chip, and the one thing on the card that is not there
    /// to be seen. A club can play in white, and a white chip on this panel is
    /// a hole in it rather than a score — so every chip is drawn with the same
    /// edge, for the same reason the pins on the seek rail wear theirs: the
    /// shape survives whatever the kit is.
    const PLATE_EDGE: Color = Plate::EDGE;

    /// How wide the panel is allowed to get, and the gutter it keeps from the
    /// edges of a window too narrow for that.
    const PANEL_WIDTH: f32 = 440.0;
    const GUTTER: f32 = 18.0;
    const CORNER: f32 = 14.0;
    /// The colour band across the top of the panel, one half per side. The
    /// crest this crate does not have: it says whose card this is in the two
    /// colours everything else in the viewer says it in.
    const BAND: f32 = 3.0;

    /// One side's chip, square and wide enough for two figures — a match can
    /// finish 10-0 and a card that reflowed at the tenth goal would drag both
    /// club names across the screen.
    const CHIP: f32 = 46.0;
    const DIGITS: f32 = 26.0;
    const CLUB: f32 = 12.0;
    /// Air between a club's name and the chip beside it.
    const BESIDE: f32 = 12.0;
    /// The bar between the two plates. The page sets the same separator as a
    /// five-pixel dot, which is a fine mark between two 30px numerals on white
    /// and a speck between two colour plates on black.
    const DASH: (f32, f32) = (10.0, 2.0);
    /// The dash is the one mark on the card that is neither a name, a figure
    /// nor a rule, and it is the faintest of them.
    const DASH_INK: Color = Color::srgb(0.706, 0.737, 0.769);
    const HEADING: f32 = 10.0;
    const SCORER: f32 = 12.0;

    /// Builds the card, hidden, at startup.
    pub fn spawn(mut commands: Commands, config: Res<ViewerConfig>, faces: Res<Faces>) {
        // The same two fallbacks the bug and the seek rail's markers take.
        let home_shirt = config.home.background_color(Color::srgb(0.0, 0.19, 0.49));
        let away_shirt = config.away.background_color(Color::srgb(0.70, 0.25, 0.0));
        let home_ink = config.home.foreground_color(Color::WHITE);
        let away_ink = config.away.foreground_color(Color::WHITE);

        // Every goal in the match, which is what a full-time score is — asked
        // of the bug's own tally, so the corner and the card cannot be made to
        // disagree by a rule that was changed in one of them.
        let (home_goals, away_goals) = Scoreboard::tally(&config, f64::INFINITY);
        let (home_scorers, away_scorers) = Self::scorers(&config);

        // Upper case, which is the vernacular of the broadcast graphics this is
        // pretending to be and what the page sets its own full-time mark in.
        // Cased BEFORE the face is chosen: a capital Outfit has no glyph for is
        // a box on screen whatever the lower-case letter was.
        let heading = config.labels.full_time.to_uppercase();

        // **The scrim is its own root, and the card is another.** They are two
        // depths, not one: the scrim belongs to the picture and goes UNDER the
        // transport bar, which has to stay lit and usable; the card is
        // furniture and takes the default depth beside the bar and the score
        // plate, which is where every label in this crate that draws correctly
        // lives. They never overlap on screen — the card is centred and the bar
        // is along the bottom — so nothing is covered by the split.
        commands.spawn((
            FullTimeVeil,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Self::SCRIM),
            FullTimeGround(Self::SCRIM),
            GlobalZIndex(Backdrop::CAPTION),
            // A full-screen node swallows the hover and the press of everything
            // under it by default, and the rail is under this one.
            FocusPolicy::Pass,
            Visibility::Hidden,
        ));

        commands
            .spawn((
                FullTimeCard,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    padding: UiRect::all(px(Self::GUTTER)),
                    ..default()
                },
                FocusPolicy::Pass,
                Visibility::Hidden,
            ))
            .with_children(|over| {
                over.spawn((
                    Node {
                        // Full width of whatever it is given and no wider than
                        // the panel: a card that shrank to fit "0 - 0" would be
                        // a different piece of furniture on a goalless draw,
                        // and one with no cap would be a banner across a
                        // desktop.
                        width: percent(100),
                        max_width: px(Self::PANEL_WIDTH),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(Self::CORNER)),
                        ..default()
                    },
                    BackgroundColor(Self::PANEL),
                    FullTimeGround(Self::PANEL),
                ))
                .with_children(|card| {
                    card.spawn(Node {
                        width: percent(100),
                        height: px(Self::BAND),
                        flex_direction: FlexDirection::Row,
                        flex_shrink: 0.0,
                        // Held off the ends by the corner it would otherwise
                        // have to be clipped to. Three pixels tall cannot
                        // express a fourteen-pixel radius — a rounded corner is
                        // clamped to half the height — so a full-bleed band
                        // either needs the card to clip it or has to stop where
                        // the curve starts. It stops.
                        padding: UiRect::axes(px(Self::CORNER), px(0)),
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
                                FullTimeGround(shirt),
                            ));
                        }
                    });

                    card.spawn(Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(px(24), px(22)),
                        ..default()
                    })
                    .with_children(|body| {
                        body.spawn((
                            Text::new(heading.clone()),
                            TextFont {
                                font: FontSource::Handle(faces.face_for(&heading)),
                                font_size: FontSize::Px(Self::HEADING),
                                ..default()
                            },
                            TextColor(Self::INK_MUTED),
                            FullTimeInk(Self::INK_MUTED),
                            Node {
                                margin: UiRect::bottom(px(16)),
                                ..default()
                            },
                        ));

                        body.spawn(Node {
                            width: percent(100),
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        })
                        .with_children(|score| {
                            // **Outward from the middle**: the two figures meet
                            // at the dash and a club's name reads away from it
                            // on either side, which is how a televised result
                            // is set and how the corner plate sets the same
                            // thing all match.
                            Self::club(score, &faces, &config.home_name, true);
                            Self::chip(score, &faces, (home_shirt, home_ink), home_goals);
                            score.spawn((
                                Node {
                                    width: px(Self::DASH.0),
                                    height: px(Self::DASH.1),
                                    flex_shrink: 0.0,
                                    margin: UiRect::axes(px(9), px(0)),
                                    border_radius: BorderRadius::all(px(Self::DASH.1 * 0.5)),
                                    ..default()
                                },
                                BackgroundColor(Self::DASH_INK),
                                FullTimeGround(Self::DASH_INK),
                            ));
                            Self::chip(score, &faces, (away_shirt, away_ink), away_goals);
                            Self::club(score, &faces, &config.away_name, false);
                        });

                        // A rule with nothing under it is a line across an empty
                        // card, so a goalless match simply ends on the score.
                        if home_scorers.is_empty() && away_scorers.is_empty() {
                            return;
                        }
                        body.spawn((
                            Node {
                                width: percent(100),
                                height: px(1),
                                margin: UiRect::axes(px(0), px(18)),
                                ..default()
                            },
                            BackgroundColor(Self::HAIRLINE),
                            FullTimeGround(Self::HAIRLINE),
                        ));
                        body.spawn(Node {
                            width: percent(100),
                            flex_direction: FlexDirection::Row,
                            column_gap: px(18),
                            ..default()
                        })
                        .with_children(|sides| {
                            Self::column(sides, &faces, &home_scorers, true);
                            Self::column(sides, &faces, &away_scorers, false);
                        });
                    });
                });
            });
    }

    /// A club's name, reading away from the score.
    ///
    /// Takes an equal share of whatever the chips leave, so a long club wraps
    /// inside its own half rather than pushing the score off the middle of the
    /// card. Nothing at all when the document carried no name: the chips then
    /// sit in the centre by themselves, which is what the card said before
    /// there were names to set.
    fn club(score: &mut RelatedSpawnerCommands<ChildOf>, faces: &Faces, name: &str, home: bool) {
        if name.is_empty() {
            return;
        }
        // Upper case, as the corner plate sets the same two names.
        let name = name.to_uppercase();
        let (reading, air) = if home {
            (Justify::Right, UiRect::right(px(Self::BESIDE)))
        } else {
            (Justify::Left, UiRect::left(px(Self::BESIDE)))
        };
        score.spawn((
            Text::new(name.clone()),
            TextFont {
                font: FontSource::Handle(faces.face_for(&name)),
                font_size: FontSize::Px(Self::CLUB),
                ..default()
            },
            TextColor(Self::INK),
            FullTimeInk(Self::INK),
            TextLayout::justify(reading),
            Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_width: px(0),
                padding: air,
                ..default()
            },
        ));
    }

    /// …and a side's figure, on a chip of its own kit.
    fn chip(
        score: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        kit: (Color, Color),
        goals: u32,
    ) {
        let (shirt, ink) = kit;
        score
            .spawn((
                Node {
                    width: px(Self::CHIP),
                    height: px(Self::CHIP),
                    flex_shrink: 0.0,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                BackgroundColor(shirt),
                FullTimeGround(shirt),
                BorderColor::all(Self::PLATE_EDGE),
                FullTimeEdge,
            ))
            .with_child((
                Text::new(goals.to_string()),
                TextFont {
                    font: FontSource::Handle(faces.face_for("0123456789")),
                    font_size: FontSize::Px(Self::DIGITS),
                    ..default()
                },
                TextColor(ink),
                FullTimeInk(ink),
            ));
    }

    /// One side's scorers, reading outward from the score the way the page
    /// above the canvas sets the same two lists: the home side's hard against
    /// the middle, the away side's away from it.
    fn column(
        sides: &mut RelatedSpawnerCommands<ChildOf>,
        faces: &Faces,
        scorers: &[String],
        home: bool,
    ) {
        let (edge, justify) = if home {
            (AlignItems::FlexEnd, Justify::Right)
        } else {
            (AlignItems::FlexStart, Justify::Left)
        };
        sides
            .spawn(Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                // A column that may not shrink pushes its neighbour off the
                // card when one side has scored four and the other none.
                min_width: px(0),
                flex_direction: FlexDirection::Column,
                align_items: edge,
                row_gap: px(5),
                ..default()
            })
            .with_children(|list| {
                for line in scorers.iter().take(Self::MOST) {
                    list.spawn((
                        Text::new(line.clone()),
                        TextFont {
                            font: FontSource::Handle(faces.face_for(line)),
                            font_size: FontSize::Px(Self::SCORER),
                            ..default()
                        },
                        TextColor(Self::INK_SOFT),
                        FullTimeInk(Self::INK_SOFT),
                        TextLayout::justify(justify),
                    ));
                }
                let Some(rest) = scorers
                    .len()
                    .checked_sub(Self::MOST)
                    .filter(|rest| *rest > 0)
                else {
                    return;
                };
                list.spawn((
                    Text::new(format!("+{rest}")),
                    TextFont {
                        font_size: FontSize::Px(Self::SCORER),
                        ..default()
                    },
                    TextColor(Self::INK_MUTED),
                    FullTimeInk(Self::INK_MUTED),
                ));
            });
    }

    /// **Who scored, as two lists of lines ready to be set.**
    ///
    /// Split out of the spawn because it is the half that is about football
    /// rather than about nodes, and it is the half worth asking without a
    /// screen: an own goal is listed under the side it was scored FOR, which is
    /// the one rule here that a list of scorers cannot be read off directly.
    /// [`ViewerConfig::goal_belongs_to_home`] owns it, the same way the bug and
    /// the celebration ask it.
    fn scorers(config: &ViewerConfig) -> (Vec<String>, Vec<String>) {
        let mut home = Vec::new();
        let mut away = Vec::new();
        for goal in &config.goals {
            let Some(player) = config
                .players
                .iter()
                .find(|player| player.id == goal.player_id)
            else {
                continue;
            };
            let minute = Self::minute(goal.time, config.match_time_ms);
            let own = if goal.is_auto_goal { " (OG)" } else { "" };
            let line = format!("{} {minute}'{own}", player.last_name);
            if config.goal_belongs_to_home(goal) {
                home.push(line);
            } else {
                away.push(line);
            }
        }
        (home, away)
    }

    /// Which minute of ninety a goal landed in.
    ///
    /// ⚠ **The page's arithmetic, deliberately** — the same truncating
    /// `time × 90 / duration` its own scoreboard prints above the canvas. The
    /// two are on one screen together, and a goal that read 45' up there and
    /// 46' down here would be two goals to anybody comparing them. A document
    /// claiming no duration has no minutes in it either.
    fn minute(time: f64, match_time_ms: f64) -> u32 {
        if match_time_ms <= 0.0 {
            return 0;
        }
        (time * 90.0 / match_time_ms) as u32
    }

    /// Runs the card up when the replay reaches the end, and back down when
    /// anybody asks for the football again.
    ///
    /// Behind [`Playback::advance`], which is what parks the playhead on the
    /// duration, and in front of [`Scoreboard::refresh`], which stands the
    /// corner bug down for whatever this leaves on the screen.
    pub fn follow_playhead(
        time: Res<Time>,
        playback: Res<Playback>,
        mut card: ResMut<FullTime>,
        veil: Single<&mut Visibility, With<FullTimeVeil>>,
        screen: Single<
            (&mut Visibility, &mut UiTransform),
            (With<FullTimeCard>, Without<FullTimeVeil>),
        >,
        mut grounds: Query<(&FullTimeGround, &mut BackgroundColor)>,
        mut inks: Query<(&FullTimeInk, &mut TextColor)>,
        mut edges: Query<&mut BorderColor, With<FullTimeEdge>>,
    ) {
        let wanted = if playback.at_full_time() { 1.0 } else { 0.0 };
        // Cut rather than walked across a scrub: the card the playhead landed
        // in is the one to be in, which is the rule every written shot in
        // `broadcast` keeps about a seek. It is also what takes the card off
        // the screen on the frame the space bar restarts the match.
        let weight = if playback.seeked {
            wanted
        } else {
            Grip::toward(card.weight, wanted, Self::RISE_TIME, &time)
        };
        // Most of a match is spent neither showing this nor hiding it, and a
        // frame that changes nothing must not repaint two dozen nodes.
        if card.weight == weight {
            return;
        }
        card.weight = weight;

        let drawn = if card.showing() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        veil.into_inner().set_if_neq(drawn);
        let (mut seen, mut lift) = screen.into_inner();
        seen.set_if_neq(drawn);

        let shown = Self::ease(weight);
        // A transform rather than the panel's own `top`: an inset is a layout
        // property, and writing one every frame of the ramp would have taffy
        // re-solve the whole card — two plates, a rule and a dozen lines of
        // text — for each pixel of a lift that moves nothing inside it.
        lift.translation = Val2::px(0.0, Self::RISE * (1.0 - shown));
        for (rest, mut ground) in &mut grounds {
            ground.0 = Self::sheer(rest.0, shown);
        }
        for (rest, mut ink) in &mut inks {
            ink.0 = Self::sheer(rest.0, shown);
        }
        let edge = Self::sheer(Self::PLATE_EDGE, shown);
        for mut border in &mut edges {
            border.set_all(edge);
        }
    }

    /// A resting colour at `weight` of its own strength.
    ///
    /// On the card rather than on either marker: the ink, the ground and the
    /// chips' shared edge all fade by the same rule, and the edge has no node
    /// of its own to be kept on.
    fn sheer(rest: Color, weight: f32) -> Color {
        let rest = rest.to_srgba();
        Color::from(rest.with_alpha(rest.alpha * weight))
    }

    /// Whether there is a card on the screen at all.
    pub fn showing(&self) -> bool {
        self.weight > Self::NOTHING
    }

    /// The ramp, with no corner at either end: a linear one lets go the instant
    /// it starts and then stops dead, and the eye reads both as the card
    /// stepping rather than coming up.
    fn ease(weight: f32) -> f32 {
        let weight = weight.clamp(0.0, 1.0);
        weight * weight * (3.0 - 2.0 * weight)
    }
}

/// What the bug and the card say is a fact about the match rather than about the
/// interface, so both can be asked without a screen to draw on.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::config::{GoalInfo, PlayerInfo};

    fn player(id: u32, is_home: bool) -> PlayerInfo {
        PlayerInfo {
            id,
            shirt_number: 9,
            first_name: "Jay-Jay".to_string(),
            last_name: "Okocha".to_string(),
            position: "ST".to_string(),
            is_home,
            starting: true,
            skin: 0,
            hair: 0,
            eyes: 0,
            photo: None,
            face: None,
        }
    }

    fn goal(player_id: u32, time: f64, is_auto_goal: bool) -> GoalInfo {
        GoalInfo {
            player_id,
            time,
            is_auto_goal,
        }
    }

    /// One forward a side, and the goals they scored between them.
    fn fixture(goals: Vec<GoalInfo>) -> ViewerConfig {
        let mut config = ViewerConfig::of_players(vec![player(1, true), player(2, false)]);
        config.goals = goals;
        config
    }

    /// The bug counts what the playhead has reached and nothing in front of
    /// it: a replay stopped in the fortieth minute does not know about the
    /// eighty-ninth, which is the whole difference between this and the
    /// full-time score printed above the canvas.
    #[test]
    fn the_score_is_the_one_at_the_playhead() {
        let config = fixture(vec![
            goal(1, 10_000.0, false),
            goal(2, 20_000.0, false),
            goal(1, 80_000.0, false),
        ]);
        assert_eq!(Scoreboard::tally(&config, 0.0), (0, 0));
        assert_eq!(Scoreboard::tally(&config, 10_000.0), (1, 0));
        assert_eq!(Scoreboard::tally(&config, 19_999.0), (1, 0));
        assert_eq!(Scoreboard::tally(&config, 20_000.0), (1, 1));
        assert_eq!(Scoreboard::tally(&config, 900_000.0), (2, 1));
    }

    /// An own goal counts for the other side, which is the one rule about a
    /// score that a list of scorers cannot be read off directly. Owned by
    /// `ViewerConfig::goal_belongs_to_home` and asked here so the bug and the
    /// celebration can never disagree about who is winning.
    #[test]
    fn an_own_goal_counts_for_the_side_it_was_scored_against() {
        let config = fixture(vec![goal(1, 10_000.0, true)]);
        assert_eq!(Scoreboard::tally(&config, 10_000.0), (0, 1));
    }

    /// The flash starts on the plate's own ink and ends on the club's, and
    /// comes all the way back — a figure that finished celebrating has to be
    /// the same colour as one that never started, or the plate is a different
    /// colour after every goal.
    #[test]
    fn the_flash_returns_the_figure_to_the_plates_own_ink() {
        let club = Color::srgb(0.0, 0.19, 0.49);
        assert_eq!(Scoreboard::mix(Scoreboard::INK, club, 0.0), Scoreboard::INK);
        assert_eq!(
            Scoreboard::mix(Scoreboard::INK, club, 1.0).to_srgba(),
            club.to_srgba(),
            "a goal did not light the figure"
        );
        let midway = Scoreboard::mix(Scoreboard::INK, club, 0.5).to_srgba();
        assert_ne!(midway, Scoreboard::INK.to_srgba());
        assert_ne!(
            midway,
            club.to_srgba(),
            "the flash is a switch rather than a fade"
        );
    }

    /// The same squad, with names worth telling apart.
    fn named(id: u32, is_home: bool, last_name: &str) -> PlayerInfo {
        PlayerInfo {
            last_name: last_name.to_string(),
            ..player(id, is_home)
        }
    }

    /// A fixture with a length to it, which [`Scoreboard::tally`] never needs
    /// and every minute on the card is measured against.
    fn played(players: Vec<PlayerInfo>, goals: Vec<GoalInfo>) -> ViewerConfig {
        let mut config = ViewerConfig::of_players(players);
        config.match_time_ms = 5_400_000.0;
        config.goals = goals;
        config
    }

    /// A scorer is his name and the minute, in the order the page above the
    /// canvas prints the same two.
    #[test]
    fn a_scorer_is_his_name_and_the_minute() {
        let config = played(
            vec![named(1, true, "Okocha"), named(2, false, "Vieri")],
            vec![goal(1, 1_350_000.0, false), goal(2, 4_050_000.0, false)],
        );
        let (home, away) = FullTime::scorers(&config);
        assert_eq!(home, vec!["Okocha 22'".to_string()]);
        assert_eq!(away, vec!["Vieri 67'".to_string()]);
    }

    /// An own goal is listed under the side it was scored FOR — the one rule
    /// on the card that a list of scorers cannot be read off directly, and the
    /// same one the bug's tally keeps.
    #[test]
    fn an_own_goal_is_listed_under_the_side_it_was_scored_for() {
        let config = played(
            vec![named(1, true, "Okocha"), named(2, false, "Vieri")],
            vec![goal(2, 2_700_000.0, true)],
        );
        let (home, away) = FullTime::scorers(&config);
        assert_eq!(home, vec!["Vieri 45' (OG)".to_string()]);
        assert!(away.is_empty(), "the man's own side was credited: {away:?}");
    }

    /// **Why the bug and the card are one module.** The digits on the card come
    /// off the bug's tally and the names off the goal list, and the two walking
    /// apart would be a card reading 2 over one scorer.
    #[test]
    fn the_card_lists_as_many_scorers_as_it_shows_goals() {
        let config = played(
            vec![
                named(1, true, "Okocha"),
                named(2, true, "Shevchenko"),
                named(3, false, "Vieri"),
            ],
            vec![
                goal(1, 600_000.0, false),
                goal(3, 1_200_000.0, false),
                goal(2, 4_800_000.0, false),
                goal(2, 5_000_000.0, true),
            ],
        );
        let (home_goals, away_goals) = Scoreboard::tally(&config, f64::INFINITY);
        let (home, away) = FullTime::scorers(&config);
        assert_eq!((home_goals, away_goals), (2, 2));
        assert_eq!(
            (home_goals as usize, away_goals as usize),
            (home.len(), away.len()),
            "the card would have shown {home_goals}-{away_goals} over {home:?} and {away:?}"
        );
    }

    /// The minute is the PAGE's, down to the truncation. The two scoreboards
    /// are on one screen together and a goal that read 45' above the canvas and
    /// 46' below it would be two goals to anybody comparing them.
    #[test]
    fn the_minute_is_the_one_the_page_prints() {
        let full = 5_400_000.0;
        assert_eq!(FullTime::minute(0.0, full), 0);
        assert_eq!(FullTime::minute(full * 0.5, full), 45);
        assert_eq!(FullTime::minute(full, full), 90);
        // Truncated rather than rounded, which is what the page's own integer
        // division does: the 44th minute is not yet the 45th.
        assert_eq!(FullTime::minute(full * 0.499, full), 44);
        // And a document claiming no duration has no minutes in it either.
        assert_eq!(FullTime::minute(1_000.0, 0.0), 0);
    }

    /// The card opens closed and settles open, and never goes back down on its
    /// way — with no corner at either end, which is the difference between a
    /// panel coming up and one being switched on.
    #[test]
    fn the_card_comes_up_without_a_step_at_either_end() {
        assert_eq!(FullTime::ease(0.0), 0.0);
        assert_eq!(FullTime::ease(1.0), 1.0);

        let mut previous = 0.0;
        for step in 1..=20 {
            let shown = FullTime::ease(step as f32 / 20.0);
            assert!(
                shown >= previous,
                "the card went back down: {previous} then {shown}"
            );
            previous = shown;
        }

        assert!(FullTime::ease(0.05) < 0.05, "the card stepped off nothing");
        assert!(FullTime::ease(0.95) > 0.95, "the card stopped dead");
    }

    /// Nothing on the card is drawn before it comes up, and everything is at
    /// its own colour once it has — including the scrim, which is translucent
    /// at rest and would black the pitch out if a fade ended anywhere else.
    #[test]
    fn the_fade_ends_on_the_colour_it_started_from() {
        // In sRGB on both sides: a fade is a write to the alpha channel, and
        // `Color::WHITE` is a linear one, so a round trip through the space the
        // fade works in is the only comparison that means anything.
        for rest in [FullTime::SCRIM, FullTime::PANEL, Color::WHITE] {
            assert_eq!(FullTime::sheer(rest, 1.0).to_srgba(), rest.to_srgba());
            assert_eq!(FullTime::sheer(rest, 0.0).to_srgba().alpha, 0.0);
            assert!(FullTime::sheer(rest, 0.5).to_srgba().alpha < rest.to_srgba().alpha);
        }
    }
}
