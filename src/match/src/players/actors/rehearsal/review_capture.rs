//! Opt-in reports and moving rig captures from the actual replay pipeline.
use super::*;
use std::io::Write;

fn percentile(values: &mut [f32], q: f32) -> f32 {
    values.sort_by(f32::total_cmp);
    if values.is_empty() {
        0.0
    } else {
        values[((values.len() - 1) as f32 * q) as usize]
    }
}

#[test]
#[ignore = "needs MATCH_REPLAY and MATCH_REVIEW_DIR"]
fn measure_movement_review() {
    let mut rehearsal = Rehearsal::open().expect("MATCH_REPLAY");
    let (from, until) = window(&mut rehearsal, 60.0);
    let ids: Vec<_> = rehearsal.players().collect();
    let mut previous: HashMap<(u32, usize), (Quat, Vec3, bool)> = HashMap::new();
    let mut contacts = HashMap::new();
    let mut roots = HashMap::new();
    let mut slips: HashMap<&str, Vec<f32>> = HashMap::new();
    let mut spikes: HashMap<&str, u64> = HashMap::new();
    let mut worst = Vec::new();
    let dt = 1.0 / 60.0;
    while rehearsal.now() < until {
        rehearsal.tick();
        let now = rehearsal.now();
        for &id in &ids {
            let position = rehearsal.position(id);
            let Some(actor) = rehearsal.actor(id) else {
                continue;
            };
            let moved = roots
                .insert(id, position)
                .map_or(0.0, |was: Vec3| position.distance(was) / dt);
            let bucket = doing(actor);
            let ordinary = matches!(bucket, "running" | "standing")
                && moved < 12.0
                && actor.pose.land < 0.02
                && actor.pose.grounded < 0.02;
            for (j, (family, limb, side)) in JOINTS.iter().copied().enumerate() {
                let rotation = step(limb, side, origin(limb, side), actor.pose).rotation;
                let mut velocity = Vec3::ZERO;
                let had_previous = previous.contains_key(&(id, j));
                if let Some((was, rate, valid)) = previous.get(&(id, j)).copied() {
                    velocity = (was.inverse() * rotation).to_scaled_axis() / dt;
                    let acceleration = (velocity - rate).length() / dt;
                    let bar = if matches!(family, "hip" | "knee" | "ankle") {
                        1500.0
                    } else {
                        250.0
                    };
                    if ordinary && valid && acceleration > bar {
                        *spikes.entry(family).or_default() += 1;
                        worst.push((acceleration, now, id, family, channels(actor)));
                    }
                }
                previous.insert((id, j), (rotation, velocity, ordinary && had_previous));
            }
            let world = placed(actor, position);
            let soles = [-1.0, 1.0].map(|side| boot(side, actor.pose));
            // `boot` returns the sole used by the existing contact census.
            for (index, sole) in soles.into_iter().enumerate() {
                let sole = world.transform_point(sole);
                let striking = actor.pose.power + actor.pose.trap + actor.pose.header > 0.02
                    && (index as f32 * 2.0 - 1.0) * actor.pose.foot > 0.0;
                if sole.y < 0.02 && !striking && ordinary {
                    if let Some(was) = contacts.insert((id, index), sole) {
                        let offset: Vec3 = sole - was;
                        slips
                            .entry(bucket)
                            .or_default()
                            .push(Vec2::new(offset.x, offset.z).length() / dt);
                    }
                } else {
                    contacts.remove(&(id, index));
                }
            }
        }
    }
    let mut slip_report = serde_json::Map::new();
    for (bucket, mut samples) in slips {
        slip_report.insert(
            bucket.into(),
            serde_json::json!({"count":samples.len(), "p95":percentile(&mut samples, 0.95)}),
        );
    }
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    worst.truncate(100);
    let report = serde_json::json!({"from_ms":from, "until_ms":until, "locomotion_spikes":spikes,
        "spike_total":spikes.values().sum::<u64>(), "slip":slip_report, "worst":worst});
    let directory = std::env::var("MATCH_REVIEW_DIR").expect("MATCH_REVIEW_DIR");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        std::path::Path::new(&directory).join("metrics.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("spikes {}, slip {}", report["spike_total"], report["slip"]);
}

#[test]
#[ignore = "writes MATCH_REVIEW_DIR/movement-<speed>.mp4 using ffmpeg"]
fn capture_movement_review() {
    let directory = std::env::var("MATCH_REVIEW_DIR").expect("MATCH_REVIEW_DIR");
    std::fs::create_dir_all(&directory).unwrap();
    let ids: Vec<u32> = std::env::var("MATCH_REVIEW_PLAYERS")
        .unwrap_or_else(|_| "101,100,200".into())
        .split(',')
        .map(|id| id.parse().expect("player id"))
        .collect();
    assert_eq!(ids.len(), 3);
    for speed in [1.0, 0.5] {
        let mut rehearsal = Rehearsal::open().expect("MATCH_REPLAY");
        let (from, until) = window(&mut rehearsal, 5.0);
        rehearsal.at_speed(speed);
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::tailor(&mut meshes, Grain::of(Tier::Multisampled, Some("spare")));
        let file = std::path::Path::new(&directory).join(format!("movement-{speed}.mp4"));
        let mut encoder = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                "480x480",
                "-framerate",
                "30",
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(file)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("ffmpeg");
        let mut input = encoder.stdin.take().unwrap();
        while rehearsal.now() < until {
            rehearsal.tick();
            rehearsal.tick();
            let mut frame = vec![0u8; 480 * 480 * 4];
            for (column, id) in ids.iter().copied().enumerate() {
                let position = rehearsal.position(id);
                let ball_at = rehearsal
                    .ball()
                    .map(|ball| ball - position)
                    .filter(|p| p.length() < 4.0);
                let Some(actor) = rehearsal.actor(id) else {
                    continue;
                };
                let (pitch, roll) = actor.topple();
                let carriage = Transform::from_rotation(Quat::from_rotation_y(actor.heading))
                    * Carriage::placed(pitch, roll, actor.lift());
                for (row, top) in [2.6, 8.0].into_iter().enumerate() {
                    let lens = Lens {
                        bearing: 0.7,
                        bottom: -0.08,
                        top,
                    };
                    let mut canvas = Canvas::new(160, 240);
                    posed(
                        &mut canvas,
                        &lens,
                        &meshes,
                        &parts,
                        actor.pose,
                        carriage,
                        actor.is_goalkeeper,
                    );
                    if let Some(ball_at) = ball_at {
                        ball(&mut canvas, &lens, ball_at);
                    }
                    let pixels = canvas.pixels();
                    for line in 0..240 {
                        let to = ((row * 240 + line) * 480 + column * 160) * 4;
                        frame[to..to + 640].copy_from_slice(&pixels[line * 640..(line + 1) * 640]);
                    }
                }
            }
            input.write_all(&frame).unwrap();
        }
        drop(input);
        assert!(encoder.wait().unwrap().success());
        println!("captured {from}..{until} ms at {speed}x");
    }
}
