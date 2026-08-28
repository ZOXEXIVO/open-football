//! How much of a ground there is, and who is sitting in it.
//!
//! Two questions that look unrelated and are the same one. A stadium is built
//! for the people who come to it: the number of steps of concrete round a
//! pitch and the number of coats on them are both read off the fixture, and a
//! scene that took one from the match and the other from a constant would put
//! a full house in a village ground or five empty rows round a cup final.
//!
//! So [`Stature`] answers both, off the venue the page hands over, and
//! [`Crowd`] builds the figures. [`Terrace`] is the flight of steps the two
//! share: [`pitch`](crate::scene::pitch) pours its concrete off exactly the
//! description the crowd is seated on, so a spectator cannot end up standing
//! in mid-air or buried in the row in front of him.

use crate::app::config::VenueInfo;
use crate::art::textures::{CrowdPalette, Textures};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

/// What kind of ground this is, as one number: 0 at a village pitch or a
/// training ground, 1 at a great stadium.
///
/// A curve rather than a set of tiers. Grounds do not come in three sizes, and
/// a step function here would show up directly as the same three stadiums over
/// and over — which is the one thing a procedural ground cannot afford, since
/// it is the same shape in every match a player ever watches.
#[derive(Clone, Copy)]
pub struct Stature {
    /// How much stand there is, 0..1.
    standing: f32,
    /// How much of it has somebody in it, 0..1.
    occupancy: f32,
    /// How big a following the visitors brought, 0..1. Read off their standing
    /// rather than their gate, because what crosses the wire about the away
    /// side is a reputation — and it is the right measure anyway: what fills
    /// an away end is how many people follow that club, not how many its own
    /// ground holds.
    following: f32,
}

impl Stature {
    /// The fewest steps this scene ever builds.
    ///
    /// Five, and it is a real terrace rather than a token one: five steps at
    /// [`Terrace`]'s riser is a bank about three metres high, which is what a
    /// non-league ground or a club's training pitch actually has round it.
    pub const FEWEST_ROWS: usize = 5;

    /// The gates the curve runs between. At or below the first a ground is as
    /// small as this scene builds; at or above the second it is as big.
    ///
    /// **A gate and not a capacity**, which is the whole of what decides how
    /// much stadium gets built. The database records what a club actually
    /// draws; its capacity is that figure grossed up through
    /// [`Self::TYPICAL_GATE`] where there is one and GUESSED from reputation
    /// where there is not — so reading capacity would build half the world's
    /// grounds off an estimate when the real number was sitting right there.
    /// A ground is built for the people who come to it, and this is them.
    ///
    /// **Both ends are set off the real distribution, not picked.** Measured
    /// across the 1,335 clubs in `database.db` that carry a gate:
    ///
    /// | p10 | p25 | p50 | p75 | p90 | p97 | max |
    /// |---|---|---|---|---|---|---|
    /// | 500 | 1,500 | 4,000 | 14,000 | 25,700 | 42,600 | 81,400 |
    ///
    /// The top of the curve was 50,000 to begin with, which is the 98th
    /// percentile: only Madrid, United, Bayern and a dozen others ever reached
    /// a full-height stand, so effectively every real fixture was played in a
    /// half-built ground. Lokomotiv Moscow — a 13,133 gate, seventh-tenth of
    /// the world's clubs, a Moscow derby — came out at thirteen rows of
    /// twenty-one. At 28,000 the top decile of grounds is a great one, which
    /// is roughly what the top decile of grounds is, and Lokomotiv gets
    /// twenty-four rows of thirty-four.
    const HUMBLE_GATE: f32 = 1_200.0;
    const GRAND_GATE: f32 = 28_000.0;

    /// World reputation below which a side plays in front of the smallest bank
    /// there is, whatever its parent club's ground holds.
    ///
    /// Four thousand is the top of `Regional` on the simulator's scale — a
    /// lower-league or semi-professional club. The gate curve alone does not
    /// catch them: a club nobody ever counted a crowd for falls back to the
    /// capacity the simulator ESTIMATED from its reputation, and that estimate
    /// is generous enough to hand a fourth-tier side a mid-size bowl. So the
    /// reputation is read as well, and read as a CAP rather than as a second
    /// curve — a small club's ground is small however the arithmetic
    /// elsewhere came out.
    const HUMBLE_REPUTATION: u16 = 4_000;

    /// What a ground is taken to be drawing when nobody ever counted.
    ///
    /// The simulator recovers a capacity by grossing a recorded gate up
    /// through exactly this figure, so it is also the ratio most grounds come
    /// back at: an ordinary league fixture, comfortably full, with visible
    /// gaps in it. See `ClubFacilities::TYPICAL_UTILISATION` in the core
    /// crate, which is the same number seen from the other end.
    const TYPICAL_GATE: f32 = 0.82;

    /// Nobody empties a ground completely and nobody fills one to the last
    /// seat, so the crowd is held inside a band whatever the numbers say.
    const SPARSEST: f32 = 0.30;
    const FULLEST: f32 = 0.96;

    /// How full a training ground's terrace is on an academy matchday: some
    /// parents, a couple of scouts and the boys who are not playing.
    ///
    /// Deliberately below [`Self::SPARSEST`], which is the floor for a fixture
    /// somebody bought a ticket to. The gate on record belongs to the PARENT
    /// club and describes a different ground on a different day — left to
    /// speak for this one it packs a five-step terrace to the same nine-tenths
    /// Old Trafford fills to, which is the one thing an under-18s game never
    /// looks like.
    const ACADEMY_GATE: f32 = 0.10;

    /// How far a visiting side moves the gate, at most, and the reputation gap
    /// that gets it there.
    ///
    /// Nobody buys a ticket to watch the home team in the abstract. Half of
    /// what fills a ground is who is coming to it, and the swing is not small:
    /// a fixture against the club everybody wants to see sells out a stand
    /// that is a third empty for a midweek game against the bottom side.
    ///
    /// Read as a DIFFERENCE rather than off the visitor's standing alone,
    /// because that is how it works: a mid-table club visiting a village side
    /// is an occasion, and the same club visiting a great one is a quiet
    /// afternoon. Four thousand points is the gap that gets the full swing,
    /// which on the simulator's scale is about two tiers.
    ///
    /// **Asymmetric, and by more than double.** Going up and going down are
    /// not the same thing to a supporter. The people who come to everything
    /// come to everything, so a poor fixture only loses you the ones who pick
    /// and choose — a fifth, and a ground that is three-quarters full for the
    /// bottom club is still three-quarters full. But a tie against the side
    /// everybody wants to see brings out people who go to nothing else, and at
    /// a small club that is most of a second crowd: half as many again, which
    /// is what fills an 8,000 ground that usually draws 5,000.
    ///
    /// The upper end needs no ceiling of its own — [`Self::FULLEST`] is the
    /// ceiling, and it is the right one. A ground cannot hold more than it
    /// holds, and the swing running into that cap IS the sell-out.
    const PULL_UP: f32 = 0.45;
    const PULL_DOWN: f32 = 0.22;
    const PULL_SPAN: f32 = 4_000.0;

    /// Reads the fixture.
    ///
    /// **The two answers come off the same gate but not off the same day**,
    /// and keeping them apart is the whole of this:
    ///
    /// - **The stand is concrete.** It was built for the crowd the club draws
    ///   season in and season out, and it is still exactly that size on the
    ///   night nobody turns up. So its height comes off the club's ORDINARY
    ///   gate and nothing about today can move it.
    /// - **The crowd is not.** How much of that stand has somebody in it is a
    ///   fact about this fixture, and the biggest thing in it after the home
    ///   club itself is who they are playing — see [`Self::PULL`].
    ///
    /// Run together, as they were at first, a club would grow and shrink its
    /// own stadium depending on the opposition.
    pub fn of(venue: &VenueInfo) -> Self {
        // Zero is "nobody counted", not "nobody came". A document written
        // before the venue crossed the wire has to keep building the stadium
        // the viewer always built, and a club with no gate on record still has
        // a ground: the capacity the simulator estimated for it is the only
        // thing left to read, at the utilisation it was estimated through.
        let capacity = if venue.capacity == 0 {
            Self::GRAND_GATE / Self::TYPICAL_GATE
        } else {
            venue.capacity as f32
        };
        let gate = if venue.attendance == 0 {
            capacity * Self::TYPICAL_GATE
        } else {
            venue.attendance as f32
        };

        // Who turned up for THIS one. The ordinary gate is what the club draws
        // across a season, so a fixture the whole city wants to see is already
        // averaged in with the ones nobody does — this puts the swing back.
        let gap = ((venue.visitor as f32 - venue.reputation as f32) / Self::PULL_SPAN)
            .clamp(-1.0, 1.0);
        let pull = if gap > 0.0 {
            Self::PULL_UP
        } else {
            Self::PULL_DOWN
        };
        let today = gate * (1.0 + pull * gap);

        let humble = venue.youth || venue.reputation < Self::HUMBLE_REPUTATION;
        let standing = if humble {
            0.0
        } else {
            // A square root rather than a straight line, because a crowd is an
            // AREA and this drives a HEIGHT. A ground holds its people across
            // the front of four banks as well as up them, so twice the gate is
            // nothing like twice the steps — read linearly, a twenty-thousand
            // crowd came out barely deeper than a village one and every
            // stadium in the game was the same low bowl.
            ((gate - Self::HUMBLE_GATE) / (Self::GRAND_GATE - Self::HUMBLE_GATE))
                .clamp(0.0, 1.0)
                .sqrt()
        };

        Stature {
            standing,
            // The reputations a travelling support runs between: below the
            // first a club is followed by its own town, above the second by a
            // country.
            following: ((venue.visitor as f32 - 3_000.0) / 6_000.0).clamp(0.0, 1.0),
            occupancy: if venue.youth {
                Self::ACADEMY_GATE
            } else {
                (today / capacity.max(1.0)).clamp(Self::SPARSEST, Self::FULLEST)
            },
        }
    }

    /// How many steps a bank that would have `most` of them at a great ground
    /// gets here.
    pub fn rows(&self, most: usize) -> usize {
        let most = most.max(Self::FEWEST_ROWS);
        let extra = (most - Self::FEWEST_ROWS) as f32 * self.standing;
        Self::FEWEST_ROWS + extra.round() as usize
    }

    /// How far a bank runs past the corner of the playing surface, between the
    /// `least` a small ground wraps and the `most` a great one does.
    ///
    /// Height is not the only thing that says how big a ground is. Left at the
    /// full wrap, a five-step bank reads as a running track: a hundred and
    /// forty metres of terracing three metres high is not a small stadium, it
    /// is a large flat one.
    pub fn overhang(&self, least: f32, most: f32) -> f32 {
        least + (most - least) * self.standing
    }

    /// The share of the places with somebody in them.
    pub fn occupancy(&self) -> f32 {
        self.occupancy
    }

    /// **What share of a bank turns up in the club's colours.**
    ///
    /// Two things decide it, and they are the two the eye actually reads.
    ///
    /// **Which bank it is.** An end behind a goal is where the support stands
    /// and it is mostly shirts; a touchline is the main stand and it is mostly
    /// coats, with the colours scattered thinly through it. The gap between
    /// the pair is not decoration — it is the only thing that makes an end
    /// read as an end from across the ground.
    ///
    /// **How big the club is.** A great club fills its ends with people who
    /// own the shirt and wear it in November; at a village ground half the
    /// crowd is somebody's father in the coat he came in. So both bands run
    /// with [`Self::standing`] — and the SIDES run too, which is the quieter
    /// half of the same fact: a big club's main stand still carries a good
    /// scattering of colour where a small club's carries almost none.
    ///
    /// The end never reaches a full house of colour and should not. A quarter
    /// of any real kop is in a coat, and an end painted at nine tenths comes
    /// out as a printed sheet rather than as people.
    /// An away end runs off the VISITORS' standing rather than the home
    /// club's, because it is their support filling it: a great club travels in
    /// numbers to a small ground, and a small one sends two coaches to a great
    /// one.
    pub fn allegiance(&self, stand: Stand) -> f32 {
        let (least, most, size) = match stand {
            Stand::HomeEnd => (0.40, 0.76, self.standing),
            Stand::AwayEnd => (0.40, 0.76, self.following),
            Stand::Side => (0.05, 0.17, self.standing),
        };
        least + (most - least) * size
    }

    /// **How much of an end's support is gathered at a given point across it**,
    /// `0` at the middle of the bank and `1` at either corner.
    ///
    /// Support massed behind the goal is the single most recognisable thing
    /// about a football ground, and an end painted at one strength all the way
    /// across does not have it: an end bank here is a hundred metres wide
    /// against a pitch of sixty-eight, so a third of it is not behind the goal
    /// at all — it is the corners, which run past the flags and into the
    /// touchline stands. Spread evenly, the colour bled round the whole bowl
    /// and there was no kop anywhere in it.
    ///
    /// So it is full across the middle — roughly the width of the six-yard box
    /// out to the posts, which is where the people who sing actually stand —
    /// and falls away through the corners to about a quarter, which is the
    /// level of a main stand. The two meet as a smoothstep because the real
    /// thing has no edge to it: a kop does not stop, it thins.
    pub fn behind_goal(across: f32) -> f32 {
        /// Where the massed support gives way, and where it has finished
        /// giving way, as a share of the half-width of the bank.
        const CORE: f32 = 0.34;
        const CORNER: f32 = 0.86;
        /// What is left of it out in the corners.
        const FLANK: f32 = 0.25;

        let out = ((across.abs() - CORE) / (CORNER - CORE)).clamp(0.0, 1.0);
        1.0 - (1.0 - FLANK) * (out * out * (3.0 - 2.0 * out))
    }

    /// **What share of a bank is on its feet** rather than in its seat.
    ///
    /// The same two facts as [`Self::allegiance`], because it is the same
    /// fact: an end behind a goal is where the support stands, and it stands —
    /// at a big club half of it is up for the whole ninety minutes — while a
    /// touchline is season tickets and directors and stays seated but for the
    /// goals. So the band runs with the bank, and with the size of the club
    /// inside it.
    ///
    /// It matters far more than it sounds. A stand of identical seated figures
    /// is a stand of identical seated figures however well each one is
    /// modelled; a man on his feet is a metre taller than his neighbour, and a
    /// scattering of them is the only thing that fills the daylight between
    /// one row and the next — which is what made the old bank read as confetti
    /// on a wall. It costs nothing at all: standing is a different set of ring
    /// heights, not another vertex.
    pub fn on_their_feet(&self, stand: Stand) -> f32 {
        let (least, most) = match stand {
            Stand::HomeEnd | Stand::AwayEnd => (0.26, 0.58),
            Stand::Side => (0.05, 0.15),
        };
        least + (most - least) * self.standing
    }

    /// Whether a bank of `rows` is deep enough for the lit walkway that splits
    /// the tiers to mean anything.
    ///
    /// On a five-step terrace it would land on the second step, which is not a
    /// tier break — it is a stripe painted across a low wall, and it reads as
    /// one. A small ground has no two tiers to split.
    pub fn tiered(rows: usize) -> bool {
        rows >= Self::FEWEST_ROWS * 2
    }
}

/// Which of the four banks this is, and so who is sitting in it.
///
/// A ground is not one crowd. The ends behind the goals are where the support
/// stands — the ones who come in the shirt, bring the flags and sing — and the
/// two long sides are the main stand and the one opposite it, which is season
/// tickets, families and directors, in whatever coat they own. Painting all
/// four the same way is what made every bank a wall of club colour, and a
/// stadium where the ends are indistinguishable from the sides has no ends.
/// …and the two ends are not the same as each other either. **One of them
/// holds the people who travelled.**
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    /// Along a touchline. Mostly neutral: coats, not colours.
    Side,
    /// Behind one goal: the home support.
    HomeEnd,
    /// Behind the other: the visitors'.
    AwayEnd,
}

/// A flight of terracing, in the bank's own space — rows receding along `+Z`
/// and climbing in `+Y` from the front one.
///
/// Built by [`pitch`](crate::scene::pitch), which pours the concrete off it,
/// and handed straight to [`Crowd::fill`], which seats people on the same
/// steps. One description and two readers: the arithmetic that puts a step's
/// surface at a given height is written once, so a spectator cannot float
/// above his row or sink into it.
pub struct Terrace {
    /// Length across the front.
    pub length: f32,
    pub rows: usize,
    /// Height gained and depth given up per step.
    pub riser: f32,
    pub tread: f32,
    /// Centre spot to the front edge of the first step.
    pub from: f32,
    /// How thick a step's slab is, as a multiple of the riser. Over one, so
    /// each step overlaps the one below and the flight is solid rather than a
    /// stack of floating shelves.
    pub slab: f32,
}

impl Terrace {
    /// The walking surface of a step: `x` is where its front edge falls and
    /// `y` how high it stands.
    pub fn step(&self, row: usize) -> Vec2 {
        Vec2::new(
            self.from + self.tread * row as f32,
            self.riser * (row as f32 + 0.5 + self.slab * 0.5),
        )
    }

    /// Where the slab of a step is centred — half a tread behind its front
    /// edge, half a slab below its surface.
    pub fn slab_centre(&self, row: usize) -> Vec3 {
        let step = self.step(row);
        Vec3::new(
            0.0,
            step.y - self.riser * self.slab * 0.5,
            step.x + self.tread * 0.5,
        )
    }

    /// The crest: the surface of the back step, which is how tall the bank is.
    pub fn crest(&self) -> f32 {
        self.step(self.rows.saturating_sub(1)).y
    }
}

/// The people in the stands.
///
/// **One mesh per bank, and that is the whole design.** A bank at a great
/// ground carries some three thousand of these; an entity each would be three
/// thousand walks, extracts and submits every frame for scenery that never
/// moves — against a frame that is spent per ENTITY and not per pixel (see
/// [`crate::app::perf`], where the same scene costs the same 3.9 ms at 720p
/// and at 4K). They are accumulated into the buffers below instead and spawned
/// as a single child of the bank, which also means they are hidden with it: a
/// lens that walks into a stand steps through the crowd in it rather than past
/// it.
///
/// **Stationary, but not identical.** Nothing here moves — a bank is one mesh
/// and it is baked once — so everything that tells one spectator from another
/// has to be baked in with him: his [`Posture`], the few degrees he is turned
/// off square, how big he is, what he came in, and which of two dozen heads is
/// on his shoulders. That is where the variety lives and it is free, because a
/// different pose is the same rings at different heights.
///
/// **Each of them is a modelled person.** A torso turned from hips to collar
/// with the shoulders sloping away, a rounded head with his own face wrapped
/// right round it, two arms, and a pair of legs for the ones on their feet.
/// They were a box with a smaller box on top, which is what a crowd looks like
/// from a hundred metres and nothing like what one looks like from three — and
/// the free camera goes to three.
pub struct Crowd;

impl Crowd {
    /// Metres between one spectator and the next along a row.
    ///
    /// A shade over a real seat, which is about half a metre. A figure is
    /// 0.44 m across the shoulders and rather more than that across the
    /// elbows, so two thirds of every row is people and the last third is the
    /// gap between them — and what closes THAT is the row behind, whose
    /// figures carry their own stagger and sit a riser higher. Which is how a
    /// real crowd reads from the halfway line: not as a row of people but as a
    /// speckled mass with its gaps filled in from behind.
    ///
    /// It was 0.80 to begin with, on the reasoning that half a metre of detail
    /// is invisible at a hundred and twenty. It is not: at that spacing the
    /// gap between two spectators is wider than a spectator, so the seats
    /// showed THROUGH the crowd and the bank read as a teal wall with figures
    /// scattered on it rather than as a stand with people in it. The thing the
    /// eye reads at distance is what fraction of the bank is crowd, and this
    /// is what sets it.
    ///
    /// Then 0.62 to 0.70, which is a seventh fewer people in every ground —
    /// and a fuller-looking one, which is the point. A spectator used to be a
    /// rectangle 0.42 m wide and that was the whole of him; he has arms now,
    /// and a tenth of any bank is standing up and a head taller than the
    /// seats. The bank closes on less.
    const SPACING: f32 = 0.70;

    /// Half the widest a figure ever gets, in metres: a man on his feet with
    /// his arms up, measured to the outside of a sleeve.
    ///
    /// What it is for is the ENDS of a bank. These are placed by their
    /// middles, so the run they are laid across has to be short of the
    /// concrete by this much either side or the man on the end has half of
    /// himself off the edge of it — which at a corner is a spectator standing
    /// in mid-air.
    const REACH: f32 = 0.50;

    /// How much of his own slot a spectator may wander across, either way.
    /// Enough to break the columns; not enough for two neighbours to meet.
    const STAGGER: f32 = 0.18;

    /// How far off square a figure may be turned, either way, in radians.
    ///
    /// Eight degrees, and it is worth more than it sounds. A bank is a grid
    /// and everything in it faces the same way; part of what says these are
    /// people rather than a printed pattern is that no two of them are quite
    /// parallel. Free, too — a turn is a different quaternion, not another
    /// vertex.
    const SQUARE: f32 = 0.14;

    /// Where in his step a seated man is, front to back. Toward the back,
    /// where the seat is — his legs take the rest of the tread. A man on his
    /// feet stands further forward, in front of his own seat.
    const SEATED_AT: f32 = 0.62;
    const AFOOT_AT: f32 = 0.46;

    /// One spectator, in metres: shoulders across, chest deep, and hips to the
    /// top of the shoulder.
    ///
    /// Off a real adult rather than picked to look right — hip to the top of
    /// the shoulder is about 0.56 m and shoulders are about 0.44 m apart, so a
    /// spectator is a little TALLER than he is wide. Built the other way round
    /// (0.54 by 0.46) they came out square, and a bank of squares reads as a
    /// stack of boxes rather than as people.
    const SHOULDERS: f32 = 0.44;
    const CHEST: f32 = 0.30;
    const TORSO: f32 = 0.56;

    /// …and the head above that, at its widest: across, up from the base of
    /// the neck, and front to back.
    ///
    /// Narrower than it is tall, and a good deal narrower than the shoulders
    /// under it. A head is roughly two fifths of the width across a man's
    /// shoulders; a cube the height of a head is nearer three fifths, and at
    /// this size that one ratio is the whole difference between a crowd and a
    /// tray of skittles.
    const HEAD: Vec3 = Vec3::new(0.198, 0.320, 0.212);

    /// How high his hips are above the step: sitting on it, and standing on
    /// it. The first is a hip joint over a seat, the second is what makes a
    /// man 1.70 m tall from the concrete to the top of his head.
    ///
    /// It is also what a lean turns about — see [`Seat`]. A figure leaned at
    /// the FEET tips the whole of himself forward, and the front of a hip ring
    /// then swings down through the step he is sitting on, which is what
    /// `every_spectator_is_sitting_on_a_step` catches.
    const SEAT: f32 = 0.10;
    const HIP: f32 = 0.86;

    /// How thick an arm is through the sleeve, and a leg through the trouser.
    const ARM: f32 = 0.052;
    const LEG: f32 = 0.088;

    /// **The rings a torso is turned on**: how far up it each sits and how far
    /// it reaches across and back — the first as a fraction of
    /// [`Self::TORSO`], the other two of half [`Self::SHOULDERS`] and half
    /// [`Self::CHEST`].
    ///
    /// The whole of what makes a coat a body rather than a filing cabinet is
    /// in the last two rings. A torso used to be one box, and a box has the
    /// same rectangular outline from every angle: two hard corners where the
    /// shoulders should slope away, and a head sitting on the flat top of it
    /// like a bucket on a crate. What the eye reads at every distance is the
    /// SILHOUETTE, and only geometry moves a silhouette.
    ///
    /// ⚠ **The trunk is 0.36 m across and not 0.44.** The wider figure is what
    /// a man measures at the shoulders, and the last four centimetres either
    /// side of him are his ARMS hanging on it — build the trunk to the full
    /// width and there is nowhere for them to be: they come out inside their
    /// own body, and the whole crowd is a slab with a hand painted on each
    /// side of it and a small head on top.
    const BODY: [(f32, f32, f32); 5] = [
        (0.00, 0.82, 1.00),
        (0.55, 0.78, 1.04),
        (0.86, 0.82, 0.94),
        (0.97, 0.52, 0.68),
        (1.04, 0.28, 0.39),
    ];
    /// How many points there are round one of them. Eight, so a ring reaches
    /// the width it is given — a hexagon has no point at a quarter turn and
    /// comes out an eighth narrow — and so a shoulder is round rather than cut
    /// off at a corner.
    const BODY_SIDES: usize = 8;

    /// **The rings a head is turned on**, and where each reads its band of the
    /// drawn tile: how far up the head it sits, how far it reaches across and
    /// back, and how far DOWN his tile it is painted. The first three are
    /// fractions of [`Self::HEAD`]; the fourth is the tile's own coordinate.
    ///
    /// A real skull seen front-on — the neck, then the jaw, **widest at the
    /// cheekbones**, the forehead standing almost as proud, and the crown
    /// taken well in. A head was one box, and no amount of drawing on the
    /// front of a box would have helped: what says "square" at ten pixels is
    /// the four hard corners of the outline, not the picture inside them.
    ///
    /// **The fourth number is not the first.** It is what the tile is spent
    /// on, and it is deliberately not proportional to height: the neck is
    /// three tenths of the lathe and gets two of the sheet, while the face —
    /// jaw to forehead, half the lathe — takes half of it. Tied to height, a
    /// third of every head's tile would go on a colour that could have been
    /// one texel. See
    /// [`CrowdPalette::head_uv`](crate::art::textures::CrowdPalette::head_uv),
    /// which does the same thing the other way round.
    /// The widest ring is where a man's EYES are, which is both true of a
    /// skull and the thing that ties the two halves of this table together:
    /// the tile column puts the eye line at 0.43, so the ring that carries it
    /// has to be the one at the cheekbones.
    const SKULL: [(f32, f32, f32, f32); 5] = [
        (0.00, 0.54, 0.56, 1.00),
        (0.30, 0.78, 0.86, 0.79),
        (0.55, 1.00, 1.00, 0.43),
        (0.76, 0.95, 0.98, 0.24),
        (0.93, 0.62, 0.68, 0.05),
    ];
    /// Points round a head. More than a body gets, because a head is the one
    /// thing in this scene a lens is ever walked up to: at eight the outline
    /// has six corners in it and reads as an oval at every size, and at four —
    /// which is what it was — it is a barrel however carefully the profile is
    /// drawn.
    const SKULL_SIDES: usize = 8;
    /// …and how many an arm or a leg gets. Four: a sleeve is a centimetre of
    /// screen at the distance any of this is seen from, and what it has to do
    /// there is BE there.
    const LIMB_SIDES: usize = 4;

    /// How much taller or shorter than that anybody gets. People are not one
    /// size, and a bank of identical figures reads as a printed pattern rather
    /// than as a crowd — the same reason the seats behind them are jittered.
    const SPREAD: f32 = 0.12;

    /// **How unevenly a crowd is spread**, as the swing either side of the
    /// ground's occupancy.
    ///
    /// Nobody arrives at a ground and is dealt a seat at random, which is what
    /// an independent draw per place amounts to and what this used to be: an
    /// even salt-and-pepper of people and gaps at exactly the same density
    /// everywhere, which is the one pattern a real crowd never makes. People
    /// come in twos and fours and sit together; a block sells out while the
    /// one beside it does not; a corner nobody wants stays empty all season.
    ///
    /// One, so the local density runs from half the ground's average to half
    /// again above it. At a near sell-out that reads as a full stand with a
    /// few thinner patches — the clamp at the top does the work — and at a
    /// half-empty one it reads as knots of people with daylight between them,
    /// which is what a poorly attended match actually looks like.
    const CLUMPING: f32 = 1.0;

    /// The two scales it varies over, in slots across and rows up: whole
    /// blocks of a stand, and knots of people inside them.
    ///
    /// Two octaves rather than one because they are two different things. One
    /// alone gives either a stand with a few large soft patches and no groups
    /// in it, or groups with no sense that one part of the ground is busier
    /// than another. The coarse one carries about two thirds.
    const CLUMP_BLOCK: (f32, f32) = (22.0, 9.0);
    const CLUMP_GROUP: (f32, f32) = (6.0, 3.0);

    /// Salt for the second and third draws off a slot.
    ///
    /// A man's coat, his complexion and his trousers come from DIFFERENT
    /// hashes rather than from three slices of one. Cut from overlapping bits
    /// they correlate, and a stand where everybody in a red coat has the same
    /// face is a stand nobody believes — the same trap `Complexion::face`
    /// documents on the pitch.
    const COMPLEXION: u32 = 0x5BF0_3635;
    const TROUSERS: u32 = 0x2E9A_C41D;

    /// Everybody in one bank, as a single mesh. `None` when there is nobody
    /// there — which a bank with no steps and a crowd of nought both are.
    ///
    /// `seed` is what makes the four banks of a ground differ. Without one
    /// every bank draws the same figures in the same places, and the two ends
    /// are visibly the same photograph.
    pub fn fill(
        terrace: &Terrace,
        stature: Stature,
        stand: Stand,
        palette: &CrowdPalette,
        seed: u32,
    ) -> Option<Mesh> {
        // The run the seats are laid across, which is the bank less the widest
        // spectator it could seat: these are placed by their middles, so a run
        // of the full length puts the two on the ends half a shoulder off the
        // end of the concrete.
        let run = (terrace.length - Self::REACH * 2.0 * (1.0 + Self::SPREAD)).max(0.0);
        let slots = (run / Self::SPACING).floor() as usize;
        if slots == 0 || terrace.rows == 0 {
            return None;
        }
        // Spread over the true run rather than left at `SPACING`, so a bank
        // ends with a seat at each end instead of with a gap of whatever the
        // division left over.
        let spacing = run / slots as f32;
        let occupancy = stature.occupancy();
        let allegiance = stature.allegiance(stand);
        let afoot = stature.on_their_feet(stand);
        // Whose colours this bank wears at all. An end belongs to one support
        // or the other; a touchline is the home club's, thinly.
        let wearing = match stand {
            Stand::AwayEnd => CrowdPalette::visitors as fn(&CrowdPalette, u32) -> Vec2,
            _ => CrowdPalette::colours as fn(&CrowdPalette, u32) -> Vec2,
        };

        let mut figures =
            Figures::with_capacity((slots * terrace.rows) as f32 * occupancy);

        for row in 0..terrace.rows {
            let step = terrace.step(row);
            for slot in 0..slots {
                // Whether anybody is here at all, against the density of THIS
                // corner of the bank rather than the ground's average — see
                // [`Self::CLUMPING`].
                let roll = Self::hash(seed, row as u32, slot as u32);
                let here = occupancy
                    * (1.0 - Self::CLUMPING * 0.5
                        + Self::CLUMPING * Self::clumping(seed, row, slot));
                if Self::unit(roll) >= here.clamp(0.0, 1.0) {
                    continue;
                }

                let stagger = (Self::unit(roll >> 8) - 0.5) * 2.0 * Self::STAGGER * spacing;
                let x = run * -0.5 + spacing * (slot as f32 + 0.5) + stagger;
                let size = 1.0 + (Self::unit(roll >> 16) - 0.5) * 2.0 * Self::SPREAD;
                let head = Self::hash(seed ^ Self::COMPLEXION, row as u32, slot as u32);
                let dress = Self::hash(seed ^ Self::TROUSERS, row as u32, slot as u32);

                // What he is doing with himself, which is what decides where
                // every ring of him goes.
                let posture = Posture::of(Self::unit(head >> 16), afoot);
                let hips = posture.hips();
                let seat = Seat {
                    pivot: Vec3::new(
                        x,
                        step.y + hips * size,
                        step.x + terrace.tread * posture.stands_at(),
                    ),
                    hips,
                    size,
                    turn: Quat::from_rotation_y(
                        (Self::unit(head >> 24) - 0.5) * 2.0 * Self::SQUARE,
                    ) * Quat::from_rotation_x(-posture.lean()),
                };

                // What he came in. Whether he is wearing anybody's colours at
                // all is a draw off the SALTED hash, because it has to be
                // independent of the tile he then picks — cut from
                // neighbouring bits of one draw, everybody in colours would
                // end up in the same shade of it.
                //
                // …and how likely that is depends on WHERE he is sitting. An
                // end's support is massed behind the goal and thins through
                // the corners; a touchline carries its scattering evenly.
                let dressed = match stand {
                    Stand::Side => allegiance,
                    _ => allegiance * Stature::behind_goal(x / (run * 0.5)),
                };
                let clothing = if Self::unit(head >> 8) < dressed {
                    wearing(palette, roll >> 24)
                } else {
                    palette.coat(roll >> 24)
                };
                let skin = palette.head(head);

                // The body, from the hem of his coat to his collar.
                let body: [Ring; Self::BODY.len()] = std::array::from_fn(|course| {
                    let (up, wide, deep) = Self::BODY[course];
                    // The bottom ring is the hem, and where it hangs depends
                    // on whether he is sitting on it — see [`Posture::hem`].
                    let up = if course == 0 { up - posture.hem() } else { up };
                    seat.upright(
                        hips + Self::TORSO * up,
                        Self::SHOULDERS * 0.5 * wide,
                        Self::CHEST * 0.5 * deep,
                    )
                });
                figures.tube(&body, Self::BODY_SIDES, false, |_, _| clothing);

                // The head on top of it, turned on [`Self::SKULL`] and skinned
                // with his own tile the whole way round — hair at the back, an
                // ear either side, his face at the front. The `turn` of each
                // point is what the tile is laid on, so the drawing follows
                // the geometry as the head narrows under it.
                let neck = hips + Self::TORSO * 0.99;
                let skull = Self::SKULL.map(|(up, wide, deep, _)| {
                    seat.upright(
                        neck + Self::HEAD.y * up,
                        Self::HEAD.x * 0.5 * wide,
                        Self::HEAD.z * 0.5 * deep,
                    )
                });
                figures.tube(&skull, Self::SKULL_SIDES, true, |turn, course| {
                    palette.head_uv(head, turn, Self::SKULL[course].3)
                });
                figures.cap(skull[Self::SKULL.len() - 1], Self::SKULL_SIDES, |turn| {
                    palette.head_uv(head, turn, 0.0)
                });

                // Two arms, which are most of what says "person" at the
                // distance a lens actually gets to. Mirrored rather than given
                // their own table: nobody has one arm longer than the other,
                // and a second set of joints is a second set of numbers
                // waiting to drift.
                let joints = posture.arm(hips);
                for hand in [-1.0f32, 1.0] {
                    let [top, elbow, wrist] =
                        joints.map(|joint| Vec3::new(joint.x * hand, joint.y, joint.z));
                    let limb = [
                        // Thickest at the top, where it is a shoulder and has to
                        // MERGE with the one it hangs off: an even tube pinned
                        // to the side of a coat is a scarecrow.
                        seat.limb(top, elbow - top, Self::ARM * 1.34),
                        seat.limb(elbow, wrist - top, Self::ARM),
                        seat.limb(wrist, wrist - elbow, Self::ARM * 0.78),
                    ];
                    figures.tube(&limb, Self::LIMB_SIDES, false, |_, _| clothing);
                    // The back of his hand, which is the one end of an arm
                    // that is out in the open air.
                    figures.cap(limb[2], Self::LIMB_SIDES, |_| skin);
                }

                // …and two legs, but only for the men on their feet. A seated
                // pair of knees is inside the tread of his own step with the
                // nose of the step in front drawn across it, and any lens high
                // enough to see over that nose is looking down at the top of
                // his head. On a man standing up they are half of him.
                if let Some(joints) = posture.leg() {
                    let trousers = palette.coat(dress);
                    for foot in [-1.0f32, 1.0] {
                        let [top, knee, ankle] =
                            joints.map(|joint| Vec3::new(joint.x * foot, joint.y, joint.z));
                        let limb = [
                            seat.limb(top, knee - top, Self::LEG),
                            seat.limb(knee, ankle - top, Self::LEG * 0.86),
                            seat.limb(ankle, ankle - knee, Self::LEG * 0.74),
                        ];
                        figures.tube(&limb, Self::LIMB_SIDES, false, |_, _| trousers);
                    }
                }
            }
        }

        figures.into_mesh()
    }

    /// **How busy this corner of the bank is**, 0..1, varying smoothly across
    /// it — the field that turns an even sprinkle of people into groups with
    /// gaps between them.
    ///
    /// Value noise: a lattice of hashed corners, read with a smoothstep
    /// between them, at the two scales [`Self::CLUMP_BLOCK`] and
    /// [`Self::CLUMP_GROUP`]. Smooth is the whole requirement — a spectator
    /// has to be MORE likely to be there when his neighbours are, and any
    /// field with that property will do. It is deterministic off the same
    /// `seed` as everything else, so a reloaded page seats the same people in
    /// the same places.
    fn clumping(seed: u32, row: usize, slot: usize) -> f32 {
        let octave = |cell: (f32, f32), salt: u32| {
            let (across, up) = (slot as f32 / cell.0, row as f32 / cell.1);
            let (left, under) = (across.floor(), up.floor());
            let corner = |along: u32, above: u32| {
                Self::unit(Self::hash(
                    seed ^ salt,
                    under as u32 + above,
                    left as u32 + along,
                ))
            };
            let (fx, fy) = (Self::ease(across - left), Self::ease(up - under));
            let low = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * fx;
            let high = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * fx;
            low + (high - low) * fy
        };
        octave(Self::CLUMP_BLOCK, 0x1B87_3593) * 0.65
            + octave(Self::CLUMP_GROUP, 0x6F1B_2D0C) * 0.35
    }

    /// Smoothstep. Straight interpolation between lattice corners leaves a
    /// crease along every cell edge, which on a stand shows up as a grid.
    fn ease(step: f32) -> f32 {
        step * step * (3.0 - 2.0 * step)
    }

    /// A mix, not a random number: the same bank has to come back the same way
    /// on every load, or a reloaded page reseats the entire ground.
    fn hash(seed: u32, row: u32, slot: u32) -> u32 {
        let mut hash = seed
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(row.wrapping_mul(0x85EB_CA6B))
            .wrapping_add(slot.wrapping_mul(0xC2B2_AE35));
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(0x2545_F491);
        hash ^ (hash >> 13)
    }

    /// The low byte of a hash as 0..1.
    fn unit(hash: u32) -> f32 {
        (hash & 0xFF) as f32 / 255.0
    }
}

/// **What a spectator is doing with himself.**
///
/// Nothing in this crowd moves — see the note on [`Crowd`] — so a posture is
/// the only variety a bank has in its SHAPES, and it is worth more than any of
/// the modelling it is applied to. A stand where every figure sits identically
/// reads as a printed pattern however carefully each one is turned, for the
/// same reason the coats have to run from near-black to pale.
///
/// It is also the cheapest thing in the file: all four of these are the same
/// rings at different heights and angles, so a bank of them costs exactly what
/// a bank of one of them costs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Posture {
    /// Back in his seat, hands on his knees.
    Seated,
    /// Forward on the edge of it, elbows on his knees and his hands together.
    /// What most of a stand is doing whenever the ball is near a box.
    Forward,
    /// On his feet. Where the support is, that is the whole match.
    Standing,
    /// On his feet with both arms up.
    Cheering,
}

impl Posture {
    /// How much of the part of a bank that is on its feet has its arms up as
    /// well. A ground always has some, and they are the figures that break the
    /// top line of a crowd — but only some: a whole end at full stretch is a
    /// cup final, not a Tuesday.
    const CHEERING: f32 = 0.14;
    /// …and how much of what is left in its seat is forward on the edge of it.
    const FORWARD: f32 = 0.34;

    /// Which of them this man is, off one draw and the share of this bank that
    /// is up.
    fn of(roll: f32, afoot: f32) -> Self {
        if roll < afoot {
            // Re-read off the same draw rather than a second one, which is
            // what keeps the two shares independent: the cheering fraction is
            // measured against the standing block it is cut out of.
            if roll < afoot * Self::CHEERING {
                Self::Cheering
            } else {
                Self::Standing
            }
        } else if roll > 1.0 - (1.0 - afoot) * Self::FORWARD {
            Self::Forward
        } else {
            Self::Seated
        }
    }

    /// Whether he is on his feet, which is what everything below turns on.
    fn afoot(self) -> bool {
        matches!(self, Self::Standing | Self::Cheering)
    }

    /// How high his hips are above the step he is on, in metres.
    fn hips(self) -> f32 {
        if self.afoot() {
            Crowd::HIP
        } else {
            Crowd::SEAT
        }
    }

    /// Where in the tread of that step he is, front to back.
    fn stands_at(self) -> f32 {
        if self.afoot() {
            Crowd::AFOOT_AT
        } else {
            Crowd::SEATED_AT
        }
    }

    /// How far forward he is leaning, in radians, measured at the step. Never
    /// zero for anybody: a figure standing exactly upright is the one pose no
    /// human being holds.
    fn lean(self) -> f32 {
        match self {
            Self::Seated => 0.07,
            Self::Forward => 0.34,
            Self::Standing => 0.05,
            Self::Cheering => -0.06,
        }
    }

    /// **His shoulder, his elbow and his hand**, in his own space, for the arm
    /// on one side of him; the other is the mirror of it. `hips` is where his
    /// hips are, so the same table serves a man sitting down and a man
    /// standing up.
    fn arm(self, hips: f32) -> [Vec3; 3] {
        // Outside the trunk, which is 0.18 m of half-width — see
        // [`Crowd::BODY`]. An arm whose centre line runs inside its own body
        // is an arm nobody can see, and what is left of it on screen is the
        // back of a hand apparently stuck to a man's ribs.
        let shoulder = Vec3::new(0.148, hips + Crowd::TORSO * 0.86, 0.005);
        match self {
            // Hanging down his side, the forearm forward and in onto his knee.
            Self::Seated => [
                shoulder,
                Vec3::new(0.204, hips + Crowd::TORSO * 0.34, -0.045),
                Vec3::new(0.172, hips + Crowd::TORSO * 0.02, -0.195),
            ],
            // Elbows on his knees, hands together in front of him.
            Self::Forward => [
                shoulder,
                Vec3::new(0.212, hips + Crowd::TORSO * 0.20, -0.215),
                Vec3::new(0.095, hips + Crowd::TORSO * 0.46, -0.320),
            ],
            // Straight down, which is what a man watching football does with
            // his arms when he is not doing anything else with them.
            Self::Standing => [
                shoulder,
                Vec3::new(0.196, hips + Crowd::TORSO * 0.32, -0.020),
                Vec3::new(0.188, hips - Crowd::TORSO * 0.12, 0.020),
            ],
            // Up and out.
            Self::Cheering => [
                shoulder,
                Vec3::new(0.285, hips + Crowd::TORSO * 1.24, -0.055),
                Vec3::new(0.305, hips + Crowd::TORSO * 1.78, -0.030),
            ],
        }
    }

    /// …and his hip, knee and ankle, where he has legs worth drawing at all.
    fn leg(self) -> Option<[Vec3; 3]> {
        self.afoot().then_some([
            Vec3::new(0.100, Crowd::HIP, 0.0),
            Vec3::new(0.114, Crowd::HIP * 0.52, -0.025),
            Vec3::new(0.114, 0.055, 0.020),
        ])
    }

    /// **How far below his hips his coat hangs**, as a fraction of
    /// [`Crowd::TORSO`].
    ///
    /// Nothing at all for a man in his seat: he is sitting on the hem of it,
    /// and a skirt of coat below the hip ring would be a skirt hanging in mid
    /// air over the step behind him. On his feet it comes a third of the way
    /// down his thigh, which is where a winter coat ends and — more to the
    /// point — the difference between a man standing up and a torso on stilts.
    fn hem(self) -> f32 {
        if self.afoot() { 0.30 } else { 0.0 }
    }
}

/// **Where a spectator is and how he is sitting in it**: the frame every ring
/// of him is turned in.
///
/// A figure is written once, in his own space — `x` across his own shoulders,
/// `y` up from the step under him, `z` back from the pitch — and this puts it
/// where it belongs. Which is what lets a lean and a turn cost nothing: they
/// are one quaternion applied to a table of joints, not a second table.
///
/// ⚠ **The turn is about his HIPS, not about his feet.** A person bends at the
/// waist and a stand full of people bending at the ankles is a stand full of
/// ironing boards — but the reason it is written down is arithmetic rather
/// than anatomy: leaned at the feet, the front of a seated man's hip ring
/// swings down through the step he is sitting on, and a fifth of the crowd
/// ends up buried in the concrete.
struct Seat {
    /// The point he turns about: the middle of his place, at hip height.
    pivot: Vec3,
    /// How far that is above the step, in his own units — what a local `y` is
    /// measured back from.
    hips: f32,
    /// How much bigger or smaller than the standard figure he is.
    size: f32,
    /// Which way he is facing and how far forward he is leaning, together.
    turn: Quat,
}

impl Seat {
    /// A point of his, in his own space, put where it belongs.
    fn point(&self, local: Vec3) -> Vec3 {
        self.pivot + self.turn * ((local - Vec3::Y * self.hips) * self.size)
    }

    /// A direction of his — scaled with him, because a bigger man's arm is
    /// both longer and thicker.
    fn axis(&self, local: Vec3) -> Vec3 {
        self.turn * (local * self.size)
    }

    /// **An upright ring**, which is what a torso and a head are turned on:
    /// `up` above the step, reaching `across` his shoulders and `depth` toward
    /// the pitch.
    fn upright(&self, up: f32, across: f32, depth: f32) -> Ring {
        Ring {
            centre: self.point(Vec3::Y * up),
            across: self.axis(Vec3::X * across),
            depth: self.axis(Vec3::Z * depth),
        }
    }

    /// **A ring square to a limb** running in `along`, which is what an arm
    /// and a leg are turned on. Round rather than elliptical: a sleeve is.
    ///
    /// The frame is taken off a world axis the limb is NOT lined up with. A
    /// fixed one puts a singularity in the middle of an arm that happens to
    /// hang straight down, which is exactly the arm most of a stand has.
    fn limb(&self, at: Vec3, along: Vec3, thick: f32) -> Ring {
        let along = self.axis(along).normalize_or(Vec3::NEG_Y);
        let seed = if along.y.abs() < 0.86 { Vec3::Y } else { Vec3::Z };
        let across = along.cross(seed).normalize_or(Vec3::X);
        Ring {
            centre: self.point(at),
            across: across * (thick * self.size),
            // Crossed back the same way round, so the ring's own outward
            // direction — which is what closes the end of a limb — runs along
            // it rather than back up it. See [`Figures::cap`].
            depth: across.cross(along) * (thick * self.size),
        }
    }
}

/// One course of a lathe: where its centre sits and the two half-axes its
/// points are laid out on.
///
/// Two axes rather than a radius and a normal, because they carry the shape as
/// well as the place: a torso is wider than it is deep and a head is deeper
/// than it is wide, and both fall out of the pair without a special case.
#[derive(Clone, Copy)]
struct Ring {
    centre: Vec3,
    across: Vec3,
    depth: Vec3,
}

impl Ring {
    /// The point `turn` of the way round, `0` at the front — the side looking
    /// at the football — and `±1` at the back.
    ///
    /// The same measure [`CrowdPalette::head_uv`](crate::art::textures::CrowdPalette::head_uv)
    /// takes, and deliberately so: the geometry and the drawing on it are laid
    /// out in one coordinate, so a man's nose is on the front of his face
    /// because it is the same number in both places rather than because two
    /// tables agree.
    fn point(&self, turn: f32) -> Vec3 {
        let (sin, cos) = (std::f32::consts::PI * turn).sin_cos();
        self.centre - self.across * sin - self.depth * cos
    }
}

/// What it takes to put people in a stand: the one material every spectator in
/// the ground is drawn with, and the palette their colours are picked out of.
///
/// One of these for the whole stadium. The material is shared on purpose — it
/// is the same PBR program the terracing behind it draws through, so the four
/// banks of people cost the browser no shader it was not already linking (see
/// [`Textures::crowd`](crate::art::textures::Textures::crowd), which is where
/// that constraint is argued out).
pub struct Spectators {
    material: Handle<StandardMaterial>,
    palette: CrowdPalette,
}

impl Spectators {
    /// Paints the palette and registers the material. `home` and `trim` are
    /// the shirt of the side whose ground it is: its support is what is in
    /// the stands.
    pub fn dressed(
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
        home: (Color, Color),
        visitor: (Color, Color),
    ) -> Self {
        let palette = Textures::crowd(images, home.0, home.1, visitor.0, visitor.1);
        Spectators {
            // White, so the swatch arrives as it was written. Rough, because
            // a crowd is wool and skin — the one thing it must not do is
            // catch a highlight, which at this size reads as a stand full of
            // wet plastic.
            material: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(palette.sheet.clone()),
                perceptual_roughness: 0.98,
                ..default()
            }),
            palette,
        }
    }

    /// One bank's worth of people, ready to be spawned as a child of it.
    /// `None` when there is nobody to seat.
    pub fn seat(
        &self,
        meshes: &mut Assets<Mesh>,
        terrace: &Terrace,
        stature: Stature,
        stand: Stand,
        seed: u32,
    ) -> Option<(Mesh3d, MeshMaterial3d<StandardMaterial>)> {
        let crowd = Crowd::fill(terrace, stature, stand, &self.palette, seed)?;
        Some((
            Mesh3d(meshes.add(crowd)),
            MeshMaterial3d(self.material.clone()),
        ))
    }
}

/// The buffers a bank's spectators are accumulated into.
///
/// Flat vectors rather than a merge of ten thousand primitives: a merge
/// allocates a mesh, its attribute vectors and its index vector per PART, and
/// there are nine parts to a spectator. Same shape as `LineMesh` in
/// [`pitch`](crate::scene::pitch), and for the same reason.
///
/// **Normals are accumulated, not written.** Every part of a figure is a
/// lathe, and a lathe's vertices are shared between the courses that meet at
/// them: each triangle adds its own normal to its three corners and
/// [`Self::into_mesh`] normalises the lot at the end, which is both the
/// cheapest way to store the thing — one vertex per point instead of one per
/// corner of every quad — and the only way to get a SMOOTH shoulder out of it.
/// Written per face, as the boxes used to be, a turned torso is a stack of
/// flat panels catching the light one at a time, which is a lantern rather
/// than a coat.
struct Figures {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Figures {
    /// Vertices to a spectator: his torso's rings, his head's — a point wider,
    /// because its `u` runs the whole way round and the two ends of it are the
    /// same place with different tile coordinates — the crown that closes it,
    /// and two arms with the back of a hand on the end of each.
    ///
    /// Only a reservation, so it costs nothing to be a little out: the men on
    /// their feet carry a pair of legs this does not count, and they are a
    /// tenth of a main stand. It costs a re-allocation of a very large buffer
    /// to be far out.
    const VERTICES: usize = Crowd::BODY.len() * Crowd::BODY_SIDES
        + Crowd::SKULL.len() * (Crowd::SKULL_SIDES + 1)
        + (Crowd::SKULL_SIDES + 1)
        + 2 * (3 * Crowd::LIMB_SIDES + Crowd::LIMB_SIDES + 1);
    /// …and the triangles over them: two to every quad up a tube, plus the fan
    /// that closes a head and a hand. Counted rather than guessed at four to a
    /// vertex, which is a shade under and so bought a re-allocation of a
    /// forty-megabyte buffer per bank.
    const INDICES: usize = 3
        * (2 * (Crowd::BODY.len() - 1) * Crowd::BODY_SIDES
            + 2 * (Crowd::SKULL.len() - 1) * Crowd::SKULL_SIDES
            + Crowd::SKULL_SIDES
            + 2 * (2 * 2 * Crowd::LIMB_SIDES + Crowd::LIMB_SIDES));

    fn with_capacity(figures: f32) -> Self {
        let figures = figures.ceil().max(0.0) as usize;
        Figures {
            positions: Vec::with_capacity(figures * Self::VERTICES),
            normals: Vec::with_capacity(figures * Self::VERTICES),
            uvs: Vec::with_capacity(figures * Self::VERTICES),
            indices: Vec::with_capacity(figures * Self::INDICES),
        }
    }

    /// **A closed tube through a stack of rings** — which is what every part
    /// of a spectator is. `sides` points round each ring, and `uv` says where
    /// each of them reads its colour, off how far round it is and which course
    /// it belongs to.
    ///
    /// `seam` is for the one part whose tile is WRAPPED rather than flat. A
    /// flat-coloured tube closes on itself and the last point is the first, so
    /// there is nothing to duplicate; a head's `u` runs from one edge of its
    /// tile to the other, and the point where it meets itself has to exist
    /// twice — once at `u = 0` and once at `u = 1` — or the whole tile is
    /// squeezed backwards across the back of his skull in one quad.
    fn tube(
        &mut self,
        rings: &[Ring],
        sides: usize,
        seam: bool,
        uv: impl Fn(f32, usize) -> Vec2,
    ) {
        let points = if seam { sides + 1 } else { sides };
        let base = self.positions.len() as u32;
        for (course, ring) in rings.iter().enumerate() {
            for point in 0..points {
                let turn = -1.0 + 2.0 * point as f32 / sides as f32;
                self.vertex(ring.point(turn), uv(turn, course));
            }
        }
        for course in 1..rings.len() {
            let (under, over) = (
                base + ((course - 1) * points) as u32,
                base + (course * points) as u32,
            );
            for point in 0..sides as u32 {
                let (here, next) = (point, (point + 1) % points as u32);
                self.face(under + here, under + next, over + next, over + here);
            }
        }
    }

    /// **A fan over the end of a ring**: the crown of a head, the back of a
    /// hand. It faces the way the ring's own two axes cross, which for a limb
    /// is out along the limb — see [`Seat::limb`].
    fn cap(&mut self, ring: Ring, sides: usize, uv: impl Fn(f32) -> Vec2) {
        let base = self.positions.len() as u32;
        self.vertex(ring.centre, uv(0.0));
        for point in 0..sides {
            let turn = -1.0 + 2.0 * point as f32 / sides as f32;
            self.vertex(ring.point(turn), uv(turn));
        }
        for point in 0..sides as u32 {
            self.triangle(base, base + 1 + point, base + 1 + (point + 1) % sides as u32);
        }
    }

    fn vertex(&mut self, at: Vec3, uv: Vec2) {
        self.positions.push(at.to_array());
        self.normals.push([0.0; 3]);
        self.uvs.push(uv.to_array());
    }

    /// One quad, its corners counter-clockwise seen from outside.
    fn face(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    /// One triangle, adding its own normal to each of its three corners.
    ///
    /// Deliberately NOT normalised first: the cross product of two edges is
    /// twice the triangle's area, so a big face pulls a shared vertex further
    /// than a sliver does. That is the weighting that keeps a lathe's poles
    /// from being dragged sideways by the ring of thin triangles that meets
    /// there.
    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        let at = |index: u32| Vec3::from_array(self.positions[index as usize]);
        let normal = (at(b) - at(a)).cross(at(c) - at(a));
        for corner in [a, b, c] {
            let running = &mut self.normals[corner as usize];
            *running = (Vec3::from_array(*running) + normal).to_array();
        }
        self.indices.extend_from_slice(&[a, b, c]);
    }

    fn into_mesh(mut self) -> Option<Mesh> {
        if self.positions.is_empty() {
            return None;
        }
        for normal in &mut self.normals {
            // `normalize_or` rather than `normalize`: a ring given no width at
            // all — which is what the apex of a fan is — collects nothing but
            // degenerate triangles, and a zero-length normal reaches the
            // shader as a black vertex.
            *normal = Vec3::from_array(*normal).normalize_or(Vec3::Y).to_array();
        }
        Some(
            Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
            .with_inserted_indices(Indices::U32(self.indices)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A club drawing `attendance` into a ground that holds `capacity` — the
    /// pair the database actually carries — hosting a side of its own
    /// standing, so nothing but the arguments moves the answer.
    fn venue(capacity: u32, attendance: u32, reputation: u16, youth: bool) -> VenueInfo {
        VenueInfo {
            capacity,
            attendance,
            reputation,
            visitor: reputation,
            youth,
        }
    }

    fn terrace(rows: usize) -> Terrace {
        Terrace {
            length: 40.0,
            rows,
            riser: 0.64,
            tread: 1.25,
            from: 37.4,
            slab: 1.9,
        }
    }

    /// The rule the stands were rebuilt to answer: an academy fixture and a
    /// small club's ground both get the smallest bank there is, whatever else
    /// is true about them.
    #[test]
    fn a_youth_ground_is_five_steps_of_concrete() {
        // A great club's under-18s, playing at the training ground behind a
        // stadium that holds sixty thousand and fills it.
        let academy = Stature::of(&venue(60_000, 54_000, 9_500, true));
        assert_eq!(academy.rows(34), Stature::FEWEST_ROWS);
        // …and the parent club's full house does NOT come with them. The
        // stand is small and so is the crowd; the gate on record belongs to a
        // different ground on a different day.
        let senior = Stature::of(&venue(60_000, 54_000, 9_500, false));
        assert!(
            academy.occupancy() < senior.occupancy() * 0.25,
            "the academy drew {} against the first team's {}",
            academy.occupancy(),
            senior.occupancy()
        );

        // …and a fourth-tier side nobody ever counted a crowd for, whose
        // capacity the simulator ESTIMATED — generously — off the very
        // reputation the cap exists to read.
        let village = Stature::of(&venue(16_000, 0, 3_900, false));
        assert_eq!(village.rows(34), Stature::FEWEST_ROWS);
    }

    /// …and that a great ground gets every row there is.
    #[test]
    fn a_great_ground_is_built_to_its_full_height() {
        let elite = Stature::of(&venue(62_000, 54_000, 9_800, false));
        assert_eq!(elite.rows(34), 34);
        assert_eq!(elite.rows(31), 31);
        assert_eq!(elite.overhang(4.0, 30.0), 30.0);
    }

    /// **The fixture this ladder was re-cut against.**
    ///
    /// Lokomotiv Moscow at home to Zenit: a 13,133 gate into a 16,000 ground,
    /// world reputation 7,200 — the club's real row in `database.db`. It is an
    /// ordinary big-club league match, the kind most of the football in this
    /// game is, and it came out on screen as a low wall under a great deal of
    /// empty sky. At 50,000-to-a-great-ground it drew thirteen rows of
    /// twenty-one; the bank it gets now is a shade over twice as tall.
    ///
    /// The numbers are pinned rather than described because the whole point of
    /// them is that a real fixture in the middle of the distribution reads as
    /// a stadium. A change that quietly puts this one back under fifteen rows
    /// is the regression this test exists to catch.
    #[test]
    fn a_moscow_derby_is_played_in_a_stadium() {
        let loko = Stature::of(&venue(16_016, 13_133, 7_200, false));
        let rows = loko.rows(34);
        assert!(
            (22..=27).contains(&rows),
            "Lokomotiv drew 13,133 and got {rows} rows"
        );
        // Twenty-four rows at 0.72 m is 17 m of stand, against the 8.3 m the
        // same gate used to be given.
        assert!(rows as f32 * 0.72 > 15.0, "{rows} rows is not a stadium");
        // …and it is a proper crowd in them: 13,133 into 16,016 is a ground
        // four fifths full.
        assert!(loko.occupancy() > 0.75, "{}", loko.occupancy());
    }

    /// The ladder has to span the world's clubs rather than bunch them at one
    /// end. Percentiles measured across the 1,335 clubs in `database.db` that
    /// carry a gate — see the note on [`Stature::GRAND_GATE`].
    #[test]
    fn the_ladder_spreads_the_worlds_grounds_across_it() {
        // (gate, the band of rows it should land in, out of 34)
        for (gate, low, high) in [
            (500u32, 5usize, 8usize),    // p10 — a village club
            (1_500, 6, 11),              // p25
            (4_000, 11, 17),             // p50
            (14_000, 21, 28),            // p75 — Lokomotiv's neighbourhood
            (25_700, 29, 34),            // p90
            (42_600, 34, 34),            // p97 — every row there is
        ] {
            let rows = Stature::of(&venue(gate * 2, gate, 8_000, false)).rows(34);
            assert!(
                (low..=high).contains(&rows),
                "a {gate} gate got {rows} rows, wanted {low}..={high}"
            );
        }
    }

    /// Between the two it has to actually vary, and it is the GATE that has to
    /// move it. A curve that saturated at either end would be the tier lookup
    /// the module note rejects, written out as arithmetic.
    #[test]
    fn the_gate_is_what_decides_the_height() {
        let mut last = Stature::FEWEST_ROWS;
        for attendance in [2_500, 7_000, 12_000, 18_000, 26_000] {
            // The same ground every time — only the crowd in it differs, so
            // nothing but the gate can be moving this.
            let rows = Stature::of(&venue(60_000, attendance, 8_000, false)).rows(34);
            assert!(
                rows > last,
                "a gate of {attendance} got {rows} rows, no more than the smaller crowd below it"
            );
            assert!(rows <= 34, "a gate of {attendance} came to {rows} rows");
            last = rows;
        }
    }

    /// …and the share of the ground that gate filled is what decides how many
    /// coats are on the steps. Same crowd, twice the stadium, half as full.
    #[test]
    fn the_gate_against_the_ground_is_what_decides_the_crowd() {
        let packed = Stature::of(&venue(12_000, 11_000, 8_000, false));
        let rattling = Stature::of(&venue(40_000, 11_000, 8_000, false));
        assert!(
            packed.occupancy() > rattling.occupancy() + 0.3,
            "{} against {}",
            packed.occupancy(),
            rattling.occupancy()
        );
        // Neither is ever a sell-out or a wake — an empty bank of concrete and
        // a bank with a figure in every slot both read as a mistake.
        for stature in [packed, rattling] {
            assert!((Stature::SPARSEST..=Stature::FULLEST).contains(&stature.occupancy()));
        }
    }

    /// **Who is visiting moves the crowd — and must not move the stand.**
    ///
    /// The two halves of this are one test on purpose. A ground filling for
    /// the fixture everybody wants to see is right; the same ground GROWING
    /// for it is the bug that was there when the height and the crowd came off
    /// one number, and it would look plausible in any single screenshot.
    #[test]
    fn the_visitors_fill_the_ground_without_enlarging_it() {
        // The same club at home three times: to a giant, to a peer, and to the
        // side propping up the table.
        let against = |visitor: u16| {
            Stature::of(&VenueInfo {
                capacity: 30_000,
                attendance: 21_000,
                reputation: 7_000,
                visitor,
                youth: false,
            })
        };
        let (glamour, ordinary, nobody) = (against(9_500), against(7_000), against(3_500));

        assert!(
            glamour.occupancy() > ordinary.occupancy(),
            "a great visiting side drew no better than a peer: {} against {}",
            glamour.occupancy(),
            ordinary.occupancy()
        );
        assert!(
            nobody.occupancy() < ordinary.occupancy() - 0.1,
            "the bottom club drew {} against a peer's {}",
            nobody.occupancy(),
            ordinary.occupancy()
        );

        // …and the concrete never moved.
        for fixture in [glamour, nobody] {
            assert_eq!(
                fixture.rows(34),
                ordinary.rows(34),
                "the stadium changed size for the opposition"
            );
            assert_eq!(fixture.overhang(4.0, 30.0), ordinary.overhang(4.0, 30.0));
        }
    }

    /// A small club hosting a giant sells the place out, which is the one case
    /// the swing has to be allowed to run all the way into.
    #[test]
    fn a_giant_visiting_a_small_club_fills_it() {
        let cup_tie = Stature::of(&VenueInfo {
            capacity: 8_000,
            attendance: 5_200,
            reputation: 4_400,
            visitor: 9_600,
            youth: false,
        });
        assert!(cup_tie.occupancy() > 0.8, "{}", cup_tie.occupancy());
    }

    /// **The ends wear the club and the sides wear coats**, and a big club
    /// wears more of both.
    ///
    /// The gap between an end and a side is the whole of what makes an end
    /// read as one from across the ground, so it is asserted as a gap rather
    /// than as two numbers: any change that narrows it has taken the ends away
    /// whatever the constants still say.
    #[test]
    fn the_colours_belong_in_the_ends() {
        let ground = |gate: u32, reputation: u16| {
            Stature::of(&venue(gate * 4 / 3, gate, reputation, false))
        };
        let great = ground(40_000, 9_500);
        let village = ground(900, 4_100);

        for club in [great, village] {
            assert!(
                club.allegiance(Stand::HomeEnd) > club.allegiance(Stand::Side) * 2.5,
                "an end at {} against a side at {} is not an end",
                club.allegiance(Stand::HomeEnd),
                club.allegiance(Stand::Side)
            );
            // …and a kop is never a printed sheet: a quarter of any real one
            // is in a coat.
            assert!(club.allegiance(Stand::HomeEnd) < 0.8);
        }

        // Bigger club, more of its shirts — in both kinds of stand.
        for stand in [Stand::HomeEnd, Stand::Side] {
            assert!(
                great.allegiance(stand) > village.allegiance(stand),
                "the village club is as well supported as the great one"
            );
        }
    }

    /// …and that it actually reaches the mesh. The proportion is checked by
    /// counting the vertices that point at the club's block of the palette
    /// against the ones that point at the coats, which is the only place the
    /// decision is visible once the mesh is built.
    #[test]
    fn an_end_is_built_in_the_clubs_colours_and_a_side_is_not() {
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        let stature = Stature::of(&venue(30_000, 24_000, 9_000, false));
        // The sheet in order: 24 heads, 16 coats, 6 home, 6 away — 52 tiles.
        // So a `u` past the heads is somebody's clothing, past the coats is
        // somebody in the home club's colours, and past those is a visitor.
        let (clothing, home, away) = (24.0 / 52.0, 40.0 / 52.0, 46.0 / 52.0);

        let worn = |stand: Stand| {
            let mesh = Crowd::fill(&terrace(10), stature, stand, &palette, 5)
                .expect("a ten-step bank holds a crowd");
            let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
                mesh.attribute(Mesh::ATTRIBUTE_UV_0)
            else {
                panic!("the crowd carries no uvs");
            };
            // Clothing only. The drawn heads are in the sheet's top row and
            // the backs of hands are flat SKIN in the head columns of the
            // bottom row — both would dilute the count with vertices that were
            // never a coat. What is left is a torso, two sleeves, and the
            // trousers of whoever is on his feet, which are deliberately a
            // neutral coat tile whatever else he came in: nobody turns up in
            // the club's shorts in November.
            let clothes: Vec<f32> = uvs
                .iter()
                .filter(|uv| uv[1] > 0.5 && uv[0] > clothing)
                .map(|uv| uv[0])
                .collect();
            let share = |from: f32, to: f32| {
                clothes.iter().filter(|u| **u > from && **u < to).count() as f32
                    / clothes.len() as f32
            };
            (share(home, away), share(away, 1.0))
        };

        let (side_home, side_away) = worn(Stand::Side);
        let (home_end, home_end_away) = worn(Stand::HomeEnd);
        let (away_end_home, away_end_away) = worn(Stand::AwayEnd);

        // **Each end belongs to one support.** Behind one goal the home club,
        // behind the other the visitors, and neither gets into the other's.
        assert!(
            home_end > 0.35,
            "only {home_end} of the home end came dressed for it"
        );
        assert!(
            away_end_away > 0.35,
            "only {away_end_away} of the away end came dressed for it"
        );
        assert!(side_home < 0.2, "{side_home} of the main stand is in colours");

        assert_eq!(side_away, 0.0, "visitors got into the main stand");
        assert_eq!(home_end_away, 0.0, "visitors got into the home end");
        assert_eq!(away_end_home, 0.0, "the home club got into the away end");
    }

    /// A club nobody has ever counted a crowd for still gets one, at the
    /// utilisation its capacity was estimated through — the alternative is an
    /// empty ground, and no club draws nobody.
    #[test]
    fn an_uncounted_gate_falls_back_to_the_ground_it_is_in() {
        let uncounted = Stature::of(&venue(30_000, 0, 8_000, false));
        let counted = Stature::of(&venue(30_000, 24_600, 8_000, false));
        assert_eq!(uncounted.rows(34), counted.rows(34));
        assert!((uncounted.occupancy() - counted.occupancy()).abs() < 0.01);
    }

    /// A silent document is one written before there was a venue to describe,
    /// and it has to keep building the stadium the viewer always built.
    #[test]
    fn a_silent_document_still_gets_its_stadium() {
        let quiet = Stature::of(&VenueInfo::default());
        assert_eq!(quiet.rows(34), 34);
        assert!(quiet.occupancy() > 0.5, "{}", quiet.occupancy());
    }

    /// The steps have to be a solid flight: every slab reaching its own
    /// surface and covering the one below it. That is the arithmetic the crowd
    /// trusts when it sits down on it, and the concrete is poured off the
    /// same two lines.
    #[test]
    fn the_flight_of_steps_is_solid() {
        let terrace = terrace(12);
        for row in 0..terrace.rows {
            let top = terrace.slab_centre(row).y + terrace.riser * terrace.slab * 0.5;
            let surface = terrace.step(row).y;
            assert!(
                (top - surface).abs() < 1e-4,
                "step {row} has its surface at {surface} and the top of its slab at {top}"
            );
            if row > 0 {
                let under = terrace.slab_centre(row).y - terrace.riser * terrace.slab * 0.5;
                assert!(
                    under < terrace.step(row - 1).y,
                    "step {row} starts above the surface of the one below it"
                );
            }
        }
        assert!((terrace.crest() - terrace.step(terrace.rows - 1).y).abs() < 1e-4);
    }

    /// **A head is not a box.** Its outline has to narrow toward the crown and
    /// come in again at the jaw, because that silhouette is the whole of what
    /// reads as a head rather than as a brick — no amount of drawing on the
    /// front of a box would have done it.
    ///
    /// Checked on the built mesh rather than on the constant, since it is the
    /// lathe that has to carry it: a `SKULL` profile that was never read would
    /// leave the constant looking perfectly correct.
    #[test]
    fn a_spectators_head_is_turned_rather_than_boxed() {
        // A bank one place wide, so every point at a given depth belongs to
        // ONE man and his profile can be read straight off the buffer. Across
        // a full bank it cannot: the postures put heads at four different
        // heights, so a height band holds one man's jaw and his neighbour's
        // knee and the widest point in it is whoever happens to be furthest
        // along the row.
        let terrace = Terrace {
            length: 2.0,
            ..terrace(14)
        };
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        let mesh = Crowd::fill(&terrace, Stature::of(&VenueInfo::default()), Stand::Side, &palette, 3)
            .expect("a bank one place wide still holds a crowd");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("the crowd carries no positions");
        };

        // The first row with somebody SITTING UP in it. A man on his feet is a
        // metre taller than his seat, so the two are told apart by how far
        // above the step he reaches — and rows are told apart by depth, since
        // a standing figure spans two of them in height.
        //
        // Sitting UP and not forward on the edge of his seat, which is the
        // second filter: a man leaned twenty degrees carries the front of his
        // head seven centimetres lower than the back of it, so a height band
        // cuts across two rings at once and the profile stops being readable
        // off `y` alone. He is upright when his crown is over his own seat.
        let (step, his) = (0..terrace.rows)
            .find_map(|row| {
                let step = terrace.step(row);
                let at = step.x + terrace.tread * Crowd::SEATED_AT;
                let his: Vec<[f32; 3]> = points
                    .iter()
                    .copied()
                    .filter(|p| (p[2] - at).abs() < terrace.tread * 0.36)
                    .collect();
                let crown = his
                    .iter()
                    .copied()
                    .fold([0.0f32; 3], |top, p| if p[1] > top[1] { p } else { top });
                let up = crown[1] - step.y;
                (up > 0.80 && up < 1.10 && (crown[2] - at).abs() < 0.12).then_some((step, his))
            })
            .expect("fourteen rows of one place seat somebody sitting up");

        // How far out from his own middle the mesh reaches, in bands measured
        // as a share of his own height — which is what takes the ±12% every
        // figure is scaled by back out again.
        let top = his.iter().map(|p| p[1] - step.y).fold(0.0f32, f32::max);
        let middle = his.iter().map(|p| p[0]).sum::<f32>() / his.len() as f32;
        let width = |low: f32, high: f32| {
            his.iter()
                .filter(|p| ((low * top)..(high * top)).contains(&(p[1] - step.y)))
                .map(|p| (p[0] - middle).abs())
                .fold(0.0f32, f32::max)
        };

        // A box would give the same answer at every height it spans; a turned
        // head has to be widest at the cheekbones, narrower at the jaw under
        // them and taken well in at the crown. The bands are one ring each —
        // jaw at 0.77 of his height, cheek at 0.86, forehead at 0.92 and crown
        // at 0.98 — so a band wide enough to catch two of them would be
        // measuring whichever is wider rather than the one it names.
        let (jaw, cheek, crown) = (width(0.74, 0.81), width(0.83, 0.89), width(0.95, 1.0));
        assert!(
            cheek > jaw && cheek > crown,
            "the head is widest at {cheek}, against a jaw of {jaw} and a crown of {crown}"
        );
        assert!(
            crown < jaw,
            "the crown ({crown}) is no narrower than the jaw ({jaw})"
        );

        // …and the shoulders under it are wider than the head, which is the
        // other half of a silhouette that reads as a person. A box torso and a
        // box head gave two rectangles of nearly the same width stacked on one
        // another, which is a chess piece.
        let shoulder = width(0.52, 0.62);
        assert!(
            shoulder > cheek * 1.6,
            "his shoulders ({shoulder}) are barely wider than his head ({cheek})"
        );
    }

    /// **The support is massed behind the goal**, not spread flat across the
    /// end and out into the corners.
    ///
    /// An end bank is a hundred metres wide against a pitch of sixty-eight, so
    /// a third of it is not behind the goal at all. Painted evenly the colour
    /// bleeds round the whole bowl and the ground has no kop in it anywhere —
    /// which is a thing you only see from the halfway line, and never in a
    /// close-up.
    #[test]
    fn an_end_is_massed_behind_its_goal() {
        const BLOCKS: usize = 6;
        let terrace = Terrace {
            length: 100.0,
            ..terrace(14)
        };
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        let stature = Stature::of(&venue(30_000, 24_000, 9_000, false));
        let colours_from = 40.0 / 52.0;

        let mesh = Crowd::fill(&terrace, stature, Stand::HomeEnd, &palette, 9)
            .expect("a bank this size holds a crowd");
        let (
            Some(bevy::mesh::VertexAttributeValues::Float32x3(points)),
            Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)),
        ) = (
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            mesh.attribute(Mesh::ATTRIBUTE_UV_0),
        ) else {
            panic!("the crowd carries no positions or no uvs");
        };

        // What share of each block across the bank is in the club's colours.
        let mut census = [(0usize, 0usize); BLOCKS];
        for (point, uv) in points.iter().zip(uvs) {
            if uv[1] < 0.5 {
                continue; // a drawn head, not clothing
            }
            let block = ((point[0] / terrace.length + 0.5) * BLOCKS as f32) as usize;
            let block = &mut census[block.min(BLOCKS - 1)];
            block.1 += 1;
            if uv[0] > colours_from {
                block.0 += 1;
            }
        }
        let share = |block: (usize, usize)| block.0 as f32 / block.1 as f32;

        // The middle two blocks are behind the goal; the outer two are the
        // corners, past the flags and running into the touchline stands.
        let goal = (share(census[2]) + share(census[3])) * 0.5;
        let corners = (share(census[0]) + share(census[BLOCKS - 1])) * 0.5;
        assert!(
            goal > corners * 2.0,
            "the colour is not massed behind the goal: {goal} there against {corners} in the \
             corners"
        );
        // …and the corners are not bare. A kop thins, it does not stop.
        assert!(corners > 0.05, "the corners came out empty of colour: {corners}");
    }

    /// **A crowd arrives in knots, not as an even sprinkle.**
    ///
    /// Measured as the spread of density from block to block across a bank: an
    /// independent draw per place gives every block the same fraction of the
    /// ground's average, give or take sampling noise, and that flatness is
    /// exactly what reads as wrong. Some parts of a stand have to be visibly
    /// busier than others.
    ///
    /// Asserted against a BINOMIAL null rather than against zero, because a
    /// per-place draw is not perfectly flat either — with about 150 places to
    /// a block it wanders by a couple of per cent on its own, and a test that
    /// only checked for "some variation" would pass on the very thing this
    /// replaced.
    #[test]
    fn the_crowd_gathers_rather_than_spreading_evenly() {
        const ACROSS: usize = 6;
        const UP: usize = 3;
        let terrace = Terrace {
            length: 120.0,
            rows: 24,
            ..terrace(24)
        };
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        // Two thirds full, so there is room both to clump and to leave gaps.
        let stature = Stature::of(&venue(30_000, 20_000, 8_000, false));
        let mesh = Crowd::fill(&terrace, stature, Stand::Side, &palette, 11)
            .expect("a bank this size holds a crowd");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("the crowd carries no positions");
        };

        // People per patch of the bank, over a grid — across AND up the rows,
        // because an even spread has to be ruled out in both.
        //
        // ⚠ **Up the bank is read off DEPTH, not off height.** Vertices are
        // the only thing a mesh hands back and a figure carries roughly the
        // same number of them wherever he is, so counting vertices stands in
        // for counting people — but only if the patch a vertex falls in is the
        // patch its OWNER is sitting in. Bucketed by height, a man on his feet
        // puts his head two rows up the census from the step he is standing
        // on, and the top band fills with the shoulders of the row below it.
        // Depth has no such problem: every part of a spectator is within half
        // a tread of his own seat.
        let mut census = [[0usize; ACROSS]; UP];
        for point in points {
            let along = ((point[0] / terrace.length + 0.5) * ACROSS as f32) as usize;
            let back = (point[2] - terrace.from) / (terrace.tread * terrace.rows as f32);
            let up = (back * UP as f32) as usize;
            census[up.min(UP - 1)][along.min(ACROSS - 1)] += 1;
        }

        let patches: Vec<f32> = census.iter().flatten().map(|n| *n as f32).collect();
        let mean = patches.iter().sum::<f32>() / patches.len() as f32;
        let spread = (patches.iter().map(|n| (n - mean).powi(2)).sum::<f32>()
            / patches.len() as f32)
            .sqrt()
            / mean;

        // What the SAME census would come to if every place were drawn
        // independently — which is what this replaced, and what a lazy
        // "assert it varies at all" would happily pass. Binomial: the relative
        // spread of a patch of `places`, each taken with probability `p`.
        let run = terrace.length - Crowd::REACH * 2.0 * (1.0 + Crowd::SPREAD);
        let places = (run / Crowd::SPACING).floor() / ACROSS as f32 * terrace.rows as f32
            / UP as f32;
        let p = stature.occupancy();
        let null = ((1.0 - p) / (p * places)).sqrt();

        // Twice the noise floor, and it was 2.5 while the census was bucketed
        // by height. That half is not clumping and never was: a figure is 0.85
        // m tall on a 0.64 m riser, so its vertices straddle two height bands
        // and the census carried a whole extra term for where a man's
        // shoulders happened to fall. Reading the row off DEPTH takes it out —
        // the measured spread fell from 0.117 to 0.099 against an unchanged
        // density field — and what is left is the field alone.
        assert!(
            spread > null * 2.0,
            "the crowd is spread evenly: patches vary by {spread} against {null} for an \
             independent draw. {census:?}"
        );
        // …and not so much that a patch of the bank comes out bare. Empty
        // SEATS are wanted; an empty quarter of a stand is a hole.
        assert!(
            patches.iter().all(|count| *count > mean * 0.15),
            "a patch of the bank is all but empty: {census:?}"
        );
    }

    /// **What the whole ground costs.**
    ///
    /// The crowd is by a long way the largest mesh in this scene, and the one
    /// thing that would make it a bad trade is quietly letting it grow: this
    /// viewer's frame is spent per entity, but a browser still has to upload
    /// and transform these, and the buffers are built on the browser's only
    /// thread during the bring-up (see [`crate::app::bringup`]).
    ///
    /// Measured against the four banks as `pitch` actually lays them out at a
    /// great ground. The ceiling is roughly a quarter above where it sits, so
    /// a deliberate change to [`Crowd::SPACING`] or the figure shows up here
    /// as a decision rather than as a surprise.
    #[test]
    fn the_whole_crowd_fits_in_its_budget() {
        use crate::scene::field::Field;

        let full = Stature::of(&venue(62_000, 54_000, 9_800, false));
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        let along = Field::HALF_LENGTH + 4.6;
        let across = Field::HALF_WIDTH + 3.4;

        let mut vertices = 0;
        let mut deepest: f32 = 0.0;
        for (seed, (length, from, most, riser)) in [
            (along * 2.0 + full.overhang(6.0, 30.0), across + 2.1, 34, 0.72),
            (along * 2.0 + full.overhang(6.0, 30.0), across + 2.1, 34, 0.72),
            (across * 2.0 + full.overhang(4.0, 24.0), along + 2.4, 31, 0.70),
            (across * 2.0 + full.overhang(4.0, 24.0), along + 2.4, 31, 0.70),
        ]
        .into_iter()
        .enumerate()
        {
            let bank = Terrace {
                length,
                rows: full.rows(most),
                riser,
                tread: 0.95,
                from,
                slab: 1.9,
            };
            if seed < 2 {
                deepest = deepest.max(bank.step(bank.rows - 1).x + bank.tread);
            }
            let stand = if seed < 2 { Stand::Side } else { Stand::HomeEnd };
            vertices += Crowd::fill(&bank, full, stand, &palette, seed as u32 + 1)
                .expect("a full bank holds a crowd")
                .count_vertices();
        }

        // ⚠ **The touchline bank must not reach the gantry.** The broadcast
        // rest shot parks at `HALF_WIDTH + SETBACK` — 82 m from the centre
        // spot — and a rake that runs out past it puts the lens inside the
        // terracing. This is the constraint that sets `Pitch::TREAD`, and the
        // one a future "make the stands taller again" would break first.
        assert!(
            deepest < 78.0,
            "the touchline bank finishes {deepest} m out, against a gantry at 82"
        );

        // 2,641,720 vertices as this stands: twenty thousand people at about a
        // hundred and thirty vertices each, some hundred and twenty-five
        // megabytes of static geometry once the indices are counted, and four
        // submissions a frame to draw all of it. By a long way the largest
        // mesh in the scene, and still four entities — which is what the frame
        // is actually spent on.
        //
        // It is also the ceiling rather than the common case: this is the
        // largest gate in the database (73,170) and every row of it. Lokomotiv
        // Moscow comes to about nine tenths of it and a median club to a third,
        // because the rows come off the fixture.
        //
        // **It was 1,249,864, at fifty-six vertices a man**, and what the other
        // seventy bought was a person instead of a box with a smaller box on
        // top: a turned torso with sloping shoulders, a rounded head skinned
        // right round with his own face, two arms, and legs on the ones
        // standing up. Paid for twice over — once by the figure and once by
        // giving each of them more room ([`Crowd::SPACING`] went 0.62 to 0.70,
        // which is an eighth fewer people in every ground).
        //
        // ⚠ **Measured before it was believed.** Headless Chrome on an RTX
        // 3080 Ti at 1920x1080, the recipe `perf` documents: 2.4 ms a frame and
        // 417 fps with the whole ground built, against the 3.9 ms the same
        // instrument recorded before any of this. The frame is spent walking
        // and submitting ENTITIES and there are still four of these, so a
        // doubled vertex count costs nothing a frame — the whole of what it
        // costs is upload and the ~90 ms the buffers take to build on the
        // browser's one thread, which is a tenth of the shader compile that
        // frame is already waiting on.
        assert!(
            vertices < 2_900_000,
            "the crowd came to {vertices} vertices across the ground"
        );
        // …and it has to be a crowd rather than a token one. Half of this was
        // measured on screen as a teal wall with people scattered on it — see
        // the note on [`Crowd::SPACING`], and note that the floor is a count of
        // VERTICES and so guards the figure as well as the population: a
        // spectator whittled back to a pair of boxes would fall through it even
        // at the same spacing.
        assert!(
            vertices > 2_200_000,
            "the crowd came to only {vertices} vertices across the ground"
        );
    }

    /// Nobody stands in mid-air, and nobody stands on the pitch side of the
    /// front row. Every figure has to be on the tread of a step of the flight
    /// the concrete was poured off.
    #[test]
    fn every_spectator_is_sitting_on_a_step() {
        let terrace = terrace(9);
        let palette = CrowdPalette::of_swatches(24, 16, 6, 6);
        let mesh = Crowd::fill(&terrace, Stature::of(&VenueInfo::default()), Stand::HomeEnd, &palette, 7)
            .expect("a nine-step bank forty metres long holds a crowd");

        let surfaces: Vec<f32> = (0..terrace.rows).map(|row| terrace.step(row).y).collect();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("the crowd carries no positions");
        };

        assert!(!points.is_empty(), "the bank came back empty");
        for point in points {
            let foot = surfaces
                .iter()
                .copied()
                .filter(|surface| *surface <= point[1] + 1e-3)
                .fold(f32::MIN, f32::max);
            assert!(
                foot > f32::MIN,
                "somebody is below the first step at {}",
                point[1]
            );
            // Nobody towers over his own step either. A man on his feet is
            // 1.70 m of him, a big one is 1.90, and a big one with both arms
            // up reaches 2.14 — which is what a person with his arms up
            // reaches, and the most anything in this crowd is allowed to be.
            assert!(
                point[1] - foot < 2.25,
                "somebody is {} m above the step under him",
                point[1] - foot
            );
            assert!(
                point[2] >= terrace.from,
                "somebody is sitting {} m in front of the terrace",
                terrace.from - point[2]
            );
            assert!(
                point[0].abs() <= terrace.length * 0.5,
                "somebody is {} m past the end of the bank",
                point[0].abs() - terrace.length * 0.5
            );
        }
    }
}
