use crate::app::bill::{Held, MemoryBill};
use crate::app::config::ViewerConfig;
use crate::app::quality::Quality;
use crate::art::textures::{Textures, Turf};
use crate::scene::crowd::{Spectators, Stand, Stature, Terrace, Throng};
use crate::scene::field::Field;
use crate::scene::net::Netting;
use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// One bank of seating, as the box it fills and the turn that points it at
/// the pitch.
///
/// A stand between the lens and the play is not scenery, it is a wall. The
/// rig used to have one place to stand, so there was one wall and the fix was
/// simply not to build it — the near touchline was left open because that is
/// where the gantry hangs. Now the rig walks all the way round, so all four
/// sides stand and whichever one it is inside steps aside instead: a camera
/// behind the goal is *in* the end stand, and not drawing that stand is what
/// being there looks like.
#[derive(Component)]
pub struct Bank {
    /// Turns a point from the world into the bank's own frame, where the box
    /// below is axis-aligned.
    frame: Quat,
    /// Half the width across the front.
    flank: f32,
    /// Centre spot to the front row. There is deliberately no far bound —
    /// see [`Bank::encloses`] for why the test is a half-space in depth.
    near: f32,
    /// Ceiling on the test: above this the lens is treated as looking over
    /// the bank rather than through it. Higher than the seating actually
    /// stands — see `SIGHTLINE_CLEARANCE`.
    top: f32,
}

impl Bank {
    /// Is this world-space point inside the bank, or behind it?
    ///
    /// "Behind" counts: a lens further out than the front row has the
    /// whole structure between it and the play, which is the same wall
    /// whether the rig is among the seats or up in the back row. So the
    /// test is a half-space in depth, bounded across the front and in
    /// height — not the closed box, which would pop the stand back on
    /// the moment the camera drifted past the top of it.
    fn encloses(&self, point: Vec3) -> bool {
        let local = self.frame * point;
        local.x.abs() <= self.flank && local.z >= self.near && local.y <= self.top
    }

    /// Hide whichever banks the camera is standing in.
    ///
    /// Without this the fourth stand is simply a wall across the shot for
    /// the quarter of the arc the rig spends behind it — which is why the
    /// near touchline was left unbuilt when the camera had one place to
    /// stand. Now it is built and steps aside instead.
    ///
    /// At a corner the rig is behind two front faces at once and both go,
    /// which is correct: from there you are looking out through the gap
    /// between them.
    pub fn cull(
        camera: Single<&GlobalTransform, With<Camera3d>>,
        mut banks: Query<(&Bank, &mut Visibility)>,
    ) {
        let eye = camera.translation();
        for (bank, mut visibility) in &mut banks {
            let wanted = if bank.encloses(eye) {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            // Only write on a change: `Visibility` is change-detected and
            // touching it every frame would dirty the whole hierarchy.
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
}

/// **One bank, planned but not yet poured.**
///
/// Everything about a flight of steps that comes off the FIXTURE, worked out
/// in one pass so the four of them can be built one frame at a time without
/// the arithmetic being done four frames apart from the venue it answers to.
pub struct BankPlan {
    terrace: Terrace,
    stand: Stand,
    /// The rotation about Y that points this bank inward.
    turn: f32,
    /// Anything that differs per bank. Without it all four draw the same
    /// crowd in the same places and the two ends are visibly one photograph.
    seed: u32,
}

/// What the desktop memory harness in [`crate::app::bill`] needs to seat a
/// bank without a `World` behind it. Nothing in the running viewer reads a
/// plan except [`Pitch::raise_bank`], which owns it.
#[cfg(test)]
impl BankPlan {
    pub(crate) fn terrace(&self) -> &Terrace {
        &self.terrace
    }

    pub(crate) fn stand(&self) -> Stand {
        self.stand
    }
}

/// **The four banks, and the frames they are still owed.**
///
/// ⚠ **All four used to go up on one frame**, at the end of the last course of
/// the bring-up, and that one frame was the largest allocation in the whole
/// session: four spectator meshes at up to 30 MB apiece, every one of them
/// alive at once, and each still alive on the NEXT frame while the renderer
/// extracted it. Computed at some 175 MiB of transient on a computer and 85 on
/// a phone.
///
/// On a desktop that is a spike nobody would ever notice. On wasm32 it is
/// permanent: linear memory has no `memory.shrink`, so the browser accounts the
/// high-water mark for the rest of the session however much of it dlmalloc
/// hands back — and on iOS crossing the tab's ceiling does not draw slowly, it
/// kills the renderer. See [`MemoryBill`](crate::app::bill::MemoryBill).
///
/// So the plan is laid on the course that builds the ground and the banks are
/// raised one per course after it, which turns one 175 MiB peak into four
/// 40 MiB ones.
///
/// ⚠ **"Freed before the next is started" is not true of one frame's gap on
/// WebGL2, and that is why a bank costs two courses rather than one.** There
/// is no `glBufferStorage` there, so wgpu's GLES backend emulates every map:
/// the mesh is packed into a `vec![0; size]` in LINEAR MEMORY with no GL
/// object behind it, and that Vec is held as a temporary resource of the
/// submission that copies it — released when a LATER submit retires that
/// submission, not on the frame that uploaded it. Raised back to back, bank
/// N's upload copy is therefore still resident while bank N+1's attribute
/// Vecs and bank N+1's own upload copy are being built: three copies of the
/// largest mesh in the scene, in the one instant that sets the tab's
/// permanent high-water mark. The idle course between them is what submits,
/// retires and frees, so dlmalloc has the block back before the next bank
/// asks for one — see [`Bringup::PER_BANK`](crate::app::bringup::Bringup).
///
/// It costs nothing else. All four share one material and one mesh layout, so
/// no course after the first queues a render pipeline — which is the thing
/// [`Bringup`](crate::app::bringup::Bringup) is spending frames on in the first
/// place, and the reason a course that queues no pipeline "costs a frame and
/// returns a frame".
#[derive(Resource)]
pub struct Stands {
    /// Still to raise. Popped off the BACK, so the plan reads in the order the
    /// banks go up: the far touchline first, because it is the one the
    /// broadcast rest shot is looking at.
    pending: Vec<BankPlan>,
    seating: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
    spectators: Spectators,
    stature: Stature,
    throng: Option<Throng>,
}

impl Stands {
    /// How many banks a ground has, which is what the bring-up spends its
    /// courses on. Named here rather than counted there, so the two cannot
    /// drift — see [`Self::plan`], which builds exactly this many.
    pub const BANKS: usize = 4;

    /// **The four banks as the flights of steps they are**: how far round, how
    /// far back off the paint, how many rows at a GREAT ground, and how high
    /// one step is. What `stature` does to the row count is the whole
    /// difference between Old Trafford and an academy pitch.
    ///
    /// **A great ground is 34 rows and 24 m of stand**, up from 21 and 13.4.
    /// Thirteen metres is a lower tier, and a bowl built to only that reads as
    /// a low wall with a great deal of sky over it whoever is playing — which
    /// is what a Moscow derby looked like.
    ///
    /// The rake that gets there is steeper than a staircase: 0.72 m up for
    /// 0.95 m back is 37°, and no step anybody walks up is built like that. It
    /// is not pretending to be one. A real ground reaches this height by
    /// STACKING tiers — a lower bowl, a cantilever, an upper ring — and one
    /// rake standing in for three is the trade this scene makes, so the rake is
    /// pitched at the steepest a real upper tier is built to rather than at the
    /// shallowest a step can be. The alternative is depth, and there is none to
    /// spend: see [`Pitch::TREAD`].
    ///
    /// The two touchlines first. The near one used to be left out because the
    /// broadcast gantry hangs over it and a stand there is a wall across the
    /// shot — but the rig walks all the way round the ground now, so leaving it
    /// out is a hole in the stadium from three quarters of the arc. It is built
    /// like the others and [`Bank::cull`] takes out whichever one the lens is
    /// inside, which is what standing in a stand actually looks like.
    ///
    /// Then both ends, rotated a quarter turn so their rows recede down the x
    /// axis instead of the z one — one each way, which is what puts them behind
    /// opposite goals.
    pub fn plan(stature: Stature) -> Vec<BankPlan> {
        let along = Field::HALF_LENGTH + Pitch::END_MARGIN;
        let across = Field::HALF_WIDTH + Pitch::SIDE_MARGIN;
        // How far each bank wraps past the corner of the playing surface. A
        // great ground carries its terracing well round the corners; a village
        // one stops at the goal line, and left at the full wrap would read as
        // a running track rather than as a small stadium.
        let touchline_span = along * 2.0 + stature.overhang(6.0, 30.0);
        let end_span = across * 2.0 + stature.overhang(4.0, 24.0);
        let side = across + Pitch::SIDE_SETBACK;
        let end = along + Pitch::END_SETBACK;

        let mut plans: Vec<BankPlan> = [
            (Stand::Side, 0.0, touchline_span, side, 34, 0.72),
            (Stand::Side, PI, touchline_span, side, 34, 0.72),
            (Stand::HomeEnd, FRAC_PI_2, end_span, end, 31, 0.70),
            (Stand::AwayEnd, -FRAC_PI_2, end_span, end, 31, 0.70),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(bank, (stand, turn, length, from, most, riser))| BankPlan {
                terrace: Terrace {
                    length,
                    rows: stature.rows(most),
                    riser,
                    tread: Pitch::TREAD,
                    from,
                    slab: Pitch::SLAB,
                },
                stand,
                turn,
                seed: bank as u32 + 1,
            },
        )
        .collect();
        // Reversed once here rather than popped off the front every course: a
        // `Vec` has no cheap front, and the order the banks appear in is worth
        // controlling — the far touchline is what the rest shot is pointed at.
        plans.reverse();
        plans
    }
}

/// Accumulates every pitch marking into a single flat mesh.
///
/// Painted lines are all the same thing — a strip of white a few centimetres
/// wide lying on the turf — so they are built as quads in one buffer rather
/// than as a few hundred entities. Circles and arcs are just segmented strips.
struct LineMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    height: f32,
    half_width: f32,
}

impl LineMesh {
    fn new(height: f32, width: f32) -> Self {
        LineMesh {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            height,
            half_width: width * 0.5,
        }
    }

    /// A straight strip between two points on the turf, given as `(x, z)`.
    /// Both ends overshoot by half the line width so corners close up.
    fn segment(&mut self, from: Vec2, to: Vec2) {
        let direction = (to - from).normalize_or_zero();
        if direction == Vec2::ZERO {
            return;
        }
        let across = Vec2::new(-direction.y, direction.x) * self.half_width;
        let from = from - direction * self.half_width;
        let to = to + direction * self.half_width;

        let base = self.positions.len() as u32;
        for corner in [from - across, from + across, to + across, to - across] {
            self.push_vertex(corner);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn rectangle(&mut self, centre: Vec2, size: Vec2) {
        let half = size * 0.5;
        let corners = [
            centre + Vec2::new(-half.x, -half.y),
            centre + Vec2::new(half.x, -half.y),
            centre + Vec2::new(half.x, half.y),
            centre + Vec2::new(-half.x, half.y),
        ];
        for index in 0..4 {
            self.segment(corners[index], corners[(index + 1) % 4]);
        }
    }

    /// Angles run from the +X axis toward +Z.
    fn arc(&mut self, centre: Vec2, radius: f32, from: f32, to: f32) {
        let steps = (((to - from).abs() / TAU) * 96.0).ceil().max(2.0) as usize;
        let mut previous = centre + Vec2::from_angle(from) * radius;
        for step in 1..=steps {
            let angle = from + (to - from) * (step as f32 / steps as f32);
            let point = centre + Vec2::from_angle(angle) * radius;
            self.segment(previous, point);
            previous = point;
        }
    }

    fn circle(&mut self, centre: Vec2, radius: f32) {
        self.arc(centre, radius, 0.0, TAU);
    }

    /// A filled spot — penalty and centre marks.
    fn spot(&mut self, centre: Vec2, radius: f32) {
        const STEPS: usize = 16;
        let hub = self.positions.len() as u32;
        self.push_vertex(centre);
        for step in 0..=STEPS {
            let angle = TAU * (step as f32 / STEPS as f32);
            self.push_vertex(centre + Vec2::from_angle(angle) * radius);
        }
        for step in 0..STEPS as u32 {
            self.indices
                .extend_from_slice(&[hub, hub + step + 1, hub + step + 2]);
        }
    }

    fn push_vertex(&mut self, point: Vec2) {
        self.positions.push([point.x, self.height, point.y]);
        self.normals.push([0.0, 1.0, 0.0]);
        self.uvs.push([0.0, 0.0]);
    }

    fn build(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// **How well this ground is kept**, and everything that follows from it.
///
/// The stands answer to the fixture — see [`Stature`] — and the pitch inside
/// them used to answer to nobody: one calibrated green, one mow, one wear
/// field, laid identically at a cup final and at an under-18s game on a
/// training pitch. A ground whose concrete says village and whose grass says
/// Wembley is a ground that reads as a stadium kit with the wrong lawn in it,
/// and it is the surface that gives it away, because the surface is most of
/// the picture.
///
/// So one number off the same fixture ([`Stature::keeping`]) grades the whole
/// playing surface, and the top of the ladder is EXACTLY the pitch that was
/// calibrated: everything here is written as a distance travelled from
/// [`Pitch::MOWN`] and its pair, so at a great ground every constant below
/// falls out of the arithmetic and leaves the picture untouched. That is
/// pinned by `a_great_ground_is_the_pitch_that_was_calibrated`, and it is the
/// only reason a graded pitch is safe to add to a colour that took an evening
/// and five rejected stops to place.
///
/// **Four things go wrong at once at a poor ground, and they are four
/// different things** — which is the point, because any one of them alone
/// reads as a rendering bug rather than as a worse pitch:
///
/// - **The green.** Thin, unfed grass with the dry stuff showing through it is
///   paler, yellower and much less saturated than a fed sward. See
///   [`Self::sward`].
/// - **The mow.** Stripes are a cylinder mower with a roller behind it. A
///   pitch cut with a rotary mower has none, and that absence is the single
///   most recognisable thing in here. See [`Self::mow`].
/// - **The wear.** A great ground's goalmouth is sanded, seeded and watered
///   between matches and is still green in April; a park pitch's is bare
///   earth. See [`Self::worn`].
/// - **The sward itself.** Patchy cover, moss and the places it never took.
///   See [`Self::rough`].
///
/// Everything else — the surround, the tile, the mip chain, the relief — comes
/// along for free, because all of it is already a tint on the one sheet and
/// the sheet is drawn in whatever green this hands it.
#[derive(Resource, Clone, Copy)]
pub struct Upkeep {
    /// 0 at a park pitch, 1 at a great ground.
    kept: f32,
}

impl Upkeep {
    /// **The other end of the pitch's colour ladder**: the same grass, unfed,
    /// thin and dry, with the ground showing through it.
    ///
    /// [`Pitch::MOWN`] is where this arrives at a great ground and carries the
    /// whole argument for that colour; this is the far end of the same walk,
    /// and it is placed the same way — by what it RENDERS as, not by what it
    /// reads as in the source. The scene is tonemapped over a flat ambient
    /// fill, so the response of a rendered channel to its albedo is close to a
    /// power law: `out = 1.27 · albedo^0.84` on the two bright channels and a
    /// little under that on blue, which is where the key light's own 0.92
    /// goes. Fitted on the one pair that is measured — `MOWN` against the
    /// rgb(52, 124, 62) it renders — and it predicts blue on that same pair to
    /// within four units out of 255, which is as much as the model is worth.
    ///
    /// Inverted through it, this pair aims the bottom of the ladder at
    /// rgb(96, 126, 78) — slightly BRIGHTER rather than darker, which is the
    /// part that is easy to get backwards. Dead and dying grass does not go
    /// dark; it loses its blue and gains red, which is desaturation with a
    /// yellow lean, and it is the same axis [`Sward::WORN`] runs a goalmouth
    /// along and [`Textures::turf`] runs a drying leaf along. Three scales,
    /// one direction.
    ///
    /// **Then measured on rendered frames**, which is the only place a viewer
    /// colour may be judged — the whole ladder shot through the loop in
    /// `match_viewer_screenshot_loop`, sampling the turf across the broadcast
    /// frame:
    ///
    /// | ground | rendered | hue | saturation |
    /// |---|---|---|---|
    /// | a great one (`kept` 1.00) | rgb(48, 118, 60) | 130° | 0.59 |
    /// | an ordinary club (0.54) | rgb(73, 126, 72) | 119° | 0.43 |
    /// | a park pitch (0.00) | rgb(96, 127, 79) | 99° | 0.38 |
    ///
    /// The extrapolation held: both graded rungs landed within two units of
    /// what the power law said they would, which is inside the noise of
    /// sampling a pitch at all. (The great ground reads a few units under the
    /// rgb(52, 124, 62) on `MOWN`'s note because this is a mean over the whole
    /// visible surface rather than that note's four patches — the code path at
    /// `kept` 1.00 is provably the old one, and the test named above is what
    /// says so.)
    const TIRED: Color = Color::srgb(0.260, 0.312, 0.240);

    /// How much harder a neglected pitch wears, and how much rougher its sward
    /// is, against a great ground's.
    ///
    /// Two constants rather than one because they are two different failures.
    /// **Wear is traffic**: the same match is played on both pitches, and what
    /// differs is whether anybody repairs the goalmouth afterwards, so this
    /// multiplies a field whose SHAPE is already right. **Roughness is
    /// husbandry**: drainage, feed and the patches that never took, which is
    /// not about where the game was played at all.
    ///
    /// Keep surface roughness below the mowing contrast so the lanes remain
    /// readable. `the_sward_never_shouts_over_the_mow` checks the ratio in
    /// linear colour space, where the shader applies it, and also keeps a
    /// park pitch visibly rougher than a well-maintained ground.
    const WORN: f32 = 2.4;
    const MOTTLE: f32 = 3.0;

    /// What the perimeter keeps of its floodlighting at the bottom of the
    /// ladder.
    ///
    /// The boards and the lit strip along their tops are a floodlit ground's;
    /// a village one has painted hoardings and whatever the sky is doing. Not
    /// nought — they are still boards, and a strip of pure black round the
    /// edge of the play would be a heavier mark on the picture than the lit
    /// one it replaced.
    const UNLIT: f32 = 0.30;

    /// Reads the fixture, through the same [`Stature`] the stands are built
    /// off.
    pub fn of(stature: Stature) -> Self {
        Upkeep {
            kept: stature.keeping(),
        }
    }

    /// A ground anywhere on the ladder, for the tests: `1.0` is the pitch that
    /// was calibrated and `0.0` is the park pitch at the other end of it.
    #[cfg(test)]
    pub(crate) const fn at(kept: f32) -> Upkeep {
        Upkeep { kept }
    }

    /// **The green the whole surface is drawn in** — the sheet, the stripes,
    /// the surround and the worn patches are all tints on it.
    ///
    /// Interpolated in the space both endpoints were WRITTEN in, which is the
    /// sRGB one. Neither was placed by arithmetic — each is a rendered picture
    /// somebody looked at — so the honest thing between them is the straight
    /// line joining the two numbers that were actually judged, and the power
    /// law in [`Self::TIRED`] then makes the walk between them close to even
    /// on screen as well.
    fn sward(&self) -> Color {
        let great = Srgba::from(Pitch::MOWN);
        let tired = Srgba::from(Self::TIRED);
        Color::srgb(
            tired.red + (great.red - tired.red) * self.kept,
            tired.green + (great.green - tired.green) * self.kept,
            tired.blue + (great.blue - tired.blue) * self.kept,
        )
    }

    /// **The stripe**, as the multiplier the band mown the other way puts on
    /// [`Self::sward`], channel by channel, in the linear space the shader
    /// works in.
    ///
    /// Two things are kept apart in here and they are easy to run together.
    ///
    /// **The RATIO never changes.** 0.743 / 0.786 / 0.867 in sRGB is what leaf
    /// bent away from the lens does against leaf bent toward it — a fact about
    /// grass and light, not about this ground — so it is read off the
    /// calibrated pair and applied to whatever green the upkeep asked for,
    /// exactly as `Pitch::MOWN`'s note says it must be. It is never
    /// re-derived and never typed in twice.
    ///
    /// **Whether there IS a stripe does change.** It takes a cylinder mower
    /// with a roller behind it to lay one, and a rotary mower over a park
    /// pitch lays none — so the ratio is faded toward unity, which is a pitch
    /// with no mow visible in it at all.
    ///
    /// Faded by the SQUARE ROOT of the upkeep, so it survives nearly all the
    /// way down and then goes. That is the right shape and not a fudge: the
    /// stripe is the cheapest thing a groundsman does — the mower is going up
    /// and down the pitch either way — so it is nearly the last thing to go,
    /// and every professional ground in the world has one. It reaches nought
    /// only where `keeping` does, which is the training ground and the
    /// non-league club.
    fn mow(&self) -> Vec3 {
        let great = Srgba::from(Pitch::MOWN);
        let against = Srgba::from(Pitch::AGAINST);
        let ratio = Vec3::new(
            against.red / great.red,
            against.green / great.green,
            against.blue / great.blue,
        );

        let sward = Srgba::from(self.sward());
        let turned = LinearRgba::from(Color::srgb(
            sward.red * ratio.x,
            sward.green * ratio.y,
            sward.blue * ratio.z,
        ));
        let sward = LinearRgba::from(Color::Srgba(sward));
        let full = Vec3::new(
            turned.red / sward.red,
            turned.green / sward.green,
            turned.blue / sward.blue,
        );

        Vec3::ONE + (full - Vec3::ONE) * self.kept.sqrt()
    }

    /// How hard the ground wears where it is played on, against a great
    /// ground's — see [`Self::WORN`].
    fn worn(&self) -> f32 {
        1.0 + (Self::WORN - 1.0) * (1.0 - self.kept)
    }

    /// How uneven the sward is, against a great ground's — see
    /// [`Self::MOTTLE`].
    fn rough(&self) -> f32 {
        1.0 + (Self::MOTTLE - 1.0) * (1.0 - self.kept)
    }

    /// **The paint**, which is the one thing on a poor pitch that is not a
    /// different colour so much as a worse one.
    ///
    /// A great ground is re-marked before every match with a wheeled
    /// transfer marker over a surface flat enough to take it. A park pitch is
    /// marked over a sward that is half moss, by somebody who did it a
    /// fortnight ago — so the line is not white, it is the grey-green of paint
    /// that has been rained on and grown through. It keeps its width: a
    /// thinner line would be a different pitch rather than a worse one, and
    /// the markings are load-bearing for reading the play.
    fn paint(&self) -> Color {
        const FRESH: Vec3 = Vec3::new(0.93, 0.95, 0.93);
        const WEATHERED: Vec3 = Vec3::new(0.74, 0.75, 0.71);
        let paint = WEATHERED + (FRESH - WEATHERED) * self.kept;
        Color::srgb(paint.x, paint.y, paint.z)
    }

    /// How much of the perimeter's floodlighting this ground has — see
    /// [`Self::UNLIT`]. Multiplies the emissive on the boards and on the lit
    /// strip above them, and nothing else: the structure is the same
    /// structure, it is simply not lit.
    fn lit(&self) -> f32 {
        Self::UNLIT + (1.0 - Self::UNLIT) * self.kept
    }
}

/// **One pass of the mower**, and everything that tells it from the pass
/// beside it.
///
/// The mow used to be a single number: a tint, per band, over one sheet
/// sampled identically everywhere on the ground. That is not a stripe, it is
/// a stripe PAINTED on, and it is most of what makes a pitch read as carpet —
/// a printed band does not know where the camera is, so it stays the darker
/// of the two from the halfway line, from behind the goal and from the deck,
/// and real ones do not.
///
/// A mowing stripe is not a second green at all. It is the same grass with its
/// leaves lying the other way: bent away from the lens a leaf shows its face
/// and the sky on it, bent toward the lens it shows its own shadow. So the
/// three things that differ between two bands here are three ways of saying
/// that one fact, and only the first of them is a colour.
#[derive(Clone, Copy)]
struct Pass {
    /// The multiplier this band puts on the sheet: one where the mower was
    /// going away from the roller, [`Upkeep::mow`] where it was coming back.
    ///
    /// Still here, and still the calibrated 16% — see [`Pitch::MOWN`]. A
    /// stadium is lit by a flat fill standing in for four corners of
    /// floodlights (there is one directional light in this scene and it is
    /// nearly overhead), so relief alone would carry a mow you had to be told
    /// about. The tint is the contrast; everything below is what makes it
    /// grass.
    tint: Vec3,
    /// Which way the leaves are lying, as a mirror of the sheet down the mow
    /// line: `1.0` with the roller, `-1.0` against it.
    ///
    /// This is the half of a stripe a tint could never carry. The UVs mirror,
    /// which turns the sheet's own comb round — and the TANGENT FRAME mirrors
    /// with them, which turns the relief round, so the two bands present
    /// opposite faces of every leaf to the one light in the scene. That
    /// contrast is worked out per pixel from the geometry rather than baked
    /// into a vertex, which is why it holds up as the rig walks round the
    /// ground.
    ///
    /// Free: no second sheet, no second material, and above all no second
    /// shader — the one thing this scene cannot afford (see
    /// [`crate::app::bringup`], where a program the browser has not seen costs
    /// about six seconds to link).
    facing: f32,
    /// Where in the sheet this pass starts, in tiles.
    ///
    /// The tile is two metres across a pitch that is a hundred, so one
    /// continuous set of UVs prints the same square of grass fifty times down
    /// the length of the ground and does it in lockstep across every band.
    /// That is a lattice, and the eye finds a lattice at any contrast once it
    /// repeats — the same failure [`Textures::turf`] refuses to put anything
    /// larger than a leaf in the tile to avoid. A phase per band breaks the
    /// columns of it; `facing` breaks the rows.
    phase: Vec2,
    /// The two mow lines this strip lies between.
    ///
    /// Held as the line's INDEX rather than as a position, because a mow line
    /// is not straight — see [`Sward::drift`].
    lines: [usize; 2],
}

impl Pass {
    /// **How deep each pass was rolled**, as a multiplier on the stripe.
    ///
    /// A mow is not fifteen identical bands. The roller bites harder on one
    /// pass than on the next — the operator's speed, how wet it was under him,
    /// how much of the last pass he overlapped — and a drone shot of any real
    /// ground shows it plainly: some stripes shout and the one beside them is
    /// barely there. Fifteen bands laid to the same depth to four decimal
    /// places is the most mechanical thing left on this surface once the lines
    /// themselves have stopped being ruled, and it is the one a plan view
    /// finds first.
    ///
    /// A quarter either way, hashed off the band's own index so a ground looks
    /// the same every time it is built, and then **divided by its own mean
    /// over the bands that carry a stripe** — which is what makes this exactly
    /// free. However the individual passes came out, the average of the mow is
    /// the calibrated pair and stays the calibrated pair; see
    /// [`Pitch::MOWN`] for why nothing may move it, and
    /// `wearing_the_grass_does_not_repaint_it` for what holds it there.
    ///
    /// Bands the mower left going away are untouched. There is no stripe on
    /// them to be deeper or shallower — they are the sheet as drawn, and the
    /// whole pair is written as a distance from it.
    fn rolled() -> [f32; Pitch::STRIPES] {
        /// How much deeper the deepest pass is rolled than the mean.
        const BITE: f32 = 0.26;

        let mut rolled = [1.0f32; Pitch::STRIPES];
        let mut total = 0.0;
        let mut passes = 0.0;
        for line in (1..Pitch::STRIPES).step_by(2) {
            rolled[line] = 1.0 + BITE * (Sward::hash(Vec2::new(line as f32, 17.0)) * 2.0 - 1.0);
            total += rolled[line];
            passes += 1.0;
        }
        for line in (1..Pitch::STRIPES).step_by(2) {
            rolled[line] *= passes / total;
        }
        rolled
    }

    /// The band between mow line `line` and the next one.
    ///
    /// Odd bands are the mower coming back: dimmed by `turned` as deeply as
    /// this pass was rolled, and mirrored — the same statement made twice, in
    /// the two places it can be seen from.
    fn mown(line: usize, turned: Vec3, rolled: f32) -> Pass {
        let back = line % 2 == 1;
        Pass {
            tint: Vec3::ONE + (turned - Vec3::ONE) * f32::from(back) * rolled,
            facing: if back { -1.0 } else { 1.0 },
            phase: Vec2::new(
                Sward::hash(Vec2::new(line as f32, 11.0)),
                Sward::hash(Vec2::new(line as f32, 29.0)),
            ),
            lines: [line, line + 1],
        }
    }

    /// Ground no mower ever went over: one strip of the sheet as drawn,
    /// between two straight edges.
    ///
    /// Line `0` and line [`Pitch::STRIPES`] are the goal lines. They are where
    /// the pitch stops rather than anything a mower left, so they are the two
    /// lines in the set that do not wander — which is what lets the surround
    /// go through the same mechanism as a band instead of round it.
    const UNMOWN: Pass = Pass {
        tint: Vec3::ONE,
        facing: 1.0,
        phase: Vec2::ZERO,
        lines: [0, Pitch::STRIPES],
    };
}

/// The playing surface as one mesh, with the state of the grass written into
/// its vertices.
///
/// [`Textures::turf`] draws a leaf and nothing larger, and says so at length:
/// the tile repeats every two metres, and anything in it above blade scale
/// comes back as a two-metre lattice of blotches, because the eye finds a grid
/// at any contrast once it repeats. That note ends by saying the unevenness a
/// real pitch has is at the scale of a penalty area, which is not something a
/// tile can hold.
///
/// This is where it goes instead. A grid of vertices over the playing surface
/// carries a field authored ONCE across the whole ground in world space — so
/// there is nothing to repeat, and features may be any size they like. Three
/// things ride on it:
///
/// - **The mow.** The stripe shade was a material each and a mesh each; it is
///   a vertex colour now, which is what lets the rest of this share one
///   surface with it.
/// - **Wear.** A pitch that has had a match played on it is not uniform: the
///   goalmouths are scuffed through, the penalty spots and the centre spot are
///   worn discs, and those three are most of what tells a played pitch from a
///   painted rectangle at the distance a broadcast camera watches from.
/// - **Unevenness.** Slow variation at ten and twenty metres — a sward is
///   laid, drained and shaded unevenly, and no real one is a single tone.
///
///
/// Cost: this replaces fifteen entities with one, and the grid is some fifty
/// thousand triangles the GPU never notices — the frame is spent per-entity,
/// not per-pixel or per-vertex (see `perf`, and the measurement that the scene
/// renders in the same time at 720p and at 4K).
struct Sward {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// Which mow stripe this vertex is in, as the multiplier on the sheet.
    tints: Vec<Vec3>,
    /// What the ground itself is doing here — wear and unevenness together,
    /// before the pass that takes the average back out.
    ground: Vec<Vec3>,
    indices: Vec<u32>,
    /// How well this one is looked after. Scales both halves of the field
    /// above; see [`Upkeep`].
    upkeep: Upkeep,
}

impl Sward {
    /// How far apart the vertices are, in metres.
    ///
    /// Set by the smallest thing the field has to hold, which is the worn disc
    /// around a penalty spot — a couple of metres across, so half a metre puts
    /// four or five cells through it and it arrives round rather than
    /// diamond-shaped. Halving this again quadruples a triangle count that is
    /// already free; it would not make anything look better.
    const CELL: f32 = 0.5;

    /// Five centimetres of blending on each side keeps the mowing boundary
    /// crisp while retaining a small transition between the roller passes.
    const FEATHER: f32 = 0.05;

    /// What wearing the grass away does to its colour, per channel, at the
    /// point where it is worn through completely.
    ///
    /// Up in red, barely up in green, DOWN in blue. Worn turf is not simply
    /// darker grass or lighter grass: it is thin grass with the dry stuff
    /// under it showing through, so it loses its blue and gains a little
    /// overall value. The same axis [`Textures::turf`] runs a drying leaf
    /// along, at the scale of a goalmouth rather than of a leaf.
    ///
    /// Deliberately short of what a February goalmouth looks like. This is a
    /// multiplier on a pitch whose colour has been calibrated against
    /// broadcast footage, and the mud-bath version of it reads as a different
    /// green rather than as the same green worn.
    ///
    /// Short **at a great ground**, which is the one this was written for and
    /// the one where it is right: a goalmouth that is sanded, seeded and
    /// watered between matches does not go bare. [`Upkeep::worn`] carries the
    /// field past 1.0 further down the ladder, where nobody repairs it and the
    /// February version is exactly what it looks like.
    const WORN: Vec3 = Vec3::new(0.22, 0.10, -0.06);

    /// The playing surface, mown in [`Pitch::STRIPES`] bands.
    ///
    /// `tile` is how much ground one repeat of the blade sheet covers; the
    /// second stripe as a fraction of the first is [`Upkeep::mow`]'s to say,
    /// and at a poor enough ground it is no fraction at all.
    fn mow(upkeep: Upkeep, tile: f32) -> Mesh {
        let mut sward = Sward {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            tints: Vec::new(),
            ground: Vec::new(),
            indices: Vec::new(),
            upkeep,
        };
        let turned = upkeep.mow();
        let rolled = Pass::rolled();
        let stripe = Field::LENGTH / Pitch::STRIPES as f32;
        for line in 0..Pitch::STRIPES {
            let from = -Field::HALF_LENGTH + stripe * line as f32;
            sward.block(
                from,
                from + stripe,
                Pass::mown(line, turned, rolled[line]),
                tile,
            );
        }
        sward.build()
    }

    /// The grass beyond the touchlines: the same field with the mow left out
    /// of it.
    ///
    /// It gets the treatment for the same reason the pitch does and rather
    /// more urgently — it is a single flat quad three hundred metres across,
    /// which is as flat as anything in the scene can be, and half of every low
    /// shot is made of it. No stripes, because a mower does not go out here,
    /// and no wear, because nobody plays on it.
    ///
    /// Coarser than the playing surface by a factor of eight: there is nothing
    /// out here at the scale of a penalty spot to resolve, and this quad is
    /// eleven times the area of the pitch.
    fn rough(upkeep: Upkeep, size: Vec2, tile: f32) -> Mesh {
        let mut sward = Sward {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            tints: Vec::new(),
            ground: Vec::new(),
            indices: Vec::new(),
            upkeep,
        };
        sward.grid(
            Vec2::new(-size.x * 0.5, -size.y * 0.5),
            Vec2::new(size.x * 0.5, size.y * 0.5),
            Self::CELL * 8.0,
            Pass::UNMOWN,
            tile,
            false,
        );
        sward.build()
    }

    /// One mown band, with its own vertices along both mow lines.
    ///
    /// `from` and `to` are where the arithmetic puts those two lines. Where
    /// the mower actually left them is [`Self::drift`]'s to say, and the band
    /// is dragged onto them in [`Self::grid`] — so the nominal width is what
    /// decides the vertex count and the real one is what decides where those
    /// vertices land.
    fn block(&mut self, from: f32, to: f32, pass: Pass, tile: f32) {
        self.grid(
            Vec2::new(from, -Field::HALF_WIDTH),
            Vec2::new(to, Field::HALF_WIDTH),
            Self::CELL,
            pass,
            tile,
            true,
        );
    }

    /// **Where a mow line actually is**, as the distance off where the
    /// arithmetic put it, in metres.
    ///
    /// Fifteen bands of exactly `LENGTH / 15`, ruled dead straight from one
    /// touchline to the other and spaced to the millimetre, is the geometry of
    /// a printed carpet. It is not the geometry of a mown pitch, and the
    /// regularity is legible from the gantry long before any single stripe is:
    /// a man on a mower walking sixty-eight metres does not hold a line to the
    /// centimetre, and he does not hold the same one on the way back.
    ///
    /// Two sinusoids down the width at periods with no common factor, plus a
    /// constant for where the pass was started, all three phased off the
    /// line's own index through the golden angle — so no two lines wander
    /// alike and the set of them never lines up into a second pattern of its
    /// own. Under a quarter of a metre at the very worst, against a band seven
    /// metres wide: three per cent, which reads as a hand rather than as a
    /// wobble.
    ///
    /// ⚠ **Zero-sum by construction.** A line is shared: every centimetre it
    /// takes from the band below it, it gives to the band above. And a band's
    /// vertex count comes off its NOMINAL width, so no band gains or loses
    /// vertices either. Which is what keeps this out of the average colour of
    /// the surface — the contract `wearing_the_grass_does_not_repaint_it`
    /// holds the whole field to, and the one thing nothing in here may touch.
    fn drift(line: usize, z: f32) -> f32 {
        /// The most a line strays off true, in metres.
        const WANDER: f32 = 0.19;

        // The first and last are the goal lines. The grass stops there; no
        // mower left them, and they do not move.
        if line == 0 || line == Pitch::STRIPES {
            return 0.0;
        }
        let phase = line as f32 * 2.399_963;
        let settle = Self::hash(Vec2::new(line as f32, 5.0)) - 0.5;
        WANDER
            * (0.50 * settle
                + 0.60 * (z / 21.3 + phase).sin()
                + 0.36 * (z / 8.7 - phase * 1.7).cos())
    }

    /// A rectangle of ground, with its own vertices all the way round.
    fn grid(&mut self, from: Vec2, to: Vec2, cell: f32, pass: Pass, tile: f32, played: bool) {
        let span = to - from;
        let down = (span.x / cell).ceil().max(1.0) as usize + if played { 2 } else { 0 };
        let across = (span.y / cell).ceil().max(1.0) as usize;
        let base = self.positions.len() as u32;
        // Hoisted rather than asked per vertex: the pair is the same for the
        // whole ground, and this loop runs a hundred thousand times.
        let (worn, rough) = (self.upkeep.worn(), self.upkeep.rough());
        let rolled = Pass::rolled();
        let line = pass.lines[0];
        let edge_tints = if played {
            [line.saturating_sub(1), (line + 1).min(Pitch::STRIPES - 1)].map(|neighbour| {
                (pass.tint + Pass::mown(neighbour, self.upkeep.mow(), rolled[neighbour]).tint) * 0.5
            })
        } else {
            [pass.tint; 2]
        };

        for row in 0..=down {
            // Extra rows keep the overlap narrow without refining the whole pitch.
            let distance = if played && row > 0 && row < down {
                Self::FEATHER
                    + (span.x - 2.0 * Self::FEATHER) * (row - 1) as f32 / (down - 2) as f32
            } else {
                span.x * row as f32 / down as f32
            };
            let across_the_band = distance / span.x;
            let nominal = from.x + span.x * across_the_band;
            for column in 0..=across {
                let z = from.y + span.y * (column as f32 / across as f32);
                // Onto the two lines the mower actually left, and linearly
                // between them, so the strip stays a strip and its cells stay
                // the size they were meant to be.
                let near = Self::drift(pass.lines[0], z);
                let far = Self::drift(pass.lines[1], z);
                let x = nominal + near + (far - near) * across_the_band;
                self.positions.push([x, 0.0, z]);
                self.normals.push([0.0, 1.0, 0.0]);
                let uv = Self::grass_uv(Vec2::new(x, z), pass, tile);
                self.uvs.push(uv.to_array());
                let tint = if row == 0 {
                    edge_tints[0]
                } else if row == down {
                    edge_tints[1]
                } else {
                    pass.tint
                };
                self.tints.push(tint);
                self.ground
                    .push(Self::ground(Vec2::new(x, z), played, worn, rough));
            }
        }

        let stride = (across + 1) as u32;
        for row in 0..down as u32 {
            for column in 0..across as u32 {
                let corner = base + row * stride + column;
                self.indices.extend_from_slice(&[
                    corner,
                    corner + 1,
                    corner + stride,
                    corner + 1,
                    corner + stride + 1,
                    corner + stride,
                ]);
            }
        }
    }

    /// Slowly bend the sampling grid so a leaf cluster never repeats in lockstep.
    /// The displacement changes over metres, keeping individual blades undistorted.
    fn grass_uv(point: Vec2, pass: Pass, tile: f32) -> Vec2 {
        let warp = Vec2::new(
            Self::grain(point / 5.3 + Vec2::new(13.7, 41.2)),
            Self::grain(point / 4.7 + Vec2::new(73.1, 9.4)),
        ) * 0.55;
        let uv = point / tile + warp;
        Vec2::new(uv.x, pass.facing * uv.y) + pass.phase
    }

    /// What the grass is doing at one point, as a multiplier on the mown
    /// shade — before normalisation, so this is free to have any average it
    /// likes and [`Self::build`] takes it back out.
    ///
    /// `bare` and `patchy` are how much harder this ground wears and how much
    /// rougher its sward is than a great one's — [`Upkeep::worn`] and
    /// [`Upkeep::rough`], both 1.0 at the top of the ladder, where every term
    /// below is exactly what it was before there was a ladder.
    ///
    /// ⚠ Note what does NOT scale with them: the pitch's own colour. That is
    /// [`Upkeep::sward`]'s, it reaches the shader through the sheet, and the
    /// normalisation in [`Self::build`] exists precisely so that nothing in
    /// here can move it. Wear says WHERE the grass is different; how green the
    /// grass is in the first place is decided somewhere else and stays
    /// decided — which is what keeps a graded pitch from being a second,
    /// silent way to repaint a calibrated one.
    fn ground(point: Vec2, played: bool, bare: f32, patchy: f32) -> Vec3 {
        // `played` rather than trusting the wear field to be zero out here:
        // the surround runs UNDER the pitch, so its grid samples the
        // goalmouths through it. Nothing of that is ever seen — it is a
        // centimetre below opaque turf — but it would land in the average
        // this is normalised against, and quietly grade the ground outside
        // the touchlines by what happens inside them.
        let worn = if played {
            Self::wear(point) * bare
        } else {
            0.0
        };
        // Growth and moisture vary in hue as well as brightness. Keep this
        // in world space so patches never recur with the blade texture.
        let growth = Self::grain(point / 2.3 + Vec2::new(53.2, 19.6)) * patchy;
        Vec3::new(
            1.0 + Self::WORN.x * worn,
            1.0 + Self::WORN.y * worn,
            1.0 + Self::WORN.z * worn,
        ) * (1.0 + Self::unevenness(point) * patchy)
            * (Vec3::ONE + Vec3::new(0.065, 0.015, -0.045) * growth)
    }

    /// How hard the grass here has been used, nought to one.
    ///
    /// **One table, one mechanism.** Every row is somewhere a football match
    /// is played: where the middle of it is, how far it reaches down the pitch
    /// and across it, and how bare it gets. Each is mirrored about both axes,
    /// because a pitch is used symmetrically and writing the far goalmouth out
    /// by hand is how two ends of a ground drift apart.
    ///
    /// `max` rather than a sum, because they overlap: the penalty spot sits
    /// inside the goalmouth, and adding them would take that one patch past
    /// bare earth while the rest of the pitch stayed green.
    ///
    /// The three rows that used to be all of it — the goalmouths, the penalty
    /// spots, the centre — are the three places a pitch wears when nobody has
    /// run anywhere else, and a surface worn in exactly three round places is
    /// a surface with three decals on it. The two that join them are the
    /// traffic with no landmark to sit on: the channel a winger and a linesman
    /// spend ninety minutes in a couple of metres inside each touchline, and
    /// the broad middle of the ground where every phase of play that is not a
    /// shot happens. Both are worth a fraction of a goalmouth and neither has
    /// an edge anybody could point at, which is the whole of what they are
    /// for.
    fn wear(point: Vec2) -> f32 {
        /// Where the traffic is, how far it reaches along the pitch and across
        /// it, and what it is worth at the middle.
        const TRAFFIC: [(Vec2, Vec2, f32); 5] = [
            // The goalmouth. An ellipse rather than a disc, and wider across
            // than it is deep, because what wears is the ground a keeper
            // covers and the ground defenders turn on in front of him — the
            // goal area and a good way either side of it. Centred four metres
            // off the line: the very back of it is behind the keeper and sees
            // less traffic than the front edge of the six-yard box.
            (
                Vec2::new(Field::HALF_LENGTH - 4.0, 0.0),
                Vec2::new(8.0, 12.0),
                1.00,
            ),
            // The penalty spot, which is stood on, run up to and dug out of.
            (
                Vec2::new(Field::HALF_LENGTH - Field::PENALTY_SPOT_DISTANCE, 0.0),
                Vec2::splat(2.6),
                0.55,
            ),
            // The centre spot: every kickoff, and every restart after a goal.
            (Vec2::ZERO, Vec2::splat(3.6), 0.45),
            // The channel inside the touchline, which is the one piece of
            // ground somebody is running on all afternoon whether the ball is
            // there or not. It stops well short of the corner — a corner is
            // the one part of a pitch nobody spends a match on.
            (
                Vec2::new(0.0, Field::HALF_WIDTH - 2.2),
                Vec2::new(Field::HALF_LENGTH * 0.72, 4.5),
                0.30,
            ),
            // And the middle of the ground, which is most of a match and has
            // no shape at all: a wide, shallow patch that never reaches a
            // quarter of what a goalmouth does.
            (Vec2::ZERO, Vec2::new(26.0, 22.0), 0.16),
        ];

        TRAFFIC
            .iter()
            .flat_map(|(at, reach, weight)| {
                [
                    Vec2::new(-1.0, -1.0),
                    Vec2::new(-1.0, 1.0),
                    Vec2::new(1.0, -1.0),
                    Vec2::new(1.0, 1.0),
                ]
                .map(|mirror| Self::blob(point - *at * mirror, *reach) * *weight)
            })
            .fold(0.0f32, f32::max)
    }

    /// A soft patch: one at the middle, nought at `radius` and beyond, with no
    /// corner at either end.
    fn blob(offset: Vec2, radius: Vec2) -> f32 {
        let reach = (offset / radius).length();
        if reach >= 1.0 {
            return 0.0;
        }
        let fade = 1.0 - reach;
        fade * fade * (3.0 - 2.0 * fade)
    }

    /// Drainage-scale variation, metre-scale patches and fine sward density.
    /// All are sampled in world space, independent of the repeating blade tile.
    fn unevenness(point: Vec2) -> f32 {
        let drift = 0.022 * (point.x / 23.7 + 0.6).sin() * (point.y / 17.3 - 1.1).cos()
            + 0.014 * (point.x / 9.1 - 2.2).sin() * (point.y / 11.7 + 0.4).cos();
        let mottle = 0.024 * Self::grain(point / 4.7)
            + 0.013 * Self::grain(point / 2.9 + Vec2::new(7.7, 43.1))
            + 0.011 * Self::grain(point / 1.9 + Vec2::new(31.4, 17.2))
            + 0.022 * Self::grain(point / 0.73 + Vec2::new(67.2, 5.8));
        drift + mottle
    }

    /// Value noise on a one-unit lattice: a hashed height at every corner,
    /// eased between. Comes back in −1..1.
    fn grain(point: Vec2) -> f32 {
        let cell = point.floor();
        let across = point - cell;
        let ease = across * across * (Vec2::splat(3.0) - 2.0 * across);
        let corner = |x: f32, y: f32| Self::hash(cell + Vec2::new(x, y));
        let near = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * ease.x;
        let far = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * ease.x;
        (near + (far - near) * ease.y) * 2.0 - 1.0
    }

    /// One lattice corner's value, nought to one.
    ///
    /// Integer coordinates go through a wrapping cast on purpose: a pitch runs
    /// either side of the origin, and a hash that folded −3 onto 3 would
    /// mirror the whole field about the halfway line.
    fn hash(cell: Vec2) -> f32 {
        let mut state = (cell.x as i32 as u32)
            .wrapping_mul(374_761_393)
            .wrapping_add((cell.y as i32 as u32).wrapping_mul(668_265_263));
        state ^= state >> 13;
        state = state.wrapping_mul(1_274_126_177);
        state ^= state >> 16;
        state as f32 / u32::MAX as f32
    }

    /// Normalises the ground term to an average of one and folds it into the
    /// mow.
    ///
    /// The pitch's colour is calibrated — see [`Pitch::MOWN`], and the note
    /// there about a third off the albedo being nineteen per cent off the
    /// picture. Wear and unevenness are meant to say where the grass is
    /// different, not what colour the grass is, so whatever they do to the
    /// average is taken back out here. The mow is untouched by it: the pair of
    /// shades and their sixteen per cent survive exactly as written.
    fn build(mut self) -> Mesh {
        let mut mean = Vec3::ZERO;
        for ground in &self.ground {
            mean += *ground;
        }
        mean /= self.ground.len().max(1) as f32;
        let gain = Vec3::new(1.0 / mean.x, 1.0 / mean.y, 1.0 / mean.z);

        let colours: Vec<[f32; 4]> = self
            .tints
            .iter()
            .zip(&self.ground)
            .map(|(tint, ground)| {
                let colour = *tint * *ground * gain;
                [colour.x, colour.y, colour.z, 1.0]
            })
            .collect();

        // Derive the frame from the warped UVs. Our procedural relief encodes
        // increasing V in green, so retain that sign when passes are mirrored.
        let mut axes = vec![Vec3::ZERO; self.positions.len()];
        let mut signs = vec![0.0; self.positions.len()];
        for triangle in self.indices.chunks_exact(3) {
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            let ab = Vec3::from(self.positions[b]) - Vec3::from(self.positions[a]);
            let ac = Vec3::from(self.positions[c]) - Vec3::from(self.positions[a]);
            let uv_ab = Vec2::from(self.uvs[b]) - Vec2::from(self.uvs[a]);
            let uv_ac = Vec2::from(self.uvs[c]) - Vec2::from(self.uvs[a]);
            let determinant = uv_ab.perp_dot(uv_ac);
            let axis = (ab * uv_ac.y - ac * uv_ab.y) / determinant;
            for index in [a, b, c] {
                axes[index] += axis;
                signs[index] = determinant.signum();
            }
        }
        let tangents: Vec<[f32; 4]> = axes
            .into_iter()
            .zip(signs)
            .map(|(axis, sign)| axis.normalize().extend(sign).to_array())
            .collect();

        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            std::mem::take(&mut self.positions),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, std::mem::take(&mut self.normals))
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, std::mem::take(&mut self.uvs))
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours)
        .with_inserted_indices(Indices::U32(std::mem::take(&mut self.indices)))
    }
}

/// The stadium: turf, painted markings, both goals and the ground around them.
pub struct Pitch;

impl Pitch {
    /// Mowing stripes. An odd count keeps the two halves mirror-imaged.
    const STRIPES: usize = 15;
    const LINE_WIDTH: f32 = 0.14;
    const LINE_HEIGHT: f32 = 0.012;

    /// The direction the stadium light travels — down, and a little across.
    ///
    /// Public because the contact shadows have to agree with it. There are no
    /// shadow maps in this scene (see below), so every shadow on the turf is
    /// a painted disc placed by hand; a disc placed symmetrically under a body
    /// lit from 28° off the vertical is the one thing that makes a figure look
    /// pasted onto the grass rather than standing on it. One constant, two
    /// readers, no chance of them disagreeing.
    pub const SUN: Vec3 = Vec3::new(-0.4, -1.0, 0.35);

    /// **How much ground there is outside the playing surface**, in metres:
    /// across the touchlines, and behind the goal lines. The advertising
    /// hoardings stand at the end of it and the banks of seating begin
    /// [`Self::SIDE_SETBACK`] beyond them, so this pair and that one are the
    /// whole run-off — everything between the paint and the first thing a
    /// camera can walk into.
    ///
    /// Read by `ChangeoverShot`, which puts a lens on the grass a few metres
    /// behind a man and has to know where the ground stops.
    pub const SIDE_MARGIN: f32 = 3.4;
    pub const END_MARGIN: f32 = 4.6;

    /// **How far behind the hoardings the front row of seating stands**, in
    /// metres: down a touchline, and behind a goal.
    ///
    /// On top of the margins above, so the apron is 7.4 m of ground from the
    /// touchline to the first step and 7.0 m from the goal line to it. More
    /// down the sides than behind the goals, which is where the room actually
    /// goes at a real ground: the benches, the technical areas, the fourth
    /// official and the photographers are all on a touchline and none of them
    /// is behind a goal.
    ///
    /// **The side figure is also what uncovers the near touchline**, and that
    /// is the reason it moved — 2.1 m put the front row 5.5 m off the paint.
    /// The gantry is only 18 m up (`TvCamera::HEIGHT`), so a bank too short
    /// for [`Bank::cull`] to take out is still tall enough to stand between
    /// the lens and the play: its crest cuts across the picture and the strip
    /// of pitch behind it is not in shot at all. Moving the bank back carries
    /// its crest toward the lens without raising it, so the sightline that
    /// grazes that crest comes down on the turf sooner, and about 2.4 m of
    /// pitch comes back for every metre spent here.
    ///
    /// Measured at the statures where it bites — the ones whose banks just
    /// miss the cull, which is where the whole cost of this falls:
    ///
    /// - **fourteen rows**, the tallest bank that still escapes the cull, hid
    ///   21.2 m of pitch and now hides 16.7. That is a stand standing across
    ///   the near touchline and a good way in toward the centre circle.
    /// - **ten rows** hid 8.7 m and now hides 5.4.
    /// - **a five-step village terrace** hid 27 cm and now hides none of it.
    /// - **a great ground** is culled and was never in this at all.
    ///
    /// ⚠ The rest of that strip is the cull's to fix, not this constant's: no
    /// setback the ground has room for clears a fourteen-row bank, which would
    /// want its front row 14 m off the touchline. What this buys is the near
    /// third of the pitch back, not all of it.
    ///
    /// The ceiling is [`Self::TREAD`]'s: the back row of a great ground's
    /// touchline bank has to finish short of the broadcast gantry, and every
    /// metre here is a metre off that clearance.
    pub(crate) const SIDE_SETBACK: f32 = 4.0;
    pub(crate) const END_SETBACK: f32 = 2.4;

    /// **Depth of one row of terracing**, front to back.
    ///
    /// 0.95 m, which is a real row of seats — the number is set by where a
    /// person's knees go, and every ground in the world lands between 0.80 and
    /// 0.95. It was 1.25 while the banks were low and depth was free; it is
    /// not free now. A bank's depth is what stands between the back row and
    /// the broadcast gantry, which is parked at `HALF_WIDTH + SETBACK` — 82 m
    /// from the centre spot — and a rake that runs past it puts the lens
    /// inside the terracing.
    ///
    /// At 0.95 the tallest touchline bank finishes at 74 m and clears it —
    /// 72 before [`Self::SIDE_SETBACK`] took two metres of that clearance to
    /// get the near bank off the pitch.
    pub(crate) const TREAD: f32 = 0.95;

    /// **How thick a step's slab is**, as a multiple of the riser.
    ///
    /// Comfortably over one, so each step overlaps the one below it and the
    /// flight comes out as a solid bank rather than as a stack of shelves with
    /// daylight between them.
    pub(crate) const SLAB: f32 = 1.9;

    /// Deep, muted forest greens for the floodlit match view. A warmer balance
    /// and less green-channel brightness keep vivid TV displays from making
    /// the well-kept turf look fluorescent.
    /// The material stays matte; blade relief and wear supply surface detail.
    ///
    /// Both mowing shades move together, preserving the roughly 21% value
    /// contrast and the slight hue shift between adjacent roller passes.
    /// These are texture colours before lighting and tone mapping, rather
    /// than final screen colours. Upkeep derives every ground from this pair.
    pub(crate) const MOWN: Color = Color::srgb(0.140, 0.210, 0.145);
    const AGAINST: Color = Color::srgb(0.1041, 0.1651, 0.1256);

    /// How much pitch one tile of [`Textures::turf`] covers, in metres.
    ///
    /// Two, which against that texture's 1024 texels is 512 to the metre. Any
    /// larger and the blades go soft before the camera is down on the deck
    /// (`CameraFlight::HEIGHT` lets it to 0.6 m); any smaller and the tile
    /// repeats often enough that the eye can start hunting for the period.
    const TURF_TILE: f32 = 2.0;

    /// The surround, as a fraction of the pitch's own light in the space the
    /// shader multiplies in.
    ///
    /// The grass beyond the touchlines is the same turf, unlit and in the
    /// shadow of the stand, so it keeps the pitch's hue and simply loses most
    /// of its value — it was a near-black blue-green (0.05 / 0.13 / 0.07) that
    /// belonged to nothing else in the scene, and a surround in a different hue
    /// family from the pitch reads as a hole cut in the world rather than as
    /// ground. A fraction rather than a colour of its own for the same reason
    /// the stripes are: it now carries the same blade texture, so the only
    /// thing that may differ is the light falling on it, and darkening the
    /// pitch has to darken this with it or the hole comes back.
    ///
    /// Blue held up against red, because a shadow outdoors is lit by sky.
    const SHADOW: Color = Color::linear_rgb(0.165, 0.139, 0.232);

    /// The playing surface and the light that falls on it — the first course
    /// of the bring-up.
    ///
    /// Split from the rest of the stadium on purpose: this is one of the four
    /// materials whose shader takes about six seconds to link in a browser
    /// that has not seen it before, and a course per frame is what gives the
    /// page the thread back between them. See [`crate::app::bringup`].
    pub fn lay_turf(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
    ) {
        // **The fixture is read on the FIRST course, not the last.** The
        // stands are built on the fifth and read it there, which was fine
        // while the ground was the only thing that answered to the venue —
        // but the grass answers to it now, and the grass is course one. Read
        // here and handed down as a resource, so the whole scene is graded
        // off one reading of one document and no two courses can disagree
        // about what kind of ground this is.
        let upkeep = Upkeep::of(Stature::of(&config.venue));

        // The one green in the scene, and every other surface is a tint on
        // it: the stripes, the worn patches, the ground beyond the touchlines.
        // So grading the pitch is grading THIS, and nothing downstream has to
        // know that a ladder exists.
        let grass = Textures::turf(&mut images, upkeep.sward());
        Self::spawn_playing_surface(&mut commands, &mut meshes, &mut materials, &grass, upkeep);
        // Kept for the surround, which is laid on the next frame off the same
        // sheet — generating it twice would cost a second 1024-square texture
        // and its mip chain for a picture nobody could tell apart.
        commands.insert_resource(grass);
        commands.insert_resource(upkeep);

        // A stadium is lit from four corners at once, so almost nothing on the
        // pitch falls into true shadow. One directional light standing in for
        // all of them has to be paired with a generous ambient (see the camera,
        // which carries it): on its own it leaves every surface turned away
        // from it near black, which is fine on a disc and wrong on a body with
        // limbs, once the camera is close enough to see them shaded.
        commands.spawn((
            DirectionalLight {
                color: Color::srgb(1.0, 0.98, 0.92),
                illuminance: 8_500.0,
                // Deliberately off: the only thing tall enough to cast a
                // meaningful shadow is the ball, which carries its own contact
                // disc, and cascaded shadow maps are the single most expensive
                // thing this scene could ask a WebGL2 context for.
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Self::SUN, Vec3::Y),
        ));
    }

    /// The ground itself: the grass inside the touchlines, and the grass
    /// outside them.
    ///
    /// One texture for all of it. The blades are painted in [`Self::MOWN`], and
    /// every surface here is that same grass under a different light — the
    /// stripe lying the other way, the shadow of the stand — so each is a TINT
    /// on the one sheet rather than a colour of its own. Which is the point: a
    /// second green written down anywhere in here is a green that can drift
    /// away from the first, and a pitch whose stripes are two different greens
    /// is a pitch with stripes painted on it.
    fn spawn_playing_surface(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        grass: &Turf,
        upkeep: Upkeep,
    ) {
        // One material for the whole playing surface. Both mow shades, the
        // wear and the unevenness are vertex colours on [`Sward`] — which is
        // what a second material could never have carried, since the thing
        // that varies varies from metre to metre and not from stripe to
        // stripe.
        //
        // White base colour, because the sheet is already painted in `MOWN`
        // and every vertex says what to do to it.
        let playing_surface = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(grass.albedo.clone()),
            normal_map_texture: Some(grass.relief.clone()),
            perceptual_roughness: 1.0,
            // **No specular.** Bevy gives every dielectric a white sheen —
            // `reflectance` defaults to 0.5, which is an F0 of about 0.04 —
            // and under this scene's very generous ambient fill (see the
            // directional light above, and the `AmbientLight` the camera
            // carries) that sheen is a large flat term added to every texel
            // AFTER the albedo has been multiplied in.
            //
            // Which makes it a desaturator. It is the same white everywhere,
            // so it is worth far more to the channels that have least: on one
            // and the same sheet, turning it off took 34 units off the
            // rendered red and 38 off the blue while taking only 14 off the
            // green. On a surface that covers most of the frame that is not a
            // highlight, it is a wash — and it is what held the turf grey
            // through every attempt to write a greener `Self::MOWN`, none
            // of which could reach past it.
            //
            // What is given up is real enough: wet grass under floodlights
            // does catch the light. But at `perceptual_roughness` 1.0 none of
            // it was ever a highlight anybody could point at, and the pitch
            // is lit by an ambient fill standing in for four corners of
            // floodlights rather than by anything with a direction to glint
            // off.
            reflectance: 0.0,
            ..default()
        });
        commands.spawn((
            Mesh3d(Self::stock(meshes, Sward::mow(upkeep, Self::TURF_TILE))),
            MeshMaterial3d(playing_surface),
        ));
    }

    /// The grass beyond the touchlines — the second course of the bring-up.
    ///
    /// A frame of its own rather than a few lines under the playing surface,
    /// and for one reason: it is the same vertex layout with a DIFFERENT
    /// shader — see the note on the relief below — so it is a second program
    /// for the driver to compile, and the two of them next to each other are
    /// what made the first frame of a match twelve seconds long.
    pub fn lay_surround(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        grass: Res<Turf>,
        upkeep: Res<Upkeep>,
    ) {
        // Same grass at the same scale, so the pitch does not stop at a change
        // of texture as well as a change of light.
        const SURROUND: Vec2 = Vec2::new(320.0, 260.0);
        // Same sheet, same scale, and deliberately WITHOUT the relief the
        // playing surface carries.
        //
        // A normal map earns its place by shading: it is what stops a lit
        // plane returning one value everywhere. Out here there is no light to
        // shade with — `SHADOW` puts this ground at about a sixth of the
        // pitch's value, because it is in the lee of a stand — so the term the
        // second sheet contributes is a sixth of a shading difference that was
        // subtle at full brightness, which is to say nothing anybody has ever
        // seen. What it costs is not nothing: this quad is eleven times the
        // area of the pitch and, by the note on `Sward::rough`, half of every
        // low shot, so dropping it takes a full anisotropic texture fetch and
        // the whole tangent-space path out of the fragment shader over most of
        // the lower frame. The blades stay — that is the albedo, and it is
        // what keeps the surround from reading as a hole cut in the world.
        let surround = materials.add(StandardMaterial {
            base_color: Self::SHADOW,
            base_color_texture: Some(grass.albedo.clone()),
            perceptual_roughness: 1.0,
            // **No sheen out here either**, and for a stronger reason than on
            // the pitch. `SHADOW` puts this ground at about a sixth of the
            // playing surface's value, so a specular term that is a CONSTANT
            // is a far larger share of what is left of it — and what it is
            // mostly made of is the blue-white fill, which is the one thing
            // this material's base colour exists to keep out. Measured on the
            // far band of surround, its blue against its green fell from 0.60
            // to 0.55 the moment the sheen came off. Leaving it on while the
            // pitch loses it is how the surround drifts away from the grass it
            // is supposed to be, and goes back to reading as a hole cut in
            // the world.
            reflectance: 0.0,
            ..default()
        });
        // ⚠ **Set 1 cm below the turf, not 5.** It only ever had to clear
        // the turf's own z-fighting, and 5 cm was free while nothing was
        // ever drawn out here — the ball was snapped back onto the pitch
        // the instant it crossed a line. It is not free now: a ball put out
        // of play runs into this strip and comes to rest on it (`RunOff`),
        // and the engine puts it at height zero, so at −0.05 it floated a
        // visible finger's width above the ground with its contact shadow
        // (pinned at +0.02) hanging under it. The fetching player's boots
        // did the same.
        commands.spawn((
            Mesh3d(Self::stock(
                &mut meshes,
                Sward::rough(*upkeep, SURROUND, Self::TURF_TILE),
            )),
            MeshMaterial3d(surround),
            Transform::from_xyz(0.0, -0.01, 0.0),
        ));
    }

    /// The paint — the third course of the bring-up.
    pub fn paint_markings(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        upkeep: Res<Upkeep>,
    ) {
        Self::spawn_markings(&mut commands, &mut meshes, &mut materials, *upkeep);
    }

    /// Both goals, frame and netting — the fourth course. The netting is not
    /// scenery: it is deformable, and [`Netting::ripple`] drives it from the
    /// ball.
    pub fn raise_goals(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        Netting::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
    }

    /// Hoardings, stands, the people in them and the ground they all stand on
    /// — the last course.
    ///
    /// The one course that reads the fixture. How much stadium there is comes
    /// off the venue the page handed over, so a cup final and an under-18s
    /// friendly are not played in the same building — see
    /// [`Stature`](crate::scene::crowd::Stature).
    pub fn build_stands(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
        quality: Res<Quality>,
        upkeep: Res<Upkeep>,
    ) {
        Self::spawn_ground(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &config,
            Throng::of(quality.footprint(), config.crowd.as_deref()),
            *upkeep,
        );
    }

    fn spawn_markings(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        upkeep: Upkeep,
    ) {
        let mut lines = LineMesh::new(Self::LINE_HEIGHT, Self::LINE_WIDTH);

        lines.rectangle(Vec2::ZERO, Vec2::new(Field::LENGTH, Field::WIDTH));
        lines.segment(
            Vec2::new(0.0, -Field::HALF_WIDTH),
            Vec2::new(0.0, Field::HALF_WIDTH),
        );
        lines.circle(Vec2::ZERO, Field::CENTRE_CIRCLE_RADIUS);
        lines.spot(Vec2::ZERO, 0.16);

        for side in [-1.0f32, 1.0] {
            let goal_line = side * Field::HALF_LENGTH;

            lines.rectangle(
                Vec2::new(goal_line - side * Field::PENALTY_AREA_DEPTH * 0.5, 0.0),
                Vec2::new(Field::PENALTY_AREA_DEPTH, Field::PENALTY_AREA_WIDTH),
            );
            lines.rectangle(
                Vec2::new(goal_line - side * Field::GOAL_AREA_DEPTH * 0.5, 0.0),
                Vec2::new(Field::GOAL_AREA_DEPTH, Field::GOAL_AREA_WIDTH),
            );

            let spot = Vec2::new(goal_line - side * Field::PENALTY_SPOT_DISTANCE, 0.0);
            lines.spot(spot, 0.16);

            // The penalty arc is the part of a 9.15 m circle around the spot
            // that falls outside the box.
            let reach = Field::PENALTY_AREA_DEPTH - Field::PENALTY_SPOT_DISTANCE;
            let half_sweep = (reach / Field::CENTRE_CIRCLE_RADIUS)
                .clamp(-1.0, 1.0)
                .acos();
            let facing = if side > 0.0 { PI } else { 0.0 };
            lines.arc(
                spot,
                Field::CENTRE_CIRCLE_RADIUS,
                facing - half_sweep,
                facing + half_sweep,
            );

            // Corner arcs sweep the quarter circle that faces into the pitch:
            // from the goal line round to the touchline.
            for touchline in [-1.0f32, 1.0] {
                let corner = Vec2::new(goal_line, touchline * Field::HALF_WIDTH);
                let from = -touchline * FRAC_PI_2;
                let to = if side > 0.0 { -touchline * PI } else { 0.0 };
                lines.arc(corner, Field::CORNER_ARC_RADIUS, from, to);
            }
        }

        let paint = materials.add(StandardMaterial {
            base_color: upkeep.paint(),
            perceptual_roughness: 0.9,
            cull_mode: None,
            ..default()
        });
        commands.spawn((
            Mesh3d(Self::stock(meshes, lines.build())),
            MeshMaterial3d(paint),
        ));
    }

    /// Folds one piece of scenery into a buffer that is accumulating others,
    /// seeding the buffer if it is the first.
    ///
    /// The whole stadium is static: not one vertex of it moves for the length
    /// of a match, and nothing in it is ever culled except the bank the lens
    /// has walked into. So its natural unit is the buffer, not the entity —
    /// and the entity is what this viewer pays for. Measured on a machine
    /// where the scene renders in the same 3.9 ms at 1280x720 and at
    /// 3840x2160, so the pixels are free and the walk is not.
    /// **Registers a piece of the ground and puts its bytes on the bill.**
    ///
    /// Every `meshes.add` in this file goes through here, and the only reason
    /// it exists is that nothing downstream can do the counting. These are
    /// `RenderAssetUsages::RENDER_WORLD` meshes: the moment one has been
    /// extracted its vertex data is dropped from the main world, so a system
    /// walking `Assets<Mesh>` afterwards finds handles with nothing behind
    /// them. See [`MemoryBill`], which carries the whole argument.
    ///
    /// The crowd is NOT stocked through here — it is charged to its own kind
    /// by [`Spectators::seat`], because it is most of the scene's bytes and a
    /// bill that folded it into the concrete would answer nothing.
    pub(crate) fn stock(meshes: &mut Assets<Mesh>, mesh: Mesh) -> Handle<Mesh> {
        MemoryBill::mesh(Held::Ground, &mesh);
        meshes.add(mesh)
    }

    fn gather(buffer: &mut Option<Mesh>, piece: Mesh) {
        match buffer {
            Some(gathered) => gathered
                .merge(&piece)
                .expect("the stadium is built out of cuboids and rectangles"),
            None => *buffer = Some(piece),
        }
    }

    /// The ground the pitch sits in: advertising hoardings around the touchlines
    /// and the low bowl of a stand behind them.
    ///
    /// Without it the turf simply stops and eleven-a-side is played in a void.
    /// All four sides are built, including the one the broadcast gantry hangs
    /// over: a rig that can be walked round the ground would otherwise find a
    /// hole in it. [`Bank`] takes the one in the way back out again.
    #[allow(clippy::too_many_arguments)]
    fn spawn_ground(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        venue: &ViewerConfig,
        throng: Option<Throng>,
        upkeep: Upkeep,
    ) {
        let stature = Stature::of(&venue.venue);

        const HOARDING_HEIGHT: f32 = 0.95;
        const HOARDING_DEPTH: f32 = 0.14;
        // How wide one advert reads on the boards, in metres. Matched to the
        // panel `Textures::hoarding` draws — 1024 texels against 128 for the
        // 0.95 m height — so a texel is square and the lockup is neither
        // stretched nor squashed.
        const AD_PANEL: f32 = 7.6;

        // Advertising hoardings. Lifted out of near-black too, but only to a
        // mid tone: these run the full length of the touchline right at the
        // edge of the play, and a bright band there pulls the eye off the
        // ball in a way the stands behind them do not.
        let board = materials.add(StandardMaterial {
            base_color: Color::srgb(0.200, 0.230, 0.300),
            perceptual_roughness: 0.65,
            ..default()
        });
        // The lit strip along the top of the hoardings. It is the one crisp
        // line in the background, and it is what draws the edge of the playing
        // surface from a camera looking down onto it.
        // Now that the stands behind it are pale, the walkway has to be paler
        // still or it stops reading as lit — at 0.22 it would be the darkest
        // thing on a light structure, which is the opposite of a lit strip.
        //
        // And it is lit by FLOODLIGHTS, which is the one thing about the
        // perimeter that a small ground does not have. So the glow — and only
        // the glow — runs down the same ladder the grass does: the same
        // concrete, the same boards, no lights on them. See [`Upkeep::lit`].
        let lit = upkeep.lit();
        let trim = materials.add(StandardMaterial {
            base_color: Color::srgb(0.86, 0.90, 0.96),
            emissive: LinearRgba::rgb(0.16 * lit, 0.19 * lit, 0.24 * lit),
            perceptual_roughness: 0.4,
            ..default()
        });

        // What the boards actually advertise. One panel, tiled — which is what
        // a ground with a single perimeter sponsor looks like, and the only
        // honest thing to put here: this is the project's own mark, name and
        // address, not an invented brand. Set in the same face as the shirts
        // in front of them; see `Textures::hoarding`.
        let advert = Textures::hoarding(images, "OF", "OpenFootball", "open-football.org");

        let along = Field::HALF_LENGTH + Self::END_MARGIN;
        let across = Field::HALF_WIDTH + Self::SIDE_MARGIN;
        // The perimeter, accumulated rather than spawned: one buffer for the
        // boards, one for the lit strip along their tops, and one per distinct
        // advert repeat.
        let mut boards: Option<Mesh> = None;
        let mut trims: Option<Mesh> = None;
        let mut adverts: Vec<(f32, Mesh)> = Vec::new();
        // `turn` points a panel at the pitch: a `Rectangle` faces +Z, and
        // rotating about Y by `turn` carries that onto `(sin, 0, cos)`.
        for (size, position, length, turn) in [
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, across),
                along * 2.0,
                PI,
            ),
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, -across),
                along * 2.0,
                0.0,
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(along, 0.0, 0.0),
                across * 2.0,
                -FRAC_PI_2,
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(-along, 0.0, 0.0),
                across * 2.0,
                FRAC_PI_2,
            ),
        ] {
            // The four sides go into two buffers rather than eight entities.
            // Same reasoning as the terraces below: a perimeter board is
            // scenery that never moves and is never culled apart from its
            // neighbours, so an entity each buys nothing and costs a walk, an
            // extract and a submit apiece on every frame.
            Self::gather(
                &mut boards,
                Mesh::from(Cuboid::new(size.x, size.y, size.z))
                    .translated_by(position + Vec3::Y * HOARDING_HEIGHT * 0.5),
            );
            Self::gather(
                &mut trims,
                Mesh::from(Cuboid::new(size.x, 0.07, size.z + 0.04))
                    .translated_by(position + Vec3::Y * HOARDING_HEIGHT),
            );

            // The advert itself, as a face hung a few millimetres in front of
            // the board rather than as a texture on the box. The box is a
            // cuboid: texturing it would print the wordmark on the top, the
            // back and both ends as well, and the top is where the lit trim
            // goes.
            //
            // A whole number of panels per side, so the tiling seam lands
            // between two words instead of through the middle of one. The
            // two sides of a ground are different lengths, hence a material
            // each — the repeat count is the only thing that differs.
            //
            // The two long sides carry the same repeat as each other and so do
            // the two ends, so this is two meshes and two materials for four
            // faces rather than four of each.
            let panels = (length / AD_PANEL).round().max(1.0);
            let facing = Quat::from_rotation_y(turn);
            let face = Mesh::from(Rectangle::new(length, HOARDING_HEIGHT)).transformed_by(
                Transform::from_translation(
                    position
                        + Vec3::Y * HOARDING_HEIGHT * 0.5
                        + facing * Vec3::Z * (HOARDING_DEPTH * 0.5 + 0.006),
                )
                .with_rotation(facing),
            );
            match adverts.iter_mut().find(|(repeat, _)| *repeat == panels) {
                Some((_, gathered)) => gathered
                    .merge(&face)
                    .expect("every face is the same rectangle"),
                None => adverts.push((panels, face)),
            }
        }

        for (panels, face) in adverts {
            commands.spawn((
                Mesh3d(Self::stock(meshes, face)),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(advert.clone()),
                    // Perimeter boards at a floodlit ground are lit panels,
                    // and the emissive TEXTURE is what keeps that honest: the
                    // lockup glows and the dark ground behind it does not,
                    // where a flat emissive would have the whole board give
                    // off light like a lightbox.
                    //
                    // Held to roughly the lit trim's level above, and for the
                    // reason the boards are a mid tone rather than a bright
                    // one in the first place: this runs the full length of the
                    // touchline at the very edge of the play, and anything
                    // here that out-glows the one crisp line in the background
                    // is competing with the ball for the eye.
                    //
                    // …and dimmed with the lit trim above it at a ground that
                    // has no floodlights to light it — a village club's boards
                    // are painted, not backlit.
                    emissive: LinearRgba::rgb(0.20 * lit, 0.24 * lit, 0.30 * lit),
                    emissive_texture: Some(advert.clone()),
                    uv_transform: Affine2::from_scale(Vec2::new(panels, 1.0)),
                    perceptual_roughness: 0.55,
                    ..default()
                })),
            ));
        }

        if let Some(mesh) = boards {
            commands.spawn((Mesh3d(Self::stock(meshes, mesh)), MeshMaterial3d(board)));
        }
        if let Some(mesh) = trims {
            commands.spawn((
                Mesh3d(Self::stock(meshes, mesh)),
                MeshMaterial3d(trim.clone()),
            ));
        }

        // Four banks of seating, open to the sky — none of these stands is
        // roofed.
        //
        // These were one tilted slab each — a smooth ramp, which from the
        // camera is a flat grey triangle and reads as scenery rather than as
        // a stand. What makes a stand recognisable at distance is not detail
        // but RELIEF: stepped rows catching the light one edge at a time,
        // now read against the sky rather than against a wall. That is cheap,
        // and with the lens widened it is a good deal more visible than it
        // used to be.
        // The seats themselves, as a texture across the face of every row —
        // see `Textures::seats` for why they cannot be geometry. White base
        // colour so the image supplies the colour unmodified.
        let seating = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(Textures::seats(images)),
            perceptual_roughness: 0.95,
            ..default()
        });

        // …and the people on them, in the colours of BOTH sides: the home
        // club's through the ends and scattered down the touchlines, and the
        // visitors' in the one block of the far end their support was given.
        // One palette and one material for the whole stadium; see
        // [`Spectators`].
        let spectators = Spectators::dressed(
            images,
            materials,
            (
                venue.home.background_color(Color::srgb(0.10, 0.16, 0.34)),
                venue.home.foreground_color(Color::WHITE),
            ),
            (
                venue.away.background_color(Color::srgb(0.70, 0.25, 0.00)),
                venue.away.foreground_color(Color::WHITE),
            ),
        );

        // …and the four banks themselves are NOT built here. They are planned
        // here and raised one per frame — see [`Stands`], which carries the
        // whole argument, and the courses in `lib.rs` that spend the frames.
        commands.insert_resource(Stands {
            pending: Stands::plan(stature),
            seating,
            trim: trim.clone(),
            spectators,
            stature,
            throng,
        });
    }

    /// **One bank of the four**, popped off the plan the last course laid.
    ///
    /// Registered four times over four courses of the bring-up — see
    /// [`Stands`] — and does nothing at all once the plan is empty, which is
    /// what makes a ground with fewer banks than courses harmless.
    pub fn raise_bank(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        stands: Option<ResMut<Stands>>,
    ) {
        let Some(mut stands) = stands else {
            return;
        };
        let Some(plan) = stands.pending.pop() else {
            return;
        };
        let (seating, trim) = (stands.seating.clone(), stands.trim.clone());
        let (stature, throng) = (stands.stature, stands.throng);
        Self::spawn_stand(
            &mut commands,
            &mut meshes,
            &seating,
            &trim,
            &stands.spectators,
            &plan.terrace,
            stature,
            plan.stand,
            plan.turn,
            plan.seed,
            throng,
        );
    }

    /// One bank of seating and the people in it, built in its own local space
    /// — rows receding along +Z and climbing in +Y from the front row at the
    /// origin — and then turned to face the pitch.
    ///
    /// `turn` is the rotation about Y that points the stand inward: zero for
    /// the far side, a quarter turn either way for the ends.
    #[allow(clippy::too_many_arguments)]
    fn spawn_stand(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        seating: &Handle<StandardMaterial>,
        trim: &Handle<StandardMaterial>,
        spectators: &Spectators,
        terrace: &Terrace,
        stature: Stature,
        stand: Stand,
        turn: f32,
        seed: u32,
        throng: Option<Throng>,
    ) {
        /// How far above the back row the cull still counts a camera as being
        /// inside the bank.
        ///
        /// This is NOT the height of anything. The banks are open and their
        /// structure stops at the crest — but a lens above the crest is not
        /// therefore looking over the stand. So the ceiling is a sightline
        /// margin, and it has to stay above the broadcast rest shot, which
        /// sits at `TvCamera::HEIGHT`: 18 m up and 82 m out. Cut it back to
        /// the crest and the near stand reappears across the default view,
        /// which is the one thing [`Bank`] exists to prevent.
        ///
        /// It used to be the height of the back wall, which happened to serve.
        /// With the wall gone the number has to be justified on its own.
        ///
        /// Measured off the CREST, which is the fixture's to decide (see
        /// [`Stature`]) — and that is right rather than merely convenient.
        /// It means the margin does different work at either end of the ladder
        /// and does the right thing at both:
        ///
        /// - **A great ground** is 24 m of stand, well over the gantry on its
        ///   own, so the near bank is hidden with margin to spare. The number
        ///   below is not what carries that case any more; the height is. (It
        ///   was, back when a full bank was 13.4 m and cleared the 18 m shot
        ///   by nearly five metres, which is what this constant was sized for.)
        /// - **A village ground** is a five-step terrace three and a half
        ///   metres high, which does not reach the sightline at all — so the
        ///   near bank correctly stays in frame. There is nothing there to be
        ///   a wall, and you can see clean over it.
        ///
        /// ⚠ That second case is what keeps the crowd's rear faces alive; see
        /// [`Figures::FACES`](crate::scene::crowd).
        const SIGHTLINE_CLEARANCE: f32 = 7.3;

        // Every row of this bank, in ONE buffer.
        //
        // They used to be an entity each — twenty-one to a touchline bank,
        // nineteen to an end, eighty-four across the ground — sharing a mesh
        // and a material and differing only in where they sat. That is exactly
        // the shape of thing this viewer cannot afford: the frame is spent
        // per-entity, not per-pixel (the scene renders in the same 3.9 ms at
        // 1280x720 and at 3840x2160), so eighty-four rows cost eighty-four
        // times the walk, the extract and the submit for one flight of steps.
        //
        // Nothing is lost by merging. A stand is stepped concrete that never
        // moves, and it is culled as a unit anyway — `Bank::cull` hides the
        // whole bank or none of it, and no row was ever in shot without the
        // rows either side of it.
        let step = || {
            Mesh::from(Cuboid::new(
                terrace.length,
                terrace.riser * terrace.slab,
                terrace.tread * 0.96,
            ))
        };
        let mut flight = step();
        for row in 1..terrace.rows {
            let offset = terrace.slab_centre(row) - terrace.slab_centre(0);
            flight
                .merge(&step().translated_by(offset))
                .expect("every row is the same cuboid");
        }
        let flight = Self::stock(meshes, flight);
        let foot = terrace.slab_centre(0);

        // The bank's own extent, so `Bank::cull` can tell whether the
        // lens has walked into this one. A metre of slack either side of the
        // seating, so a camera just off the end of a bank still counts as
        // being behind it.
        let bank_extent = Bank {
            // World → local is the inverse of the placement turn.
            frame: Quat::from_rotation_y(-turn),
            flank: terrace.length * 0.5 + 1.0,
            near: terrace.from,
            top: terrace.crest() + SIGHTLINE_CLEARANCE,
        };

        let placement = Transform::from_rotation(Quat::from_rotation_y(turn));
        let anchor = commands
            .spawn((placement, Visibility::default(), bank_extent))
            .id();

        commands.entity(anchor).with_children(|bank| {
            // The merged flight, placed where its first row used to sit — the
            // rest are built off that one inside the mesh.
            bank.spawn((
                Mesh3d(flight.clone()),
                MeshMaterial3d(seating.clone()),
                Transform::from_translation(foot),
            ));

            // The crowd, as one more mesh on the same steps. A child of the
            // bank rather than a thing of its own, so `Bank::cull` takes the
            // people out with the structure they are sitting on.
            //
            // Handhelds and `?crowd=off` omit spectator meshes while retaining
            // the stadium structure. See [`Throng::of`].
            if let Some(crowd) = throng
                .and_then(|throng| spectators.seat(meshes, terrace, stature, stand, seed, throng))
            {
                bank.spawn(crowd);
            }

            // The walkway that splits the tiers. Deliberately not along the
            // crest — the camera looks UP at these from below their top, so a
            // line drawn on the crest is never in shot. A third of the way up
            // is, and it is the one thing that stops the background reading as
            // a flat wall.
            //
            // Only where there are two tiers to split: on a five-step terrace
            // it lands on the second step, which is not a tier break but a
            // stripe painted across a low wall — and reads as one.
            if Stature::tiered(terrace.rows) {
                let tier = terrace.rows as f32 * Stature::TIER;
                bank.spawn((
                    Mesh3d(Self::stock(
                        meshes,
                        Cuboid::new(terrace.length, 0.5, 1.1).into(),
                    )),
                    MeshMaterial3d(trim.clone()),
                    Transform::from_xyz(
                        0.0,
                        terrace.riser * tier,
                        terrace.from + terrace.tread * tier,
                    ),
                ));
            }

            // No back wall. There used to be one — a slab of pale concrete
            // standing seven metres above the back row — on the reasoning that
            // a bank open to the sky reads as a ramp with nothing behind it.
            // In practice it read as what it was: a flat grey rectangle
            // filling the top of the frame on every side of the ground, and it
            // took more attention than the seating in front of it. The rows
            // now finish against the sky, which is where the eye expects the
            // structure of an open ground to stop.
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the vertex colours back off a built sward.
    fn colours(mesh: &Mesh) -> Vec<Vec3> {
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(values)) => values
                .iter()
                .map(|colour| Vec3::new(colour[0], colour[1], colour[2]))
                .collect(),
            _ => panic!("the sward carries no vertex colours"),
        }
    }

    fn turned() -> Vec3 {
        let mown = LinearRgba::from(Pitch::MOWN);
        let against = LinearRgba::from(Pitch::AGAINST);
        Vec3::new(
            against.red / mown.red,
            against.green / mown.green,
            against.blue / mown.blue,
        )
    }

    /// A rendered colour's hue in degrees and saturation 0..1, from an sRGB
    /// albedo. Both are read off the CONSTANT here rather than off a frame,
    /// which is fine for the two things the ladder tests ask of them — that
    /// the walk goes the right way and that it goes there evenly. Where it
    /// actually lands on screen is a question for a rendered frame and the
    /// note on [`Upkeep::TIRED`] says so.
    fn hue_and_saturation(colour: Color) -> (f32, f32) {
        let rgb = Srgba::from(colour);
        let (red, green, blue) = (rgb.red, rgb.green, rgb.blue);
        let high = red.max(green).max(blue);
        let low = red.min(green).min(blue);
        let span = high - low;
        let hue = if span <= f32::EPSILON {
            0.0
        } else if high == red {
            60.0 * (((green - blue) / span) % 6.0)
        } else if high == green {
            60.0 * ((blue - red) / span + 2.0)
        } else {
            60.0 * ((red - green) / span + 4.0)
        };
        (hue, if high <= 0.0 { 0.0 } else { span / high })
    }

    /// The contract the whole field rests on, and the same one
    /// [`Textures::turf`] holds for the sheet: wear and unevenness say where
    /// the grass is different, NOT what colour the grass is.
    ///
    /// `Pitch::MOWN` is calibrated against broadcast footage and the note on
    /// it records what a third off the albedo did to the picture. Anything in
    /// here that moved the average would be re-grading the pitch by the back
    /// door — and would do it silently, because the pitch would still look
    /// like a pitch.
    #[test]
    fn wearing_the_grass_does_not_repaint_it() {
        // Both ends of the ladder, because the poor one is where this could
        // now go wrong: it wears two and a half times as hard and its sward is
        // twice as rough, and if either leaked into the average then the way
        // to a browner pitch would be to neglect it TWICE — once through the
        // colour, which is deliberate, and once by the back door, which is
        // exactly what this forbids.
        for kept in [1.0, 0.5, 0.0] {
            let upkeep = Upkeep::at(kept);
            let turned = upkeep.mow();
            let mesh = Sward::mow(upkeep, Pitch::TURF_TILE);
            let colours = colours(&mesh);

            let mut worn = Vec3::ZERO;
            for colour in &colours {
                worn += *colour;
            }
            worn /= colours.len() as f32;

            // What the same mow comes to with a perfectly even sward under it:
            // every vertex is either the sheet as drawn or the sheet turned,
            // and the bands are equal in area, so the average is decided by
            // the stripe count alone.
            let bands = Pitch::STRIPES as f32;
            let against = (Pitch::STRIPES / 2) as f32;
            let flat = (Vec3::ONE * (bands - against) + turned * against) / bands;

            for channel in 0..3 {
                assert!(
                    (worn[channel] - flat[channel]).abs() < 0.004,
                    "at {kept} kept, channel {channel} averaged {} against a \
                     flat sward's {}",
                    worn[channel],
                    flat[channel]
                );
            }
        }
    }

    /// **The pitch that took an evening and five rejected greens to place is
    /// still exactly that pitch.**
    ///
    /// Everything [`Upkeep`] does is written as a distance travelled from
    /// `Pitch::MOWN` and its pair, so a great ground has to come out bit for
    /// bit where it came out before the ladder existed. This is the test that
    /// says so, and it is the whole licence for grading a calibrated colour at
    /// all: get it wrong and the top of the game's grounds quietly drift off a
    /// number that was set by eye against rendered frames and cannot be
    /// recovered from the source.
    #[test]
    fn a_great_ground_is_the_pitch_that_was_calibrated() {
        let great = Upkeep::at(1.0);

        let sward = Srgba::from(great.sward());
        let mown = Srgba::from(Pitch::MOWN);
        assert_eq!(
            (sward.red, sward.green, sward.blue),
            (mown.red, mown.green, mown.blue),
            "a great ground is drawn in a green that is not Pitch::MOWN"
        );

        // …and the stripe is the one derived from the calibrated pair, to the
        // precision two routes through sRGB can agree to.
        let mow = great.mow();
        let turned = turned();
        for channel in 0..3 {
            assert!(
                (mow[channel] - turned[channel]).abs() < 1e-5,
                "channel {channel} mows at {} against the calibrated {}",
                mow[channel],
                turned[channel]
            );
        }

        assert_eq!(great.worn(), 1.0, "a great ground wears no harder");
        assert_eq!(great.rough(), 1.0, "a great ground is no rougher");
        assert_eq!(great.lit(), 1.0, "a great ground keeps its floodlights");
    }

    /// …and the other end of the same ladder is a visibly worse pitch, in
    /// every one of the four ways it is supposed to be worse.
    ///
    /// Four assertions rather than one because any single one of them alone
    /// reads as a bug rather than as a poor ground: a duller green with crisp
    /// stripes still on it is a colour mistake, and full stripes over bare
    /// earth is a texture mistake. It is the four together that read as a
    /// pitch nobody looks after.
    #[test]
    fn a_park_pitch_is_a_poorer_pitch() {
        let (great, park) = (Upkeep::at(1.0), Upkeep::at(0.0));

        // 1. The green. Paler, yellower and much less saturated — and NOT
        //    darker, which is the way it would be easy to take it and the way
        //    dying grass does not go.
        let (top_hue, top_saturation) = hue_and_saturation(great.sward());
        let (low_hue, low_saturation) = hue_and_saturation(park.sward());
        assert!(
            low_saturation < top_saturation * 0.75,
            "a park pitch is barely less saturated: {low_saturation} against {top_saturation}"
        );
        assert!(
            low_hue < top_hue - 15.0,
            "a park pitch has not turned toward yellow: {low_hue}° against {top_hue}°"
        );
        let value = |colour: Color| Srgba::from(colour).green;
        assert!(
            value(park.sward()) >= value(great.sward()),
            "a park pitch has been made darker rather than drier"
        );

        // 2. The mow. Gone entirely — a rotary mower leaves no stripe, and a
        //    multiplier of one is a band you cannot see.
        let mow = park.mow();
        for channel in 0..3 {
            assert!(
                (mow[channel] - 1.0).abs() < 1e-5,
                "a park pitch is still striped on channel {channel}: {}",
                mow[channel]
            );
        }

        // 3. The wear, and 4. the sward. Both harder than a great ground's.
        assert!(
            park.worn() > great.worn() * 2.0,
            "a park pitch's goalmouth wears no harder: {}",
            park.worn()
        );
        assert!(
            park.rough() > great.rough() * 1.5,
            "a park pitch's sward is no rougher: {}",
            park.rough()
        );

        // And the paint has stopped being white.
        assert!(
            Srgba::from(park.paint()).red < Srgba::from(great.paint()).red - 0.1,
            "a park pitch is marked in the same fresh white"
        );
    }

    /// The ladder is a ladder: every rung between the two ends is between the
    /// two ends, and the walk never doubles back.
    ///
    /// Worth pinning because the green is interpolated in sRGB and the mow is
    /// derived through two conversions on top of that — plenty of room for a
    /// mid-table club to come out greener than a great one, and nothing on
    /// screen would say so except that ground looking odd once.
    #[test]
    fn the_ladder_runs_one_way() {
        let mut previous: Option<(f32, f32, f32)> = None;
        for step in 0..=10 {
            let upkeep = Upkeep::at(step as f32 / 10.0);
            let (_, saturation) = hue_and_saturation(upkeep.sward());
            let stripe = 1.0 - upkeep.mow().y;
            let here = (saturation, stripe, upkeep.worn());

            if let Some(before) = previous {
                assert!(
                    here.0 >= before.0 - 1e-6,
                    "green went backwards at step {step}: {here:?} after {before:?}"
                );
                assert!(
                    here.1 >= before.1 - 1e-6,
                    "the mow went backwards at step {step}: {here:?} after {before:?}"
                );
                assert!(
                    here.2 <= before.2 + 1e-6,
                    "the wear went backwards at step {step}: {here:?} after {before:?}"
                );
            }
            previous = Some(here);
        }
    }

    /// …and having established that it does not move the average, that it
    /// does actually do something. A field that normalised itself to nothing
    /// would pass the test above perfectly.
    #[test]
    fn a_played_pitch_is_worn_where_it_is_played_on() {
        let goalmouth = Sward::wear(Vec2::new(Field::HALF_LENGTH - 4.0, 0.0));
        let spot = Sward::wear(Vec2::new(
            Field::HALF_LENGTH - Field::PENALTY_SPOT_DISTANCE,
            0.0,
        ));
        let centre = Sward::wear(Vec2::ZERO);
        // A corner is the one part of a pitch nobody spends a match on.
        let corner = Sward::wear(Vec2::new(Field::HALF_LENGTH - 1.0, Field::HALF_WIDTH - 1.0));

        assert!(goalmouth > 0.9, "the goalmouth goes bare: {goalmouth}");
        assert!(spot > 0.2, "the penalty spot is stood on: {spot}");
        assert!(centre > 0.2, "every kickoff is here: {centre}");
        assert!(corner < 0.05, "nothing happens in the corner: {corner}");
    }

    #[test]
    fn grass_sampling_breaks_the_tile_repeat_without_folding() {
        let tile = Pitch::TURF_TILE;
        let mut displacement = 0.0;
        let mut count = 0;
        for line in 0..2 {
            let pass = Pass::mown(line, turned(), 1.0);
            for x in -52..53 {
                for z in -34..35 {
                    let point = Vec2::new(x as f32, z as f32);
                    let uv = Sward::grass_uv(point, pass, tile);
                    let dx = Sward::grass_uv(point + Vec2::X * 0.1, pass, tile) - uv;
                    let dz = Sward::grass_uv(point + Vec2::Y * 0.1, pass, tile) - uv;
                    assert!(dx.perp_dot(dz) * pass.facing > 0.0, "folded UVs at {point}");
                    let repeat = Sward::grass_uv(point + Vec2::X * tile, pass, tile) - uv;
                    displacement += (repeat - Vec2::X).length();
                    count += 1;
                }
            }
        }
        assert!(
            displacement / count as f32 > 0.05,
            "the tile still repeats in place"
        );
    }

    /// **A stripe is the same grass turned round, not a second green.**
    ///
    /// The tint is the part that was always here and the part that, on its
    /// own, is a band painted onto a carpet: baked into a vertex, it is the
    /// darker of the two from every seat in the ground. The other two halves
    /// of [`Pass`] are what make it grass — a mirrored sheet and a mirrored
    /// tangent frame, so the renderer works the contrast out per pixel off the
    /// relief and the mow answers to where the camera and the light are.
    ///
    /// Held here because both are invisible in the source of the mesh: they
    /// are a sign on one float of a vertex attribute, and getting either
    /// backwards is a pitch that still looks like a pitch.
    #[test]
    fn a_stripe_is_the_same_grass_turned_round() {
        // The pair itself, band by band: a stripe alternates, and both halves
        // of it alternate together.
        let rolled = Pass::rolled();
        let mut stripes = Vec3::ZERO;
        let mut passes = 0.0;
        for line in 0..Pitch::STRIPES {
            let pass = Pass::mown(line, turned(), rolled[line]);
            let back = line % 2 == 1;
            assert_eq!(
                pass.facing,
                if back { -1.0 } else { 1.0 },
                "band {line} is lying the wrong way"
            );
            if !back {
                assert_eq!(
                    pass.tint,
                    Vec3::ONE,
                    "band {line} is not the sheet as drawn"
                );
                continue;
            }
            // Deeper or shallower than the mean, but always a stripe and
            // always in the same direction — a pass rolled the wrong way would
            // be a band LIGHTER than the grass it was cut from.
            for channel in 0..3 {
                assert!(
                    pass.tint[channel] < 1.0,
                    "band {line} is not a stripe on channel {channel}"
                );
            }
            stripes += pass.tint;
            passes += 1.0;
        }

        // …and however deep each one came out, the pair averages to the one
        // that was calibrated. This is the same contract
        // `wearing_the_grass_does_not_repaint_it` holds the mesh to, stated
        // where it is decided rather than where it shows up.
        let mean = stripes / passes;
        for channel in 0..3 {
            assert!(
                (mean[channel] - turned()[channel]).abs() < 1e-5,
                "the mow averages {} on channel {channel} against the calibrated {}",
                mean[channel],
                turned()[channel]
            );
        }

        // …and no two bands start at the same place in the sheet, which is
        // what stops the two-metre tile printing down the pitch in lockstep.
        for line in 0..Pitch::STRIPES {
            for other in (line + 1)..Pitch::STRIPES {
                let apart = Pass::mown(line, Vec3::ONE, 1.0).phase
                    - Pass::mown(other, Vec3::ONE, 1.0).phase;
                assert!(
                    apart.length() > 0.02,
                    "bands {line} and {other} start on the same texel"
                );
            }
        }

        let mesh = Sward::mow(Upkeep::at(1.0), Pitch::TURF_TILE);
        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) => values.clone(),
            _ => panic!("the sward carries no positions"),
        };
        let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(values)) => values.clone(),
            _ => panic!("the sward carries no uvs"),
        };
        let tangents = match mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(values)) => values.clone(),
            _ => panic!("the sward carries no tangents"),
        };

        // Then on the mesh: every step along the mow has to move through the
        // sheet the way that vertex's own tangent frame says it does. Read off
        // neighbours rather than off a band index, because the wander means a
        // vertex is no longer where the arithmetic would put it — and a step
        // backwards across the pitch is the next band starting, so there is no
        // pair here that straddles a mow line.
        let mut lying = [0usize; 2];
        for index in 1..positions.len() {
            let reach = positions[index][2] - positions[index - 1][2];
            if reach <= 0.0 {
                continue;
            }
            let facing = -tangents[index][3];
            let slope = uvs[index][1] - uvs[index - 1][1];
            assert_eq!(
                slope.signum(),
                facing,
                "the sheet is not mirrored with the tangent frame at x={}",
                positions[index][0]
            );
            lying[usize::from(facing < 0.0)] += 1;
        }

        // And the ground is mown both ways in equal measure — an odd band
        // count puts one more pass one way than the other, and nothing more.
        let apart = lying[0].abs_diff(lying[1]) as f32 / lying[0].max(lying[1]) as f32;
        assert!(
            apart < 2.0 / Pitch::STRIPES as f32,
            "the mow is lopsided: {lying:?}"
        );
    }

    /// **A mow line is not a ruled line**, and the touchlines are.
    ///
    /// Bands of exactly `LENGTH / 15`, spaced to the millimetre and ruled
    /// straight from one touchline to the other, is the geometry of printed
    /// carpet — and it is legible as one from the gantry long before any
    /// single stripe is, because regularity survives distance better than
    /// detail does. What must NOT move with it is the pitch itself: the paint,
    /// the goals and the run-off are all laid on a rectangle, and a touchline
    /// that wandered would be a pitch that no longer met its own markings.
    #[test]
    fn a_mow_line_is_not_a_ruled_line() {
        let mut worst = 0.0f32;
        let mut mean = 0.0f32;
        let mut count = 0.0f32;
        for line in 0..=Pitch::STRIPES {
            let mut z = -Field::HALF_WIDTH;
            while z <= Field::HALF_WIDTH {
                let drift = Sward::drift(line, z);
                if line == 0 || line == Pitch::STRIPES {
                    assert_eq!(drift, 0.0, "the goal line at {line} has wandered off");
                } else {
                    worst = worst.max(drift.abs());
                    mean += drift;
                    count += 1.0;
                }
                z += 0.5;
            }
        }

        // Plainly off true — a tenth of a metre is the width of the paint, and
        // anything under it is a line nobody would call crooked.
        assert!(worst > 0.10, "every mow line is still ruled: {worst}");
        // …and never so far off that a band visibly narrows. A stripe is seven
        // metres; a twentieth of one is a hand on a mower.
        let stripe = Field::LENGTH / Pitch::STRIPES as f32;
        assert!(
            worst < stripe / 20.0,
            "the mow wanders by {worst} of a {stripe} m band"
        );
        // And it goes both ways. A wander with a mean would be a shear — the
        // whole mow leaning one way down the pitch, which is a pattern again.
        assert!(
            (mean / count).abs() < worst * 0.25,
            "the mow leans one way: {} against a worst of {worst}",
            mean / count
        );
    }

    /// **The whole surface from overhead**, as near to what the GPU lays down
    /// as a test can get: the blade sheet sampled through each vertex's own
    /// UVs, lit through the relief with the same tangent frame and the same
    /// sun, and multiplied by its own colour.
    ///
    /// The mow is the one thing on this pitch that cannot be judged from a
    /// texture dump. [`Textures::turf`] draws one tile, and every complaint
    /// that ever got the word "carpet" attached to this surface was about how
    /// that tile is LAID — the ruler-straight bands, the one grain running
    /// under all of them, the two-metre lattice printed fifty times down the
    /// ground in lockstep. None of that is in the tile and all of it is here.
    ///
    /// ```text
    /// MATCH_PITCH_DUMP=<dir> cargo test --release --lib dump_mow -- --ignored --nocapture
    /// ```
    ///
    /// Writes `pitch.rgba`, the whole playing surface, where the wander and
    /// the wear are what there is to look at; and `mow.rgba`, a window a few
    /// metres across straddling one line at one texel to the pixel, which is
    /// the only scale the grain reversal is visible at.
    ///
    /// Both are sampled off the level of the mip chain where a texel is about
    /// a pixel. Anything coarser is the chain resolving to the mean and
    /// anything finer is aliasing, and either one answers a question about the
    /// sampler rather than about the mow.
    #[test]
    #[ignore = "writes files; run by hand when the mow changes"]
    fn dump_mow() {
        let Ok(directory) = std::env::var("MATCH_PITCH_DUMP") else {
            panic!("set MATCH_PITCH_DUMP to a directory");
        };
        let directory = std::path::Path::new(&directory);

        let mut images = Assets::<Image>::default();
        let upkeep = Upkeep::at(1.0);
        let grass = Textures::turf(&mut images, upkeep.sward());
        let mesh = Sward::mow(upkeep, Pitch::TURF_TILE);

        for (name, from, to, width, level) in [
            (
                "pitch",
                Vec2::new(-Field::HALF_LENGTH, -Field::HALF_WIDTH),
                Vec2::new(Field::HALF_LENGTH, Field::HALF_WIDTH),
                1680u32,
                5u32,
            ),
            (
                "mow",
                Vec2::new(-47.2, -1.2),
                Vec2::new(-43.6, 1.2),
                1680,
                0,
            ),
        ] {
            let (across, down, pixels) = overhead(&mesh, &images, &grass, from, to, width, level);
            std::fs::write(directory.join(format!("{name}.rgba")), &pixels).expect("wrote it");
            println!(
                "{across}x{down} at {}",
                directory.join(format!("{name}.rgba")).display()
            );
        }
    }

    /// One texel of a packed mip chain, wrapped.
    fn texel(image: &Image, level: u32, uv: Vec2) -> Vec3 {
        let mut size = image.texture_descriptor.size.width;
        let mut offset = 0usize;
        for _ in 0..level {
            offset += (size * size * 4) as usize;
            size = (size / 2).max(1);
        }
        let data = image.data.as_ref().expect("pixels");
        let wrap =
            |value: f32| (((value * size as f32).floor() as i64).rem_euclid(size as i64)) as u32;
        let at = offset + ((wrap(uv.y) * size + wrap(uv.x)) * 4) as usize;
        Vec3::new(
            data[at] as f32 / 255.0,
            data[at + 1] as f32 / 255.0,
            data[at + 2] as f32 / 255.0,
        )
    }

    /// The sward seen from straight up, rasterised a triangle at a time.
    ///
    /// Not a renderer and not trying to be one — no tonemapper, no ambient
    /// from the camera, no perspective. What it does carry is everything the
    /// mow is made of: the sheet through the mesh's own UVs, the relief
    /// through the mesh's own tangent frame, [`Pitch::SUN`], and the vertex
    /// colours. Which is enough to see whether a stripe is a lit thing or a
    /// painted one.
    fn overhead(
        mesh: &Mesh,
        images: &Assets<Image>,
        grass: &Turf,
        from: Vec2,
        to: Vec2,
        width: u32,
        level: u32,
    ) -> (u32, u32, Vec<u8>) {
        let span = to - from;
        let height = (width as f32 * span.y / span.x).round() as u32;
        let albedo = images.get(&grass.albedo).expect("just built");
        let relief = images.get(&grass.relief).expect("just built");
        let sun = -Pitch::SUN.normalize();

        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) => values,
            _ => panic!("no positions"),
        };
        let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(bevy::mesh::VertexAttributeValues::Float32x2(values)) => values,
            _ => panic!("no uvs"),
        };
        let tangents = match mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(values)) => values,
            _ => panic!("no tangents"),
        };
        let tints = colours(mesh);
        let indices: Vec<u32> = match mesh.indices() {
            Some(Indices::U32(values)) => values.clone(),
            _ => panic!("no indices"),
        };

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let to_pixel = |point: Vec2| {
            Vec2::new(
                (point.x - from.x) / span.x * width as f32,
                (point.y - from.y) / span.y * height as f32,
            )
        };

        for triangle in indices.chunks_exact(3) {
            let corner: Vec<usize> = triangle.iter().map(|index| *index as usize).collect();
            let plan: Vec<Vec2> = corner
                .iter()
                .map(|index| to_pixel(Vec2::new(positions[*index][0], positions[*index][2])))
                .collect();
            let low = plan[0].min(plan[1]).min(plan[2]);
            let high = plan[0].max(plan[1]).max(plan[2]);
            if high.x < 0.0 || high.y < 0.0 || low.x > width as f32 || low.y > height as f32 {
                continue;
            }
            let edge = |a: Vec2, b: Vec2, point: Vec2| {
                (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
            };
            let area = edge(plan[0], plan[1], plan[2]);
            if area.abs() < 1e-6 {
                continue;
            }

            for row in (low.y.floor().max(0.0) as u32)..=(high.y.ceil() as u32).min(height - 1) {
                for column in
                    (low.x.floor().max(0.0) as u32)..=(high.x.ceil() as u32).min(width - 1)
                {
                    let point = Vec2::new(column as f32 + 0.5, row as f32 + 0.5);
                    let weight = Vec3::new(
                        edge(plan[1], plan[2], point) / area,
                        edge(plan[2], plan[0], point) / area,
                        edge(plan[0], plan[1], point) / area,
                    );
                    if weight.min_element() < 0.0 {
                        continue;
                    }

                    let mut uv = Vec2::ZERO;
                    let mut tint = Vec3::ZERO;
                    let mut tangent = Vec3::ZERO;
                    let mut handed = 0.0f32;
                    for (slot, index) in corner.iter().enumerate() {
                        uv += Vec2::new(uvs[*index][0], uvs[*index][1]) * weight[slot];
                        tint += tints[*index] * weight[slot];
                        tangent += Vec3::from_slice(&tangents[*index][..3]) * weight[slot];
                        handed += tangents[*index][3] * weight[slot];
                    }

                    // Use the generated frame, including the bend in the UV grid.
                    let packed = texel(relief, level, uv) * 2.0 - Vec3::ONE;
                    let tangent = tangent.normalize_or(Vec3::X);
                    let bitangent = Vec3::Y.cross(tangent) * handed.signum();
                    let normal = (tangent * packed.x + bitangent * packed.y + Vec3::Y * packed.z)
                        .normalize_or(Vec3::Y);
                    let lit = 0.55 + 0.45 * normal.dot(sun).max(0.0);

                    let sheet = texel(albedo, level, uv);
                    // sRGB in, sRGB out, with the tint and the light applied
                    // in between where the shader applies them.
                    let linear = Vec3::new(sheet.x.powf(2.2), sheet.y.powf(2.2), sheet.z.powf(2.2))
                        * tint
                        * lit;
                    let at = ((row * width + column) * 4) as usize;
                    for channel in 0..3 {
                        pixels[at + channel] =
                            (linear[channel].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
                    }
                    pixels[at + 3] = 255;
                }
            }
        }
        (width, height, pixels)
    }

    /// **At a ground that is mown, the mow is the loudest thing on the
    /// surface.** Unevenness that approached the stripe's own contrast would
    /// stop reading as ground and start reading as a second, wrong set of
    /// stripes.
    ///
    /// Stated at a GREAT ground, which is where it is a claim about anything:
    /// further down the ladder there is progressively less mow to shout over
    /// and eventually none at all, and the patchiness left behind is then the
    /// whole picture — which is the point of it and not a violation of this.
    /// What that end owes instead is a ceiling of its own, which is the second
    /// half below.
    #[test]
    fn the_sward_never_shouts_over_the_mow() {
        let stripe = 1.0 - turned().y;
        let mut worst = 0.0f32;
        // Every half metre of the playing surface, which is where the field is
        // sampled in earnest.
        let mut x = -Field::HALF_LENGTH;
        while x <= Field::HALF_LENGTH {
            let mut z = -Field::HALF_WIDTH;
            while z <= Field::HALF_WIDTH {
                worst = worst.max(Sward::unevenness(Vec2::new(x, z)).abs());
                z += 0.5;
            }
            x += 0.5;
        }
        assert!(
            worst * Upkeep::at(1.0).rough() < stripe * 0.5,
            "unevenness reached {worst} against a mow of {stripe}"
        );

        // …and a park pitch, which has no mow left, is held between two bars
        // instead. It has to be plainly rougher than a kept one — half a
        // stripe's worth at least, or the whole bottom of the ladder is a
        // colour change with nothing under it — and it must never reach a
        // whole stripe, which is where the eye stops reading ground and starts
        // reading camouflage.
        let patchy = worst * Upkeep::at(0.0).rough();
        assert!(
            patchy < stripe,
            "a park pitch is blotchier than a great one is striped: \
             {patchy} against {stripe}"
        );
        assert!(
            patchy > stripe * 0.5,
            "a park pitch is barely rougher than a kept one: {patchy} against {stripe}"
        );
    }
}
