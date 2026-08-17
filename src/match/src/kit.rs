use crate::config::{PlayerInfo, TeamColors, ViewerConfig};
use crate::textures::{Beard, FaceLayout, FaceLook, Textures};
use bevy::image::Image;
use bevy::prelude::*;

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
/// Picked from the player's id rather than at random, so a squad reads as
/// eleven individuals and looks the same every time the match is replayed.
pub struct Complexion;

impl Complexion {
    const SKIN: [Color; 5] = [
        Color::srgb(0.93, 0.77, 0.65),
        Color::srgb(0.85, 0.66, 0.51),
        Color::srgb(0.71, 0.51, 0.37),
        Color::srgb(0.51, 0.34, 0.23),
        Color::srgb(0.35, 0.22, 0.15),
    ];
    const HAIR: [Color; 5] = [
        Color::srgb(0.07, 0.06, 0.06),
        Color::srgb(0.20, 0.13, 0.09),
        Color::srgb(0.33, 0.21, 0.12),
        Color::srgb(0.70, 0.56, 0.30),
        Color::srgb(0.46, 0.44, 0.42),
    ];
    /// Irises, darkest first. Weighted toward brown below, which is what most
    /// of the world's eyes are.
    const EYES: [Color; 4] = [
        Color::srgb(0.20, 0.13, 0.08),
        Color::srgb(0.38, 0.25, 0.13),
        Color::srgb(0.36, 0.44, 0.30),
        Color::srgb(0.38, 0.50, 0.58),
    ];
    const BOOTS: [Color; 4] = [
        Color::srgb(0.06, 0.06, 0.07),
        Color::srgb(0.92, 0.93, 0.95),
        Color::srgb(0.86, 0.15, 0.24),
        Color::srgb(0.55, 0.90, 0.24),
    ];

    fn skin(id: u32) -> usize {
        (Self::hash(id) % Self::SKIN.len() as u32) as usize
    }

    fn hair(id: u32) -> usize {
        ((Self::hash(id) >> 8) % Self::HAIR.len() as u32) as usize
    }

    fn boots(id: u32) -> usize {
        ((Self::hash(id) >> 16) % Self::BOOTS.len() as u32) as usize
    }

    /// Everything about a player's face that is his and not his club's.
    ///
    /// Drawn off SALTED hashes rather than off further bits of the one above:
    /// that hash has thirty-two bits and six traits already spoken for, and
    /// two features cut from overlapping bits are correlated — every blond
    /// would end up with the same beard.
    pub fn face(id: u32) -> FaceLook {
        FaceLook {
            skin: Self::SKIN[Self::skin(id)],
            hair: Self::HAIR[Self::hair(id)],
            eyes: Self::EYES[match Self::trait_of(id, 0x51ED) {
                0..45 => 0,
                45..75 => 1,
                75..90 => 2,
                _ => 3,
            }],
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

    /// …and how fast he ticks over standing still — breathing, shifting his
    /// weight. Twenty-two men breathing in unison is its own kind of robot.
    pub fn tempo(id: u32) -> f32 {
        0.80 + Self::trait_of(id, 0x3D9E) as f32 / 250.0
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
    pub shirt: Handle<StandardMaterial>,
    pub shorts: Handle<StandardMaterial>,
    pub socks: Handle<StandardMaterial>,
    /// Collar and cuffs.
    pub trim: Handle<StandardMaterial>,
    pub boots: Handle<StandardMaterial>,
    pub skin: Handle<StandardMaterial>,
    /// The head, which is his own: skin with a face painted on it.
    pub face: Handle<StandardMaterial>,
    pub hands: Handle<StandardMaterial>,
    pub hair: Handle<StandardMaterial>,
    pub hair_style: HairStyle,
    /// `None` for a player the team sheet gave no shirt number, and for one
    /// whose name has nothing the shirt printer can set.
    pub number: Option<Handle<StandardMaterial>>,
    pub name: Option<Handle<StandardMaterial>>,
}

/// One strip, as materials.
struct Kit {
    shirt: Handle<StandardMaterial>,
    shorts: Handle<StandardMaterial>,
    socks: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
}

/// Every material the twenty-two players and their markers need, built once.
///
/// Sharing them is what keeps a pitch full of footballers down to a couple of
/// dozen draw calls: the renderer batches by mesh and material, and there are
/// only ever four strips and a handful of appearances on the field.
pub struct Wardrobe {
    kits: [Kit; 4],
    skin: Vec<Handle<StandardMaterial>>,
    hair: Vec<Handle<StandardMaterial>>,
    boots: Vec<Handle<StandardMaterial>>,
    gloves: Handle<StandardMaterial>,
    markers: [Handle<StandardMaterial>; 2],
    shadow: Handle<StandardMaterial>,
    /// The three materials that cannot be shared, because what makes them
    /// differ is baked into a texture: a printed number, a printed name and a
    /// face. Twenty-two of each is a rounding error against the draw calls
    /// they save everywhere else.
    numbers: Vec<(u32, Handle<StandardMaterial>)>,
    names: Vec<(u32, Handle<StandardMaterial>)>,
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
        face: &FaceLayout,
    ) -> Self {
        let skin: Vec<Handle<StandardMaterial>> = Complexion::SKIN
            .iter()
            .map(|color| Self::flesh(materials, *color))
            .collect();
        let hair: Vec<Handle<StandardMaterial>> = Complexion::HAIR
            .iter()
            .map(|color| Self::cloth(materials, *color, 0.95))
            .collect();
        let boots: Vec<Handle<StandardMaterial>> = Complexion::BOOTS
            .iter()
            .map(|color| Self::cloth(materials, *color, 0.35))
            .collect();
        let gloves = Self::cloth(materials, Self::GLOVES, 0.7);

        // One kit per (side, keeper) pairing: the only four strips that can
        // take the field.
        let strips = [
            Strip::outfield(&config.home, Self::HOME_FALLBACK),
            Strip::outfield(&config.away, Self::AWAY_FALLBACK),
            Strip::keeper(Self::KEEPERS[0]),
            Strip::keeper(Self::KEEPERS[1]),
        ];
        let kits = [0, 1, 2, 3].map(|index| Kit {
            shirt: Self::cloth(materials, strips[index].shirt, 0.72),
            shorts: Self::cloth(materials, strips[index].shorts, 0.75),
            socks: Self::cloth(materials, strips[index].socks, 0.88),
            trim: Self::cloth(materials, strips[index].trim, 0.70),
        });

        let numbers = config
            .players
            .iter()
            .filter(|player| player.shirt_number > 0)
            .map(|player| {
                let texture = Textures::number(images, player.shirt_number);
                (
                    player.id,
                    Self::printed(materials, strips[Self::strip_index(player)].print, texture),
                )
            })
            .collect();
        let names = config
            .players
            .iter()
            .filter_map(|player| {
                let texture = Textures::name(images, &player.last_name)?;
                Some((
                    player.id,
                    Self::printed(materials, strips[Self::strip_index(player)].print, texture),
                ))
            })
            .collect();
        let faces = config
            .players
            .iter()
            .map(|player| {
                let texture = Textures::face(images, face, &Complexion::face(player.id));
                (player.id, Self::flesh_texture(materials, texture))
            })
            .collect();

        let ring = Textures::ring(images);
        let blob = Textures::blob(images);
        Wardrobe {
            kits,
            numbers,
            names,
            faces,
            skin,
            hair,
            boots,
            gloves,
            markers: [
                Self::paint(
                    materials,
                    &ring,
                    config.home.background_color(Self::HOME_FALLBACK),
                ),
                Self::paint(
                    materials,
                    &ring,
                    config.away.background_color(Self::AWAY_FALLBACK),
                ),
            ],
            shadow: materials.add(StandardMaterial {
                base_color: Color::srgba(0.0, 0.0, 0.0, 0.40),
                base_color_texture: Some(blob),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            }),
        }
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
    pub fn outfit(&self, player: &PlayerInfo) -> Outfit {
        let kit = &self.kits[Self::strip_index(player)];
        let skin = self.skin[Complexion::skin(player.id)].clone();
        let own = |table: &Vec<(u32, Handle<StandardMaterial>)>| {
            table
                .iter()
                .find(|(id, _)| *id == player.id)
                .map(|(_, material)| material.clone())
        };
        Outfit {
            shirt: kit.shirt.clone(),
            shorts: kit.shorts.clone(),
            socks: kit.socks.clone(),
            trim: kit.trim.clone(),
            boots: self.boots[Complexion::boots(player.id)].clone(),
            hands: if player.is_goalkeeper() {
                self.gloves.clone()
            } else {
                skin.clone()
            },
            // Every player has a face; falling back to bare skin would be a
            // head with nothing on the front of it, which is worse than any
            // face this could draw.
            face: own(&self.faces).unwrap_or_else(|| skin.clone()),
            skin,
            hair: self.hair[Complexion::hair(player.id)].clone(),
            hair_style: Complexion::hair_style(player.id),
            number: own(&self.numbers),
            name: own(&self.names),
        }
    }

    /// The team-coloured ring drawn round a player's boots — the one thing
    /// carried over from the flat markers this replaced, and still the fastest
    /// way to read a crowded penalty area.
    pub fn marker(&self, is_home: bool) -> Handle<StandardMaterial> {
        self.markers[usize::from(!is_home)].clone()
    }

    pub fn shadow(&self) -> Handle<StandardMaterial> {
        self.shadow.clone()
    }

    fn cloth(
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

    /// The same, but with the colour coming out of an image — a face.
    fn flesh_texture(
        materials: &mut Assets<StandardMaterial>,
        texture: Handle<Image>,
    ) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture),
            perceptual_roughness: 0.62,
            reflectance: 0.35,
            metallic: 0.0,
            ..default()
        })
    }

    /// Lettering on a shirt: the glyphs come out of the texture's alpha and
    /// the ink colour off the strip, so black-on-white and white-on-black are
    /// the same material with two arguments.
    fn printed(
        materials: &mut Assets<StandardMaterial>,
        ink: Color,
        texture: Handle<Image>,
    ) -> Handle<StandardMaterial> {
        let ink = ink.to_srgba();
        materials.add(StandardMaterial {
            base_color: Color::srgba(ink.red, ink.green, ink.blue, 1.0),
            base_color_texture: Some(texture),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.85,
            ..default()
        })
    }

    fn paint(
        materials: &mut Assets<StandardMaterial>,
        texture: &Handle<Image>,
        color: Color,
    ) -> Handle<StandardMaterial> {
        let color = color.to_srgba();
        materials.add(StandardMaterial {
            base_color: Color::srgba(color.red, color.green, color.blue, 0.55),
            base_color_texture: Some(texture.clone()),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })
    }
}
