use serde::Deserialize;
use std::sync::OnceLock;

// ── Skin color distribution ───────────────────────────────────

#[derive(Clone, Copy)]
pub struct SkinDist {
    pub white: u8,
    pub black: u8,
    pub metis: u8,
}

impl Default for SkinDist {
    fn default() -> Self {
        SkinDist { white: 50, black: 20, metis: 30 }
    }
}

#[derive(Deserialize)]
struct CountryJson {
    code: String,
    #[serde(default)]
    skin_colors: Option<SkinColorsJson>,
}

#[derive(Deserialize)]
struct SkinColorsJson {
    white: u8,
    black: u8,
    metis: u8,
}

static SKIN_MAP: OnceLock<Vec<(String, SkinDist)>> = OnceLock::new();

fn load_skin_map() -> Vec<(String, SkinDist)> {
    let json_str = include_str!("../../../database/src/data/countries.json");
    let countries: Vec<CountryJson> = serde_json::from_str(json_str).unwrap_or_default();
    countries.into_iter().map(|c| {
        let dist = c.skin_colors.map(|sc| SkinDist {
            white: sc.white,
            black: sc.black,
            metis: sc.metis,
        }).unwrap_or_default();
        (c.code, dist)
    }).collect()
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
        if s == 0 { s = 1; }
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
    "#FCEBD5", "#F5D6B8", "#EDCBA0", "#DEB887",
    "#D4A373", "#C49064", "#AD7A52", "#96663D",
    "#7B5139", "#5E3A27", "#47291B", "#3B2014",
];

const HAIR: [&str; 10] = [
    "#0A0A0A", "#1A1209", "#2C1B0E", "#4A3728",
    "#6B4F35", "#8B7355", "#A89070", "#C4A67A",
    "#D4B896", "#8B2500",
];

const EYES: [&str; 8] = [
    "#3D2B1F", "#5B4332", "#6B5B45", "#2C4A6E",
    "#3B6B4E", "#5A7463", "#7B7F84", "#4A6B8A",
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
    // Natural lip tone — darker, less saturated than skin
    rgb_hex(
        ((r as f32 * 0.88) + 10.0).min(255.0) as u8,
        (g as f32 * 0.68).min(255.0) as u8,
        (b as f32 * 0.62).min(255.0) as u8,
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
    // cheek_w <= temple_w always — face tapers smoothly from forehead down
    match variant {
        0 => FaceShape { // Oval
            head_top: 16.0, temple_w: 22.5 + fw, cheek_w: 21.0 + fw * 0.8,
            cheek_y: 49.0, jaw_w: 17.5 + fw * 0.7, jaw_y: 69.0,
            chin_w: 7.0 + fw * 0.3, chin_y: 80.0, chin_round: 4.5,
        },
        1 => FaceShape { // Square
            head_top: 16.0, temple_w: 23.0 + fw, cheek_w: 21.5 + fw * 0.8,
            cheek_y: 47.0, jaw_w: 21.0 + fw * 0.9, jaw_y: 70.0,
            chin_w: 11.0 + fw * 0.5, chin_y: 79.0, chin_round: 2.5,
        },
        2 => FaceShape { // Round
            head_top: 15.0, temple_w: 23.5 + fw, cheek_w: 22.0 + fw * 0.9,
            cheek_y: 50.0, jaw_w: 19.5 + fw * 0.9, jaw_y: 70.0,
            chin_w: 9.5 + fw * 0.4, chin_y: 80.0, chin_round: 6.0,
        },
        3 => FaceShape { // Heart
            head_top: 15.5, temple_w: 23.5 + fw, cheek_w: 21.0 + fw * 0.7,
            cheek_y: 48.0, jaw_w: 15.5 + fw * 0.5, jaw_y: 70.0,
            chin_w: 6.0 + fw * 0.2, chin_y: 81.0, chin_round: 3.5,
        },
        4 => FaceShape { // Oblong
            head_top: 14.0, temple_w: 21.5 + fw, cheek_w: 20.5 + fw * 0.8,
            cheek_y: 48.0, jaw_w: 18.0 + fw * 0.7, jaw_y: 71.0,
            chin_w: 7.5 + fw * 0.3, chin_y: 82.0, chin_round: 3.5,
        },
        _ => FaceShape { // Diamond
            head_top: 15.0, temple_w: 22.0 + fw, cheek_w: 21.0 + fw * 0.8,
            cheek_y: 48.0, jaw_w: 16.0 + fw * 0.5, jaw_y: 70.0,
            chin_w: 6.5 + fw * 0.2, chin_y: 81.0, chin_round: 3.0,
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

/// viewBox = "0 0 200 250" — portrait rectangle, head centered at x=100
pub fn generate_face_svg(player_id: u32, age: u8, skin_dist: SkinDist) -> String {
    let mut r = FaceRng::new(player_id);

    let skin = SKIN[pick_skin_index(&mut r, skin_dist)];
    let hair = HAIR[r.range(HAIR.len())];
    let eye_col = EYES[r.range(EYES.len())];

    let face_var = r.range(6);
    let hair_st = r.range(12);
    let brow_st = r.range(6);
    let eye_st = r.range(5);
    let nose_st = r.range(6);
    let mouth_st = r.range(5);

    // Facial hair by age
    let (bc, mc): (u8, u8) = match age {
        0..=19  => (0, 0),
        20..=24 => (18, 10),
        25..=29 => (40, 30),
        30..=34 => (55, 42),
        _       => (65, 50),
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
        0..=19  => r.frange(-2.0, -0.5),
        20..=24 => r.frange(-1.0, 0.5),
        25..=29 => r.frange(-0.3, 1.5),
        30..=34 => r.frange(0.5, 2.5),
        _       => r.frange(1.5, 3.5),
    };

    let fs = face_shape(face_var, fw);
    let cx = 100.0f32;

    // Derived colors
    let skin_hi = shade(skin, 1.12);
    let skin_mid = skin.to_string();
    let skin_dk = shade(skin, 0.84);
    let skin_dk2 = shade(skin, 0.72);
    let skin_shadow = shade(skin, 0.60);
    let bg_color = "#2B3640";

    // Jersey colors (deterministic from player_id)
    let jersey_hue = (player_id * 137) % 360;
    let jersey_color = format!("hsl({}, 60%, 40%)", jersey_hue);
    let jersey_light = format!("hsl({}, 55%, 52%)", jersey_hue);

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

    // Scale everything 2.5x from old 80x100 coordinate space
    let s2 = |v: f32| v * 2.5;

    let mut s = String::with_capacity(16000);

    // ── SVG open + defs ─────────────────────────────────────
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 250">"#);

    s.push_str(&format!(
        r#"<defs>
        <radialGradient id="sg" cx="46%" cy="32%" r="58%">
            <stop offset="0%" stop-color="{skin_hi}"/>
            <stop offset="40%" stop-color="{skin_mid}"/>
            <stop offset="85%" stop-color="{skin_dk}"/>
            <stop offset="100%" stop-color="{skin_dk2}"/>
        </radialGradient>
        <linearGradient id="nsg" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="{skin_shadow}" stop-opacity="0"/>
            <stop offset="80%" stop-color="{skin_shadow}" stop-opacity="0.15"/>
        </linearGradient>
        <radialGradient id="fh" cx="50%" cy="25%" r="50%">
            <stop offset="0%" stop-color="{skin_hi}" stop-opacity="0.15"/>
            <stop offset="100%" stop-color="{skin_hi}" stop-opacity="0"/>
        </radialGradient>
        </defs>"#,
    ));

    // ── Background ──────────────────────────────────────────
    s.push_str(&format!(r#"<rect width="200" height="250" fill="{bg_color}"/>"#));

    // ── Jersey / shoulders ──────────────────────────────────
    s.push_str(&format!(
        r#"<path d="M0 250 Q30 205 70 200 Q100 194 130 200 Q170 205 200 250Z" fill="{jersey_color}"/>"#,
    ));
    // Jersey highlight stripe
    s.push_str(&format!(
        r#"<path d="M85 200 Q100 196 115 200 L112 250 L88 250Z" fill="{jersey_light}" opacity="0.3"/>"#,
    ));
    // Collar
    s.push_str(&format!(
        r#"<path d="M82 200 Q100 194 118 200 Q115 207 100 205 Q85 207 82 200Z" fill="{}" opacity="0.8"/>"#,
        shade(&jersey_color, 0.7)
    ));

    // ── Neck ────────────────────────────────────────────────
    let neck_w = 18.0 + fw;
    s.push_str(&format!(
        r#"<path d="M{} 197 Q100 192 {} 197 L{} 210 L{} 210Z" fill="{}"/>"#,
        cx - neck_w, cx + neck_w, cx + neck_w + 3.0, cx - neck_w - 3.0, skin_mid
    ));
    // Neck shadow
    s.push_str(&format!(
        r#"<path d="M{} 197 Q100 194 {} 197 L{} 204 Q100 201 {} 204Z" fill="{}" opacity="0.15"/>"#,
        cx - neck_w, cx + neck_w, cx + neck_w, cx - neck_w, skin_dk2
    ));
    // Adam's apple hint
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="200" rx="2" ry="1.5" fill="{skin_dk}" opacity="0.06"/>"#,
    ));

    // ── Ears ────────────────────────────────────────────────
    let ear_y = s2(fs.cheek_y) - 5.0;
    let ear_lx = cx - s2(fs.cheek_w) - 5.0;
    let ear_rx = cx + s2(fs.cheek_w) + 5.0;
    let ei = shade(skin, 0.78);
    for ex in [ear_lx, ear_rx] {
        s.push_str(&format!(
            r#"<ellipse cx="{ex}" cy="{ear_y}" rx="7" ry="13" fill="{skin_mid}"/>"#,
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{ear_y}" rx="4" ry="8" fill="{ei}" opacity="0.4"/>"#,
            if ex < cx { ex + 1.5 } else { ex - 1.5 }
        ));
        // Earlobe
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="3" ry="3.5" fill="{skin_mid}" opacity="0.7"/>"#,
            if ex < cx { ex + 1.0 } else { ex - 1.0 }, ear_y + 10.0
        ));
    }

    // ── Head shape ──────────────────────────────────────────
    let hl = cx - s2(fs.temple_w);
    let hr = cx + s2(fs.temple_w);
    let cl = cx - s2(fs.cheek_w);
    let cr = cx + s2(fs.cheek_w);
    let jl = cx - s2(fs.jaw_w) + ax * 2.0;
    let jr = cx + s2(fs.jaw_w) - ax;
    let chl = cx - s2(fs.chin_w);
    let chr = cx + s2(fs.chin_w);
    let ht = s2(fs.head_top);
    let cy_cheek = s2(fs.cheek_y);
    let jy = s2(fs.jaw_y);
    let chy = s2(fs.chin_y);
    let cr_val = s2(fs.chin_round);

    // Smooth face outline — cubic beziers for gentle temple→cheek→jaw flow
    // No sharp cheekbone angle: the curve flows continuously
    let mid_r = (cr + jr) / 2.0; // midpoint between cheek and jaw on right
    let mid_l = (cl + jl) / 2.0; // midpoint between cheek and jaw on left
    let mid_y = (cy_cheek + jy) / 2.0;

    s.push_str(&format!(
        r#"<path d="
            M{hl} {cy_cheek}
            C{hl} {} {} {ht} {cx} {ht}
            C{} {ht} {hr} {} {hr} {cy_cheek}
            C{hr} {} {mid_r} {mid_y} {jr} {jy}
            Q{} {} {chr} {chy}
            Q{cx} {} {chl} {chy}
            Q{} {} {jl} {jy}
            C{mid_l} {mid_y} {hl} {} {hl} {cy_cheek}Z
        " fill="url(#sg)"/>"#,
        ht + 22.0, hl + 14.0,
        hr - 14.0, ht + 22.0,
        jy - 10.0,
        jr - cr_val, jy + cr_val,
        chy + cr_val,
        jl + cr_val, jy + cr_val,
        jy - 10.0,
    ));

    // ── Facial lighting layers ─────────────────────────────
    // Forehead highlight (top-left light source)
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="28" ry="16" fill="url(#fh)"/>"#,
        cx - 2.0, ht + 30.0
    ));

    // Cheekbone highlights — gives structure to the face
    let cheek_hi = blend(skin, "#F8E8D8", 0.20);
    for chx in [cx - 24.0, cx + 24.0] {
        s.push_str(&format!(
            r#"<ellipse cx="{chx}" cy="{}" rx="10" ry="6" fill="{cheek_hi}" opacity="0.10"/>"#,
            cy_cheek + 2.0
        ));
    }

    // Cheek warmth (subtle blush below cheekbone)
    let blush = blend(skin, "#D89080", 0.12);
    for chx in [cx - 26.0, cx + 26.0] {
        s.push_str(&format!(
            r#"<ellipse cx="{chx}" cy="{}" rx="12" ry="8" fill="{blush}" opacity="0.10"/>"#,
            cy_cheek + 10.0
        ));
    }

    // Temple shadows — more visible
    for tx in [hl + 8.0, hr - 8.0] {
        s.push_str(&format!(
            r#"<ellipse cx="{tx}" cy="{}" rx="10" ry="20" fill="{skin_dk2}" opacity="0.09"/>"#,
            cy_cheek - 8.0
        ));
    }

    // Jaw shadow — more defined
    s.push_str(&format!(
        r#"<path d="M{jl} {jy} Q{cx} {} {jr} {jy} Q{cx} {} {jl} {jy}Z" fill="{skin_dk2}" opacity="0.10"/>"#,
        chy - 4.0, chy + 2.0
    ));

    // Under-chin shadow
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="20" ry="6" fill="{skin_shadow}" opacity="0.08"/>"#,
        chy + 4.0
    ));

    // Nasolabial folds
    if wrinkle_opacity > 0.0 {
        s.push_str(&format!(
            r#"<path d="M{} 139 Q{} 158 {} 172" stroke="{skin_dk2}" stroke-width="0.6" fill="none" opacity="{}"/>"#,
            cx - 20.0, cx - 24.0, cx - 22.0, wrinkle_opacity
        ));
        s.push_str(&format!(
            r#"<path d="M{} 139 Q{} 158 {} 172" stroke="{skin_dk2}" stroke-width="0.6" fill="none" opacity="{}"/>"#,
            cx + 20.0, cx + 24.0, cx + 22.0, wrinkle_opacity
        ));
    }

    // Forehead wrinkles
    if wrinkle_opacity > 0.06 {
        for wy in [72.0f32, 78.0, 84.0] {
            s.push_str(&format!(
                r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="0.4" fill="none" opacity="{}"/>"#,
                cx - 18.0, wy, wy - 1.5, cx + 18.0, wy, wrinkle_opacity * 0.5
            ));
        }
    }

    // ── Brow ridge shadow ──────────────────────────────────
    {
        let brow_y = 105.0 + ay;
        // Horizontal shadow across brow bone — gives depth to the eye sockets
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="3.5" fill="none" opacity="0.08" stroke-linecap="round"/>"#,
            cx - 35.0, brow_y + 1.0, brow_y - 2.0, cx + 35.0, brow_y + 1.0
        ));
        // Inner eye socket shadows
        for sx in [cx - 19.0, cx + 19.0] {
            s.push_str(&format!(
                r#"<ellipse cx="{sx}" cy="{}" rx="14" ry="9" fill="{skin_dk2}" opacity="0.06"/>"#,
                brow_y + 12.0
            ));
        }
    }

    // ── Eyes (almond-shaped, realistic) ─────────────────────
    {
        let ey = 117.0 + ay * 2.0;
        let lx = cx - 19.0 + ax * 1.5;
        let rx_e = cx + 19.0 - ax;
        // Muted sclera — real eyes are off-white, not bright white
        let sclera = "#EAE6E1";

        // Smaller eyes with more height variation
        let (erx, ery, iris_r, pupil_r): (f32, f32, f32, f32) = match eye_st {
            0 => (10.5, 4.8, 4.5, 1.8),   // narrow
            1 => (10.0, 4.2, 4.2, 1.7),   // small
            2 => (11.0, 5.5, 4.8, 1.9),   // round-ish
            3 => (9.8, 4.5, 4.3, 1.7),    // squinting
            _ => (10.8, 5.0, 4.6, 1.8),   // medium
        };

        let lid_col = shade(skin, 0.72);
        let lash_col = shade(skin, 0.45);
        let iris_rim = shade(eye_col, 0.55);
        let iris_hi = shade(eye_col, 1.25);

        for (i, (ex, side)) in [(lx, -1.0f32), (rx_e, 1.0)].iter().enumerate() {
            let clip_id = format!("ec{i}");

            // Almond-shaped eye path — pointed at inner/outer corners
            let inner_x = ex - erx * side; // inner corner (near nose)
            let outer_x = ex + erx * side; // outer corner
            let el = ex - erx;
            let er = ex + erx;

            // Sclera — almond shape using cubic beziers
            s.push_str(&format!(
                r#"<path d="M{el} {ey} Q{} {} {ex} {} Q{} {} {er} {ey} Q{} {} {ex} {} Q{} {} {el} {ey}Z" fill="{sclera}"/>"#,
                el + erx * 0.3, ey - ery * 0.8, ey - ery,
                ex + erx * 0.7, ey - ery * 0.8,
                er - erx * 0.3, ey + ery * 0.5, ey + ery * 0.6,
                ex - erx * 0.7, ey + ery * 0.5,
            ));

            // Clip path matches the almond shape
            s.push_str(&format!(
                r#"<clipPath id="{clip_id}"><path d="M{el} {ey} Q{} {} {ex} {} Q{} {} {er} {ey} Q{} {} {ex} {} Q{} {} {el} {ey}Z"/></clipPath>"#,
                el + erx * 0.3, ey - ery * 0.8, ey - ery,
                ex + erx * 0.7, ey - ery * 0.8,
                er - erx * 0.3, ey + ery * 0.5, ey + ery * 0.6,
                ex - erx * 0.7, ey + ery * 0.5,
            ));

            s.push_str(&format!(r#"<g clip-path="url(#{clip_id})">"#));

            // Upper sclera shadow (eyelid shadow cast onto eye)
            s.push_str(&format!(
                r#"<ellipse cx="{ex}" cy="{}" rx="{erx}" ry="3.5" fill="{lid_col}" opacity="0.25"/>"#,
                ey - ery + 1.5
            ));

            // Iris — outer ring
            s.push_str(&format!(
                r#"<circle cx="{ex}" cy="{ey}" r="{iris_r}" fill="{iris_rim}"/>"#,
            ));
            // Iris — main color
            s.push_str(&format!(
                r#"<circle cx="{ex}" cy="{ey}" r="{}" fill="{eye_col}"/>"#,
                iris_r * 0.78
            ));
            // Iris — lighter arc detail
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="{iris_hi}" opacity="0.20"/>"#,
                ex - 0.4 * side, ey - 0.6, iris_r * 0.40
            ));
            // Pupil
            s.push_str(&format!(
                r##"<circle cx="{ex}" cy="{ey}" r="{pupil_r}" fill="#0A0A0A"/>"##,
            ));
            // Catchlight
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="1.0" fill="white" opacity="0.55"/>"#,
                ex - 1.2 * side, ey - 1.5
            ));

            s.push_str("</g>");

            // Upper eyelid line — thick lash line
            s.push_str(&format!(
                r#"<path d="M{el} {ey} Q{ex} {} {er} {}" stroke="{lash_col}" stroke-width="1.6" fill="none" stroke-linecap="round"/>"#,
                ey - ery - 1.5, ey - 0.5
            ));
            // Lower lid — very subtle
            s.push_str(&format!(
                r#"<path d="M{} {} Q{ex} {} {} {}" stroke="{lid_col}" stroke-width="0.5" fill="none" opacity="0.25"/>"#,
                el + 2.0, ey + 0.3, ey + ery + 0.5, er - 2.0, ey + 0.3
            ));
            // Eyelid crease
            s.push_str(&format!(
                r#"<path d="M{} {} Q{ex} {} {} {}" stroke="{skin_dk2}" stroke-width="0.6" fill="none" opacity="0.12"/>"#,
                el - 0.5, ey - ery + 2.0, ey - ery - 4.5, er + 0.5, ey - ery + 2.0
            ));

            // Under-eye shadow
            s.push_str(&format!(
                r#"<ellipse cx="{ex}" cy="{}" rx="{}" ry="2.5" fill="{skin_shadow}" opacity="{}"/>"#,
                ey + ery + 3.0, erx - 2.0, undereye_opacity
            ));
        }
    }

    // ── Eyebrows ────────────────────────────────────────────
    {
        let by = 103.0 + ay * 2.0;
        let blx = cx - 19.0 + ax;
        let brx = cx + 19.0 - ax * 0.5;
        let brow_col = shade(hair, 0.90);

        let (bw, bt, arch): (f32, f32, f32) = match brow_st {
            0 => (2.2, 0.0, 3.0),    // straight
            1 => (2.4, -1.0, 5.5),   // arched
            2 => (2.8, 0.5, 2.0),    // flat thick
            3 => (2.0, -0.5, 4.5),   // medium arch
            4 => (3.2, 0.0, 3.5),    // bushy
            _ => (2.0, -0.3, 5.0),   // high arch
        };

        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            blx - 12.0, by + bt, blx, by - arch, blx + 12.0, by + bt * 0.6
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            brx - 12.0, by + bt * 0.6, brx, by - arch * 0.95, brx + 12.0, by + bt
        ));
    }

    // ── Nose (visible, structured) ──────────────────────────
    {
        let ny = 142.0;
        let ns = shade(skin, 0.55);
        let nt = shade(skin, 0.70);
        let nhi = shade(skin, 1.08);

        let (bw, tw, th, nostril_w): (f32, f32, f32, f32) = match nose_st {
            0 => (1.0, 7.5, 3.5, 2.8),   // small straight
            1 => (1.2, 11.0, 5.0, 4.0),  // wide
            2 => (1.1, 9.0, 4.0, 3.2),   // medium
            3 => (0.9, 7.0, 4.5, 2.5),   // narrow
            4 => (1.1, 10.0, 4.5, 3.5),  // aquiline
            _ => (1.0, 8.5, 3.8, 3.0),   // button
        };

        // Bridge — curved lines on both sides from brow to tip
        s.push_str(&format!(
            r#"<path d="M{} 104 Q{} 125 {} {ny}" stroke="{ns}" stroke-width="{bw}" fill="none" opacity="0.30"/>"#,
            cx - 3.5, cx - 4.0, cx - 2.0
        ));
        s.push_str(&format!(
            r#"<path d="M{} 104 Q{} 125 {} {ny}" stroke="{ns}" stroke-width="{}" fill="none" opacity="0.20"/>"#,
            cx + 3.5, cx + 4.0, cx + 2.0, bw * 0.7
        ));
        // Bridge highlight (light catches center ridge)
        s.push_str(&format!(
            r#"<path d="M{} 108 L{} {}" stroke="{nhi}" stroke-width="1.5" fill="none" opacity="0.12"/>"#,
            cx + 0.5, cx + 0.3, ny - 5.0
        ));
        // Nose tip — visible shape
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{ny}" rx="{tw}" ry="{th}" fill="{nt}" opacity="0.20"/>"#,
        ));
        // Tip highlight
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="2.5" ry="1.8" fill="{nhi}" opacity="0.14"/>"#,
            cx + 0.8, ny - 0.8
        ));
        // Nostrils — clearly visible
        let nl = cx - tw * 0.50;
        let nr = cx + tw * 0.50;
        s.push_str(&format!(
            r#"<ellipse cx="{nl}" cy="{}" rx="{nostril_w}" ry="2.0" fill="{ns}" opacity="0.35"/>"#,
            ny + 2.0
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{nr}" cy="{}" rx="{nostril_w}" ry="2.0" fill="{ns}" opacity="0.35"/>"#,
            ny + 2.0
        ));
        // Nose wing shadows (alar creases)
        for (gx, dir) in [(nl - nostril_w - 0.5, -1.0f32), (nr + nostril_w + 0.5, 1.0)] {
            s.push_str(&format!(
                r#"<path d="M{gx} {} Q{} {} {} {}" stroke="{ns}" stroke-width="0.7" fill="none" opacity="0.22"/>"#,
                ny, gx + dir * 1.0, ny + 3.0, gx - dir * 0.5, ny + 5.0
            ));
        }
        // Bottom of nose shadow
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="{}" ry="2.0" fill="{ns}" opacity="0.12"/>"#,
            ny + 4.0, tw * 0.7
        ));
    }

    // ── Mouth (visible, natural) ─────────────────────────────
    {
        let my = 164.0;
        let lp = lip_color(skin);
        let lp_dk = shade(&lp, 0.60);
        let lp_hi = shade(&lp, 1.12);
        let sep = shade(skin, 0.42);

        let (mw, upper_h, lower_h): (f32, f32, f32) = match mouth_st {
            0 => (14.0, 3.0, 4.5),    // medium
            1 => (16.5, 2.5, 5.5),    // wide
            2 => (12.0, 3.5, 4.0),    // small
            3 => (15.0, 4.0, 5.0),    // full
            _ => (13.0, 2.8, 4.2),    // thin
        };

        let ml = cx - mw;
        let mr = cx + mw;

        // Lip separation line — defines the mouth
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}" stroke="{lp_dk}" stroke-width="0.8" fill="none" opacity="0.55"/>"#,
            my + 0.5
        ));
        // Upper lip — cupid's bow
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{} {} {} {} Q{cx} {} {} {} Q{} {} {mr} {my} L{ml} {my}Z" fill="{lp}" opacity="0.70"/>"#,
            ml + mw * 0.3, my - upper_h * 0.3,
            cx - 3.0, my - upper_h,
            my - upper_h * 0.5,
            cx + 3.0, my - upper_h,
            mr - mw * 0.3, my - upper_h * 0.3,
        ));
        // Lower lip — fuller, lighter
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}Z" fill="{lp}" opacity="0.50"/>"#,
            my + lower_h
        ));
        // Upper lip vermilion border
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{lp_dk}" stroke-width="0.4" fill="none" opacity="0.20"/>"#,
            cx - 4.0, my - upper_h * 0.7, my - upper_h - 0.5, cx + 4.0, my - upper_h * 0.7
        ));
        // Lower lip shine
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="5" ry="1.5" fill="{lp_hi}" opacity="0.12"/>"#,
            my + lower_h * 0.4
        ));
        // Philtrum — subtle but visible
        for px in [cx - 2.2, cx + 2.2] {
            s.push_str(&format!(
                r#"<path d="M{px} 149 L{} {}" stroke="{}" stroke-width="0.45" fill="none" opacity="0.10"/>"#,
                if px < cx { px + 0.3 } else { px - 0.3 }, my - upper_h, shade(skin, 0.62)
            ));
        }
        // Mouth corners — shadow dots
        for mcx in [ml, mr] {
            s.push_str(&format!(
                r#"<circle cx="{mcx}" cy="{my}" r="1.0" fill="{sep}" opacity="0.15"/>"#,
            ));
        }
        // Chin shadow
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="10" ry="3" fill="{skin_dk2}" opacity="0.05"/>"#,
            my + lower_h + 4.0
        ));
    }

    // ── Beard ───────────────────────────────────────────────
    if beard {
        let bd = shade(hair, 0.85);
        match beard_v {
            0 => { // Light stubble
                s.push_str(&format!(
                    r#"<path d="M{} 155 Q{} 188 {cx} 198 Q{} 188 {} 155 Q{cx} 182 {} 155Z" fill="{bd}" opacity="0.08"/>"#,
                    cx - 36.0, cx - 32.0, cx + 32.0, cx + 36.0, cx - 36.0
                ));
            }
            1 => { // Short beard
                s.push_str(&format!(
                    r#"<path d="M{} 150 Q{} 192 {cx} 202 Q{} 192 {} 150 Q{cx} 182 {} 150Z" fill="{bd}" opacity="0.25"/>"#,
                    cx - 40.0, cx - 36.0, cx + 36.0, cx + 40.0, cx - 40.0
                ));
            }
            2 => { // Full beard
                s.push_str(&format!(
                    r#"<path d="M{} 140 Q{} 196 {cx} 210 Q{} 196 {} 140 Q{cx} 186 {} 140Z" fill="{bd}" opacity="0.40"/>"#,
                    cx - 44.0, cx - 40.0, cx + 40.0, cx + 44.0, cx - 44.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{} 155 Q{} 186 {cx} 196 Q{} 186 {} 155Z" fill="{}" opacity="0.06"/>"#,
                    cx - 30.0, cx - 26.0, cx + 26.0, cx + 30.0, shade(hair, 1.15)
                ));
            }
            3 => { // Goatee
                s.push_str(&format!(
                    r#"<path d="M{} 157 Q{} 194 {cx} 206 Q{} 194 {} 157 Q{cx} 178 {} 157Z" fill="{bd}" opacity="0.30"/>"#,
                    cx - 16.0, cx - 14.0, cx + 14.0, cx + 16.0, cx - 16.0
                ));
            }
            _ => { // Chinstrap
                s.push_str(&format!(
                    r#"<path d="M{} 150 Q{} 188 {cx} 198 Q{} 188 {} 150 Q{cx} 175 {} 150Z" fill="{bd}" opacity="0.18"/>"#,
                    cx - 42.0, cx - 38.0, cx + 38.0, cx + 42.0, cx - 42.0
                ));
            }
        }
    }

    // ── Mustache ────────────────────────────────────────────
    if mstache {
        let mc_col = shade(hair, 0.88);
        match mst_v {
            0 => { // Pencil
                s.push_str(&format!(
                    r#"<path d="M{} 161 Q{cx} 159 {} 161" stroke="{mc_col}" stroke-width="1.4" fill="none"/>"#,
                    cx - 13.0, cx + 13.0
                ));
            }
            1 => { // Full
                s.push_str(&format!(
                    r#"<path d="M{} 160 Q{} 156 {cx} 160 Q{} 156 {} 160 Q{} 164 {cx} 163 Q{} 164 {} 160Z" fill="{mc_col}" opacity="0.42"/>"#,
                    cx - 15.0, cx - 8.0, cx + 8.0, cx + 15.0, cx + 11.0, cx - 11.0, cx - 15.0
                ));
            }
            2 => { // Handlebar
                s.push_str(&format!(
                    r#"<path d="M{} 158 Q{cx} 155 {} 158 L{} 163 Q{cx} 161 {} 163Z" fill="{mc_col}" opacity="0.38"/>"#,
                    cx - 18.0, cx + 18.0, cx + 16.0, cx - 16.0
                ));
            }
            _ => { // Chevron
                s.push_str(&format!(
                    r#"<path d="M{} 159 Q{cx} 155 {} 159 Q{cx} 164 {} 159Z" fill="{mc_col}" opacity="0.35"/>"#,
                    cx - 14.0, cx + 14.0, cx - 14.0
                ));
            }
        }
    }

    // ── Hair ────────────────────────────────────────────────
    {
        let hd = shade(hair, 0.80);
        let hl_c = shade(hair, 1.22);

        match hair_st {
            0 => { // Short crop
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {ht} {cx} {ht} C{} {ht} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 14.0, ht + 18.0, hl + 14.0,
                    hr - 14.0, ht + 18.0, cy_cheek - 14.0,
                    ht + 8.0, ht + 14.0, hr - 12.0, ht - 4.0, ht - 4.0,
                    hl + 12.0, ht - 4.0, ht + 14.0, ht + 8.0,
                ));
            }
            1 => { // Side part
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 18.0, hl + 12.0, ht + 4.0, ht + 4.0,
                    hr - 12.0, ht + 4.0, ht + 18.0, cy_cheek - 10.0,
                    ht, ht + 10.0, hr - 10.0, ht - 8.0, ht - 8.0,
                    hl + 10.0, ht - 8.0, ht + 10.0, ht,
                ));
                s.push_str(&format!(
                    r#"<path d="M{} {} L{} {}" stroke="{hd}" stroke-width="0.6" opacity="0.25"/>"#,
                    cx - 16.0, ht + 2.0, cx - 12.0, ht + 18.0
                ));
            }
            2 => { // Medium textured
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 2.0, cy_cheek - 14.0, ht + 18.0, hl + 10.0, ht, ht,
                    hr - 10.0, ht, ht + 18.0, hr + 2.0, cy_cheek - 14.0,
                    hr + 2.0, ht - 4.0, ht + 10.0, hr - 8.0, ht - 14.0, ht - 14.0,
                    hl + 8.0, ht - 14.0, ht + 10.0, hl - 2.0, ht - 4.0,
                ));
            }
            3 => { // Buzz cut
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}" opacity="0.55"/>"#,
                    cy_cheek - 18.0, ht + 22.0, hl + 14.0, ht + 4.0, ht + 4.0,
                    hr - 14.0, ht + 4.0, ht + 22.0, cy_cheek - 18.0,
                    ht + 8.0, ht + 16.0, hr - 12.0, ht, ht,
                    hl + 12.0, ht, ht + 16.0, ht + 8.0,
                ));
            }
            4 => { // Swept back
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 4.0, cy_cheek - 10.0, ht + 14.0, hl + 10.0, ht - 4.0, ht - 4.0,
                    hr - 10.0, ht - 4.0, ht + 14.0, hr + 4.0, cy_cheek - 10.0,
                    hr + 4.0, ht - 14.0, ht + 5.0, hr - 8.0, ht - 22.0, ht - 22.0,
                    hl + 8.0, ht - 22.0, ht + 5.0, hl - 4.0, ht - 14.0,
                ));
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="0.8" fill="none" opacity="0.12"/>"#,
                    hl + 8.0, ht - 8.0, ht - 18.0, hr - 8.0, ht - 8.0
                ));
            }
            5 => { // Afro
                let afro_r = 65.0 + fw * 2.0;
                s.push_str(&format!(
                    r#"<path d="
                        M{} {}
                        C{} {} {} {} {cx} {}
                        C{} {} {} {} {} {}
                        L{hr} {} C{hr} {} {} {} {cx} {}
                        C{} {} {hl} {} {hl} {}Z
                    " fill="{hair}"/>"#,
                    cx - afro_r, cy_cheek - 4.0,
                    cx - afro_r, ht - 18.0, cx - afro_r * 0.6, ht - 36.0, ht - 36.0,
                    cx + afro_r * 0.6, ht - 36.0, cx + afro_r, ht - 18.0, cx + afro_r, cy_cheek - 4.0,
                    cy_cheek - 14.0, ht + 18.0, hr - 14.0, ht + 4.0, ht + 4.0,
                    hl + 14.0, ht + 4.0, ht + 18.0, cy_cheek - 14.0,
                ));
                for dy in [-28.0f32, -18.0, -8.0] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hd}" stroke-width="0.6" fill="none" opacity="0.10"/>"#,
                        cx - 36.0, ht + dy, ht + dy - 3.0, cx + 36.0, ht + dy
                    ));
                }
            }
            6 => { // Bald
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {}" stroke="{hd}" stroke-width="0.6" fill="none" opacity="0.10"/>"#,
                    cy_cheek - 22.0, ht + 18.0, hl + 14.0, ht + 4.0, ht + 4.0,
                    hr - 14.0, ht + 4.0, ht + 18.0, cy_cheek - 22.0,
                ));
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="22" ry="12" fill="{skin_hi}" opacity="0.10"/>"#,
                    cx + 4.0, ht + 18.0
                ));
            }
            7 => { // Curly top
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 14.0, hl + 10.0, ht - 4.0, ht - 4.0,
                    hr - 10.0, ht - 4.0, ht + 14.0, cy_cheek - 10.0,
                    ht - 8.0, ht + 5.0, hr - 8.0, ht - 18.0, ht - 18.0,
                    hl + 8.0, ht - 18.0, ht + 5.0, ht - 8.0,
                ));
                for bx in [-22.0f32, -10.0, 4.0, 18.0] {
                    s.push_str(&format!(
                        r#"<circle cx="{}" cy="{}" r="6.5" fill="{hair}"/>"#,
                        cx + bx, ht - 14.0 + (bx.abs() * 0.12)
                    ));
                }
            }
            8 => { // Long / flowing
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} Q{} {} {} {} L{hl} {} Q{} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 6.0, cy_cheek + 14.0, ht + 14.0, hl + 8.0, ht - 8.0, ht - 8.0,
                    hr - 8.0, ht - 8.0, ht + 14.0, hr + 6.0, cy_cheek + 14.0,
                    hr + 6.0, ht - 18.0, hr - 4.0, ht - 28.0, cx, ht - 28.0,
                    ht - 18.0, hl + 4.0, ht - 28.0, hl - 6.0, ht - 18.0,
                ));
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="0.5" fill="none" opacity="0.10"/>"#,
                    hl + 4.0, ht, ht - 12.0, hr - 4.0, ht
                ));
            }
            9 => { // Fade / undercut
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl + 5.0, cy_cheek - 18.0, ht + 22.0, hl + 14.0, ht, ht,
                    hr - 14.0, ht, ht + 22.0, hr - 5.0, cy_cheek - 18.0,
                    hr - 5.0, ht - 4.0, ht + 14.0, hr - 12.0, ht - 14.0, ht - 14.0,
                    hl + 12.0, ht - 14.0, ht + 14.0, hl + 5.0, ht - 4.0,
                ));
                s.push_str(&format!(
                    r#"<path d="M{hl} {} L{} {} L{hl} {}Z" fill="{hair}" opacity="0.20"/>"#,
                    cy_cheek - 18.0, hl + 5.0, cy_cheek - 18.0, cy_cheek + 4.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} L{} {} L{hr} {}Z" fill="{hair}" opacity="0.20"/>"#,
                    cy_cheek - 18.0, hr - 5.0, cy_cheek - 18.0, cy_cheek + 4.0
                ));
            }
            10 => { // Mohawk / short top
                s.push_str(&format!(
                    r#"<path d="M{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {} L{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {}Z" fill="{hair}"/>"#,
                    cx - 14.0, cy_cheek - 22.0, cx - 14.0, ht + 10.0, cx - 10.0, ht - 4.0, ht - 4.0,
                    cx + 10.0, ht - 4.0, cx + 14.0, ht + 10.0, cx + 14.0, cy_cheek - 22.0,
                    cx + 14.0, ht - 8.0, cx + 12.0, ht + 4.0, cx + 8.0, ht - 14.0, ht - 14.0,
                    cx - 8.0, ht - 14.0, cx - 12.0, ht + 4.0, cx - 14.0, ht - 8.0,
                ));
                // Faded sides
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {} {} L{} {} L{hl} {}Z" fill="{hair}" opacity="0.15"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hl + 14.0, ht + 8.0, cx - 14.0, ht + 8.0,
                    cx - 14.0, cy_cheek - 22.0, cy_cheek + 4.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} C{hr} {} {} {} {} {} L{} {} L{hr} {}Z" fill="{hair}" opacity="0.15"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hr - 14.0, ht + 8.0, cx + 14.0, ht + 8.0,
                    cx + 14.0, cy_cheek - 22.0, cy_cheek + 4.0
                ));
            }
            _ => { // Cornrows / tight braids
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 14.0, hl + 10.0, ht - 2.0, ht - 2.0,
                    hr - 10.0, ht - 2.0, ht + 14.0, cy_cheek - 10.0,
                    ht - 6.0, ht + 8.0, hr - 8.0, ht - 14.0, ht - 14.0,
                    hl + 8.0, ht - 14.0, ht + 8.0, ht - 6.0,
                ));
                // Row lines
                for rx in [-16.0f32, -8.0, 0.0, 8.0, 16.0] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} L{} {}" stroke="{hd}" stroke-width="0.5" opacity="0.20"/>"#,
                        cx + rx, ht - 10.0, cx + rx * 0.8, cy_cheek - 14.0
                    ));
                }
            }
        }
    }

    s.push_str("</svg>");
    s
}
