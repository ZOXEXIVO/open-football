use database::CountryLoader;
use std::sync::OnceLock;

// ── Skin color distribution ───────────────────────────────────

#[derive(Clone, Copy)]
pub struct SkinDist {
    pub white: u8,
    pub black: u8,
    pub _metis: u8,
}

impl Default for SkinDist {
    fn default() -> Self {
        SkinDist {
            white: 50,
            black: 20,
            _metis: 30,
        }
    }
}

static SKIN_MAP: OnceLock<Vec<(String, SkinDist)>> = OnceLock::new();

fn load_skin_map() -> Vec<(String, SkinDist)> {
    CountryLoader::load()
        .into_iter()
        .map(|c| {
            let d = SkinDist {
                white: c.skin_colors.white,
                black: c.skin_colors.black,
                _metis: c.skin_colors.metis,
            };
            (c.code, d)
        })
        .collect()
}

pub fn skin_distribution_for_country(code: &str) -> SkinDist {
    if code.is_empty() {
        return SkinDist::default();
    }
    let map = SKIN_MAP.get_or_init(load_skin_map);
    map.iter()
        .find(|(c, _)| c == code)
        .map(|(_, d)| *d)
        .unwrap_or_default()
}

// ── RNG ───────────────────────────────────────────────────────

struct FaceRng {
    state: u64,
}

impl FaceRng {
    fn new(player_id: u32) -> Self {
        let mut s = player_id as u64;
        s = s.wrapping_add(0x9E3779B97F4A7C15);
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
        s ^= s >> 31;
        if s == 0 {
            s = 1;
        }
        FaceRng { state: s }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn range(&mut self, max: usize) -> usize {
        (self.next() % max as u64) as usize
    }

    fn chance(&mut self, pct: u8) -> bool {
        (self.next() % 100) < pct as u64
    }

    fn frange(&mut self, min: f32, max: f32) -> f32 {
        let t = (self.next() % 10000) as f32 / 10000.0;
        min + t * (max - min)
    }
}

// ── Color palettes ──────────────────────────────────────────

const SKIN: [&str; 12] = [
    "#F5E0CB", "#EACFB0", "#DDBF98", "#CDA97A", "#C09368", "#A87D58", "#926845", "#7D5535",
    "#694530", "#503322", "#3D2518", "#2E1B11",
];

const HAIR: [&str; 10] = [
    "#0E0E0E", "#1C150C", "#2F1F11", "#4D3A2B", "#6A5038", "#7E644A", "#96795A", "#B0946C",
    "#C4A882", "#6B2010",
];

const EYES: [&str; 8] = [
    "#33251A", "#4A3828", "#5C4E3A", "#384F62", "#3D5844", "#4E6356", "#686D72", "#3F5A72",
];

// ── Color math ──────────────────────────────────────────────

fn hex_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&h[0..2], 16).unwrap_or(128),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(128),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(128),
    )
}

fn rgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

fn shade(hex: &str, f: f32) -> String {
    let (r, g, b) = hex_rgb(hex);
    rgb_hex(
        (r as f32 * f).min(255.0) as u8,
        (g as f32 * f).min(255.0) as u8,
        (b as f32 * f).min(255.0) as u8,
    )
}

fn opacity(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn blend(a: &str, b: &str, t: f32) -> String {
    let (ar, ag, ab) = hex_rgb(a);
    let (br, bg, bb) = hex_rgb(b);
    rgb_hex(
        (ar as f32 * (1.0 - t) + br as f32 * t) as u8,
        (ag as f32 * (1.0 - t) + bg as f32 * t) as u8,
        (ab as f32 * (1.0 - t) + bb as f32 * t) as u8,
    )
}

fn lip_color(skin: &str) -> String {
    let (r, g, b) = hex_rgb(skin);
    // Muted male lip tone: close to skin, not lipstick-like.
    rgb_hex(
        ((r as f32 * 0.78) + 20.0).min(255.0) as u8,
        ((g as f32 * 0.66) + 8.0).min(255.0) as u8,
        ((b as f32 * 0.62) + 8.0).min(255.0) as u8,
    )
}

// ── Face shape parameters ───────────────────────────────────

struct FaceShape {
    head_top: f32,
    temple_w: f32,
    cheek_w: f32,
    cheek_y: f32,
    jaw_w: f32,
    jaw_y: f32,
    chin_w: f32,
    chin_y: f32,
    chin_round: f32,
}

fn face_shape(variant: usize, fw: f32) -> FaceShape {
    // cheek_w <= temple_w always â€” face tapers smoothly from forehead down
    match variant {
        0 => FaceShape {
            // Oval
            head_top: 14.5,
            temple_w: 21.5 + fw * 0.7,
            cheek_w: 19.8 + fw * 0.65,
            cheek_y: 49.0,
            jaw_w: 17.2 + fw * 0.8,
            jaw_y: 70.0,
            chin_w: 7.8 + fw * 0.35,
            chin_y: 82.0,
            chin_round: 3.0,
        },
        1 => FaceShape {
            // Square
            head_top: 14.5,
            temple_w: 22.0 + fw * 0.75,
            cheek_w: 20.5 + fw * 0.7,
            cheek_y: 48.0,
            jaw_w: 20.0 + fw * 0.9,
            jaw_y: 71.0,
            chin_w: 11.5 + fw * 0.45,
            chin_y: 81.5,
            chin_round: 1.8,
        },
        2 => FaceShape {
            // Round
            head_top: 14.0,
            temple_w: 22.0 + fw * 0.75,
            cheek_w: 20.8 + fw * 0.75,
            cheek_y: 50.0,
            jaw_w: 18.8 + fw * 0.85,
            jaw_y: 71.0,
            chin_w: 9.8 + fw * 0.4,
            chin_y: 82.0,
            chin_round: 3.8,
        },
        3 => FaceShape {
            // Heart
            head_top: 14.0,
            temple_w: 22.0 + fw * 0.7,
            cheek_w: 19.6 + fw * 0.6,
            cheek_y: 48.0,
            jaw_w: 16.3 + fw * 0.55,
            jaw_y: 71.0,
            chin_w: 7.2 + fw * 0.25,
            chin_y: 82.5,
            chin_round: 2.4,
        },
        4 => FaceShape {
            // Oblong
            head_top: 12.5,
            temple_w: 20.5 + fw * 0.65,
            cheek_w: 19.3 + fw * 0.65,
            cheek_y: 48.0,
            jaw_w: 17.8 + fw * 0.75,
            jaw_y: 72.0,
            chin_w: 8.0 + fw * 0.3,
            chin_y: 84.0,
            chin_round: 2.2,
        },
        _ => FaceShape {
            // Diamond
            head_top: 13.5,
            temple_w: 20.8 + fw * 0.65,
            cheek_w: 20.2 + fw * 0.65,
            cheek_y: 48.0,
            jaw_w: 16.4 + fw * 0.55,
            jaw_y: 71.5,
            chin_w: 7.4 + fw * 0.25,
            chin_y: 83.0,
            chin_round: 2.0,
        },
    }
}

// ── Skin index picker ──────────────────────────────────────

fn pick_skin_index(r: &mut FaceRng, dist: SkinDist) -> usize {
    let roll = r.range(100) as u8;
    if roll < dist.white {
        r.range(4)
    } else if roll < dist.white + dist.black {
        8 + r.range(4)
    } else {
        3 + r.range(6)
    }
}

// ── Main generator ──────────────────────────────────────────

/// viewBox = "0 0 200 250" — portrait rectangle, head centered at x=100.
///
/// Painterly layered rendering: one top-left key light drives every
/// highlight and shadow, all face shading is Gaussian-blurred and clipped
/// to the head silhouette, and features are built from soft light/shadow
/// planes instead of stroked cartoon outlines.
///
/// `heft` is the player's weight-for-height deviation (≈ -2 lean .. +2.5
/// heavy): it fills the cheeks/jaw/neck instead of random width alone.
pub fn generate_face_svg(player_id: u32, age: u8, skin_dist: SkinDist, heft: f32) -> String {
    let heft = heft.clamp(-2.0, 2.5);
    let mut r = FaceRng::new(player_id);

    let skin_idx = pick_skin_index(&mut r, skin_dist);
    let skin = SKIN[skin_idx];
    // Hair color correlates with skin tone: darker complexions almost
    // always carry black/dark-brown hair, light blond stays European.
    let hair_roll = r.range(HAIR.len());
    let hair = HAIR[match skin_idx {
        8..=11 => hair_roll % 3,
        5..=7 => hair_roll % 6,
        _ => hair_roll,
    }];
    let eye_col = EYES[r.range(EYES.len())];

    let face_var = r.range(6);
    // Weighted style roll — everyday cuts dominate; statement styles
    // (mohawk, afro, long hair) are rare accents like on a real pitch
    let hair_st = match r.range(48) {
        0..=7 => 0,    // short crop
        8..=13 => 1,   // side part
        14..=19 => 2,  // medium
        20..=25 => 3,  // buzz
        26..=30 => 4,  // swept back
        31..=35 => 9,  // fade
        36..=38 => 7,  // curly
        39..=40 => 5,  // afro
        41..=42 => 8,  // long
        43..=44 => 11, // cornrows
        45..=46 => 6,  // bald
        _ => 10,       // faux-hawk
    };
    // A bald 17-year-old is not a thing — young players keep hair
    let hair_st = if age <= 23 && hair_st == 6 {
        0
    } else {
        hair_st
    };
    let brow_st = r.range(6);
    // Weighted eye-shape roll: open/large forms are the majority so narrow
    // forms stay distinct accents rather than the average look
    let eye_st = match r.range(12) {
        0..=2 => 0, // standard almond
        3..=4 => 2, // big round
        5..=6 => 7, // wide-open almond
        7 => 1,     // hooded
        8 => 3,     // monolid
        9 => 4,     // deep-set
        10 => 5,    // thin
        _ => 6,     // downturned
    };
    let nose_st = r.range(6);
    let mouth_st = r.range(5);
    let texture_seed = r.range(9999);
    let _cheekbone_st = r.range(4);
    let face_marks = r.range(5);

    // Facial hair by age
    let (bc, mc): (u8, u8) = match age {
        0..=19 => (0, 0),
        20..=24 => (18, 10),
        25..=29 => (40, 30),
        30..=34 => (55, 42),
        _ => (65, 50),
    };
    let beard = bc > 0 && r.chance(bc);
    let mstache = mc > 0 && r.chance(mc);
    let beard_v = r.range(5);
    let mst_v = r.range(4);

    // Asymmetry
    let ax = r.frange(-0.8, 0.8);
    let ay = r.frange(-0.5, 0.5);

    // Face width by age
    let fw: f32 = match age {
        0..=19 => r.frange(-1.1, 0.1),
        20..=24 => r.frange(-0.5, 0.8),
        25..=29 => r.frange(0.0, 1.6),
        30..=34 => r.frange(0.8, 2.4),
        _ => r.frange(1.3, 3.0),
    };

    // Continuous skull morph — identity-driven and age-independent, so two
    // same-age players still get visibly different heads (the 6 archetypes
    // only set the base proportions)
    let m_width = r.frange(-1.6, 1.6); // overall skull breadth
    let m_jaw = r.frange(-1.8, 1.8); // jaw breadth vs the rest of the skull
    let m_chin_w = r.frange(-1.4, 1.6); // chin breadth
    let m_length = r.frange(-1.5, 1.8); // face length
    let m_cheek = r.frange(-1.0, 1.2); // cheekbone prominence
    let m_round = r.frange(-1.0, 1.4); // chin rounding

    // Slight photographic head tilt
    let tilt = r.frange(-2.2, 2.2);

    // Continuous eye differentiation — deterministic per player via the
    // id-seeded rng, so a player's eyes never change between renders.
    let eye_spacing = r.frange(-1.3, 1.6); // inter-ocular distance
    let eye_tilt = r.frange(-1.2, 1.2); // outer-corner lift: down- vs upturned
    let lid_heavy = r.frange(0.0, 1.0); // hooded lids + deep sockets
    let eye_scale = r.frange(0.90, 1.10); // overall eye size
    let brow_gap = r.frange(-0.6, 2.0); // brow-to-eye distance

    let mut fs = face_shape(face_var, fw);
    // Apply the morph in shape space; clamps keep the silhouette tapering
    // (temple >= cheek >= jaw > chin) and the chin above the jersey line
    // Body weight fills the soft tissue: cheeks, jaw, chin — not the skull
    fs.temple_w += m_width + heft * 0.15;
    fs.cheek_w = (fs.cheek_w + m_width * 0.9 + m_cheek + heft * 0.5).min(fs.temple_w - 0.4);
    fs.jaw_w = (fs.jaw_w + m_width * 0.7 + m_jaw + heft * 0.8).min(fs.cheek_w - 0.2);
    fs.chin_w = (fs.chin_w + m_chin_w + heft * 0.3).clamp(5.5, fs.jaw_w - 3.0);
    fs.head_top -= m_length * 0.3;
    fs.cheek_y += m_length * 0.4;
    fs.jaw_y += m_length * 0.6;
    fs.chin_y = (fs.chin_y + m_length * 0.8).clamp(79.5, 84.5);
    // Floor 2.0: below that the jaw/chin corners render razor-sharp
    fs.chin_round = (fs.chin_round + m_round + heft * 0.35).clamp(2.0, 5.6);
    let cx = 100.0f32;
    let maturity = match age {
        0..=21 => 0.35,
        22..=27 => 0.55,
        28..=33 => 0.75,
        _ => 0.95,
    };

    // Deterministic per-strand jitter that never consumes the RNG stream
    let jit = move |i: usize, k: usize| ((texture_seed + i * 37 + k * 101) % 1000) as f32 / 1000.0;

    // ── Derived palette (single key light, top-left) ────────
    let skin_hi = blend(skin, "#FFDFC0", 0.26);
    let skin_hi2 = blend(skin, "#FFEBD2", 0.46);
    let skin_warm = blend(skin, "#C87850", 0.15);
    let skin_dk = shade(skin, 0.86);
    let skin_dk2 = shade(skin, 0.70);
    let skin_shadow = blend(&shade(skin, 0.52), "#2A1B22", 0.20);
    let hair_hi = shade(hair, 1.35);
    let hair_dk = shade(hair, 0.50);
    let iris_hi = shade(eye_col, 1.45);
    let iris_dk = shade(eye_col, 0.62);
    let iris_rim = shade(eye_col, 0.30);
    // Lips stay close to the skin tone — saturated pink reads feminine
    let lip = blend(&lip_color(skin), skin, 0.35);
    let lip_dk = shade(&lip, 0.62);
    let lip_hi = shade(&lip, 1.18);
    // Sclera follows the complexion so eyes don't glare on dark faces
    let sclera_0 = blend("#F6F2EA", skin, 0.16);
    let sclera_1 = blend("#EAE4D8", skin, 0.22);
    let sclera_2 = blend("#B4AB9D", skin, 0.30);

    // Jersey colors (deterministic from player_id). Wrapping_mul so large
    // 8-digit generated ids don't overflow u32 and crash the request.
    let jersey_hue = player_id.wrapping_mul(137) % 360;
    let jersey_color = format!("hsl({jersey_hue}, 30%, 30%)");
    let jersey_light = format!("hsl({jersey_hue}, 26%, 41%)");
    let jersey_dark = format!("hsl({jersey_hue}, 32%, 19%)");

    // Age features
    let wrinkle_opacity = match age {
        0..=25 => 0.0f32,
        26..=29 => 0.03,
        30..=33 => 0.07,
        34..=36 => 0.12,
        _ => 0.18,
    };
    let undereye_opacity = match age {
        0..=25 => 0.02f32,
        26..=29 => 0.05,
        30..=33 => 0.08,
        _ => 0.13,
    };

    // ── Geometry (scaled 2.5x from the 80x100 shape space) ──
    let s2 = |v: f32| v * 2.5;
    let ht = s2(fs.head_top);
    let hl = cx - s2(fs.temple_w);
    let hr = cx + s2(fs.temple_w);
    let cl = cx - s2(fs.cheek_w);
    let cr = cx + s2(fs.cheek_w);
    let jl = cx - s2(fs.jaw_w) + ax * 2.0;
    let jr = cx + s2(fs.jaw_w) - ax;
    let chl = cx - s2(fs.chin_w);
    let chr = cx + s2(fs.chin_w);
    let cy_cheek = s2(fs.cheek_y);
    let jy = s2(fs.jaw_y);
    let chy = s2(fs.chin_y);
    let cr_val = s2(fs.chin_round);
    let mid_r = (cr + jr) / 2.0;
    let mid_l = (cl + jl) / 2.0;
    let mid_y = (cy_cheek + jy) / 2.0;

    let ey = 118.0 + ay * 2.0;
    let by = 104.0 + ay - brow_gap;
    let ny = 148.0f32;
    let my = 169.0f32;
    let eye_off = 17.3 + eye_spacing;
    let exl = cx - eye_off + ax * 1.2;
    let exr = cx + eye_off - ax;

    // Feature style parameters
    // Eye archetypes — visibly distinct structures, not just size jitter:
    // (erx, ery, iris_r, pupil_r, bottom roundness, crease opacity,
    //  lid heaviness bonus, canthal-tilt bias)
    let (erx, ery, iris_r, pupil_r, eye_bot, crease_op, lid_extra, tilt_bias): (
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    ) = match eye_st {
        // Standard almond
        0 => (8.0, 3.6, 3.7, 1.45, 0.62, 0.30, 0.0, 0.0),
        // Hooded — skin fold droops over the outer lid, crease hidden
        1 => (8.0, 2.8, 3.5, 1.35, 0.52, 0.0, 1.0, -0.3),
        // Big round, wide open — iris fully visible
        2 => (8.3, 4.8, 4.1, 1.60, 1.00, 0.35, -0.5, 0.1),
        // Monolid, upturned — flat lid, no crease, epicanthic fold
        3 => (8.8, 2.4, 3.5, 1.30, 0.50, 0.0, 0.7, 1.3),
        // Deep-set — smaller opening in a strong socket
        4 => (7.4, 3.2, 3.4, 1.35, 0.62, 0.42, 0.4, -0.2),
        // Thin slit — long and very narrow, iris heavily cropped
        5 => (9.2, 1.9, 3.4, 1.25, 0.45, 0.15, 0.6, 0.4),
        // Downturned — outer corners drop, slightly heavy bottom
        6 => (8.1, 3.5, 3.7, 1.45, 0.70, 0.30, 0.2, -1.6),
        // Wide-open almond — tall but still pointed corners
        _ => (8.6, 4.4, 4.0, 1.55, 0.80, 0.28, -0.4, 0.5),
    };
    let erx = erx * eye_scale;
    let ery = ery * eye_scale;
    let iris_r = iris_r * (0.5 + 0.5 * eye_scale);
    let pupil_r = pupil_r * (0.5 + 0.5 * eye_scale);
    let tilt_eff = eye_tilt + tilt_bias;
    let (bridge_w, tip_w, _tip_h, nostril_w): (f32, f32, f32, f32) = match nose_st {
        0 => (0.9, 7.8, 3.6, 2.4),
        1 => (1.25, 11.5, 4.6, 3.6),
        2 => (1.05, 9.4, 4.2, 3.0),
        3 => (0.85, 7.2, 4.4, 2.3),
        4 => (1.15, 10.4, 4.6, 3.2),
        _ => (1.0, 8.7, 3.9, 2.7),
    };
    let (mw, upper_h, lower_h): (f32, f32, f32) = match mouth_st {
        0 => (12.5, 2.1, 3.0),
        1 => (15.0, 1.9, 3.3),
        2 => (10.8, 2.3, 2.7),
        3 => (13.4, 2.7, 3.6),
        _ => (11.8, 1.7, 2.5),
    };
    let (brow_len, brow_tilt, brow_arch, brow_n): (f32, f32, f32, usize) = match brow_st {
        0 => (12.5, 0.0, 1.6, 20),
        1 => (12.0, -0.4, 3.0, 20),
        2 => (13.5, 0.5, 1.3, 26),
        3 => (12.0, -0.2, 2.5, 22),
        4 => (14.0, 0.1, 2.0, 30),
        _ => (11.5, -0.2, 2.8, 18),
    };

    // Head outline — shared by the fill and the shading clip.
    // Lower face is all cubics with controls held OFF the chords: a control
    // on the straight line renders a flat plane and the jaw reads polygonal
    let chin_sag = (cr_val * 1.15).min(10.0);
    let head_d = format!(
        "M{hl} {cy_cheek} C{hl} {} {} {ht} {cx} {ht} C{} {ht} {hr} {} {hr} {cy_cheek} \
         C{hr} {} {} {mid_y} {jr} {jy} C{} {} {} {} {chr} {chy} \
         C{} {} {} {} {chl} {chy} C{} {} {} {} {jl} {jy} \
         C{} {mid_y} {hl} {} {hl} {cy_cheek}Z",
        ht + 22.0,
        hl + 14.0,
        hr - 14.0,
        ht + 22.0,
        // right cheek → jaw: c2 bulges outward off the chord
        jy - 10.0,
        mid_r + 2.0,
        // right jaw → chin: widely rounded corner, lands horizontal
        jr - cr_val * 0.2,
        jy + cr_val * 0.9,
        chr + 3.0,
        chy - cr_val * 0.1,
        // chin bottom: symmetric sag with horizontal corner tangents
        cx + (chr - cx) * 0.55,
        chy + chin_sag,
        cx - (cx - chl) * 0.55,
        chy + chin_sag,
        // left chin → jaw (mirror)
        chl - 3.0,
        chy - cr_val * 0.1,
        jl + cr_val * 0.2,
        jy + cr_val * 0.9,
        // left jaw → cheek
        mid_l - 2.0,
        jy - 10.0,
    );

    // Neck geometry (used by defs clip + drawing). Wide and flared into the
    // trapezius so it reads as part of the body, not a pedestal.
    // Thick neck is the strongest heavy-build cue in a portrait crop
    let neck_w = 19.0 + fw * 0.9 + heft * 1.5;
    let neck_top = chy - 20.0;
    let nkl = cx - neck_w;
    let nkr = cx + neck_w;
    let nkbl = nkl - 7.0;
    let nkbr = nkr + 7.0;
    let neck_d = format!(
        "M{nkl} {neck_top} C{nkl} {} {} 224 {nkbl} 228 L{nkbr} 228 C{} 224 {nkr} {} {nkr} {neck_top}Z",
        neck_top + 22.0,
        nkl - 4.0,
        nkr + 4.0,
        neck_top + 22.0,
    );

    let mut s = String::with_capacity(24000);
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 250">"#);
    // Debug trace of the sampled variants (invisible; keeps visual QA cheap)
    s.push_str(&format!(
        "<!--h{hair_st} e{eye_st} f{face_var} n{nose_st} b{} w{heft:.1}-->",
        u8::from(beard),
    ));

    // ── Defs ────────────────────────────────────────────────
    let seed_a = texture_seed;
    let seed_b = texture_seed + 7;
    let seed_c = texture_seed + 13;
    s.push_str(&format!(
        r##"<defs>
<radialGradient id="bgg" cx="50%" cy="34%" r="80%">
<stop offset="0%" stop-color="#51585E"/><stop offset="55%" stop-color="#383D41"/><stop offset="100%" stop-color="#212426"/>
</radialGradient>
<radialGradient id="sg" cx="42%" cy="30%" r="70%">
<stop offset="0%" stop-color="{skin_hi}"/><stop offset="30%" stop-color="{skin_warm}"/><stop offset="58%" stop-color="{skin}"/><stop offset="82%" stop-color="{skin_dk}"/><stop offset="100%" stop-color="{skin_dk2}"/>
</radialGradient>
<linearGradient id="sv" x1="0" y1="0" x2="0" y2="1">
<stop offset="0%" stop-color="{skin_shadow}" stop-opacity="0"/><stop offset="76%" stop-color="{skin_shadow}" stop-opacity="0"/><stop offset="100%" stop-color="{skin_shadow}" stop-opacity="0.30"/>
</linearGradient>
<linearGradient id="ng" x1="0" y1="0" x2="0" y2="1">
<stop offset="0%" stop-color="{skin_dk}"/><stop offset="45%" stop-color="{skin}"/><stop offset="100%" stop-color="{skin_dk}"/>
</linearGradient>
<linearGradient id="hg" x1="0" y1="0" x2="0" y2="1">
<stop offset="0%" stop-color="{hair_hi}"/><stop offset="45%" stop-color="{hair}"/><stop offset="100%" stop-color="{hair_dk}"/>
</linearGradient>
<linearGradient id="jg" x1="0" y1="0" x2="0" y2="1">
<stop offset="0%" stop-color="{jersey_light}"/><stop offset="70%" stop-color="{jersey_color}"/><stop offset="100%" stop-color="{jersey_dark}"/>
</linearGradient>
<radialGradient id="scg">
<stop offset="0%" stop-color="{sclera_0}"/><stop offset="65%" stop-color="{sclera_1}"/><stop offset="100%" stop-color="{sclera_2}"/>
</radialGradient>
<radialGradient id="irg">
<stop offset="0%" stop-color="{iris_hi}"/><stop offset="55%" stop-color="{eye_col}"/><stop offset="82%" stop-color="{iris_dk}"/><stop offset="100%" stop-color="{iris_rim}"/>
</radialGradient>
<radialGradient id="vig" cx="50%" cy="42%" r="74%">
<stop offset="0%" stop-color="#000" stop-opacity="0"/><stop offset="70%" stop-color="#000" stop-opacity="0"/><stop offset="100%" stop-color="#000" stop-opacity="0.36"/>
</radialGradient>
<filter id="b1" x="-40%" y="-40%" width="180%" height="180%"><feGaussianBlur stdDeviation="0.7"/></filter>
<filter id="b2" x="-60%" y="-60%" width="220%" height="220%"><feGaussianBlur stdDeviation="1.6"/></filter>
<filter id="b3" x="-80%" y="-80%" width="260%" height="260%"><feGaussianBlur stdDeviation="3.2"/></filter>
<filter id="b4" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="6"/></filter>
<filter id="gr" x="-5%" y="-5%" width="110%" height="110%">
<feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="{seed_a}" result="n"/>
<feColorMatrix in="n" type="saturate" values="0" result="d"/>
<feComponentTransfer in="d" result="a"><feFuncA type="linear" slope="0.09" intercept="0"/></feComponentTransfer>
<feComposite in="a" in2="SourceGraphic" operator="in"/>
</filter>
<filter id="stb" x="-20%" y="-20%" width="140%" height="140%">
<feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="3" seed="{seed_b}" result="n"/>
<feColorMatrix in="n" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1.8 -0.5" result="a"/>
<feComposite in="SourceGraphic" in2="a" operator="in"/>
</filter>
<filter id="htx" x="-15%" y="-15%" width="130%" height="130%">
<feTurbulence type="turbulence" baseFrequency="0.09 0.015" numOctaves="3" seed="{seed_c}" result="n"/>
<feDisplacementMap in="SourceGraphic" in2="n" scale="4" xChannelSelector="R" yChannelSelector="G"/>
</filter>
<filter id="hfx" x="-25%" y="-25%" width="150%" height="150%">
<feTurbulence type="fractalNoise" baseFrequency="0.05" numOctaves="3" seed="{seed_c}" result="n"/>
<feDisplacementMap in="SourceGraphic" in2="n" scale="11" xChannelSelector="R" yChannelSelector="G"/>
</filter>
<clipPath id="hc"><path d="{head_d}"/></clipPath>
<clipPath id="nc"><path d="{neck_d}"/></clipPath>
</defs>"##,
    ));

    // ── Background ──────────────────────────────────────────
    s.push_str(r#"<rect width="200" height="250" fill="url(#bgg)"/>"#);

    // ── Head group (slight photographic tilt) ───────────────
    s.push_str(&format!(r#"<g transform="rotate({tilt:.2} 100 205)">"#));

    // Long hair — back mass falls behind the head and shoulders
    if hair_st == 8 {
        let bl = hl - 6.0;
        let br_ = hr + 6.0;
        let bt = ht + 8.0;
        s.push_str(&format!(
            r#"<path d="M{bl} {bt} C{bl} {} {} {} {cx} {} C{} {} {br_} {} {br_} {bt} C{} 170 {} 195 {} 210 Q{cx} 222 {} 210 C{} 195 {} 170 {bl} {bt}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
            ht - 12.0,
            hl + 12.0,
            ht - 18.0,
            ht - 18.0,
            hr - 12.0,
            ht - 18.0,
            ht - 12.0,
            br_ + 3.0,
            br_ + 5.0,
            br_ - 2.0,
            bl + 2.0,
            bl - 5.0,
            bl - 3.0,
        ));
        // Hair falling behind the shoulders sits in shadow near the neck
        s.push_str(&format!(
            r#"<path d="M{} 155 L{} 212 L{} 212 L{} 155Z" fill="{hair_dk}" opacity="0.45" filter="url(#b3)"/>"#,
            nkl - 12.0,
            nkl - 8.0,
            nkr + 8.0,
            nkr + 12.0,
        ));
    }

    // ── Neck (behind head; jersey later covers its base) ────
    s.push_str(&format!(r#"<path d="{neck_d}" fill="url(#ng)"/>"#));
    s.push_str(r#"<g clip-path="url(#nc)">"#);
    // Cast shadow of the jaw onto the neck — strong photographic cue
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="{}" ry="6" fill="{skin_shadow}" filter="url(#b2)" opacity="0.40"/>"#,
        neck_top + 6.0,
        neck_w + 2.0,
    ));
    // Side planes
    s.push_str(&format!(
        r#"<path d="M{} {} L{} 228" stroke="{skin_dk2}" stroke-width="5" filter="url(#b2)" opacity="0.35" fill="none"/>"#,
        nkr - 1.0,
        neck_top + 6.0,
        nkbr - 1.0,
    ));
    s.push_str(&format!(
        r#"<path d="M{} {} L{} 228" stroke="{skin_hi}" stroke-width="3" filter="url(#b2)" opacity="0.16" fill="none"/>"#,
        nkl + 2.0,
        neck_top + 10.0,
        nkbl + 2.0,
    ));
    // Sternocleidomastoid hint
    s.push_str(&format!(
        r#"<path d="M{} {} Q{} {} {} 226" stroke="{skin_hi}" stroke-width="2" filter="url(#b2)" opacity="0.10" fill="none"/>"#,
        cx - 6.0,
        neck_top + 12.0,
        cx - 9.0,
        neck_top + 24.0,
        cx - 12.0,
    ));
    s.push_str("</g>");

    // ── Ears (drawn before the head so the head overlaps) ───
    let ear_y = ey + 9.0;
    let ear_col = blend(&skin_dk, "#B05840", 0.10);
    for (excc, side) in [(cl - 3.5, -1.0f32), (cr + 3.5, 1.0)] {
        s.push_str(&format!(
            r#"<ellipse cx="{excc}" cy="{ear_y}" rx="6.0" ry="11.5" fill="{ear_col}"/>"#,
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{ear_y}" rx="3.4" ry="7.4" fill="{skin_dk2}" opacity="0.55" filter="url(#b1)"/>"#,
            excc + side * 1.2,
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {ear_y} {} {}" stroke="{skin_hi}" stroke-width="1.1" fill="none" opacity="0.30" filter="url(#b1)"/>"#,
            excc + side * 1.0,
            ear_y - 7.5,
            excc + side * 4.6,
            excc + side * 1.6,
            ear_y + 7.0,
        ));
    }

    // ── Head base ───────────────────────────────────────────
    s.push_str(&format!(r#"<path d="{head_d}" fill="url(#sg)"/>"#));

    // ── Soft shading planes (all blurred, clipped to head) ──
    s.push_str(r#"<g clip-path="url(#hc)">"#);

    // Vertical falloff + side core shadow (light from upper-left)
    s.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="url(#sv)"/>"#,
        hl - 4.0,
        ht - 4.0,
        (hr - hl) + 8.0,
        (chy - ht) + 12.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="130" rx="24" ry="74" fill="{skin_dk2}" filter="url(#b4)" opacity="0.36"/>"#,
        hr - 4.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="120" rx="13" ry="60" fill="{skin_hi}" filter="url(#b4)" opacity="0.09"/>"#,
        hl + 3.0,
    ));

    // Forehead + brow-bone light
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="25" ry="15" fill="{skin_hi}" filter="url(#b3)" opacity="0.22"/>"#,
        cx - 8.0,
        ht + 32.0,
    ));
    for (bxh, op) in [(cx - 17.0, 0.20f32), (cx + 15.0, 0.10)] {
        s.push_str(&format!(
            r#"<ellipse cx="{bxh}" cy="{}" rx="11" ry="3.6" fill="{skin_hi}" filter="url(#b2)" opacity="{op}"/>"#,
            by - 5.5,
        ));
    }

    // Temple shadows
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="9" ry="20" fill="{skin_dk2}" filter="url(#b3)" opacity="0.14"/>"#,
        hl + 6.0,
        cy_cheek - 8.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="10" ry="21" fill="{skin_dk2}" filter="url(#b3)" opacity="0.26"/>"#,
        hr - 6.0,
        cy_cheek - 8.0,
    ));

    // Eye sockets — depth scales with the hooded-lid draw and archetype
    let socket_mul = 0.75 + lid_heavy * 0.5 + if eye_st == 4 { 0.45 } else { 0.0 };
    for (sxx, op) in [(exl, 0.13f32), (exr, 0.17)] {
        s.push_str(&format!(
            r#"<ellipse cx="{sxx}" cy="{}" rx="12.5" ry="8" fill="{skin_dk2}" filter="url(#b3)" opacity="{:.3}"/>"#,
            ey - 1.5,
            op * socket_mul,
        ));
    }

    // Cheekbone light + cheek hollow
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="134" rx="10" ry="5.5" fill="{skin_hi}" filter="url(#b3)" opacity="0.18" transform="rotate(-16 {} 134)"/>"#,
        cx - 26.0,
        cx - 26.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="134" rx="9" ry="5" fill="{skin_hi}" filter="url(#b3)" opacity="0.08" transform="rotate(16 {} 134)"/>"#,
        cx + 24.0,
        cx + 24.0,
    ));
    // Lean players get hollowed cheeks; heavy builds lose the hollow
    let hollow_op = (0.07 + maturity * 0.10 - heft * 0.035).max(0.02);
    for (hxx, rot) in [(cx - 25.0, 18.0f32), (cx + 23.0, -18.0)] {
        s.push_str(&format!(
            r#"<ellipse cx="{hxx}" cy="149" rx="9" ry="4.6" fill="{skin_dk2}" filter="url(#b3)" opacity="{hollow_op}" transform="rotate({rot} {hxx} 149)"/>"#,
        ));
    }

    // Nose planes: side shadows + dorsum highlight + base shadow
    let bx_off = 3.4 * bridge_w;
    s.push_str(&format!(
        r#"<path d="M{} 106 C{} 122 {} 136 {} {}" stroke="{skin_dk2}" stroke-width="3.2" fill="none" filter="url(#b2)" opacity="0.13"/>"#,
        cx - bx_off,
        cx - bx_off - 1.2,
        cx - bx_off - 1.8,
        cx - bx_off - 3.0,
        ny - 4.0,
    ));
    s.push_str(&format!(
        r#"<path d="M{} 106 C{} 122 {} 136 {} {}" stroke="{skin_dk2}" stroke-width="3.8" fill="none" filter="url(#b2)" opacity="0.21"/>"#,
        cx + bx_off,
        cx + bx_off + 1.2,
        cx + bx_off + 1.8,
        cx + bx_off + 3.0,
        ny - 4.0,
    ));
    s.push_str(&format!(
        r#"<path d="M{} 109 L{} {}" stroke="{skin_hi2}" stroke-width="2.4" fill="none" filter="url(#b2)" opacity="0.30"/>"#,
        cx + 0.4,
        cx + 0.2,
        ny - 6.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="8" ry="2.6" fill="{skin_shadow}" filter="url(#b2)" opacity="0.20"/>"#,
        ny + 5.0,
    ));

    // Jawline + chin + under-lip modelling
    s.push_str(&format!(
        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="3" fill="none" filter="url(#b3)" opacity="{}"/>"#,
        jl + 6.0,
        jy + 4.0,
        chy + 4.0,
        jr - 6.0,
        jy + 4.0,
        0.13 + maturity * 0.08,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="7" ry="4.5" fill="{skin_hi}" filter="url(#b2)" opacity="0.16"/>"#,
        cx - 2.0,
        chy - 9.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="7" ry="2.5" fill="{skin_dk2}" filter="url(#b2)" opacity="0.20"/>"#,
        my + 7.0,
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="18" ry="6" fill="{skin_shadow}" filter="url(#b3)" opacity="0.28"/>"#,
        chy + 2.0,
    ));

    // Nasolabial folds (deepen with age)
    let fold_opacity = 0.05 + wrinkle_opacity;
    for (fx0, fx1, fx2) in [
        (cx - 12.0, cx - 19.5, cx - 16.5),
        (cx + 12.0, cx + 19.5, cx + 16.5),
    ] {
        s.push_str(&format!(
            r#"<path d="M{fx0} {} Q{fx1} {} {fx2} {}" stroke="{skin_dk2}" stroke-width="1.1" fill="none" filter="url(#b1)" opacity="{fold_opacity}"/>"#,
            ny + 2.0,
            ny + 12.0,
            my + 2.0,
        ));
    }

    // Forehead wrinkles + eye-corner creases
    if wrinkle_opacity > 0.06 {
        for (wi, wy) in [74.0f32, 80.0, 86.0].iter().enumerate() {
            let wob = jit(wi, 3) * 1.6 - 0.8;
            s.push_str(&format!(
                r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="0.9" fill="none" filter="url(#b1)" opacity="{}"/>"#,
                cx - 19.0,
                wy + wob,
                wy + wob - 2.5,
                cx + 19.0,
                wy + wob,
                wrinkle_opacity * 0.65,
            ));
        }
    }
    if age >= 30 {
        for sidef in [-1.0f32, 1.0] {
            let ex_edge = cx + sidef * (17.3 + erx + 1.5);
            for k in 0..2 {
                let dy = k as f32 * 2.4 - 0.6;
                s.push_str(&format!(
                    r#"<path d="M{ex_edge} {} q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.7" fill="none" filter="url(#b1)" opacity="{}"/>"#,
                    ey + dy,
                    sidef * 2.6,
                    0.8 + k as f32 * 0.9,
                    sidef * 4.2,
                    2.2 + k as f32 * 1.4,
                    wrinkle_opacity * 0.55,
                ));
            }
        }
    }

    // Under-eye
    for uxx in [exl, exr] {
        s.push_str(&format!(
            r#"<ellipse cx="{uxx}" cy="{}" rx="7" ry="2.6" fill="{skin_dk2}" filter="url(#b2)" opacity="{undereye_opacity}"/>"#,
            ey + ery + 4.0,
        ));
    }

    // Beauty mark (sparse)
    if face_marks == 4 {
        let mkx = cx + (jit(11, 5) * 44.0 - 22.0);
        let mky = 128.0 + jit(7, 9) * 46.0;
        s.push_str(&format!(
            r#"<circle cx="{mkx:.1}" cy="{mky:.1}" r="0.8" fill="{skin_shadow}" opacity="0.55" filter="url(#b1)"/>"#,
        ));
    }

    // Film grain over the face
    s.push_str(&format!(
        r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#888" filter="url(#gr)"/>"##,
        hl - 4.0,
        ht - 4.0,
        (hr - hl) + 8.0,
        (chy - ht) + 12.0,
    ));

    s.push_str("</g>");

    // ── Eyes ────────────────────────────────────────────────
    for (i, (exc, side)) in [(exl, -1.0f32), (exr, 1.0)].iter().enumerate() {
        let exc = *exc;
        let el = exc - erx;
        let er_ = exc + erx;
        // Canthal tilt: outer corner (away from the nose) lifts or drops
        let el_y = if *side < 0.0 {
            ey - tilt_eff
        } else {
            ey + tilt_eff * 0.3
        };
        let er_y = if *side < 0.0 {
            ey + tilt_eff * 0.3
        } else {
            ey - tilt_eff
        };
        let top_y = ey - ery;
        let bot_y = ey + ery * eye_bot;
        let q_tl_x = el + erx * 0.3;
        let q_tr_x = exc + erx * 0.7;
        // Control above the peak so the upper lid arches instead of squinting
        let q_ty = ey - ery * 1.05;
        let q_br_x = er_ - erx * 0.3;
        let q_bl_x = exc - erx * 0.7;
        let q_by = ey + ery * eye_bot * 1.0;
        let almond = format!(
            "M{el} {el_y} Q{q_tl_x} {q_ty} {exc} {top_y} Q{q_tr_x} {q_ty} {er_} {er_y} \
             Q{q_br_x} {q_by} {exc} {bot_y} Q{q_bl_x} {q_by} {el} {el_y}Z"
        );

        // Sclera + clip
        s.push_str(&format!(r#"<path d="{almond}" fill="url(#scg)"/>"#));
        s.push_str(&format!(
            r#"<clipPath id="ec{i}"><path d="{almond}"/></clipPath><g clip-path="url(#ec{i})">"#
        ));

        // Iris (radial gradient), pupil, lid cast shadow, catchlights
        let ix = exc + side * 0.3;
        let iy = ey - 0.2;
        s.push_str(&format!(
            r#"<circle cx="{ix}" cy="{iy}" r="{iris_r}" fill="url(#irg)"/>"#
        ));
        s.push_str(&format!(
            r##"<circle cx="{ix}" cy="{iy}" r="{pupil_r}" fill="#0B0906"/>"##
        ));
        // Lid cast shadow scales with the opening so big eyes stay open;
        // only genuinely heavy-lidded archetypes may cover most of it
        let lid_cap = if lid_extra > 0.3 { ery } else { ery * 0.62 };
        let lid_ry = (ery * 0.36 + lid_heavy * 0.9 + lid_extra * 0.7).clamp(1.0, lid_cap);
        let lid_op = (0.18 + lid_heavy * 0.13 + lid_extra * 0.07).clamp(0.10, 0.42);
        s.push_str(&format!(
            r##"<ellipse cx="{exc}" cy="{top_y}" rx="{erx}" ry="{lid_ry:.2}" fill="#241713" opacity="{lid_op:.2}" filter="url(#b1)"/>"##,
        ));
        s.push_str(&format!(
            r##"<circle cx="{}" cy="{}" r="0.7" fill="#FFFFFF" opacity="0.85"/>"##,
            ix - 1.2,
            iy - 1.4,
        ));
        s.push_str(&format!(
            r##"<circle cx="{}" cy="{}" r="0.35" fill="#FFFFFF" opacity="0.25"/>"##,
            ix + 1.3,
            iy + 0.9,
        ));
        s.push_str("</g>");

        // Lash line — tapered, heavier on the outer half; weight follows opening
        let lash_w = (0.95 + ery * 0.13).min(1.6);
        s.push_str(&format!(
            r##"<path d="M{el} {} Q{exc} {} {er_} {}" stroke="#1C120C" stroke-width="{lash_w:.2}" fill="none" stroke-linecap="round" opacity="0.85" filter="url(#b1)"/>"##,
            el_y - 0.4,
            top_y - 1.3,
            er_y - 0.5,
        ));
        let lash_tip = exc + side * (erx + 1.4);
        s.push_str(&format!(
            r##"<path d="M{exc} {} Q{} {} {lash_tip} {}" stroke="#1C120C" stroke-width="0.9" fill="none" stroke-linecap="round" opacity="0.55" filter="url(#b1)"/>"##,
            top_y - 0.4,
            exc + side * erx * 0.65,
            top_y - 0.7,
            ey - 0.9,
        ));

        if eye_st == 1 {
            // Hooded fold — skin sags over the outer half of the lash line
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{skin}" stroke-width="2.6" fill="none" stroke-linecap="round" opacity="0.88" filter="url(#b1)"/>"#,
                exc - side * 2.0,
                top_y - 1.8,
                exc + side * erx * 0.55,
                top_y - 1.0,
                exc + side * (erx + 1.2),
                er_y + 0.3,
            ));
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.7" fill="none" opacity="0.30" filter="url(#b1)"/>"#,
                exc - side * 1.0,
                top_y - 2.4,
                exc + side * erx * 0.6,
                top_y - 1.8,
                exc + side * (erx + 1.6),
                er_y - 0.4,
            ));
        }

        // Lower lid: faint lash shadow + bright waterline
        s.push_str(&format!(
            r#"<path d="M{} {} Q{exc} {} {} {}" stroke="{skin_dk2}" stroke-width="0.6" fill="none" opacity="0.35" filter="url(#b1)"/>"#,
            el + 2.0,
            ey + 1.1,
            ey + ery + 0.9,
            er_ - 1.0,
            ey + 0.6,
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{exc} {} {} {}" stroke="{skin_hi2}" stroke-width="0.5" fill="none" opacity="0.40" filter="url(#b1)"/>"#,
            el + 2.5,
            ey + 0.6,
            ey + ery + 0.2,
            er_ - 1.5,
            ey + 0.2,
        ));

        // Eyelid crease — absent on monolid eyes
        if crease_op > 0.01 {
            s.push_str(&format!(
                r#"<path d="M{} {} Q{exc} {} {} {}" stroke="{skin_dk2}" stroke-width="0.8" fill="none" opacity="{crease_op}" filter="url(#b1)"/>"#,
                el - 0.8,
                top_y + 1.6,
                top_y - 3.6,
                er_ + 0.8,
                top_y + 1.6,
            ));
        }

        let inner_x = exc - side * erx;
        if eye_st == 3 {
            // Epicanthic fold — skin covers the inner corner
            s.push_str(&format!(
                r#"<path d="M{} {} Q{inner_x} {} {} {}" stroke="{skin}" stroke-width="1.7" fill="none" stroke-linecap="round" opacity="0.85" filter="url(#b1)"/>"#,
                inner_x - side * 0.6,
                ey - 2.2,
                ey - 0.4,
                inner_x + side * 2.4,
                ey + 1.6,
            ));
        } else {
            // Inner canthus
            let canthus = blend(skin, "#A85B50", 0.40);
            s.push_str(&format!(
                r#"<circle cx="{inner_x}" cy="{}" r="0.8" fill="{canthus}" opacity="0.50" filter="url(#b1)"/>"#,
                ey + 0.3,
            ));
        }
    }

    // ── Eyebrows (individual strands over a soft base) ──────
    {
        let brow_col = shade(hair, 0.85);
        for (bi, (exc, side)) in [(exl, -1.0f32), (exr, 1.0)].iter().enumerate() {
            let inner_x = exc - side * (brow_len - 2.5);
            let outer_x = exc + side * (brow_len + 2.0);
            let y0 = by + 1.4;
            let yc = by - brow_arch * 1.6;
            let y1 = by + 0.6 + brow_tilt * 2.4;
            let peak_x = (inner_x + outer_x) / 2.0 - side * 1.5;

            // Soft mass beneath the strands
            s.push_str(&format!(
                r#"<path d="M{inner_x} {y0} Q{peak_x} {yc} {outer_x} {y1}" stroke="{brow_col}" stroke-width="2.9" fill="none" stroke-linecap="round" opacity="0.30" filter="url(#b2)"/>"#,
            ));

            s.push_str(r#"<g filter="url(#b1)">"#);
            for k in 0..brow_n {
                let t = k as f32 / (brow_n - 1) as f32;
                let sx = inner_x + (outer_x - inner_x) * t + side * (jit(k, bi) - 0.5) * 1.6;
                let sy = (1.0 - t) * (1.0 - t) * y0
                    + 2.0 * t * (1.0 - t) * yc
                    + t * t * y1
                    + (jit(k, bi + 2) - 0.5) * 1.2;
                let dx = side * (0.8 + 2.4 * t);
                let dy = -(2.4 - 3.0 * t);
                let op = 0.35 + jit(k, bi + 4) * 0.30;
                s.push_str(&format!(
                    r#"<path d="M{sx:.1} {sy:.1} l{dx:.1} {dy:.1}" stroke="{brow_col}" stroke-width="0.55" fill="none" stroke-linecap="round" opacity="{op:.2}"/>"#,
                ));
            }
            s.push_str("</g>");
        }
    }

    // ── Nose: crisp details over the shading planes ─────────
    {
        let nl_ = cx - tip_w * 0.48;
        let nr_ = cx + tip_w * 0.48;
        // Tip highlight
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="2.6" ry="1.9" fill="{skin_hi2}" opacity="0.30" filter="url(#b1)"/>"#,
            cx + 0.6,
            ny - 2.0,
        ));
        // Nostrils — rotated soft ellipses
        for (nx, rot) in [(nl_, 18.0f32), (nr_, -18.0)] {
            s.push_str(&format!(
                r##"<ellipse cx="{nx}" cy="{}" rx="{}" ry="1.25" fill="#1A0E0A" opacity="0.55" filter="url(#b1)" transform="rotate({rot} {nx} {})"/>"##,
                ny + 1.6,
                nostril_w * 0.85,
                ny + 1.6,
            ));
        }
        // Alar wings
        for (wx, dir) in [
            (nl_ - nostril_w - 0.6, -1.0f32),
            (nr_ + nostril_w + 0.6, 1.0),
        ] {
            s.push_str(&format!(
                r#"<path d="M{wx} {} Q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.9" fill="none" opacity="0.35" filter="url(#b1)"/>"#,
                ny - 1.5,
                wx + dir * 1.2,
                ny + 1.5,
                wx - dir * 0.6,
                ny + 3.4,
            ));
        }
        // Septum shadow
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="2" ry="1.2" fill="{skin_dk2}" opacity="0.25" filter="url(#b1)"/>"#,
            ny + 2.4,
        ));
    }

    // ── Mouth (gradient lips, no hard outlines) ─────────────
    {
        let ml = cx - mw;
        let mr_ = cx + mw;
        let up_col = shade(&lip, 0.78);
        let line_col = shade(&lip_dk, 0.70);

        // Upper lip — cupid's bow, sits in shadow
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{} {} {} {} Q{cx} {} {} {} Q{} {} {mr_} {my} Q{cx} {} {ml} {my}Z" fill="{up_col}" opacity="0.55" filter="url(#b1)"/>"#,
            ml + mw * 0.35,
            my - upper_h * 0.4,
            cx - 3.2,
            my - upper_h,
            my - upper_h * 0.55,
            cx + 3.2,
            my - upper_h,
            mr_ - mw * 0.35,
            my - upper_h * 0.4,
            my + 0.8,
        ));
        // Mouth line
        s.push_str(&format!(
            r#"<path d="M{} {my} Q{cx} {} {} {my}" stroke="{line_col}" stroke-width="1.0" fill="none" stroke-linecap="round" opacity="0.80" filter="url(#b1)"/>"#,
            ml + 0.5,
            my + 1.1,
            mr_ - 0.5,
        ));
        // Lower lip — volume from a soft fill + highlight band
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {} Q{cx} {} {} {}Z" fill="{lip}" opacity="0.32" filter="url(#b1)"/>"#,
            ml + 1.5,
            my + 0.7,
            my + lower_h * 2.1,
            mr_ - 1.5,
            my + 0.7,
            my + 1.4,
            ml + 1.5,
            my + 0.7,
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="{lip_hi}" opacity="0.20" filter="url(#b1)"/>"#,
            cx - 1.0,
            my + lower_h * 1.05,
            mw * 0.38,
            lower_h * 0.5,
        ));
        // Mouth corners
        for mcx in [ml - 0.5, mr_ + 0.5] {
            s.push_str(&format!(
                r#"<circle cx="{mcx}" cy="{}" r="1.1" fill="{skin_shadow}" opacity="0.30" filter="url(#b1)"/>"#,
                my + 0.3,
            ));
        }
        // Philtrum ridges
        for (px, tx_) in [(cx - 2.1, cx - 1.6), (cx + 2.1, cx + 1.6)] {
            s.push_str(&format!(
                r#"<path d="M{px} {} L{tx_} {}" stroke="{skin_hi}" stroke-width="0.7" fill="none" opacity="0.18" filter="url(#b1)"/>"#,
                ny + 5.0,
                my - upper_h - 0.6,
            ));
        }
    }

    // ── Facial hair ─────────────────────────────────────────
    {
        // Lower-face region: outer edge follows the jaw, top edge runs under
        // the nose, with an evenodd hole punched around the lips.
        let btl = cl + 2.0;
        let btr = cr - 2.0;
        let bt_y = cy_cheek + 10.0;
        let jy8 = jy + 8.0;
        // Beard bottom must clear the head's chin sag or skin peeks through
        let chy7 = chy + chin_sag * 1.5 + 3.0;
        let nyb = ny + 2.0;
        let hole_rx = mw - 2.5;
        let hole_ry = (upper_h + lower_h - 1.5).max(3.0);
        let hole_cy = my + 0.5;
        let hole_l = cx - hole_rx;
        let hole_d = 2.0 * hole_rx;
        let beard_d = format!(
            "M{btl} {bt_y} C{} {jy} {} {jy8} {chl} {chy} Q{cx} {chy7} {chr} {chy} \
             C{} {jy8} {} {jy} {btr} {bt_y} C{} {nyb} {} {nyb} {btl} {bt_y}Z \
             M{hole_l} {hole_cy} a{hole_rx} {hole_ry} 0 1 0 {hole_d} 0 a{hole_rx} {hole_ry} 0 1 0 -{hole_d} 0Z",
            cl + 2.0,
            jl + 2.0,
            jr - 2.0,
            cr - 2.0,
            cx + 30.0,
            cx - 30.0,
        );
        let stubble_col = shade(hair, 0.72);

        if beard {
            match beard_v {
                0 => {
                    // Heavy stubble
                    s.push_str(&format!(
                        r#"<path d="{beard_d}" fill-rule="evenodd" fill="{stubble_col}" filter="url(#stb)" opacity="{}"/>"#,
                        opacity(0.38 + maturity * 0.12),
                    ));
                }
                1 => {
                    // Short boxed beard: soft mass + speckle texture
                    s.push_str(&format!(
                        r#"<path d="{beard_d}" fill-rule="evenodd" fill="{hair}" filter="url(#b3)" opacity="0.28"/>"#,
                    ));
                    s.push_str(&format!(
                        r#"<path d="{beard_d}" fill-rule="evenodd" fill="{stubble_col}" filter="url(#stb)" opacity="0.85"/>"#,
                    ));
                }
                2 => {
                    // Full beard with an under-chin curtain
                    s.push_str(&format!(
                        r#"<path d="{beard_d}" fill-rule="evenodd" fill="{hair}" filter="url(#b2)" opacity="0.75"/>"#,
                    ));
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{cx} {} {} {} Q{cx} {} {} {}Z" fill="{hair}" filter="url(#b1)" opacity="0.75"/>"#,
                        jl + 4.0,
                        jy + 10.0,
                        chy + 16.0,
                        jr - 4.0,
                        jy + 10.0,
                        chy + 6.0,
                        jl + 4.0,
                        jy + 10.0,
                    ));
                    s.push_str(&format!(
                        r#"<path d="{beard_d}" fill-rule="evenodd" fill="{hair_hi}" filter="url(#stb)" opacity="0.25"/>"#,
                    ));
                }
                3 => {
                    // Goatee — chin blob + soft edge
                    let goatee_d = format!(
                        "M{} {} Q{cx} {} {} {} L{} {chy} Q{cx} {} {} {chy}Z",
                        chl - 4.0,
                        my + 3.0,
                        my + 7.0,
                        chr + 4.0,
                        my + 3.0,
                        chr + 5.0,
                        chy + 8.0,
                        chl - 5.0,
                    );
                    s.push_str(&format!(
                        r#"<path d="{goatee_d}" fill="{hair}" filter="url(#b1)" opacity="0.55"/>"#,
                    ));
                    s.push_str(&format!(
                        r#"<path d="{goatee_d}" fill="{stubble_col}" filter="url(#stb)" opacity="0.80"/>"#,
                    ));
                }
                _ => {
                    // Chinstrap — speckled band hugging the jaw
                    s.push_str(&format!(
                        r#"<path d="M{} {} C{} {jy} {} {} {cx} {} C{} {} {} {jy} {} {}" stroke="{stubble_col}" stroke-width="6.5" fill="none" filter="url(#stb)" opacity="0.70"/>"#,
                        cl + 1.5,
                        cy_cheek + 12.0,
                        cl + 1.5,
                        jl + 3.0,
                        jy + 9.0,
                        chy + 3.0,
                        jr - 3.0,
                        jy + 9.0,
                        cr - 1.5,
                        cr - 1.5,
                        cy_cheek + 12.0,
                    ));
                }
            }
        } else if age >= 22 {
            // Five o'clock shadow — deepens with maturity
            s.push_str(&format!(
                r#"<path d="{beard_d}" fill-rule="evenodd" fill="{stubble_col}" filter="url(#stb)" opacity="{}"/>"#,
                opacity(0.10 + maturity * 0.14),
            ));
        }

        if mstache {
            let (mst_w, mst_h, mst_op): (f32, f32, f32) = match mst_v {
                0 => (11.0, 2.4, 0.50),
                1 => (14.0, 5.0, 0.75),
                2 => (17.0, 4.5, 0.70),
                _ => (15.0, 6.0, 0.70),
            };
            let mst_d = format!(
                "M{} {} Q{cx} {} {} {} Q{cx} {} {} {}Z",
                cx - mst_w,
                my - 1.2,
                my - upper_h - mst_h,
                cx + mst_w,
                my - 1.2,
                my - 2.2,
                cx - mst_w,
                my - 1.2,
            );
            s.push_str(&format!(
                r#"<path d="{mst_d}" fill="{hair}" filter="url(#b1)" opacity="{}"/>"#,
                mst_op * 0.55,
            ));
            s.push_str(&format!(
                r#"<path d="{mst_d}" fill="{stubble_col}" filter="url(#stb)" opacity="{mst_op}"/>"#,
            ));
            if mst_v == 2 {
                // Handlebar ends
                for dir in [-1.0f32, 1.0] {
                    let hx = cx + dir * mst_w;
                    s.push_str(&format!(
                        r#"<path d="M{hx} {} q{} {} {} {}" stroke="{hair}" stroke-width="1.6" fill="none" stroke-linecap="round" filter="url(#b1)" opacity="0.6"/>"#,
                        my - 1.5,
                        dir * 2.4,
                        2.0,
                        dir * 3.2,
                        5.0,
                    ));
                }
            }
        }
    }

    // ── Hair ────────────────────────────────────────────────
    {
        // Sides stop at the ear-top junction: lower, the skull is wider than
        // the temples and the hair edge would float inside the cheek
        let side_y = cy_cheek - 7.0;
        // Inner hairline height per style; None = no forehead edge (bald)
        let mut hairline: Option<f32> = None;
        // Temple recession control: deepens with age; teens keep a rounded
        // hairline instead of an M-shaped one
        let rec_y = ht
            + match age {
                0..=23 => 17.0,
                24..=29 => 14.5,
                _ => 12.0,
            };
        // Outer hair edge mirroring the real skull bezier from head_d
        // (controls hl/ht+22 and hl+14/ht), pushed out by `o` and lifted to
        // `crown` — hair must track the morphed head, not a fixed template
        let skull_edge = |o: f32, crown: f32, peak_dx: f32| -> String {
            format!(
                "M{} {side_y} C{} {} {} {crown} {} {crown} C{} {crown} {} {} {} {side_y}",
                hl - o,
                hl - o,
                ht + 20.0,
                hl + 14.0 - o,
                cx + peak_dx,
                hr - 14.0 + o,
                hr + o,
                ht + 20.0,
                hr + o,
            )
        };

        match hair_st {
            0 => {
                // Short crop
                let crown = ht - 2.0;
                let hli = ht + 24.0;
                hairline = Some(hli);
                let outer = skull_edge(1.0, crown, 0.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hr - 3.5, hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5,
                ));
            }
            1 => {
                // Side part — crown volume swept to one side
                let crown = ht - 6.0;
                let hli = ht + 23.0;
                hairline = Some(hli);
                let outer = skull_edge(1.0, crown, -8.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hr - 3.5, hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5,
                ));
                // Part line
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{} {} {} {}" stroke="{hair_dk}" stroke-width="1.2" fill="none" filter="url(#b1)" opacity="0.5"/>"#,
                    cx - 16.0, crown + 3.0, cx - 14.0, ht + 12.0, cx - 12.0, hli,
                ));
            }
            2 => {
                // Medium textured volume
                let crown = ht - 10.0;
                let hli = ht + 22.0;
                hairline = Some(hli);
                let outer = skull_edge(3.0, crown, 0.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hr - 3.5, hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5,
                ));
            }
            3 => {
                // Buzz cut — scalp speckle, like heavy stubble; clipped to the
                // head so no speckle floats past the silhouette
                let crown = ht - 1.0;
                let hli = ht + 24.0;
                hairline = Some(hli);
                s.push_str(r#"<g clip-path="url(#hc)">"#);
                let outer = skull_edge(0.5, crown, 0.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="{hair}" filter="url(#stb)" opacity="0.60"/>"#,
                    hr - 3.5, hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5,
                ));
                s.push_str("</g>");
            }
            4 => {
                // Swept back
                let crown = ht - 12.0;
                let hli = ht + 20.0;
                hairline = Some(hli);
                let outer = skull_edge(2.0, crown, 0.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hr - 3.5, hr - 4.0, rec_y - 2.0, hr - 14.0,
                    hl + 14.0, hl + 4.0, rec_y - 2.0, hl + 3.5,
                ));
            }
            5 => {
                // Afro — ball wrapping the skull; the bottom edge sags to the
                // hairline curve so no skin gap opens at the temples
                let hli = ht + 22.0;
                hairline = Some(hli);
                // Corners tuck in at the temples; outward control points put
                // the widest bulge at mid-height, not the bottom edge
                let left = hl + 1.0;
                let right = hr - 1.0;
                let top = ht - 34.0;
                let bot = ht + 26.0;
                let kw = (cx - left) * 0.72;
                let kh = (bot - top) * 0.60;
                let ball = format!(
                    "M{left} {bot} C{} {} {} {top} {cx} {top} C{} {top} {} {} {right} {bot} Q{cx} {} {left} {bot}Z",
                    left - 10.0,
                    bot - kh,
                    cx - kw,
                    cx + kw,
                    right + 10.0,
                    bot - kh,
                    hli + 14.0,
                );
                // One displaced group: ball + curl speckle share the same
                // wobbled silhouette, so the texture never spills past the edge
                s.push_str(r#"<g filter="url(#hfx)">"#);
                s.push_str(&format!(r#"<path d="{ball}" fill="url(#hg)"/>"#));
                s.push_str(&format!(
                    r#"<path d="{ball}" fill="{hair_dk}" filter="url(#stb)" opacity="0.40"/>"#,
                ));
                s.push_str("</g>");
            }
            6 => {
                // Bald — scalp sheen only
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="22" ry="13" fill="{skin_hi2}" filter="url(#b3)" opacity="0.20"/>"#,
                    cx + 2.0,
                    ht + 14.0,
                ));
            }
            7 => {
                // Curly top — dome with curl lobes straddling the outer edge
                // so the silhouette itself reads bumpy
                let crown = ht - 9.0;
                let hli = ht + 22.0;
                hairline = Some(hli);
                let outer = skull_edge(2.0, crown, 0.0);
                let dome = format!(
                    "{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z",
                    hr - 3.5,
                    hr - 4.0,
                    rec_y,
                    hr - 16.0,
                    hl + 16.0,
                    hl + 4.0,
                    rec_y,
                    hl + 3.5,
                );
                s.push_str(&format!(
                    r#"<path d="{dome}" fill="url(#hg)" filter="url(#htx)"/>"#,
                ));
                // rel stays within ±0.72: past that the ellipse approximation
                // diverges from the dome path and lobes float off the head
                let half_w = (hr - hl) / 2.0;
                for k in 0..7 {
                    let rel = -0.72 + 1.44 * (k as f32 / 6.0);
                    let bx = cx + rel * (half_w - 1.0);
                    let dome_y = crown + (1.0 - (1.0 - rel * rel).sqrt()) * (side_y - crown);
                    let rr = 3.6 + jit(k, 6) * 2.2;
                    s.push_str(&format!(
                        r#"<circle cx="{bx:.1}" cy="{:.1}" r="{rr:.1}" fill="{hair}" filter="url(#htx)"/>"#,
                        dome_y + jit(k, 8) * 1.2,
                    ));
                }
                // Curl texture inside the mass
                s.push_str(&format!(
                    r#"<path d="{dome}" fill="{hair_dk}" filter="url(#stb)" opacity="0.35"/>"#,
                ));
            }
            8 => {
                // Long — crown dome + slim curtains hugging the face sides
                // (back mass drawn earlier, behind the head)
                let crown = ht - 8.0;
                let hli = ht + 21.0;
                hairline = Some(hli);
                s.push_str(&format!(
                    r#"<path d="M{} {} C{} {} {} {crown} {cx} {crown} C{} {crown} {} {} {} {} L{} {} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hl - 4.0, cy_cheek - 2.0,
                    hl - 4.0, ht + 18.0, hl + 10.0,
                    hr - 10.0, hr + 4.0, ht + 18.0, hr + 4.0, cy_cheek - 2.0,
                    hr - 3.5, cy_cheek - 2.0,
                    hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5, cy_cheek - 2.0,
                ));
                // Strand lines break the flat curtain mass
                for (bi, sidef) in [-1.0f32, 1.0].into_iter().enumerate() {
                    let base = if sidef < 0.0 { hl } else { hr };
                    for k in 0..3 {
                        let x_top = base - sidef * (2.0 + k as f32 * 3.0 + jit(k, bi) * 1.4);
                        s.push_str(&format!(
                            r#"<path d="M{x_top:.1} {} Q{:.1} {} {:.1} {}" stroke="{hair_dk}" stroke-width="0.9" fill="none" stroke-linecap="round" filter="url(#b1)" opacity="0.28"/>"#,
                            ht + 15.0 + k as f32 * 1.5,
                            x_top - sidef * 2.0,
                            (ht + cy_cheek) / 2.0,
                            x_top + sidef * 2.0,
                            cy_cheek - 4.0,
                        ));
                    }
                    let hi_x = base - sidef * 6.0;
                    s.push_str(&format!(
                        r#"<path d="M{hi_x:.1} {} Q{:.1} {} {:.1} {}" stroke="{hair_hi}" stroke-width="0.8" fill="none" stroke-linecap="round" filter="url(#b1)" opacity="0.22"/>"#,
                        ht + 17.0,
                        hi_x - sidef * 2.5,
                        (ht + cy_cheek) / 2.0,
                        hi_x + sidef * 1.5,
                        cy_cheek - 6.0,
                    ));
                }
            }
            9 => {
                // Fade — solid top, speckled sides
                let crown = ht - 7.0;
                let hli = ht + 22.0;
                hairline = Some(hli);
                let cut_l = hl + 7.0;
                let cut_r = hr - 7.0;
                s.push_str(&format!(
                    r#"<path d="M{cut_l} {} C{cut_l} {} {} {crown} {cx} {crown} C{} {crown} {cut_r} {} {cut_r} {} L{cut_r} {} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {cut_l} {}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    ht + 30.0, ht + 4.0, cut_l + 9.0,
                    cut_r - 9.0, ht + 4.0, ht + 30.0,
                    ht + 30.0, cut_r - 2.0, rec_y, cut_r - 12.0,
                    cut_l + 12.0, cut_l + 2.0, rec_y, ht + 30.0,
                ));
                // Faded sides — shaved hair sits ON the scalp, so clip to the
                // head silhouette instead of floating past it
                s.push_str(r#"<g clip-path="url(#hc)">"#);
                for (fx0, fx1) in [(hl - 0.5, cut_l + 2.0), (cut_r - 2.0, hr + 0.5)] {
                    s.push_str(&format!(
                        r#"<path d="M{fx0} {side_y} Q{fx0} {} {} {} Q{fx1} {} {fx1} {}Z" fill="{hair}" filter="url(#stb)" opacity="0.45"/>"#,
                        ht + 18.0,
                        (fx0 + fx1) / 2.0,
                        ht + 14.0,
                        ht + 12.0,
                        side_y,
                    ));
                }
                // Transition band melds the solid top into the shaved sides
                for bx in [cut_l + 1.0, cut_r - 1.0] {
                    s.push_str(&format!(
                        r#"<path d="M{bx} {} L{bx} {}" stroke="{hair_dk}" stroke-width="3" fill="none" filter="url(#b2)" opacity="0.30"/>"#,
                        ht + 16.0,
                        side_y - 6.0,
                    ));
                }
                s.push_str("</g>");
            }
            10 => {
                // Faux-hawk — raised centre, tightly faded sides
                let crown = ht - 8.0;
                let strip_l = cx - 21.0;
                let strip_r = cx + 21.0;
                let hli = ht + 24.0;
                s.push_str(&format!(
                    r#"<path d="M{strip_l} {} C{strip_l} {} {} {crown} {cx} {crown} C{} {crown} {strip_r} {} {strip_r} {} L{strip_r} {hli} Q{cx} {} {strip_l} {hli}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    ht + 26.0, ht + 2.0, strip_l + 12.0,
                    strip_r - 12.0, ht + 2.0, ht + 26.0,
                    hli - 5.0,
                ));
                // Tight faded sides — shaved hair on the scalp, clipped to the
                // head; the bare gap between fade and strip is how a real
                // high fade looks
                s.push_str(r#"<g clip-path="url(#hc)">"#);
                for (fx0, fx1) in [(hl - 0.5, hl + 10.0), (hr - 10.0, hr + 0.5)] {
                    s.push_str(&format!(
                        r#"<path d="M{fx0} {} Q{fx0} {} {} {} Q{fx1} {} {fx1} {}Z" fill="{hair}" filter="url(#stb)" opacity="0.48"/>"#,
                        side_y - 10.0,
                        ht + 14.0,
                        (fx0 + fx1) / 2.0,
                        ht + 10.0,
                        ht + 16.0,
                        side_y - 10.0,
                    ));
                }
                s.push_str("</g>");
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="2" fill="none" filter="url(#b3)" opacity="0.12"/>"#,
                    strip_l + 3.0,
                    hli + 5.0,
                    hli + 1.0,
                    strip_r - 3.0,
                    hli + 5.0,
                ));
            }
            _ => {
                // Cornrows — tight dome with braided row lines
                let crown = ht - 4.0;
                let hli = ht + 23.0;
                hairline = Some(hli);
                let outer = skull_edge(1.0, crown, 0.0);
                s.push_str(&format!(
                    r#"<path d="{outer} L{} {side_y} C{} {} {} {hli} {cx} {hli} C{} {hli} {} {} {} {side_y}Z" fill="url(#hg)" filter="url(#htx)"/>"#,
                    hr - 3.5, hr - 4.0, rec_y, hr - 16.0,
                    hl + 16.0, hl + 4.0, rec_y, hl + 3.5,
                ));
                // Braid lines stay inside the hair mass — stop at the hairline
                for k in 0..7 {
                    let rx_off = (k as f32 - 3.0) * 6.5;
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{} {} {} {}" stroke="{hair_dk}" stroke-width="0.7" fill="none" filter="url(#b1)" opacity="0.30"/>"#,
                        cx + rx_off, crown + 2.0,
                        cx + rx_off * 0.97, (crown + hli) / 2.0,
                        cx + rx_off * 0.92, hli - 2.0,
                    ));
                }
            }
        }

        // Hairline cast shadow + wispy edge strands — sells the transition
        if let Some(hli) = hairline {
            let shadow_op = if hair_st == 3 { 0.09 } else { 0.15 };
            s.push_str(&format!(
                r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="2.6" fill="none" filter="url(#b3)" opacity="{shadow_op}"/>"#,
                hl + 7.0,
                hli + 6.0,
                hli + 1.0,
                hr - 7.0,
                hli + 6.0,
            ));
        }
    }

    // Close the tilted head group
    s.push_str("</g>");

    // ── Jersey / shoulders ──────────────────────────────────
    let jersey_d = "M8 250 C20 224 48 216 76 213 Q100 208 124 213 C152 216 180 224 192 250Z";
    s.push_str(&format!(
        r#"<defs><clipPath id="jc"><path d="{jersey_d}"/></clipPath></defs>"#
    ));
    s.push_str(&format!(r#"<path d="{jersey_d}" fill="url(#jg)"/>"#));
    s.push_str(r#"<g clip-path="url(#jc)">"#);
    // Head cast shadow onto the chest
    s.push_str(&format!(
        r##"<ellipse cx="{cx}" cy="220" rx="30" ry="10" fill="#000" filter="url(#b3)" opacity="0.30"/>"##,
    ));
    // Fabric folds
    for (fx, fy) in [(cx - 30.0, 232.0f32), (cx + 28.0, 234.0)] {
        s.push_str(&format!(
            r#"<path d="M{fx} {fy} Q{} {} {} 250" stroke="{jersey_dark}" stroke-width="3" fill="none" filter="url(#b2)" opacity="0.5"/>"#,
            fx + 3.0,
            fy + 8.0,
            fx + 1.0,
        ));
    }
    s.push_str("</g>");
    // Crew collar
    s.push_str(&format!(
        r#"<path d="M{} 219 Q{cx} 231 {} 219 L{} 225 Q{cx} 238 {} 225Z" fill="{jersey_dark}" opacity="0.92"/>"#,
        cx - 24.0,
        cx + 24.0,
        cx + 26.0,
        cx - 26.0,
    ));

    // ── Vignette ────────────────────────────────────────────
    s.push_str(r#"<rect width="200" height="250" fill="url(#vig)"/>"#);

    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dev-only contact sheet: writes one SVG file per face (inline SVGs in a
    /// single HTML document would collide on gradient/filter ids) plus a
    /// faces.html grid into $FACE_PREVIEW_DIR for visual review.
    /// Run with:
    ///   FACE_PREVIEW_DIR=<dir> cargo test -p web --lib preview_contact_sheet -- --ignored
    #[test]
    #[ignore]
    fn preview_contact_sheet() {
        let Ok(dir) = std::env::var("FACE_PREVIEW_DIR") else {
            return;
        };
        let root = std::path::Path::new(&dir);

        let dists = [
            (
                "white",
                SkinDist {
                    white: 100,
                    black: 0,
                    _metis: 0,
                },
            ),
            (
                "metis",
                SkinDist {
                    white: 0,
                    black: 0,
                    _metis: 100,
                },
            ),
            (
                "black",
                SkinDist {
                    white: 0,
                    black: 100,
                    _metis: 0,
                },
            ),
        ];
        let ages: [u8; 5] = [17, 21, 26, 31, 36];

        let mut html = String::with_capacity(1 << 16);
        html.push_str(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
             body{background:#222;color:#ccc;font:12px sans-serif;margin:12px}\
             .row{display:flex;gap:6px;margin-bottom:6px;align-items:flex-end}\
             .cell{text-align:center}\
             .cell img{width:150px;height:auto;border-radius:6px}\
             .small img{width:44px}\
             h2{color:#eee;margin:14px 0 6px}\
             </style></head><body>",
        );

        for (dist_name, dist) in dists {
            html.push_str(&format!("<h2>{dist_name}</h2>"));
            for age in ages {
                html.push_str("<div class=\"row\">");
                for i in 0..8u32 {
                    let player_id = 2_000_000_000u32 + age as u32 * 1000 + i * 77 + 13;
                    // Sweep the build axis across each row: lean → heavy
                    let heft = -1.6 + i as f32 * 0.5;
                    let svg = generate_face_svg(player_id, age, dist, heft);
                    let fname = format!("face_{dist_name}_{age}_{i}.svg");
                    std::fs::write(root.join(&fname), svg).expect("write face svg");
                    html.push_str(&format!(
                        "<div class=\"cell\"><img src=\"{fname}\"><div>age {age} #{i}</div></div>"
                    ));
                }
                html.push_str("</div>");
            }
            // Avatar-size row — the faces must still read at list size
            html.push_str("<div class=\"row small\">");
            for i in 0..8u32 {
                html.push_str(&format!(
                    "<div class=\"cell\"><img src=\"face_{dist_name}_26_{i}.svg\"></div>"
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</body></html>");

        std::fs::write(root.join("faces.html"), html).expect("write contact sheet");
    }
}
