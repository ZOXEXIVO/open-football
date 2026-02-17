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
}

const SKIN: [&str; 8] = [
    "#FFDBB4", "#EDB98A", "#D08B5B", "#AE5D29",
    "#8D5524", "#694D3A", "#F1C27D", "#C68642",
];
const HAIR: [&str; 8] = [
    "#090806", "#2C222B", "#71635A", "#B7A69E",
    "#D6C4C2", "#CABFB1", "#A52A2A", "#E6BE8A",
];
const EYES: [&str; 6] = [
    "#634E34", "#2E536F", "#3D671D", "#1C7847",
    "#497665", "#7E8B92",
];

fn dk(hex: &str, f: f32) -> String {
    let h = hex.trim_start_matches('#');
    let r = (u8::from_str_radix(&h[0..2], 16).unwrap_or(128) as f32 * f).min(255.0) as u8;
    let g = (u8::from_str_radix(&h[2..4], 16).unwrap_or(128) as f32 * f).min(255.0) as u8;
    let b = (u8::from_str_radix(&h[4..6], 16).unwrap_or(128) as f32 * f).min(255.0) as u8;
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

fn lip(skin: &str) -> String {
    let h = skin.trim_start_matches('#');
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(160);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(120);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(100);
    format!("#{:02X}{:02X}{:02X}",
        ((r as f32 * 0.72) + 45.0).min(255.0) as u8,
        (g as f32 * 0.50).min(255.0) as u8,
        (b as f32 * 0.45).min(255.0) as u8,
    )
}

/// viewBox = "0 0 80 100" — portrait rectangle, head centered at x=40
pub fn generate_face_svg(player_id: u32, age: u8) -> String {
    let mut r = FaceRng::new(player_id);

    let skin = SKIN[r.range(SKIN.len())];
    let hair = HAIR[r.range(HAIR.len())];
    let eye  = EYES[r.range(EYES.len())];
    let hair_st = r.range(8);
    let brow_st = r.range(4);
    let eye_st  = r.range(3);
    let nose_st = r.range(3);
    let mouth_st= r.range(3);

    let (bc, mc): (u8, u8) = match age {
        0..=19  => (0, 0),
        20..=24 => (15, 10),
        25..=29 => (40, 30),
        30..=34 => (60, 45),
        _       => (75, 55),
    };
    let beard   = bc > 0 && r.chance(bc);
    let mstache = mc > 0 && r.chance(mc);
    let beard_v = r.range(3);
    let mst_v   = r.range(3);

    let sd  = dk(skin, 0.85);  // shadow
    let sd2 = dk(skin, 0.75);  // deeper shadow
    let hi  = dk(skin, 1.08);  // highlight
    let bg  = dk(skin, 0.42);

    let mut s = String::with_capacity(8192);

    // SVG header — portrait rectangle
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 100">"#);

    // Defs: skin gradient for 3D lighting
    s.push_str(&format!(
        r#"<defs><radialGradient id="sg" cx="45%" cy="38%" r="55%"><stop offset="0%" stop-color="{}"/><stop offset="70%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></radialGradient></defs>"#,
        hi, skin, sd
    ));

    // Background
    s.push_str(&format!(r#"<rect width="80" height="100" fill="{}"/>"#, bg));

    // === SHOULDERS ===
    s.push_str(&format!(
        r#"<ellipse cx="40" cy="104" rx="36" ry="18" fill="{}"/>"#, sd2
    ));
    s.push_str(&format!(
        r#"<ellipse cx="40" cy="104" rx="34" ry="16" fill="{}"/>"#, sd
    ));

    // === NECK ===
    s.push_str(&format!(
        r#"<rect x="32" y="76" width="16" height="18" rx="3" fill="{}"/>"#, skin
    ));
    s.push_str(&format!(
        r#"<rect x="33" y="78" width="14" height="14" rx="2" fill="{}" opacity="0.15"/>"#, sd
    ));
    // Neck side shadows
    s.push_str(&format!(
        r#"<path d="M32 78 L32 90 Q34 88 34 78Z" fill="{}" opacity="0.12"/>"#, sd2
    ));
    s.push_str(&format!(
        r#"<path d="M48 78 L48 90 Q46 88 46 78Z" fill="{}" opacity="0.12"/>"#, sd2
    ));

    // === EARS ===
    let ei = dk(skin, 0.80);
    s.push_str(&format!(r#"<ellipse cx="13" cy="50" rx="4" ry="6.5" fill="{}"/>"#, skin));
    s.push_str(&format!(r#"<ellipse cx="13.5" cy="50" rx="2" ry="4" fill="{}"/>"#, ei));
    s.push_str(&format!(r#"<ellipse cx="67" cy="50" rx="4" ry="6.5" fill="{}"/>"#, skin));
    s.push_str(&format!(r#"<ellipse cx="66.5" cy="50" rx="2" ry="4" fill="{}"/>"#, ei));

    // === HEAD — path with jaw, uses gradient fill ===
    s.push_str(
        r#"<path d="M16 46 C16 28 24 16 40 16 C56 16 64 28 64 46 C64 58 61 68 56 74 C51 79 46 80 40 80 C34 80 29 79 24 74 C19 68 16 58 16 46Z" fill="url(#sg)"/>"#,
    );
    // Forehead-to-temple shadow (left)
    s.push_str(&format!(
        r#"<path d="M18 38 C18 30 22 22 30 19 L20 30Z" fill="{}" opacity="0.07"/>"#, sd2
    ));
    // Forehead-to-temple shadow (right)
    s.push_str(&format!(
        r#"<path d="M62 38 C62 30 58 22 50 19 L60 30Z" fill="{}" opacity="0.07"/>"#, sd2
    ));
    // Cheekbone structure
    s.push_str(&format!(
        r#"<ellipse cx="22" cy="56" rx="5" ry="7" fill="{}" opacity="0.06"/>"#, sd2
    ));
    s.push_str(&format!(
        r#"<ellipse cx="58" cy="56" rx="5" ry="7" fill="{}" opacity="0.06"/>"#, sd2
    ));
    // Under-eye area
    s.push_str(&format!(
        r#"<ellipse cx="32" cy="50" rx="4" ry="2" fill="{}" opacity="0.04"/>"#, sd2
    ));
    s.push_str(&format!(
        r#"<ellipse cx="48" cy="50" rx="4" ry="2" fill="{}" opacity="0.04"/>"#, sd2
    ));
    // Jaw shadow
    s.push_str(&format!(
        r#"<path d="M22 68 Q32 78 40 79 Q48 78 58 68 Q50 76 40 77 Q30 76 22 68Z" fill="{}" opacity="0.1"/>"#, sd2
    ));
    // Nasolabial
    s.push_str(&format!(
        r#"<path d="M30 57 Q28 65 28 70" stroke="{}" stroke-width="0.4" fill="none" opacity="0.1"/>"#, sd2
    ));
    s.push_str(&format!(
        r#"<path d="M50 57 Q52 65 52 70" stroke="{}" stroke-width="0.4" fill="none" opacity="0.1"/>"#, sd2
    ));

    // === EYES ===
    // Positioned at y=47, lx=32, rx=48
    {
        let lx: f32 = 32.0;
        let rx: f32 = 48.0;
        let ey: f32 = 47.0;
        let lid = dk(skin, 0.68);
        let sclera = "#EDEAE8";

        match eye_st {
            0 => {
                // Standard
                s.push_str(&format!(r##"<ellipse cx="{lx}" cy="{ey}" rx="5" ry="2.8" fill="{sclera}"/>"##));
                s.push_str(&format!(r##"<ellipse cx="{rx}" cy="{ey}" rx="5" ry="2.8" fill="{sclera}"/>"##));
                s.push_str(&format!(r#"<circle cx="{lx}" cy="{ey}" r="2.2" fill="{}"/>"#, eye));
                s.push_str(&format!(r#"<circle cx="{rx}" cy="{ey}" r="2.2" fill="{}"/>"#, eye));
                s.push_str(&format!(r##"<circle cx="{lx}" cy="{ey}" r="1.0" fill="#1A1A1A"/>"##));
                s.push_str(&format!(r##"<circle cx="{rx}" cy="{ey}" r="1.0" fill="#1A1A1A"/>"##));
            }
            1 => {
                // Narrower
                s.push_str(&format!(r##"<ellipse cx="{lx}" cy="{ey}" rx="5" ry="2.2" fill="{sclera}"/>"##));
                s.push_str(&format!(r##"<ellipse cx="{rx}" cy="{ey}" rx="5" ry="2.2" fill="{sclera}"/>"##));
                s.push_str(&format!(r#"<circle cx="{lx}" cy="{ey}" r="1.8" fill="{}"/>"#, eye));
                s.push_str(&format!(r#"<circle cx="{rx}" cy="{ey}" r="1.8" fill="{}"/>"#, eye));
                s.push_str(&format!(r##"<circle cx="{lx}" cy="{ey}" r="0.8" fill="#1A1A1A"/>"##));
                s.push_str(&format!(r##"<circle cx="{rx}" cy="{ey}" r="0.8" fill="#1A1A1A"/>"##));
            }
            _ => {
                // Rounder
                s.push_str(&format!(r##"<ellipse cx="{lx}" cy="{ey}" rx="4.8" ry="3.2" fill="{sclera}"/>"##));
                s.push_str(&format!(r##"<ellipse cx="{rx}" cy="{ey}" rx="4.8" ry="3.2" fill="{sclera}"/>"##));
                s.push_str(&format!(r#"<circle cx="{lx}" cy="{}" r="2.4" fill="{}"/>"#, ey + 0.2, eye));
                s.push_str(&format!(r#"<circle cx="{rx}" cy="{}" r="2.4" fill="{}"/>"#, ey + 0.2, eye));
                s.push_str(&format!(r##"<circle cx="{lx}" cy="{ey}" r="1.1" fill="#1A1A1A"/>"##));
                s.push_str(&format!(r##"<circle cx="{rx}" cy="{ey}" r="1.1" fill="#1A1A1A"/>"##));
            }
        }
        // Upper eyelid
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="1.0" fill="none"/>"#,
            lx - 5.0, ey - 0.3, lx, ey - 3.5, lx + 5.0, ey - 0.3, lid
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="1.0" fill="none"/>"#,
            rx - 5.0, ey - 0.3, rx, ey - 3.5, rx + 5.0, ey - 0.3, lid
        ));
        // Eyelid crease
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="0.35" fill="none" opacity="0.2"/>"#,
            lx - 4.5, ey - 2.0, lx, ey - 5.0, lx + 4.5, ey - 2.0, lid
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="0.35" fill="none" opacity="0.2"/>"#,
            rx - 4.5, ey - 2.0, rx, ey - 5.0, rx + 4.5, ey - 2.0, lid
        ));
        // Lower lid
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="0.3" fill="none" opacity="0.15"/>"#,
            lx - 4.0, ey + 1.2, lx, ey + 2.8, lx + 4.0, ey + 1.2, lid
        ));
        s.push_str(&format!(
            r#"<path d="M{} {} Q{} {} {} {}" stroke="{}" stroke-width="0.3" fill="none" opacity="0.15"/>"#,
            rx - 4.0, ey + 1.2, rx, ey + 2.8, rx + 4.0, ey + 1.2, lid
        ));
        // Catchlight
        s.push_str(&format!(
            r#"<circle cx="{}" cy="{}" r="0.5" fill="white" opacity="0.5"/>"#, lx - 0.6, ey - 0.6
        ));
        s.push_str(&format!(
            r#"<circle cx="{}" cy="{}" r="0.5" fill="white" opacity="0.5"/>"#, rx - 0.6, ey - 0.6
        ));
    }

    // === EYEBROWS ===
    {
        let y: f32 = 41.5;
        match brow_st {
            0 => {
                s.push_str(&format!(
                    r#"<path d="M26 {} L38 {}" stroke="{}" stroke-width="1.6" fill="none" stroke-linecap="round"/>"#,
                    y, y - 0.5, hair
                ));
                s.push_str(&format!(
                    r#"<path d="M42 {} L54 {}" stroke="{}" stroke-width="1.6" fill="none" stroke-linecap="round"/>"#,
                    y - 0.5, y, hair
                ));
            }
            1 => {
                s.push_str(&format!(
                    r#"<path d="M26 {} Q32 {} 38 {}" stroke="{}" stroke-width="1.6" fill="none" stroke-linecap="round"/>"#,
                    y, y - 2.5, y, hair
                ));
                s.push_str(&format!(
                    r#"<path d="M42 {} Q48 {} 54 {}" stroke="{}" stroke-width="1.6" fill="none" stroke-linecap="round"/>"#,
                    y, y - 2.5, y, hair
                ));
            }
            2 => {
                s.push_str(&format!(
                    r#"<path d="M26 {} L38 {}" stroke="{}" stroke-width="1.8" fill="none" stroke-linecap="round"/>"#,
                    y + 1.0, y - 1.0, hair
                ));
                s.push_str(&format!(
                    r#"<path d="M42 {} L54 {}" stroke="{}" stroke-width="1.8" fill="none" stroke-linecap="round"/>"#,
                    y - 1.0, y + 1.0, hair
                ));
            }
            _ => {
                s.push_str(&format!(
                    r#"<path d="M25 {} Q32 {} 39 {}" stroke="{}" stroke-width="2.6" fill="none" stroke-linecap="round"/>"#,
                    y, y - 2.5, y, hair
                ));
                s.push_str(&format!(
                    r#"<path d="M41 {} Q48 {} 55 {}" stroke="{}" stroke-width="2.6" fill="none" stroke-linecap="round"/>"#,
                    y, y - 2.5, y, hair
                ));
            }
        }
    }

    // === NOSE ===
    {
        let ns = dk(skin, 0.70);
        let nt = dk(skin, 0.78);
        match nose_st {
            0 => {
                s.push_str(&format!(
                    r#"<path d="M40 38 L39 55 Q37 58 35 59" stroke="{}" stroke-width="0.6" fill="none" opacity="0.25"/>"#, ns
                ));
                s.push_str(&format!(
                    r#"<ellipse cx="40" cy="57.5" rx="4" ry="2" fill="{}" opacity="0.2"/>"#, nt
                ));
                s.push_str(&format!(r#"<ellipse cx="37" cy="58.5" rx="1.6" ry="0.9" fill="{}" opacity="0.18"/>"#, ns));
                s.push_str(&format!(r#"<ellipse cx="43" cy="58.5" rx="1.6" ry="0.9" fill="{}" opacity="0.18"/>"#, ns));
            }
            1 => {
                s.push_str(&format!(
                    r#"<path d="M40 39 L39 56 Q37 60 34 61" stroke="{}" stroke-width="0.6" fill="none" opacity="0.22"/>"#, ns
                ));
                s.push_str(&format!(
                    r#"<ellipse cx="40" cy="58" rx="5.5" ry="2.5" fill="{}" opacity="0.18"/>"#, nt
                ));
                s.push_str(&format!(r#"<ellipse cx="36" cy="59" rx="1.8" ry="1.0" fill="{}" opacity="0.2"/>"#, ns));
                s.push_str(&format!(r#"<ellipse cx="44" cy="59" rx="1.8" ry="1.0" fill="{}" opacity="0.2"/>"#, ns));
            }
            _ => {
                s.push_str(&format!(
                    r#"<path d="M40 38 L39.5 56 Q38 59 37 60" stroke="{}" stroke-width="0.5" fill="none" opacity="0.25"/>"#, ns
                ));
                s.push_str(&format!(
                    r#"<path d="M40 55 L37.5 59 Q40 61 42.5 59Z" fill="{}" opacity="0.12"/>"#, nt
                ));
                s.push_str(&format!(r#"<ellipse cx="37.5" cy="59.5" rx="1.3" ry="0.7" fill="{}" opacity="0.16"/>"#, ns));
                s.push_str(&format!(r#"<ellipse cx="42.5" cy="59.5" rx="1.3" ry="0.7" fill="{}" opacity="0.16"/>"#, ns));
            }
        }
    }

    // === MOUTH ===
    {
        let lp = lip(skin);
        let ld = dk(&lp, 0.78);
        match mouth_st {
            0 => {
                s.push_str(&format!(r#"<path d="M33 66 Q37 64.5 40 65.5 Q43 64.5 47 66" fill="{}" opacity="0.65"/>"#, lp));
                s.push_str(&format!(r#"<path d="M33 66 Q40 69.5 47 66" fill="{}" opacity="0.45"/>"#, lp));
                s.push_str(&format!(r#"<path d="M33 66 Q40 67 47 66" stroke="{}" stroke-width="0.5" fill="none" opacity="0.45"/>"#, ld));
            }
            1 => {
                s.push_str(&format!(r#"<path d="M32 66 Q36 64 40 65.5 Q44 64 48 66" fill="{}" opacity="0.65"/>"#, lp));
                s.push_str(&format!(r#"<path d="M32 66 Q40 70.5 48 66" fill="{}" opacity="0.45"/>"#, lp));
                s.push_str(&format!(r#"<path d="M32 66 Q40 67.5 48 66" stroke="{}" stroke-width="0.5" fill="none" opacity="0.45"/>"#, ld));
            }
            _ => {
                s.push_str(&format!(r#"<path d="M31 66 Q35 63.5 40 65.5 Q45 63.5 49 66" fill="{}" opacity="0.65"/>"#, lp));
                s.push_str(&format!(r#"<path d="M31 66 Q40 70.5 49 66" fill="{}" opacity="0.45"/>"#, lp));
                s.push_str(&format!(r#"<path d="M31 66 Q40 67 49 66" stroke="{}" stroke-width="0.5" fill="none" opacity="0.45"/>"#, ld));
            }
        }
    }

    // === BEARD ===
    if beard {
        match beard_v {
            0 => {
                s.push_str(&format!(
                    r#"<path d="M24 63 Q24 76 40 80 Q56 76 56 63 Q50 72 40 73 Q30 72 24 63Z" fill="{}" opacity="0.12"/>"#, hair
                ));
            }
            1 => {
                s.push_str(&format!(
                    r#"<path d="M22 61 Q22 76 40 82 Q58 76 58 61 Q54 72 40 74 Q26 72 22 61Z" fill="{}" opacity="0.4"/>"#, hair
                ));
            }
            _ => {
                s.push_str(&format!(
                    r#"<path d="M20 58 Q18 78 40 88 Q62 78 60 58 Q56 72 40 76 Q24 72 20 58Z" fill="{}" opacity="0.55"/>"#, hair
                ));
                s.push_str(&format!(
                    r#"<path d="M28 64 Q28 74 40 80 Q52 74 52 64 Q46 70 40 72 Q34 70 28 64Z" fill="{}" opacity="0.1"/>"#,
                    dk(hair, 1.15)
                ));
            }
        }
    }

    // === MUSTACHE ===
    if mstache {
        match mst_v {
            0 => {
                s.push_str(&format!(
                    r#"<path d="M34 64 Q37 63 40 64 Q43 63 46 64" stroke="{}" stroke-width="1.0" fill="none" stroke-linecap="round"/>"#, hair
                ));
            }
            1 => {
                s.push_str(&format!(
                    r#"<path d="M30 65 Q35 62 40 64 Q45 62 50 65 Q45 66 40 66 Q35 66 30 65Z" fill="{}" opacity="0.6"/>"#, hair
                ));
            }
            _ => {
                s.push_str(&format!(
                    r#"<path d="M32 63 Q36 61 40 63 Q44 61 48 63 L46 66 Q42 64 40 65 Q38 64 34 66Z" fill="{}" opacity="0.55"/>"#, hair
                ));
            }
        }
    }

    // === HAIR ===
    match hair_st {
        0 => {
            // Short cropped
            s.push_str(&format!(
                r#"<path d="M17 40 C17 20 26 12 40 12 C54 12 63 20 63 40 L63 32 C63 16 54 8 40 8 C26 8 17 16 17 32Z" fill="{}"/>"#, hair
            ));
        }
        1 => {
            // Side part
            s.push_str(&format!(
                r#"<path d="M15 42 C15 16 26 8 40 8 C54 8 65 16 65 42 L65 28 C65 12 54 4 40 4 C26 4 15 12 15 28Z" fill="{}"/>"#, hair
            ));
            s.push_str(&format!(
                r#"<path d="M15 30 C15 24 19 18 28 16 L15 24Z" fill="{}" opacity="0.5"/>"#, hair
            ));
        }
        2 => {
            // Medium textured
            s.push_str(&format!(
                r#"<path d="M15 38 C15 14 26 4 40 4 C54 4 65 14 65 38 L65 26 C65 10 54 2 40 2 C26 2 15 10 15 26Z" fill="{}"/>"#, hair
            ));
        }
        3 => {
            // Buzz
            s.push_str(&format!(
                r#"<path d="M18 40 C18 22 28 14 40 14 C52 14 62 22 62 40 L62 34 C62 18 52 10 40 10 C28 10 18 18 18 34Z" fill="{}"/>"#, hair
            ));
        }
        4 => {
            // Swept back
            s.push_str(&format!(
                r#"<path d="M14 40 C14 12 26 2 40 2 C54 2 66 12 66 40 L66 24 C66 8 54 0 40 0 C26 0 14 8 14 24Z" fill="{}"/>"#, hair
            ));
            s.push_str(&format!(
                r#"<path d="M14 26 C16 18 22 12 32 10 L16 20Z" fill="{}" opacity="0.35"/>"#, hair
            ));
        }
        5 => {
            // Afro
            s.push_str(&format!(
                r#"<ellipse cx="40" cy="22" rx="30" ry="20" fill="{}"/>"#, hair
            ));
        }
        6 => {
            // Bald
            s.push_str(&format!(
                r#"<path d="M20 34 C20 22 30 16 40 16 C50 16 60 22 60 34 L60 32 C60 20 50 14 40 14 C30 14 20 20 20 32Z" fill="{}" opacity="0.2"/>"#, hair
            ));
        }
        _ => {
            // Curly
            s.push_str(&format!(
                r#"<path d="M15 36 C15 12 26 2 40 2 C54 2 65 12 65 36 L65 24 C65 8 54 0 40 0 C26 0 15 8 15 24Z" fill="{}"/>"#, hair
            ));
            s.push_str(&format!(r#"<circle cx="28" cy="8" r="5" fill="{}"/>"#, hair));
            s.push_str(&format!(r#"<circle cx="40" cy="4" r="5" fill="{}"/>"#, hair));
            s.push_str(&format!(r#"<circle cx="52" cy="8" r="5" fill="{}"/>"#, hair));
        }
    }

    s.push_str("</svg>");
    s
}
