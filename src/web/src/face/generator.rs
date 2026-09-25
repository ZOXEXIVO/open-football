//! The portrait: one player, painted as a flat illustration.
//!
//! The page is `200 × 250` units with the head centred at x = 100, the eye
//! line at y = 118 and the chin at y ≈ 205, which is what the match viewer
//! projects the cutout by. Every part of the head — face, ears, neck, hair,
//! beard — is a shape in flat pigment with a few flat shapes of shade on it,
//! laid down back to front. Head and neck only: the man is fitted onto a
//! body elsewhere, and a body of his own would never sit on it.
//!
//! Everything is decided by the player id and his record. Same player, same
//! face, every render.

use std::sync::OnceLock;

use log::error;
use rayon::{ThreadPool, ThreadPoolBuilder};
use shared::{AppearanceRng, Palette, SkinDist};
use tokio::sync::oneshot;

use super::beard::FacialHair;
use super::canvas::{Canvas, Grid, Layer, Plane, Ramp};
use super::color::Linear;
use super::features::Features;
use super::geometry::Landmarks;
use super::hair::Hair;
use super::identity::Identity;
use super::noise::Noise;
use super::shading::Shade;
use super::tones::Tones;

/// What is drawn AROUND the head.
///
/// The head itself is identical either way — same rng stream, same features,
/// same shade, same framing — because the two are the same man seen in two
/// places.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FaceFrame {
    /// The profile-page portrait, on a studio card.
    Portrait,
    /// The head alone, on transparent ground.
    ///
    /// For the match viewer, which lays this over the front of a
    /// footballer's skull: a backdrop there would be a rectangle painted
    /// across his cheeks.
    Cutout,
}

impl FaceFrame {
    /// The file this frame is delivered as: a JPEG for the page, a picture
    /// with transparency for the viewer at the size it reads.
    pub fn mime(self) -> &'static str {
        match self {
            FaceFrame::Portrait => "image/jpeg",
            FaceFrame::Cutout => "image/png",
        }
    }
}

/// Everything about the player the portrait is taken of.
pub struct Sitter {
    pub player_id: u32,
    pub age: u8,
    pub skin: SkinDist,
    /// Weight-for-height deviation (≈ −2 lean .. +2.5 heavy): it fills the
    /// cheeks, jaw and neck instead of random width alone
    pub heft: f32,
    /// 0..1, from temperament and dirtiness: brows drop and knit, lids weigh
    /// down, mouth corners tighten
    pub aggression: f32,
}

pub struct Portrait;

impl Portrait {
    const QUALITY: u8 = 92;

    /// The picture, painted on the portraits' own threads: a page of thirty
    /// faces queues there instead of spreading over every core the
    /// simulation and the other requests are running on. `None` if the
    /// render failed.
    pub async fn commission(sitter: Sitter, frame: FaceFrame) -> Option<Vec<u8>> {
        let (tx, rx) = oneshot::channel();
        Self::pool().spawn(move || {
            // Nobody is waiting for a face whose page has already gone
            if !tx.is_closed() {
                let _ = tx.send(Self::take(&sitter, frame));
            }
        });
        rx.await.ok()
    }

    /// A quarter of the machine, never fewer than two threads.
    fn pool() -> &'static ThreadPool {
        static POOL: OnceLock<ThreadPool> = OnceLock::new();
        POOL.get_or_init(|| {
            let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
            ThreadPoolBuilder::new()
                .num_threads((cores / 4).max(2))
                .thread_name(|n| format!("portrait-{n}"))
                // A render that panics fails its own request, not the process
                .panic_handler(|_| error!("portrait render panicked"))
                .build()
                .expect("the portrait pool starts")
        })
    }

    /// The encoded picture. Both frames are painted at twice the size they
    /// are delivered at and averaged down, so every strand, lash and edge is
    /// anti-aliased by real coverage: the portrait at two pixels a unit, the
    /// cutout at the one the viewer reads it at.
    pub fn take(sitter: &Sitter, frame: FaceFrame) -> Vec<u8> {
        match frame {
            FaceFrame::Portrait => Self::render(sitter, frame, &Grid::new(4.0))
                .halved()
                .jpeg(Self::QUALITY),
            FaceFrame::Cutout => Self::render(sitter, frame, &Grid::new(2.0)).halved().png(),
        }
    }

    pub fn render(sitter: &Sitter, frame: FaceFrame, grid: &Grid) -> Canvas {
        let grid = *grid;
        let age = sitter.age;
        let heft = sitter.heft.clamp(-2.0, 2.5);
        let aggr = sitter.aggression.clamp(0.0, 1.0);
        let mut rng = AppearanceRng::new(sitter.player_id);

        // Nation-driven phenotype class: skin band, hair/eye palettes, eye
        // shape family, nose/lip/brow weights and beard density all follow it.
        // Drawn FIRST off the stream, and by the shared crate rather than here,
        // because the match viewer asks the same question about the same player
        // and has to get the same answer — see `shared::Appearance`.
        let id = Identity::draw(&mut rng, sitter.skin, age);
        let t = Tones::derive(
            Palette::SKIN[id.look.skin],
            Palette::HAIR[id.look.hair],
            Palette::EYES[id.look.eyes],
            id.morph.redness,
        );
        let l = Landmarks::new(&id, age, heft, aggr);
        let noise = Noise::new(sitter.player_id as u64 * 0x2545_F491 + id.seed as u64);

        let head = l.head.coverage(&grid);
        let neck = l.neck.coverage(&grid);
        let (ear_cover, ear_shade) = Features::ears(&grid, &l);
        let back = Hair::back(&grid, &l, &t, &id, &noise);
        let tufts = Hair::scalp(&grid, &l, &t, &id, &noise, age);
        let growth = FacialHair::growth(&grid, &l, &id, &t, &noise, age);
        let face_shade = Shade::face(&grid, &l, &id);
        let beard = FacialHair::kept(&grid, &l, &head, &face_shade, &id, &t, &noise);

        // The man is painted on a sheet of his own and laid in turned by his
        // photographic tilt about the chin
        let mut man = Canvas::new(grid);
        if let Some(back) = &back {
            man.over(&back.paint());
        }
        let neck_skin = Features::plain_skin(&grid, &neck, &t, &noise, 0.08);
        man.over(&Self::skin(
            &grid,
            &neck,
            &neck_skin,
            &Shade::neck(&grid, &l),
        ));
        // Cartilage is thin: an ear is redder and a shade deeper than the
        // cheek beside it
        let ear_skin: Vec<Linear> = Features::plain_skin(&grid, &ear_cover, &t, &noise, 0.6)
            .into_iter()
            .map(|c| c * 0.9)
            .collect();
        man.over(&Self::skin(&grid, &ear_cover, &ear_skin, &ear_shade));
        let face_skin =
            Features::complexion(&grid, &l, &id, &t, &noise, &head, &growth, aggr, id.grey);
        man.over(&Self::skin(&grid, &head, &face_skin, &face_shade));
        let opening = Features::openings(&grid, &l);
        let seeds = l.eyes.each_ref().map(|e| Features::eye_seed(&id, e.side));
        man.over(&Layer::paint(&grid, |i, j, x, y| {
            let k = j * grid.w + i;
            let open = opening.v[k] * head.v[k];
            if open <= 0.0 {
                return None;
            }
            let e = usize::from(x > l.cx);
            Some((Features::eye(x, y, &l.eyes[e], &t, &noise, seeds[e]), open))
        }));
        man.over(&Layer::paint(&grid, |_, _, x, y| {
            let e = usize::from(x > l.cx);
            Features::lashes(x, y, &l.eyes[e], &t, &noise, seeds[e])
                .map(|(lash, a)| (lash * 0.9, a))
        }));
        if let Some(beard) = &beard {
            man.over(&beard.paint());
        }
        for tuft in &tufts {
            man.over(&tuft.paint());
        }

        let mut canvas = Canvas::new(grid);
        if frame == FaceFrame::Portrait {
            canvas.over(&Self::backdrop(&grid));
        }
        canvas.lay(&man.rotated(id.tilt * 0.7, (l.cx, l.skull.chin)));
        canvas
    }

    /// A stretch of skin: its pigment under the flat shade laid on it.
    fn skin(grid: &Grid, cover: &Plane, albedo: &[Linear], shade: &Plane) -> Layer {
        Layer::paint(grid, |i, j, _, _| {
            let k = j * grid.w + i;
            let a = cover.v[k];
            (a > 0.0).then(|| (Shade::over(albedo[k], shade.v[k]), a))
        })
    }

    /// The studio card: near-white, falling off to grey at the edges.
    fn backdrop(grid: &Grid) -> Layer {
        let card = Linear::new(0.92, 0.92, 0.91);
        let edge = Linear::new(0.70, 0.70, 0.70);
        Layer::paint(grid, |_, _, x, y| {
            let r = (((x - 100.0) / 110.0).powi(2) + ((y - 110.0) / 140.0).powi(2)).sqrt();
            Some((card.mix(edge, Ramp::smooth(0.3, 1.3, r)), 1.0))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::color::Rgb;
    use super::*;
    use shared::{Appearance, Region, SkinBucket};

    fn sitter(player_id: u32, skin: SkinDist) -> Sitter {
        Sitter {
            player_id,
            age: 26,
            skin,
            heft: 0.4,
            aggression: 0.3,
        }
    }

    /// The tone this paints and the tone the match page sends to the replay
    /// viewer are meant to be one draw off one stream. The only thing holding
    /// them together is that `Appearance::draw` is the FIRST call made on the
    /// rng — slip anything in front of it and the two quietly diverge.
    ///
    /// Read off the picture itself: the cheek clear of the shade must come
    /// out nearer the palette entry the viewer was told than any entry more
    /// than one step away from it.
    #[test]
    fn the_portrait_paints_the_tone_the_viewer_is_told_about() {
        let nations = [
            SkinDist::pure(SkinBucket::Black, Region::SubSaharan),
            SkinDist::pure(SkinBucket::White, Region::NorthEurope),
            SkinDist::pure(SkinBucket::Metis, Region::EastAsia),
            SkinDist::pure(SkinBucket::Metis, Region::Andes),
            SkinDist::default(),
        ];
        for dist in nations {
            for player_id in [1u32, 7, 91, 4242, 900_001] {
                let told = Appearance::of(player_id, dist).skin;
                let canvas =
                    Portrait::render(&sitter(player_id, dist), FaceFrame::Cutout, &Grid::new(2.0));
                let g = canvas.grid;
                let (mut sum, mut n) = ([0.0f32; 3], 0.0);
                for j in g.rows(136.0, 146.0) {
                    for i in g.cols(66.0, 78.0) {
                        let px = canvas.developed(j * g.w + i);
                        for c in 0..3 {
                            sum[c] += px[c] as f32;
                        }
                        n += 1.0;
                    }
                }
                let mean = sum.map(|s| s / n);
                let nearest = (0..Palette::SKIN.len())
                    .min_by(|&a, &b| {
                        let d = |i: usize| {
                            let p = Rgb::hex(Palette::SKIN[i]);
                            (p.r - mean[0]).powi(2)
                                + (p.g - mean[1]).powi(2)
                                + (p.b - mean[2]).powi(2)
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .expect("the palette is not empty");
                assert!(
                    nearest.abs_diff(told) <= 1,
                    "player {player_id} was told tone {told} and painted {nearest} ({mean:?})"
                );
            }
        }
    }

    /// A cutout is the same head with nothing round it: no card and no
    /// shirt, transparent where they would be, and the man himself pixel
    /// for pixel the man on the profile page.
    #[test]
    fn a_cutout_is_the_portrait_with_the_setting_taken_away() {
        let dist = SkinDist::pure(SkinBucket::White, Region::WestEurope);
        for player_id in [7u32, 4242, 900_001] {
            let s = sitter(player_id, dist);
            let grid = Grid::new(2.0);
            let portrait = Portrait::render(&s, FaceFrame::Portrait, &grid);
            let cutout = Portrait::render(&s, FaceFrame::Cutout, &grid);
            let g = cutout.grid;
            assert_eq!(
                cutout.alpha(0),
                0.0,
                "the cutout is still painted on a card"
            );
            assert_eq!(
                cutout.alpha(g.len() - 1 - g.w / 5),
                0.0,
                "the cutout still has shoulders in it"
            );
            assert_eq!(portrait.alpha(0), 1.0, "the portrait lost its card");
            // Above the collar nothing of the setting stands in front of him
            for j in g.rows(30.0, 200.0) {
                for i in 0..g.w {
                    let k = j * g.w + i;
                    // Only where he covers the whole pixel: at a partly
                    // covered one the card behind rightly shows through
                    if cutout.alpha(k) >= 1.0 - 1e-5 {
                        let (a, b) = (cutout.developed(k), portrait.developed(k));
                        assert!(
                            (0..3).all(|c| a[c].abs_diff(b[c]) <= 1),
                            "player {player_id} is a different man at ({i}, {j}) once the setting is taken away: {a:?} against {b:?}"
                        );
                    }
                }
            }
        }
    }

    /// The files are what the page and the viewer expect: the viewer lays
    /// the cutout out by its 200 × 250 page.
    #[test]
    fn the_cutout_is_a_page_sized_png_and_the_portrait_a_jpeg() {
        let s = sitter(4242, SkinDist::default());
        let png = Portrait::take(&s, FaceFrame::Cutout);
        assert_eq!(&png[1..4], b"PNG");
        let dim = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        assert_eq!((dim(16), dim(20)), (200, 250));
        let jpeg = Portrait::take(&s, FaceFrame::Portrait);
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    }

    /// The landmarks the viewer projects by are where it thinks they are.
    #[test]
    fn the_landmarks_stay_where_the_viewer_expects() {
        for player_id in 1..200u32 {
            for (age, heft) in [(17u8, -2.0f32), (26, 0.0), (36, 2.5)] {
                let mut rng = AppearanceRng::new(player_id);
                let id = Identity::draw(&mut rng, SkinDist::default(), age);
                let l = Landmarks::new(&id, age, heft, 0.3);
                assert!((l.eye - 118.0).abs() < 1.0, "eye line drifted: {}", l.eye);
                assert!(
                    (202.0..=208.0).contains(&l.skull.chin),
                    "chin drifted: {}",
                    l.skull.chin
                );
                // The viewer's drawn framing puts the edge of the face 40
                // units out at the eye line — see `Framing::DRAWN`
                assert!(
                    (40.0..=60.0).contains(&l.half_width_at(l.eye)),
                    "face width drifted: {}",
                    l.half_width_at(l.eye)
                );
                // The silhouette narrows from the cheekbones to the chin
                assert!(l.skull.zygo >= l.skull.sub);
                assert!(l.skull.sub > l.skull.jaw);
                assert!(l.skull.jaw > l.skull.chin_half + 6.0);
            }
        }
    }

    /// Dev-only close-ups: a handful of men across the classes, ages and
    /// cuts, full size, as close_<n>.jpg in $FACE_PREVIEW_DIR.
    #[test]
    #[ignore]
    fn preview_close() {
        let Ok(dir) = std::env::var("FACE_PREVIEW_DIR") else {
            return;
        };
        let root = std::path::Path::new(&dir);
        let cast = [
            (
                SkinDist::pure(SkinBucket::White, Region::WestEurope),
                26u8,
                0u32,
            ),
            (SkinDist::pure(SkinBucket::Black, Region::SubSaharan), 26, 3),
            (SkinDist::pure(SkinBucket::Metis, Region::EastAsia), 31, 5),
            (SkinDist::pure(SkinBucket::Metis, Region::Mena), 31, 1),
            (
                SkinDist::pure(SkinBucket::White, Region::NorthEurope),
                21,
                2,
            ),
            (SkinDist::pure(SkinBucket::Metis, Region::Andes), 36, 6),
        ];
        for (n, (skin, age, i)) in cast.into_iter().enumerate() {
            let sitter = Sitter {
                player_id: 2_000_000_000u32 + age as u32 * 1000 + i * 77 + 13,
                age,
                skin,
                heft: -1.6 + i as f32 * 0.5,
                aggression: (i * 3 % 8) as f32 / 7.0,
            };
            std::fs::write(
                root.join(format!("close_{n}.jpg")),
                Portrait::take(&sitter, FaceFrame::Portrait),
            )
            .expect("write face");
            std::fs::write(
                root.join(format!("cutout_{n}.png")),
                Portrait::take(&sitter, FaceFrame::Cutout),
            )
            .expect("write cutout");
        }
    }

    /// Dev-only: the first player of every haircut, for a class with
    /// straight hair and one with afro-textured hair, as hair_<class>_<n>.jpg
    /// in $FACE_PREVIEW_DIR.
    #[test]
    #[ignore]
    fn preview_hair() {
        let Ok(dir) = std::env::var("FACE_PREVIEW_DIR") else {
            return;
        };
        let root = std::path::Path::new(&dir);
        for (class, skin) in [
            (
                "straight",
                SkinDist::pure(SkinBucket::White, Region::SouthEurope),
            ),
            (
                "afro",
                SkinDist::pure(SkinBucket::Black, Region::SubSaharan),
            ),
        ] {
            let mut found: Vec<crate::face::identity::HairStyle> = Vec::new();
            for player_id in 3_000_000_000u32..3_000_002_000 {
                let mut rng = AppearanceRng::new(player_id);
                let id = Identity::draw(&mut rng, skin, 29);
                if found.contains(&id.hair) {
                    continue;
                }
                found.push(id.hair);
                let sitter = Sitter {
                    player_id,
                    age: 29,
                    skin,
                    heft: 0.0,
                    aggression: 0.3,
                };
                std::fs::write(
                    root.join(format!("hair_{class}_{:?}.jpg", id.hair)),
                    Portrait::take(&sitter, FaceFrame::Portrait),
                )
                .expect("write face");
            }
        }
    }

    /// Dev-only contact sheet: one JPEG per face plus a faces.html grid in
    /// $FACE_PREVIEW_DIR for visual review.
    /// Run with:
    ///   FACE_PREVIEW_DIR=<dir> cargo test -p web --lib preview_contact_sheet -- --ignored
    #[test]
    #[ignore]
    fn preview_contact_sheet() {
        let Ok(dir) = std::env::var("FACE_PREVIEW_DIR") else {
            return;
        };
        let root = std::path::Path::new(&dir);

        // One section per phenotype showcase: pure-bucket dists route each
        // section straight into a single class via Phenotype::classify
        let dists = [
            (
                "west_european",
                SkinDist::pure(SkinBucket::White, Region::WestEurope),
            ),
            (
                "nordic",
                SkinDist::pure(SkinBucket::White, Region::NorthEurope),
            ),
            ("mena", SkinDist::pure(SkinBucket::Metis, Region::Mena)),
            (
                "west_african",
                SkinDist::pure(SkinBucket::Black, Region::SubSaharan),
            ),
            (
                "east_asian",
                SkinDist::pure(SkinBucket::Metis, Region::EastAsia),
            ),
            ("andean", SkinDist::pure(SkinBucket::Metis, Region::Andes)),
        ];
        let ages: [u8; 5] = [17, 21, 26, 31, 36];
        let only = std::env::var("FACE_PREVIEW_ONLY").ok();

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
            if only.as_deref().is_some_and(|o| o != dist_name) {
                continue;
            }
            html.push_str(&format!("<h2>{dist_name}</h2>"));
            for age in ages {
                html.push_str("<div class=\"row\">");
                for i in 0..8u32 {
                    let player_id = 2_000_000_000u32 + age as u32 * 1000 + i * 77 + 13;
                    // Sweep the build axis across each row: lean → heavy;
                    // scrambled aggression sweep so it decorrelates from heft
                    let sitter = Sitter {
                        player_id,
                        age,
                        skin: dist,
                        heft: -1.6 + i as f32 * 0.5,
                        aggression: (i * 3 % 8) as f32 / 7.0,
                    };
                    let fname = format!("face_{dist_name}_{age}_{i}.jpg");
                    std::fs::write(
                        root.join(&fname),
                        Portrait::take(&sitter, FaceFrame::Portrait),
                    )
                    .expect("write face");
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
                    "<div class=\"cell\"><img src=\"face_{dist_name}_26_{i}.jpg\"></div>"
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</body></html>");

        std::fs::write(root.join("faces.html"), html).expect("write contact sheet");
    }
}
