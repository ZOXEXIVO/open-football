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

// 12 skin tones: very light → very dark with warm/cool undertone variety
// White range: 0-3, Metis range: 3-8, Black range: 8-11
const SKIN: [&str; 12] = [
    "#FAE0C8", "#F5D0A9", "#EDCBA0", "#E0B68B",
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

#[allow(dead_code)]
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
    rgb_hex(
        ((r as f32 * 0.78) + 50.0).min(255.0) as u8,
        (g as f32 * 0.48).min(255.0) as u8,
        (b as f32 * 0.42).min(255.0) as u8,
    )
}

// ── Face shape parameters ───────────────────────────────────

struct FaceShape {
    // cranium
    head_top: f32,     // y of skull top
    temple_w: f32,     // half-width at temple
    // cheeks
    cheek_w: f32,      // half-width at cheekbone
    cheek_y: f32,      // y of widest point
    // jaw
    jaw_w: f32,        // half-width at jaw angle
    jaw_y: f32,        // y of jaw angle
    // chin
    chin_w: f32,       // half-width at chin
    chin_y: f32,       // y of chin bottom
    chin_round: f32,   // curvature
}

fn face_shape(variant: usize, fw: f32) -> FaceShape {
    match variant {
        0 => FaceShape { // Oval — classic proportions
            head_top: 15.0, temple_w: 23.0 + fw, cheek_w: 24.0 + fw * 1.2,
            cheek_y: 48.0, jaw_w: 18.0 + fw * 0.8, jaw_y: 68.0,
            chin_w: 7.0 + fw * 0.3, chin_y: 80.0, chin_round: 4.0,
        },
        1 => FaceShape { // Square — strong jaw
            head_top: 15.0, temple_w: 23.5 + fw, cheek_w: 24.5 + fw * 1.1,
            cheek_y: 46.0, jaw_w: 22.0 + fw * 1.0, jaw_y: 70.0,
            chin_w: 12.0 + fw * 0.5, chin_y: 79.0, chin_round: 2.5,
        },
        2 => FaceShape { // Round — soft features
            head_top: 14.0, temple_w: 24.0 + fw * 1.1, cheek_w: 25.0 + fw * 1.3,
            cheek_y: 50.0, jaw_w: 20.0 + fw * 1.0, jaw_y: 69.0,
            chin_w: 10.0 + fw * 0.5, chin_y: 80.0, chin_round: 6.0,
        },
        _ => FaceShape { // Heart / triangular
            head_top: 14.5, temple_w: 24.5 + fw, cheek_w: 24.0 + fw * 1.2,
            cheek_y: 47.0, jaw_w: 16.0 + fw * 0.6, jaw_y: 69.0,
            chin_w: 6.0 + fw * 0.2, chin_y: 81.0, chin_round: 3.0,
        },
    }
}

// ── Main generator ──────────────────────────────────────────

/// Pick a skin tone index based on country skin color distribution.
/// First rolls white/black/metis, then picks a specific shade within that group.
fn pick_skin_index(r: &mut FaceRng, dist: SkinDist) -> usize {
    let roll = r.range(100) as u8;
    if roll < dist.white {
        // White: indices 0-3
        r.range(4)
    } else if roll < dist.white + dist.black {
        // Black: indices 8-11
        8 + r.range(4)
    } else {
        // Metis: indices 3-8
        3 + r.range(6)
    }
}

/// viewBox = "0 0 80 100" — portrait rectangle, head centered at x=40
pub fn generate_face_svg(player_id: u32, age: u8, skin_dist: SkinDist) -> String {
    let mut r = FaceRng::new(player_id);

    // Pick features using country-based skin distribution
    let skin = SKIN[pick_skin_index(&mut r, skin_dist)];
    let hair = HAIR[r.range(HAIR.len())];
    let eye_col = EYES[r.range(EYES.len())];

    let face_var = r.range(4);
    let hair_st = r.range(10);
    let brow_st = r.range(5);
    let eye_st = r.range(4);
    let nose_st = r.range(5);
    let mouth_st = r.range(4);

    // Facial hair probability by age
    let (bc, mc): (u8, u8) = match age {
        0..=19  => (0, 0),
        20..=24 => (20, 12),
        25..=29 => (45, 35),
        30..=34 => (60, 48),
        _       => (72, 55),
    };
    let beard = bc > 0 && r.chance(bc);
    let mstache = mc > 0 && r.chance(mc);
    let beard_v = r.range(4);
    let mst_v = r.range(3);

    // Micro-asymmetry
    let ax = r.frange(-0.6, 0.6);
    let ay = r.frange(-0.4, 0.4);

    // Face width widens with age
    let fw: f32 = match age {
        0..=19  => r.frange(-2.0, -1.0),
        20..=24 => r.frange(-1.0, 0.5),
        25..=29 => r.frange(-0.5, 1.5),
        30..=34 => r.frange(0.5, 2.5),
        _       => r.frange(1.5, 3.5),
    };

    let fs = face_shape(face_var, fw);

    // Derived colors
    let skin_hi = shade(skin, 1.10);
    let skin_mid = skin.to_string();
    let skin_dk = shade(skin, 0.82);
    let skin_dk2 = shade(skin, 0.70);
    let skin_shadow = shade(skin, 0.58);
    let bg_color = shade(skin, 0.32);

    // Age-dependent features
    let wrinkle_opacity = match age {
        0..=24 => 0.0f32,
        25..=29 => 0.04,
        30..=34 => 0.09,
        35..=37 => 0.14,
        _ => 0.20,
    };
    let undereye_opacity = match age {
        0..=24 => 0.03f32,
        25..=29 => 0.06,
        30..=34 => 0.10,
        _ => 0.15,
    };

    let cx = 40.0; // face center x

    let mut s = String::with_capacity(12000);

    // ── SVG open + defs ─────────────────────────────────────
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 100">"#);

    s.push_str(&format!(
        r#"<defs>
        <radialGradient id="sg" cx="48%" cy="34%" r="62%">
            <stop offset="0%" stop-color="{skin_hi}"/>
            <stop offset="50%" stop-color="{skin_mid}"/>
            <stop offset="100%" stop-color="{skin_dk}"/>
        </radialGradient>
        <radialGradient id="chk" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stop-color="{skin_dk}" stop-opacity="0.12"/>
            <stop offset="100%" stop-color="{skin_dk}" stop-opacity="0"/>
        </radialGradient>
        <linearGradient id="nsg" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="{skin_shadow}" stop-opacity="0"/>
            <stop offset="80%" stop-color="{skin_shadow}" stop-opacity="0.18"/>
        </linearGradient>
        </defs>"#,
    ));

    // ── Background ──────────────────────────────────────────
    s.push_str(&format!(r#"<rect width="80" height="100" fill="{bg_color}"/>"#));

    // ── Shoulders + neck ────────────────────────────────────
    let neck_w = 8.0 + fw * 0.4;
    s.push_str(&format!(
        r#"<path d="M{} 88 Q40 84 {} 88 L{} 100 L{} 100Z" fill="{}"/>"#,
        cx - neck_w, cx + neck_w, cx + neck_w + 2.0, cx - neck_w - 2.0, skin_mid
    ));
    // Neck shadow
    s.push_str(&format!(
        r#"<path d="M{} 88 Q40 86 {} 88 L{} 92 Q40 90 {} 92Z" fill="{}" opacity="0.12"/>"#,
        cx - neck_w, cx + neck_w, cx + neck_w, cx - neck_w, skin_dk2
    ));
    // Shoulders
    s.push_str(&format!(
        r#"<ellipse cx="40" cy="102" rx="38" ry="14" fill="{}"/>"#,
        shade(skin, 0.50)
    ));

    // ── Ears ────────────────────────────────────────────────
    let ear_y = fs.cheek_y - 2.0;
    let ear_lx = cx - fs.cheek_w - 2.5;
    let ear_rx = cx + fs.cheek_w + 2.5;
    let ei = shade(skin, 0.76);
    for ex in [ear_lx, ear_rx] {
        s.push_str(&format!(
            r#"<ellipse cx="{ex}" cy="{ear_y}" rx="3.2" ry="5.5" fill="{skin_mid}"/>"#,
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{ear_y}" rx="1.8" ry="3.5" fill="{ei}" opacity="0.5"/>"#,
            if ex < cx { ex + 0.6 } else { ex - 0.6 }
        ));
    }

    // ── Head shape ──────────────────────────────────────────
    let hl = cx - fs.temple_w;
    let hr = cx + fs.temple_w;
    let cl = cx - fs.cheek_w;
    let cr = cx + fs.cheek_w;
    let jl = cx - fs.jaw_w + ax;
    let jr = cx + fs.jaw_w - ax * 0.5;
    let chl = cx - fs.chin_w;
    let chr = cx + fs.chin_w;
    let ht = fs.head_top;
    let cy_cheek = fs.cheek_y;
    let jy = fs.jaw_y;
    let chy = fs.chin_y;
    let cr_val = fs.chin_round;

    s.push_str(&format!(
        r#"<path d="
            M{hl} {cy_cheek}
            C{hl} {} {} {ht} {cx} {ht}
            C{} {ht} {hr} {} {hr} {cy_cheek}
            C{hr} {} {} {jy} {jr} {jy}
            Q{} {} {chr} {chy}
            Q{cx} {} {chl} {chy}
            Q{} {} {jl} {jy}
            C{} {jy} {hl} {} {hl} {cy_cheek}Z
        " fill="url(#sg)"/>"#,
        ht + 10.0, hl + 6.0,
        hr - 6.0, ht + 10.0,
        jy - 6.0, cr + 2.0,
        jr - cr_val, jy + cr_val,
        chy + cr_val,
        jl + cr_val, jy + cr_val,
        cl - 2.0, jy - 6.0,
    ));

    // ── Facial shadows and highlights ───────────────────────

    // Forehead highlight
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="12" ry="7" fill="{skin_hi}" opacity="0.10"/>"#,
        cx + 1.0, ht + 14.0
    ));

    // Temple shadows
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="5" ry="10" fill="{skin_dk2}" opacity="0.06"/>"#,
        hl + 4.0, cy_cheek - 4.0
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="5" ry="10" fill="{skin_dk2}" opacity="0.06"/>"#,
        hr - 4.0, cy_cheek - 4.0
    ));

    // Cheekbone highlight
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="5" ry="3" fill="{skin_hi}" opacity="0.08"/>"#,
        cx - 11.0, cy_cheek + 2.0
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="5" ry="3" fill="{skin_hi}" opacity="0.06"/>"#,
        cx + 11.0, cy_cheek + 2.0
    ));

    // Jaw shadow
    s.push_str(&format!(
        r#"<path d="M{jl} {jy} Q{cx} {} {jr} {jy} Q{cx} {} {jl} {jy}Z" fill="{skin_dk2}" opacity="0.08"/>"#,
        chy - 2.0, chy + 1.0
    ));

    // Nasolabial folds (age-dependent)
    if wrinkle_opacity > 0.0 {
        s.push_str(&format!(
            r#"<path d="M{} 56 Q{} 64 {} 70" stroke="{skin_dk2}" stroke-width="0.35" fill="none" opacity="{}"/>"#,
            cx - 9.0, cx - 11.0, cx - 10.0, wrinkle_opacity
        ));
        s.push_str(&format!(
            r#"<path d="M{} 56 Q{} 64 {} 70" stroke="{skin_dk2}" stroke-width="0.35" fill="none" opacity="{}"/>"#,
            cx + 9.0, cx + 11.0, cx + 10.0, wrinkle_opacity
        ));
    }

    // Forehead wrinkles
    if wrinkle_opacity > 0.08 {
        for wy in [28.0f32, 31.0, 34.0] {
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.25" fill="none" opacity="{}"/>"#,
                cx - 8.0, wy, cx, wy - 0.8, cx + 8.0, wy, wrinkle_opacity * 0.6
            ));
        }
    }

    // ── Eyes ────────────────────────────────────────────────
    {
        let ey = 47.0 + ay;
        let lx = cx - 8.0 + ax;
        let rx_e = cx + 8.0 - ax * 0.5;
        let sclera = "#F0EDED";

        // Eye opening dimensions by variant
        let (erx, ery, iris_r, pupil_r): (f32, f32, f32, f32) = match eye_st {
            0 => (5.2, 2.6, 2.4, 0.9),  // standard
            1 => (5.0, 2.1, 2.2, 0.85), // narrow
            2 => (5.4, 3.0, 2.6, 0.95), // wide
            _ => (4.8, 2.4, 2.3, 0.88), // deep-set
        };

        let lid_col = shade(skin, 0.74);
        let iris_rim = shade(eye_col, 0.65);
        let iris_hi = shade(eye_col, 1.25);

        for (ex, side) in [(lx, -1.0f32), (rx_e, 1.0)] {
            // Sclera
            s.push_str(&format!(
                r#"<ellipse cx="{ex}" cy="{ey}" rx="{erx}" ry="{ery}" fill="{sclera}"/>"#,
            ));

            // Clip to eye shape
            let clip_id = if side < 0.0 { "ecL" } else { "ecR" };
            s.push_str(&format!(
                r#"<clipPath id="{clip_id}"><ellipse cx="{ex}" cy="{ey}" rx="{erx}" ry="{ery}"/></clipPath>"#
            ));

            s.push_str(&format!(r#"<g clip-path="url(#{clip_id})">"#));

            // Iris outer ring
            s.push_str(&format!(
                r#"<circle cx="{ex}" cy="{ey}" r="{iris_r}" fill="{iris_rim}"/>"#,
            ));
            // Iris main
            s.push_str(&format!(
                r#"<circle cx="{ex}" cy="{ey}" r="{}" fill="{eye_col}"/>"#,
                iris_r * 0.82
            ));
            // Iris inner light
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="{}" fill="{iris_hi}" opacity="0.3"/>"#,
                ex - 0.3, ey - 0.3, iris_r * 0.5
            ));
            // Pupil
            s.push_str(&format!(
                r##"<circle cx="{ex}" cy="{ey}" r="{pupil_r}" fill="#0D0D0D"/>"##,
            ));
            // Catchlight
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="0.55" fill="white" opacity="0.65"/>"#,
                ex - 0.7 * side, ey - 0.7
            ));
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="0.3" fill="white" opacity="0.35"/>"#,
                ex + 0.5 * side, ey + 0.4
            ));

            // Upper lid shadow inside eye
            s.push_str(&format!(
                r#"<ellipse cx="{ex}" cy="{}" rx="{erx}" ry="1.2" fill="{lid_col}" opacity="0.25"/>"#,
                ey - ery + 0.8
            ));

            s.push_str("</g>");

            // Upper eyelid line
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{lid_col}" stroke-width="0.8" fill="none"/>"#,
                ex - erx, ey, ex, ey - ery - 0.8, ex + erx, ey
            ));
            // Lower lash line (subtle)
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.3" fill="none" opacity="0.2"/>"#,
                ex - erx + 1.0, ey + 0.3, ex, ey + ery + 0.2, ex + erx - 1.0, ey + 0.3
            ));
            // Eyelid crease
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {} {}" stroke="{skin_dk2}" stroke-width="0.3" fill="none" opacity="0.10"/>"#,
                ex - erx - 0.5, ey - ery + 0.5, ex, ey - ery - 2.5, ex + erx + 0.5, ey - ery + 0.5
            ));
            // Tear duct
            let td_x = if side < 0.0 { ex + erx - 0.5 } else { ex - erx + 0.5 };
            s.push_str(&format!(
                r##"<circle cx="{td_x}" cy="{}" r="0.6" fill="#E8D4D0" opacity="0.5"/>"##,
                ey + 0.2
            ));

            // Under-eye shadow
            s.push_str(&format!(
                r#"<ellipse cx="{ex}" cy="{}" rx="{}" ry="1.5" fill="{skin_shadow}" opacity="{}"/>"#,
                ey + ery + 1.5, erx - 0.5, undereye_opacity
            ));
        }
    }

    // ── Eyebrows ────────────────────────────────────────────
    {
        let by = 41.0 + ay;
        let blx = cx - 8.0 + ax;
        let brx = cx + 8.0 - ax * 0.5;
        let brow_col = shade(hair, 0.90);

        let (bw, bt, arch): (f32, f32, f32) = match brow_st {
            0 => (1.3, 0.0, 1.5),   // straight
            1 => (1.4, -0.5, 2.8),  // arched
            2 => (1.6, 0.5, 1.0),   // flat thick
            3 => (1.2, -0.3, 2.2),  // medium arch
            _ => (1.8, 0.0, 2.0),   // bushy
        };

        // Left brow
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            blx - 5.5, by + bt, blx, by - arch, blx + 5.5, by + bt * 0.7
        ));
        // Right brow
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            brx - 5.5, by + bt * 0.7, brx, by - arch * 0.95, brx + 5.5, by + bt
        ));
    }

    // ── Nose ────────────────────────────────────────────────
    {
        let ny = 57.0;
        let ns = shade(skin, 0.58);
        let nt = shade(skin, 0.72);
        let nhi = shade(skin, 1.08);

        // Bridge width and tip dimensions
        let (bw, tw, th, nostril_w): (f32, f32, f32, f32) = match nose_st {
            0 => (0.4, 3.5, 1.6, 1.4),  // small straight
            1 => (0.5, 5.0, 2.2, 1.8),  // wide
            2 => (0.45, 4.0, 1.8, 1.5), // medium
            3 => (0.4, 3.2, 2.0, 1.3),  // narrow pointed
            _ => (0.5, 4.5, 2.0, 1.6),  // aquiline
        };

        // Bridge shadow
        s.push_str(&format!(
            r#"<path d="M{cx} 38 L{} {ny}" stroke="{ns}" stroke-width="{bw}" fill="none" opacity="0.18"/>"#,
            cx - 0.3
        ));

        // Bridge highlight
        s.push_str(&format!(
            r#"<path d="M{} 40 L{} {}" stroke="{nhi}" stroke-width="0.6" fill="none" opacity="0.08"/>"#,
            cx + 0.5, cx + 0.3, ny - 2.0
        ));

        // Nose tip
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{ny}" rx="{tw}" ry="{th}" fill="{nt}" opacity="0.12"/>"#,
        ));

        // Tip highlight
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="1.2" ry="0.8" fill="{nhi}" opacity="0.10"/>"#,
            cx + 0.5, ny - 0.5
        ));

        // Nostrils
        let nl = cx - tw * 0.55;
        let nr = cx + tw * 0.55;
        s.push_str(&format!(
            r#"<ellipse cx="{nl}" cy="{}" rx="{nostril_w}" ry="0.8" fill="{ns}" opacity="0.22"/>"#,
            ny + 0.8
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{nr}" cy="{}" rx="{nostril_w}" ry="0.8" fill="{ns}" opacity="0.22"/>"#,
            ny + 0.8
        ));

        // Alar grooves
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{ns}" stroke-width="0.3" fill="none" opacity="0.12"/>"#,
            nl - nostril_w, ny + 0.5, nl - nostril_w - 0.5, ny + 1.5, nl - nostril_w + 0.3, ny + 2.0
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{ns}" stroke-width="0.3" fill="none" opacity="0.12"/>"#,
            nr + nostril_w, ny + 0.5, nr + nostril_w + 0.5, ny + 1.5, nr + nostril_w - 0.3, ny + 2.0
        ));
    }

    // ── Mouth ───────────────────────────────────────────────
    {
        let my = 66.0;
        let lp = lip_color(skin);
        let lp_dk = shade(&lp, 0.72);
        let lp_hi = shade(&lp, 1.15);
        let sep = shade(skin, 0.48);

        let (mw, upper_h, lower_h): (f32, f32, f32) = match mouth_st {
            0 => (6.5, 1.4, 2.0),  // medium
            1 => (7.5, 1.2, 2.4),  // wide
            2 => (5.5, 1.6, 1.8),  // small
            _ => (7.0, 1.8, 2.2),  // full
        };

        let ml = cx - mw;
        let mr = cx + mw;

        // Upper lip with cupid's bow
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{} {} {} {} Q{cx} {} {} {} Q{} {} {mr} {my}" fill="{lp}" opacity="0.65"/>"#,
            ml + mw * 0.3, my - upper_h * 0.3,
            cx - 1.5, my - upper_h,
            my - upper_h * 0.5,
            cx + 1.5, my - upper_h,
            mr - mw * 0.3, my - upper_h * 0.3,
        ));

        // Lower lip
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}" fill="{lp}" opacity="0.45"/>"#,
            my + lower_h
        ));

        // Lip line
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}" stroke="{lp_dk}" stroke-width="0.4" fill="none" opacity="0.5"/>"#,
            my + 0.3
        ));

        // Upper lip highlight
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{lp_hi}" stroke-width="0.3" fill="none" opacity="0.15"/>"#,
            cx - 2.0, my - upper_h * 0.8, my - upper_h, cx + 2.0, my - upper_h * 0.8
        ));

        // Lower lip shine
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="2.5" ry="0.6" fill="{lp_hi}" opacity="0.10"/>"#,
            my + lower_h * 0.4
        ));

        // Philtrum
        s.push_str(&format!(
            r#"<path d="M{} 59 L{} {}" stroke="{}" stroke-width="0.25" fill="none" opacity="0.08"/>"#,
            cx - 1.0, cx - 0.8, my - upper_h, shade(skin, 0.65)
        ));
        s.push_str(&format!(
            r#"<path d="M{} 59 L{} {}" stroke="{}" stroke-width="0.25" fill="none" opacity="0.08"/>"#,
            cx + 1.0, cx + 0.8, my - upper_h, shade(skin, 0.65)
        ));

        // Mouth corner shadows
        s.push_str(&format!(
            r#"<circle cx="{ml}" cy="{my}" r="0.6" fill="{sep}" opacity="0.10"/>"#,
        ));
        s.push_str(&format!(
            r#"<circle cx="{mr}" cy="{my}" r="0.6" fill="{sep}" opacity="0.10"/>"#,
        ));

        // Chin shadow
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="5" ry="1.5" fill="{skin_dk2}" opacity="0.07"/>"#,
            my + lower_h + 2.0
        ));
    }

    // ── Beard ───────────────────────────────────────────────
    if beard {
        let bd = shade(hair, 0.85);
        match beard_v {
            0 => { // Light stubble
                s.push_str(&format!(
                    r#"<path d="M{} 62 Q{} 76 {cx} 80 Q{} 76 {} 62 Q{cx} 74 {} 62Z" fill="{bd}" opacity="0.10"/>"#,
                    cx - 16.0, cx - 14.0, cx + 14.0, cx + 16.0, cx - 16.0
                ));
            }
            1 => { // Short beard
                s.push_str(&format!(
                    r#"<path d="M{} 60 Q{} 78 {cx} 82 Q{} 78 {} 60 Q{cx} 74 {} 60Z" fill="{bd}" opacity="0.30"/>"#,
                    cx - 18.0, cx - 16.0, cx + 16.0, cx + 18.0, cx - 18.0
                ));
            }
            2 => { // Full beard
                s.push_str(&format!(
                    r#"<path d="M{} 56 Q{} 80 {cx} 86 Q{} 80 {} 56 Q{cx} 76 {} 56Z" fill="{bd}" opacity="0.48"/>"#,
                    cx - 20.0, cx - 18.0, cx + 18.0, cx + 20.0, cx - 20.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{} 62 Q{} 76 {cx} 80 Q{} 76 {} 62Z" fill="{}" opacity="0.08"/>"#,
                    cx - 14.0, cx - 12.0, cx + 12.0, cx + 14.0, shade(hair, 1.15)
                ));
            }
            _ => { // Goatee
                s.push_str(&format!(
                    r#"<path d="M{} 63 Q{} 78 {cx} 84 Q{} 78 {} 63 Q{cx} 72 {} 63Z" fill="{bd}" opacity="0.35"/>"#,
                    cx - 8.0, cx - 7.0, cx + 7.0, cx + 8.0, cx - 8.0
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
                    r#"<path d="M{} 64.5 Q{cx} 63 {} 64.5" stroke="{mc_col}" stroke-width="0.8" fill="none"/>"#,
                    cx - 6.0, cx + 6.0
                ));
            }
            1 => { // Full
                s.push_str(&format!(
                    r#"<path d="M{} 64 Q{} 62 {cx} 64 Q{} 62 {} 64 Q{} 66 {cx} 65.5 Q{} 66 {} 64Z" fill="{mc_col}" opacity="0.50"/>"#,
                    cx - 7.0, cx - 4.0, cx + 4.0, cx + 7.0, cx + 5.0, cx - 5.0, cx - 7.0
                ));
            }
            _ => { // Handlebar
                s.push_str(&format!(
                    r#"<path d="M{} 63 Q{cx} 61 {} 63 L{} 65 Q{cx} 64 {} 65Z" fill="{mc_col}" opacity="0.45"/>"#,
                    cx - 8.0, cx + 8.0, cx + 7.0, cx - 7.0
                ));
            }
        }
    }

    // ── Hair ────────────────────────────────────────────────
    {
        let hd = shade(hair, 0.78);
        let hl_c = shade(hair, 1.20);
        let ht_y = fs.head_top;

        match hair_st {
            0 => { // Short crop
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {ht_y} {cx} {ht_y} C{} {ht_y} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 6.0, ht_y + 8.0, hl + 6.0,
                    hr - 6.0, ht_y + 8.0, cy_cheek - 6.0,
                    ht_y + 4.0, ht_y + 6.0, hr - 5.0, ht_y - 2.0, ht_y - 2.0,
                    hl + 5.0, ht_y - 2.0, ht_y + 6.0, ht_y + 4.0,
                ));
            }
            1 => { // Side part
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 4.0, ht_y + 8.0, hl + 5.0, ht_y + 2.0, ht_y + 2.0,
                    hr - 5.0, ht_y + 2.0, ht_y + 8.0, cy_cheek - 4.0,
                    ht_y, ht_y + 4.0, hr - 4.0, ht_y - 4.0, ht_y - 4.0,
                    hl + 4.0, ht_y - 4.0, ht_y + 4.0, ht_y,
                ));
                // Part line highlight
                s.push_str(&format!(
                    r#"<path d="M{} {} L{} {}" stroke="{hd}" stroke-width="0.4" opacity="0.3"/>"#,
                    cx - 8.0, ht_y + 1.0, cx - 6.0, ht_y + 8.0
                ));
            }
            2 => { // Medium textured
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 1.0, cy_cheek - 6.0, ht_y + 8.0, hl + 4.0, ht_y, ht_y,
                    hr - 4.0, ht_y, ht_y + 8.0, hr + 1.0, cy_cheek - 6.0,
                    hr + 1.0, ht_y - 2.0, ht_y + 4.0, hr - 3.0, ht_y - 6.0, ht_y - 6.0,
                    hl + 3.0, ht_y - 6.0, ht_y + 4.0, hl - 1.0, ht_y - 2.0,
                ));
            }
            3 => { // Buzz cut
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}" opacity="0.6"/>"#,
                    cy_cheek - 8.0, ht_y + 10.0, hl + 6.0, ht_y + 2.0, ht_y + 2.0,
                    hr - 6.0, ht_y + 2.0, ht_y + 10.0, cy_cheek - 8.0,
                    ht_y + 4.0, ht_y + 8.0, hr - 5.0, ht_y + 0.0, ht_y + 0.0,
                    hl + 5.0, ht_y + 0.0, ht_y + 8.0, ht_y + 4.0,
                ));
            }
            4 => { // Swept back
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 2.0, cy_cheek - 4.0, ht_y + 6.0, hl + 4.0, ht_y - 2.0, ht_y - 2.0,
                    hr - 4.0, ht_y - 2.0, ht_y + 6.0, hr + 2.0, cy_cheek - 4.0,
                    hr + 2.0, ht_y - 6.0, ht_y + 2.0, hr - 3.0, ht_y - 10.0, ht_y - 10.0,
                    hl + 3.0, ht_y - 10.0, ht_y + 2.0, hl - 2.0, ht_y - 6.0,
                ));
                // Volume highlight
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="0.5" fill="none" opacity="0.15"/>"#,
                    hl + 4.0, ht_y - 4.0, ht_y - 8.0, hr - 4.0, ht_y - 4.0
                ));
            }
            5 => { // Afro
                let afro_r = 28.0 + fw;
                s.push_str(&format!(
                    r#"<path d="
                        M{} {}
                        C{} {} {} {} {cx} {}
                        C{} {} {} {} {} {}
                        L{hr} {} C{hr} {} {} {} {cx} {}
                        C{} {} {hl} {} {hl} {}Z
                    " fill="{hair}"/>"#,
                    cx - afro_r, cy_cheek - 2.0,
                    cx - afro_r, ht_y - 8.0, cx - afro_r * 0.6, ht_y - 16.0, ht_y - 16.0,
                    cx + afro_r * 0.6, ht_y - 16.0, cx + afro_r, ht_y - 8.0, cx + afro_r, cy_cheek - 2.0,
                    cy_cheek - 6.0, ht_y + 8.0, hr - 6.0, ht_y + 2.0, ht_y + 2.0,
                    hl + 6.0, ht_y + 2.0, ht_y + 8.0, cy_cheek - 6.0,
                ));
                // Texture
                for dy in [-12.0f32, -8.0, -4.0] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hd}" stroke-width="0.4" fill="none" opacity="0.12"/>"#,
                        cx - 16.0, ht_y + dy, ht_y + dy - 1.5, cx + 16.0, ht_y + dy
                    ));
                }
            }
            6 => { // Bald
                // Just a subtle hairline shadow
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {}" stroke="{hd}" stroke-width="0.4" fill="none" opacity="0.12"/>"#,
                    cy_cheek - 10.0, ht_y + 8.0, hl + 6.0, ht_y + 2.0, ht_y + 2.0,
                    hr - 6.0, ht_y + 2.0, ht_y + 8.0, cy_cheek - 10.0,
                ));
                // Scalp shine
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="10" ry="6" fill="{skin_hi}" opacity="0.12"/>"#,
                    cx + 2.0, ht_y + 8.0
                ));
            }
            7 => { // Curly top
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 4.0, ht_y + 6.0, hl + 4.0, ht_y - 2.0, ht_y - 2.0,
                    hr - 4.0, ht_y - 2.0, ht_y + 6.0, cy_cheek - 4.0,
                    ht_y - 4.0, ht_y + 2.0, hr - 3.0, ht_y - 8.0, ht_y - 8.0,
                    hl + 3.0, ht_y - 8.0, ht_y + 2.0, ht_y - 4.0,
                ));
                // Curl texture bumps
                for bx in [-10.0f32, -4.0, 2.0, 8.0] {
                    s.push_str(&format!(
                        r#"<circle cx="{}" cy="{}" r="3" fill="{hair}"/>"#,
                        cx + bx, ht_y - 6.0 + (bx.abs() * 0.15)
                    ));
                }
            }
            8 => { // Long / flowing
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} Q{} {} {} {} L{hl} {} Q{} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 3.0, cy_cheek + 6.0, ht_y + 6.0, hl + 3.0, ht_y - 4.0, ht_y - 4.0,
                    hr - 3.0, ht_y - 4.0, ht_y + 6.0, hr + 3.0, cy_cheek + 6.0,
                    hr + 3.0, ht_y - 8.0, hr - 2.0, ht_y - 12.0, cx, ht_y - 12.0,
                    ht_y - 8.0, hl + 2.0, ht_y - 12.0, hl - 3.0, ht_y - 8.0,
                ));
                // Strand highlights
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="0.3" fill="none" opacity="0.12"/>"#,
                    hl + 2.0, ht_y, ht_y - 6.0, hr - 2.0, ht_y
                ));
            }
            _ => { // Fade / undercut
                // Top hair
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl + 2.0, cy_cheek - 8.0, ht_y + 10.0, hl + 6.0, ht_y, ht_y,
                    hr - 6.0, ht_y, ht_y + 10.0, hr - 2.0, cy_cheek - 8.0,
                    hr - 2.0, ht_y - 2.0, ht_y + 6.0, hr - 5.0, ht_y - 6.0, ht_y - 6.0,
                    hl + 5.0, ht_y - 6.0, ht_y + 6.0, hl + 2.0, ht_y - 2.0,
                ));
                // Faded sides (lighter opacity)
                s.push_str(&format!(
                    r#"<path d="M{hl} {} L{} {} L{hl} {}Z" fill="{hair}" opacity="0.25"/>"#,
                    cy_cheek - 8.0, hl + 2.0, cy_cheek - 8.0, cy_cheek + 2.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} L{} {} L{hr} {}Z" fill="{hair}" opacity="0.25"/>"#,
                    cy_cheek - 8.0, hr - 2.0, cy_cheek - 8.0, cy_cheek + 2.0
                ));
            }
        }
    }

    s.push_str("</svg>");
    s
}
