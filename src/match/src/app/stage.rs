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
use crate::app::quality::Quality;
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
    /// unless [`Self::CEILING`] or the device had to cut into it. Kept only so
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

    /// **The most memory the picture may cost**, in bytes, counting every
    /// attachment behind it.
    ///
    /// Not a quality limit — a memory one, and it is stated in BYTES because
    /// bytes are what a browser kills a tab for. It used to be an area in
    /// pixels, and an area cannot express this cost: the sample count
    /// multiplies the bytes without touching the pixels, so one pixel budget
    /// bought a 16-byte-a-pixel frame on one machine and a 44-byte-a-pixel
    /// frame on the next. See [`Self::planes`], which is the factor the old
    /// figure could not see.
    ///
    /// It covers BOTH views, which the pixel budget also could not. The
    /// replay's target was the only thing it clamped, and on a handheld the
    /// replay's target is the smaller half — the window camera's own colour
    /// pair and depth are taken at the canvas's full size whatever the replay
    /// is drawn at, and on a phone in portrait they are two thirds of the
    /// bill. A ceiling that could only reach the smaller half is why the
    /// clamp added for iOS took 0% off a phone held upright.
    ///
    /// **One figure, and no device term in it.** That is not an oversight: the
    /// canvas is already the device. A phone's canvas is three megapixels and
    /// a desktop's is eight, so the same ceiling leaves the phone the larger
    /// share for its replay and holds the desktop to something a tab can
    /// survive — without anything having to guess which is which, and without
    /// the guess being wrong on a Mac. 128 MiB is comfortably above what the
    /// scene wants at one sample on every canvas measured (a 16-inch Retina
    /// display fullscreen comes to 123 MiB) and comfortably below where any
    /// engine has been seen to fall over.
    ///
    /// Where the canvas alone is larger than this — a 5K panel fullscreen
    /// spends 177 MiB on the window camera before the replay asks for
    /// anything — there is nothing left to give and the replay falls to the
    /// bottom rung. That is the honest answer: the two surfaces the window
    /// camera needs are not this file's to decline.
    pub(crate) const CEILING: usize = 128 * 1024 * 1024;

    /// **How many full-resolution planes the replay's view costs**, per pixel
    /// of the target, at a given sample count.
    ///
    /// The colour pair, the resolve target where there is something to
    /// resolve, the depth buffer at the colour's sample count, and the stage
    /// image the whole thing is resolved into. Four at one sample and eleven
    /// at four — which is the arithmetic that makes multisampling a memory
    /// decision rather than a picture one, and the number [`Self::CEILING`]
    /// exists to be divided by.
    fn planes(samples: u32) -> usize {
        let samples = samples.max(1) as usize;
        let resolved = if samples > 1 { samples } else { 0 };
        3 + resolved + samples
    }

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

    /// **The most pixels the replay may be drawn into**, in area.
    ///
    /// Derived rather than declared: [`Self::CEILING`] is the bill, the window
    /// camera's own surfaces are subtracted from it because nothing here can
    /// decline them, and what is left is divided by what a pixel of the target
    /// costs at the sample count in force. One rule, and it answers every
    /// machine — a phone, a tablet, a laptop, a desktop and any of them
    /// fullscreen — without being told which it is looking at.
    ///
    /// ⚠ **It answers to the SAMPLE COUNT, which the figure it replaced could
    /// not.** Four samples cost eleven planes a pixel against one sample's
    /// four, so the same ceiling buys 36% of the area at four that it buys at
    /// one. That is the exchange rate multisampling has always had; it was
    /// simply not being charged, and a Retina Mac was quietly spending 412 MiB
    /// on it.
    ///
    /// `?stage=<megapixels>` overrides it, alongside `?device=` and `?crowd=`:
    /// the attachments are the largest allocation in this scene that is not
    /// geometry, and a device that reloads its tab rather than reporting
    /// anything can only be bisected from its address bar. Anything
    /// unreadable, zero or negative falls through to the answer the device
    /// would have had, which is the same way every other override in this
    /// crate declines to be given nonsense.
    pub fn budget(window: UVec2, samples: u32, asked: Option<f32>) -> u32 {
        if let Some(megapixels) = asked.filter(|megapixels| *megapixels > 0.0) {
            return (megapixels * 1_000_000.0) as u32;
        }
        // The window camera's colour pair and depth, which are taken at the
        // canvas's own size whatever the replay is drawn at.
        let taken = window.x as usize * window.y as usize * 12;
        let left = Self::CEILING.saturating_sub(taken);
        (left / (4 * Self::planes(samples))) as u32
    }

    /// Whether this canvas can afford to be multisampled at all.
    ///
    /// Read by [`Quality`](crate::app::quality::Quality) before the first
    /// frame, and it is the whole of that decision now. Four samples are worth
    /// having when they cost nothing but bandwidth; they are not worth having
    /// when the only way to pay for them is to draw the match at three fifths
    /// of the size, which is the trade [`Self::budget`] would otherwise make
    /// silently. The module note at the top of this file is the argument:
    /// resolution is the better of the two, so where both cannot be had, the
    /// samples go.
    pub fn affords_multisampling(canvas: UVec2) -> bool {
        Self::attachments(canvas, canvas, 4) <= Self::CEILING
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
        // The colour pair, the resolve target where there is something to
        // resolve, the depth buffer, and the stage image — see
        // [`Self::planes`], which is the same count [`Self::budget`] divides
        // by. Stated once so the price and the ceiling cannot drift apart.
        let replay = pixels(target) * Self::planes(samples);
        // The window's pair and its own depth, always at one sample.
        let canvas = pixels(window) * 3;
        replay + canvas
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
        // …but never past the bottom of the ladder, whatever the budget says.
        //
        // The window camera's own surfaces come out of the ceiling before the
        // replay is given anything, and on a large enough canvas they take all
        // of it — a 5K panel fullscreen spends 169 MiB on them alone. Without
        // this the allowance goes to zero there and the replay is drawn into a
        // two-pixel image: not a soft picture, a broken one.
        //
        // The floor is the ladder's own last rung, and for the reason already
        // written against [`Self::SCALES`] — below about this the replay stops
        // reading as a soft picture and starts reading as a small one, so
        // there is nothing to be bought below it. A canvas that cannot fit its
        // own bottom rung under the ceiling is a canvas where the ceiling has
        // run out of things to decline, and the honest answer is to say so in
        // the bill rather than to draw a postage stamp.
        shrink = shrink.max(Self::SCALES[Self::SCALES.len() - 1]);
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
    pub(crate) fn measured(window: UVec2, samples: u32) -> UVec2 {
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
            budget: Self::budget(window, samples, None),
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
        let canvas = UVec2::new(window.physical_width(), window.physical_height());
        let samples = match quality.msaa() {
            Msaa::Sample4 => 4,
            _ => 1,
        };
        stage.budget = Self::budget(canvas, samples, stage.asked);

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
        // **And nothing worth holding on the CPU either.** `new_target_texture`
        // fills a zeroed RGBA `Vec` the size of the whole image, and
        // `RenderAssetUsages::default()` is `MAIN_WORLD | RENDER_WORLD`, so
        // that buffer is not dropped after extraction — it is kept, resized on
        // every rung of the ladder and every canvas change, and CLONED whole
        // into the render world each time, for a texture no code on this side
        // ever reads. At a 16-inch Retina canvas that is 30 MB held and 30 MB
        // copied, on wasm32, where the module note in [`MemoryBill`] says the
        // peak is permanent.
        //
        // Taking the data away rather than the usage: the image has to stay in
        // the main world, because `fit` reaches for it by handle to resize it.
        // With no data `Image::resize` only writes the descriptor, and
        // `GpuImage::prepare_asset` takes its `create_texture` branch and
        // uploads nothing — which is what a render target wanted in the first
        // place.
        canvas.data = None;

        // Both read before the image is handed over, because the budget is
        // decided once and for the life of the session: it answers to what
        // KIND of machine this is, not to how the frame is going. `Quality`
        // and the config are both in the world before `init_resource` reaches
        // here — see the order in `MatchViewer::start`.
        let samples = match world.resource::<Quality>().msaa() {
            Msaa::Sample4 => 4,
            _ => 1,
        };
        let asked = world.resource::<ViewerConfig>().stage;
        Stage {
            canvas: world.resource_mut::<Assets<Image>>().add(canvas),
            // The window has not been adopted yet, so this is the ceiling
            // with nothing taken out of it for the canvas. `fit` re-derives it
            // against the real one on the first frame, before the first
            // allocation — see the field's own note.
            budget: Self::budget(UVec2::ZERO, samples, asked),
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

    /// A stage at one sample, which is what the ceiling leaves every canvas
    /// large enough to be worth testing.
    fn stage(step: usize) -> Stage {
        Stage::on(step, UVec2::ZERO, 1)
    }

    /// The same against a named canvas and sample count, for the tests that
    /// ask what the ceiling leaves once the window camera has been paid for.
    impl Stage {
        fn on(step: usize, window: UVec2, samples: u32) -> Stage {
            Stage {
                canvas: Handle::default(),
                step,
                size: UVec2::ZERO,
                drawn: Stage::SCALES[step],
                missing: 0,
                review: 0.0,
                budget: Stage::budget(window, samples, None),
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
    /// what actually costs — see [`Stage::CEILING`].
    ///
    /// 7680x4320 is the case where the ceiling has nothing left to decline:
    /// the window camera's own three surfaces come to 398 MB before the replay
    /// is offered anything, so what holds here is the ladder's bottom rung and
    /// not the ceiling. That is the honest answer rather than a failure — see
    /// the floor in [`Stage::wanted`] — and the thing worth asserting is that
    /// the cut happened at all and did not bend the picture.
    #[test]
    fn an_enormous_canvas_is_cut_back_to_the_bottom_rung() {
        let window = canvas(7680, 4320);
        let size = UVec2::new(7680, 4320);
        let wanted = Stage::on(0, size, 1).wanted(&window, DESKTOP);
        let rung = Stage::SCALES[Stage::SCALES.len() - 1];
        assert_eq!(
            wanted,
            UVec2::new(
                ((size.x as f32 * rung) / 2.0).round() as u32 * 2,
                ((size.y as f32 * rung) / 2.0).round() as u32 * 2,
            ),
            "a canvas this large should sit on the bottom rung"
        );
        assert!(misshapen(&window, wanted) <= quantisation(wanted));
    }

    /// **The whole bill, on every canvas, at whatever sampling it is given.**
    ///
    /// This is the invariant the pixel budget it replaced could not state. It
    /// clamped the replay's target alone, so a phone in portrait was cut by 0%
    /// and a Retina Mac by nothing at all — the window camera's own surfaces
    /// were never in the figure, and neither was the sample count that
    /// multiplies it. Every one of these canvases is a real device: a phone in
    /// portrait, an iPad in landscape, the same two with the browser chrome
    /// gone, a Retina laptop in the page and fullscreen, and a 5K panel whose
    /// canvas alone is over the ceiling.
    #[test]
    fn every_canvas_is_held_to_the_ceiling() {
        for (width, height) in [
            (780, 1200),   // iPhone in portrait, in the page
            (1179, 2556),  // iPhone, chrome gone
            (2360, 1328),  // iPad in landscape
            (2732, 2048),  // iPad Pro, chrome gone
            (2702, 1520),  // 14-inch Retina laptop, in the page
            (3456, 2234),  // 16-inch Retina laptop, fullscreen
            (3840, 2160),  // a 4K panel
            (5120, 2880),  // a 5K panel — the canvas alone is over the ceiling
        ] {
            let window = canvas(width, height);
            let size = UVec2::new(width, height);
            for samples in [1, 4] {
                let wanted = Stage::on(0, size, samples).wanted(&window, DESKTOP);
                let held = Stage::attachments(wanted, size, samples);
                // Two things the ceiling may not override, so the bound is
                // the larger of it and them: the window camera's own three
                // surfaces, which this file cannot decline, and the ladder's
                // bottom rung, below which the replay would be a broken
                // picture rather than a soft one.
                let rung = Stage::SCALES[Stage::SCALES.len() - 1];
                let bottom = UVec2::new(
                    (size.x as f32 * rung) as u32,
                    (size.y as f32 * rung) as u32,
                );
                let floor = Stage::attachments(bottom, size, samples);
                assert!(
                    held <= Stage::CEILING.max(floor) + Stage::CEILING / 64,
                    "{width}x{height} at {samples} sample(s) holds {} MiB",
                    held / (1024 * 1024)
                );
                assert!(
                    misshapen(&window, wanted) <= quantisation(wanted),
                    "{wanted} is not the shape of the {width}x{height} window"
                );
            }
        }
    }

    /// Four samples cost eleven planes a pixel against one sample's four, so
    /// the same ceiling buys a visibly smaller picture — which is the trade
    /// [`Stage::affords_multisampling`] exists to decline on a large canvas
    /// and to allow on a small one.
    #[test]
    fn multisampling_is_priced_and_declined_where_it_cannot_be_paid_for() {
        let window = canvas(2702, 1520);
        let size = UVec2::new(2702, 1520);
        let one = Stage::on(0, size, 1).wanted(&window, DESKTOP);
        let four = Stage::on(0, size, 4).wanted(&window, DESKTOP);
        assert!(
            four.x * four.y < one.x * one.y,
            "four samples came out no smaller than one: {four} against {one}"
        );
        // A laptop-sized canvas cannot pay for it; a windowed stage on a
        // desktop can, and keeps it.
        assert!(!Stage::affords_multisampling(size));
        assert!(Stage::affords_multisampling(UVec2::new(1600, 900)));
    }

    /// A small canvas is not cut at all. An iPhone in portrait is far under
    /// the ceiling and should be drawn at its own resolution.
    #[test]
    fn a_phone_sized_canvas_is_not_cut(){
        let window = canvas(780, 1200);
        assert_eq!(
            Stage::on(0, UVec2::new(780, 1200), 1).wanted(&window, DESKTOP),
            UVec2::new(780, 1200)
        );
    }

    /// `?stage=` overrides it, and nonsense in it falls through to the answer
    /// the device would have had — the same way every other override in this
    /// crate declines to be given nonsense.
    #[test]
    fn the_page_can_name_its_own_budget() {
        let phone = UVec2::new(1179, 2556);
        assert_eq!(Stage::budget(phone, 1, Some(2.0)), 2_000_000);
        assert_eq!(
            Stage::budget(phone, 1, Some(0.0)),
            Stage::budget(phone, 1, None)
        );
        assert_eq!(
            Stage::budget(phone, 1, Some(-1.0)),
            Stage::budget(phone, 1, None)
        );
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
