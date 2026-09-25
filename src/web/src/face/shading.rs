//! The skin: base tone, the light on it, and everything that lives in it.
//!
//! One key light stands upper-left of the camera, as in every studio head
//! shot the site serves. The face is painted the way a portrait painter
//! lights a head: a lateral falloff that turns the far side of the face
//! into shadow, a vertical one that tucks the jaw under, and then the
//! planes — brow ridge, sockets, the nose and its cast shadow, cheekbones
//! and the hollows under them, the chin — each a soft shape in the skin's
//! own warm shadow tone or the light's colour. Nothing is outlined, and
//! nothing is a grey multiply: shadow on skin is redder than the skin.
//!
//! Colour lives under the light: the blood in the cheeks, nose and ears,
//! the cooler lower face of a shaven man, the orbit, and the lines a face
//! earns with age.

use super::canvas::{Blur, Canvas, PathBuilder};
use super::color::Rgb;
use super::geometry::Landmarks;
use super::identity::Identity;
use super::tones::Tones;

pub struct Shading;

impl Shading {
    /// Gradients, filters and clips.
    pub fn defs(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, jersey: (Rgb, Rgb, Rgb)) {
        let seed_a = id.seed;
        let seed_b = id.seed + 7;
        let seed_c = id.seed + 13;
        let seed_d = id.seed + 29;
        let skin_mid = t.skin.mix(t.skin_hi, 0.35);
        let (j_light, j_base, j_dark) = jersey;
        let s = &l.skull;
        let shadow = t.skin_dk2.toward("#8A3E2A", 0.12);
        let core = t.skin_shadow;

        c.raw("<defs>");
        c.raw(&format!(
            r##"<radialGradient id="bgg" cx="50%" cy="38%" r="78%"><stop offset="0%" stop-color="#FCFCFB"/><stop offset="55%" stop-color="#F2F2F1"/><stop offset="100%" stop-color="#D6D6D5"/></radialGradient>
<radialGradient id="sg" cx="44%" cy="36%" r="74%"><stop offset="0%" stop-color="{skin_mid}"/><stop offset="38%" stop-color="{}"/><stop offset="78%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></radialGradient>
<linearGradient id="lat" gradientUnits="userSpaceOnUse" x1="{:.1}" y1="0" x2="{:.1}" y2="0"><stop offset="0%" stop-color="{shadow}" stop-opacity="0.30"/><stop offset="20%" stop-color="{shadow}" stop-opacity="0.0"/><stop offset="58%" stop-color="{shadow}" stop-opacity="0.0"/><stop offset="80%" stop-color="{shadow}" stop-opacity="0.40"/><stop offset="100%" stop-color="{core}" stop-opacity="0.70"/></linearGradient>
<linearGradient id="vert" gradientUnits="userSpaceOnUse" x1="0" y1="{:.1}" x2="0" y2="{:.1}"><stop offset="0%" stop-color="{}" stop-opacity="0.14"/><stop offset="30%" stop-color="{}" stop-opacity="0.0"/><stop offset="80%" stop-color="{shadow}" stop-opacity="0.0"/><stop offset="100%" stop-color="{core}" stop-opacity="0.40"/></linearGradient>
<linearGradient id="ng" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="{}"/><stop offset="45%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></linearGradient>
<linearGradient id="hg" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="{}"/><stop offset="40%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></linearGradient>
<linearGradient id="jg" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="{j_light}"/><stop offset="60%" stop-color="{j_base}"/><stop offset="100%" stop-color="{j_dark}"/></linearGradient>
<radialGradient id="scg" cx="50%" cy="45%" r="60%"><stop offset="0%" stop-color="{}"/><stop offset="70%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></radialGradient>
<radialGradient id="irg" cx="50%" cy="50%" r="50%"><stop offset="0%" stop-color="{}"/><stop offset="45%" stop-color="{}"/><stop offset="78%" stop-color="{}"/><stop offset="100%" stop-color="{}"/></radialGradient>
<radialGradient id="vig" cx="50%" cy="45%" r="72%"><stop offset="0%" stop-color="#000" stop-opacity="0"/><stop offset="75%" stop-color="#000" stop-opacity="0"/><stop offset="100%" stop-color="#000" stop-opacity="0.10"/></radialGradient>"##,
            t.skin.css(),
            t.skin_dk.css(),
            t.skin_dk2.css(),
            l.cx - s.parietal,
            l.cx + s.parietal,
            s.crown,
            s.chin + 2.0,
            t.skin_hi.css(),
            t.skin_hi.css(),
            t.skin_dk.css(),
            t.skin.css(),
            t.skin_dk.css(),
            t.hair_hi.css(),
            t.hair.css(),
            t.hair_dk.css(),
            t.sclera.css(),
            t.sclera.mix(t.sclera_dk, 0.35).css(),
            t.sclera_dk.css(),
            t.iris_hi.css(),
            t.iris.css(),
            t.iris_dk.css(),
            t.iris_rim.css(),
        ));
        c.raw(Blur::defs());

        // Textures. `gr` is film grain; `pore` a coarser mottle under it;
        // `stb` the speckle of cropped hair; `htx`/`hfx` displace a hair edge
        // so it never reads as a vector curve
        c.raw(&format!(
            r##"<filter id="gr" x="0" y="0" width="200" height="250" filterUnits="userSpaceOnUse"><feTurbulence type="fractalNoise" baseFrequency="1.1" numOctaves="2" seed="{seed_a}" result="n"/><feColorMatrix in="n" type="saturate" values="0" result="d"/><feComponentTransfer in="d" result="a"><feFuncA type="linear" slope="0.10" intercept="0"/></feComponentTransfer><feComposite in="a" in2="SourceGraphic" operator="in"/></filter>
<filter id="pore" x="0" y="0" width="200" height="250" filterUnits="userSpaceOnUse"><feTurbulence type="fractalNoise" baseFrequency="0.42" numOctaves="3" seed="{seed_d}" result="n"/><feColorMatrix in="n" type="saturate" values="0" result="d"/><feComponentTransfer in="d" result="a"><feFuncA type="linear" slope="0.11" intercept="-0.01"/></feComponentTransfer><feComposite in="a" in2="SourceGraphic" operator="in"/></filter>
<filter id="stb" x="-20%" y="-20%" width="140%" height="140%"><feTurbulence type="fractalNoise" baseFrequency="1.5" numOctaves="3" seed="{seed_b}" result="n"/><feColorMatrix in="n" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1.5 -0.38" result="a"/><feComposite in="SourceGraphic" in2="a" operator="in"/></filter>
<filter id="htx" x="-15%" y="-15%" width="130%" height="130%"><feTurbulence type="turbulence" baseFrequency="0.09 0.015" numOctaves="3" seed="{seed_c}" result="n"/><feDisplacementMap in="SourceGraphic" in2="n" scale="4" xChannelSelector="R" yChannelSelector="G"/></filter>
<filter id="hfx" x="-25%" y="-25%" width="150%" height="150%"><feTurbulence type="fractalNoise" baseFrequency="0.05" numOctaves="3" seed="{seed_c}" result="n"/><feDisplacementMap in="SourceGraphic" in2="n" scale="11" xChannelSelector="R" yChannelSelector="G"/></filter>"##
        ));
        c.raw(&format!(
            r#"<clipPath id="hc"><path d="{}"/></clipPath><clipPath id="nc"><path d="{}"/></clipPath>"#,
            l.head_path, l.neck_path
        ));
        c.raw("</defs>");
    }

    /// The studio card behind the man: near-white, falling off to grey at
    /// the edges, with his shadow thrown onto it by the key light.
    pub fn backdrop(c: &mut Canvas, l: &Landmarks) {
        c.rect(0.0, 0.0, 200.0, 250.0, "url(#bgg)", 1.0);
        c.ellipse(
            l.cx + 14.0,
            132.0,
            l.skull.parietal + 14.0,
            96.0,
            Rgb::hex("#6A6560"),
            0.16,
            Blur::Vast,
            0.0,
        );
    }

    /// The skin, lit. Everything here is clipped to the head silhouette.
    pub fn head(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, age: u8, heft: f32) {
        let cx = l.cx;
        let s = &l.skull;
        let m = &id.morph;
        let es = &l.eye_shape;
        let ns = &l.nose_shape;
        let maturity = l.maturity;

        c.fill_ref(&l.head_path, "sg", 1.0, Blur::Crisp);
        c.clip("hc");

        // ── Colour zoning: where the blood is ──────────────
        // Forehead and the bridge cooler and lighter; cheeks, nose tip, ears
        // and chin flushed; the lower face of a shaven man cooler still
        c.ellipse(
            cx,
            l.hairline + 22.0,
            34.0,
            20.0,
            t.skin_hi,
            0.16,
            Blur::Vast,
            0.0,
        );
        let flush = 0.42 + m.redness * 0.28;
        // The whole mid-face is warmer than the forehead
        c.ellipse(
            cx,
            l.eye + 22.0,
            s.zygo * 0.9,
            26.0,
            t.skin_warm,
            0.22,
            Blur::Vast,
            0.0,
        );
        for side in [-1.0f32, 1.0] {
            c.ellipse(
                cx + side * s.zygo * 0.58,
                s.zygo_y + 11.0,
                19.0,
                13.0,
                t.skin_warm,
                flush * 0.85,
                Blur::Vast,
                side * -12.0,
            );
        }
        c.circle(
            cx,
            l.nose - 3.0,
            ns.tip * 0.9,
            t.skin_warm,
            flush * 0.7,
            Blur::Broad,
        );
        c.ellipse(
            cx,
            s.chin - 9.0,
            10.0,
            7.0,
            t.skin_warm,
            flush * 0.35,
            Blur::Broad,
            0.0,
        );
        if age >= 21 && !id.phenotype.epicanthic() {
            let cool = 0.12 + maturity * 0.12;
            let beard_zone = format!(
                "M{:.1} {:.1} Q{cx:.1} {:.1} {:.1} {:.1} L{:.1} 230 L{:.1} 230Z",
                cx - s.sub - 4.0,
                s.sub_y - 6.0,
                l.nose + 10.0,
                cx + s.sub + 4.0,
                s.sub_y - 6.0,
                cx + s.jaw + 10.0,
                cx - s.jaw - 10.0,
            );
            c.fill(&beard_zone, t.skin_cool, cool, Blur::Vast);
        }
        // The orbit: a little bluer and darker, deepening with the years
        let orbit = 0.05 + maturity * 0.09 + m.lid_heavy * 0.04;
        for ex in [l.eye_l, l.eye_r] {
            c.ellipse(
                ex,
                l.eye + 3.0,
                12.0,
                7.0,
                t.skin_cool,
                orbit,
                Blur::Broad,
                0.0,
            );
        }

        // ── Form: the big turns of the head ────────────────
        // The far side of the face turns from the light, the near side
        // only just; the jaw tucks under into shadow, the forehead lifts
        c.rect(0.0, 0.0, 200.0, 250.0, "url(#lat)", 1.0);
        c.rect(0.0, 0.0, 200.0, 250.0, "url(#vert)", 1.0);
        // The front plane — forehead, nose, chin — stands forward of the
        // cheeks, and the cheekbones forward of the hollows below them
        c.ellipse(
            cx - 3.0,
            128.0,
            s.zygo * 0.42,
            62.0,
            t.skin_hi,
            0.12,
            Blur::Vast,
            0.0,
        );
        let ear_mid = (l.ear.top + l.ear.bottom) / 2.0;
        for side in [-1.0f32, 1.0] {
            // Temple hollow
            c.ellipse(
                cx + side * (s.temple - 6.0),
                s.temple_y - 4.0,
                7.0,
                13.0,
                t.skin_dk2,
                if side > 0.0 { 0.20 } else { 0.10 },
                Blur::Broad,
                0.0,
            );
            // The ear is already painted, behind this; the cheek in front
            // of its root drops into shadow, and without that contact the
            // ear reads as a shape laid on the silhouette rather than a
            // part of the head standing off it
            c.ellipse(
                l.edge_x(ear_mid, side) - side * 3.0,
                ear_mid,
                4.5,
                (l.ear.bottom - l.ear.top) * 0.42,
                t.skin_dk2,
                if side > 0.0 { 0.24 } else { 0.16 },
                Blur::Soft,
                0.0,
            );
            // Cheekbone catching light, and the plane under it dropping away
            let lit = if side < 0.0 { 0.28 } else { 0.10 } + m.cheek * 0.03;
            c.ellipse(
                cx + side * s.zygo * 0.60,
                s.zygo_y + 4.0,
                12.0,
                6.0,
                t.skin_hi,
                lit,
                Blur::Broad,
                side * -18.0,
            );
            let hollow = (0.12 + maturity * 0.08 + (if side > 0.0 { 0.10 } else { 0.0 })
                - heft * 0.035
                - m.cheek.min(0.0) * 0.03)
                .max(0.03);
            c.ellipse(
                cx + side * s.sub * 0.70,
                s.sub_y + 1.0,
                10.0,
                9.0,
                t.skin_dk2,
                hollow,
                Blur::Broad,
                side * -18.0,
            );
        }

        // ── Brow and sockets ───────────────────────────────
        for (ex, side) in [(l.eye_l, -1.0f32), (l.eye_r, 1.0)] {
            // The socket: a shallow bowl under the brow bone, its shadow
            // deepest under the ridge and against the nose
            let depth = 0.24 + m.lid_heavy * 0.12 + if id.eye_st == 4 { 0.12 } else { 0.0 };
            c.ellipse(
                ex,
                l.eye + 1.0,
                es.rx + 3.0,
                7.5,
                t.skin_dk2,
                depth,
                Blur::Broad,
                0.0,
            );
            let op = 0.16 + m.lid_heavy * 0.10 + if side > 0.0 { 0.08 } else { 0.0 };
            c.ellipse(
                ex,
                l.eye - es.ry - 2.5,
                es.rx + 3.5,
                3.4,
                t.skin_dk2,
                op,
                Blur::Soft,
                0.0,
            );
            c.ellipse(
                ex - side * (es.rx - 1.0),
                l.eye - 0.5,
                4.0,
                5.0,
                t.skin_dk2,
                0.22,
                Blur::Soft,
                0.0,
            );
            // The lid itself has a little light on it
            c.ellipse(
                ex - side * 1.0,
                l.eye - es.ry - 1.2,
                es.rx * 0.6,
                1.4,
                t.skin_hi,
                0.16,
                Blur::Fine,
                0.0,
            );
        }
        // Glabella and the brow bone above each eye catch the light
        c.ellipse(cx, l.brow + 2.0, 5.0, 5.0, t.skin_hi, 0.14, Blur::Soft, 0.0);

        // ── Nose ───────────────────────────────────────────
        // The bridge is a ridge: lit down its left edge, its right side a
        // plane in shadow, the ball a sphere with its own highlight and
        // underside, and the whole thing throws a soft shadow rightward
        let bridge_l = PathBuilder::smooth_closed(
            &[
                (cx - ns.bridge * 0.3, l.brow + 9.0),
                (cx + ns.bridge * 0.25, l.brow + 9.0),
                (cx + ns.bridge * 0.15, l.eye + 12.0),
                (cx + ns.bridge * 0.1, l.nose - 10.0),
                (cx - ns.bridge * 0.9, l.nose - 8.0),
                (cx - ns.bridge * 0.75, l.eye + 12.0),
            ],
            0.2,
        );
        let bridge_r = PathBuilder::smooth_closed(
            &[
                (cx + ns.bridge * 0.35, l.eye - 1.0),
                (cx + ns.bridge * 1.1, l.eye + 2.0),
                (cx + ns.bridge * 1.35 + ns.hump * 0.8, l.eye + 14.0),
                (cx + ns.tip * 0.95, l.nose - 4.0),
                (cx + ns.tip * 0.4, l.nose - 2.0),
                (cx + ns.bridge * 0.4, l.eye + 14.0),
            ],
            0.2,
        );
        c.fill(&bridge_r, t.skin_dk2, 0.34, Blur::Soft);
        c.fill(&bridge_r, t.skin_dk2, 0.16, Blur::Broad);
        c.fill(&bridge_l, t.skin_hi, 0.30, Blur::Soft);
        c.fill(&bridge_l, t.skin_spec, 0.14, Blur::Soft);
        if ns.hump > 0.05 {
            c.ellipse(
                cx - ns.bridge * 0.2,
                l.eye + 11.0,
                ns.bridge * 0.8,
                4.0,
                t.skin_hi,
                ns.hump * 0.25,
                Blur::Fine,
                0.0,
            );
        }
        // The ball
        c.circle(
            cx + 0.5,
            l.nose - ns.ball * 0.9,
            ns.ball * 1.3,
            t.skin_hi,
            0.24,
            Blur::Soft,
        );
        c.ellipse(
            cx + ns.ball * 0.8,
            l.nose - ns.ball * 0.45,
            ns.ball * 0.9,
            ns.ball * 1.0,
            t.skin_dk2,
            0.32,
            Blur::Soft,
            0.0,
        );
        c.ellipse(
            cx - 1.2,
            l.nose - ns.ball * 1.25,
            3.4,
            2.4,
            t.skin_spec,
            0.42,
            Blur::Fine,
            0.0,
        );
        // Alae: each wing a small sphere
        for side in [-1.0f32, 1.0] {
            let op = if side < 0.0 { 0.20 } else { 0.08 };
            c.circle(
                cx + side * ns.tip * 0.68,
                l.nose - 2.8,
                4.0,
                t.skin_hi,
                op,
                Blur::Fine,
            );
            c.ellipse(
                cx + side * ns.tip * 0.92,
                l.nose - 1.5,
                2.8,
                3.8,
                t.skin_dk2,
                0.24 + if side > 0.0 { 0.14 } else { 0.0 },
                Blur::Soft,
                0.0,
            );
        }
        // Base plane facing down, and the cast shadow to the lower right
        c.ellipse(
            cx + 1.5,
            l.nose + 3.8,
            ns.tip * 1.0,
            3.2,
            t.skin_shadow,
            0.46,
            Blur::Soft,
            0.0,
        );
        c.ellipse(
            cx + ns.tip * 0.8 + 1.5,
            l.nose + 2.0,
            ns.tip * 0.5,
            5.5,
            t.skin_dk2,
            0.28,
            Blur::Soft,
            -25.0,
        );

        // ── Mouth region and chin ──────────────────────────
        // Philtrum: two ridges with a groove, the upper lip's plane in
        // shadow under the nose, the chin a sphere with its light on top
        for side in [-1.0f32, 1.0] {
            c.stroke(
                &PathBuilder::line(
                    (cx + side * 2.4, l.nose + 4.0),
                    (cx + side * 3.0, l.mouth - l.mouth_shape.upper - 0.6),
                ),
                t.skin_hi,
                1.0,
                0.18,
                Blur::Fine,
            );
        }
        c.stroke(
            &PathBuilder::line(
                (cx, l.nose + 4.5),
                (cx, l.mouth - l.mouth_shape.upper - 1.0),
            ),
            t.skin_dk2,
            1.2,
            0.14,
            Blur::Fine,
        );
        c.ellipse(
            cx,
            l.nose + 7.0,
            l.mouth_shape.half * 0.8,
            3.5,
            t.skin_dk,
            0.16,
            Blur::Broad,
            0.0,
        );
        c.ellipse(
            cx - 2.0,
            s.chin - 12.0,
            8.0,
            5.0,
            t.skin_hi,
            0.16,
            Blur::Broad,
            0.0,
        );
        c.ellipse(
            cx - 2.0,
            s.chin - 11.0,
            4.0,
            2.5,
            t.skin_spec,
            0.12,
            Blur::Soft,
            0.0,
        );
        c.ellipse(
            cx,
            l.sulcus,
            l.mouth_shape.half * 0.6,
            3.0,
            t.skin_dk2,
            0.22,
            Blur::Soft,
            0.0,
        );

        // ── Jaw and the far side ───────────────────────────
        let jaw_d = PathBuilder::smooth_open(
            &[
                (cx - s.jaw + 2.0, s.jaw_y - 2.0),
                (cx - s.chin_half - 4.0, s.chin - 5.0),
                (cx, s.chin - 1.0),
                (cx + s.chin_half + 4.0, s.chin - 5.0),
                (cx + s.jaw - 2.0, s.jaw_y - 2.0),
            ],
            0.2,
        );
        c.stroke(&jaw_d, t.skin_dk2, 5.0, 0.28 + maturity * 0.10, Blur::Broad);
        c.ellipse(
            cx,
            s.chin + 1.0,
            s.chin_half + 8.0,
            5.5,
            t.skin_shadow,
            0.50,
            Blur::Broad,
            0.0,
        );
        c.ellipse(
            cx + s.zygo - 3.0,
            140.0,
            9.0,
            58.0,
            t.skin_dk2,
            0.26,
            Blur::Vast,
            0.0,
        );
        c.ellipse(
            cx + s.jaw - 1.0,
            s.jaw_y - 6.0,
            7.0,
            16.0,
            t.skin_shadow,
            0.20,
            Blur::Broad,
            0.0,
        );
        // A little rim from the fill light on the near edge
        c.ellipse(
            cx - s.zygo + 4.0,
            134.0,
            5.0,
            50.0,
            t.skin_hi,
            0.12,
            Blur::Vast,
            0.0,
        );

        // ── Sheen ──────────────────────────────────────────
        c.ellipse(
            cx - 6.0,
            l.hairline + 20.0,
            14.0,
            9.0,
            t.skin_spec,
            0.24,
            Blur::Broad,
            -10.0,
        );
        c.ellipse(
            cx - 9.0,
            l.hairline + 18.0,
            6.0,
            3.5,
            t.skin_spec,
            0.18,
            Blur::Soft,
            -10.0,
        );
        for side in [-1.0f32, 1.0] {
            let op = if side < 0.0 { 0.22 } else { 0.06 };
            c.ellipse(
                cx + side * s.zygo * 0.58,
                s.zygo_y + 2.0,
                8.0,
                3.5,
                t.skin_spec,
                op,
                Blur::Broad,
                side * -18.0,
            );
        }

        Self::age_lines(c, l, t, id, age);
        Self::marks(c, l, t, id);

        // ── Texture: mottle and grain ──────────────────────
        let pore_op = 0.75 + maturity * 0.25;
        c.raw(&format!(
            r##"<rect x="0" y="0" width="200" height="250" fill="{}" filter="url(#pore)" opacity="{pore_op:.2}"/>"##,
            t.skin_dk2
        ));
        c.raw(r##"<rect x="0" y="0" width="200" height="250" fill="#888" filter="url(#gr)"/>"##);

        c.close("g");
    }

    /// The lines a face earns.
    fn age_lines(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity, age: u8) {
        let cx = l.cx;
        let es = &l.eye_shape;
        let maturity = l.maturity;
        let wrinkle = match age {
            0..=25 => 0.0f32,
            26..=29 => 0.04,
            30..=33 => 0.09,
            34..=36 => 0.15,
            _ => 0.22,
        };

        // Nasolabial: from the alar groove down and out past the mouth
        // corner; present on everyone, a crease on the old
        let fold = 0.08 + wrinkle * 1.2;
        for side in [-1.0f32, 1.0] {
            let d = PathBuilder::smooth_open(
                &[
                    (cx + side * (l.nose_shape.tip * 0.9 + 1.0), l.nose - 1.0),
                    (cx + side * (l.mouth_shape.half + 2.5), l.mouth - 6.0),
                    (cx + side * (l.mouth_shape.half + 4.0), l.mouth + 4.0),
                ],
                0.2,
            );
            let op = fold * if side > 0.0 { 1.2 } else { 0.85 };
            c.stroke(&d, t.skin_dk2, 2.2, op, Blur::Soft);
            c.stroke(&d, t.skin_hi, 1.0, op * 0.35, Blur::Fine);
        }

        if wrinkle > 0.05 {
            // Forehead: three furrows following the brow's curve
            let top = l.hairline + 8.0;
            let span = (l.brow - 12.0) - top;
            for wi in 0..3 {
                let wy = top + span * (0.25 + wi as f32 * 0.28) + id.jitter_signed(wi, 3) * 1.2;
                let d =
                    PathBuilder::arc((cx - 21.0, wy + 1.5), (cx, wy - 2.5), (cx + 21.0, wy + 1.5));
                c.stroke(&d, t.skin_dk2, 0.9, wrinkle * 0.7, Blur::Fine);
                c.stroke(&d, t.skin_hi, 0.6, wrinkle * 0.3, Blur::Hair);
            }
        }
        if age >= 29 {
            // Crow's feet fanning from the outer corners
            for (ex, side) in [(l.eye_l, -1.0f32), (l.eye_r, 1.0)] {
                let x0 = ex + side * (es.rx + 1.5);
                for k in 0..3 {
                    let dy = (k as f32 - 1.0) * 2.2;
                    let d = PathBuilder::arc(
                        (x0, l.eye + dy),
                        (x0 + side * 3.0, l.eye + dy * 1.4),
                        (x0 + side * 5.5, l.eye + dy * 2.4 + 1.0),
                    );
                    c.stroke(&d, t.skin_dk2, 0.7, (wrinkle + 0.02) * 0.6, Blur::Fine);
                }
            }
        }
        if age >= 33 {
            // Marionette lines
            for side in [-1.0f32, 1.0] {
                let d = PathBuilder::arc(
                    (cx + side * (l.mouth_shape.half + 1.0), l.mouth + 2.0),
                    (cx + side * (l.mouth_shape.half + 3.0), l.sulcus + 4.0),
                    (cx + side * (l.mouth_shape.half + 1.0), l.skull.chin - 8.0),
                );
                c.stroke(&d, t.skin_dk2, 1.4, wrinkle * 0.5, Blur::Soft);
            }
        }
        // Under-eye: a bag with a crease under it, filling in with age
        let bag = 0.04 + maturity * 0.14 + id.morph.lid_heavy * 0.04;
        for ex in [l.eye_l, l.eye_r] {
            c.ellipse(
                ex,
                l.eye + es.ry + 3.5,
                es.rx * 0.8,
                2.2,
                t.skin_hi,
                bag * 0.6,
                Blur::Soft,
                0.0,
            );
            c.ellipse(
                ex,
                l.eye + es.ry + 6.0,
                es.rx * 0.9,
                2.0,
                t.skin_dk2,
                bag,
                Blur::Soft,
                0.0,
            );
        }
    }

    /// Freckles and the odd mole.
    fn marks(c: &mut Canvas, l: &Landmarks, t: &Tones, id: &Identity) {
        let cx = l.cx;
        let s = &l.skull;
        if id.morph.freckles > 0.05 && t.luma > 0.45 {
            let n = (id.morph.freckles * 50.0) as usize + 10;
            let col = t.skin_dk2.toward("#9A5A32", 0.4);
            for i in 0..n {
                // Across the nose and the tops of the cheeks
                let u = id.jitter_signed(i, 21);
                let v = id.jitter(i, 22);
                let fx = cx + u * s.zygo * 0.72;
                let fy = l.eye + 8.0 + v * 22.0 - (u * u) * 6.0;
                let r = 0.3 + id.jitter(i, 23) * 0.35;
                c.circle(fx, fy, r, col, 0.12 + id.jitter(i, 24) * 0.2, Blur::Hair);
            }
        }
        if id.marks == 4 {
            let mkx = cx + id.jitter_signed(11, 5) * 24.0;
            let mky = 130.0 + id.jitter(7, 9) * 48.0;
            c.circle(mkx, mky, 0.9, t.skin_shadow, 0.55, Blur::Hair);
            c.circle(mkx - 0.3, mky - 0.3, 0.4, t.skin_hi, 0.25, Blur::Hair);
        }
    }
}
