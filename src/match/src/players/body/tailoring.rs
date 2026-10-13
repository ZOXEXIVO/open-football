//! A footballer cut as finely as the screen has room for: each grain this
//! machine draws is cut once and shared by the squad, and each man wears the
//! one his height on the screen calls for.

use super::*;
use crate::app::bill::{Held, MemoryBill};
use crate::app::perf::FrameCost;
use crate::players::kit::Outfit;

/// **The squad at every grain this machine draws**, finest first, each cut
/// once and shared by all twenty-two.
///
/// The finest is cut before anybody is dressed and the coarser ones on a
/// bring-up course each ([`Tailoring::cut_next`]): the full cut takes the
/// best part of half a second, and a man walking to the camera cannot wait
/// for it.
#[derive(Resource)]
pub struct Tailoring {
    /// Each grain cut so far, its parts, and an outfielder's triangles there.
    cut: Vec<(Grain, BodyParts, usize)>,
}

/// **How finely a man is cut right now**, on his root.
#[derive(Component)]
pub struct Detail(Grain);

/// **Which of [`BodyParts`]'s meshes this piece of a footballer wears**, and
/// whose footballer it is, so he can be re-dressed at another grain without
/// being rebuilt.
#[derive(Component, Clone, Copy)]
pub struct Worn {
    actor: Entity,
    part: Part,
}

/// One of [`BodyParts`]'s meshes, by name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Torso,
    TorsoLong,
    Pelvis,
    Head,
    /// By [`HairStyle`](crate::players::kit::HairStyle) index.
    Hair(usize),
    UpperArm,
    Forearm,
    ForearmSleeved,
    /// `[0]` left.
    Hand(usize),
    Glove,
    Finger,
    Fingertip,
    Thumb,
    Thumbtip,
    Thigh,
    Shin,
    Boot,
    Number,
    Name,
    NameFront,
}

impl Worn {
    pub fn on(actor: Entity, part: Part) -> Worn {
        Worn { actor, part }
    }
}

impl BodyParts {
    /// The mesh this cut makes of `part`.
    fn worn(&self, part: Part) -> &Handle<Mesh> {
        match part {
            Part::Torso => &self.torso,
            Part::TorsoLong => &self.torso_long,
            Part::Pelvis => &self.pelvis,
            Part::Head => &self.head,
            Part::Hair(style) => self.hair[style]
                .as_ref()
                .expect("every cut has the same hair styles"),
            Part::UpperArm => &self.upper_arm,
            Part::Forearm => &self.forearm,
            Part::ForearmSleeved => &self.forearm_sleeved,
            Part::Hand(side) => &self.hand[side],
            Part::Glove => &self.glove,
            Part::Finger => &self.finger,
            Part::Fingertip => &self.fingertip,
            Part::Thumb => &self.thumb,
            Part::Thumbtip => &self.thumbtip,
            Part::Thigh => &self.thigh,
            Part::Shin => &self.shin,
            Part::Boot => &self.boot,
            Part::Number => &self.number,
            Part::Name => &self.name,
            Part::NameFront => &self.name_front,
        }
    }
}

impl Tailoring {
    /// The squad cut at `finest`, the finest grain this machine draws, and at
    /// nothing else yet.
    pub fn new(
        meshes: &mut Assets<Mesh>,
        bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
        finest: Grain,
    ) -> Tailoring {
        let mut parts = BodyParts::new(meshes, finest);
        parts.bind_clothes(bindposes);
        let mut tailoring = Tailoring { cut: Vec::new() };
        tailoring.keep(meshes, finest, parts);
        tailoring
    }

    /// Counted while the vertices are still in the main world: the parts are
    /// `RENDER_WORLD`-only, so once extracted nothing can count them again.
    fn keep(&mut self, meshes: &Assets<Mesh>, grain: Grain, parts: BodyParts) {
        MemoryBill::note(Held::Squad, parts.bytes(meshes));
        let triangles = parts.triangles(meshes);
        self.cut.push((grain, parts, triangles));
    }

    /// The finest grain this machine draws.
    pub fn grain(&self) -> Grain {
        self.cut[0].0
    }

    /// The parts every man is assembled from: the finest cut. Coarser ones
    /// are worn by swapping meshes, never by building him again.
    pub fn finest(&self) -> &BodyParts {
        &self.cut[0].1
    }

    /// **Builds a man's body** at the finest cut; [`Self::fit`] cuts him
    /// coarser on the next frame if the camera is far from him.
    pub fn dress(&self, commands: &mut Commands, root: Entity, outfit: &Outfit, keeper: bool) {
        Footballer::assemble(commands, root, self.finest(), outfit, keeper);
        commands.entity(root).insert(Detail(self.grain()));
    }

    /// What an outfielder comes to at each grain cut so far, for the frame
    /// cost line.
    pub fn describe(&self) -> String {
        self.cut
            .iter()
            .map(|(grain, _, triangles)| format!("{triangles} tri/player ({})", grain.describe()))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The cut nearest `wanted` that has been made: it, or the coarsest one
    /// finer than it while the bring-up has still to cut it.
    fn nearest(&self, wanted: Grain) -> (Grain, &BodyParts) {
        let index = (wanted.rank() - self.grain().rank()).min(self.cut.len() - 1);
        let (grain, parts, _) = &self.cut[index];
        (*grain, parts)
    }

    /// **Cuts the next coarser grain**, on its own bring-up course.
    pub fn cut_next(
        mut tailoring: ResMut<Tailoring>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut cost: ResMut<FrameCost>,
    ) {
        let Some(&grain) = tailoring.grain().and_coarser().get(tailoring.cut.len()) else {
            return;
        };
        let parts = BodyParts::new(&mut meshes, grain);
        tailoring.keep(&meshes, grain, parts);
        cost.note_geometry(tailoring.describe());
    }

    /// **Cuts each man on the pitch as finely as his height on the screen
    /// calls for** ([`Grain::at_height`]), and re-dresses only the men whose
    /// grain changed. His height is read off the lens the frame is drawn
    /// through, at the target's own pixels, so a zoom, a follow and a smaller
    /// rung of the stage ladder all count.
    pub fn fit(
        tailoring: Res<Tailoring>,
        camera: Single<(&Camera, &Projection, &Transform), With<Camera3d>>,
        mut men: Query<(Entity, &Transform, &Visibility, &mut Detail)>,
        mut worn: Query<(&Worn, &mut Mesh3d)>,
        mut cost: ResMut<FrameCost>,
        mut changed: Local<Vec<(Entity, Grain)>>,
    ) {
        let (camera, projection, lens) = *camera;
        let (Projection::Perspective(perspective), Some(target)) =
            (projection, camera.physical_viewport_size())
        else {
            return;
        };
        changed.clear();
        let mut cut = [0; Grain::ALL.len()];
        for (actor, place, visibility, mut detail) in &mut men {
            if *visibility == Visibility::Hidden {
                continue;
            }
            let pixels = Self::height_on_screen(lens, perspective.fov, target.y, place);
            let (grain, _) = tailoring.nearest(detail.0.at_height(pixels, tailoring.grain()));
            if grain != detail.0 {
                detail.0 = grain;
                changed.push((actor, grain));
            }
            cut[grain.rank()] += 1;
        }
        cost.note_cut(cut);
        if changed.is_empty() {
            return;
        }
        for (worn, mut mesh) in &mut worn {
            if let Some(&(_, grain)) = changed.iter().find(|(actor, _)| *actor == worn.actor) {
                mesh.0 = tailoring.nearest(grain).1.worn(worn.part).clone();
            }
        }
    }

    /// **How tall a man standing at `place` is drawn**, turf to crown, in
    /// pixels of a target `rows` tall seen through `lens` with a vertical
    /// field of view of `fov`. Nothing for a man behind the lens.
    fn height_on_screen(lens: &Transform, fov: f32, rows: u32, place: &Transform) -> f32 {
        let tall = Physique::STATURE * place.scale.y;
        let depth =
            (place.translation + Vec3::Y * tall * 0.5 - lens.translation).dot(*lens.forward());
        if depth <= 0.0 {
            return 0.0;
        }
        tall / depth * rows as f32 / (2.0 * (fov * 0.5).tan())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::players::kit::HairStyle;

    /// Well inside each band a man is cut at that band's grain, whatever he
    /// was cut at before.
    #[test]
    fn a_man_is_cut_by_his_height_on_the_screen() {
        for was in Grain::ALL {
            assert_eq!(was.at_height(2000.0, Grain::FULL), Grain::FULL);
            assert_eq!(was.at_height(280.0, Grain::FULL), Grain::SPARE);
            assert_eq!(was.at_height(60.0, Grain::FULL), Grain::FAR);
        }
    }

    /// A man the camera holds near a boundary keeps his grain until he is
    /// past it by the hysteresis, either way.
    #[test]
    fn a_man_at_a_boundary_is_not_recut_every_frame() {
        for (rank, boundary) in Grain::COARSER_BELOW.into_iter().enumerate() {
            let (finer, coarser) = (Grain::ALL[rank], Grain::ALL[rank + 1]);
            assert_eq!(finer.at_height(boundary * 0.95, Grain::FULL), finer);
            assert_eq!(coarser.at_height(boundary * 1.05, Grain::FULL), coarser);
            assert_eq!(finer.at_height(boundary * 0.8, Grain::FULL), coarser);
            assert_eq!(coarser.at_height(boundary * 1.2, Grain::FULL), finer);
        }
    }

    /// The finest grain is the machine's: a man filling a post-processed
    /// screen is cut no finer than SPARE, and on `?grain=far` nobody is cut
    /// finer than FAR.
    #[test]
    fn nobody_is_cut_finer_than_his_machine_draws() {
        let finest = Grain::of(Tier::PostProcessed, None);
        assert_eq!(finest, Grain::SPARE);
        for &was in finest.and_coarser() {
            assert_eq!(was.at_height(5000.0, finest), Grain::SPARE);
        }
        let far = Grain::of(Tier::Multisampled, Some("far"));
        for pixels in [10.0, 300.0, 5000.0] {
            assert_eq!(Grain::FAR.at_height(pixels, far), Grain::FAR);
        }
        assert_eq!(Grain::of(Tier::Multisampled, None), Grain::FULL);
    }

    /// From the gantry at rest, a man on the centre spot of a 1080-row target
    /// is under a hundred pixels tall, and cut FAR. Nothing behind the lens is
    /// drawn at all.
    #[test]
    fn the_rest_shot_cuts_the_middle_of_the_pitch_far() {
        let lens =
            Transform::from_xyz(0.0, Physique::STATURE * 0.5, -84.0).looking_to(Vec3::Z, Vec3::Y);
        let spot = Transform::default();
        let pixels = Tailoring::height_on_screen(&lens, 0.265, 1080, &spot);
        assert!((80.0..95.0).contains(&pixels), "he is {pixels:.0} px tall");
        assert_eq!(Grain::FULL.at_height(pixels, Grain::FULL), Grain::FAR);
        let behind = Transform::from_xyz(0.0, 0.0, -90.0);
        assert_eq!(
            Tailoring::height_on_screen(&lens, 0.265, 1080, &behind),
            0.0
        );
    }

    /// **Every mesh a man wears names its part**, and that part resolves to
    /// the mesh he is wearing — or he could not be re-dressed at another
    /// grain. Each part is its own mesh at every grain.
    #[test]
    fn every_mesh_a_man_wears_can_be_cut_again() {
        let mut meshes = Assets::<Mesh>::default();
        let mut bindposes = Assets::<SkinnedMeshInverseBindposes>::default();
        let mut tailoring = Tailoring::new(&mut meshes, &mut bindposes, Grain::SPARE);
        let coarser = BodyParts::new(&mut meshes, Grain::FAR);
        tailoring.keep(&meshes, Grain::FAR, coarser);
        for keeper in [false, true] {
            let mut world = World::new();
            let root = world.spawn(Transform::default()).id();
            let outfit = Outfit {
                kit: Handle::default(),
                limb: Handle::default(),
                strip: 0,
                boots: Handle::default(),
                skin: Handle::default(),
                face: Handle::default(),
                hands: Handle::default(),
                hair: Handle::default(),
                hair_style: HairStyle::Short,
                number: Some(Handle::default()),
                name: Some(Handle::default()),
                name_front: Some(Handle::default()),
            };
            tailoring.dress(&mut world.commands(), root, &outfit, keeper);
            world.flush();
            assert_eq!(
                world.get::<Detail>(root).map(|detail| detail.0),
                Some(Grain::SPARE)
            );
            let mut worn = world.query::<(&Mesh3d, Option<&Worn>)>();
            let mut count = 0;
            for (mesh, tag) in worn.iter(&world) {
                let tag = tag.expect("a mesh he wears names no part");
                assert_eq!(tag.actor, root);
                assert_eq!(&mesh.0, tailoring.finest().worn(tag.part), "{:?}", tag.part);
                assert_ne!(
                    tailoring.nearest(Grain::FAR).1.worn(tag.part),
                    tailoring.finest().worn(tag.part),
                    "{:?} is the same mesh at both grains",
                    tag.part
                );
                count += 1;
            }
            assert!(count > 15, "only {count} meshes on a man");
        }
    }

    /// While the bring-up has still to cut a grain, a man who wants it is cut
    /// at the coarsest one there is.
    #[test]
    fn a_grain_not_yet_cut_falls_back_to_the_coarsest_there_is() {
        let mut meshes = Assets::<Mesh>::default();
        let mut bindposes = Assets::<SkinnedMeshInverseBindposes>::default();
        let tailoring = Tailoring::new(&mut meshes, &mut bindposes, Grain::SPARE);
        assert_eq!(tailoring.nearest(Grain::FAR).0, Grain::SPARE);
        assert_eq!(tailoring.nearest(Grain::SPARE).0, Grain::SPARE);
    }
}
