//! **What the scene HOLDS**, in bytes, said out loud at every phase.
//!
//! [`perf`](crate::app::perf) measures a frame and [`quality`](crate::app::quality)
//! and [`stage`](crate::app::stage) spend against it. None of the three can see
//! the failure this file exists for: on iOS a tab that asks for too much is not
//! slow, it is **killed** — WebKit tears the WebContent process down for
//! crossing a memory ceiling and the browser starts the page again. There is no
//! console, no stack and no error page afterwards, because nothing threw. The
//! only trace is that the match reloads instead of opening.
//!
//! So the viewer has to be able to SAY how much it holds, from inside, while it
//! is still holding it. Two figures, and they answer different halves:
//!
//! - **The heap.** `wasm_bindgen::memory()` is the `WebAssembly.Memory` this
//!   module was instantiated with, and its buffer's `byteLength` is the whole of
//!   the linear memory the browser has committed. ⚠ **It only ever grows.**
//!   wasm32 has no `memory.shrink`; dlmalloc keeps freed pages for reuse and the
//!   browser accounts the entire buffer either way, so every transient peak is a
//!   permanent bill and *sequencing* two large allocations is worth as much as
//!   shrinking either of them. That is why the reading kept here is a
//!   high-water mark rather than a level.
//! - **The ledger.** Everything uploaded to the GPU is invisible to the figure
//!   above — a vertex buffer and a texture live in the driver, not in the wasm
//!   heap — and it is the larger of the two on this scene. It cannot be counted
//!   afterwards either: every mesh in this crate is built with
//!   `RenderAssetUsages::RENDER_WORLD`, so its data is dropped from the main
//!   world the moment it has been extracted. The bytes have to be **told** at
//!   the moment they are made, which is what [`MemoryBill::note`] is for, and
//!   the same argument [`FrameCost::note_geometry`](crate::app::perf::FrameCost)
//!   records for the triangle count it grew out of.
//!
//! ## Why it is a thread-local and not a resource
//!
//! Most of what is worth counting is made a long way from a system. A mip chain
//! is built inside a private associated function of
//! [`Textures`](crate::art::textures::Textures) with no world in reach; a
//! spectator bank is built by `Crowd::fill`, four calls down from the course
//! that spawns it. Threading a `ResMut` to all of them would rewrite half the
//! crate to carry a diagnostic. WebAssembly is single-threaded and the crate
//! already keeps a thread-local for exactly this reason (see `COMMISSIONED` in
//! [`loader`](crate::recording::loader)), so the ledger is one too: any code
//! that can make bytes can name them, and nothing has to be plumbed.

use bevy::image::Image;
use bevy::mesh::{Indices, Mesh};
use std::cell::Cell;

/// **What a byte is FOR**, which is the only division that makes a bill worth
/// reading.
///
/// One entry per thing that could plausibly be the answer to "which part of the
/// scene was too big", because a single total says the tab died and nothing
/// else. The order is the order the bill prints in, which is roughly the order
/// the bring-up allocates in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Held {
    /// The four spectator banks. Computed to be some 88% of the scene's GPU
    /// bytes, which is why it is first — see
    /// [`Throng`](crate::scene::crowd::Throng).
    Crowd,
    /// The ground the crowd sits on and the pitch it looks at: the turf and
    /// surround swards, the paint, the goal frames, the netting, the terraces
    /// and the hoardings.
    Ground,
    /// Every sheet this crate paints for itself — the turf albedo and its
    /// relief, the crowd palette, the seats, the adverts, the sky, the icons.
    /// Mip chains included, which for a 1024-square sheet is a third again.
    Sheets,
    /// The shared body parts the twenty-two are assembled from, cut once at
    /// [`Grain`](crate::players::body::Grain) and worn by everybody.
    Squad,
    /// …and what is not shared: a kit, a number, a name and a face per man.
    Faces,
    /// The render target the replay is drawn into, and the attachments Bevy
    /// puts behind it. The one entry that is *replaced* rather than added to —
    /// see [`MemoryBill::hold`] — because a resize frees the old one.
    Stage,
    /// The recording, as samples resident in
    /// [`ReplayTracks`](crate::recording::replay::ReplayTracks). Replaced, for
    /// the same reason: it is a level, not a total.
    Recording,
}

impl Held {
    /// How many there are. Written out rather than derived, because the array
    /// below has to be sized at compile time and a `const` count is the only
    /// thing that can do it without a crate for it.
    const KINDS: usize = 7;

    fn slot(self) -> usize {
        match self {
            Held::Crowd => 0,
            Held::Ground => 1,
            Held::Sheets => 2,
            Held::Squad => 3,
            Held::Faces => 4,
            Held::Stage => 5,
            Held::Recording => 6,
        }
    }

    /// Short enough to fit six of them on a phone's transport bar.
    fn label(self) -> &'static str {
        match self {
            Held::Crowd => "crowd",
            Held::Ground => "ground",
            Held::Sheets => "sheets",
            Held::Squad => "squad",
            Held::Faces => "faces",
            Held::Stage => "stage",
            Held::Recording => "rec",
        }
    }

    /// The whole set, in printing order.
    const ALL: [Held; Self::KINDS] = [
        Held::Crowd,
        Held::Ground,
        Held::Sheets,
        Held::Squad,
        Held::Faces,
        Held::Stage,
        Held::Recording,
    ];
}

thread_local! {
    /// Bytes told, by kind. A `Cell` of an array rather than a `RefCell` of a
    /// map: the array is `Copy`, so a note is a read, an add and a write with
    /// no borrow to get wrong on a re-entrant call — and mesh building is
    /// re-entrant, since a merged part is built out of parts.
    static LEDGER: Cell<[usize; Held::KINDS]> = const { Cell::new([0; Held::KINDS]) };
    /// The largest linear memory ever read. See the module note: it is the
    /// figure that matters, because the small one is a lie on wasm32.
    static PEAK: Cell<usize> = const { Cell::new(0) };
    /// Which kind an untagged sheet is charged to. See [`Charge`].
    static CHARGE: Cell<Held> = const { Cell::new(Held::Sheets) };
}

/// **Whose bill the next sheet goes on**, for the one place the maker cannot
/// say.
///
/// Every image in this crate is built by two private associated functions of
/// [`Textures`](crate::art::textures::Textures) — the mip chain and the plain
/// one — and neither has the faintest idea what it is drawing. A turf sheet
/// and a photographed face go through exactly the same three lines. Passing a
/// kind down would mean threading one through some forty call sites for a
/// diagnostic, and hard-coding "everything is scenery" would file a squad's
/// worth of faces under the pitch.
///
/// So the CALLER says, for as long as it is drawing: `let _charge =
/// MemoryBill::charge(Held::Faces);` at the top of a function puts every sheet
/// made under it on the face bill and puts the ledger back on the way out.
/// Sound because the target is single-threaded and the guard restores what it
/// found rather than a constant, so two of them nest.
pub struct Charge {
    restored: Held,
}

impl Drop for Charge {
    fn drop(&mut self) {
        CHARGE.with(|charge| charge.set(self.restored));
    }
}

/// The bill, as somewhere to hang the arithmetic. Holds nothing itself — the
/// figures are in the thread-locals above, for the reason the module note
/// gives.
pub struct MemoryBill;

impl MemoryBill {
    /// **Add bytes to a kind**, at the moment they are made.
    pub fn note(kind: Held, bytes: usize) {
        LEDGER.with(|ledger| {
            let mut kinds = ledger.get();
            kinds[kind.slot()] += bytes;
            ledger.set(kinds);
        });
    }

    /// **Replace** a kind's figure, for the two that are a level rather than a
    /// running total: the render target, which is freed and remade on every
    /// resize and every rung of the ladder, and the recording, which is a
    /// window that both grows and is evicted from. Adding to either would
    /// print the sum of every size it has ever been.
    pub fn hold(kind: Held, bytes: usize) {
        LEDGER.with(|ledger| {
            let mut kinds = ledger.get();
            kinds[kind.slot()] = bytes;
            ledger.set(kinds);
        });
    }

    /// One mesh, counted off its own vertex layout rather than off an assumed
    /// one.
    ///
    /// A crowd figure carries position, normal and uv — 32 bytes — and a
    /// footballer's parts carry the same, but a change to either would
    /// silently invalidate a hard-coded stride, and this ledger exists to
    /// catch exactly that class of drift. `get_vertex_size` is the buffer
    /// layout the renderer will actually upload.
    ///
    /// Indices are counted at their real width, which is the whole reason the
    /// U16/U32 choice is worth anything: a bank that fits in `U16` halves this
    /// term.
    pub fn mesh(kind: Held, mesh: &Mesh) {
        Self::note(kind, Self::mesh_bytes(mesh));
    }

    /// The same figure without the note, for a caller that wants to say it
    /// itself — [`BodyParts`](crate::players::body::BodyParts) sums a working
    /// set of eighteen parts and reports one line.
    pub fn mesh_bytes(mesh: &Mesh) -> usize {
        let vertices = mesh.count_vertices() * mesh.get_vertex_size() as usize;
        let indices = match mesh.indices() {
            Some(Indices::U16(indices)) => indices.len() * 2,
            Some(Indices::U32(indices)) => indices.len() * 4,
            None => 0,
        };
        vertices + indices
    }

    /// One image, counted off the buffer that will be uploaded — which for
    /// anything mipped is the whole chain and not the base level.
    pub fn image(kind: Held, image: &Image) {
        Self::note(kind, image.data.as_ref().map_or(0, |texels| texels.len()));
    }

    /// The same, charged to whatever the caller above said it was drawing —
    /// see [`Charge`]. Called from the two places every sheet in this crate is
    /// actually made.
    pub fn sheet(image: &Image) {
        Self::image(CHARGE.with(|charge| charge.get()), image);
    }

    /// Opens a charge. Hold the returned guard for as long as the sheets being
    /// made belong to `kind`.
    #[must_use = "the charge lasts exactly as long as the guard"]
    pub fn charge(kind: Held) -> Charge {
        let restored = CHARGE.with(|charge| charge.replace(kind));
        Charge { restored }
    }

    /// What one kind has come to.
    pub fn of(kind: Held) -> usize {
        LEDGER.with(|ledger| ledger.get()[kind.slot()])
    }

    /// …and the whole scene.
    pub fn total() -> usize {
        LEDGER.with(|ledger| ledger.get().iter().sum())
    }

    /// **The linear memory the browser has committed**, in bytes, and the
    /// high-water mark updated as a side effect.
    ///
    /// Read through `Reflect` rather than through `js_sys::WebAssembly::Memory`
    /// deliberately: `wasm_bindgen::memory()` hands back a `JsValue` and the
    /// only property wanted off it is `buffer.byteLength`, so going through the
    /// typed wrapper would tie this line to a `js-sys` shape for no reading it
    /// does not already get.
    ///
    /// Zero when there is no browser to ask, which is every test in this crate.
    #[cfg(target_arch = "wasm32")]
    pub fn heap() -> usize {
        use wasm_bindgen::JsValue;

        let bytes = js_sys::Reflect::get(&wasm_bindgen::memory(), &JsValue::from_str("buffer"))
            .ok()
            .and_then(|buffer| js_sys::Reflect::get(&buffer, &JsValue::from_str("byteLength")).ok())
            .and_then(|length| length.as_f64())
            .unwrap_or(0.0) as usize;
        PEAK.with(|peak| peak.set(peak.get().max(bytes)));
        bytes
    }

    /// There is no `WebAssembly.Memory` off the web, and no tab to be killed
    /// for filling one.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn heap() -> usize {
        0
    }

    /// The largest the heap has ever been — see the module note for why this
    /// and not the current figure is the number that decides whether the tab
    /// lives.
    pub fn peak() -> usize {
        PEAK.with(|peak| peak.get())
    }

    /// Megabytes, as everything that prints one of these wants them.
    pub fn mib(bytes: usize) -> f32 {
        bytes as f32 / (1024.0 * 1024.0)
    }

    /// **The bill**: heap, then every kind that has anything in it.
    ///
    /// Printed once at `ready` and available on the transport strip, which on
    /// a phone is the only readout there is — there is no console on iOS
    /// without a Mac and a cable.
    pub fn line() -> String {
        let heap = Self::heap();
        let mut line = format!(
            "match viewer — holding {:.0} MiB of GPU assets · wasm heap {:.0} MiB (peak {:.0})",
            Self::mib(Self::total()),
            Self::mib(heap),
            Self::mib(Self::peak()),
        );
        for kind in Held::ALL {
            let bytes = Self::of(kind);
            if bytes > 0 {
                line.push_str(&format!(" · {} {:.1}", kind.label(), Self::mib(bytes)));
            }
        }
        line
    }

    /// The console, on the one target that has one. Twin of
    /// [`FrameCost::announce`](crate::app::perf::FrameCost), and separate from
    /// it for the same reason this module is separate from that one: the two
    /// are read by different people chasing different failures.
    #[cfg(target_arch = "wasm32")]
    pub fn announce(line: &str) {
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(line));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn announce(_line: &str) {}

    /// The same, cut to what fits beside the frame cost on the bar: the heap
    /// peak, the ledger total, and the crowd — which is the one entry big
    /// enough that watching it alone answers most of the question.
    pub fn strip() -> String {
        format!(
            "h{:.0}/{:.0} g{:.0} c{:.0}",
            Self::mib(Self::heap()),
            Self::mib(Self::peak()),
            Self::mib(Self::total()),
            Self::mib(Self::of(Held::Crowd)),
        )
    }

    /// Everything back to nothing. Tests only: the ledger is a thread-local
    /// and Rust's test harness runs several tests on one thread, so a test
    /// that asserts on a figure has to start from a known one.
    #[cfg(test)]
    pub fn forget() {
        LEDGER.with(|ledger| ledger.set([0; Held::KINDS]));
        PEAK.with(|peak| peak.set(0));
        CHARGE.with(|charge| charge.set(Held::Sheets));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    /// A mesh is counted off its own layout, and its indices at their own
    /// width — which is the property the U16/U32 decision is worth anything
    /// against.
    #[test]
    fn a_mesh_is_counted_off_the_buffers_it_will_upload() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]; 4])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4]);
        mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        // Four vertices of position, normal and uv — twelve, twelve and eight
        // — and six 32-bit indices.
        assert_eq!(MemoryBill::mesh_bytes(&mesh), 4 * 32 + 6 * 4);

        mesh.insert_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]));
        assert_eq!(MemoryBill::mesh_bytes(&mesh), 4 * 32 + 6 * 2);
    }

    /// A note accumulates and a hold replaces, which is the whole difference
    /// between a total and a level.
    #[test]
    fn a_level_is_replaced_where_a_total_is_added_to() {
        MemoryBill::forget();
        MemoryBill::note(Held::Crowd, 100);
        MemoryBill::note(Held::Crowd, 50);
        assert_eq!(MemoryBill::of(Held::Crowd), 150);

        MemoryBill::hold(Held::Stage, 400);
        MemoryBill::hold(Held::Stage, 200);
        assert_eq!(MemoryBill::of(Held::Stage), 200);
        assert_eq!(MemoryBill::total(), 350);
        MemoryBill::forget();
    }

    /// **The desktop half of the instrument.**
    ///
    /// The bill this crate prints in a browser can only be read on the device
    /// it is printed on, and the device it matters on is a phone with no
    /// console. So the same figures are built here, off the same builders the
    /// scene uses, and printed on a terminal — which is where a regression
    /// should be caught, rather than months later off somebody's tablet.
    ///
    /// Run it with:
    ///
    /// ```text
    /// cargo test --manifest-path src/match/Cargo.toml \
    ///     dump_memory_bill -- --ignored --nocapture
    /// ```
    ///
    /// ⚠ **It is not the whole scene.** Anything that needs a `World` — the
    /// paint, the goal frames, the per-player kits and faces, the render
    /// attachments — is absent, because building it would mean standing up a
    /// Bevy app with a GPU behind it. What is here is what the measurements in
    /// `docs/match_viewer_handheld_memory_prompt.md` say the bill is made of:
    /// the crowd, the ground it sits on, the shared squad and the sheets. The
    /// missing entries are the reason the browser-side readout exists as well.
    #[test]
    #[ignore = "prints the scene's memory bill; run with --ignored --nocapture"]
    fn dump_memory_bill() {
        use crate::app::config::VenueInfo;
        use crate::app::quality::Footprint;
        use crate::app::stage::Stage;
        use crate::art::textures::Textures;
        use crate::players::body::{BodyParts, Grain};
        use crate::scene::crowd::{Crowd, Spectators, Stature, Throng};
        use crate::scene::pitch::Stands;
        use bevy::prelude::{Assets, Color, StandardMaterial, UVec2};

        for footprint in [Footprint::Roomy, Footprint::Handheld] {
            MemoryBill::forget();
            let mut images = Assets::<Image>::default();
            let mut materials = Assets::<StandardMaterial>::default();
            let mut meshes = Assets::<Mesh>::default();

            // A great ground, comfortably full — the ceiling rather than the
            // common case, and the one a bill should be written against.
            let stature = Stature::of(&VenueInfo::default());
            let throng = Throng::of(footprint, None);

            // The sheets, in the order the bring-up paints them.
            let _turf = Textures::turf(&mut images, Color::srgb(0.10, 0.35, 0.12));
            let _seats = Textures::seats(&mut images);
            let _sky = Textures::sky(&mut images);
            let _net = Textures::netting(&mut images);
            let _ball = Textures::football(&mut images);
            let _boards =
                Textures::hoarding(&mut images, "OF", "OpenFootball", "open-football.org");
            let spectators = Spectators::dressed(
                &mut images,
                &mut materials,
                (Color::srgb(0.10, 0.16, 0.34), Color::WHITE),
                (Color::srgb(0.70, 0.25, 0.00), Color::WHITE),
            );

            // The four banks, off the same plan the scene builds from.
            for (bank, plan) in Stands::plan(stature).iter().enumerate() {
                let Some(throng) = throng else { continue };
                let Some(crowd) = Crowd::fill(
                    plan.terrace(),
                    stature,
                    plan.stand(),
                    spectators.palette(),
                    bank as u32 + 1,
                    throng,
                ) else {
                    continue;
                };
                MemoryBill::mesh(Held::Crowd, &crowd);
            }

            // The shared squad, at the grain this footprint would cut.
            let grain = match footprint {
                Footprint::Roomy => Grain::FULL,
                Footprint::Handheld => Grain::SPARE,
            };
            let parts = BodyParts::new(&mut meshes, grain);
            MemoryBill::note(Held::Squad, parts.bytes(&meshes));

            // …and the attachments, at the canvas an iPad shows in landscape.
            let window = UVec2::new(2360, 1328);
            let target = Stage::measured(window, Stage::budget(footprint, None));
            let samples = match footprint {
                Footprint::Roomy => 4,
                Footprint::Handheld => 1,
            };
            MemoryBill::hold(Held::Stage, Stage::attachments(target, window, samples));

            println!(
                "\n{} · squad {} tri/player · stage {}x{} at {} sample(s)\n  {}",
                match footprint {
                    Footprint::Roomy => "ROOMY",
                    Footprint::Handheld => "HANDHELD",
                },
                parts.triangles(&meshes),
                target.x,
                target.y,
                samples,
                MemoryBill::line(),
            );
        }
        MemoryBill::forget();
    }

    /// A charge nests and puts back what it found, which is what lets a face
    /// be drawn from inside something that was already drawing scenery.
    #[test]
    fn a_charge_nests_and_restores() {
        MemoryBill::forget();
        let mut sheet = Image::new_uninit(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        sheet.data = Some(vec![0; 64]);

        MemoryBill::sheet(&sheet);
        {
            let _faces = MemoryBill::charge(Held::Faces);
            MemoryBill::sheet(&sheet);
            {
                let _ground = MemoryBill::charge(Held::Ground);
                MemoryBill::sheet(&sheet);
            }
            MemoryBill::sheet(&sheet);
        }
        MemoryBill::sheet(&sheet);

        assert_eq!(MemoryBill::of(Held::Sheets), 128, "two sheets, uncharged");
        assert_eq!(MemoryBill::of(Held::Faces), 128, "two inside the charge");
        assert_eq!(MemoryBill::of(Held::Ground), 64, "one inside the nested");
        MemoryBill::forget();
    }
}
