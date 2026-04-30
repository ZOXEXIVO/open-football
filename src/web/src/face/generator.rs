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
        SkinDist { white: 50, black: 20, _metis: 30 }
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
    "#F5E0CB", "#EACFB0", "#DDBF98", "#CDA97A",
    "#C09368", "#A87D58", "#926845", "#7D5535",
    "#694530", "#503322", "#3D2518", "#2E1B11",
];

const HAIR: [&str; 10] = [
    "#0E0E0E", "#1C150C", "#2F1F11", "#4D3A2B",
    "#6A5038", "#7E644A", "#96795A", "#B0946C",
    "#C4A882", "#6B2010",
];

const EYES: [&str; 8] = [
    "#33251A", "#4A3828", "#5C4E3A", "#384F62",
    "#3D5844", "#4E6356", "#686D72", "#3F5A72",
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
    // cheek_w <= temple_w always — face tapers smoothly from forehead down
    match variant {
        0 => FaceShape { // Oval
            head_top: 14.5, temple_w: 21.5 + fw * 0.7, cheek_w: 19.8 + fw * 0.65,
            cheek_y: 49.0, jaw_w: 17.2 + fw * 0.8, jaw_y: 70.0,
            chin_w: 7.8 + fw * 0.35, chin_y: 82.0, chin_round: 3.0,
        },
        1 => FaceShape { // Square
            head_top: 14.5, temple_w: 22.0 + fw * 0.75, cheek_w: 20.5 + fw * 0.7,
            cheek_y: 48.0, jaw_w: 20.0 + fw * 0.9, jaw_y: 71.0,
            chin_w: 11.5 + fw * 0.45, chin_y: 81.5, chin_round: 1.8,
        },
        2 => FaceShape { // Round
            head_top: 14.0, temple_w: 22.0 + fw * 0.75, cheek_w: 20.8 + fw * 0.75,
            cheek_y: 50.0, jaw_w: 18.8 + fw * 0.85, jaw_y: 71.0,
            chin_w: 9.8 + fw * 0.4, chin_y: 82.0, chin_round: 3.8,
        },
        3 => FaceShape { // Heart
            head_top: 14.0, temple_w: 22.0 + fw * 0.7, cheek_w: 19.6 + fw * 0.6,
            cheek_y: 48.0, jaw_w: 16.3 + fw * 0.55, jaw_y: 71.0,
            chin_w: 7.2 + fw * 0.25, chin_y: 82.5, chin_round: 2.4,
        },
        4 => FaceShape { // Oblong
            head_top: 12.5, temple_w: 20.5 + fw * 0.65, cheek_w: 19.3 + fw * 0.65,
            cheek_y: 48.0, jaw_w: 17.8 + fw * 0.75, jaw_y: 72.0,
            chin_w: 8.0 + fw * 0.3, chin_y: 84.0, chin_round: 2.2,
        },
        _ => FaceShape { // Diamond
            head_top: 13.5, temple_w: 20.8 + fw * 0.65, cheek_w: 20.2 + fw * 0.65,
            cheek_y: 48.0, jaw_w: 16.4 + fw * 0.55, jaw_y: 71.5,
            chin_w: 7.4 + fw * 0.25, chin_y: 83.0, chin_round: 2.0,
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
    let texture_seed = r.range(9999);
    let cheekbone_st = r.range(4);
    let face_marks = r.range(5);

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
        0..=19  => r.frange(-1.1, 0.1),
        20..=24 => r.frange(-0.5, 0.8),
        25..=29 => r.frange(0.0, 1.6),
        30..=34 => r.frange(0.8, 2.4),
        _       => r.frange(1.3, 3.0),
    };

    let fs = face_shape(face_var, fw);
    let cx = 100.0f32;
    let maturity = match age {
        0..=21 => 0.35,
        22..=27 => 0.55,
        28..=33 => 0.75,
        _ => 0.95,
    };

    // Derived colors
    let skin_hi = shade(skin, 1.12);
    let skin_mid = skin.to_string();
    let skin_dk = shade(skin, 0.84);
    let skin_dk2 = shade(skin, 0.72);
    let skin_shadow = shade(skin, 0.60);
    let bg_color = "#34383A";

    // Jersey colors (deterministic from player_id)
    let jersey_hue = (player_id * 137) % 360;
    let jersey_color = format!("hsl({}, 28%, 27%)", jersey_hue);
    let jersey_light = format!("hsl({}, 24%, 38%)", jersey_hue);

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

    // Skin subsurface color — warm undertone for realism
    let skin_warm = blend(skin, "#C48060", 0.08);

    s.push_str(&format!(
        r#"<defs>
        <radialGradient id="sg" cx="44%" cy="30%" r="62%">
            <stop offset="0%" stop-color="{skin_hi}"/>
            <stop offset="25%" stop-color="{skin_warm}"/>
            <stop offset="55%" stop-color="{skin_mid}"/>
            <stop offset="80%" stop-color="{skin_dk}"/>
            <stop offset="100%" stop-color="{skin_dk2}"/>
        </radialGradient>
        <linearGradient id="nsg" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="{skin_shadow}" stop-opacity="0"/>
            <stop offset="80%" stop-color="{skin_shadow}" stop-opacity="0.18"/>
        </linearGradient>
        <radialGradient id="fh" cx="48%" cy="22%" r="45%">
            <stop offset="0%" stop-color="{skin_hi}" stop-opacity="0.12"/>
            <stop offset="70%" stop-color="{skin_hi}" stop-opacity="0.04"/>
            <stop offset="100%" stop-color="{skin_hi}" stop-opacity="0"/>
        </radialGradient>
        <filter id="tx">
            <feTurbulence type="fractalNoise" baseFrequency="0.92" numOctaves="4" seed="{texture_seed}" result="n"/>
            <feColorMatrix type="saturate" values="0" in="n" result="ng"/>
            <feComponentTransfer in="ng" result="grain"><feFuncA type="table" tableValues="0 0.35"/></feComponentTransfer>
            <feBlend in="SourceGraphic" in2="grain" mode="multiply"/>
        </filter>
        <pattern id="pores" width="9" height="9" patternUnits="userSpaceOnUse">
            <circle cx="1.5" cy="2" r="0.35" fill="{skin_shadow}" opacity="0.16"/>
            <circle cx="6.5" cy="4.5" r="0.28" fill="{skin_dk2}" opacity="0.13"/>
            <circle cx="4" cy="7.4" r="0.24" fill="{skin_hi}" opacity="0.10"/>
        </pattern>
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
            r#"<ellipse cx="{ex}" cy="{ear_y}" rx="5.4" ry="11.2" fill="{skin_mid}"/>"#,
        ));
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{ear_y}" rx="3.1" ry="7.1" fill="{ei}" opacity="0.42"/>"#,
            if ex < cx { ex + 1.5 } else { ex - 1.5 }
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="0.55" fill="none" opacity="0.22"/>"#,
            if ex < cx { ex + 0.8 } else { ex - 0.8 }, ear_y - 5.8,
            if ex < cx { ex + 3.0 } else { ex - 3.0 }, ear_y,
            if ex < cx { ex + 0.4 } else { ex - 0.4 }, ear_y + 5.5,
            shade(skin, 0.55)
        ));
        // Earlobe
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="2.5" ry="3.0" fill="{skin_mid}" opacity="0.68"/>"#,
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

    // Human head outline: narrower cranium, visible cheek plane and adult jaw.
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

    // Fine skin texture and adult male planes reduce the flat cartoon look.
    s.push_str(&format!(
        r#"<path d="M{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {} C{} {} {} {} {} {} Q{cx} {} {} {} Q{} {} {} {} C{} {} {} {} {} {}Z" fill="url(#pores)" opacity="{}" filter="url(#tx)"/>"#,
        hl + 2.0, cy_cheek - 1.0,
        hl + 1.5, ht + 23.0, hl + 15.0, ht + 2.0, ht + 2.0,
        hr - 15.0, ht + 2.0, hr - 1.5, ht + 23.0, hr - 2.0, cy_cheek - 1.0,
        hr - 2.0, jy - 10.0, mid_r, mid_y, jr - 2.0, jy,
        chy + cr_val - 1.0, chl + 2.0, chy,
        jl + 2.0, jy + cr_val - 1.0, jl + 2.0, jy,
        mid_l, mid_y, hl + 2.0, jy - 10.0, hl + 2.0, cy_cheek - 1.0,
        0.11 + maturity * 0.05
    ));

    let cheek_shadow = shade(skin, 0.66);
    let cheek_opacity = 0.08 + maturity * 0.08 + cheekbone_st as f32 * 0.01;
    for (side, dip) in [(-1.0f32, 0.0), (1.0, 1.5)] {
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {} Q{} {} {} {}" stroke="{cheek_shadow}" stroke-width="1.1" fill="none" opacity="{}" stroke-linecap="round"/>"#,
            cx + side * 12.0, cy_cheek + 5.0 + dip,
            cx + side * 25.0, cy_cheek + 13.0 + dip,
            cx + side * 31.0, cy_cheek + 27.0 + dip,
            cx + side * 25.0, cy_cheek + 34.0 + dip,
            cx + side * 15.0, cy_cheek + 37.0 + dip,
            cheek_opacity
        ));
    }
    s.push_str(&format!(
        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_shadow}" stroke-width="1.2" fill="none" opacity="{}" stroke-linecap="round"/>"#,
        jl + 8.0, jy + 5.0, chy + 5.5, jr - 8.0, jy + 5.0, 0.10 + maturity * 0.07
    ));
    s.push_str(&format!(
        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_hi}" stroke-width="0.7" fill="none" opacity="{}" stroke-linecap="round"/>"#,
        cx - 8.0, chy - 2.0, chy + 1.0, cx + 8.0, chy - 2.0, 0.07 + maturity * 0.03
    ));

    if face_marks > 1 {
        let mark_col = shade(skin, 0.55);
        for i in 0..face_marks {
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let mx = cx + side * (14.0 + (i as f32 * 5.2) % 19.0);
            let my = 120.0 + ((texture_seed as f32 / (i + 2) as f32) % 43.0);
            s.push_str(&format!(
                r#"<circle cx="{mx}" cy="{my}" r="0.55" fill="{mark_col}" opacity="0.16"/>"#,
            ));
        }
    }

    // ── Facial lighting layers ─────────────────────────────
    // Forehead highlight (top-left light source)
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="22" ry="14" fill="url(#fh)"/>"#,
        cx - 2.0, ht + 30.0
    ));

    // Cheekbone highlights — gives structure to the face
    let cheek_hi = blend(skin, "#E8D8C8", 0.15);
    for (chx, rx_off) in [(cx - 25.0, 0.0f32), (cx + 23.0, 1.0)] {
        s.push_str(&format!(
            r#"<ellipse cx="{chx}" cy="{}" rx="{}" ry="5" fill="{cheek_hi}" opacity="0.14"/>"#,
            cy_cheek + 1.0, 9.0 + rx_off
        ));
    }

    // Cheek warmth — asymmetric for natural look
    let blush = blend(skin, "#C07868", 0.10);
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="13" ry="9" fill="{blush}" opacity="0.08"/>"#,
        cx - 27.0, cy_cheek + 12.0
    ));
    s.push_str(&format!(
        r#"<ellipse cx="{}" cy="{}" rx="11" ry="7" fill="{blush}" opacity="0.06"/>"#,
        cx + 25.0, cy_cheek + 11.0
    ));

    // Temple shadows — stronger for depth
    for tx in [hl + 6.0, hr - 6.0] {
        s.push_str(&format!(
            r#"<ellipse cx="{tx}" cy="{}" rx="9" ry="21" fill="{skin_dk2}" opacity="0.18"/>"#,
            cy_cheek - 6.0
        ));
    }

    // Jaw shadow — defined jawline
    s.push_str(&format!(
        r#"<path d="M{jl} {jy} Q{cx} {} {jr} {jy} Q{cx} {} {jl} {jy}Z" fill="{skin_dk2}" opacity="0.16"/>"#,
        chy - 4.0, chy + 2.0
    ));

    // Under-chin shadow — stronger
    s.push_str(&format!(
        r#"<ellipse cx="{cx}" cy="{}" rx="22" ry="7" fill="{skin_shadow}" opacity="0.14"/>"#,
        chy + 4.0
    ));

    // Side jaw shadows for definition
    for jx in [jl - 4.0, jr + 4.0] {
        s.push_str(&format!(
            r#"<ellipse cx="{jx}" cy="{}" rx="6" ry="14" fill="{skin_dk2}" opacity="0.10"/>"#,
            jy - 6.0
        ));
    }

    // Natural nasolabial and mid-face planes exist even on younger adults.
    let fold_opacity = 0.055 + wrinkle_opacity;
    s.push_str(&format!(
        r#"<path d="M{} 139 Q{} 158 {} 172" stroke="{skin_dk2}" stroke-width="0.55" fill="none" opacity="{fold_opacity}"/>"#,
        cx - 18.5, cx - 23.0, cx - 21.0
    ));
    s.push_str(&format!(
        r#"<path d="M{} 139 Q{} 158 {} 172" stroke="{skin_dk2}" stroke-width="0.55" fill="none" opacity="{fold_opacity}"/>"#,
        cx + 18.5, cx + 23.0, cx + 21.0
    ));

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
        let brow_y = 104.0 + ay;
        // Horizontal shadow across brow bone — gives depth to the eye sockets
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{skin_dk2}" stroke-width="4.8" fill="none" opacity="0.17" stroke-linecap="round"/>"#,
            cx - 36.0, brow_y + 1.0, brow_y - 2.5, cx + 36.0, brow_y + 1.0
        ));
        // Inner eye socket shadows — deeper
        for sx in [cx - 19.0, cx + 19.0] {
            s.push_str(&format!(
                r#"<ellipse cx="{sx}" cy="{}" rx="13" ry="8" fill="{skin_dk2}" opacity="0.10"/>"#,
                brow_y + 12.0
            ));
        }
    }

    // ── Eyes (almond-shaped, realistic) ─────────────────────
    {
        let ey = 116.5 + ay * 2.0;
        let lx = cx - 17.2 + ax * 1.2;
        let rx_e = cx + 17.2 - ax;
        // Muted sclera — slightly warm off-white
        let sclera = "#E2DDD6";

        // Proportional eyes — smaller iris/pupil ratio for realism
        let (erx, ery, iris_r, pupil_r): (f32, f32, f32, f32) = match eye_st {
            0 => (7.8, 3.0, 2.9, 1.2),    // narrow
            1 => (7.4, 2.7, 2.7, 1.1),    // small
            2 => (8.2, 3.4, 3.0, 1.2),    // open
            3 => (7.2, 2.8, 2.7, 1.1),    // squinting
            _ => (8.0, 3.1, 2.9, 1.2),    // medium
        };

        let lid_col = shade(skin, 0.72);
        let lash_col = shade(skin, 0.45);
        let iris_rim = shade(eye_col, 0.55);
        let iris_hi = shade(eye_col, 1.25);

        for (i, (ex, side)) in [(lx, -1.0f32), (rx_e, 1.0)].iter().enumerate() {
            let clip_id = format!("ec{i}");

            // Almond-shaped eye path — pointed at inner/outer corners
            let _inner_x = ex - erx * side; // inner corner (near nose)
            let _outer_x = ex + erx * side; // outer corner
            let el = ex - erx;
            let er = ex + erx;

            // Sclera — almond shape using cubic beziers
            s.push_str(&format!(
                r#"<path d="M{el} {ey} Q{} {} {ex} {} Q{} {} {er} {ey} Q{} {} {ex} {} Q{} {} {el} {ey}Z" fill="{sclera}" opacity="0.78"/>"#,
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
            // Catchlight — smaller, softer
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="0.45" fill="white" opacity="0.28"/>"#,
                ex - 1.0 * side, ey - 1.3
            ));
            // Secondary catchlight — very faint
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="0.25" fill="white" opacity="0.12"/>"#,
                ex + 0.8 * side, ey + 0.5
            ));

            s.push_str("</g>");

            // Upper eyelid line — natural lash line, tapers at edges
            s.push_str(&format!(
                r#"<path d="M{el} {ey} Q{ex} {} {er} {}" stroke="{lash_col}" stroke-width="1.15" fill="none" stroke-linecap="round"/>"#,
                ey - ery - 1.2, ey - 0.3
            ));
            // Outer lash thickening
            s.push_str(&format!(
                r#"<path d="M{} {} Q{} {} {er} {}" stroke="{lash_col}" stroke-width="0.8" fill="none" opacity="0.5" stroke-linecap="round"/>"#,
                ex, ey - ery - 0.5, ex + erx * 0.6 * side.signum(), ey - ery * 0.6, ey - 0.3
            ));
            // Lower lid — subtle definition
            s.push_str(&format!(
                r#"<path d="M{} {} Q{ex} {} {} {}" stroke="{lid_col}" stroke-width="0.5" fill="none" opacity="0.30"/>"#,
                el + 2.0, ey + 0.3, ey + ery + 0.3, er - 2.0, ey + 0.3
            ));
            // Inner and outer canthus details.
            let tear = blend(skin, "#A85B50", 0.22);
            s.push_str(&format!(
                r#"<circle cx="{}" cy="{}" r="0.85" fill="{tear}" opacity="0.28"/>"#,
                if *side < 0.0 { er - 0.4 } else { el + 0.4 }, ey + 0.2
            ));
            s.push_str(&format!(
                r#"<path d="M{} {} l{} {}" stroke="{lash_col}" stroke-width="0.45" fill="none" opacity="0.34" stroke-linecap="round"/>"#,
                if *side < 0.0 { el + 0.8 } else { er - 0.8 },
                ey - 0.1,
                2.0 * side,
                -1.0
            ));
            // Eyelid crease — deeper
            s.push_str(&format!(
                r#"<path d="M{} {} Q{ex} {} {} {}" stroke="{skin_dk2}" stroke-width="0.7" fill="none" opacity="0.18"/>"#,
                el - 0.5, ey - ery + 2.0, ey - ery - 4.0, er + 0.5, ey - ery + 2.0
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
        let by = 102.0 + ay * 2.0;
        let blx = cx - 17.5 + ax;
        let brx = cx + 17.5 - ax * 0.5;
        let brow_col = shade(hair, 0.90);

        let (bw, bt, arch): (f32, f32, f32) = match brow_st {
            0 => (2.7, 0.0, 1.8),    // straight
            1 => (2.6, -0.4, 3.2),   // arched
            2 => (3.3, 0.5, 1.4),    // flat thick
            3 => (2.6, -0.2, 2.7),   // medium arch
            4 => (3.7, 0.0, 2.2),    // bushy
            _ => (2.4, -0.2, 3.0),   // high arch
        };

        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            blx - 12.0, by + bt, blx, by - arch, blx + 12.0, by + bt * 0.6
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{brow_col}" stroke-width="{bw}" fill="none" stroke-linecap="round"/>"#,
            brx - 12.0, by + bt * 0.6, brx, by - arch * 0.95, brx + 12.0, by + bt
        ));

        // Individual brow hairs make the portrait read less like a sticker.
        for i in 0..8 {
            let t = i as f32 / 7.0;
            let lx0 = blx - 11.0 + t * 22.0;
            let rx0 = brx - 11.0 + t * 22.0;
            let lift = (0.5 - (t - 0.5).abs()) * arch * 0.45;
            s.push_str(&format!(
                r#"<path d="M{} {} l{} {}" stroke="{brow_col}" stroke-width="0.45" fill="none" opacity="0.45" stroke-linecap="round"/>"#,
                lx0, by + bt - lift * 0.5, 2.2, -1.1 - lift * 0.18
            ));
            s.push_str(&format!(
                r#"<path d="M{} {} l{} {}" stroke="{brow_col}" stroke-width="0.45" fill="none" opacity="0.45" stroke-linecap="round"/>"#,
                rx0, by + bt - lift * 0.45, -2.2, -1.0 - lift * 0.16
            ));
        }
    }

    // ── Nose (visible, structured) ──────────────────────────
    {
        let ny = 145.0;
        let ns = shade(skin, 0.55);
        let nt = shade(skin, 0.70);
        let nhi = shade(skin, 1.08);

        let (bw, tw, th, nostril_w): (f32, f32, f32, f32) = match nose_st {
            0 => (1.1, 7.8, 3.8, 2.6),   // small straight
            1 => (1.3, 11.5, 5.2, 3.8),  // wide
            2 => (1.2, 9.4, 4.4, 3.1),   // medium
            3 => (1.0, 7.2, 4.8, 2.4),   // narrow
            4 => (1.2, 10.4, 4.8, 3.3),  // aquiline
            _ => (1.1, 8.7, 4.0, 2.8),   // compact
        };

        // Bridge — curved lines on both sides from brow to tip
        s.push_str(&format!(
            r#"<path d="M{} 103 Q{} 126 {} {ny}" stroke="{ns}" stroke-width="{bw}" fill="none" opacity="0.36"/>"#,
            cx - 3.8, cx - 4.3, cx - 2.2
        ));
        s.push_str(&format!(
            r#"<path d="M{} 103 Q{} 126 {} {ny}" stroke="{ns}" stroke-width="{}" fill="none" opacity="0.26"/>"#,
            cx + 3.8, cx + 4.3, cx + 2.2, bw * 0.7
        ));
        // Bridge highlight (light catches center ridge)
        s.push_str(&format!(
            r#"<path d="M{} 108 L{} {}" stroke="{nhi}" stroke-width="1.5" fill="none" opacity="0.12"/>"#,
            cx + 0.5, cx + 0.3, ny - 5.0
        ));
        // Harder bridge planes read more adult and masculine.
        s.push_str(&format!(
            r#"<path d="M{} 118 Q{} 132 {} {}" stroke="{skin_dk2}" stroke-width="0.55" fill="none" opacity="{}"/>"#,
            cx - 7.0, cx - 8.0, cx - 6.0, ny - 2.0, 0.12 + maturity * 0.06
        ));
        s.push_str(&format!(
            r#"<path d="M{} 118 Q{} 132 {} {}" stroke="{skin_dk2}" stroke-width="0.45" fill="none" opacity="{}"/>"#,
            cx + 7.0, cx + 8.0, cx + 6.0, ny - 2.0, 0.09 + maturity * 0.04
        ));
        // Nose tip — softer, less prominent
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{ny}" rx="{}" ry="{}" fill="{nt}" opacity="0.10"/>"#,
            tw * 0.75, th * 0.75
        ));
        // Tip highlight — off-center for realism
        s.push_str(&format!(
            r#"<ellipse cx="{}" cy="{}" rx="2.0" ry="1.4" fill="{nhi}" opacity="0.10"/>"#,
            cx + 0.6, ny - 1.0
        ));
        // Nostrils — teardrop-shaped, not circles
        let nl = cx - tw * 0.45;
        let nr = cx + tw * 0.45;
        s.push_str(&format!(
            r#"<path d="M{nl} {} Q{} {} {} {} Q{} {} {nl} {}Z" fill="{ns}" opacity="0.30"/>"#,
            ny + 1.0, nl - nostril_w, ny + 1.5, nl - nostril_w * 0.6, ny + 3.0,
            nl + nostril_w * 0.3, ny + 3.2, ny + 1.0
        ));
        s.push_str(&format!(
            r#"<path d="M{nr} {} Q{} {} {} {} Q{} {} {nr} {}Z" fill="{ns}" opacity="0.30"/>"#,
            ny + 1.0, nr + nostril_w, ny + 1.5, nr + nostril_w * 0.6, ny + 3.0,
            nr - nostril_w * 0.3, ny + 3.2, ny + 1.0
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
        let my = 166.0;
        let lp = lip_color(skin);
        let lp_dk = shade(&lp, 0.60);
        let lp_hi = shade(&lp, 1.12);
        let sep = shade(skin, 0.42);

        let (mw, upper_h, lower_h): (f32, f32, f32) = match mouth_st {
            0 => (12.5, 2.0, 2.8),    // medium
            1 => (15.0, 1.8, 3.2),    // wide
            2 => (10.8, 2.2, 2.5),    // small
            3 => (13.4, 2.6, 3.4),    // full
            _ => (11.8, 1.6, 2.4),    // thin
        };

        let ml = cx - mw;
        let mr = cx + mw;

        // Lip separation line — defines the mouth
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}" stroke="{lp_dk}" stroke-width="0.75" fill="none" opacity="0.62"/>"#,
            my + 0.5
        ));
        // Upper lip — cupid's bow
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{} {} {} {} Q{cx} {} {} {} Q{} {} {mr} {my} L{ml} {my}Z" fill="{lp}" opacity="0.24"/>"#,
            ml + mw * 0.3, my - upper_h * 0.3,
            cx - 3.0, my - upper_h,
            my - upper_h * 0.5,
            cx + 3.0, my - upper_h,
            mr - mw * 0.3, my - upper_h * 0.3,
        ));
        // Lower lip — fuller, lighter
        s.push_str(&format!(
            r#"<path d="M{ml} {my} Q{cx} {} {mr} {my}Z" fill="{lp}" opacity="0.18"/>"#,
            my + lower_h
        ));
        // Upper lip vermilion border
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{lp_dk}" stroke-width="0.4" fill="none" opacity="0.20"/>"#,
            cx - 4.0, my - upper_h * 0.7, my - upper_h - 0.5, cx + 4.0, my - upper_h * 0.7
        ));
        // Lower lip shine
        s.push_str(&format!(
            r#"<ellipse cx="{cx}" cy="{}" rx="4.5" ry="1.0" fill="{lp_hi}" opacity="0.06"/>"#,
            my + lower_h * 0.4
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{sep}" stroke-width="0.35" fill="none" opacity="0.16"/>"#,
            ml + 3.0, my + lower_h + 1.8, my + lower_h + 3.0, mr - 3.0, my + lower_h + 1.8
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
                    r#"<path d="M{} 155 Q{} 190 {cx} 201 Q{} 190 {} 155 Q{cx} 182 {} 155Z" fill="{bd}" opacity="0.13"/>"#,
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
        let dot_col = shade(hair, 0.65);
        for i in 0..58 {
            let row = (i / 11) as f32;
            let col = (i % 11) as f32;
            let px = cx - 31.0 + col * 6.2 + ((texture_seed + i) % 5) as f32 * 0.35 - 0.7;
            let py = 154.0 + row * 8.4 + ((texture_seed / (i + 1)) % 5) as f32 * 0.55;
            let jaw_taper = ((py - 156.0) / 43.0).max(0.0).min(1.0);
            if (px - cx).abs() < 33.0 - jaw_taper * 13.0 {
                s.push_str(&format!(
                    r#"<circle cx="{px}" cy="{py}" r="0.34" fill="{dot_col}" opacity="0.24"/>"#,
                ));
            }
        }
    } else if age >= 22 {
        let shave_col = shade(hair, 0.72);
        s.push_str(&format!(
            r#"<path d="M{} 153 Q{} 190 {cx} 201 Q{} 190 {} 153 Q{cx} 176 {} 153Z" fill="{shave_col}" opacity="{}"/>"#,
            cx - 35.0, cx - 31.0, cx + 31.0, cx + 35.0, cx - 35.0, opacity(0.05 + maturity * 0.055)
        ));
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
        let hd = shade(hair, 0.75);
        let hl_c = shade(hair, 1.18);
        let h_mid = shade(hair, 0.90);

        // Base hairline helper — shared curved top matching the skull
        // outer = visible edge of hair, inner = where hair meets forehead
        let hair_outer_top = ht - 6.0;
        let hair_inner_top = ht + 6.0;

        match hair_st {
            0 => { // Short crop — tight to skull with visible scalp gradient
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 14.0, ht + 18.0, hl + 14.0, hair_inner_top, hair_inner_top,
                    hr - 14.0, hair_inner_top, ht + 18.0, cy_cheek - 14.0,
                    hair_outer_top + 2.0, ht + 12.0, hr - 12.0, hair_outer_top - 4.0, hair_outer_top - 4.0,
                    hl + 12.0, hair_outer_top - 4.0, ht + 12.0, hair_outer_top + 2.0,
                ));
                // Volume highlight
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="18" ry="8" fill="{hl_c}" opacity="0.08"/>"#,
                    cx - 4.0, hair_outer_top + 4.0
                ));
            }
            1 => { // Side part — clean with natural volume
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 18.0, hl + 12.0, hair_inner_top, hair_inner_top,
                    hr - 12.0, hair_inner_top, ht + 18.0, cy_cheek - 10.0,
                    hair_outer_top - 4.0, ht + 8.0, hr - 10.0, hair_outer_top - 12.0, hair_outer_top - 12.0,
                    hl + 10.0, hair_outer_top - 8.0, ht + 10.0, hair_outer_top - 2.0,
                ));
                // Part line
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{} {} {} {}" stroke="{hd}" stroke-width="0.8" fill="none" opacity="0.30"/>"#,
                    cx - 18.0, hair_outer_top - 2.0, cx - 15.0, hair_inner_top + 4.0, cx - 12.0, hair_inner_top + 14.0
                ));
                // Volume highlight on the swept side
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="14" ry="6" fill="{hl_c}" opacity="0.07"/>"#,
                    cx + 6.0, hair_outer_top + 2.0
                ));
            }
            2 => { // Medium textured — natural volume with soft edge
                let hair_ext = 3.0;
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - hair_ext, cy_cheek - 14.0, ht + 16.0, hl + 10.0, hair_inner_top - 2.0, hair_inner_top - 2.0,
                    hr - 10.0, hair_inner_top - 2.0, ht + 16.0, hr + hair_ext, cy_cheek - 14.0,
                    hr + hair_ext, hair_outer_top - 6.0, ht + 8.0, hr - 8.0, hair_outer_top - 16.0, hair_outer_top - 16.0,
                    hl + 8.0, hair_outer_top - 16.0, ht + 8.0, hl - hair_ext, hair_outer_top - 6.0,
                ));
                // Hair texture strands
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{h_mid}" stroke-width="0.5" fill="none" opacity="0.12"/>"#,
                    hl + 12.0, hair_outer_top - 6.0, hair_outer_top - 14.0, hr - 12.0, hair_outer_top - 6.0
                ));
            }
            3 => { // Buzz cut — very short, scalp shows through
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}" opacity="0.45"/>"#,
                    cy_cheek - 18.0, ht + 22.0, hl + 14.0, hair_inner_top, hair_inner_top,
                    hr - 14.0, hair_inner_top, ht + 22.0, cy_cheek - 18.0,
                    hair_outer_top + 4.0, ht + 14.0, hr - 12.0, hair_outer_top - 2.0, hair_outer_top - 2.0,
                    hl + 12.0, hair_outer_top - 2.0, ht + 14.0, hair_outer_top + 4.0,
                ));
                // Scalp sheen
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="20" ry="10" fill="{skin_hi}" opacity="0.06"/>"#,
                    cx + 2.0, hair_outer_top + 10.0
                ));
            }
            4 => { // Swept back — slicked with volume
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 4.0, cy_cheek - 10.0, ht + 14.0, hl + 10.0, hair_inner_top - 6.0, hair_inner_top - 6.0,
                    hr - 10.0, hair_inner_top - 6.0, ht + 14.0, hr + 4.0, cy_cheek - 10.0,
                    hr + 4.0, hair_outer_top - 16.0, ht + 5.0, hr - 8.0, hair_outer_top - 24.0, hair_outer_top - 24.0,
                    hl + 8.0, hair_outer_top - 24.0, ht + 5.0, hl - 4.0, hair_outer_top - 16.0,
                ));
                // Swept highlight
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="1.0" fill="none" opacity="0.10"/>"#,
                    hl + 10.0, hair_outer_top - 8.0, hair_outer_top - 20.0, hr - 10.0, hair_outer_top - 8.0
                ));
            }
            5 => { // Afro — rounded volume
                let afro_r = 60.0 + fw * 2.0;
                s.push_str(&format!(
                    r#"<path d="
                        M{} {}
                        C{} {} {} {} {cx} {}
                        C{} {} {} {} {} {}
                        L{hr} {} C{hr} {} {} {} {cx} {}
                        C{} {} {hl} {} {hl} {}Z
                    " fill="{hair}"/>"#,
                    cx - afro_r, cy_cheek - 4.0,
                    cx - afro_r, ht - 14.0, cx - afro_r * 0.6, ht - 30.0, ht - 30.0,
                    cx + afro_r * 0.6, ht - 30.0, cx + afro_r, ht - 14.0, cx + afro_r, cy_cheek - 4.0,
                    cy_cheek - 14.0, ht + 18.0, hr - 14.0, hair_inner_top, hair_inner_top,
                    hl + 14.0, hair_inner_top, ht + 18.0, cy_cheek - 14.0,
                ));
                // Afro texture — subtle volume rings
                for dy in [-22.0f32, -12.0, -2.0] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hd}" stroke-width="0.5" fill="none" opacity="0.08"/>"#,
                        cx - afro_r * 0.7, ht + dy + 4.0, ht + dy, cx + afro_r * 0.7, ht + dy + 4.0
                    ));
                }
                // Afro highlight
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="{}" ry="14" fill="{hl_c}" opacity="0.06"/>"#,
                    cx - 8.0, ht - 12.0, afro_r * 0.35
                ));
            }
            6 => { // Bald — clean head with realistic sheen
                // Subtle hairline shadow
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {}" stroke="{hd}" stroke-width="0.5" fill="none" opacity="0.08"/>"#,
                    cy_cheek - 22.0, ht + 18.0, hl + 14.0, hair_inner_top, hair_inner_top,
                    hr - 14.0, hair_inner_top, ht + 18.0, cy_cheek - 22.0,
                ));
                // Head sheen — realistic light reflection
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="24" ry="14" fill="{skin_hi}" opacity="0.12"/>"#,
                    cx + 3.0, ht + 16.0
                ));
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="12" ry="8" fill="{skin_hi}" opacity="0.06"/>"#,
                    cx + 6.0, ht + 12.0
                ));
            }
            7 => { // Curly top — defined curls without cartoon bubbles
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 14.0, hl + 10.0, hair_inner_top - 4.0, hair_inner_top - 4.0,
                    hr - 10.0, hair_inner_top - 4.0, ht + 14.0, cy_cheek - 10.0,
                    hair_outer_top - 10.0, ht + 4.0, hr - 8.0, hair_outer_top - 20.0, hair_outer_top - 20.0,
                    hl + 8.0, hair_outer_top - 20.0, ht + 4.0, hair_outer_top - 10.0,
                ));
                // Curl texture — wavy lines instead of circles
                for (bx, dy) in [(-18.0f32, 0.0), (-6.0, -2.0), (6.0, -1.0), (18.0, 1.0)] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{} {} {} {} Q{} {} {} {}" stroke="{hd}" stroke-width="0.6" fill="none" opacity="0.15"/>"#,
                        cx + bx - 4.0, hair_outer_top - 12.0 + dy,
                        cx + bx - 2.0, hair_outer_top - 16.0 + dy,
                        cx + bx, hair_outer_top - 12.0 + dy,
                        cx + bx + 2.0, hair_outer_top - 8.0 + dy,
                        cx + bx + 4.0, hair_outer_top - 12.0 + dy,
                    ));
                }
                // Volume highlight
                s.push_str(&format!(
                    r#"<ellipse cx="{}" cy="{}" rx="16" ry="6" fill="{hl_c}" opacity="0.07"/>"#,
                    cx - 2.0, hair_outer_top - 8.0
                ));
            }
            8 => { // Long / flowing — past ears
                s.push_str(&format!(
                    r#"<path d="M{} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {} {} L{} {} Q{} {} {} {} L{hl} {} Q{} {} {} {}Z" fill="{hair}"/>"#,
                    hl - 6.0, cy_cheek + 14.0, ht + 14.0, hl + 8.0, hair_inner_top - 10.0, hair_inner_top - 10.0,
                    hr - 8.0, hair_inner_top - 10.0, ht + 14.0, hr + 6.0, cy_cheek + 14.0,
                    hr + 6.0, hair_outer_top - 20.0, hr - 4.0, hair_outer_top - 30.0, cx, hair_outer_top - 30.0,
                    hair_outer_top - 20.0, hl + 4.0, hair_outer_top - 30.0, hl - 6.0, hair_outer_top - 20.0,
                ));
                // Hair strands
                s.push_str(&format!(
                    r#"<path d="M{} {} Q{cx} {} {} {}" stroke="{hl_c}" stroke-width="0.6" fill="none" opacity="0.08"/>"#,
                    hl + 4.0, hair_outer_top - 4.0, hair_outer_top - 14.0, hr - 4.0, hair_outer_top - 4.0
                ));
                // Side strand shadows
                for sx in [hl - 4.0, hr + 4.0] {
                    s.push_str(&format!(
                        r#"<path d="M{sx} {} L{sx} {}" stroke="{hd}" stroke-width="0.4" fill="none" opacity="0.10"/>"#,
                        cy_cheek - 6.0, cy_cheek + 12.0
                    ));
                }
            }
            9 => { // Fade / undercut — short sides, longer top
                // Top volume
                s.push_str(&format!(
                    r#"<path d="M{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {} L{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {}Z" fill="{hair}"/>"#,
                    hl + 6.0, cy_cheek - 18.0, hl + 6.0, ht + 20.0, hl + 14.0, hair_inner_top - 2.0, hair_inner_top - 2.0,
                    hr - 14.0, hair_inner_top - 2.0, hr - 6.0, ht + 20.0, hr - 6.0, cy_cheek - 18.0,
                    hr - 6.0, hair_outer_top - 6.0, hr - 6.0, ht + 12.0, hr - 12.0, hair_outer_top - 16.0, hair_outer_top - 16.0,
                    hl + 12.0, hair_outer_top - 16.0, hl + 6.0, ht + 12.0, hl + 6.0, hair_outer_top - 6.0,
                ));
                // Faded sides — gradient opacity
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {} {} L{} {} L{hl} {}Z" fill="{hair}" opacity="0.18"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hl + 14.0, ht + 8.0, hl + 6.0, ht + 8.0,
                    hl + 6.0, cy_cheek - 18.0, cy_cheek + 4.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} C{hr} {} {} {} {} {} L{} {} L{hr} {}Z" fill="{hair}" opacity="0.18"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hr - 14.0, ht + 8.0, hr - 6.0, ht + 8.0,
                    hr - 6.0, cy_cheek - 18.0, cy_cheek + 4.0
                ));
                // Fade line
                s.push_str(&format!(
                    r#"<path d="M{hl} {} L{} {}" stroke="{hd}" stroke-width="0.3" fill="none" opacity="0.12"/>"#,
                    cy_cheek - 6.0, hl + 6.0, cy_cheek - 18.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} L{} {}" stroke="{hd}" stroke-width="0.3" fill="none" opacity="0.12"/>"#,
                    cy_cheek - 6.0, hr - 6.0, cy_cheek - 18.0
                ));
            }
            10 => { // Mohawk — narrow strip, clean faded sides
                // Central strip
                s.push_str(&format!(
                    r#"<path d="M{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {} L{} {} C{} {} {} {} {cx} {} C{} {} {} {} {} {}Z" fill="{hair}"/>"#,
                    cx - 14.0, cy_cheek - 22.0, cx - 14.0, ht + 10.0, cx - 10.0, hair_outer_top - 6.0, hair_outer_top - 6.0,
                    cx + 10.0, hair_outer_top - 6.0, cx + 14.0, ht + 10.0, cx + 14.0, cy_cheek - 22.0,
                    cx + 14.0, hair_outer_top - 10.0, cx + 12.0, ht + 4.0, cx + 8.0, hair_outer_top - 16.0, hair_outer_top - 16.0,
                    cx - 8.0, hair_outer_top - 16.0, cx - 12.0, ht + 4.0, cx - 14.0, hair_outer_top - 10.0,
                ));
                // Faded sides — very transparent
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {} {} L{} {} L{hl} {}Z" fill="{hair}" opacity="0.12"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hl + 14.0, ht + 8.0, cx - 14.0, ht + 8.0,
                    cx - 14.0, cy_cheek - 22.0, cy_cheek + 4.0
                ));
                s.push_str(&format!(
                    r#"<path d="M{hr} {} C{hr} {} {} {} {} {} L{} {} L{hr} {}Z" fill="{hair}" opacity="0.12"/>"#,
                    cy_cheek - 6.0, ht + 18.0, hr - 14.0, ht + 8.0, cx + 14.0, ht + 8.0,
                    cx + 14.0, cy_cheek - 22.0, cy_cheek + 4.0
                ));
            }
            _ => { // Cornrows / tight braids
                s.push_str(&format!(
                    r#"<path d="M{hl} {} C{hl} {} {} {} {cx} {} C{} {} {hr} {} {hr} {} L{hr} {} C{hr} {} {} {} {cx} {} C{} {} {hl} {} {hl} {}Z" fill="{hair}"/>"#,
                    cy_cheek - 10.0, ht + 14.0, hl + 10.0, hair_inner_top - 4.0, hair_inner_top - 4.0,
                    hr - 10.0, hair_inner_top - 4.0, ht + 14.0, cy_cheek - 10.0,
                    hair_outer_top - 8.0, ht + 6.0, hr - 8.0, hair_outer_top - 16.0, hair_outer_top - 16.0,
                    hl + 8.0, hair_outer_top - 16.0, ht + 6.0, hair_outer_top - 8.0,
                ));
                // Cornrow lines — evenly spaced curves
                for rx in [-16.0f32, -8.0, 0.0, 8.0, 16.0] {
                    s.push_str(&format!(
                        r#"<path d="M{} {} Q{} {} {} {}" stroke="{hd}" stroke-width="0.6" fill="none" opacity="0.18"/>"#,
                        cx + rx, hair_outer_top - 10.0,
                        cx + rx * 0.95, (hair_outer_top + cy_cheek - 14.0) / 2.0,
                        cx + rx * 0.85, cy_cheek - 14.0
                    ));
                }
            }
        }

        if hair_st != 6 {
            for i in 0..28 {
                let t = i as f32 / 27.0;
                let x0 = hl + 9.0 + t * (hr - hl - 18.0);
                let y0 = hair_outer_top - 7.0 + ((i * 7 + texture_seed) % 12) as f32;
                let x1 = x0 + (0.5 - t) * 6.0;
                let y1 = hair_inner_top + 7.0 + ((i * 5 + texture_seed) % 10) as f32;
                s.push_str(&format!(
                    r#"<path d="M{x0} {y0} Q{} {} {x1} {y1}" stroke="{}" stroke-width="0.38" fill="none" opacity="0.20" stroke-linecap="round"/>"#,
                    (x0 + x1) / 2.0, y0 - 4.0, if i % 3 == 0 { hd.clone() } else { h_mid.clone() }
                ));
            }
        }
    }

    // Skin texture overlay — breaks up flat plastic look
    s.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" filter="url(#tx)" opacity="0.03" fill="none"/>"#,
        hl - 10.0, ht - 5.0, (hr - hl) + 20.0, chy - ht + 15.0
    ));

    s.push_str("</svg>");
    s
}
