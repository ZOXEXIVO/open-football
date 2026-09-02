use crate::app::bill::{Held, MemoryBill};
use crate::app::config::{PlayerInfo, TeamColors, ViewerConfig};
use crate::art::textures::{Beard, FaceLook, Textures};
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use shared::Palette;

/// One square of the wardrobe sheet: which cloth (or skin) a run of a merged
/// mesh's triangles is painted from.
///
/// # Why a sheet exists at all
///
/// The viewer's frame is spent per ENTITY, and half the entities on a
/// footballer were separate for exactly one reason: they wore a different
/// material than the part they are welded to — a trim cuff on a shirt sleeve
/// on a skin arm, a shorts leg on a thigh. A mesh takes one material, so
/// merging those parts means one material that can answer with several
/// colours, and the smallest device that does that is a strip of texels with
/// every vertex of a part pointed at its own texel.
///
/// Every vertex of a region maps to the CENTRE of its texel and every
/// triangle therefore interpolates a constant, so no filtering, mip or
/// atlas-bleed caveat applies — the sheet behaves exactly like the flat
/// colours it replaces. Roughness rides the same slots in a second strip
/// (see [`Wardrobe::roughness_sheet`]), so the cloth keeps its matte and the
/// skin its faint sheen inside one material.
#[derive(Clone, Copy)]
pub enum Swatch {
    Skin,
    Shirt,
    Trim,
    Shorts,
    Socks,
}

impl Swatch {
    /// How many texels wide a sheet is. Eight rather than the five in use, so
    /// the layout has room without every sheet in memory changing shape.
    pub const SPAN: usize = 8;

    fn slot(self) -> usize {
        match self {
            Swatch::Skin => 0,
            Swatch::Shirt => 1,
            Swatch::Trim => 2,
            Swatch::Shorts => 3,
            Swatch::Socks => 4,
        }
    }

    /// The UV every vertex of a region carries: the centre of its texel.
    pub fn uv(self) -> [f32; 2] {
        [(self.slot() as f32 + 0.5) / Self::SPAN as f32, 0.5]
    }

    /// How rough each slot's surface is — the numbers the separate materials
    /// used to carry, moved into texels. See [`Wardrobe::cloth`] for why
    /// cloth sits high and [`Wardrobe::flesh`] for why skin does not.
    const ROUGHNESS: [f32; Self::SPAN] = [0.62, 0.80, 0.78, 0.82, 0.88, 0.82, 0.82, 0.82];
}

/// The colours one side takes the field in.
struct Strip {
    shirt: Color,
    shorts: Color,
    socks: Color,
    /// Ink for the printed number: whichever of black or white the shirt can
    /// actually carry.
    print: Color,
    /// Collar and cuffs. Every kit ever made has trim on the neck and the
    /// sleeves, and it is the detail that separates a football shirt from a
    /// coloured shape at any range a face is legible from.
    trim: Color,
}

impl Strip {
    /// A club's own colours: the shirt as registered, and shorts in the club's
    /// contrasting colour — the one its badge and lettering use.
    fn outfield(colors: &TeamColors, fallback: Color) -> Self {
        let shirt = colors.background_color(fallback);
        let contrast = colors.foreground_color(Color::WHITE);
        // A club whose two colours sit close together (claret on red, say) would
        // otherwise field a player in one flat silhouette.
        let shorts = if Self::separation(shirt, contrast) > 0.20 {
            contrast
        } else if Self::luminance(shirt) > 0.42 {
            Wardrobe::DARK
        } else {
            Wardrobe::LIGHT
        };
        let print = Self::print_for(shirt);
        Strip {
            shirt,
            shorts,
            // Socks in the shirt colour: down among twenty-two pairs of legs it
            // is the last place the eye can still pick a side out.
            socks: shirt,
            print,
            // The club's second colour if it can be seen against the first at
            // the width of a collar, which is a harder test than the shorts
            // have to pass — a band two centimetres wide either contrasts or
            // is not there at all.
            trim: if Self::separation(shirt, contrast) > 0.35 {
                contrast
            } else {
                print
            },
        }
    }

    fn print_for(shirt: Color) -> Color {
        if Self::luminance(shirt) > 0.42 {
            Wardrobe::DARK
        } else {
            Wardrobe::LIGHT
        }
    }

    /// A keeper is the one player who has to be told apart from everybody on
    /// the pitch, so their colours come from neither club.
    fn keeper(shirt: Color) -> Self {
        Strip {
            shirt,
            shorts: Wardrobe::DARK,
            socks: Wardrobe::DARK,
            print: Self::print_for(shirt),
            trim: Self::print_for(shirt),
        }
    }

    fn luminance(color: Color) -> f32 {
        let rgb = color.to_srgba();
        0.2126 * rgb.red + 0.7152 * rgb.green + 0.0722 * rgb.blue
    }

    fn separation(first: Color, second: Color) -> f32 {
        let (first, second) = (first.to_srgba(), second.to_srgba());
        (first.red - second.red).abs()
            + (first.green - second.green).abs()
            + (first.blue - second.blue).abs()
    }
}

/// How a footballer wears his hair. One mesh each, bar the first — a shaved
/// head is the scalp itself, with the stubble drawn onto the face texture.
///
/// Written down as a type rather than an index because the mesh table and the
/// face texture both have to be told, and they disagree about what "no hair"
/// means: one wants nothing hung on the head and the other wants a wash of
/// colour over the top of it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HairStyle {
    Shaved,
    Crop,
    Short,
    Mop,
}

impl HairStyle {
    pub fn index(self) -> usize {
        match self {
            HairStyle::Shaved => 0,
            HairStyle::Crop => 1,
            HairStyle::Short => 2,
            HairStyle::Mop => 3,
        }
    }
}

/// The parts of a footballer's appearance that have nothing to do with the
/// club: height, skin, hair, face and boots.
///
/// The ones that carry no meaning — how he is built, how he runs, what
/// colour his boots are — are cut from the player's id, so a squad reads as
/// eleven individuals and looks the same every time the match is replayed.
///
/// COLOURING is not one of those. Skin, hair and eyes arrive already decided
/// on [`PlayerInfo`], because they mean something: they follow the man's
/// nationality, and the country table that says so lives on the server. A
/// hash was picking them here, which is why a Nigerian back four used to take
/// the field in four unrelated complexions, none of them the one on the
/// player's own profile page.
pub struct Complexion;

impl Complexion {
    const BOOTS: [Color; 4] = [
        Color::srgb(0.06, 0.06, 0.07),
        Color::srgb(0.92, 0.93, 0.95),
        Color::srgb(0.86, 0.15, 0.24),
        Color::srgb(0.55, 0.90, 0.24),
    ];

    fn boots(id: u32) -> usize {
        ((Self::hash(id) >> 16) % Self::BOOTS.len() as u32) as usize
    }

    /// Where this player sits on each of the shared ramps.
    ///
    /// Clamped rather than trusted. The number came over the wire, and an
    /// index past the end of the table would take the whole match down on the
    /// frame the wardrobe was built.
    pub fn skin(player: &PlayerInfo) -> usize {
        (player.skin as usize).min(Palette::SKIN.len() - 1)
    }

    pub fn hair(player: &PlayerInfo) -> usize {
        (player.hair as usize).min(Palette::HAIR.len() - 1)
    }

    pub fn eyes(player: &PlayerInfo) -> usize {
        (player.eyes as usize).min(Palette::EYES.len() - 1)
    }

    /// The entry of the shared skin ramp nearest a colour read off a real
    /// picture of the player.
    ///
    /// The ramps are what his neck, his arms and the cap on his head are
    /// painted in. When a photograph turns up, the nationality-drawn entry is
    /// a guess and the picture is the answer — so the guess is replaced by
    /// whichever entry of the same ramp sits closest to what the picture
    /// says, which keeps every player on the shared materials the renderer
    /// batches by. See [`crate::players::portrait::Portraits::attach`].
    pub fn nearest_skin(tone: Vec3) -> usize {
        Self::nearest(&Palette::SKIN, tone)
    }

    fn nearest(ramp: &[&str], tone: Vec3) -> usize {
        ramp.iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                let distance = |hex: &str| {
                    let entry = Self::tone(hex).to_srgba();
                    Vec3::new(entry.red, entry.green, entry.blue).distance_squared(tone)
                };
                distance(left)
                    .partial_cmp(&distance(right))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    /// One entry of a shared table as a colour. They are written as `#rrggbb`
    /// because the other renderer puts them straight into an SVG attribute;
    /// this side parses each of them once, when the wardrobe is built.
    fn tone(hex: &str) -> Color {
        Srgba::hex(hex)
            .map(Color::from)
            // Unreachable with a table that parses, which the test at the
            // bottom of this file is there to keep true.
            .unwrap_or(Color::srgb(0.85, 0.66, 0.51))
    }

    /// Everything about a player's face that is his and not his club's.
    ///
    /// The colours come off the team sheet; the rest is drawn off SALTED
    /// hashes rather than off further bits of one hash: it has thirty-two
    /// bits and several traits already spoken for, and two features cut from
    /// overlapping bits are correlated — every blond would end up with the
    /// same beard.
    pub fn face(player: &PlayerInfo) -> FaceLook {
        let id = player.id;
        FaceLook {
            skin: Self::tone(Palette::SKIN[Self::skin(player)]),
            hair: Self::tone(Palette::HAIR[Self::hair(player)]),
            eyes: Self::tone(Palette::EYES[Self::eyes(player)]),
            // Brows carry as much of a face's expression as the eyes under
            // them, and they are the feature that survives longest as the head
            // minifies — so this is the one that most needs to vary.
            brow: 0.55 + Self::trait_of(id, 0x9E37) as f32 / 222.0,
            beard: match Self::trait_of(id, 0xB5C1) {
                0..46 => Beard::Clean,
                46..72 => Beard::Stubble,
                72..84 => Beard::Goatee,
                _ => Beard::Full,
            },
            shaved: Self::hair_style(id) == HairStyle::Shaved,
        }
    }

    pub fn hair_style(id: u32) -> HairStyle {
        match Self::trait_of(id, 0x2545) {
            0..12 => HairStyle::Shaved,
            12..50 => HairStyle::Crop,
            50..82 => HairStyle::Short,
            _ => HairStyle::Mop,
        }
    }

    /// A 0..99 draw for one trait of one player, independent of every other.
    fn trait_of(id: u32, salt: u32) -> u32 {
        Self::hash(id ^ salt) % 100
    }

    /// **How long a stride he takes**, as a multiplier on the distance the
    /// run cycle covers per step.
    ///
    /// The single biggest reason a squad reads as one animation played
    /// twenty-two times: `Actors::STRIDE` was a global, so every player on
    /// the pitch had the same cadence at the same speed — and cadence is
    /// what the eye actually picks a runner out by, far more than his
    /// height or his colour. A loping centre-half and a scurrying winger
    /// covering the same ground at the same pace were drawn taking the same
    /// number of steps to do it.
    ///
    /// Blended with `height`, because leg length really does set most of
    /// it, plus an independent salt for the rest — two men of a height do
    /// not run alike. Roughly 0.86 to 1.16.
    pub fn stride(id: u32) -> f32 {
        let legs = (Self::height(id) - 1.008) * 1.30;
        let own = Self::trait_of(id, 0x7F4A) as f32 / 100.0 - 0.5;
        (1.0 + legs + own * 0.16).clamp(0.84, 1.20)
    }

    /// How much he BOUNCES doing it — the amplitude of the whole run cycle,
    /// hips, knees and arms together.
    ///
    /// Some players run low and economical, some pick their knees up. It is
    /// deliberately a separate hash from [`Self::stride`]: a short choppy
    /// stride with a high knee is a real runner and so is its opposite, and
    /// cutting both from the same bits would only ever produce two kinds of
    /// player instead of four.
    pub fn spring(id: u32) -> f32 {
        0.86 + Self::trait_of(id, 0xC13B) as f32 / 360.0
    }

    /// **How bent he runs at the elbow**, −1 nearly straight to +1 folded
    /// tight — and **how far forward he carries himself**, on the same
    /// scale.
    ///
    /// Both used to be [`Self::carriage`], which also sets how wide he holds
    /// his arms, which of them does more and where in the cycle he starts:
    /// nine things off one hash, so the squad varied along a single axis and
    /// a man who ran with his arms wide always also ran bent and leaning.
    /// That is exactly the argument [`Self::spring`] makes against cutting
    /// the stride from the carriage, and it applies to every one of them —
    /// one hash gives two kinds of runner and four independent hashes give
    /// sixteen.
    pub fn elbows(id: u32) -> f32 {
        Self::trait_of(id, 0x51C7) as f32 / 50.0 - 1.0
    }

    pub fn lean(id: u32) -> f32 {
        Self::trait_of(id, 0x2E8D) as f32 / 50.0 - 1.0
    }

    /// **How far his feet point out when he runs**, in radians.
    ///
    /// Skewed rather than centred: nobody runs pigeon-toed to speak of and
    /// plenty of footballers run at twenty-five degrees, so the draw is
    /// mostly positive with a small tail the other way. The rest of the
    /// squad's spread is amplitudes of one cycle; this is a shape, and it is
    /// on the part of him nearest the camera.
    pub fn toes(id: u32) -> f32 {
        Self::trait_of(id, 0x9B04) as f32 / 380.0 - 0.03
    }

    /// …and how fast he ticks over standing still — breathing, shifting his
    /// weight. Twenty-two men breathing in unison is its own kind of robot.
    pub fn tempo(id: u32) -> f32 {
        0.80 + Self::trait_of(id, 0x3D9E) as f32 / 250.0
    }

    /// **How he takes a goal**, as a 0..99 draw — hands on his head, on his
    /// hips, bent over his knees, or arms hanging.
    ///
    /// Its own salt rather than a sign off [`Self::carriage`], which is what
    /// it used to be. Carriage already drives how wide he holds his arms,
    /// how bent his elbows are and where in the run cycle he starts, so a
    /// reaction cut from it is not an independent fact about the man: every
    /// player who runs with his arms wide reacts to conceding the same way,
    /// and the correlation is exactly what
    /// `no_two_players_run_alike` exists to keep out of the squad.
    pub fn reaction(id: u32) -> u32 {
        Self::trait_of(id, 0x6A11)
    }

    /// Multiplier on the model's height. Spans roughly 1.70 m to 1.92 m
    /// against the 1.79 m base — the real range of a senior squad, from a
    /// pocket winger to a centre-half.
    ///
    /// Was ±4%, which is 1.72-1.86 and far too tight to tell anyone apart
    /// at broadcast camera distance.
    pub fn height(id: u32) -> f32 {
        0.950 + ((Self::hash(id) >> 20) % 116) as f32 / 1000.0
    }

    /// Multiplier on the model's GIRTH — width and depth, independent of
    /// height.
    ///
    /// Every player used to be one mesh under a uniform scale, so the whole
    /// squad had an identical build and differed only in size and colour.
    /// Two footballers of the same height are not the same shape: one is a
    /// whippet and one is built like a bouncer, and applied to x and z only
    /// this separates them without a second mesh. It widens the shoulders
    /// and the hips and thickens every limb together, which is exactly what
    /// build means.
    ///
    /// Drawn from the top bits of the hash so it is independent of height —
    /// tall players are not systematically broad.
    pub fn build(id: u32) -> f32 {
        0.930 + ((Self::hash(id) >> 26) % 64) as f32 / 440.0
    }

    /// How a player carries himself, −1..1 and fixed for the match.
    ///
    /// Feeds arm width, elbow flex and forward lean, and offsets where in
    /// the run cycle he starts. Without it all twenty-two run one identical
    /// animation in phase with each other, which no amount of modelling
    /// will stop looking mechanical.
    pub fn carriage(id: u32) -> f32 {
        ((Self::hash(id) >> 12) % 200) as f32 / 100.0 - 1.0
    }

    /// Which foot he kicks with, −1 left and +1 right.
    ///
    /// Only ever consulted for a player striking a ball from a standstill —
    /// one on the move swings whichever leg was coming through anyway. About
    /// one in four is left-footed, which is roughly the real proportion.
    pub fn footedness(id: u32) -> f32 {
        if (Self::hash(id) >> 4) % 4 == 0 {
            -1.0
        } else {
            1.0
        }
    }

    /// Consecutive player ids have to land on unrelated appearances, and squad
    /// lists number their players consecutively.
    fn hash(id: u32) -> u32 {
        let mut hash = id.wrapping_mul(2_654_435_761);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^ (hash >> 13)
    }
}

/// The materials one player wears. Handles only — most of them are shared
/// with the rest of the squad.
#[derive(Clone)]
pub struct Outfit {
    /// Every stitch of the strip that is nobody's skin — shirt with its
    /// collar, shorts, socks with their turnover — as ONE sheeted material
    /// per strip. The parts that used to wear shirt, trim, shorts and socks
    /// separately are merged meshes now, each region pointed at its
    /// [`Swatch`].
    pub kit: Handle<StandardMaterial>,
    /// The same sheet with this player's own complexion in the skin slot,
    /// for the merged parts that show both cloth and the man inside it — a
    /// sleeved arm, a thigh with the leg of the shorts on it. Shared by
    /// everybody of his strip and tone.
    pub limb: Handle<StandardMaterial>,
    /// Which of the four strips he is wearing, kept so the portrait repaint
    /// can ask [`Wardrobe::limb_sheet`] for the same strip in the
    /// photographed tone — see [`crate::players::body::DressedFlesh`].
    pub strip: usize,
    pub boots: Handle<StandardMaterial>,
    pub skin: Handle<StandardMaterial>,
    /// The head, which is his own: his complexion until his picture arrives
    /// and lands on this very material — see [`crate::players::portrait`].
    pub face: Handle<StandardMaterial>,
    pub hands: Handle<StandardMaterial>,
    pub hair: Handle<StandardMaterial>,
    pub hair_style: HairStyle,
    /// `None` for a player the team sheet gave no shirt number, and for one
    /// whose name has nothing the shirt printer can set.
    pub number: Option<Handle<StandardMaterial>>,
    pub name: Option<Handle<StandardMaterial>>,
    /// The same name as a walk-out plate: inverted, and so a material of its
    /// own rather than the one above. `None` on the same terms as `name`.
    pub name_front: Option<Handle<StandardMaterial>>,
}

/// Every material the twenty-two players need, built once.
///
/// Sharing them is what keeps a pitch full of footballers down to a couple of
/// dozen draw calls: the renderer batches by mesh and material, and there are
/// only ever four strips and a handful of appearances on the field.
///
/// A resource, and kept for the whole match rather than dropped at the end of
/// the spawn: a substitute is dressed on the way onto the pitch and not before
/// — see [`crate::players::actors::Actors::take_the_field`] — so this has
/// to still be here in the sixtieth minute.
#[derive(Resource)]
pub struct Wardrobe {
    /// One sheeted material per strip — see [`Swatch`] for what a sheet is.
    /// This one paints the four cloths and is shared by a whole side, so the
    /// merged torso and legs batch exactly as widely as the old flat kit
    /// materials did.
    kit_sheets: [Handle<StandardMaterial>; 4],
    /// The same sheets with a real complexion in the skin slot, one per
    /// (strip, tone) pairing that has actually taken the field. Filled as men
    /// are dressed, like the faces below, because 4 strips by 12 tones is a
    /// wardrobe of forty-eight and a match wears a handful of them.
    limb_sheets: Vec<((usize, usize), Handle<StandardMaterial>)>,
    /// The colours each strip's sheet is painted from — shirt, trim, shorts,
    /// socks — kept because a limb sheet is painted on demand, long after the
    /// `Strip`s themselves are gone.
    wear: [[Color; 4]; 4],
    /// One roughness strip for every sheet in the wardrobe: the slots are
    /// the same everywhere, so the surface qualities are too.
    roughness_sheet: Handle<Image>,
    skin: Vec<Handle<StandardMaterial>>,
    hair: Vec<Handle<StandardMaterial>>,
    boots: Vec<Handle<StandardMaterial>>,
    gloves: Handle<StandardMaterial>,
    shadow: Handle<StandardMaterial>,
    /// The ink each of the four strips prints in, kept because the prints are
    /// no longer all made at once and the strips they come off are otherwise
    /// long gone by the time a substitute needs one.
    prints: [Color; 4],
    /// …and the cloth each prints ON, which the walk-out plate needs as the
    /// colour of its LETTERING — the plate is the print colour and the name is
    /// knocked out of it. See
    /// [`Textures::name_plate`](crate::art::textures::Textures::name_plate).
    cloths: [Color; 4],
    /// The three materials that cannot be shared, because what makes them
    /// differ is baked into a texture: a printed number, a printed name and a
    /// face.
    ///
    /// **Filled as men take the field, not up front.** The page sends both full
    /// team sheets — eleven and seven a side, thirty-six men — and the
    /// fourteen on the benches are three pictures each that nobody may ever
    /// see. A face is the expensive one: a 256-square sheet painted texel by
    /// texel through [`crate::art::textures::Painter`] and then mip-chained, on
    /// the browser's main thread, before the first frame. Fourteen of those is
    /// most of a second of the load spent on men who are sitting down.
    numbers: Vec<(u32, Handle<StandardMaterial>)>,
    names: Vec<(u32, Handle<StandardMaterial>)>,
    plates: Vec<(u32, Handle<StandardMaterial>)>,
    faces: Vec<(u32, Handle<StandardMaterial>)>,
}

impl Wardrobe {
    const LIGHT: Color = Color::srgb(0.91, 0.92, 0.94);
    const DARK: Color = Color::srgb(0.10, 0.11, 0.15);
    /// Keeper strips, home then away. Neither belongs to a club: a keeper has
    /// to be told apart from twenty outfielders, from the other keeper, and
    /// from the grass they are standing on.
    ///
    /// Both yellow, which is the goalkeeper colour — the away keeper used to
    /// be magenta. They are two SHADES of it rather than one, because the
    /// second constraint above is real: with the lens pulled back, both
    /// penalty areas can be in frame at once. Bright yellow and a
    /// green-tinted one read as the same colour from the stand and stay
    /// separable side by side, which is exactly what kit rules do with a
    /// keeper clash.
    ///
    /// The green-yellow is deliberately the AWAY one: it is the weaker of the
    /// two against grass, and the away keeper is the one at the far end.
    const KEEPERS: [Color; 2] = [Color::srgb(0.98, 0.84, 0.10), Color::srgb(0.80, 0.88, 0.18)];
    const GLOVES: Color = Color::srgb(0.88, 0.90, 0.94);
    const HOME_FALLBACK: Color = Color::srgb(0.0, 0.19, 0.49);
    const AWAY_FALLBACK: Color = Color::srgb(0.70, 0.25, 0.0);

    pub fn new(
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        config: &ViewerConfig,
    ) -> Self {
        // One material per entry of the shared ramps rather than per player:
        // twelve tones and ten hair colours cover any twenty-two men who
        // could take the field, and the renderer batches by material.
        let skin: Vec<Handle<StandardMaterial>> = Palette::SKIN
            .iter()
            .map(|hex| Self::flesh(materials, Complexion::tone(hex)))
            .collect();
        let hair: Vec<Handle<StandardMaterial>> = Palette::HAIR
            .iter()
            .map(|hex| Self::cloth(materials, Complexion::tone(hex), 0.95))
            .collect();
        let boots: Vec<Handle<StandardMaterial>> = Complexion::BOOTS
            .iter()
            .map(|color| Self::moulded(materials, *color, 0.35))
            .collect();
        let gloves = Self::moulded(materials, Self::GLOVES, 0.7);

        // One kit per (side, keeper) pairing: the only four strips that can
        // take the field.
        let strips = [
            Strip::outfield(&config.home, Self::HOME_FALLBACK),
            Strip::outfield(&config.away, Self::AWAY_FALLBACK),
            Strip::keeper(Self::KEEPERS[0]),
            Strip::keeper(Self::KEEPERS[1]),
        ];
        // The cloth roughnesses the old flat materials carried — jersey 0.80,
        // trim 0.78, shorts 0.82, socks roughest of all at 0.88 — now live in
        // [`Swatch::ROUGHNESS`] and ride every sheet as texels, so a merged
        // shirt keeps reading as knit and a sock as terry inside one
        // material. See [`Self::cloth`] for why they all sit that high.
        let wear =
            [0, 1, 2, 3].map(|index| {
                [
                    strips[index].shirt,
                    strips[index].trim,
                    strips[index].shorts,
                    strips[index].socks,
                ]
            });
        let roughness_sheet = images.add(Self::roughness_image());
        // The kit sheets carry a mid-table complexion in their skin slot. No
        // cloth-only merge ever points a vertex there — it exists so the slot
        // is never uninitialised, and a keeper's sleeve-buried arm painted
        // from it by mistake would come out a plausible arm rather than a
        // magenta one.
        let neutral = Complexion::tone(Palette::SKIN[Palette::SKIN.len() / 2]);
        let kit_sheets = [0, 1, 2, 3].map(|index| {
            let sheet = images.add(Self::sheet_image(neutral, &wear[index]));
            // Cloth end to end, so the sheet keeps the cloth's own dead
            // specular — see [`Self::cloth`] for the number.
            Self::sheeted(materials, sheet, roughness_sheet.clone(), 0.18)
        });

        let blob = Textures::blob(images);
        Wardrobe {
            prints: [0, 1, 2, 3].map(|index| strips[index].print),
            cloths: [0, 1, 2, 3].map(|index| strips[index].shirt),
            kit_sheets,
            limb_sheets: Vec::new(),
            wear,
            roughness_sheet,
            // Empty. Every one of these is painted the first time the man it
            // belongs to is dressed — see the note on the fields.
            numbers: Vec::new(),
            names: Vec::new(),
            plates: Vec::new(),
            faces: Vec::new(),
            skin,
            hair,
            boots,
            gloves,
            shadow: materials.add(StandardMaterial {
                base_color: Color::srgba(0.0, 0.0, 0.0, 0.40),
                base_color_texture: Some(blob),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            }),
        }
    }

    /// The shared skin ramp itself, one material per entry. Handed over for
    /// the same reason: a photograph moves a player from one entry to another,
    /// and moving him means giving his parts a handle out of these — not
    /// building him a material of his own, which would take him out of the
    /// batch every other player on that tone is drawn in.
    pub fn complexions(&self) -> Vec<Handle<StandardMaterial>> {
        self.skin.clone()
    }

    /// The sheet for a merged limb — this strip's cloth with this complexion
    /// in the skin slot — painted the first time anybody wears the pairing
    /// and handed back whole every time after, so a substitute in a common
    /// tone lands in the batch his team-mates are already drawn in.
    ///
    /// Public alongside [`Self::complexions`] and on the same argument: a
    /// photograph re-tones a player after the fact, and the repaint has to
    /// come back HERE for the sheet in the new tone rather than minting a
    /// material of his own. See
    /// [`Portraits::attach`](crate::players::portrait::Portraits::attach).
    pub fn limb_sheet(
        &mut self,
        strip: usize,
        tone: usize,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) -> Handle<StandardMaterial> {
        let strip = strip.min(self.wear.len() - 1);
        let tone = tone.min(Palette::SKIN.len() - 1);
        if let Some((_, sheet)) = self
            .limb_sheets
            .iter()
            .find(|(pairing, _)| *pairing == (strip, tone))
        {
            return sheet.clone();
        }
        let sheet = images.add(Self::sheet_image(
            Complexion::tone(Palette::SKIN[tone]),
            &self.wear[strip],
        ));
        // The skin's reflectance rather than the cloth's, because the skin is
        // the half of these merges that cannot afford to lose it: a trace of
        // specular is the difference between an arm and a painted stick (see
        // [`Self::flesh`]). The cloth regions carry a fourfold-of-nearly-
        // nothing extra — at their roughness the lobe is spread too wide to
        // read as sheen, where the arm's loss would read at once.
        let material = Self::sheeted(materials, sheet, self.roughness_sheet.clone(), 0.35);
        self.limb_sheets.push(((strip, tone), material.clone()));
        material
    }

    /// One [`Swatch`] strip as pixels: the complexion, then the four cloths,
    /// then padding in the last cloth so a stray UV lands on kit rather than
    /// on black.
    fn sheet_image(skin: Color, wear: &[Color; 4]) -> Image {
        let mut texels = Vec::with_capacity(Swatch::SPAN * 4);
        let slots = [skin, wear[0], wear[1], wear[2], wear[3]];
        for slot in 0..Swatch::SPAN {
            let colour = slots.get(slot).unwrap_or(&wear[2]).to_srgba();
            texels.extend_from_slice(&[
                (colour.red * 255.0).round() as u8,
                (colour.green * 255.0).round() as u8,
                (colour.blue * 255.0).round() as u8,
                255,
            ]);
        }
        Image::new(
            Extent3d {
                width: Swatch::SPAN as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            texels,
            // The colours are authored in sRGB exactly as the flat materials'
            // `base_color`s were, so the sampled texel and the old uniform
            // decode to the same number.
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    /// And the matching surface strip: roughness in the green channel, where
    /// the standard material reads it, metal in the blue, which nothing on a
    /// footballer is. Linear, not sRGB — these are numbers, not colours.
    fn roughness_image() -> Image {
        let mut texels = Vec::with_capacity(Swatch::SPAN * 4);
        for slot in 0..Swatch::SPAN {
            texels.extend_from_slice(&[
                0,
                (Swatch::ROUGHNESS[slot] * 255.0).round() as u8,
                0,
                255,
            ]);
        }
        Image::new(
            Extent3d {
                width: Swatch::SPAN as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            texels,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    /// A sheet as a material. White base and unit roughness, because the
    /// texels are the values: the standard material multiplies both straight
    /// through, so the identity is what keeps the swatches meaning what they
    /// say.
    fn sheeted(
        materials: &mut Assets<StandardMaterial>,
        sheet: Handle<Image>,
        roughness: Handle<Image>,
        reflectance: f32,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(sheet),
            perceptual_roughness: 1.0,
            metallic_roughness_texture: Some(roughness),
            metallic: 0.0,
            reflectance,
            ..default()
        })
    }

    fn strip_index(player: &PlayerInfo) -> usize {
        match (player.is_goalkeeper(), player.is_home) {
            (false, true) => 0,
            (false, false) => 1,
            (true, true) => 2,
            (true, false) => 3,
        }
    }

    /// What this player is wearing. The strip comes off the team sheet, the
    /// rest from who they are.
    ///
    /// Takes the asset stores because the three materials that are his alone
    /// are painted HERE, on the first call for him, rather than up front for a
    /// squad of thirty-six — see the note on those fields. Every call after
    /// the first hands back what the first one made, so a player who goes off
    /// and comes back on is not repainted and, more to the point, does not
    /// leave the batch his old material was in.
    pub fn outfit(
        &mut self,
        player: &PlayerInfo,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) -> Outfit {
        self.paint(player, materials, images);

        let strip = Self::strip_index(player);
        let tone = Complexion::skin(player);
        let skin = self.skin[tone].clone();
        let limb = self.limb_sheet(strip, tone, materials, images);
        let own = |table: &Vec<(u32, Handle<StandardMaterial>)>| {
            table
                .iter()
                .find(|(id, _)| *id == player.id)
                .map(|(_, material)| material.clone())
        };
        Outfit {
            kit: self.kit_sheets[strip].clone(),
            limb,
            strip,
            boots: self.boots[Complexion::boots(player.id)].clone(),
            hands: if player.is_goalkeeper() {
                self.gloves.clone()
            } else {
                skin.clone()
            },
            // Painted by `paint` above, so the fallback is unreachable — and
            // it is the shared ramp entry rather than a material of his own,
            // which is the one thing a face must never be: a picture folded
            // into it would go onto every player wearing that complexion.
            face: own(&self.faces).unwrap_or_else(|| skin.clone()),
            skin,
            hair: self.hair[Complexion::hair(player)].clone(),
            hair_style: Complexion::hair_style(player.id),
            number: own(&self.numbers),
            name: own(&self.names),
            name_front: own(&self.plates),
        }
    }

    /// Paints this player's number, name and plate, and gives him the material
    /// his face will arrive on, once.
    ///
    /// Split out of [`Self::outfit`] because it is the only part of dressing a
    /// man that is expensive, and because the early return is the whole
    /// contract: called twice for the same player it must do nothing the
    /// second time, or he changes material and drops out of every batch he
    /// was sharing.
    fn paint(
        &mut self,
        player: &PlayerInfo,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) {
        if self.faces.iter().any(|(id, _)| *id == player.id) {
            return;
        }
        // Everything below is his alone rather than shared with the squad, so
        // it goes on the per-player half of the bill beside his face — see
        // [`MemoryBill::charge`]. Uncharged it would file a team sheet's worth
        // of numbers and names under the stadium's scenery, and the two grow
        // for entirely different reasons.
        let _charge = MemoryBill::charge(Held::Faces);
        let ink = self.prints[Self::strip_index(player)];

        if player.shirt_number > 0 {
            let texture = Textures::number(images, player.shirt_number);
            let printed = Self::printed(materials, ink, texture);
            self.numbers.push((player.id, printed));
        }
        if let Some(texture) = Textures::name(images, &player.last_name) {
            let printed = Self::printed(materials, ink, texture);
            self.names.push((player.id, printed));
        }
        // The walk-out plate, whose two colours are the strip's the other way
        // round. Painted alongside the back print rather than when the ceremony
        // starts: the ceremony IS the first thing in the recording, so putting
        // it off would only move the cost into the frame the camera is already
        // panning down the line in.
        let cloth = self.cloths[Self::strip_index(player)];
        if let Some(texture) = Textures::name_plate(images, &player.last_name, ink, cloth) {
            let plated = Self::plated(materials, texture);
            self.plates.push((player.id, plated));
        }
        // His head: a material of his own, in his own complexion, with nothing
        // painted on the front of it yet.
        //
        // **A face on this pitch is a PHOTOGRAPH**, and the sheet the viewer
        // could draw for him is not a second-best version of one — it is a
        // different thing, and a squad wearing a mix of the two reads as half
        // the men being somebody and half being nobody. So the picture is the
        // only thing that ever goes on a head: the page serves a photograph
        // for a real footballer and a drawn portrait for a regen, both of them
        // pictures of the man, and [`Portraits::attach`] lays whichever came
        // back over this material a few frames later.
        //
        // It has to be his own material even while it carries no picture,
        // because that handle is what the arrival is folded into — see
        // [`Portraits::send_for`], which is handed this and holds it.
        let painted = Self::flesh(materials, Complexion::face(player).skin);
        self.faces.push((player.id, painted));
    }

    pub fn shadow(&self) -> Handle<StandardMaterial> {
        self.shadow.clone()
    }

    /// # Why cloth is barely specular
    ///
    /// Half of what read as an inflated balloon on the shirt and the shorts
    /// was never the geometry: a `StandardMaterial` leaves `reflectance` at
    /// 0.5, which is a dielectric with a glassy 4% normal-incidence Fresnel,
    /// and a broad soft highlight rolling across every big convex panel is
    /// exactly the cue the eye uses to read a surface as pressurised. Compare
    /// [`Self::flesh`], which deliberately asks for 0.35 to get a trace of
    /// specular back on an arm.
    ///
    /// A jersey is matte and dry and its highlight is diffuse. 0.18 puts the
    /// sheen most of the way out, and what is left of the shading on a chest
    /// is the shape of the chest.
    fn cloth(
        materials: &mut Assets<StandardMaterial>,
        color: Color,
        roughness: f32,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            reflectance: 0.18,
            metallic: 0.0,
            ..default()
        })
    }

    /// The two things on a footballer that are NOT fabric: a boot and a
    /// keeper's glove. Both are moulded surfaces with a real sheen on them —
    /// a boot is the shiniest thing on the pitch — so they keep the specular
    /// [`Self::cloth`] gives up.
    fn moulded(
        materials: &mut Assets<StandardMaterial>,
        color: Color,
        roughness: f32,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic: 0.0,
            ..default()
        })
    }

    fn flesh(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.62,
            // Skin is not a diffuse surface: a trace of specular is the
            // difference between an arm and a painted stick.
            reflectance: 0.35,
            metallic: 0.0,
            ..default()
        })
    }

    /// Lettering on a shirt: the glyphs come out of the texture's alpha and
    /// the ink colour off the strip, so black-on-white and white-on-black are
    /// the same material with two arguments.
    ///
    /// # Why these are not blended
    ///
    /// A print is a cutout — every texel is either ink or shirt, and the only
    /// texels between the two are the handful the rasteriser softened at a
    /// glyph's edge. That is not what `AlphaMode::Blend` is for, and asking for
    /// it was costing a good deal more than it looked:
    ///
    /// **Blended materials go into the sorted transparent phase.** Forty-four
    /// of them — a number and a name per player, each with a texture of its
    /// own and so a material of its own — are depth-sorted every frame and
    /// submitted one at a time. Worse, they sort INTO the twenty-three contact
    /// shadows, which share one mesh and one material and would otherwise
    /// batch into a single draw: a player and the shadow at his feet are the
    /// same distance from the lens, so the sorted phase came out as shadow,
    /// name, number, shadow, name, number all the way down the pitch, and
    /// every one of those alternations breaks the batch. Twenty-odd draw calls
    /// a frame, spent on nothing, in the phase that also blends rather than
    /// tests its way past the depth buffer.
    ///
    /// **Alpha to coverage is the same picture in the opaque phase.** With
    /// multisampling on — see [`crate::app::quality`] — the hardware turns
    /// the glyph's alpha into sample coverage, which for a cutout is what
    /// blending was approximating in the first place; the print is binned
    /// rather than sorted, it is drawn front to back, and it gets the depth
    /// buffer's early rejection like everything else on the man. With
    /// multisampling off Bevy falls back to a discard at 0.5 and the edge
    /// hardens by a texel — on a number that lands twenty pixels wide, under
    /// an FXAA pass whose whole job is high-contrast edges exactly like this
    /// one.
    ///
    /// The print floats four millimetres off the cloth (`BodyParts::
    /// PRINT_LIFT`), so writing depth from the opaque phase orders it against
    /// the shirt correctly rather than fighting it.
    fn printed(
        materials: &mut Assets<StandardMaterial>,
        ink: Color,
        texture: Handle<Image>,
    ) -> Handle<StandardMaterial> {
        let ink = ink.to_srgba();
        materials.add(StandardMaterial {
            base_color: Color::srgba(ink.red, ink.green, ink.blue, 1.0),
            base_color_texture: Some(texture),
            alpha_mode: AlphaMode::AlphaToCoverage,
            perceptual_roughness: 0.85,
            ..default()
        })
    }

    /// The walk-out plate: the same cutout, and the same reasons for it, with
    /// the tint taken OFF.
    ///
    /// A plate is two colours — the band and the name knocked out of it — and a
    /// `base_color` can only be one, so
    /// [`Textures::name_plate`](crate::art::textures::Textures::name_plate)
    /// paints both into the texels and this leaves them alone. White is the
    /// identity for that multiply, not a colour choice.
    fn plated(
        materials: &mut Assets<StandardMaterial>,
        texture: Handle<Image>,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            alpha_mode: AlphaMode::AlphaToCoverage,
            perceptual_roughness: 0.85,
            ..default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(skin: u8, hair: u8, eyes: u8) -> PlayerInfo {
        PlayerInfo {
            id: 1,
            shirt_number: 9,
            last_name: "Okocha".to_string(),
            position: "ST".to_string(),
            is_home: true,
            starting: true,
            skin,
            hair,
            eyes,
            photo: None,
            face: None,
        }
    }

    /// Every entry of the shared tables has to survive the trip through
    /// `Srgba::hex`, because the fallback in [`Complexion::tone`] would put a
    /// whole ramp of complexions on one colour without saying so.
    #[test]
    fn every_shared_colour_parses() {
        for hex in Palette::SKIN
            .iter()
            .chain(Palette::HAIR.iter())
            .chain(Palette::EYES.iter())
        {
            assert!(Srgba::hex(hex).is_ok(), "{hex} is not a colour");
        }
    }

    /// The indices arrive over the wire from a page this crate does not
    /// compile with, so a bad one has to land somewhere rather than index
    /// past the end of a table.
    #[test]
    fn an_index_off_the_end_of_a_ramp_lands_on_the_last_entry() {
        let stray = player(200, 200, 200);
        assert_eq!(Complexion::skin(&stray), Palette::SKIN.len() - 1);
        assert_eq!(Complexion::hair(&stray), Palette::HAIR.len() - 1);
        assert_eq!(Complexion::eyes(&stray), Palette::EYES.len() - 1);
        // And the face built off them is still a face.
        let look = Complexion::face(&stray);
        assert_eq!(look.skin, Complexion::tone(Palette::SKIN[11]));
    }

    /// A sheet is the flat kit colours by other means, so the texel a merged
    /// shirt samples has to BE the shirt colour the separate material carried
    /// — to the one sRGB byte the trip through a texture costs. A sheet that
    /// drifted would repaint every merged part on the pitch and nothing else
    /// would notice: the materials compile, the batches draw, the club just
    /// comes out in the wrong colours.
    #[test]
    fn the_sheet_answers_in_the_strips_own_colours() {
        let colors = TeamColors {
            background: "#132257".to_string(),
            foreground: "#ffffff".to_string(),
        };
        let strip = Strip::outfield(&colors, Wardrobe::HOME_FALLBACK);
        let sheet = Wardrobe::sheet_image(
            Complexion::tone(Palette::SKIN[3]),
            &[strip.shirt, strip.trim, strip.shorts, strip.socks],
        );
        let texel = |swatch: Swatch| {
            let x = (swatch.uv()[0] * Swatch::SPAN as f32) as usize;
            let data = sheet.data.as_ref().expect("the sheet has pixels");
            Color::srgb_u8(data[x * 4], data[x * 4 + 1], data[x * 4 + 2])
        };
        let close = |painted: Color, worn: Color| {
            let (painted, worn) = (painted.to_srgba(), worn.to_srgba());
            (painted.red - worn.red).abs() < 0.005
                && (painted.green - worn.green).abs() < 0.005
                && (painted.blue - worn.blue).abs() < 0.005
        };
        assert!(close(texel(Swatch::Shirt), strip.shirt), "the shirt drifted");
        assert!(close(texel(Swatch::Trim), strip.trim), "the trim drifted");
        assert!(
            close(texel(Swatch::Shorts), strip.shorts),
            "the shorts drifted"
        );
        assert!(close(texel(Swatch::Socks), strip.socks), "the socks drifted");
        assert!(
            close(texel(Swatch::Skin), Complexion::tone(Palette::SKIN[3])),
            "the skin drifted"
        );
    }

    /// …and the surface strip keeps each cloth's own roughness where the
    /// separate materials wrote it, in the green channel the standard
    /// material reads.
    #[test]
    fn the_roughness_strip_holds_each_cloths_grain() {
        let strip = Wardrobe::roughness_image();
        let data = strip.data.as_ref().expect("the strip has pixels");
        for (swatch, wanted) in [
            (Swatch::Skin, 0.62f32),
            (Swatch::Shirt, 0.80),
            (Swatch::Shorts, 0.82),
            (Swatch::Socks, 0.88),
        ] {
            let x = (swatch.uv()[0] * Swatch::SPAN as f32) as usize;
            let painted = data[x * 4 + 1] as f32 / 255.0;
            assert!(
                (painted - wanted).abs() < 0.01,
                "{painted} where {wanted} belongs"
            );
            assert_eq!(data[x * 4 + 2], 0, "nothing on a footballer is metal");
        }
    }

    /// Two players from opposite ends of the world are not the same colour,
    /// and two players with the same nationality-derived indices ARE — which
    /// is exactly what the id hash this replaced could not do.
    #[test]
    fn colouring_follows_the_team_sheet_not_the_id() {
        let nigerian = player(9, 0, 0);
        let norwegian = player(0, 8, 3);
        assert_ne!(
            Complexion::face(&nigerian).skin,
            Complexion::face(&norwegian).skin
        );

        let mut same = player(9, 0, 0);
        same.id = 4_000_001;
        assert_eq!(
            Complexion::face(&nigerian).skin,
            Complexion::face(&same).skin
        );
        assert_eq!(
            Complexion::face(&nigerian).hair,
            Complexion::face(&same).hair
        );
    }
}
