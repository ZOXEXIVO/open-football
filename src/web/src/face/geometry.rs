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
use super::identity::{Identity, Morph};

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
}

impl Skull {
    /// The six archetypes. Widths are half-widths.
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
        };
        match variant {
            // Oval
            0 => base(52.0, 50.5, 51.0, 46.0, 41.0, 20.0, 179.0, 205.0, 36.5),
            // Square
            1 => base(53.0, 52.0, 52.5, 49.5, 47.0, 25.0, 182.0, 204.0, 37.0),
            // Round
            2 => base(54.0, 52.5, 53.5, 50.5, 45.0, 23.0, 180.0, 203.0, 38.0),
            // Heart
            3 => base(54.0, 52.0, 51.5, 45.0, 38.0, 17.0, 177.0, 206.0, 36.5),
            // Oblong
            4 => base(50.0, 48.5, 49.0, 45.0, 41.5, 20.0, 183.0, 208.0, 34.0),
            // Diamond
            _ => base(49.0, 48.0, 52.5, 46.0, 39.0, 18.0, 180.0, 205.0, 36.0),
        }
    }

    fn morphed(variant: usize, m: &Morph, fw: f32, heft: f32, maturity: f32) -> Skull {
        let mut s = Skull::archetype(variant);
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
        // the cheekbones and narrows from there to the chin, and the chin
        // stays where the viewer expects it
        s.temple = s.temple.min(s.parietal + 1.0);
        s.zygo = s.zygo.clamp(s.temple - 2.5, s.parietal + 2.0);
        s.sub = s.sub.min(s.zygo - 1.0);
        s.jaw = s.jaw.clamp(28.0, s.sub - 2.0);
        s.chin_half = s.chin_half.clamp(11.0, s.jaw - 8.0);
        s.chin = s.chin.clamp(202.0, 208.0);
        s.jaw_y = s.jaw_y.clamp(174.0, s.chin - 18.0);
        s.crown = s.crown.clamp(31.0, 40.0);
        s
    }

    /// The right-hand outline from the crown down to the chin point. The
    /// left is the mirror, with the asymmetry the caller adds.
    fn right_side(&self, cx: f32, spread: f32) -> Vec<(f32, f32)> {
        let w = |half: f32| cx + half * spread;
        vec![
            (cx, self.crown),
            (w(self.parietal * 0.52), self.crown + 4.5),
            (w(self.parietal * 0.86), self.crown + 18.0),
            (w(self.parietal), self.parietal_y),
            (w(self.temple), self.temple_y),
            (w(self.zygo), self.zygo_y),
            (w(self.sub), self.sub_y),
            (w(self.jaw), self.jaw_y),
            (w(self.chin_half + 4.5), self.chin - 7.5),
            (w(self.chin_half * 0.72), self.chin - 1.4),
            (cx, self.chin),
        ]
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
        let maturity = ((age as f32 - 17.0) / 19.0).clamp(0.0, 1.0);
        let skull = Skull::morphed(id.face_var, m, id.fw, heft, maturity);

        // Outline: right side as drawn, left side mirrored and nudged by the
        // asymmetry and the turn so no head is a perfect reflection
        let turn = id.turn * 0.35;
        let right = skull.right_side(cx, 1.0 + turn * 0.06);
        let mut left = PathBuilder::mirrored(cx, &right);
        let left_n = left.len();
        for (i, p) in left.iter_mut().enumerate() {
            let t = i as f32 / (left_n - 1) as f32;
            p.0 = cx - (cx - p.0) * (1.0 - turn * 0.06) + id.asym.0 * t * 1.5;
        }
        // The mirrored run repeats the chin point at its start and the crown
        // point at its end; both are dropped so the outline stays one loop
        let mut pts = right;
        pts.extend(left.into_iter().skip(1).take(left_n - 2));
        let head_path = PathBuilder::smooth_closed(&pts, 0.05);

        let eye = 118.0 + id.asym.1 * 0.6;
        let brow = 107.5 - m.brow_gap * 1.3 + aggr * 1.8 + id.asym.1 * 0.3;
        let nose = (156.5 + m.nose_len * 2.8 + m.length * 0.5).clamp(150.0, 163.0);
        let mouth = nose + (skull.chin - nose) * (0.36 + m.lip * 0.01);
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
            0 => (3.4, 8.6, 2.6, 4.6, 0.0),
            // Broad, flat bridge
            1 => (4.6, 12.4, 3.9, 5.8, -0.5),
            // Medium
            2 => (4.0, 10.2, 3.3, 5.2, 0.1),
            // Fine, slightly upturned
            3 => (3.3, 8.2, 2.5, 4.9, -0.3),
            // Aquiline — strong bridge, hump
            4 => (4.3, 10.8, 3.4, 5.4, 0.9),
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
            1 => (22.0, 3.3, 5.6, 0.8),
            2 => (17.0, 3.9, 5.0, 1.2),
            3 => (20.0, 4.6, 6.6, 1.1),
            _ => (18.0, 3.0, 4.6, 0.9),
        };
        let lip_k = 1.0 + m.lip * 0.18 - maturity * 0.12;
        let mouth_shape = MouthShape {
            half: (mh + 1.5 + m.mouth_w * 1.4 + id.fw * 0.25).clamp(16.0, 25.0),
            upper: upper * lip_k,
            lower: lower * lip_k,
            bow,
        };

        let (blen, btilt, barch, strands): (f32, f32, f32, usize) = match id.brow_st {
            0 => (14.0, 0.0, 1.6, 34),
            1 => (13.5, -0.4, 3.0, 34),
            2 => (15.0, 0.5, 1.3, 42),
            3 => (13.5, -0.2, 2.5, 36),
            4 => (15.5, 0.1, 2.0, 50),
            _ => (13.0, -0.2, 2.8, 30),
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
        let s = &self.skull;
        let stops = [
            (s.crown, 0.0),
            (s.crown + 4.5, s.parietal * 0.52),
            (s.crown + 18.0, s.parietal * 0.86),
            (s.parietal_y, s.parietal),
            (s.temple_y, s.temple),
            (s.zygo_y, s.zygo),
            (s.sub_y, s.sub),
            (s.jaw_y, s.jaw),
            (s.chin - 7.5, s.chin_half + 4.5),
            (s.chin - 1.4, s.chin_half * 0.72),
            (s.chin, 0.0),
        ];
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
