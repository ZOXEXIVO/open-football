use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::actors::Actors;
use crate::kit::Outfit;

/// One cross-section of a body part: an ellipse of half-widths `x` (across the
/// body) and `z` (front to back) at height `y`, in the part's own space.
#[derive(Clone, Copy)]
struct Ring {
    y: f32,
    x: f32,
    z: f32,
}

impl Ring {
    const fn round(y: f32, radius: f32) -> Self {
        Ring {
            y,
            x: radius,
            z: radius,
        }
    }

    const fn oval(y: f32, x: f32, z: f32) -> Self {
        Ring { y, x, z }
    }
}

/// Lathes stacked ellipses into a closed, smooth-shaded mesh.
///
/// Every part of a footballer — a thigh, the torso, a skull — is a tube through
/// a handful of cross-sections, so one lathe covers the whole model and the
/// shapes stay editable as a list of numbers rather than as geometry. Nothing
/// here is loaded from disk: a glTF character would cost more to ship than the
/// entire viewer.
#[derive(Default)]
struct Sculptor {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Sculptor {
    /// Sides around each ring. Sixteen holds a round silhouette at the range
    /// the broadcast camera now sits at, and still leaves a full squad at a few
    /// thousand triangles — the meshes are shared by all twenty-two.
    const SIDES: usize = 16;
    /// Rings from pole to pole on a sphere.
    const STACKS: usize = 10;

    /// A part described by its profile, from either end.
    fn part(rings: &[Ring]) -> Mesh {
        let mut sculptor = Sculptor::default();
        sculptor.loft(rings, Vec3::ZERO);
        sculptor.build()
    }

    /// A rounded lump — a hand, a boot — sized on each axis.
    fn ellipsoid(radii: Vec3) -> Mesh {
        let mut sculptor = Sculptor::default();
        sculptor.loft(&Self::sphere(radii), Vec3::ZERO);
        sculptor.build()
    }

    /// Sphere profile, bottom pole to top pole.
    fn sphere(radii: Vec3) -> Vec<Ring> {
        (0..=Self::STACKS)
            .map(|stack| {
                let angle = PI * (1.0 - stack as f32 / Self::STACKS as f32);
                Ring {
                    y: radii.y * angle.cos(),
                    x: radii.x * angle.sin(),
                    z: radii.z * angle.sin(),
                }
            })
            .collect()
    }

    fn loft(&mut self, rings: &[Ring], offset: Vec3) {
        if rings.len() < 2 {
            return;
        }
        // Wound as though the profile always climbed, so a part described from
        // the top down — which every limb is — still ends up with its faces
        // pointing outward.
        let mut profile = rings.to_vec();
        if profile[0].y > profile[profile.len() - 1].y {
            profile.reverse();
        }

        let base = self.positions.len() as u32;
        let foot = profile[0].y;
        let span = (profile[profile.len() - 1].y - foot).max(f32::EPSILON);
        for ring in &profile {
            for side in 0..Self::SIDES {
                let angle = TAU * side as f32 / Self::SIDES as f32;
                self.positions.push([
                    offset.x + angle.cos() * ring.x,
                    offset.y + ring.y,
                    offset.z + angle.sin() * ring.z,
                ]);
                self.uvs
                    .push([side as f32 / Self::SIDES as f32, (ring.y - foot) / span]);
            }
        }

        let sides = Self::SIDES as u32;
        for step in 0..profile.len() as u32 - 1 {
            for side in 0..sides {
                let next = (side + 1) % sides;
                let a = base + step * sides + side;
                let b = base + step * sides + next;
                let c = base + (step + 1) * sides + next;
                let d = base + (step + 1) * sides + side;
                self.indices.extend_from_slice(&[a, d, b, b, d, c]);
            }
        }

        self.cap(profile[0], offset, false);
        self.cap(profile[profile.len() - 1], offset, true);
    }

    /// Closes one end of a loft. Skipped where the profile has already pinched
    /// to a point, as the crown of a head does.
    fn cap(&mut self, ring: Ring, offset: Vec3, upward: bool) {
        if ring.x.abs() < 1e-4 && ring.z.abs() < 1e-4 {
            return;
        }
        let hub = self.positions.len() as u32;
        self.positions.push([offset.x, offset.y + ring.y, offset.z]);
        self.uvs.push([0.5, 0.5]);
        for side in 0..Self::SIDES {
            let angle = TAU * side as f32 / Self::SIDES as f32;
            self.positions.push([
                offset.x + angle.cos() * ring.x,
                offset.y + ring.y,
                offset.z + angle.sin() * ring.z,
            ]);
            self.uvs
                .push([0.5 + angle.cos() * 0.5, 0.5 + angle.sin() * 0.5]);
        }

        let sides = Self::SIDES as u32;
        for side in 0..sides {
            let next = (side + 1) % sides;
            if upward {
                self.indices
                    .extend_from_slice(&[hub, hub + 1 + next, hub + 1 + side]);
            } else {
                self.indices
                    .extend_from_slice(&[hub, hub + 1 + side, hub + 1 + next]);
            }
        }
    }

    /// Smooth normals from the faces meeting at each vertex. Ring vertices are
    /// shared the whole way round, so shading runs continuously over a limb;
    /// the caps carry their own copies, which keeps their rims crisp.
    fn build(self) -> Mesh {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for triangle in self.indices.chunks_exact(3) {
            let corners = [
                Vec3::from(self.positions[triangle[0] as usize]),
                Vec3::from(self.positions[triangle[1] as usize]),
                Vec3::from(self.positions[triangle[2] as usize]),
            ];
            // Left unnormalised on purpose: the cross product's length is twice
            // the triangle's area, which is the weighting a smooth normal wants.
            let face = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
            for index in triangle {
                normals[*index as usize] += face;
            }
        }

        let normals: Vec<[f32; 3]> = normals
            .into_iter()
            .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y).to_array())
            .collect();

        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Where a footballer's joints sit, in metres above the turf. Both the meshes
/// and the rig that hangs them together read these, so the model can be made
/// taller or leggier by editing one block of numbers.
pub struct Physique;

impl Physique {
    pub const HIP: f32 = 0.95;
    pub const HIP_SPREAD: f32 = 0.088;
    pub const THIGH: f32 = 0.455;
    pub const SHIN: f32 = 0.455;
    /// Hip to the base of the neck.
    pub const TORSO: f32 = 0.58;
    /// Shoulder joints, in the torso's own space.
    ///
    /// Deliberately sunk BENEATH and INSIDE the torso's shoulder crest
    /// (which peaks 0.209 wide at y=0.512). An arm socket level with the
    /// crest, as this was, leaves the top cap of the upper arm standing
    /// proud of the shirt as a separate rounded lump — the single thing
    /// that made the figures read as assembled out of parts. Buried, the
    /// only bit of arm you see above the armpit is the deltoid, which is
    /// how a shoulder actually looks.
    pub const SHOULDER: f32 = 0.470;
    pub const SHOULDER_SPREAD: f32 = 0.176;
    pub const UPPER_ARM: f32 = 0.30;
    pub const FOREARM: f32 = 0.26;
    /// Crown to turf. Only the name plates want this, to size themselves
    /// against the player they belong to.
    pub const STATURE: f32 = 1.79;

    /// Where a goalkeeper holds the ball once he has gathered it, in his own
    /// space and before his build is applied.
    ///
    /// Not a free choice. It is worked forward down the arm from the shoulder
    /// socket through [`Joint::CRADLE_SHOULDER`] and [`Joint::CRADLE_ELBOW`],
    /// which put the wrists at (±0.184, 1.151, 0.325); the ball sits in the
    /// fork just above and between them. The viewer draws the ball here rather
    /// than at its recorded position while the keeper has it, so the two MUST
    /// come out of the same numbers — move one of the three and the gloves
    /// close on empty air.
    pub const CRADLE: Vec3 = Vec3::new(0.0, 1.20, 0.32);

    /// And where he holds it at full stretch, in the same space.
    ///
    /// Same discipline as [`Physique::CRADLE`], worked down the arm from
    /// [`Joint::REACH_SHOULDER`] / [`Joint::REACH_ELBOW`] with the claim
    /// closing the gloves to [`Joint::CLAIM_SPREAD`]: those put the wrists at
    /// (±0.072, 1.953, 0.054), and the ball sits a glove's length further
    /// along the arms, between the palms.
    ///
    /// Without it the ball stayed at the chest point through a diving catch —
    /// a keeper thrown full length with the ball hanging in mid-air where his
    /// sternum would have been if he had stayed on his feet.
    const CATCH: Vec3 = Vec3::new(0.0, 2.03, 0.06);
    /// How far the claim travels across his body as he turns onto the ball,
    /// in metres at a dive flat across the goal. The chest twist
    /// ([`Joint::DIVE_TWIST`]) carries both shoulders round with it, and the
    /// gloves go too.
    const CATCH_TURN: f32 = 0.13;

    /// Where a ball claimed at full stretch is drawn, for a dive with this
    /// much of a leading side. See [`Gait::lead`].
    pub fn catch(lead: f32) -> Vec3 {
        Self::CATCH + Vec3::X * (lead.clamp(-1.0, 1.0) * Self::CATCH_TURN)
    }
}

/// Every mesh a footballer is made of, built once and shared by all
/// twenty-two: only the materials differ from player to player.
pub struct BodyParts {
    torso: Handle<Mesh>,
    pelvis: Handle<Mesh>,
    head: Handle<Mesh>,
    hair: Handle<Mesh>,
    upper_arm: Handle<Mesh>,
    sleeve: Handle<Mesh>,
    forearm: Handle<Mesh>,
    hand: Handle<Mesh>,
    /// A keeper's glove. Its own mesh rather than a scaled hand: the whole
    /// point of the thing is that it is broad and flat, and the two of them
    /// splayed at the end of a dive are what says *catching* from the stand.
    glove: Handle<Mesh>,
    shorts_leg: Handle<Mesh>,
    thigh: Handle<Mesh>,
    shin: Handle<Mesh>,
    sock_top: Handle<Mesh>,
    boot: Handle<Mesh>,
    /// The flat panel the shirt number is printed on.
    number: Handle<Mesh>,
}

impl BodyParts {
    pub fn new(meshes: &mut Assets<Mesh>) -> Self {
        BodyParts {
            // Hips, waist, chest, shoulders — the shirt. A footballer's torso
            // is a V: the waist is the narrowest point and the shoulders carry
            // most of the width, which is most of what reads as "athlete" in a
            // silhouette this size.
            // The shoulder line is the whole problem with a figure this size.
            // It used to go 0.196 wide at y=0.530 and 0.150 at y=0.558 — a
            // 23% collapse across 28 mm — which is a coat-hanger, not a
            // shoulder: a flat shelf ending in a cliff, with the arm hung off
            // the edge of it. Real shoulders are a dome. The trapezius leaves
            // the neck and falls away over a good ten centimetres, and the
            // widest point is the acromion, out at the very end of the
            // collarbone, with a rounded crest over it.
            //
            // So the crest is now broad and slightly domed (0.482-0.535 all
            // within 4% of each other) and the run into the neck takes three
            // rings instead of one.
            torso: meshes.add(Sculptor::part(&[
                Ring::oval(0.000, 0.142, 0.098),
                Ring::oval(0.070, 0.134, 0.092),
                Ring::oval(0.140, 0.131, 0.089),
                Ring::oval(0.220, 0.148, 0.100),
                Ring::oval(0.300, 0.168, 0.107),
                Ring::oval(0.370, 0.185, 0.111),
                Ring::oval(0.440, 0.198, 0.111),
                Ring::oval(0.482, 0.207, 0.108),
                Ring::oval(0.512, 0.209, 0.104),
                Ring::oval(0.535, 0.201, 0.099),
                Ring::oval(0.552, 0.176, 0.093),
                Ring::oval(0.568, 0.134, 0.082),
                Ring::oval(0.580, 0.088, 0.070),
            ])),
            // The seat of the shorts, which stays put while the legs swing.
            pelvis: meshes.add(Sculptor::part(&[
                Ring::oval(0.100, 0.140, 0.098),
                Ring::oval(0.020, 0.152, 0.104),
                Ring::oval(-0.060, 0.160, 0.108),
                Ring::oval(-0.130, 0.154, 0.104),
            ])),
            // Neck, jaw and skull, hung off the base of the neck.
            head: meshes.add(Sculptor::part(&[
                Ring::oval(-0.060, 0.048, 0.052),
                Ring::oval(0.000, 0.053, 0.057),
                Ring::oval(0.022, 0.070, 0.074),
                Ring::oval(0.045, 0.086, 0.092),
                Ring::oval(0.075, 0.096, 0.103),
                Ring::oval(0.110, 0.100, 0.109),
                Ring::oval(0.145, 0.096, 0.104),
                Ring::oval(0.180, 0.086, 0.093),
                Ring::oval(0.212, 0.068, 0.074),
                Ring::oval(0.238, 0.040, 0.044),
                Ring::oval(0.253, 0.000, 0.000),
            ])),
            // A cap over the skull, set back a little so a hairline reads.
            hair: meshes.add({
                let mut sculptor = Sculptor::default();
                sculptor.loft(
                    &[
                        Ring::oval(0.085, 0.101, 0.110),
                        Ring::oval(0.125, 0.102, 0.111),
                        Ring::oval(0.170, 0.095, 0.103),
                        Ring::oval(0.210, 0.078, 0.084),
                        Ring::oval(0.240, 0.049, 0.053),
                        Ring::oval(0.262, 0.000, 0.000),
                    ],
                    Vec3::new(0.0, 0.0, -0.008),
                );
                sculptor.build()
            }),
            // Deltoid, bicep, taper to the elbow.
            //
            // The top ring is now narrow (0.030) and sits high, so it ends up
            // inside the torso and its cap is never seen; the deltoid swells
            // 20 mm BELOW the socket, which is where a shoulder muscle
            // actually sits. Previously the widest ring was 5 mm below the
            // top, so the arm's broadest point was level with its own
            // socket and the join showed as a seam.
            upper_arm: meshes.add(Sculptor::part(&[
                Ring::round(0.055, 0.030),
                Ring::round(0.020, 0.050),
                Ring::round(-0.020, 0.057),
                Ring::round(-0.075, 0.054),
                Ring::round(-0.140, 0.049),
                Ring::round(-0.215, 0.044),
                Ring::round(-0.280, 0.040),
                Ring::round(-0.300, 0.037),
            ])),
            // A short sleeve, riding on the shoulder joint so it swings with
            // it. Dropped to sit over the deltoid rather than above it —
            // level with the socket it stood off the shoulder like a pad.
            sleeve: meshes.add(Sculptor::part(&[
                Ring::oval(0.030, 0.064, 0.062),
                Ring::oval(-0.015, 0.074, 0.071),
                Ring::oval(-0.080, 0.070, 0.067),
                Ring::oval(-0.130, 0.062, 0.060),
            ])),
            forearm: meshes.add(Sculptor::part(&[
                Ring::round(0.000, 0.046),
                Ring::round(-0.060, 0.043),
                Ring::round(-0.140, 0.037),
                Ring::round(-0.215, 0.031),
                Ring::round(-0.260, 0.027),
            ])),
            hand: meshes.add(Sculptor::ellipsoid(Vec3::new(0.035, 0.050, 0.028))),
            // Cuff, back of the hand, then the padded palm out to the
            // fingertips. Half again as long as a bare hand and nearly twice
            // as wide, which is what a keeper's glove is: at this range the
            // pair of them are the only part of him the eye actually tracks
            // through a save.
            glove: meshes.add(Sculptor::part(&[
                Ring::oval(0.055, 0.036, 0.030),
                Ring::oval(0.020, 0.044, 0.034),
                Ring::oval(-0.010, 0.056, 0.036),
                Ring::oval(-0.060, 0.062, 0.034),
                Ring::oval(-0.105, 0.060, 0.030),
                Ring::oval(-0.135, 0.048, 0.024),
            ])),
            // The leg of the shorts: it belongs to the thigh, not to the hips.
            shorts_leg: meshes.add(Sculptor::part(&[
                Ring::oval(0.040, 0.114, 0.104),
                Ring::oval(-0.040, 0.120, 0.109),
                Ring::oval(-0.130, 0.116, 0.105),
                Ring::oval(-0.205, 0.106, 0.097),
            ])),
            // Quadriceps high on the thigh, narrowing into the knee.
            thigh: meshes.add(Sculptor::part(&[
                Ring::oval(-0.090, 0.089, 0.086),
                Ring::oval(-0.160, 0.086, 0.083),
                Ring::oval(-0.250, 0.078, 0.075),
                Ring::oval(-0.340, 0.068, 0.066),
                Ring::oval(-0.420, 0.059, 0.058),
                Ring::oval(-0.455, 0.053, 0.053),
            ])),
            // Shin and sock in one: socks cover a footballer's leg to the knee.
            // The calf sits high and at the back, which is what stops the lower
            // leg reading as a broom handle when a stride opens out.
            shin: meshes.add(Sculptor::part(&[
                Ring::oval(0.020, 0.060, 0.060),
                Ring::oval(-0.040, 0.062, 0.063),
                Ring::oval(-0.110, 0.059, 0.061),
                Ring::oval(-0.200, 0.050, 0.052),
                Ring::oval(-0.300, 0.042, 0.043),
                Ring::oval(-0.390, 0.036, 0.037),
                Ring::oval(-0.440, 0.034, 0.036),
                Ring::oval(-0.455, 0.033, 0.035),
            ])),
            // The turnover at the top of the sock, in the shorts colour — the
            // one piece of kit detail that survives at this distance.
            sock_top: meshes.add(Sculptor::part(&[
                Ring::oval(0.012, 0.0620, 0.0625),
                Ring::oval(-0.030, 0.0645, 0.0655),
                Ring::oval(-0.078, 0.0620, 0.0635),
            ])),
            boot: meshes.add(Sculptor::ellipsoid(Vec3::new(0.048, 0.038, 0.105))),
            // A real shirt number covers most of the upper back.
            number: meshes.add(Rectangle::new(0.250, 0.215).mesh().build()),
        }
    }
}

/// Which limb a joint drives. The rig is small enough that the run cycle can be
/// written out per joint rather than sampled from an animation clip.
#[derive(Clone, Copy)]
pub enum Limb {
    /// The seat of the shorts. It never turns — it is only here so that it
    /// rides the bob with the rest of the body instead of staying behind.
    Pelvis,
    Torso,
    Head,
    Shoulder,
    Elbow,
    /// The hand. Only ever visibly articulated on a goalkeeper — but a hand
    /// welded in line with its forearm is exactly what makes a save read as a
    /// mannequin being swung about, so the joint exists for everybody and the
    /// run cycle simply asks very little of it.
    Wrist,
    Hip,
    Knee,
}

/// One articulated joint of one player.
#[derive(Component)]
pub struct Joint {
    /// The actor this limb belongs to; the run cycle is kept there.
    pub owner: Entity,
    limb: Limb,
    /// −1 on the left of the body, +1 on the right, 0 down the middle.
    side: f32,
    /// Where the joint sits at rest, so the bob can be added back onto it.
    origin: Vec3,
}

/// How a player is moving right now, in the only two terms the pose needs:
/// where in the stride they are, and how hard they are running.
#[derive(Clone, Copy)]
pub struct Gait {
    pub phase: f32,
    /// 0 standing, 1 flat out.
    pub run: f32,
    /// −1..1, fixed for the life of a player: how he carries himself.
    ///
    /// Twenty-two figures running one identical animation is most of what
    /// reads as "robots" — more than any amount of geometry. Real players
    /// are individually recognisable at distance almost entirely by
    /// carriage: how wide the arms are held, how bent the elbows, how far
    /// forward the shoulders go. One number per player, applied to those
    /// three, is enough to break the lockstep.
    pub signature: f32,
    /// A slow cycle in radians that runs on the clock rather than on ground
    /// covered, so it keeps going when a player is not.
    ///
    /// Standing still was literally standing still: every amplitude in this
    /// rig is scaled by `run`, so at rest a footballer froze into a statue.
    /// The path trace says that is most of the pitch most of the time —
    /// forwards are stationary 68% of a match and keepers 77% — so a frozen
    /// idle is not an edge case, it is the default state of the scene.
    pub idle: f32,
    /// How hard he is turning, −1..1, for the lean into it.
    pub turn: f32,
    /// Where he is looking, as a yaw off his own facing in radians. A player
    /// watches the ball; his head does not sit welded to his chest.
    pub look: f32,
    /// And how far up or down, in radians: positive when the ball is above
    /// his eyeline. Without it a keeper tracks a cross along the floor and
    /// then catches it above his head without ever having looked at it.
    pub look_pitch: f32,
    /// 0 for everybody all match; ramps to 1 for the one goalkeeper who has
    /// the ball in his gloves.
    ///
    /// A keeper who had gathered the ball was posed exactly like one who had
    /// not: arms hanging at his sides, swinging with the run cycle, while the
    /// ball hung at chest height inside his own torso. This is the signal that
    /// brings the forearms up and settles the chest so it can sit in his
    /// hands instead.
    pub carry: f32,
    /// 0..1: he is off his feet and committed. Only ever non-zero for a
    /// goalkeeper — see [`crate::actors::Actors::animate`] for how a dive is
    /// told from a run.
    ///
    /// The topple itself is not here: a body going horizontal is a rotation
    /// of the WHOLE figure and belongs on [`Carriage`]. What this drives is
    /// everything the limbs do differently once he has left his feet — the
    /// run cycle cut dead rather than faded out.
    ///
    /// On the instant, and held long after he lands: a keeper does not stand
    /// straight back up.
    pub dive: f32,
    /// 0..1: how far through the extension he is, ramped across the flight.
    ///
    /// The single most important number in a save, and the one this rig used
    /// not to have. `dive` says he has left the ground, and it is true within
    /// a frame of take-off; a man in the air is then in exactly one pose for
    /// the next four hundred milliseconds, which is most of what made a save
    /// look like a photograph being slid across the grass. Measured off a
    /// recorded match, a keeper is airborne for 390–660 ms with a median of
    /// 450 — long enough that every part of the extension has to be drawn:
    /// he leaves the ground gathered, opens out, and is at full stretch by
    /// the time he arrives.
    ///
    /// Ratchets: it climbs over the flight and is given back only by the
    /// recovery, because nobody folds himself up again mid-air.
    pub stretch: f32,
    /// 0..1: how long he has been down since the landing.
    ///
    /// The pose he lands in is not the pose he lies in. This is what takes
    /// him from full stretch to a man curled on his side with the ball
    /// pulled in — and it is scaled by `dive`, so it goes away as he gets up
    /// rather than pinning him to the turf.
    pub grounded: f32,
    /// −1..1: which way he went, as a share of his travel across his own
    /// body. ±1 is a dive flat across the goal, 0 one straight down the
    /// pitch at a striker's feet.
    ///
    /// Nothing in this rig used to know, so both arms and both legs did the
    /// same thing as each other — the superman, and the one pose no
    /// goalkeeper has ever adopted. A save has a lead side: the top arm goes
    /// through the ball, the bottom one trails, the underneath leg stays
    /// straight and the top one folds.
    pub lead: f32,
    /// 0..1: both arms thrown out to their full length, for the dive and for
    /// the standing leap at a cross. Held apart from `dive` because a keeper
    /// jumping at a corner reaches without toppling at all.
    pub reach: f32,
    /// 0..1: on his toes with a shot on — knees bent, weight forward, gloves
    /// up and out.
    ///
    /// A keeper spends the overwhelming majority of a match on his feet
    /// (measured: `Standing` alone is 8770 s of one recording against 48 s
    /// of `Diving`), and until now he spent all of it standing exactly like
    /// a centre-forward waiting for a throw-in. The set position is the
    /// posture the save comes out of, so a dive with nothing in front of it
    /// arrives from nowhere.
    pub set: f32,
    /// 0..1: he has the ball, at full stretch, in the air.
    ///
    /// The difference between a save and a catch, and they do not look alike:
    /// a keeper covering his goal has his gloves half a metre apart because
    /// he is trying to be in two places, and one who has actually got the
    /// thing brings both hands onto it. Without this, a ball claimed at full
    /// stretch hangs between two gloves that never close.
    pub claimed: f32,
    /// Where he is in a kick: −1 at the top of the backswing, 0 at the
    /// instant the boot meets the ball, +1 at the end of the follow through.
    /// 0 for everybody not kicking, which is why it must be read together
    /// with `power`.
    ///
    /// The ball is struck 14.8 times a minute in a recorded match and none of
    /// it used to be drawn: the ball left, the player kept running, and the
    /// single most repeated action in football was a thing that happened to
    /// the ball rather than a thing anybody did.
    pub swing: f32,
    /// How hard, 0..1 — a five-yard pass and a shot from twenty are the same
    /// movement at very different amplitudes. Doubles as the gate on the
    /// whole kick: at zero none of it applies.
    pub power: f32,
    /// Which boot: −1 left, +1 right, 0 nobody.
    pub foot: f32,
    /// 0..1 over a keeper throwing the ball out, which is the same swing
    /// routed to his shoulder instead of his hip. Peaks at the release.
    pub throwing: f32,
    /// −1..1: driving off the mark at +1, pulling up short at −1.
    ///
    /// A footballer changes pace far more often than he changes direction,
    /// and the rig had a lean for the turn but nothing at all for this — so
    /// a man going from a standstill to a sprint did it bolt upright, and one
    /// stopping dead just stopped.
    pub drive: f32,
    /// 0..1: the ball is at his feet.
    ///
    /// Measured, somebody is within a stride of a slow ball in 72% of frames,
    /// so this is the standing condition of one player on the pitch rather
    /// than a rare event — and the man with the ball ran identically to the
    /// twenty-one without it.
    pub carrying: f32,
    /// 0..1: an outfielder off the ground — a header, not a save.
    ///
    /// Its own signal because the sprawl is not one: twelve outfield players
    /// in a recorded match leave the turf, up to 1.13 m, and every one of
    /// them was being drawn toppling sideways with both arms over his head
    /// like a keeper going full length.
    pub jump: f32,
}

impl Joint {
    /// The angle the leading leg swings through, standing and at a sprint.
    const HIP_SWING: (f32, f32) = (0.10, 0.62);
    const KNEE_FLEX: (f32, f32) = (0.16, 1.55);
    const ARM_SWING: (f32, f32) = (0.10, 0.75);
    const ELBOW_FLEX: (f32, f32) = (0.25, 1.05);
    const LEAN: (f32, f32) = (0.045, 0.20);
    /// How far a running player's whole body rises as the stride closes up,
    /// in metres.
    const BOB: f32 = 0.06;
    /// And how far a standing one rises and falls just breathing. Small on
    /// purpose — this is the difference between a statue and a man waiting,
    /// not a visible bounce.
    const BREATHE: f32 = 0.013;
    /// How far a standing player rolls as his weight goes from one foot to
    /// the other, in radians.
    const WEIGHT_SHIFT: f32 = 0.038;
    /// Roll of the torso through the stride — a runner rocks, he does not
    /// stay square.
    const ROCK: f32 = 0.055;
    /// How far the hips counter-rotate against the shoulders.
    const HIP_TWIST: f32 = 0.065;
    /// How far a player leans into a turn at full tilt.
    const BANK: f32 = 0.30;
    /// The hold, as rotations about X in the shoulder's and the elbow's own
    /// frames. The upper arm eases forward off the ribs and the forearm comes
    /// most of the way up, so the two make a shelf in front of the chest —
    /// which is how a goalkeeper carries a ball he has just claimed, elbows in
    /// and gloves under it, not out in front of him at arm's length.
    ///
    /// [`Physique::CRADLE`] is where these two angles land the wrists. Keep
    /// them together.
    const CRADLE_SHOULDER: f32 = -0.12;
    const CRADLE_ELBOW: f32 = -1.55;
    /// A little outward roll at the shoulder, so the gloves close on the sides
    /// of the ball rather than inside it.
    const CRADLE_SPREAD: f32 = 0.03;
    /// And the wrists under it: the gloves come up around the ball rather
    /// than hanging off the ends of the forearms.
    const CRADLE_WRIST: f32 = -1.15;
    /// Where the arms are on the way up, before the extension opens them
    /// out. A keeper leaves the ground with his elbows still bent and his
    /// hands in front of his chest — the reach happens in the air, which is
    /// the whole reason [`Gait::stretch`] exists.
    const LAUNCH_SHOULDER: f32 = -1.05;
    const LAUNCH_ELBOW: f32 = -1.00;
    /// The reach, as rotations in the shoulder's and elbow's own frames. The
    /// leading arm goes past the head and its elbow comes nearly straight — a
    /// keeper at full stretch is measured from his fingertips, and every
    /// degree left in the elbow is a centimetre he does not cover.
    ///
    /// Read together with the topple on [`Carriage`]: rolled onto his side,
    /// arms along his own up-axis point ACROSS the goal, which is what makes
    /// the same two angles serve both the dive and the standing leap.
    const REACH_SHOULDER: f32 = -2.48;
    const REACH_ELBOW: f32 = -0.07;
    /// Hands apart rather than together — he is covering an area, not
    /// catching a pass.
    ///
    /// Applied with the side NEGATED, unlike every other spread in this rig,
    /// and it has to be: a roll about +Z carries a hanging arm outward and a
    /// RAISED one inward, because the arm has swung through the vertical and
    /// taken its sense of "out" with it. Signed like the others, this pulled
    /// the two gloves in to 11 cm apart — inside the shoulders, which is the
    /// one thing a keeper covering his goal is not doing.
    const REACH_SPREAD: f32 = 0.30;
    /// What the TRAILING arm does instead, as offsets on the three above.
    ///
    /// Both arms at full stretch is the pose from the front of a cereal box.
    /// The real one is asymmetric: the top arm is thrown through the ball,
    /// and the bottom one stays out at shoulder height with the elbow soft —
    /// partly for balance, mostly because it is the one that hits the ground
    /// first and he knows it.
    const TRAIL_SHOULDER: f32 = 0.80;
    const TRAIL_ELBOW: f32 = -0.55;
    const TRAIL_SPREAD: f32 = 0.24;
    /// And what the same two arms do once they have the ball: brought in off
    /// the spread until the gloves are a ball's width apart, and both of
    /// them, because the trailing arm comes onto it too.
    const CLAIM_SPREAD: f32 = -0.22;
    /// Gloves at full stretch: broken back off the forearm and turned out, so
    /// the palms face what is coming rather than each other.
    const REACH_WRIST: f32 = -0.55;
    const REACH_WRIST_SPREAD: f32 = 0.40;
    /// The legs in a dive: trailing behind the hips and all but straight. A
    /// footballer in the air has nothing to push against, so the run cycle
    /// has to stop dead rather than fade out over a stride.
    const DIVE_HIP: f32 = 0.22;
    const DIVE_KNEE: f32 = 0.12;
    /// And how the two of them differ, scaled by how square the dive is. The
    /// leg on the side he is going to is the one he pushed off: it finishes
    /// straight and trailing. The far one folds up over it.
    const DIVE_SCISSOR_HIP: f32 = 0.34;
    const DIVE_SCISSOR_KNEE: f32 = 1.35;
    /// The chest in flight: arched away from the ball and turned onto it.
    ///
    /// The torso used to be cancelled outright by a dive — mathematically
    /// upright, which reads as a shop dummy laid on its side. A keeper in the
    /// air is the most extended a footballer ever gets: the spine is in
    /// extension and the leading shoulder is rotated hard through the line of
    /// the ball.
    const DIVE_ARCH: f32 = -0.30;
    const DIVE_TWIST: f32 = 0.38;
    /// And on the grass: the curl over the impact, the legs coming up, the
    /// arms pulling in. Landing is not a freeze-frame of the flight.
    const DOWN_CURL: f32 = 0.34;
    const DOWN_HIP: f32 = -0.58;
    const DOWN_KNEE: f32 = 1.10;
    const DOWN_SHOULDER: f32 = -0.85;
    const DOWN_SPREAD: f32 = 0.04;
    const DOWN_ELBOW: f32 = -1.35;
    /// The set: knees bent, chest over the toes, gloves up and out in front.
    const SET_HIP: f32 = -0.30;
    const SET_KNEE: f32 = 0.55;
    const SET_LEAN: f32 = 0.26;
    /// How far the whole figure settles as those knees bend, in metres.
    ///
    /// Not a style choice — it is what the leg angles above cost in height,
    /// and without it a crouching keeper's boots hang three and a half
    /// centimetres over the grass.
    const SET_DROP: f32 = 0.035;
    const SET_SHOULDER: f32 = -0.62;
    const SET_SPREAD: f32 = 0.40;
    const SET_ELBOW: f32 = -1.10;
    const SET_WRIST: f32 = -0.40;
    /// An outfielder in the air: knees folded under him, arms out from his
    /// sides for balance. A header, and nothing like a save.
    const JUMP_HIP: f32 = -0.22;
    const JUMP_KNEE: f32 = 1.15;
    const JUMP_SHOULDER: f32 = -0.62;
    const JUMP_SPREAD: f32 = 0.52;
    const JUMP_ELBOW: f32 = -0.50;
    /// How much of a hand's rest angle is this particular player's, so
    /// twenty-two pairs of hands are not all cocked identically.
    const WRIST_REST: f32 = 0.12;
    /// The kick, as the three angles the kicking leg passes through: the top
    /// of the backswing, the instant of contact, and the end of the follow
    /// through.
    ///
    /// Hip first — positive carries the thigh BACK, so the backswing is
    /// positive and everything after it is negative. The leg finishes higher
    /// than it was at contact because a struck ball is hit through, not at:
    /// the boot keeps going and takes the hip with it.
    const KICK_HIP: (f32, f32, f32) = (0.85, -0.72, -1.30);
    /// And the knee, which folds hard on the way back, snaps straight through
    /// the ball, and stays nearly straight into the follow through. The whip
    /// of that fold is most of where a kick's power reads from.
    const KICK_KNEE: (f32, f32, f32) = (1.70, 0.06, 0.34);
    /// What the standing leg does: bends to take the whole body's weight as
    /// the other one comes through, and pushes back up out of it.
    const PLANT_KNEE: f32 = 0.58;
    /// And how far the whole figure sinks over it, in metres — the same
    /// bookkeeping as [`Joint::SET_DROP`]. A bent standing leg is shorter than
    /// a straight one, and without paying for it he kicks the ball while
    /// hovering six centimetres above the pitch.
    const KICK_DROP: f32 = 0.058;
    /// How much of each end of the swing is given over to blending into and
    /// out of the run cycle, as a fraction of it.
    const KICK_BLEND: f32 = 0.20;
    /// How far the hips and then the chest rotate through the ball. The hips
    /// lead and the shoulders follow, which is the sequence that makes a kick
    /// look like it came from the ground up rather than from the knee.
    const KICK_TWIST: f32 = 0.44;
    /// How far ahead of the swing the hips run, and how much more of the same
    /// turn the shoulders take. Together they are the separation between the
    /// two: coiled the other way at the top of the backswing, hips through
    /// first at contact.
    const KICK_HIP_LEAD: f32 = 0.35;
    const KICK_CHEST: f32 = 0.90;
    /// And how far he leans away from the kicking side, which is what keeps a
    /// man upright while one leg is over his own head.
    const KICK_ROLL: f32 = 0.26;
    /// The counter-swing, as the same three keys again: the arm OPPOSITE the
    /// kicking leg comes forward and up through the ball, and the one on the
    /// kicking side goes back. Without it the whole action reads as a
    /// mechanism hinged at one joint.
    const KICK_COUNTER: (f32, f32, f32) = (-0.35, -1.25, -0.95);
    const KICK_TRAIL: (f32, f32, f32) = (0.20, 0.60, 0.40);
    /// And how far across his own chest the counter-arm comes.
    const KICK_ARM_SPREAD: f32 = 0.55;
    /// A keeper's throw: the same three keys as a kick, sent to his shoulder
    /// instead of his hip — cocked behind the ear, over the top, and down
    /// across the follow through.
    ///
    /// Written as one INCREASING sweep past a half turn rather than as the
    /// angles it would be natural to write, because these are interpolated as
    /// scalars: cocking at +2.35 and releasing at −1.98 is the same pair of
    /// poses but takes the arm down past his hip to get between them, which is
    /// an underarm bowl.
    const THROW_SHOULDER: (f32, f32, f32) = (2.35, 4.30, 5.15);
    const THROW_ELBOW: (f32, f32, f32) = (-1.60, -0.25, -0.15);
    /// Driving off the mark and pulling up short: how far the chest goes over
    /// the toes at full acceleration, and how far it sits back at a full
    /// stop. Braking is the bigger angle — stopping is more violent than
    /// starting, and it is the one every footballer does at the end of every
    /// run.
    const DRIVE_LEAN: (f32, f32) = (0.34, -0.30);
    /// And what his legs do about it: feet driving out behind under
    /// acceleration, planted out in front under the brakes.
    const DRIVE_HIP: f32 = 0.16;
    /// The man on the ball: down over it, arms out from his sides for
    /// balance, strides shortened. All small — he is a footballer running,
    /// not a man in a crouch — but between them they are what makes the
    /// player with the ball findable in a crowd.
    const CARRY_LEAN: f32 = 0.16;
    const CARRY_KNEE: f32 = 0.16;
    const CARRY_DROP: f32 = 0.022;
    const CARRY_SPREAD: f32 = 0.16;
    const CARRY_ELBOW: f32 = -0.30;

    fn new(owner: Entity, limb: Limb, side: f32, origin: Vec3) -> Self {
        Joint {
            owner,
            limb,
            side,
            origin,
        }
    }

    /// Where the joint sits this frame.
    ///
    /// A runner rides highest with their legs underneath them and drops as the
    /// stride opens out, twice per cycle. The bob only ever lifts — taking the
    /// body below its standing height would put the boots through the turf —
    /// and pelvis, hips and torso all take the same offset, so the whole
    /// footballer rises as one.
    pub fn place(&self, gait: Gait) -> Vec3 {
        match self.limb {
            Limb::Pelvis | Limb::Torso | Limb::Hip => {
                let bob = Self::BOB * gait.run * (0.5 + 0.5 * (gait.phase * 2.0).cos());
                // Breathing, for a player who is not running. Fades out as he
                // does, where the stride bob takes over.
                let breathe = Self::BREATHE * (1.0 - gait.run) * (0.5 + 0.5 * gait.idle.sin());
                // And the settle onto bent knees, which is a real loss of
                // height rather than a pose: see [`Joint::SET_DROP`]. The man
                // on the ball sinks over it the same way, by less.
                self.origin
                    + Vec3::Y
                        * (bob + breathe
                            - Self::SET_DROP * gait.set
                            - Self::CARRY_DROP * gait.carrying
                            - Self::KICK_DROP * gait.power * Self::taper(gait.swing))
            }
            _ => self.origin,
        }
    }

    /// The joint's rotation at this point in the stride.
    ///
    /// Positive rotation about X carries the top of a part forward and its
    /// far end back, so a leg swings forward on a negative angle and a knee
    /// folds on a positive one. Every amplitude is interpolated out of
    /// standing still, which is what lets a player come to a stop without an
    /// idle animation to blend into.
    pub fn pose(&self, gait: Gait) -> Quat {
        // The left leg is half a cycle behind the right.
        let leg = gait.phase + if self.side < 0.0 { PI } else { 0.0 };
        let swing = leg.sin();
        let lean = Self::blend(Self::LEAN, gait.run);

        // Weight going from one foot to the other, and back. Half the idle
        // rate, because a shift is a whole cycle where a breath is half of
        // one. Fades out the moment he starts running.
        let standing = 1.0 - gait.run;
        let weight = (gait.idle * 0.5).sin() * standing;

        // Which of a pair this limb is, relative to the way he went: +1 on
        // the leading side of the dive, −1 on the trailing side, 0 for a man
        // going straight forward — and 0 for everybody who is not diving,
        // since `lead` is only ever set as a keeper leaves the ground.
        let leading = (self.side * gait.lead).clamp(-1.0, 1.0);
        // How much of the asymmetry this limb takes. Scaled by how sideways
        // the dive was, so a smother at a striker's feet stays square.
        // Squared off again once he has the ball: both arms come onto it.
        let trailing = (1.0 - leading) * 0.5 * gait.lead.abs() * (1.0 - gait.claimed);
        // The arms on the grass. Held apart from `grounded` because what they
        // do down there depends entirely on whether he came up with the ball:
        // with it, he curls around it and the cradle already has them; without
        // it, they come in under him and take his weight.
        let bracing = gait.grounded * (1.0 - gait.carry);
        // How much of the kick this limb takes, 0..1. Peaks at contact and
        // eases away at both ends, so the swing arrives out of the run cycle
        // and returns to it rather than being cut in and out.
        // Full authority across the middle of the swing and a quick blend to
        // the run cycle at each end, so the kick arrives and leaves without a
        // pop. NOT a taper across the whole swing: the keys below put the
        // backswing at −1 and the follow through at +1, so fading the
        // authority out toward them cancels the two halves of a kick that are
        // not the moment of contact — which is to say, all of it.
        let taper = Self::taper(gait.swing);
        let kicking = gait.power * taper;
        let throwing = gait.throwing * taper;
        // +1 if this is the kicking side of the body, −1 if it is the standing
        // side. Zero for everybody not kicking, which leaves both halves equal
        // and every term below at rest.
        let striking = self.side * gait.foot;

        match self.limb {
            // Hips counter-rotate against the shoulders — the thing that
            // makes a run read as a person rather than a mechanism — and
            // carry the weight shift when he is standing.
            Limb::Pelvis => {
                // The hips lead a kick and the shoulders follow it: the pelvis
                // is already opening toward the target while the boot is still
                // travelling, which is the sequence that makes a strike look
                // as though it came from the ground up rather than from the
                // knee. The phase offset is what puts it ahead — at contact
                // the hips have turned through and the chest has not.
                //
                // Signed off the kicking foot, so a left-footer turns the
                // other way.
                let opening = Self::KICK_TWIST
                    * gait.foot
                    * (gait.swing + Self::KICK_HIP_LEAD).clamp(-1.0, 1.0)
                    * kicking;
                Quat::from_rotation_y(Self::HIP_TWIST * gait.run * gait.phase.sin() - opening)
                    * Quat::from_rotation_z(Self::WEIGHT_SHIFT * weight)
            }
            Limb::Torso => {
                // Rock through the stride, a quarter cycle off the twist so
                // the shoulder drops as the opposite leg drives; the weight
                // shift when standing; and the bank into a turn.
                //
                // Bank sign: heading increases turning toward +X, and a
                // positive Z rotation carries the head toward −X, so leaning
                // INTO a left turn is the negative of the turn signal. If it
                // ever looks like a motorbike falling the wrong way, this is
                // the sign to flip.
                let roll = Self::ROCK * gait.run * (gait.phase + FRAC_PI_2).sin()
                    + Self::WEIGHT_SHIFT * 0.8 * weight
                    - Self::BANK * gait.turn;
                // A keeper with the ball stands up out of all of it. Two
                // reasons, and the second is the load-bearing one: he does
                // straighten up, and the arm angles above are measured off an
                // upright chest — leave the lean and the rock in and the ball
                // drifts a couple of centimetres out of his gloves every
                // stride.
                //
                // A diving keeper comes out of it for the same reason and one
                // more: the lean is the forward pitch of a man driving off the
                // ground, and there is no ground under him.
                let settle = (1.0 - gait.carry) * (1.0 - gait.dive) * (1.0 - gait.set);
                let running = Quat::from_rotation_x(lean * (1.0 + 0.16 * gait.signature) * settle)
                    * Quat::from_rotation_y(-0.14 * gait.run * gait.phase.sin() * settle)
                    * Quat::from_rotation_z(roll * settle);
                // Then, in order: the set's forward lean over bent knees, the
                // arch and the turn onto the ball in flight, and the curl
                // over the landing. Each one is scaled by its own signal, so
                // for twenty-one players out of twenty-two every term after
                // the first is identity.
                running
                    * Quat::from_rotation_x(Self::SET_LEAN * gait.set)
                    * Quat::from_rotation_x(
                        Self::DIVE_ARCH * gait.stretch * (1.0 - gait.grounded),
                    )
                    * Quat::from_rotation_y(
                        Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                    )
                    * Quat::from_rotation_x(Self::DOWN_CURL * gait.grounded)
                    // An outfielder's leap is a much smaller version of the
                    // same arch, and none of the turn: he is going up at a
                    // ball, not across a goal.
                    * Quat::from_rotation_x(Self::DIVE_ARCH * 0.45 * gait.jump)
                    // Over his toes off the mark, back on his heels under the
                    // brakes — and lower over the ball when he has it.
                    * Quat::from_rotation_x(
                        Self::DRIVE_LEAN.0 * gait.drive.max(0.0)
                            + Self::DRIVE_LEAN.1 * (-gait.drive).max(0.0)
                            + Self::CARRY_LEAN * gait.carrying,
                    )
                    // The chest coils further than the hips going back and
                    // arrives after them coming through — the separation
                    // between the two is where a kick's whip comes from — and
                    // he leans away from the leg that is over his own head.
                    * Quat::from_rotation_y(
                        -Self::KICK_TWIST
                            * Self::KICK_CHEST
                            * gait.foot
                            * (gait.swing - Self::KICK_HIP_LEAD).clamp(-1.0, 1.0)
                            * kicking,
                    )
                    * Quat::from_rotation_z(Self::KICK_ROLL * gait.foot * kicking)
            }
            // He watches the ball. The head hangs off the torso, so this yaw
            // is already relative to his chest — turning it is the single
            // cheapest thing in the whole rig that reads as attention, and
            // without it twenty-two players stare rigidly down their own
            // running line all match.
            //
            // Still kept level in pitch against his own forward lean — a
            // runner leans from the hips and looks up the pitch, not at his
            // own boots — and then tipped onto the ball, which for a keeper
            // is most of what a save is: he does not take his eyes off it,
            // and up to now he never once looked up at one.
            //
            // The twist is subtracted back out because the head hangs off the
            // torso: without that, rotating the chest onto the ball in a dive
            // swings the face straight past it.
            Limb::Head => {
                // A runner's head stays level while his chest rocks under it —
                // the eyes are on the ball and the neck does the work. So the
                // torso's own roll is subtracted back out, the same way the
                // dive's twist is: without it the whole head tips side to side
                // once per stride, which is the one thing nobody's does.
                let steady = -Self::ROCK * gait.run * (gait.phase + FRAC_PI_2).sin();
                Quat::from_rotation_y(
                    gait.look
                        - Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                ) * Quat::from_rotation_x(-lean * 0.75 - gait.look_pitch)
                    * Quat::from_rotation_z(steady)
            }
            Limb::Shoulder => {
                // Arms swing against the leg on the same side, and are carried
                // wider the harder the player is running — and wider again, or
                // tighter, depending on the man.
                // — and wider again with the ball at his feet, which is a
                // man balancing over it rather than running freely.
                let carriage = 0.15
                    + 0.07 * gait.run
                    + 0.055 * gait.signature
                    + Self::CARRY_SPREAD * gait.carrying;
                // Nobody's two arms do the same thing. Tied to the signature
                // so it is this player's asymmetry, the same one every time.
                let asymmetry = 1.0 + 0.11 * gait.signature * self.side;
                // Standing, they drift instead of locking. Offset by side so
                // the two do not move as a pair.
                let drift = 0.055 * standing * (gait.idle + self.side).sin();
                // The counter-swing: the arm OPPOSITE the kicking leg comes
                // forward and up through the ball while the one on the kicking
                // side goes back. Every rotation a footballer puts into a ball
                // has to be paid for somewhere, and this is where — without it
                // the whole action reads as a mechanism hinged at one joint.
                let counter = Self::through(
                    if striking < 0.0 {
                        Self::KICK_COUNTER
                    } else {
                        Self::KICK_TRAIL
                    },
                    gait.swing,
                ) * kicking
                    * striking.abs();
                let arm =
                    Self::blend(Self::ARM_SWING, gait.run) * swing * asymmetry + drift + counter;
                // And the counter-arm alone comes across his chest, which is
                // the other half of paying for the turn.
                let across = self.side * Self::KICK_ARM_SPREAD * kicking * (-striking).max(0.0);
                let swinging = Quat::from_rotation_z(self.side * carriage - across)
                    * Quat::from_rotation_x(arm);
                let ready = Self::held(
                    swinging,
                    Quat::from_rotation_z(self.side * Self::SET_SPREAD)
                        * Quat::from_rotation_x(Self::SET_SHOULDER),
                    gait.set,
                );
                let leaping = Self::held(
                    ready,
                    Quat::from_rotation_z(self.side * Self::JUMP_SPREAD)
                        * Quat::from_rotation_x(Self::JUMP_SHOULDER),
                    gait.jump,
                );
                let holding = Self::held(
                    leaping,
                    Quat::from_rotation_z(self.side * Self::CRADLE_SPREAD)
                        * Quat::from_rotation_x(Self::CRADLE_SHOULDER),
                    gait.carry,
                );
                // The reach itself, opening out across the flight rather than
                // arriving whole: gathered at take-off, past the head by the
                // apex — and this arm only gets all the way there if it is
                // the leading one.
                let shoulder = Self::LAUNCH_SHOULDER
                    + (Self::REACH_SHOULDER - Self::LAUNCH_SHOULDER) * gait.stretch
                    + Self::TRAIL_SHOULDER * trailing * gait.stretch;
                let spread = (Self::REACH_SPREAD + Self::TRAIL_SPREAD * trailing)
                    * (1.0 - gait.claimed)
                    + Self::CLAIM_SPREAD * gait.claimed;
                // Reach after the cradle, so a keeper who takes the ball
                // cleanly at the top of a leap has his arms come down into
                // the hold rather than stay up around a ball he already has.
                let out = Self::held(
                    holding,
                    Quat::from_rotation_z(-self.side * spread) * Quat::from_rotation_x(shoulder),
                    gait.reach,
                );
                // The landing: whatever he was doing up there, the arms come
                // in as he hits the grass.
                let down = Self::held(
                    out,
                    Quat::from_rotation_z(self.side * Self::DOWN_SPREAD)
                        * Quat::from_rotation_x(Self::DOWN_SHOULDER),
                    bracing,
                );
                // And a keeper's throw, which is the same swing as a kick sent
                // to the arm instead: cocked behind his ear and hurled
                // overarm. Only the throwing side moves.
                Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::THROW_SHOULDER, gait.swing)),
                    throwing * striking.max(0.0),
                )
            }
            // Elbow carriage is the most individual thing about a runner:
            // some hold them almost straight, some at a right angle.
            Limb::Elbow => {
                let running = Quat::from_rotation_x(
                    -Self::blend(Self::ELBOW_FLEX, gait.run) * (1.0 + 0.24 * gait.signature)
                        - 0.18 * gait.run * swing
                        // Elbows come up over a ball he is carrying, and the
                        // counter-arm bends hard through a kick.
                        + Self::CARRY_ELBOW * gait.carrying
                        - 0.55 * kicking * (-striking).max(0.0),
                );
                let ready = Self::held(running, Quat::from_rotation_x(Self::SET_ELBOW), gait.set);
                let leaping = Self::held(ready, Quat::from_rotation_x(Self::JUMP_ELBOW), gait.jump);
                let holding = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::CRADLE_ELBOW),
                    gait.carry,
                );
                let elbow = Self::LAUNCH_ELBOW
                    + (Self::REACH_ELBOW - Self::LAUNCH_ELBOW) * gait.stretch
                    + Self::TRAIL_ELBOW * trailing * gait.stretch;
                let out = Self::held(holding, Quat::from_rotation_x(elbow), gait.reach);
                let down = Self::held(out, Quat::from_rotation_x(Self::DOWN_ELBOW), bracing);
                Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::THROW_ELBOW, gait.swing)),
                    throwing * striking.max(0.0),
                )
            }
            // The hands. Loose on the run, up and open in the set, broken
            // back and turned out at full stretch, cupped under a ball he has
            // claimed. A glove that stays in line with its forearm reads as
            // the end of a stick, however good the arm above it is.
            Limb::Wrist => {
                let loose = Quat::from_rotation_x(
                    Self::WRIST_REST * (1.0 + 0.5 * gait.signature * self.side),
                );
                let ready = Self::held(
                    loose,
                    Quat::from_rotation_z(self.side * 0.28)
                        * Quat::from_rotation_x(Self::SET_WRIST),
                    gait.set,
                );
                let holding = Self::held(
                    ready,
                    Quat::from_rotation_z(-self.side * 0.20)
                        * Quat::from_rotation_x(Self::CRADLE_WRIST),
                    gait.carry,
                );
                // Splayed to cover an area, or turned in around a ball he has
                // actually got hold of.
                let out = Self::held(
                    holding,
                    Quat::from_rotation_z(
                        self.side * Self::REACH_WRIST_SPREAD * (1.0 - 2.0 * gait.claimed),
                    ) * Quat::from_rotation_x(Self::REACH_WRIST),
                    gait.reach,
                );
                Self::held(out, Quat::from_rotation_x(Self::CRADLE_WRIST), bracing)
            }
            Limb::Hip => {
                // The run, plus what his legs do about a change of pace: feet
                // driving out behind him off the mark, planted out in front
                // under the brakes.
                let running = Quat::from_rotation_x(
                    -Self::blend(Self::HIP_SWING, gait.run) * swing + Self::DRIVE_HIP * gait.drive,
                );
                let ready = Self::held(running, Quat::from_rotation_x(Self::SET_HIP), gait.set);
                let leaping = Self::held(ready, Quat::from_rotation_x(Self::JUMP_HIP), gait.jump);
                // In flight the legs trail — the near one straight behind
                // him because it is the one he pushed off, the far one
                // swinging up over it.
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_HIP + Self::DIVE_SCISSOR_HIP * leading),
                    gait.dive * gait.stretch,
                );
                let down = Self::held(diving, Quat::from_rotation_x(Self::DOWN_HIP), gait.grounded);
                // And the kick, on the striking leg only — the other one is
                // planted and keeps its stride.
                Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::KICK_HIP, gait.swing)),
                    kicking * striking.max(0.0),
                )
            }
            // Deepest as the leg folds through underneath the player, and all
            // but straight again by the time it reaches out to land. Squaring
            // the curve is what narrows the tuck to that one part of the
            // cycle; a plain cosine leaves the leading leg bent on touchdown,
            // which reads as a stumble rather than a stride.
            Limb::Knee => {
                let running = Quat::from_rotation_x(
                    0.07
                        + Self::blend(Self::KNEE_FLEX, gait.run)
                            * (0.5 + 0.5 * (leg - 0.5).cos()).powi(2)
                        // Sunk over the ball, the way a man carrying one runs.
                        + Self::CARRY_KNEE * gait.carrying,
                );
                let ready = Self::held(running, Quat::from_rotation_x(Self::SET_KNEE), gait.set);
                let leaping = Self::held(ready, Quat::from_rotation_x(Self::JUMP_KNEE), gait.jump);
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_KNEE + Self::DIVE_SCISSOR_KNEE * trailing),
                    gait.dive * gait.stretch,
                );
                let down = Self::held(
                    diving,
                    Quat::from_rotation_x(Self::DOWN_KNEE),
                    gait.grounded,
                );
                // The kicking knee whips through the ball; the standing one
                // bends to take the weight while it does.
                let swung = Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::KICK_KNEE, gait.swing)),
                    kicking * striking.max(0.0),
                );
                Self::held(
                    swung,
                    Quat::from_rotation_x(Self::PLANT_KNEE),
                    kicking * (-striking).max(0.0),
                )
            }
        }
    }

    /// How much authority a swing has over the pose at this point in it.
    ///
    /// Full across the middle and blended to the run cycle at each end, so
    /// the kick arrives and leaves without a pop. NOT a taper across the whole
    /// swing: the keys put the backswing at −1 and the follow through at +1,
    /// so fading the authority out toward them cancels the two halves of a
    /// kick that are not the moment of contact — which is to say, all of it.
    fn taper(swing: f32) -> f32 {
        Actors::ease((1.0 - swing.abs()) / Self::KICK_BLEND)
    }

    fn blend(range: (f32, f32), run: f32) -> f32 {
        range.0 + range.1 * run
    }

    /// The angle a kicking joint holds at this point in the swing, from its
    /// three keys: top of the backswing, contact, end of the follow through.
    ///
    /// Piecewise rather than one curve because contact is not a smooth point —
    /// it is the instant a boot stops a moving ball, and the leg goes into it
    /// and out of it at quite different rates.
    fn through(keys: (f32, f32, f32), swing: f32) -> f32 {
        if swing < 0.0 {
            // Winding up: eased, so the leg gathers rather than snapping back.
            let back = Actors::ease(-swing);
            keys.1 + (keys.0 - keys.1) * back
        } else {
            keys.1 + (keys.2 - keys.1) * Actors::ease(swing)
        }
    }

    /// Fades a limb off the run cycle and onto the hold as a keeper gathers
    /// the ball. Short-circuited at zero because twenty-one players out of
    /// twenty-two are never holding anything, and a slerp per joint per frame
    /// for all of them is a cost with nothing to show for it.
    fn held(swinging: Quat, cradle: Quat, carry: f32) -> Quat {
        if carry <= 1e-3 {
            swinging
        } else {
            swinging.slerp(cradle, carry.min(1.0))
        }
    }
}

/// The whole figure, hung off the actor so it can leave the ground without
/// taking the actor's own marks with it.
///
/// Everything else in this rig moves a limb against a body that is standing
/// on the turf. A dive is the one thing that moves the body itself: the man
/// goes horizontal and airborne, and no arrangement of hips and knees
/// expresses that. Putting one node between the actor and the figure is what
/// lets [`crate::actors::Actors::animate`] topple and lift the lot in one
/// transform — while the contact shadow and the team ring, which stay
/// children of the actor, stay flat on the grass where they belong.
#[derive(Component)]
pub struct Carriage {
    /// The actor this figure belongs to; the dive is kept there.
    pub owner: Entity,
}

impl Carriage {
    /// Height of the point the body turns about, in metres — the hips.
    ///
    /// A diving keeper pivots around his middle: head one way, boots the
    /// other, weight staying over the spot the recording puts him on. Turning
    /// him about his feet instead would swing his whole body a metre sideways
    /// out of his own shadow.
    pub const PIVOT: f32 = Physique::HIP;

    /// And where those hips end up once he is all the way over, in metres.
    ///
    /// This is the number the dive was missing, and it is the difference
    /// between a save and a mannequin being spun on a pole. Rotating a body
    /// about a point fixed at [`Carriage::PIVOT`] means a keeper thrown flat
    /// is horizontal *at standing hip height*: shoulders at 1.2 m, boots
    /// half a metre in the air, and no part of him anywhere near the grass —
    /// for the whole 390–660 ms a recorded dive lasts, and then he lands
    /// still floating. A man lying on his side has his hips a hand's width
    /// off the turf, so the pivot has to travel down as the body goes over.
    ///
    /// Taken together with the recorded height this also gets the arc right
    /// for free. A big dive peaks at 0.5 m of recorded lift with the body
    /// most of the way over, which puts the hips at 0.95 − 0.67·sin(tilt)
    /// + 0.5 ≈ 0.85 m — full stretch, a metre up, exactly the shape of the
    /// photograph — and the same expression walks him down to 0.31 m as the
    /// lift returns to zero, which is a keeper on the ground.
    const LYING: f32 = 0.32;

    /// The transform that tips a figure `pitch` radians over its toes and
    /// `roll` radians onto its side, `lift` metres off the turf, pivoting at
    /// the hips.
    ///
    /// Two axes because a keeper's dive mostly is not the poster one: across
    /// a recorded match, dives divide about evenly between those that travel
    /// further across the goal and those that travel further up the pitch —
    /// a man going down at a striker's feet rather than flying into a top
    /// corner. Roll alone drew every one of those side-on.
    ///
    /// A Bevy `Transform` is translate-rotate-scale, so the pivot cannot be
    /// expressed directly — it is folded into the translation instead, which
    /// is what the second term is: rotate about the origin, then put the hips
    /// back where they were.
    pub fn placed(pitch: f32, roll: f32, lift: f32) -> Transform {
        let rotation = Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll);
        let pivot = Vec3::Y * Self::PIVOT;
        // How far from upright the figure has ended up, as the sine of the
        // angle between its own up-axis and the world's. Off the composed
        // rotation rather than off either Euler angle, so a dive that is
        // half across the goal and half up the pitch settles as far as one
        // that is all of either.
        let upright = (rotation * Vec3::Y).y.clamp(-1.0, 1.0);
        let tilt = (1.0 - upright * upright).max(0.0).sqrt();
        let settle = (Self::PIVOT - Self::LYING) * tilt;
        Transform::from_translation(pivot - rotation * pivot + Vec3::Y * (lift - settle))
            .with_rotation(rotation)
    }
}

/// Hangs one footballer's meshes off an actor entity.
pub struct Footballer;

impl Footballer {
    /// Builds the rig under `root`, which carries the player's position, facing
    /// and stride. Legs hang from the [`Carriage`] so the torso can lean without
    /// taking the feet with it; arms and head hang from the torso so that they
    /// do.
    pub fn assemble(
        commands: &mut Commands,
        root: Entity,
        parts: &BodyParts,
        outfit: &Outfit,
        keeper: bool,
    ) {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        let elbow = Vec3::new(0.0, -Physique::UPPER_ARM, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
        let wrist = Vec3::new(0.0, -Physique::FOREARM - 0.03, 0.0);

        let carriage = commands
            .spawn((
                Carriage { owner: root },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        commands.entity(root).add_child(carriage);

        commands.entity(carriage).with_children(|body| {
            body.spawn((
                Joint::new(root, Limb::Pelvis, 0.0, hips),
                Mesh3d(parts.pelvis.clone()),
                MeshMaterial3d(outfit.shorts.clone()),
                Transform::from_translation(hips),
            ));

            body.spawn((
                Joint::new(root, Limb::Torso, 0.0, hips),
                Mesh3d(parts.torso.clone()),
                MeshMaterial3d(outfit.shirt.clone()),
                Transform::from_translation(hips),
            ))
            .with_children(|torso| {
                // Printed across the shoulder blades, standing a couple of
                // millimetres off the shirt so it never fights it for depth.
                if let Some(number) = outfit.number.clone() {
                    torso.spawn((
                        Mesh3d(parts.number.clone()),
                        MeshMaterial3d(number),
                        Transform::from_xyz(0.0, 0.330, -0.115)
                            .with_rotation(Quat::from_rotation_y(PI)),
                    ));
                }

                torso
                    .spawn((
                        Joint::new(root, Limb::Head, 0.0, neck),
                        Mesh3d(parts.head.clone()),
                        MeshMaterial3d(outfit.skin.clone()),
                        Transform::from_translation(neck),
                    ))
                    .with_child((
                        Mesh3d(parts.hair.clone()),
                        MeshMaterial3d(outfit.hair.clone()),
                        Transform::default(),
                    ));

                for side in [-1.0f32, 1.0] {
                    let shoulder =
                        Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
                    torso
                        .spawn((
                            Joint::new(root, Limb::Shoulder, side, shoulder),
                            Mesh3d(parts.upper_arm.clone()),
                            MeshMaterial3d(outfit.skin.clone()),
                            Transform::from_translation(shoulder),
                        ))
                        .with_children(|arm| {
                            arm.spawn((
                                Mesh3d(parts.sleeve.clone()),
                                MeshMaterial3d(outfit.shirt.clone()),
                                Transform::default(),
                            ));
                            arm.spawn((
                                Joint::new(root, Limb::Elbow, side, elbow),
                                Mesh3d(parts.forearm.clone()),
                                MeshMaterial3d(outfit.skin.clone()),
                                Transform::from_translation(elbow),
                            ))
                            .with_child((
                                Joint::new(root, Limb::Wrist, side, wrist),
                                Mesh3d(if keeper {
                                    parts.glove.clone()
                                } else {
                                    parts.hand.clone()
                                }),
                                MeshMaterial3d(outfit.hands.clone()),
                                Transform::from_translation(wrist),
                            ));
                        });
                }
            });

            for side in [-1.0f32, 1.0] {
                let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
                body.spawn((
                    Joint::new(root, Limb::Hip, side, hip),
                    Mesh3d(parts.thigh.clone()),
                    MeshMaterial3d(outfit.skin.clone()),
                    Transform::from_translation(hip),
                ))
                .with_children(|leg| {
                    leg.spawn((
                        Mesh3d(parts.shorts_leg.clone()),
                        MeshMaterial3d(outfit.shorts.clone()),
                        Transform::default(),
                    ));
                    leg.spawn((
                        Joint::new(root, Limb::Knee, side, knee),
                        Mesh3d(parts.shin.clone()),
                        MeshMaterial3d(outfit.socks.clone()),
                        Transform::from_translation(knee),
                    ))
                    .with_children(|shin| {
                        shin.spawn((
                            Mesh3d(parts.sock_top.clone()),
                            MeshMaterial3d(outfit.shorts.clone()),
                            Transform::default(),
                        ));
                        shin.spawn((
                            Mesh3d(parts.boot.clone()),
                            MeshMaterial3d(outfit.boots.clone()),
                            Transform::from_xyz(0.0, -Physique::SHIN + 0.005, 0.035),
                        ));
                    });
                });
            }
        });
    }
}

/// Forward kinematics over the rig, so the poses above can be checked as
/// positions rather than as angles.
///
/// Every constant in a save is an angle at a joint, and an angle is impossible
/// to argue about — but "his boots are eleven centimetres under the turf" is
/// not. These walk the same offsets [`Footballer::assemble`] hangs the meshes
/// off and call the same [`Joint::pose`] the renderer calls, so a sign error
/// anywhere in the chain shows up here as a body part in the wrong place.
#[cfg(test)]
mod skeleton {
    use super::*;

    /// A player standing still, with nothing switched on.
    pub fn still() -> Gait {
        Gait {
            phase: 0.0,
            run: 0.0,
            signature: 0.0,
            idle: 0.0,
            turn: 0.0,
            look: 0.0,
            look_pitch: 0.0,
            carry: 0.0,
            dive: 0.0,
            stretch: 0.0,
            grounded: 0.0,
            lead: 0.0,
            claimed: 0.0,
            reach: 0.0,
            set: 0.0,
            jump: 0.0,
            swing: 0.0,
            power: 0.0,
            foot: 0.0,
            throwing: 0.0,
            drive: 0.0,
            carrying: 0.0,
        }
    }

    /// A right-footed kick at full power, at this point in the swing.
    pub fn kicking(swing: f32) -> Gait {
        let mut gait = still();
        gait.swing = swing;
        gait.power = 1.0;
        gait.foot = 1.0;
        gait
    }

    fn step(limb: Limb, side: f32, origin: Vec3, gait: Gait) -> Transform {
        let joint = Joint::new(Entity::from_raw_u32(0).unwrap(), limb, side, origin);
        Transform::from_translation(joint.place(gait)).with_rotation(joint.pose(gait))
    }

    /// The glove centre, in the figure's own space — that is, under the
    /// carriage, which is where [`Physique::CRADLE`] and [`Physique::CATCH`]
    /// are expressed too.
    pub fn glove(side: f32, gait: Gait) -> Vec3 {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let shoulder = Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
        let elbow = Vec3::new(0.0, -Physique::UPPER_ARM, 0.0);
        let wrist = Vec3::new(0.0, -Physique::FOREARM - 0.03, 0.0);
        (step(Limb::Torso, 0.0, hips, gait)
            * step(Limb::Shoulder, side, shoulder, gait)
            * step(Limb::Elbow, side, elbow, gait)
            * step(Limb::Wrist, side, wrist, gait))
        .translation
    }

    /// The sole of a boot, in the same space.
    pub fn boot(side: f32, gait: Gait) -> Vec3 {
        let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
        let sole = Vec3::new(0.0, -Physique::SHIN + 0.005 - 0.038, 0.035);
        (step(Limb::Hip, side, hip, gait) * step(Limb::Knee, side, knee, gait))
            .transform_point(sole)
    }

    /// And the crown of the head.
    pub fn crown(gait: Gait) -> Vec3 {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        (step(Limb::Torso, 0.0, hips, gait) * step(Limb::Head, 0.0, neck, gait))
            .transform_point(Vec3::new(0.0, 0.253, 0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::skeleton::*;
    use super::*;
    use crate::actors::Actors;

    /// A keeper who has gathered the ball holds it where the viewer draws it.
    ///
    /// [`Physique::CRADLE`] is not a free choice — it is worked down the arm
    /// from the two cradle angles, and the ball is drawn there rather than at
    /// its recorded position. If the arms and the constant ever part company,
    /// the gloves close on empty air.
    #[test]
    fn the_cradle_is_where_the_gloves_are() {
        let mut gait = still();
        gait.carry = 1.0;
        let between = (glove(-1.0, gait) + glove(1.0, gait)) * 0.5;
        assert!(
            between.distance(Physique::CRADLE) < 0.16,
            "ball drawn at {:?}, gloves meet at {between:?}",
            Physique::CRADLE
        );
    }

    /// And the same for a ball claimed at full stretch, which is drawn at
    /// [`Physique::catch`] instead — out at the end of the arms rather than
    /// against a chest that is nowhere near it.
    #[test]
    fn the_catch_is_where_the_gloves_are() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        gait.claimed = 1.0;
        for lead in [-1.0f32, 0.0, 1.0] {
            gait.lead = lead;
            let between = (glove(-1.0, gait) + glove(1.0, gait)) * 0.5;
            let drawn = Physique::catch(lead);
            assert!(
                between.distance(drawn) < 0.13,
                "lead {lead}: ball drawn at {drawn:?}, gloves meet at {between:?}"
            );
            // Both hands are on it, not half a metre either side of it.
            assert!(
                glove(-1.0, gait).distance(glove(1.0, gait)) < 0.22,
                "lead {lead}: gloves never close"
            );
        }
        // And the two hold points are a long way apart, which is the entire
        // reason for having both.
        assert!(Physique::catch(0.0).distance(Physique::CRADLE) > 0.5);
    }

    /// Full stretch finishes above the crown of his head with the gloves
    /// properly apart, which is the whole point of leaving the ground: he is
    /// covering an area, not catching a pass.
    #[test]
    fn the_reach_goes_over_his_head() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        let hands = glove(1.0, gait);
        let crown = crown(gait);
        assert!(
            hands.y > crown.y,
            "gloves at {hands:?} below crown {crown:?}"
        );
        assert!(
            glove(-1.0, gait).distance(glove(1.0, gait)) > 0.5,
            "gloves converging: {:?} and {hands:?}",
            glove(-1.0, gait)
        );
        // Outside the shoulders, not tucked in between them.
        assert!(hands.x.abs() > Physique::SHOULDER_SPREAD);
    }

    /// The two arms in a lateral dive do different things. Both at full
    /// stretch is the pose off the front of a cereal box.
    #[test]
    fn a_lateral_dive_has_a_leading_arm() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        gait.lead = 1.0;
        let (trail, lead) = (glove(-1.0, gait), glove(1.0, gait));
        // Further along his own up-axis — which, once the carriage has rolled
        // him onto his side, is further across the goal: the top arm going
        // through the ball while the bottom one stays out for balance.
        assert!(
            lead.y - trail.y > 0.12,
            "arms level: lead {lead:?} trail {trail:?}"
        );
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        assert!(
            hips.distance(lead) - hips.distance(trail) > 0.10,
            "arms equally extended: {} vs {}",
            hips.distance(lead),
            hips.distance(trail)
        );
        // A dive straight down the pitch keeps them square, because it is a
        // smother at a striker's feet and has no leading side.
        gait.lead = 0.0;
        assert!((glove(-1.0, gait).y - glove(1.0, gait).y).abs() < 1e-3);
    }

    /// The legs scissor with it: the leg he pushed off finishes straight and
    /// trailing, the far one folds up over it.
    #[test]
    fn a_lateral_dive_has_a_trailing_leg() {
        let mut gait = still();
        gait.dive = 1.0;
        gait.stretch = 1.0;
        gait.lead = 1.0;
        // Measured as how far each boot ends up from its own hip, because the
        // fold is a leg getting SHORTER: comparing heights only catches it
        // once the body is already over, and the body goes over on the
        // carriage rather than here.
        let hip = |side: f32| Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
        let folded = hip(-1.0).distance(boot(-1.0, gait));
        let straight = hip(1.0).distance(boot(1.0, gait));
        assert!(
            straight - folded > 0.15,
            "legs together: folded {folded} straight {straight}"
        );
        // The straight one is the one he pushed off, so it is as long as a
        // standing leg.
        assert!((straight - hip(1.0).distance(boot(1.0, still()))).abs() < 0.02);
    }

    /// A keeper who has gone over comes down with it.
    ///
    /// Rotating a body about a pivot fixed at standing hip height leaves him
    /// horizontal in mid-air with his boots half a metre off the grass, and
    /// then lands him there — which is most of what stopped a save reading as
    /// a save.
    #[test]
    fn going_over_brings_him_down() {
        assert!(Carriage::placed(0.0, 0.0, 0.0).translation.length() < 1e-5);

        // Flat across the goal at the moment of landing: no recorded lift
        // left to hold him up.
        let flat = Carriage::placed(0.0, -Actors::SPRAWL_ANGLE, 0.0);
        let hips = flat.transform_point(Vec3::new(0.0, Physique::HIP, 0.0));
        assert!(
            hips.y < 0.40,
            "hips still at {} m with the body flat",
            hips.y
        );
        assert!(hips.y > 0.15, "hips through the turf at {} m", hips.y);

        // And nothing on him ends up under the grass.
        let mut gait = still();
        gait.dive = 1.0;
        gait.stretch = 1.0;
        gait.grounded = 1.0;
        gait.lead = 1.0;
        for part in [
            boot(-1.0, gait),
            boot(1.0, gait),
            crown(gait),
            glove(-1.0, gait),
            glove(1.0, gait),
        ] {
            let world = flat.transform_point(part);
            assert!(world.y > 0.0, "{part:?} lands at {} m", world.y);
        }
    }

    /// Two axes of topple settle him as far as either one alone: a dive half
    /// across the goal and half up the pitch is just as horizontal.
    #[test]
    fn both_axes_of_topple_settle_him() {
        let hips = |pitch: f32, roll: f32| {
            Carriage::placed(pitch, roll, 0.0)
                .transform_point(Vec3::new(0.0, Physique::HIP, 0.0))
                .y
        };
        let sideways = hips(0.0, -1.4);
        let forwards = hips(1.4, 0.0);
        // Two axes that compose to the same tilt: cos(0.99)² ≈ cos(1.44).
        let diagonal = hips(0.99, -0.99);
        assert!((sideways - forwards).abs() < 1e-4);
        assert!(
            (diagonal - sideways).abs() < 0.05,
            "diagonal {diagonal} vs square {sideways}"
        );
        // And standing up costs him nothing.
        assert!((hips(0.0, 0.0) - Physique::HIP).abs() < 1e-5);
    }

    /// The set is a real crouch, so it costs real height — and the drop that
    /// pays for it has to leave his boots on the grass.
    #[test]
    fn the_set_keeps_his_boots_down() {
        let standing = boot(1.0, still());
        let mut gait = still();
        gait.set = 1.0;
        let crouched = boot(1.0, gait);
        assert!(
            (crouched.y - standing.y).abs() < 0.015,
            "boots move {} m into the set",
            crouched.y - standing.y
        );
        // And he really is lower: the crown comes down with the knees.
        assert!(crown(gait).y < crown(still()).y - 0.03);
    }

    /// The extension is a ramp and not a switch: a keeper halfway through a
    /// flight is halfway out of it. This is the whole difference between a
    /// dive and a photograph of one.
    #[test]
    fn the_extension_opens_out() {
        let mut gait = still();
        gait.dive = 1.0;
        gait.reach = 1.0;
        let mut reach_at = |stretch: f32| {
            gait.stretch = stretch;
            glove(1.0, gait).y
        };
        let (gathered, half, full) = (reach_at(0.0), reach_at(0.5), reach_at(1.0));
        assert!(
            full - gathered > 0.30,
            "no extension at all: {gathered} to {full}"
        );
        assert!(
            half > gathered + 0.08 && half < full - 0.08,
            "not a ramp: {gathered} / {half} / {full}"
        );
    }

    /// An outfielder leaving the ground is heading a ball, not saving one:
    /// he tucks his knees and keeps his arms out for balance, and he does not
    /// go over.
    #[test]
    fn a_header_is_not_a_dive() {
        let mut gait = still();
        gait.jump = 1.0;
        assert!(
            boot(1.0, gait).y > boot(1.0, still()).y + 0.15,
            "knees not tucked: {:?}",
            boot(1.0, gait)
        );
        let hands = glove(1.0, gait);
        assert!(hands.y < crown(gait).y, "outfielder reaching like a keeper");
        assert!(hands.x.abs() > glove(1.0, still()).x.abs(), "arms not out");
    }

    /// A jumping outfielder does not go over, whatever he does in the air —
    /// the sprawl is a keeper's alone, and the tip that drives it is never
    /// written for anybody else.
    #[test]
    fn only_a_keeper_sprawls() {
        let mut gait = still();
        gait.jump = 1.0;
        assert_eq!(gait.dive, 0.0);
        assert_eq!(gait.stretch, 0.0);
        assert_eq!(gait.reach, 0.0);
    }

    /// A kick is a leg going somewhere. The whole point of the swing is that
    /// the boot travels: back behind him, through the ball, and up the other
    /// side.
    #[test]
    fn the_boot_travels_through_the_ball() {
        // Sampled inside the swing rather than at its ends, where the pose
        // is deliberately handed back to the run cycle.
        let back = boot(1.0, kicking(-0.8));
        let contact = boot(1.0, kicking(0.0));
        let through = boot(1.0, kicking(0.8));

        // Behind him at the top of the backswing, in front of him at contact.
        assert!(back.z < -0.35, "no backswing: {back:?}");
        assert!(contact.z > 0.45, "no contact: {contact:?}");
        assert!(
            contact.z - back.z > 0.9,
            "the boot barely moves: {back:?} to {contact:?}"
        );
        // And still rising afterwards — a struck ball is hit through, not at.
        assert!(
            through.y > contact.y + 0.1,
            "no follow through: {contact:?} to {through:?}"
        );
        // It is a boot, not a foot through the turf.
        for stage in [-1.0f32, -0.8, -0.5, 0.0, 0.5, 0.8, 1.0] {
            let sole = boot(1.0, kicking(stage));
            assert!(sole.y > -0.01, "boot under the grass at {stage}: {sole:?}");
        }
    }

    /// Only one leg swings. The other one is planted, taking the whole body's
    /// weight while it does.
    #[test]
    fn the_other_leg_stays_planted() {
        let gait = kicking(0.0);
        let swinging = boot(1.0, gait);
        let planted = boot(-1.0, gait);
        assert!(
            swinging.z - planted.z > 0.5,
            "both legs swinging: {swinging:?} and {planted:?}"
        );
        // Bent under him rather than straight: he is standing on it.
        assert!(
            planted.y - boot(-1.0, still()).y < 0.015,
            "standing foot floating: {planted:?}"
        );
        // He sinks over it rather than standing tall on a bent leg.
        assert!(
            crown(gait).y < crown(still()).y - 0.03,
            "no sink into the plant"
        );
        // And a left-footed kick is the mirror of a right-footed one.
        let mut left = kicking(0.0);
        left.foot = -1.0;
        let mirrored = boot(-1.0, left);
        assert!((mirrored.z - swinging.z).abs() < 1e-3);
        assert!((mirrored.x + swinging.x).abs() < 1e-3);
    }

    /// Every rotation a footballer puts into a ball is paid for somewhere: the
    /// arm opposite the kicking leg comes across and up.
    #[test]
    fn the_arms_pay_for_the_kick() {
        let gait = kicking(0.0);
        let counter = glove(-1.0, gait);
        let same = glove(1.0, gait);
        assert!(
            counter.y - same.y > 0.15,
            "arms doing nothing: {counter:?} and {same:?}"
        );
        assert!(
            counter.y > glove(-1.0, still()).y + 0.15,
            "counter-arm never lifts: {counter:?}"
        );
    }

    /// Nothing happens to anybody who is not kicking, whatever the phase says.
    /// `swing` is zero for the whole squad every frame, so a term that ignored
    /// `power` would have twenty-two players permanently mid-kick.
    #[test]
    fn power_gates_the_whole_kick() {
        let mut idle = still();
        idle.swing = -0.5;
        idle.foot = 1.0;
        assert!((boot(1.0, idle) - boot(1.0, still())).length() < 1e-4);
        assert!((glove(-1.0, idle) - glove(-1.0, still())).length() < 1e-4);
        assert!((crown(idle) - crown(still())).length() < 1e-4);
    }

    /// A tap and a shot are the same movement at very different sizes.
    #[test]
    fn power_scales_the_swing() {
        let mut gentle = kicking(0.0);
        gentle.power = 0.15;
        let travel = |gait: Gait| boot(1.0, gait).distance(boot(1.0, still()));
        assert!(
            travel(kicking(0.0)) > travel(gentle) * 2.0,
            "a tap swings like a shot: {} vs {}",
            travel(gentle),
            travel(kicking(0.0))
        );
    }

    /// The hips lead a kick and the shoulders follow, which is what makes it
    /// look like it came from the ground up rather than from the knee.
    #[test]
    fn the_hips_lead_the_shoulders() {
        // Measured as how far each has turned by the moment of contact,
        // against where they are for a man standing still.
        let turned = |limb: Limb, gait: Gait| {
            let joint = Joint::new(Entity::from_raw_u32(0).unwrap(), limb, 0.0, Vec3::ZERO);
            let point = joint.pose(gait) * Vec3::Z;
            point.x.atan2(point.z)
        };
        // Going back, the chest coils further than the hips do.
        let (hips, chest) = (
            turned(Limb::Pelvis, kicking(-0.7)),
            turned(Limb::Torso, kicking(-0.7)),
        );
        assert!(
            hips * chest > 0.0,
            "coiling opposite ways: {hips} vs {chest}"
        );
        assert!(
            chest.abs() > hips.abs(),
            "hips coil further: {hips} vs {chest}"
        );
        // Coming through, the hips arrive first: at contact they have already
        // turned toward the target and the chest is still closed.
        for stage in [0.0f32, 0.5] {
            let hips = turned(Limb::Pelvis, kicking(stage));
            let chest = turned(Limb::Torso, kicking(stage));
            assert!(
                hips < chest - 0.05,
                "chest ahead of the hips at {stage}: {hips} vs {chest}"
            );
        }
        // And the turn reverses between the backswing and the follow through.
        assert!(turned(Limb::Pelvis, kicking(-0.8)) * turned(Limb::Pelvis, kicking(0.8)) < 0.0);
    }

    /// A keeper's throw is the same swing sent to his arm — and it must go
    /// over the top, not underarm.
    #[test]
    fn a_throw_goes_over_the_top() {
        let throwing = |swing: f32| {
            let mut gait = still();
            gait.swing = swing;
            gait.foot = 1.0;
            gait.throwing = 1.0;
            gait
        };
        let cocked = glove(1.0, throwing(-0.8));
        let release = glove(1.0, throwing(0.0));
        let after = glove(1.0, throwing(0.8));
        assert!(cocked.z < -0.1, "not cocked back: {cocked:?}");
        assert!(
            release.y > cocked.y,
            "released below the wind-up: {release:?}"
        );
        assert!(after.z > release.z, "no follow through: {after:?}");
        // Over the top: the hand never drops back down past the hip on the
        // way, which is what a linear interpolation between the two end
        // angles would have done.
        for step in 0..=32 {
            let swing = -0.8 + step as f32 / 20.0;
            let hand = glove(1.0, throwing(swing));
            assert!(hand.y > 1.05, "underarm at {swing}: {hand:?}");
        }
        // And his legs are not involved.
        assert!((boot(1.0, throwing(0.0)) - boot(1.0, still())).length() < 1e-4);
    }

    /// Driving off the mark and pulling up short both bend a player, and in
    /// opposite directions.
    #[test]
    fn a_change_of_pace_bends_him() {
        let lean = |drive: f32| {
            let mut gait = still();
            gait.drive = drive;
            crown(gait).z
        };
        assert!(lean(1.0) > lean(0.0) + 0.15, "no drive: {}", lean(1.0));
        assert!(lean(-1.0) < lean(0.0) - 0.12, "no brake: {}", lean(-1.0));
    }

    /// The man on the ball does not run like the twenty-one who have not got
    /// it: lower over it, arms wider, and genuinely shorter.
    #[test]
    fn the_carrier_runs_over_the_ball() {
        let mut gait = still();
        gait.carrying = 1.0;
        assert!(crown(gait).y < crown(still()).y - 0.02, "not lower");
        assert!(crown(gait).z > crown(still()).z + 0.05, "not over it");
        assert!(
            glove(1.0, gait).x.abs() > glove(1.0, still()).x.abs() + 0.02,
            "arms not out"
        );
        // But still a footballer running, not a man in a crouch.
        assert!(crown(gait).y > crown(still()).y - 0.10);
    }
}
