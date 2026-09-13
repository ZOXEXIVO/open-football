//! Where everything is on the page.
//!
//! The picture is a `0 0 200 250` box with the head centred on x=100. Three
//! landmarks are fixed by contract with the match viewer, which projects the
//! cutout onto a footballer's skull by them: the eye line at y=118, the chin
//! at y≈205 and the cheek about 50 units either side of the mid-line. Every
//! other measurement hangs off those by the classical proportions — the
//! face in thirds from hairline to brow to nose to chin, five eye-widths
//! across, the mouth as wide as the pupils are apart — and then departs from
//! them by the player's own [`Morph`].

use super::canvas::PathBuilder;
use super::identity::{Identity, Morph, Structure};

/// The head's half-widths at named heights, right side, in page units.
#[derive(Clone, Copy, Debug)]
pub struct Skull {
    pub crown: f32,
    pub parietal: f32,
    pub parietal_y: f32,
    pub temple: f32,
    pub temple_y: f32,
    pub zygo: f32,
    pub zygo_y: f32,
    pub sub: f32,
    pub sub_y: f32,
    pub jaw: f32,
    pub jaw_y: f32,
    pub chin_half: f32,
    pub chin: f32,
    /// Crown shoulders as (fraction of parietal, y below crown), twice:
    /// together they make the top of the head domed or flat
    pub crown_sh: [(f32, f32); 2],
    /// Chin curve: (extra half-width above the tip, tip width fraction) —
    /// a pointed chin against a broad blunt one
    pub chin_curve: (f32, f32),
}

impl Skull {
    /// The eight archetypes. Widths are half-widths.
    fn archetype(variant: usize) -> Skull {
        let base = |parietal, temple, zygo, sub, jaw, chin_half, jaw_y, chin, crown| Skull {
            crown,
            parietal,
            parietal_y: 88.0,
            temple,
            temple_y: 104.0,
            zygo,
            zygo_y: 130.0,
            sub,
            sub_y: 157.0,
            jaw,
            jaw_y,
            chin_half,
            chin,
            crown_sh: [(0.52, 4.5), (0.86, 18.0)],
            chin_curve: (4.5, 0.72),
        };
        match variant {
            // Oval
            0 => base(51.5, 50.0, 50.5, 45.5, 40.5, 19.5, 179.0, 205.0, 36.5),
            // Square
            1 => base(53.5, 52.5, 53.0, 50.5, 47.5, 26.0, 183.0, 203.5, 38.0),
            // Round
            2 => base(55.0, 53.5, 54.5, 51.5, 45.5, 23.5, 179.0, 202.5, 39.0),
            // Heart
            3 => base(54.5, 52.5, 51.5, 44.5, 37.0, 16.0, 176.0, 206.5, 36.0),
            // Oblong
            4 => base(48.5, 47.5, 48.0, 44.0, 40.0, 19.0, 184.0, 208.0, 32.5),
            // Diamond
            5 => base(48.5, 48.0, 53.0, 46.0, 38.0, 17.0, 180.0, 205.5, 35.5),
            // Triangle — narrow through the temples, all jaw
            6 => base(47.5, 47.0, 48.5, 46.5, 44.0, 24.5, 185.0, 204.0, 34.0),
            // Long angular — tall skull, late jaw corner
            _ => base(50.0, 49.0, 51.5, 45.5, 42.5, 21.0, 185.0, 207.5, 33.0),
        }
    }

    fn morphed(
        variant: usize,
        m: &Morph,
        st: &Structure,
        fw: f32,
        heft: f32,
        maturity: f32,
    ) -> Skull {
        let mut s = Skull::archetype(variant);

        // Scaffolding first: where the planes of THIS skull sit. High or
        // low cheekbones, a domed or flat crown, a forehead- or jaw-heavy
        // width gradient — the axes that used to be constants
        s.parietal_y += st.temple_dy * 3.0;
        s.temple_y += st.temple_dy * 3.5;
        s.zygo_y += st.zygo_dy * 5.0;
        s.sub_y += st.zygo_dy * 2.5 + st.sub_dy * 3.0;
        s.crown_sh = [
            (0.42 + st.crown_round * 0.18, 3.2 + st.crown_round * 2.2),
            (0.80 + st.crown_round * 0.10, 15.0 + st.crown_round * 5.0),
        ];
        s.chin_curve = (4.5 + st.chin_point * 1.8, 0.72 + st.chin_point * 0.12);
        s.parietal *= 1.0 + st.taper * 0.045;
        s.temple *= 1.0 + st.taper * 0.035;
        s.jaw *= 1.0 - st.taper * 0.04;
        s.chin_half *= 1.0 - st.taper * 0.03;

        // Bone: the skull morphs move bone; heft and age fill soft tissue.
        // A boy's jaw is still growing — it squares off through the twenties
        let grown = (maturity - 0.5) * 3.0;
        s.parietal += m.width * 1.2;
        s.temple += m.width * 1.2;
        s.zygo += m.width * 1.0 + m.cheek * 1.2 + fw * 0.2;
        s.sub += m.width * 0.9 + m.cheek * 0.4 + heft * 0.9 + fw * 0.45 + grown * 0.4;
        s.jaw += m.width * 0.7 + m.jaw * 2.6 + m.jaw_angle * 1.2 + heft * 1.4 + fw * 0.5 + grown;
        s.jaw_y += m.jaw_angle * 1.5 + m.length * 1.0;
        s.chin_half += m.chin_w * 2.0 + m.round * 0.5 + heft * 0.5 + grown * 0.7;
        s.crown -= m.length * 1.2;
        s.chin += m.length * 0.9;

        // Keep the silhouette an actual head: it widens from the temples to
        // the cheekbones and narrows from there to the chin, and the eye
        // line, chin and face width stay where the viewer expects them
        s.parietal = s.parietal.clamp(44.0, 58.0);
        s.temple = s.temple.min(s.parietal + 1.0).max(44.5);
        s.zygo = s
            .zygo
            .clamp((s.temple - 2.5).max(44.5), (s.parietal + 2.0).min(59.0));
        s.sub = s.sub.min(s.zygo - 1.0).max(34.0);
        s.jaw = s.jaw.clamp(28.0, s.sub - 2.0);
        s.chin_half = s.chin_half.clamp(11.0, s.jaw - 8.0);
        s.chin = s.chin.clamp(202.0, 208.0);
        s.jaw_y = s.jaw_y.clamp(174.0, s.chin - 18.0);
        s.crown = s.crown.clamp(31.0, 40.0);
        s
    }

    /// The silhouette as (y, half-width) stops, crown to chin. The single
    /// source both the outline and [`Landmarks::half_width_at`] read, so
    /// the two can never drift apart.
    fn stops(&self) -> [(f32, f32); 11] {
        [
            (self.crown, 0.0),
            (
                self.crown + self.crown_sh[0].1,
                self.parietal * self.crown_sh[0].0,
            ),
            (
                self.crown + self.crown_sh[1].1,
                self.parietal * self.crown_sh[1].0,
            ),
            (self.parietal_y, self.parietal),
            (self.temple_y, self.temple),
            (self.zygo_y, self.zygo),
            (self.sub_y, self.sub),
            (self.jaw_y, self.jaw),
            (self.chin - 7.5, self.chin_half + self.chin_curve.0),
            (self.chin - 1.4, self.chin_half * self.chin_curve.1),
            (self.chin, 0.0),
        ]
    }

    /// The right-hand outline from the crown down to the chin point. The
    /// left is the mirror, with the asymmetry the caller adds.
    fn right_side(&self, cx: f32, spread: f32) -> Vec<(f32, f32)> {
        self.stops()
            .iter()
            .map(|&(y, half)| (cx + half * spread, y))
            .collect()
    }
}

/// One eye's opening.
#[derive(Clone, Copy, Debug)]
pub struct EyeShape {
    pub rx: f32,
    pub ry: f32,
    pub iris_r: f32,
    pub pupil_r: f32,
    /// Lower lid roundness relative to the upper
    pub bottom: f32,
    pub crease: f32,
    pub lid_extra: f32,
    /// Effective canthal tilt (positive lifts the outer corner)
    pub tilt: f32,
    /// Where along the width the upper lid peaks, 0..1 from the inner corner
    pub peak: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct NoseShape {
    /// Bridge half-width at the eyes
    pub bridge: f32,
    /// Half-width across the alae
    pub tip: f32,
    /// Nostril length
    pub nostril: f32,
    /// Ball of the nose radius
    pub ball: f32,
    /// Dorsum profile: negative dips, positive humps
    pub hump: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MouthShape {
    /// Half-width
    pub half: f32,
    pub upper: f32,
    pub lower: f32,
    /// Cupid's bow depth
    pub bow: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BrowShape {
    pub len: f32,
    pub tilt: f32,
    pub arch: f32,
    pub strands: usize,
    pub thickness: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Ear {
    pub top: f32,
    pub bottom: f32,
    /// How far the helix stands clear of the head's edge at its widest
    pub width: f32,
    /// Extra stand-off for a man whose ears sit proud of his skull
    pub out: f32,
}

pub struct Landmarks {
    pub cx: f32,
    pub skull: Skull,
    pub hairline: f32,
    pub brow: f32,
    pub eye: f32,
    pub nose: f32,
    pub mouth: f32,
    pub sulcus: f32,
    pub head_path: String,
    pub neck_path: String,
    pub neck_half: f32,
    pub neck_top: f32,
    /// Eye centres, left and right on the page
    pub eye_l: f32,
    pub eye_r: f32,
    pub eye_shape: EyeShape,
    pub nose_shape: NoseShape,
    pub mouth_shape: MouthShape,
    pub brow_shape: BrowShape,
    pub ear: Ear,
    /// 0 at 17, 1 at 36+
    pub maturity: f32,
}

impl Landmarks {
    pub fn new(id: &Identity, age: u8, heft: f32, aggr: f32) -> Landmarks {
        let cx = 100.0f32;
        let m = &id.morph;
        let st = &id.structure;
        let maturity = ((age as f32 - 17.0) / 19.0).clamp(0.0, 1.0);
        let skull = Skull::morphed(id.face_var, m, st, id.fw, heft, maturity);

        // Outline: right side as drawn, left side mirrored and nudged by the
        // asymmetry and the turn so no head is a perfect reflection
        let turn = id.turn * 0.35;
        let right = skull.right_side(cx, 1.0 + turn * 0.09);
        let mut left = PathBuilder::mirrored(cx, &right);
        let left_n = left.len();
        for (i, p) in left.iter_mut().enumerate() {
            let t = i as f32 / (left_n - 1) as f32;
            p.0 = cx - (cx - p.0) * (1.0 - turn * 0.09) + id.asym.0 * t * 1.5;
        }
        // The mirrored run repeats the chin point at its start and the crown
        // point at its end; both are dropped so the outline stays one loop
        let mut pts = right;
        pts.extend(left.into_iter().skip(1).take(left_n - 2));
        let head_path = PathBuilder::smooth_closed(&pts, 0.05);

        let eye = 118.0 + id.asym.1 * 0.6 + st.eye_y * 0.3;
        let brow = 107.5 - m.brow_gap * 1.3 + aggr * 1.8 + id.asym.1 * 0.3 + st.brow_h * 1.7;
        let nose =
            (156.5 + m.nose_len * 2.8 + m.length * 0.5 + st.nose_y * 1.5).clamp(150.0, 163.0);
        let mouth = nose + (skull.chin - nose) * (0.36 + st.philtrum * 0.035 + m.lip * 0.01);
        let sulcus = mouth + (skull.chin - mouth) * 0.42;
        let recession = match age {
            0..=25 => 0.0,
            26..=30 => 1.5,
            31..=34 => 3.0,
            _ => 4.5,
        };
        let hairline = (64.0 - m.forehead * 3.5 + recession * 0.7 + (skull.crown - 36.0) * 0.6)
            .max(skull.crown + 18.0);

        // Eyes: an eye is a fifth of the face wide and the pair sit one eye
        // apart, which puts the centres about 20 units either side
        let eye_off = 20.0 + m.eye_spacing * 1.1;
        let eye_l = cx - eye_off + id.asym.0 * 0.8 + turn * 1.5;
        let eye_r = cx + eye_off - id.asym.0 * 0.4 + turn * 1.5;
        let (rx, ry, iris, pupil, bottom, crease, lid_extra, tilt_bias, peak): (
            f32,
            f32,
            f32,
            f32,
            f32,
            f32,
            f32,
            f32,
            f32,
        ) = match id.eye_st {
            // Standard almond
            0 => (9.6, 4.0, 4.15, 1.6, 0.66, 0.30, 0.0, 0.0, 0.42),
            // Hooded — fold droops over the outer lid, crease hidden
            1 => (9.6, 3.2, 4.0, 1.5, 0.58, 0.0, 1.0, -0.3, 0.40),
            // Big round, wide open
            2 => (9.9, 5.0, 4.4, 1.75, 0.95, 0.35, -0.5, 0.1, 0.46),
            // Monolid, upturned — flat lid, no crease, epicanthic fold
            3 => (10.4, 2.9, 4.0, 1.45, 0.55, 0.0, 0.7, 1.3, 0.50),
            // Deep-set — smaller opening in a strong socket
            4 => (9.0, 3.5, 3.9, 1.5, 0.66, 0.42, 0.4, -0.2, 0.42),
            // Thin — long and narrow, iris heavily cropped
            5 => (10.8, 2.3, 3.9, 1.4, 0.50, 0.15, 0.6, 0.4, 0.48),
            // Downturned — outer corners drop
            6 => (9.7, 3.9, 4.15, 1.6, 0.74, 0.30, 0.2, -1.6, 0.40),
            // Wide-open almond — tall, pointed corners
            _ => (10.2, 4.6, 4.3, 1.7, 0.84, 0.28, -0.4, 0.5, 0.44),
        };
        let es = m.eye_scale * 1.13;
        let eye_shape = EyeShape {
            rx: rx * es,
            ry: ry * es * 0.92 * (1.0 - aggr * 0.08),
            iris_r: iris * (0.5 + 0.5 * es),
            pupil_r: pupil * (0.5 + 0.5 * es),
            bottom,
            crease,
            lid_extra,
            tilt: m.eye_tilt + tilt_bias,
            peak,
        };

        let (bridge, tip, nostril, ball, hump): (f32, f32, f32, f32, f32) = match id.nose_st {
            // Narrow, straight
            0 => (3.2, 8.0, 2.4, 4.4, 0.0),
            // Broad, flat bridge
            1 => (4.8, 13.2, 4.2, 6.2, -0.5),
            // Medium
            2 => (4.0, 10.2, 3.3, 5.2, 0.1),
            // Fine, slightly upturned
            3 => (3.1, 7.8, 2.4, 4.7, -0.35),
            // Aquiline — strong bridge, hump
            4 => (4.4, 10.8, 3.4, 5.4, 1.1),
            // Roman, long
            _ => (3.9, 9.4, 3.0, 5.0, 0.5),
        };
        let nw = 1.0 + m.nose_w * 0.12 + id.fw * 0.03 + maturity * 0.05;
        let nose_shape = NoseShape {
            bridge: bridge * (1.0 + m.nose_w * 0.08),
            tip: tip * nw,
            nostril: nostril * nw,
            ball: ball * (1.0 + m.nose_w * 0.06 + maturity * 0.05),
            hump,
        };

        let (mh, upper, lower, bow): (f32, f32, f32, f32) = match id.mouth_st {
            0 => (19.0, 3.6, 5.4, 1.0),
            // Wide
            1 => (23.5, 3.2, 5.6, 0.7),
            // Small
            2 => (15.5, 3.9, 5.0, 1.3),
            // Full
            3 => (20.5, 5.2, 7.2, 1.1),
            // Thin
            _ => (18.0, 2.6, 4.2, 0.9),
        };
        let lip_k = 1.0 + m.lip * 0.22 - maturity * 0.12;
        let mouth_shape = MouthShape {
            half: (mh + 1.5 + m.mouth_w * 1.4 + id.fw * 0.25).clamp(14.5, 26.5),
            upper: upper * lip_k,
            lower: lower * lip_k,
            bow,
        };

        let (blen, btilt, barch, strands): (f32, f32, f32, usize) = match id.brow_st {
            0 => (14.0, 0.0, 1.6, 34),
            // High arch
            1 => (13.0, -0.5, 3.4, 34),
            // Long and flat
            2 => (16.0, 0.6, 1.1, 46),
            3 => (13.5, -0.2, 2.5, 36),
            // Bushy
            4 => (16.5, 0.1, 2.0, 56),
            // Short and sparse
            _ => (12.2, -0.3, 3.0, 26),
        };
        let brow_shape = BrowShape {
            len: blen,
            tilt: btilt,
            arch: barch,
            strands,
            thickness: (1.0 + m.brow_thick * 0.25).clamp(0.7, 1.4),
        };

        let ear = Ear {
            top: brow + 3.0 - m.ear * 1.2,
            bottom: nose - 8.0 + m.ear * 1.2 + maturity * 1.2,
            width: 9.6 + m.ear * 1.4 + maturity * 0.8,
            out: 2.4 + m.ear * 0.8,
        };

        // Neck: nearly as wide as the jaw on an athlete, and thickest of
        // all on a heavy build — the strongest weight cue in a head shot
        let neck_half = (skull.jaw * 0.92 + heft * 1.6 + 2.0).clamp(28.0, skull.jaw + 2.0);
        let neck_top = skull.chin - 24.0;
        let neck_path = format!(
            "M{:.1} {neck_top:.1} C{:.1} {:.1} {:.1} 232 {:.1} 250 L{:.1} 250 C{:.1} 232 {:.1} {:.1} {:.1} {neck_top:.1}Z",
            cx - neck_half,
            cx - neck_half,
            neck_top + 26.0,
            cx - neck_half - 3.0,
            cx - neck_half - 14.0,
            cx + neck_half + 14.0,
            cx + neck_half + 3.0,
            cx + neck_half,
            neck_top + 26.0,
            cx + neck_half,
        );

        Landmarks {
            cx,
            skull,
            hairline,
            brow,
            eye,
            nose,
            mouth,
            sulcus,
            head_path,
            neck_path,
            neck_half,
            neck_top,
            eye_l,
            eye_r,
            eye_shape,
            nose_shape,
            mouth_shape,
            brow_shape,
            ear,
            maturity,
        }
    }

    /// The head's half-width at height `y`, interpolated along the outline.
    pub fn half_width_at(&self, y: f32) -> f32 {
        let stops = self.skull.stops();
        if y <= stops[0].0 {
            return 0.0;
        }
        for w in stops.windows(2) {
            let (y0, w0) = w[0];
            let (y1, w1) = w[1];
            if y <= y1 {
                let t = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
                // Smoothstep so the interpolation follows the curve, not
                // the polygon
                let t = t * t * (3.0 - 2.0 * t);
                return w0 + (w1 - w0) * t;
            }
        }
        0.0
    }

    /// The x of the outline at `y`, on the given side (−1 left, +1 right).
    pub fn edge_x(&self, y: f32, side: f32) -> f32 {
        self.cx + side * self.half_width_at(y)
    }
}
