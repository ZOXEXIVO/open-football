//! How many pixels the replay is actually drawn into.
//!
//! [`crate::app::quality`] deals with the other half of the same problem and
//! says why it is a problem at all: on an integrated part the frame is bound
//! per SAMPLE, and there are only two ways to write fewer of them — take
//! fewer samples per pixel, which is what the tier does, or have fewer pixels,
//! which is what this does.
//!
//! Of the two this is the one with the better exchange rate. Sampling is a
//! ladder with two rungs on the web (WebGL2 offers one sample or four and
//! nothing between), where resolution is continuous: a scene drawn at 87% of
//! the canvas costs 76% of the fill and gives up a quarter of a pixel of
//! sharpness, which nobody can see on a moving picture. **A softer frame that
//! arrives on time reads as better than a sharp one that does not** — motion
//! is what the eye judges a replay by, and a camera that hitches four times a
//! second is the complaint this whole file exists to answer.
//!
//! ## Why it cannot simply be asked for
//!
//! Bevy has a knob that looks like exactly this — `WindowResolution::
//! set_scale_factor_override` — and on the web it is a trap, twice over:
//!
//! - It does not size the backing store. winit's web backend reports the
//!   canvas size from a `ResizeObserver` in physical pixels (CSS size times
//!   `devicePixelRatio`), and `react_to_resize` writes that straight into the
//!   resolution the surface is configured from. The override only changes what
//!   Bevy calls the LOGICAL size derived from it.
//! - Setting it does damage. `changed_windows` answers a changed override by
//!   calling `request_inner_size`, and winit's web backend implements that by
//!   writing `style.width` and `style.height` in pixels — over the `100%` that
//!   `fit_canvas_to_parent` put there. The canvas stops being responsive, and
//!   the ResizeObserver then reports the size it was just given, which feeds
//!   back in.
//!
//! So the pixels have to come from somewhere this crate owns. The 3D camera
//! renders into an image of its own choosing; a second, orthographic camera
//! owns the window and draws that image across it, with the transport bar on
//! top. Which turns out to be better than a scale factor would have been
//! anyway: **the interface is not scaled**. Text, the seek rail and the chips
//! are laid out and rasterised at the canvas's own resolution whatever the
//! replay behind them is drawn at, so the one part of the frame where a soft
//! pixel is legible is the one part that never gets one.
//!
//! ## The ladder, and why it only goes down
//!
//! The steps are quantised so a resize is a rare event rather than a
//! continuous one — every change reallocates a multi-megabyte texture and
//! re-configures the view.
//!
//! And it is deliberately one-way. The obvious controller raises the scale
//! again when the frame looks comfortable, and on a browser it cannot work:
//! the page is held to the display's refresh, so a machine with three times
//! the headroom it needs reports exactly the same 16.7 ms as one with none,
//! and the only honest reading available is "frames are being MISSED". A
//! controller that steps up on the absence of that signal steps up into the
//! load it just escaped, misses again, steps down, and hunts — trading a
//! settled soft picture for a sharp one that stutters twice a minute and
//! reallocates its render target each time. Going one way converges in the
//! first seconds and then stops, which is what smoothness is made of.

use crate::app::bill::{Held, MemoryBill};
use crate::app::config::ViewerConfig;
use crate::app::perf::FrameCost;
use crate::app::quality::{Footprint, Quality};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::render::renderer::RenderDevice;
use bevy::render::view::Msaa;
use bevy::window::PrimaryWindow;

/// The full-screen node the replay is shown in, behind the transport bar.
#[derive(Component)]
pub struct Backdrop;

impl Backdrop {
    /// **Below zero is the picture; zero and above is the interface.**
    ///
    /// Three roots share the window and the order between them is not
    /// negotiable, so it is written down here rather than left to the order
    /// their `Startup` systems happen to run in — which Bevy does not promise.
    ///
    /// The replay is the floor. The name plates over the players' heads are
    /// part of the football, so they sit on it and go dark with it. The dip
    /// between two clips ([`crate::broadcast::cut`]) covers both. And
    /// everything at the default zero — the transport bar, the flight stick,
    /// the altitude buttons — is furniture laid over the lot of it, which
    /// never dims, because a control that went dark at every cut would read as
    /// a fault.
    pub const PICTURE: i32 = -3;
    /// The plates, over the picture and under the dip. See [`Self::PICTURE`].
    pub const PLATES: i32 = -2;
    /// The cut's own veil, over both. See [`Self::PICTURE`].
    pub const DIP: i32 = -1;
}

/// The image the replay is drawn into, and how large it is being kept.
#[derive(Resource)]
pub struct Stage {
    /// What [`crate::broadcast::camera::TvCamera`] renders into and
    /// [`Backdrop`] shows.
    canvas: Handle<Image>,
    /// Where on [`Self::SCALES`] the controller has settled.
    step: usize,
    /// The size the image currently is, so a frame that changed nothing does
    /// not reallocate it.
    size: UVec2,
    /// The share of the canvas [`Self::size`] came out at, which is the rung
    /// unless [`Self::BUDGET`] or the device had to cut into it. Kept only so
    /// [`Self::readout`] reports the picture on screen rather than the one
    /// that was asked for.
    drawn: f32,
    /// Seconds left before the frame cost is consulted again.
    review: f32,
    /// Consecutive reviews that have found the display being missed. See
    /// [`Self::CONSECUTIVE`].
    missing: u32,
    /// The most pixels this device may be asked for, in area — see
    /// [`Self::budget`].
    ///
    /// Re-derived at the top of every [`Self::fit`] rather than settled once
    /// here, and that is not laziness. This resource is built by
    /// `init_resource`, which runs before `PreStartup`, and `PreStartup` is
    /// where [`Quality::confirm`] gets its one chance to correct the footprint
    /// off the adapter wgpu actually opened. A budget cached at construction
    /// would be the guess and not the correction.
    budget: u32,
    /// What the page asked for, if anything, kept so the line above can be
    /// re-derived without going back to the config every frame.
    asked: Option<f32>,
}

impl Stage {
    /// The rungs, sharpest first.
    ///
    /// Five of them across a range of about three to one in fill cost —
    /// 1.00, 0.76, 0.56, 0.42 and 0.30 of full — which is enough to carry a
    /// part that is missing every other frame back onto the refresh. The gaps
    /// are geometric rather than even so each step is the same proportional
    /// relief; an even ladder spends its rungs at the sharp end where they buy
    /// least.
    ///
    /// It stops at 0.55 rather than going on down. Below about that the
    /// replay stops reading as a soft picture and starts reading as a small
    /// one — the paint on the pitch breaks up and the players lose their
    /// outline — and a machine that cannot hold 30% of a canvas is not going
    /// to be rescued by 20%.
    const SCALES: [f32; 5] = [1.0, 0.87, 0.75, 0.65, 0.55];

    /// Median frame time, in milliseconds, that counts as missing the display.
    ///
    /// The page is held to the refresh rate, so a machine keeping up on a
    /// 60 Hz panel reads 16.7 and one keeping up on 120 Hz reads 8.3 — neither
    /// is near this. 24 ms is a frame and a half of a 60 Hz display: it can
    /// only be read by dropping frames, which is the thing the eye is
    /// objecting to. Shared with [`Quality`] on purpose; they are answering
    /// the same question and a machine that fails one should fail the other.
    const STRUGGLING_MS: f32 = 24.0;

    /// …and the ceiling on believing it. Past this the page is not slow, it is
    /// SUSPENDED — a backgrounded tab, a breakpoint, a laptop lid — and no
    /// amount of resolution would have helped. Stepping down on it would leave
    /// a viewer who came back to their tab looking at a picture they never
    /// asked to lose.
    ///
    /// ⚠ **It was 100 ms, and there it disabled this ladder on precisely the
    /// machines the ladder is for.** See
    /// [`Quality::STALLED_MS`](crate::app::quality::Quality), which carries
    /// the whole argument: a part managing six to nine frames a second reads
    /// 110–170 ms, was classified as suspended, and got not one rung. The
    /// ceiling belongs here, but the thing that separates a suspended page
    /// from a slow one is that a suspended page comes BACK — so the test is
    /// persistence ([`Self::CONSECUTIVE`]) and the magnitude bound moves up to
    /// where it only catches a page that has genuinely stopped.
    const STALLED_MS: f32 = 500.0;

    /// How many consecutive reviews have to say the frame is being missed
    /// before a rung is spent.
    ///
    /// Cheaper to be wrong here than in [`Quality`] — a rung costs a texture
    /// reallocation where a tier costs a scene-wide shader recompile — but the
    /// same discriminator applies and it is worth a single extra review to
    /// avoid handing a soft picture to somebody whose tab was simply behind
    /// another window for five seconds.
    const CONSECUTIVE: u32 = 2;

    /// How often the frame cost is consulted, in seconds.
    ///
    /// `FrameCost`'s own window is two seconds wide, so anything under that is
    /// asking the same question twice and would step down twice for one slow
    /// stretch.
    const REVIEW: f32 = 2.5;

    /// The most pixels a render target will be asked for.
    ///
    /// Not a quality limit — a memory one. `fit_canvas_to_parent` plus a
    /// fullscreen button plus a 4K panel at 200% scaling would otherwise ask
    /// for a 33-megapixel target and the multisampled colour and depth
    /// attachments behind it, which is a browser tab falling over rather than
    /// a replay looking better.
    ///
    /// **Stated as an area, because area is what an attachment costs.** It
    /// used to be a length capped per axis, and that is exactly how it went
    /// wrong: a canvas over the cap on ONE axis came back a different SHAPE,
    /// and [`Backdrop`] stretches whatever it is handed across the whole
    /// window — so a 5120x1440 panel was drawing every player a third too
    /// wide. See [`Self::wanted`], which now has one factor for both axes.
    ///
    /// 3840x2160 worth of it: a 4K panel at its own resolution, and the
    /// largest frame this viewer has been measured on. A super-ultrawide
    /// comes in UNDER it — 5120x1440 is 7.4 megapixels against 8.3 — and so
    /// keeps every one of its pixels, which is the right answer. It is not a
    /// bigger frame to draw, only a differently shaped one.
    ///
    /// ⚠ **A desktop number, and on a phone it never bites at all.** See
    /// [`Self::HANDHELD_BUDGET`], which is the same constant asked of the
    /// device the ceiling actually matters on.
    const BUDGET: u32 = 3840 * 2160;

    /// **…and what a handheld may ask for**, which is a different question
    /// with a different answer.
    ///
    /// The budget above was chosen as "big enough that nothing sane hits it".
    /// On an iPad nothing sane does: landscape on a 10-inch panel is about
    /// 1180x664 CSS pixels at `devicePixelRatio` 2, which is 2360x1328 — 3.1
    /// megapixels, comfortably under 8.3 — so the cap that exists to stop a
    /// tab falling over never fired on the one class of device whose tab
    /// actually falls over.
    ///
    /// 1.3 megapixels is about that canvas at DPR 1. The picture is upscaled
    /// to the window by [`Backdrop`] with a bilinear filter, which is exactly
    /// what every rung of [`Self::SCALES`] already does and what the note at
    /// the top of this file argues is invisible in motion.
    ///
    /// What it buys, measured as what the scene ASKS for rather than off a
    /// device: the 3D view's colour pair, its depth and the stage image come
    /// to some 50 MB at 3.1 MP and 21 MB at 1.3 MP, and in the 12.9-inch
    /// fullscreen case (2732x2048 = 5.6 MP) the saving doubles again.
    ///
    /// It is also the one item in this file that is a memory lever and a FRAME
    /// lever at the same time. The measurement the module note quotes — the
    /// same 3.9 ms at 720p and at 4K — was taken on a discrete card, and it
    /// does not travel to a tile-based mobile part where every attachment
    /// write crosses the same bus as everything else. 3.1 MP through the main
    /// pass, FXAA and the upscale, sixty times a second, is real work there.
    ///
    /// ⚠ **The ladder is deliberately NOT started a rung down as well.** This
    /// cap already cuts an iPad's stage to roughly 0.55 on a side, which is the
    /// bottom rung's worth of relief before the ladder has spent anything;
    /// compounding the two would open the replay at a third of the canvas on a
    /// device that might well have held two thirds. The ladder is still free to
    /// walk down from here on the measurement, which is the right way round.
    const HANDHELD_BUDGET: u32 = 1_300_000;

    /// The longest side to allow before the renderer has said what it can
    /// actually allocate.
    ///
    /// WebGL2 guarantees only 2048, and a device that means it would refuse
    /// the texture rather than draw a soft one. In practice nothing reaches
    /// here: [`Self::fit`] reads the real figure off the device — which
    /// exists before the first frame, `RenderPlugin::finish` puts it in the
    /// main world before any schedule runs — and desktop parts report 8192 or
    /// 16384. This is what the tests use, and what one frame would fall back
    /// to if that ever stopped being true.
    const GUARANTEED_SIDE: u32 = 2048;

    fn scale(&self) -> f32 {
        Self::SCALES[self.step]
    }

    /// **The most pixels this device may be asked for**, in area.
    ///
    /// One factor decides it — what kind of machine this is, not what it
    /// measures at — because the failure the handheld figure answers happens
    /// on the way UP, before there is a frame time to read. See
    /// [`Self::HANDHELD_BUDGET`].
    ///
    /// `?stage=<megapixels>` overrides it, alongside `?device=` and `?crowd=`:
    /// the attachments are the largest allocation in this scene that is not
    /// geometry, and a device that reloads its tab rather than reporting
    /// anything can only be bisected from its address bar. Anything
    /// unreadable, zero or negative falls through to the answer the device
    /// would have had, which is the same way every other override in this
    /// crate declines to be given nonsense.
    pub fn budget(footprint: Footprint, asked: Option<f32>) -> u32 {
        match asked {
            Some(megapixels) if megapixels > 0.0 => (megapixels * 1_000_000.0) as u32,
            _ => match footprint {
                Footprint::Roomy => Self::BUDGET,
                Footprint::Handheld => Self::HANDHELD_BUDGET,
            },
        }
    }

    /// **What the render attachments behind this stage come to**, in bytes.
    ///
    /// Told rather than counted, for the reason the whole ledger is — see
    /// [`MemoryBill`]. Nothing in the main world can see a texture the render
    /// sub-app cached, and these are the largest allocation in the scene that
    /// is not a mesh.
    ///
    /// Two views, and they never share. Bevy keys its colour pair on
    /// `(target, usage, format, samples)` — `prepare_view_targets` — and the
    /// replay's camera draws into an IMAGE while the window camera draws into
    /// the WINDOW, so the two targets differ whatever the sizes do. That was
    /// worth checking rather than assuming: at the top rung the two are the
    /// same size and the same format, and a cache that keyed on the descriptor
    /// alone would have halved this figure.
    ///
    /// - The replay's view, at the stage's size: `main_texture_a` and `_b` at
    ///   four bytes a pixel, the resolve target at `samples` times that when
    ///   there is multisampling, and a `Depth32Float` at `samples` times.
    /// - The window's view, at the canvas's size, always one sample — see
    ///   [`Self::spawn`], which turns multisampling off there because nothing
    ///   in that pass has an edge for a sample to find.
    /// - The stage image itself, which is what the first is resolved into and
    ///   the second reads.
    ///
    /// ⚠ **The swapchain is NOT in this figure.** The browser owns those
    /// buffers and there are two or three of them at the canvas's size; a
    /// reader chasing the difference between this and a device's own
    /// measurement should add roughly `window x 4 x 2`.
    pub(crate) fn attachments(target: UVec2, window: UVec2, samples: u32) -> usize {
        let pixels = |size: UVec2| size.x as usize * size.y as usize * 4;
        let samples = samples.max(1) as usize;
        // The resolve target exists only where there is something to resolve.
        let resolved = if samples > 1 { samples } else { 0 };
        // Colour pair, the resolve target, and the depth buffer at the same
        // sample count as the colour.
        let replay = pixels(target) * (2 + resolved + samples);
        // The window's pair and its own depth, always at one sample.
        let canvas = pixels(window) * 3;
        replay + canvas + pixels(target)
    }

    /// The band this ladder judges by, for
    /// [`Quality`](crate::app::quality::Quality) to check itself against. The
    /// two controllers read the same median and answer it the same way, and
    /// keeping them in step by comment alone is what let them drift apart:
    /// the ceiling was wrong in both files and had to be found twice.
    #[cfg(test)]
    pub(crate) fn struggling_ms() -> f32 {
        Self::STRUGGLING_MS
    }

    #[cfg(test)]
    pub(crate) fn stalled_ms() -> f32 {
        Self::STALLED_MS
    }

    /// Which rung the ladder has settled on, for the debug strip. The size
    /// alongside it, because "75%" of what is the question a reader of a
    /// resolution actually has.
    ///
    /// The percentage is what was actually drawn rather than the rung that
    /// was asked for. On a canvas the budget has to cut into, the two are not
    /// the same number, and the one worth reading is the picture on screen.
    pub fn readout(&self) -> String {
        format!("{:.0}%={}x{}", self.drawn * 100.0, self.size.x, self.size.y)
    }

    /// The size the image should be for this window and this rung: the same
    /// SHAPE as the canvas, within the budget, and never a side longer than
    /// the device will allocate.
    ///
    /// **One factor, both axes.** Every reason this target might be smaller
    /// than the canvas — the rung, the budget, the hardware — multiplies into
    /// a single scale, and none of them may touch one axis without the other.
    /// The replay is drawn into this image and then stretched across the
    /// window by [`Backdrop`]; a target of a different shape from the window
    /// is not a smaller picture, it is a WRONG one, and it lands as players
    /// too wide or too tall by exactly the ratio between the two shapes.
    ///
    /// Rounded to an even number of pixels on both axes: a multisampled
    /// attachment and its resolve are happier on one, and it keeps a one-pixel
    /// window jitter from reallocating the target on alternate frames. That
    /// rounding is itself a shape error, and the only one tolerated here — a
    /// pixel on each side of the ratio, which is a fraction of a percent at
    /// any size worth drawing into and three orders of magnitude below a
    /// stretch anybody could see.
    fn wanted(&self, window: &Window, longest_side: u32) -> UVec2 {
        let canvas = Vec2::new(
            window.physical_width() as f32,
            window.physical_height() as f32,
        )
        .max(Vec2::splat(2.0));

        let mut shrink = self.scale();
        // Memory: the budget is an area, so the factor that meets it is a
        // square root — halving both sides is what quarters an attachment.
        let affordable = self.budget as f32 / (canvas.x * canvas.y);
        if affordable < shrink * shrink {
            shrink = affordable.sqrt();
        }
        // Hardware: nothing may ask for a texture the device will not make.
        shrink = shrink.min(longest_side as f32 / canvas.max_element());

        let scaled = canvas * shrink.clamp(0.0, 1.0);
        // To the NEAREST even number rather than down to one. Truncating puts
        // the whole of the rounding error on one side of the shape, and it is
        // a whole pixel of it on a side that a rung has already made short:
        // 1280x720 at 0.65 landed 832x466 against the 832x468 it wanted, which
        // is four times the shape error of rounding the same figure properly.
        let even = (scaled / 2.0).round() * 2.0;
        UVec2::new(even.x as u32, even.y as u32).max(UVec2::splat(2))
    }

    /// What this window and this budget come to at the top rung.
    ///
    /// Here rather than in the test module because the caller is in another
    /// one — the desktop memory harness in [`crate::app::bill`] — and what it
    /// needs is the real arithmetic. A second copy of it there would be a
    /// second copy to keep in step, which is exactly the drift the harness
    /// exists to catch.
    #[cfg(test)]
    pub(crate) fn measured(window: UVec2, budget: u32) -> UVec2 {
        let mut canvas = Window::default();
        canvas
            .resolution
            .set_physical_resolution(window.x, window.y);
        Stage {
            canvas: Handle::default(),
            step: 0,
            size: UVec2::ZERO,
            drawn: Self::SCALES[0],
            review: 0.0,
            missing: 0,
            budget,
            asked: None,
        }
        .wanted(&canvas, 8192)
    }

    /// The image the camera should be pointed at.
    pub fn target(&self) -> RenderTarget {
        RenderTarget::Image(ImageRenderTarget {
            handle: self.canvas.clone(),
            // The image is sized in physical pixels already — see
            // `Self::wanted` — so its logical size and its physical size are
            // the same thing and there is no second factor to apply. What the
            // window's own scale factor does to the plates drawn over the top
            // is `Actors::place_labels`' problem, and it reads it off the
            // camera rather than being told.
            scale_factor: 1.0,
        })
    }

    /// The window camera, and the sheet the replay is shown on.
    ///
    /// Runs at `Startup` beside the rest of the spawns. Order against
    /// [`crate::ui::timeline::Timeline::spawn`] does not matter: the backdrop
    /// is held at a negative global depth, so it is behind the bar whichever of
    /// them is built first. See [`Backdrop::PICTURE`] for the whole ladder.
    pub fn spawn(mut commands: Commands, stage: Res<Stage>) {
        commands.spawn((
            Camera2d,
            Camera {
                // After the replay, which is what it is drawing.
                order: 1,
                ..default()
            },
            // Said out loud rather than left to be inferred. `bevy_ui` picks
            // the highest-order camera pointed at the primary window and this
            // is the only one that is — but "the only one" is a property of
            // the scene that a second camera added later would quietly break,
            // where this is not.
            IsDefaultUiCamera,
            // The window itself is never multisampled. Nothing is rasterised
            // into it but one textured quad and the transport bar, both of
            // them axis-aligned rectangles with no edge for a sample to find,
            // so four samples here would be four times the bandwidth of the
            // final target for a picture identical to the pixel.
            Msaa::Off,
        ));

        commands.spawn((
            Backdrop,
            ImageNode {
                image: stage.canvas.clone(),
                // The node's size decides the image's, not the other way
                // round: it is a window's worth of screen showing a target
                // that may be two thirds of that on a side, and `Auto` would
                // lay it out at its texture size and leave a band of
                // background round it.
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            // Behind every other root — the plates over the players' heads,
            // the dip between two clips, the bar and the flight stick. See
            // [`Backdrop::PICTURE`], which is where that order is set out.
            GlobalZIndex(Backdrop::PICTURE),
        ));
    }

    /// Keeps the image the size the window and the ladder say it should be,
    /// and walks down the ladder when the frame cannot be afforded.
    ///
    /// Registered at the head of the `Update` chain so a resize lands on the
    /// frame it was decided, rather than being drawn once at the old size
    /// first.
    pub fn fit(
        mut stage: ResMut<Stage>,
        mut images: ResMut<Assets<Image>>,
        window: Single<&Window, With<PrimaryWindow>>,
        quality: Res<Quality>,
        cost: Res<FrameCost>,
        time: Res<Time>,
        // Asked rather than assumed. The largest texture a device will make is
        // 2048 by the WebGL2 guarantee and 8192 or 16384 on anything with a
        // desktop part in it, and the difference decides whether a very wide
        // canvas is drawn at its own resolution or at two thirds of it. Held
        // as an `Option` for the same reason the size starts at 2x2: this is
        // one system that would rather return a frame late than not compile
        // for want of a resource that was always going to be there.
        device: Option<Res<RenderDevice>>,
    ) {
        // Before anything reads it: the footprint can still have moved in
        // `PreStartup` — see the field's own note.
        stage.budget = Self::budget(quality.footprint(), stage.asked);

        // Ahead of the resize, so a rung taken this frame is applied this
        // frame rather than costing a second reallocation on the next one.
        //
        // Held off until the sampling tier has stopped moving. Both
        // controllers read the same median and answer it the same way, so
        // running them together would spend two corrections on one slow
        // stretch — and the tier is the one to spend first: dropping from four
        // samples to one costs nothing at the distance a replay is watched
        // from, where every rung of this ladder costs a little sharpness
        // everywhere.
        if quality.settled() {
            stage.review -= time.delta_secs();
            if stage.review <= 0.0 {
                stage.review = Self::REVIEW;
                let median = cost.typical_frame_ms();
                let missing = (Self::STRUGGLING_MS..Self::STALLED_MS).contains(&median);
                // A comfortable reading and a stalled one both say the last
                // one was not a measurement of a machine that is missing the
                // display, so both put the count back to nothing. See
                // [`Self::CONSECUTIVE`].
                stage.missing = if missing { stage.missing + 1 } else { 0 };
                if stage.missing >= Self::CONSECUTIVE && stage.step + 1 < Self::SCALES.len() {
                    stage.step += 1;
                    // Spent, so the next rung needs its own two readings
                    // rather than walking the whole ladder down on the
                    // strength of one slow stretch.
                    stage.missing = 0;
                    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
                        "match viewer — {median:.0} ms frames; drawing the replay at {:.0}% of the canvas",
                        stage.scale() * 100.0,
                    )));
                }
            }
        }

        let longest_side = device
            .map(|device| device.limits().max_texture_dimension_2d)
            .unwrap_or(Self::GUARANTEED_SIDE);
        let wanted = stage.wanted(&window, longest_side);
        if wanted == stage.size {
            return;
        }
        let Some(mut canvas) = images.get_mut(&stage.canvas) else {
            return;
        };
        canvas.resize(Extent3d {
            width: wanted.x,
            height: wanted.y,
            depth_or_array_layers: 1,
        });
        stage.size = wanted;
        stage.drawn = wanted.x as f32 / window.physical_width().max(1) as f32;
        // Held rather than added: a resize FREES the old attachments, so
        // accumulating would print the sum of every size the stage has ever
        // been. See [`MemoryBill::hold`].
        MemoryBill::hold(
            Held::Stage,
            Self::attachments(
                wanted,
                UVec2::new(window.physical_width(), window.physical_height()),
                match quality.msaa() {
                    Msaa::Sample4 => 4,
                    _ => 1,
                },
            ),
        );
    }
}

impl FromWorld for Stage {
    /// Built through `init_resource` rather than at `Startup`, because
    /// `TvCamera::spawn` needs the handle and startup systems have no order
    /// between them worth relying on. `Assets<Image>` exists from the moment
    /// `DefaultPlugins` is added, which is before this runs.
    fn from_world(world: &mut World) -> Self {
        // One by one until the first `fit`, which is on the first frame: the
        // window's size is not known here — winit has not adopted the canvas
        // yet — and allocating a guess would mean allocating twice.
        let size = UVec2::new(2, 2);
        let mut canvas = Image::new_target_texture(
            size.x,
            size.y,
            // Eight bits a channel, as the camera is not in HDR. Asking for
            // more would double the bandwidth of every attachment in the
            // frame, which is the opposite of the errand.
            TextureFormat::Rgba8UnormSrgb,
            None,
        );
        // Bilinear, so the step back up to the canvas is a soft picture rather
        // than a blocky one. It is also what makes the ladder viable at all:
        // point sampling a target at 75% is visibly a smaller image stretched,
        // where filtering it is simply less sharp.
        canvas.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
        // There is nothing in a render target worth carrying across a resize —
        // the next frame overwrites every pixel of it — and the copy would
        // want a `COPY_SRC` usage that `new_target_texture` does not ask for.
        canvas.copy_on_resize = false;

        // Both read before the image is handed over, because the budget is
        // decided once and for the life of the session: it answers to what
        // KIND of machine this is, not to how the frame is going. `Quality`
        // and the config are both in the world before `init_resource` reaches
        // here — see the order in `MatchViewer::start`.
        let footprint = world.resource::<Quality>().footprint();
        let asked = world.resource::<ViewerConfig>().stage;
        Stage {
            canvas: world.resource_mut::<Assets<Image>>().add(canvas),
            budget: Self::budget(footprint, asked),
            asked,
            step: 0,
            // Deliberately NOT `size`: the image was just built at 2x2 and this
            // says what `fit` has been told about, so leaving them equal would
            // have the first frame decide there was nothing to do.
            size: UVec2::ZERO,
            drawn: Self::SCALES[0],
            missing: 0,
            review: Self::REVIEW,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a desktop part reports, and what every test below assumes unless
    /// it is the one asking what happens when the device is smaller than the
    /// canvas.
    const DESKTOP: u32 = 8192;

    fn stage(step: usize) -> Stage {
        Stage::on(step, Footprint::Roomy)
    }

    /// The same at a chosen footprint, for the two tests that ask what a
    /// phone is handed.
    impl Stage {
        fn on(step: usize, footprint: Footprint) -> Stage {
            Stage {
                canvas: Handle::default(),
                step,
                size: UVec2::ZERO,
                drawn: Stage::SCALES[step],
                missing: 0,
                review: 0.0,
                budget: Stage::budget(footprint, None),
                asked: None,
            }
        }
    }

    fn canvas(width: u32, height: u32) -> Window {
        let mut window = Window::default();
        window.resolution.set_physical_resolution(width, height);
        window
    }

    /// How far the target's shape sits from the canvas's, as a share.
    fn misshapen(window: &Window, target: UVec2) -> f32 {
        let canvas = window.physical_width() as f32 / window.physical_height() as f32;
        let drawn = target.x as f32 / target.y as f32;
        (drawn / canvas - 1.0).abs()
    }

    /// …and how far it is ALLOWED to, which is not a number anybody chose.
    ///
    /// Each side is rounded to an even number of pixels, so the ratio carries
    /// up to a pixel of slack on each of its two terms and no more. Deriving
    /// the bound rather than writing one down is the point: a constant loose
    /// enough for the smallest rung would be loose enough to hide a real
    /// stretch on the largest, and a stretch anybody can see is tens of
    /// percent — two orders of magnitude above the worst this allows.
    fn quantisation(target: UVec2) -> f32 {
        1.0 / target.x as f32 + 1.0 / target.y as f32
    }

    /// The top rung has to be the canvas itself and nothing less. A viewer on
    /// a machine that can afford the full picture must never be handed a
    /// resampled one — every rung below this is a cost being paid for a reason.
    #[test]
    fn the_first_rung_is_the_canvas() {
        assert_eq!(Stage::SCALES[0], 1.0);
        assert_eq!(
            stage(0).wanted(&canvas(1600, 900), DESKTOP),
            UVec2::new(1600, 900)
        );
    }

    /// **The invariant this file exists to keep.** The replay is drawn into
    /// this target and then stretched across the window, so the target has to
    /// be the window's SHAPE at every rung, on every panel, whichever cap is
    /// biting — or players come out too wide.
    ///
    /// The super-ultrawides are the ones that broke it, and they are here by
    /// name: 5120x1440 came back 3840x1440 under a per-axis clamp and drew
    /// everybody a third too wide.
    #[test]
    fn the_target_is_always_the_shape_of_the_canvas() {
        let panels = [
            (1280, 720),  // a laptop
            (1920, 1080), // the common case
            (3840, 2160), // 4K, exactly on the budget
            (7680, 4320), // 4K at 200%, well over it
            (3440, 1440), // ultrawide
            (5120, 1440), // super-ultrawide, under the budget on area
            (5120, 2160), // super-ultrawide at 4K height, over it
            (2560, 2880), // and one taller than it is wide
        ];
        for (width, height) in panels {
            let window = canvas(width, height);
            for step in 0..Stage::SCALES.len() {
                for side in [DESKTOP, 4096, 2048] {
                    let wanted = stage(step).wanted(&window, side);
                    let error = misshapen(&window, wanted);
                    assert!(
                        error <= quantisation(wanted),
                        "{width}x{height} at rung {step} on a {side} device came back \
                         {wanted}, off shape by {:.2}%",
                        error * 100.0
                    );
                    // And the bound itself has to stay somewhere near a pixel.
                    // Without this the assertion above would pass on a target
                    // small enough for its own quantisation to swallow a
                    // visible stretch.
                    assert!(
                        error < 0.01,
                        "{wanted} is off shape by {:.2}%",
                        error * 100.0
                    );
                }
            }
        }
    }

    /// Every rung is smaller than the one above it, on both axes. The ladder
    /// is walked one way and a rung that did not shrink would be a step that
    /// bought nothing and reallocated a render target to do it.
    #[test]
    fn every_rung_is_smaller_than_the_one_above() {
        let window = canvas(1600, 900);
        let mut previous = stage(0).wanted(&window, DESKTOP);
        for step in 1..Stage::SCALES.len() {
            let wanted = stage(step).wanted(&window, DESKTOP);
            assert!(
                wanted.x < previous.x && wanted.y < previous.y,
                "rung {step} came out at {wanted} against {previous}"
            );
            previous = wanted;
        }
    }

    /// Even on both axes, whatever the canvas is. An odd render target makes
    /// the multisample resolve unhappy, and a one-pixel wobble in the canvas
    /// would otherwise reallocate it on alternate frames.
    #[test]
    fn a_target_is_always_an_even_number_of_pixels() {
        for step in 0..Stage::SCALES.len() {
            for (width, height) in [(1601, 901), (1023, 767), (2, 2), (5121, 1441)] {
                let wanted = stage(step).wanted(&canvas(width, height), DESKTOP);
                assert_eq!(wanted.x % 2, 0, "{wanted} is odd across");
                assert_eq!(wanted.y % 2, 0, "{wanted} is odd down");
            }
        }
    }

    /// A canvas with no area yet — the first frame, before winit has adopted
    /// it — must still produce a target something can be rendered into.
    #[test]
    fn a_canvas_with_no_size_still_gets_a_target() {
        let wanted = stage(0).wanted(&canvas(0, 0), DESKTOP);
        assert!(
            wanted.x >= 2 && wanted.y >= 2,
            "{wanted} cannot be rendered into"
        );
    }

    /// And a wall-sized one is cut back, because the attachments behind it are
    /// what actually costs — see [`Stage::BUDGET`].
    #[test]
    fn an_enormous_canvas_is_cut_back_to_the_budget() {
        let wanted = stage(0).wanted(&canvas(7680, 4320), DESKTOP);
        assert!(
            wanted.x * wanted.y <= Stage::BUDGET,
            "{wanted} is over the budget"
        );
        // Cut back to it rather than past it: this is a 4K panel at 200%
        // scaling and it should still be drawing a 4K picture.
        assert_eq!(wanted, UVec2::new(3840, 2160));
    }

    /// A super-ultrawide is a differently shaped frame, not a bigger one, and
    /// must not be charged for pixels it never asked for. 5120x1440 is 7.4
    /// megapixels against a 4K panel's 8.3, so it keeps all of them.
    #[test]
    fn a_super_ultrawide_keeps_its_pixels() {
        assert_eq!(
            stage(0).wanted(&canvas(5120, 1440), DESKTOP),
            UVec2::new(5120, 1440)
        );
    }

    /// Nothing is ever asked of the device that the device has said it cannot
    /// do — and the refusal costs shape on neither axis.
    #[test]
    fn no_side_is_longer_than_the_device_allows() {
        for side in [2048, 4096] {
            let window = canvas(5120, 1440);
            let wanted = stage(0).wanted(&window, side);
            assert!(
                wanted.x <= side && wanted.y <= side,
                "{wanted} is longer than the {side} this device allows"
            );
            assert!(misshapen(&window, wanted) <= quantisation(wanted));
        }
    }

    /// **The budget that never fired on the device it was for.**
    ///
    /// An iPad in landscape is about 1180x664 CSS pixels at `devicePixelRatio`
    /// 2 — 2360x1328, or 3.1 megapixels — which is comfortably under the
    /// desktop cap, so a phone got every pixel of its canvas and the
    /// attachments behind them. The handheld figure is the same cap asked of
    /// the enclosure rather than of a 4K panel.
    #[test]
    fn a_handheld_is_held_to_its_own_budget() {
        let window = canvas(2360, 1328);
        let roomy = Stage::on(0, Footprint::Roomy).wanted(&window, DESKTOP);
        assert_eq!(roomy, UVec2::new(2360, 1328), "a computer keeps its canvas");

        let handheld = Stage::on(0, Footprint::Handheld).wanted(&window, DESKTOP);
        // Plus the rounding, derived rather than written down — the same
        // argument [`quantisation`] carries. Each side is rounded to the
        // NEAREST even pixel rather than down, deliberately (see
        // [`Stage::wanted`]), so each can stand a pixel above the exact
        // figure and the area a row and a column above the budget. Two parts
        // in a thousand, against a cap whose whole job is to be the right
        // order of magnitude.
        let rounding = handheld.x + handheld.y + 1;
        assert!(
            handheld.x * handheld.y <= Stage::HANDHELD_BUDGET + rounding,
            "{handheld} is over what a handheld may hold"
        );
        // …and it is a smaller picture, not a wrong one. The one invariant
        // this whole file exists for: one shrink factor, both axes.
        assert!(
            misshapen(&window, handheld) <= quantisation(handheld),
            "{handheld} is not the shape of the window"
        );
    }

    /// A small canvas is not cut at all, whatever kind of machine it is on.
    /// An iPhone in portrait is well under the handheld budget and should be
    /// drawn at its own resolution.
    #[test]
    fn a_handheld_budget_does_not_bite_a_phone_sized_canvas() {
        let window = canvas(780, 1200);
        assert_eq!(
            Stage::on(0, Footprint::Handheld).wanted(&window, DESKTOP),
            UVec2::new(780, 1200)
        );
    }

    /// `?stage=` overrides both, and nonsense in it falls through to the
    /// answer the device would have had — the same way every other override in
    /// this crate declines to be given nonsense.
    #[test]
    fn the_page_can_name_its_own_budget() {
        assert_eq!(Stage::budget(Footprint::Roomy, Some(2.0)), 2_000_000);
        assert_eq!(
            Stage::budget(Footprint::Handheld, Some(0.0)),
            Stage::HANDHELD_BUDGET
        );
        assert_eq!(
            Stage::budget(Footprint::Handheld, Some(-1.0)),
            Stage::HANDHELD_BUDGET
        );
        assert_eq!(Stage::budget(Footprint::Roomy, None), Stage::BUDGET);
    }

    /// The attachment arithmetic, against the figure the memory bill for this
    /// scene was written from: a 1.3 MP stage inside a 3.1 MP canvas, one
    /// sample, comes to some 63 MB — and four samples on the same stage is
    /// half as much again on the replay's side alone.
    #[test]
    fn the_attachments_are_counted_at_their_sample_count() {
        let (target, window) = (UVec2::new(1524, 858), UVec2::new(2360, 1328));
        let one = Stage::attachments(target, window, 1);
        let four = Stage::attachments(target, window, 4);
        // Colour pair, depth and the stage image at one sample.
        assert_eq!(one, 1524 * 858 * 4 * 4 + 2360 * 1328 * 4 * 3);
        // Four samples adds the resolve target and thickens the depth: seven
        // more target-sized surfaces.
        assert_eq!(four - one, 1524 * 858 * 4 * 7);
    }
}
