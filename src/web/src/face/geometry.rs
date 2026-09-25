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

use super::canvas::{Outline, Path, Polyline};
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
        // A real jaw is about seven tenths of the cheekbones across
        s.jaw *= 0.93;

        // Keep the silhouette an actual head: it widens from the temples to
        // the cheekbones and narrows from there to the chin, and the eye
        // line, chin and face width stay where the viewer expects them
        // A little narrower through the vault and cheekbones than the old
        // drawn heads, so the eyes sit in a face of human proportions
        s.parietal = (s.parietal * 0.95).clamp(43.0, 56.0);
        s.zygo = (s.zygo * 0.96).clamp(44.0, (s.parietal + 2.0).min(57.0));
        // The temples run straight from the vault down to the cheekbones:
        // their hollow is a shadow on the face, never a waist in its outline
        let t = (s.temple_y - s.parietal_y) / (s.zygo_y - s.parietal_y);
        s.temple = s.parietal + (s.zygo - s.parietal) * t - 0.3;
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

/// One eye as the lids frame it.
pub struct Eye {
    /// −1 for the eye on the left of the page, +1 for the right
    pub side: f32,
    pub cx: f32,
    /// The almond between the lids, where the eyeball shows
    pub opening: Outline,
    /// The lid margins, both run from the inner corner to the outer
    pub upper: Polyline,
    pub lower: Polyline,
    /// The fold above the lid; a monolid has none
    pub crease: Option<Polyline>,
    pub inner: (f32, f32),
    pub outer: (f32, f32),
    /// Height of the upper lid at its peak
    pub top: f32,
    pub iris: (f32, f32),
    pub iris_r: f32,
    pub pupil_r: f32,
}

/// The mouth: the red of each lip.
pub struct Lips {
    pub upper: Outline,
    pub lower: Outline,
    /// How far the corners sit below (+) or above (−) the centre of the
    /// line between the lips
    pub corner_dy: f32,
}

impl Eye {
    /// The lids round one eye centred on `(ex, ey)`: an upper margin that
    /// peaks off-centre and a flatter lower one, meeting at canthi tilted
    /// by the shape's canthal tilt.
    fn frame(ex: f32, ey: f32, side: f32, es: &EyeShape, lid_heavy: f32) -> Eye {
        let (rx, ry) = (es.rx, es.ry);
        let inner = (ex - side * rx, ey + es.tilt * 0.35);
        let outer = (ex + side * rx, ey - es.tilt);
        let peak_x = inner.0 + side * 2.0 * rx * es.peak;
        let top = ey - ry;
        let low = (ex + side * rx * 0.1, ey + ry * es.bottom);
        // Lids arch over the ball: they leave the corners steeply and run
        // round over the iris, rather than meeting in two long points
        let upper = Path::from(inner)
            .cubic(
                (inner.0 + side * rx * 0.18, inner.1 - ry * 0.75),
                (peak_x - side * rx * 0.45, top),
                (peak_x, top),
            )
            .cubic(
                (peak_x + side * rx * 0.50, top),
                (outer.0 - side * rx * 0.12, outer.1 - ry * 0.60),
                outer,
            );
        let lower = Path::from(inner)
            .cubic(
                (inner.0 + side * rx * 0.20, inner.1 + ry * es.bottom * 0.60),
                (low.0 - side * rx * 0.45, low.1),
                low,
            )
            .cubic(
                (low.0 + side * rx * 0.45, low.1),
                (outer.0 - side * rx * 0.18, outer.1 + ry * es.bottom * 0.80),
                outer,
            );
        let upper = upper.polyline();
        let lower = lower.polyline();
        let mut ring: Vec<(f32, f32)> = (0..=24).map(|k| upper.at(k as f32 / 24.0)).collect();
        ring.extend((1..24).rev().map(|k| lower.at(k as f32 / 24.0)));

        let crease = (es.crease > 0.01).then(|| {
            let y = top - 3.0 - es.lid_extra * 0.4 - lid_heavy * 0.6;
            Path::from((inner.0 + side * 1.0, inner.1 - 1.0))
                .quad((peak_x, y - 1.2), (outer.0 + side * 1.0, outer.1 - 1.6))
                .polyline()
        });

        Eye {
            side,
            cx: ex,
            opening: Outline::polygon(ring),
            upper,
            lower,
            crease,
            inner,
            outer,
            top,
            // Looking a touch inward, like a pair of eyes fixed on the lens
            iris: (ex - side * 0.25, ey - 0.15),
            iris_r: es.iris_r,
            pupil_r: es.pupil_r,
        }
    }
}

impl Lips {
    fn frame(cx: f32, my: f32, ms: &MouthShape, corner_dy: f32) -> Lips {
        let half = ms.half;
        let lip_top = my - ms.upper;
        let upper = Path::from((cx - half, my + corner_dy))
            .quad((cx - half * 0.45, lip_top - 0.6), (cx - 3.4, lip_top))
            .quad((cx, lip_top + ms.bow), (cx + 3.4, lip_top))
            .quad(
                (cx + half * 0.45, lip_top - 0.6),
                (cx + half, my + corner_dy),
            )
            .quad((cx, my + 1.0), (cx - half, my + corner_dy))
            .outline();
        let side_y = my + 0.6 + corner_dy * 0.6;
        let lower = Path::from((cx - half + 1.2, side_y))
            .quad((cx, my + ms.lower * 2.15), (cx + half - 1.2, side_y))
            .quad((cx, my + 1.2), (cx - half + 1.2, side_y))
            .outline();
        Lips {
            upper,
            lower,
            corner_dy,
        }
    }
}

pub struct Landmarks {
    pub cx: f32,
    pub skull: Skull,
    pub hairline: f32,
    pub brow: f32,
    pub eye: f32,
    pub nose: f32,
    pub mouth: f32,
    pub head: Outline,
    pub neck: Outline,
    pub eyes: [Eye; 2],
    pub lips: Lips,
    pub neck_half: f32,
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
        let mut left: Vec<(f32, f32)> = right
            .iter()
            .rev()
            .map(|&(x, y)| (2.0 * cx - x, y))
            .collect();
        let left_n = left.len();
        for (i, p) in left.iter_mut().enumerate() {
            let t = i as f32 / (left_n - 1) as f32;
            p.0 = cx - (cx - p.0) * (1.0 - turn * 0.09) + id.asym.0 * t * 1.5;
        }
        // The mirrored run repeats the chin point at its start and the crown
        // point at its end; both are dropped so the outline stays one loop
        let mut pts = right;
        pts.extend(left.into_iter().skip(1).take(left_n - 2));
        let head = Outline::smooth(&pts, 0.05);

        let eye = 118.0 + id.asym.1 * 0.6 + st.eye_y * 0.3;
        let brow = 107.5 - m.brow_gap * 1.3 + aggr * 1.8 + id.asym.1 * 0.3 + st.brow_h * 1.7;
        let nose =
            (156.5 + m.nose_len * 2.8 + m.length * 0.5 + st.nose_y * 1.5).clamp(150.0, 163.0);
        let mouth = nose + (skull.chin - nose) * (0.36 + st.philtrum * 0.035 + m.lip * 0.01);
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
        let eye_off = 22.0 + m.eye_spacing * 1.0;
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
        let iris_r = iris * (0.55 + 0.55 * es);
        // A resting eye opens less far than its iris is wide: the upper lid
        // takes the top off the iris and the lower one touches its foot,
        // which is the difference between a man and a startled doll
        let open = ry * es * 0.93 * (1.0 - aggr * 0.08) * (1.0 + bottom);
        let ry = ry * es * 0.93 * (1.0 - aggr * 0.08) * (iris_r * 1.85 / open).min(1.0);
        let eye_shape = EyeShape {
            rx: rx * es,
            ry,
            iris_r,
            pupil_r: pupil * (0.5 + 0.5 * es),
            bottom,
            crease,
            lid_extra,
            tilt: m.eye_tilt + tilt_bias,
            peak,
        };

        let (bridge, tip, nostril, ball): (f32, f32, f32, f32) = match id.nose_st {
            // Narrow, straight
            0 => (3.2, 8.0, 2.4, 4.4),
            // Broad, flat bridge
            1 => (4.8, 13.2, 4.2, 6.2),
            // Medium
            2 => (4.0, 10.2, 3.3, 5.2),
            // Fine, slightly upturned
            3 => (3.1, 7.8, 2.4, 4.7),
            // Aquiline — strong bridge
            4 => (4.4, 10.8, 3.4, 5.4),
            // Roman, long
            _ => (3.9, 9.4, 3.0, 5.0),
        };
        let nw = 1.0 + m.nose_w * 0.12 + id.fw * 0.03 + maturity * 0.05;
        let nose_shape = NoseShape {
            bridge: bridge * (1.0 + m.nose_w * 0.08),
            tip: tip * nw,
            nostril: nostril * nw,
            ball: ball * (1.0 + m.nose_w * 0.06 + maturity * 0.05),
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

        // Neck: an athlete's is most of the width of his face, and thickest
        // of all on a heavy build — the strongest weight cue in a head shot
        let neck_half = (skull.zygo * 0.84 + heft * 1.6).clamp(36.0, skull.zygo * 0.94);
        // It starts up behind the ears, where the head hides where it begins
        let neck_top = skull.jaw_y - 30.0;
        // A column, a touch wider at its foot, running on past the bottom
        // of the page so it ends in the frame rather than in an edge
        let chin = skull.chin;
        let foot = |s: f32| {
            [
                (cx + s * neck_half, neck_top),
                (cx + s * neck_half, chin - 4.0),
                (cx + s * (neck_half + 1.5), chin + 20.0),
                (cx + s * (neck_half + 3.0), chin + 60.0),
            ]
        };
        let mut rim = Path::through(&foot(-1.0), 0.1).points();
        rim.extend(Path::through(&foot(1.0), 0.1).points().into_iter().rev());
        let neck = Outline::polygon(rim);

        let eyes = [(eye_l, -1.0), (eye_r, 1.0)]
            .map(|(ex, side)| Eye::frame(ex, eye, side, &eye_shape, id.morph.lid_heavy));
        let lips = Lips::frame(
            cx,
            mouth,
            &mouth_shape,
            (aggr * 1.3 - 0.9 - st.smile * 1.0).clamp(-2.2, 1.6),
        );

        Landmarks {
            cx,
            skull,
            hairline,
            brow,
            eye,
            nose,
            mouth,
            head,
            neck,
            eyes,
            lips,
            neck_half,
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
