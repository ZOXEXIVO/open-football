//! The portrait: one player, drawn as a studio head shot.
//!
//! `viewBox = "0 0 200 250"` — a portrait rectangle, head centred at x=100,
//! eye line at y=118 and the chin at y≈205, which is what the match viewer
//! projects the cutout by. The picture is built in the order a camera would
//! see it: what hangs behind the head, the neck, the ears, the lit skin,
//! the features on it, facial hair, scalp hair, and — for the profile page
//! only — shoulders in the club's shirt on a studio card.
//!
//! Everything is decided by the player id and his record. Same player, same
//! face, every render.

use shared::{AppearanceRng, Palette, SkinDist};

use super::beard::FacialHair;
use super::body::Body;
use super::canvas::Canvas;
use super::color::Rgb;
use super::features::Features;
use super::geometry::Landmarks;
use super::hair::Hair;
use super::identity::Identity;
use super::shading::Shading;
use super::tones::Tones;

/// What is drawn AROUND the head.
///
/// The head itself is identical either way — same rng stream, same features,
/// same tone — because the two are the same man seen in two places.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FaceFrame {
    /// The profile-page portrait: studio card, shoulders in a jersey.
    Portrait,
    /// The head alone, on transparent ground.
    ///
    /// For the match viewer, which lays this over the front of a
    /// footballer's skull: a backdrop there would be a rectangle painted
    /// across his cheeks, and shoulders would be a second pair under the
    /// ones he already has.
    Cutout,
}

impl FaceFrame {
    fn cutout(self) -> bool {
        self == FaceFrame::Cutout
    }
}

/// `heft` is the player's weight-for-height deviation (≈ -2 lean .. +2.5
/// heavy): it fills the cheeks/jaw/neck instead of random width alone.
///
/// `aggression` (0..1, from temperament/dirtiness) hardens the expression:
/// brows drop and knit, lids weigh down, mouth corners tighten.
///
/// `jersey` is the club's background colour ("#rrggbb"); None falls back to
/// a deterministic per-player hue.
pub fn generate_face_svg(
    player_id: u32,
    age: u8,
    skin_dist: SkinDist,
    heft: f32,
    aggression: f32,
    jersey: Option<&str>,
    frame: FaceFrame,
) -> String {
    let heft = heft.clamp(-2.0, 2.5);
    let aggr = aggression.clamp(0.0, 1.0);
    let mut rng = AppearanceRng::new(player_id);

    // Nation-driven phenotype class: skin band, hair/eye palettes, eye
    // shape family, nose/lip/brow weights and beard density all follow it.
    // Drawn FIRST off the stream, and by the shared crate rather than here,
    // because the match viewer asks the same question about the same player
    // and has to get the same answer — see `shared::Appearance`.
    let id = Identity::draw(&mut rng, skin_dist, age);
    let tones = Tones::derive(
        Palette::SKIN[id.look.skin],
        Palette::HAIR[id.look.hair],
        Palette::EYES[id.look.eyes],
        id.morph.redness,
    );
    let l = Landmarks::new(&id, age, heft, aggr);

    // Jersey: real club colour when provided; otherwise a deterministic
    // per-player hue (wrapping_mul so large generated ids don't overflow)
    let jersey = match jersey {
        Some(bg) => {
            let base = Rgb::hex(bg);
            (base.lift(0.15), base, base.shade(0.55))
        }
        None => {
            let hue = (player_id.wrapping_mul(137) % 360) as f32;
            (
                Rgb::hsl(hue, 0.26, 0.41),
                Rgb::hsl(hue, 0.30, 0.30),
                Rgb::hsl(hue, 0.32, 0.19),
            )
        }
    };

    let mut c = Canvas::new();
    c.raw(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 250">"#);
    // Debug trace of the sampled variants (invisible; keeps visual QA cheap)
    c.raw(&format!(
        "<!--h{} e{} f{} n{} b{} m{} w{heft:.1} a{aggr:.1} p{}-->",
        id.hair.code(),
        id.eye_st,
        id.face_var,
        id.nose_st,
        id.beard.map(|b| b as u8).unwrap_or(9),
        id.moustache.map(|m| m as u8).unwrap_or(9),
        id.phenotype as u8,
    ));

    Shading::defs(&mut c, &l, &tones, &id, jersey);

    if !frame.cutout() {
        c.raw(r#"<g id="bg">"#);
        Shading::backdrop(&mut c, &l);
        c.close("g");
    }

    // The head, with its slight photographic tilt
    c.raw(&format!(
        r#"<g transform="rotate({:.2} 100 205)">"#,
        id.tilt
    ));
    Hair::back(&mut c, &l, &tones, &id);
    Body::neck(&mut c, &l, &tones);
    Features::ears(&mut c, &l, &tones);
    Shading::head(&mut c, &l, &tones, &id, age, heft);
    Features::eyes(&mut c, &l, &tones, &id, aggr);
    Features::brows(&mut c, &l, &tones, &id, aggr, tones.hair_greyed(id.grey));
    Features::nose(&mut c, &l, &tones);
    Features::mouth(&mut c, &l, &tones, &id, aggr);
    FacialHair::paint(&mut c, &l, &tones, &id, age);
    Hair::scalp(&mut c, &l, &tones, &id, age);
    c.close("g");

    // Everything from here down is the SETTING rather than the man, so a
    // cutout stops at the closing tag: no shoulders, no collar, no card
    if frame.cutout() {
        c.raw("</svg>");
        return c.finish();
    }

    Body::jersey(&mut c, &l, &tones, jersey);
    c.raw(r#"<rect width="200" height="250" fill="url(#vig)"/>"#);
    c.raw("</svg>");
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Appearance, Region, SkinBucket};

    /// The tone this paints and the tone the match page sends to the replay
    /// viewer are meant to be one draw off one stream. The only thing holding
    /// them together is that `Appearance::draw` is the FIRST call made on the
    /// rng — slip anything in front of it and the two quietly diverge.
    #[test]
    fn the_portrait_paints_the_tone_the_viewer_is_told_about() {
        let nations = [
            SkinDist::pure(SkinBucket::Black, Region::SubSaharan),
            SkinDist::pure(SkinBucket::White, Region::NorthEurope),
            SkinDist::pure(SkinBucket::Metis, Region::EastAsia),
            SkinDist::pure(SkinBucket::Metis, Region::Andes),
            // The mixed default a player of unknown nationality falls back to
            SkinDist::default(),
        ];
        for dist in nations {
            for player_id in [1u32, 7, 91, 4242, 900_001] {
                let told = Palette::SKIN[Appearance::of(player_id, dist).skin];
                let svg =
                    generate_face_svg(player_id, 26, dist, 0.0, 0.3, None, FaceFrame::Portrait);
                assert!(
                    svg.contains(&format!("stop-color=\"{told}\"")),
                    "player {player_id} is painted a tone the viewer was never given ({told})"
                );
            }
        }
    }

    /// A cutout is the same head with nothing round it.
    ///
    /// Both halves matter: the card, the shoulders and the vignette all have
    /// to go, and the head itself has to be untouched — same rng stream,
    /// same features, same tone as the portrait the profile page draws, or
    /// the man on the pitch is a different man.
    #[test]
    fn a_cutout_is_the_portrait_with_the_setting_taken_away() {
        let dist = SkinDist::pure(SkinBucket::White, Region::WestEurope);
        for player_id in [7u32, 4242, 900_001] {
            let portrait = generate_face_svg(
                player_id,
                26,
                dist,
                0.4,
                0.3,
                Some("#123456"),
                FaceFrame::Portrait,
            );
            let cutout = generate_face_svg(
                player_id,
                26,
                dist,
                0.4,
                0.3,
                Some("#123456"),
                FaceFrame::Cutout,
            );

            assert!(portrait.contains(r#"fill="url(#bgg)""#));
            assert!(
                !cutout.contains(r#"fill="url(#bgg)""#),
                "the cutout is still painted on a card"
            );
            assert!(
                !cutout.contains(r#"fill="url(#vig)""#),
                "the cutout still carries the portrait's vignette"
            );
            assert!(
                !cutout.contains(r#"fill="url(#jg)""#),
                "the cutout still has shoulders in it"
            );
            assert!(cutout.ends_with("</svg>"));

            // The head is the head. Everything the portrait draws between the
            // card and the shoulders IS the cutout, byte for byte.
            let bg_start = portrait
                .find(r#"<g id="bg">"#)
                .expect("the portrait has a card");
            let bg_end = portrait[bg_start..].find("</g>").expect("the card closes") + bg_start + 4;
            let shoulders = portrait
                .find(r#"<defs><clipPath id="jc">"#)
                .expect("the portrait puts shoulders on");
            let head = format!(
                "{}{}</svg>",
                &portrait[..bg_start],
                &portrait[bg_end..shoulders]
            );
            assert_eq!(
                cutout, head,
                "player {player_id} is a different man once the setting is taken away"
            );
        }
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
                assert!(
                    (44.0..=60.0).contains(&l.half_width_at(l.eye)),
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
                    // Sweep the build axis across each row: lean → heavy;
                    // scrambled aggression sweep so it decorrelates from heft
                    let heft = -1.6 + i as f32 * 0.5;
                    let aggression = (i * 3 % 8) as f32 / 7.0;
                    let svg = generate_face_svg(
                        player_id,
                        age,
                        dist,
                        heft,
                        aggression,
                        None,
                        FaceFrame::Portrait,
                    );
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
