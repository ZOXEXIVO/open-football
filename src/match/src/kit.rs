use crate::config::{PlayerInfo, TeamColors, ViewerConfig};
use crate::textures::Textures;
use bevy::image::Image;
use bevy::prelude::*;

/// The colours one side takes the field in.
struct Strip {
    shirt: Color,
    shorts: Color,
    socks: Color,
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
        Strip {
            shirt,
            shorts,
            // Socks in the shirt colour: down among twenty-two pairs of legs it
            // is the last place the eye can still pick a side out.
            socks: shirt,
        }
    }

    /// A keeper is the one player who has to be told apart from everybody on
    /// the pitch, so their colours come from neither club.
    fn keeper(shirt: Color) -> Self {
        Strip {
            shirt,
            shorts: Wardrobe::DARK,
            socks: Wardrobe::DARK,
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

/// The parts of a footballer's appearance that have nothing to do with the
/// club: height, skin, hair and boots.
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

    /// Multiplier on the model's height — a hand's width either side of 1.80 m.
    pub fn height(id: u32) -> f32 {
        0.962 + ((Self::hash(id) >> 20) % 76) as f32 / 1000.0
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

/// The materials one player wears. Handles only — every one of them is shared
/// with the rest of the squad.
#[derive(Clone)]
pub struct Outfit {
    pub shirt: Handle<StandardMaterial>,
    pub shorts: Handle<StandardMaterial>,
    pub socks: Handle<StandardMaterial>,
    pub boots: Handle<StandardMaterial>,
    pub skin: Handle<StandardMaterial>,
    pub hands: Handle<StandardMaterial>,
    pub hair: Handle<StandardMaterial>,
}

/// One strip, as materials.
struct Kit {
    shirt: Handle<StandardMaterial>,
    shorts: Handle<StandardMaterial>,
    socks: Handle<StandardMaterial>,
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
}

impl Wardrobe {
    const LIGHT: Color = Color::srgb(0.91, 0.92, 0.94);
    const DARK: Color = Color::srgb(0.10, 0.11, 0.15);
    /// Keeper strips, home then away. Neither belongs to a club: a keeper has
    /// to be told apart from twenty outfielders, from the other keeper, and
    /// from the grass they are standing on.
    const KEEPERS: [Color; 2] = [Color::srgb(0.96, 0.83, 0.15), Color::srgb(0.90, 0.28, 0.62)];
    const GLOVES: Color = Color::srgb(0.88, 0.90, 0.94);
    const HOME_FALLBACK: Color = Color::srgb(0.0, 0.19, 0.49);
    const AWAY_FALLBACK: Color = Color::srgb(0.70, 0.25, 0.0);

    pub fn new(
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        config: &ViewerConfig,
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
        let kits = [
            Strip::outfield(&config.home, Self::HOME_FALLBACK),
            Strip::outfield(&config.away, Self::AWAY_FALLBACK),
            Strip::keeper(Self::KEEPERS[0]),
            Strip::keeper(Self::KEEPERS[1]),
        ]
        .map(|strip| Kit {
            shirt: Self::cloth(materials, strip.shirt, 0.72),
            shorts: Self::cloth(materials, strip.shorts, 0.75),
            socks: Self::cloth(materials, strip.socks, 0.88),
        });

        let ring = Textures::ring(images);
        let blob = Textures::blob(images);
        Wardrobe {
            kits,
            skin,
            hair,
            boots,
            gloves,
            markers: [
                Self::paint(materials, &ring, config.home.background_color(Self::HOME_FALLBACK)),
                Self::paint(materials, &ring, config.away.background_color(Self::AWAY_FALLBACK)),
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

    /// What this player is wearing. The strip comes off the team sheet, the
    /// rest from who they are.
    pub fn outfit(&self, player: &PlayerInfo) -> Outfit {
        let kit = match (player.is_goalkeeper(), player.is_home) {
            (false, true) => &self.kits[0],
            (false, false) => &self.kits[1],
            (true, true) => &self.kits[2],
            (true, false) => &self.kits[3],
        };
        let skin = self.skin[Complexion::skin(player.id)].clone();
        Outfit {
            shirt: kit.shirt.clone(),
            shorts: kit.shorts.clone(),
            socks: kit.socks.clone(),
            boots: self.boots[Complexion::boots(player.id)].clone(),
            hands: if player.is_goalkeeper() {
                self.gloves.clone()
            } else {
                skin.clone()
            },
            skin,
            hair: self.hair[Complexion::hair(player.id)].clone(),
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

    fn flesh(
        materials: &mut Assets<StandardMaterial>,
        color: Color,
    ) -> Handle<StandardMaterial> {
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
