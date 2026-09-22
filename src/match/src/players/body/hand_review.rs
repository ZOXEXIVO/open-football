//! Close-up motion review using the same mesh hierarchy and poses as playback.
use super::preview::{Canvas, Lens, posed};
use super::*;

#[test]
#[ignore = "renders hand transitions; requires MATCH_FIGURE_DUMP"]
fn dump_hand_motion() {
    let directory =
        std::path::PathBuf::from(std::env::var("MATCH_FIGURE_DUMP").expect("MATCH_FIGURE_DUMP"));
    let mut meshes = Assets::<Mesh>::default();
    let parts = BodyParts::tailor(&mut meshes, Grain::FULL);
    const W: usize = 240;
    const H: usize = 240;
    // Three side-by-side panels: catch and grip, open palm parry, closed fist.
    for frame in 0..60 {
        let t = frame as f32 / 30.0;
        let approach = Actors::ease(t / 0.5);
        let impact = Actors::ease((t - 0.6) / 0.12) * (1.0 - Actors::ease((t - 0.72) / 0.28));
        let release = 1.0 - Actors::ease((t - 1.3) / 0.65);
        let mut sheet = vec![0u8; W * 3 * H * 4];
        for column in 0..3 {
            let mut gait = Gait {
                keeper: 1.0,
                set: 1.0,
                save: approach * release,
                save_aim: Vec2::new(0.25, 0.25),
                save_recoil: impact,
                idle: t,
                ..Gait::resting()
            };
            if column == 0 {
                gait.carry = Actors::ease((t - 0.72) / 0.30) * release;
            } else {
                gait.parry = 1.0;
                if column == 2 {
                    gait.punch = approach * release;
                }
            }
            let hand = Physique::glove(1.0, gait);
            let lens = Lens {
                bearing: 2.2,
                bottom: hand.y - 0.20,
                top: hand.y + 0.20,
            };
            let mut canvas = Canvas::new(W, H);
            posed(
                &mut canvas,
                &lens,
                &meshes,
                &parts,
                gait,
                Transform::from_translation(Vec3::new(-hand.x, 0.0, -hand.z)),
                true,
            );
            let pixels = canvas.pixels();
            for row in 0..H {
                let to = (row * W * 3 + column * W) * 4;
                sheet[to..to + W * 4].copy_from_slice(&pixels[row * W * 4..(row + 1) * W * 4]);
            }
        }
        std::fs::write(directory.join(format!("hands-{frame:03}.rgba")), sheet).unwrap();
    }
    println!("60 frames, 720x240; catch / parry / punch, 30 fps");
}
