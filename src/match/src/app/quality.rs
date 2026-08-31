//! What the machine on the other end can afford to draw.
//!
//! The replay was tuned on a desktop GPU, where the scene is bound by how many
//! things are submitted and not by how many pixels they cover — `Pitch` records
//! the measurement: the same 3.9 ms at 1280x720 and at 3840x2160. That result
//! does not travel. An Intel or AMD integrated part has no memory of its own:
//! every attachment write, every texture fetch and every resolve goes across
//! the same bus the CPU is using, at a fifth to a tenth of the bandwidth a
//! discrete card has to itself. On one of those the scene stops being bound per
//! entity and starts being bound per SAMPLE, and the two costs this file exists
//! to control are the two largest of those:
//!
//! - **Multisampling.** Bevy's default is four samples and nothing here ever
//!   said otherwise. On WebGL2 that means the whole 3D pass renders into a
//!   4x-sampled colour target and a 4x-sampled depth target and is then
//!   resolved down — four times the attachment traffic of the frame, plus a
//!   full-screen resolve, every frame. At 1600x900 that is some 46 MB of
//!   attachment bandwidth per frame against the ~15 GB/s an integrated part
//!   actually has. It is invisible on the machine this was written on, where
//!   the tile memory resolves for free.
//!
//! - **What replaces it.** Turning multisampling off and leaving nothing in its
//!   place is not an option: this scene is white paint on dark grass, stepped
//!   concrete against sky and a goal net — thin, high-contrast geometry is most
//!   of what is in frame, and unantialiased it crawls as the camera pans, which
//!   is exactly the complaint. FXAA is a single full-screen pass that reads the
//!   already-resolved image; it costs a fraction of a millisecond and it cannot
//!   recover an edge the rasteriser never sampled, but on edges it CAN see it
//!   is most of the way to four samples for a twentieth of the bandwidth.
//!
//! So the tier is not "pretty" against "fast" — both tiers are antialiased, and
//! the picture the reduced one draws differs by softer coverage on edges, not
//! by anything missing from the scene. Nothing is culled, nothing is turned
//! off, no distance is cut short.
//!
//! ## Deciding which one
//!
//! Two mechanisms, because neither is sufficient alone.
//!
//! **The probe** asks the browser what it is running on, before the first frame
//! and before the camera is built, so a machine that is known to be integrated
//! never renders a single 4x frame and never has to change its mind. Chrome and
//! Edge answer; Firefox answers depending on a preference and Safari removed
//! the question altogether. So it is an optimisation, not the mechanism.
//!
//! **The measurement** is the mechanism. [`FrameCost`] is already keeping a
//! two-second median of the whole frame for the debug overlay; if that median
//! is still bad once the load has settled, the tier steps down. Once, and only
//! downward: changing the sample count re-specialises every render pipeline,
//! which on WebGL2 means the driver recompiles every shader in the scene — a
//! visible hitch. One hitch to buy the rest of the match is a good trade; a
//! controller that hunts between two tiers is worse than either of them.

use crate::app::perf::FrameCost;
use bevy::anti_alias::fxaa::{Fxaa, Sensitivity};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::render::view::Msaa;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext, WebglDebugRendererInfo};

/// How much sampling the frame can afford.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Four samples per pixel, resolved by the hardware. What a discrete card
    /// draws.
    Multisampled,
    /// One sample per pixel plus a full-screen FXAA pass. What an integrated
    /// part draws, and what a machine that measures badly is moved to.
    PostProcessed,
}

/// The chosen tier, and whether it is still open to being lowered.
#[derive(Resource)]
pub struct Quality {
    tier: Tier,
    /// Set once the tier has been lowered by measurement, or once the window
    /// in which that may happen has closed. A tier is never raised.
    settled: bool,
    /// When the scene finished being built and the recording started playing.
    /// The measurement is not consulted before [`Self::SETTLE_AFTER`] past
    /// this, because the frames either side of the first chunk arriving are
    /// not the frames this is trying to judge — and neither are the ones the
    /// browser spent compiling shaders. Written once by [`Self::start`].
    started: Option<Instant>,
    /// Consecutive readings that have said the machine is struggling, and when
    /// the last of them was taken. See [`Self::CONSECUTIVE`] — the count is
    /// what tells a slow machine from a stall, and it resets the moment a
    /// reading comes back comfortable.
    struggling: u32,
    reviewed: Option<Instant>,
}

impl Quality {
    /// Frame time, in milliseconds, above which the multisampled tier is
    /// judged unaffordable.
    ///
    /// Sixty frames a second is 16.7 ms and the browser will hold the app to
    /// whatever the display does, so a machine COMFORTABLY making the refresh
    /// rate reads a shade under it. This sits well past that: at 24 ms the
    /// display is being missed roughly every other frame, which is the
    /// juddering the eye reads as "not smooth" rather than as "slow". Set
    /// tighter and a 60 Hz machine that is merely at its limit gets a shader
    /// recompile it did not need.
    const STRUGGLING_MS: f32 = 24.0;

    /// How long to let the app settle before the measurement is believed.
    ///
    /// The first seconds are not representative and never will be: two
    /// 1024-square turf sheets and their mip chains are generated on this
    /// thread, twenty-two faces are painted, the first chunk of the recording
    /// lands as one JSON document and is parsed here. `FrameCost`'s window is
    /// two seconds wide, so this has to be at least that much past the last of
    /// them or the median is measuring the load and not the scene.
    ///
    /// ⚠ Measured from [`Self::start`], which is the end of the bring-up and
    /// NOT the first frame. It used to be the first frame, and that was wrong
    /// by an order of magnitude: opening a match compiles four PBR shaders and
    /// the browser blocks four to six seconds inside each of them, so by the
    /// time this clock read six the frame window held nothing but those
    /// stalls. An RTX 3080 Ti measured 440 ms "typical" frames and docked
    /// itself to one sample for the rest of the match — the exact false
    /// positive the module note says must not happen, fired by every discrete
    /// card there is.
    const SETTLE_AFTER: f32 = 6.0;

    /// And how long the door stays open. Past this the tier is what it is: a
    /// pipeline recompile in the ninetieth minute is a stutter with nothing to
    /// show for it, since whatever it would have fixed has been survived for
    /// eighty-nine of them.
    const SETTLE_BEFORE: f32 = 20.0;

    /// The ceiling on believing the measurement at all — the same number, and
    /// the same argument, as [`Stage::STALLED_MS`](crate::app::stage::Stage):
    /// past this the page is not slow, it is STALLED. A median in the
    /// hundreds of milliseconds is a shader link that slipped past the
    /// bring-up, a backgrounded tab, a breakpoint — and none of those is a
    /// sampling problem: taking three samples in four away turns 300 ms into
    /// 300 ms. The tier is one-way, so answering a stall here docks a machine
    /// for the whole match.
    ///
    /// Not hypothetical. The squad's own shader link lands wherever the
    /// recording's arrival puts it, and measured on 2026-08-30 it landed at
    /// twenty-two seconds once in nine openings — inside this window, filling
    /// the frame history with 291 ms "frames" — and an RTX 3080 Ti spent the
    /// match at one sample + FXAA. The module note's exact false positive,
    /// back in through a second door.
    ///
    /// ⚠ **It was 100 ms, and at 100 ms it switched this mechanism off on
    /// exactly the machines it exists for.** A part genuinely managing six to
    /// nine frames a second sits at 110–170 ms, which was read as "stalled" —
    /// so a viewer whose replay was unwatchable got neither this nor a single
    /// rung of the resolution ladder, and both controllers sat there declining
    /// to act for the whole match. Reported 2026-09-01 on an integrated
    /// Radeon.
    ///
    /// The ceiling is right and the number was wrong, because MAGNITUDE is the
    /// wrong test. What separates a stall from a slow machine is that a stall
    /// ENDS: a shader link, a backgrounded tab and a breakpoint are all
    /// transient, and a two-compute-unit GPU is not. So the test is
    /// persistence — see [`Self::CONSECUTIVE`] — and with a second reading
    /// required this can go up to where it only ever catches a page that has
    /// genuinely stopped. Half a second a frame is two frames of animation in
    /// a second; nothing anybody would sit through, and far above any real
    /// frame rate the ladder should be answering.
    const STALLED_MS: f32 = 500.0;

    /// How many consecutive readings have to say the machine is struggling
    /// before the tier moves.
    ///
    /// This is what replaces the magnitude test above, and it is the honest
    /// discriminator: [`FrameCost`]'s window is two seconds wide, so two
    /// readings that both say "struggling" cannot both be the same four-second
    /// shader link unless the link is still going — and a machine that is
    /// merely slow says it every time it is asked. Two, not three: the whole
    /// window this may act in is fourteen seconds wide, and the tier is worth
    /// less the later it lands.
    const CONSECUTIVE: u32 = 2;

    /// How long between two readings, in seconds. The same figure and the same
    /// argument as [`Stage::REVIEW`](crate::app::stage::Stage): `FrameCost`'s
    /// own window is two seconds wide, so anything shorter is asking the same
    /// question twice and would let one slow stretch answer
    /// [`Self::CONSECUTIVE`] on its own.
    const REVIEW: f32 = 2.5;

    /// Asks the browser what it is drawing with, and picks a starting tier.
    ///
    /// Called before the app is built rather than from a system, so the camera
    /// can be spawned already carrying the answer — a tier decided on the first
    /// frame is a tier that costs nothing to adopt.
    pub fn probe() -> Self {
        let tier = match Self::renderer() {
            Some(name) => {
                let integrated = Self::is_integrated(&name);
                web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "match viewer — renderer: {name} ({})",
                    if integrated {
                        "integrated, one sample + FXAA"
                    } else {
                        "discrete, four samples"
                    }
                )));
                if integrated {
                    Tier::PostProcessed
                } else {
                    Tier::Multisampled
                }
            }
            // Nothing to go on. Start where the picture is best and let the
            // measurement move it — which is the right way round: a machine
            // that can afford four samples must not be docked them because its
            // browser declined to introduce itself.
            None => Tier::Multisampled,
        };

        Quality {
            tier,
            settled: tier == Tier::PostProcessed,
            started: None,
            struggling: 0,
            reviewed: None,
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// **Checks the probe's guess against the adapter that is actually going
    /// to draw**, and corrects it before anything is built.
    ///
    /// [`Self::probe`] has to run before the app exists, so it asks a canvas
    /// of its own — and a canvas of its own is not the canvas wgpu got. The
    /// two now ask for the same power preference (see [`Self::renderer`]), but
    /// "the same request" is not "the same answer": the browser is free to
    /// place two contexts on two adapters, and on a desktop with a discrete
    /// card beside an integrated one it sometimes does.
    ///
    /// `RenderAdapterInfo` is the real answer — the adapter wgpu opened, named
    /// by the backend itself. Bevy's `RenderPlugin::finish` puts it in the main
    /// world before any schedule runs, which is what makes this correction
    /// possible at all: registered in `PreStartup`, it lands before
    /// [`TvCamera::spawn`](crate::broadcast::camera::TvCamera::spawn) reads the
    /// tier and before [`BodyParts`](crate::players::body::BodyParts) is cut,
    /// so a machine that was guessed wrong is simply built right.
    ///
    /// ⚠ **Correcting it here rather than leaving it to [`Self::relent`] is
    /// the whole value.** Relenting changes the sample count, and changing the
    /// sample count re-specialises every render pipeline in the scene — which
    /// on this backend is the four-to-six-seconds-per-program shader link that
    /// `bringup` exists to keep behind the loading overlay, happening again,
    /// in the middle of the football. A tier that is right before the first
    /// frame costs nothing to adopt.
    ///
    /// It only ever moves DOWNWARD, like every other decision in this file: if
    /// wgpu names an integrated part the tier drops and settles. A card named
    /// here does not raise a tier the probe already lowered — the probe's
    /// false positives are the cheap ones, and a machine that has already
    /// started cutting a coarse figure would have to be rebuilt to undo it.
    pub fn confirm(mut quality: ResMut<Quality>, adapter: Option<Res<RenderAdapterInfo>>) {
        let Some(adapter) = adapter else {
            return;
        };
        let name = adapter.name.clone();
        let integrated = Self::is_integrated(&name);
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
            "match viewer — wgpu opened: {name} ({})",
            if integrated { "integrated" } else { "discrete" },
        )));
        if !integrated || quality.tier == Tier::PostProcessed {
            return;
        }
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(
            "match viewer — the probe read a different adapter; \
             dropping to one sample + FXAA before the first frame",
        ));
        quality.tier = Tier::PostProcessed;
        quality.settled = true;
    }

    /// Whether the tier has stopped moving.
    ///
    /// Read by [`crate::app::stage::Stage::fit`], which holds its own ladder
    /// until this is true so that one slow stretch does not get answered twice.
    pub fn settled(&self) -> bool {
        self.settled
    }

    /// The sample count this tier renders at.
    ///
    /// Only 1 and 4 exist on the web — WebGL2 offers no two-sample target —
    /// so there is no middle rung to stand on and the two tiers are the whole
    /// ladder.
    pub fn msaa(&self) -> Msaa {
        match self.tier {
            Tier::Multisampled => Msaa::Sample4,
            Tier::PostProcessed => Msaa::Off,
        }
    }

    /// The post-process pass that stands in for those samples.
    ///
    /// `Low` sensitivity deliberately, against Bevy's own `High` default. FXAA
    /// finds edges by local contrast, and the two things in this scene with the
    /// most local contrast in them are the mown stripes and the blades of the
    /// turf sheet itself — neither of which is an edge. At `High` the pitch
    /// smears; at `Low` the pass keeps to the goal net, the paint and the
    /// silhouettes, which is what it is here for.
    pub fn fxaa(&self) -> Fxaa {
        Fxaa {
            enabled: matches!(self.tier, Tier::PostProcessed),
            edge_threshold: Sensitivity::Low,
            edge_threshold_min: Sensitivity::Low,
        }
    }

    /// Starts the clock the measurement runs against.
    ///
    /// Called once, by [`crate::app::bringup::Bringup`], when the stadium is up
    /// and the recording is playing — which is the first moment a frame time
    /// means what this file thinks it means.
    pub fn start(&mut self) {
        self.started.get_or_insert_with(Instant::now);
    }

    /// Lowers the tier if the frame says it has to be lowered.
    ///
    /// Runs every frame and does nothing on almost all of them: the whole body
    /// is behind a `settled` flag that is set the first time either branch
    /// fires. See the module note for why this only ever goes one way.
    pub fn relent(
        mut quality: ResMut<Quality>,
        cost: Res<FrameCost>,
        mut camera: Single<(&mut Msaa, &mut Fxaa), With<Camera3d>>,
    ) {
        if quality.settled {
            return;
        }
        // Nothing to judge until there is football on the screen. `start` is
        // called by the bring-up when the last course is laid; until then the
        // frame window is full of shader compiles, which no render tier would
        // have helped with and which are over by the time this reads them.
        let Some(started) = quality.started else {
            return;
        };
        let now = Instant::now();
        let age = (now - started).as_secs_f32();
        if age < Self::SETTLE_AFTER {
            return;
        }
        if age > Self::SETTLE_BEFORE {
            quality.settled = true;
            return;
        }
        // One reading per window, not one per frame. `FrameCost`'s median is
        // two seconds wide, so asking it sixty times a second counts the same
        // two seconds sixty times over and [`Self::CONSECUTIVE`] would be
        // satisfied by two frames rather than by two measurements — which is
        // the whole thing it exists to rule out.
        let due = quality
            .reviewed
            .is_none_or(|last| (now - last).as_secs_f32() >= Self::REVIEW);
        if !due {
            return;
        }
        quality.reviewed = Some(now);

        // Between struggling and stalled, exactly as the resolution ladder
        // reads the same figure: below the band the machine is fine, above it
        // the page is not slow but STOPPED and no tier would help — see
        // [`Self::STALLED_MS`], which is where the argument for both bounds
        // is. Either way the count goes back to nothing: a comfortable
        // reading and a stalled one are both evidence that the last reading
        // was not a measurement of a struggling machine.
        if !(Self::STRUGGLING_MS..Self::STALLED_MS).contains(&cost.typical_frame_ms()) {
            quality.struggling = 0;
            return;
        }
        quality.struggling += 1;
        if quality.struggling < Self::CONSECUTIVE {
            return;
        }

        quality.tier = Tier::PostProcessed;
        quality.settled = true;
        let (msaa, fxaa) = &mut *camera;
        **msaa = quality.msaa();
        **fxaa = quality.fxaa();
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!(
            "match viewer — {:.0} ms frames at four samples; dropping to one sample + FXAA",
            cost.typical_frame_ms(),
        )));
    }

    /// What the browser says it is drawing with, where it will say.
    ///
    /// A throwaway canvas rather than the viewer's own: this runs before winit
    /// has adopted the real one, and taking a context on a canvas that is about
    /// to be handed to wgpu is how you get a canvas wgpu cannot have. The
    /// element is never added to the document, so it is collected as soon as
    /// this returns.
    ///
    /// ⚠ **It must ask for the context on the same terms wgpu will**, and it
    /// used to ask on different ones. A bare `getContext('webgl2')` carries
    /// `powerPreference: "default"`, where Bevy hands wgpu its canvas with
    /// `PowerPreference::HighPerformance` — and on a machine with two adapters
    /// in it a browser is entitled to answer those two requests with two
    /// different GPUs. That is the whole purpose of the preference. Asked the
    /// weaker question on a desktop holding a discrete card beside an
    /// integrated one, this can read the card, start the viewer at four
    /// samples, and leave a machine that is drawing on the iGPU to be rescued
    /// by [`Self::relent`] — which then pays for the correction with a
    /// recompile of every shader in the scene, in play. Reported 2026-09-01 on
    /// exactly that hardware.
    fn renderer() -> Option<String> {
        let canvas: HtmlCanvasElement = web_sys::window()?
            .document()?
            .create_element("canvas")
            .ok()?
            .dyn_into()
            .ok()?;
        let attributes = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &attributes,
            &wasm_bindgen::JsValue::from_str("powerPreference"),
            &wasm_bindgen::JsValue::from_str("high-performance"),
        );
        let context: WebGl2RenderingContext = canvas
            .get_context_with_context_options("webgl2", &attributes)
            .ok()??
            .dyn_into()
            .ok()?;
        // The extension has to be asked for before the parameter it defines can
        // be read, and asking for it is where a browser that declines to answer
        // declines.
        context.get_extension("WEBGL_debug_renderer_info").ok()??;
        context
            .get_parameter(WebglDebugRendererInfo::UNMASKED_RENDERER_WEBGL)
            .ok()?
            .as_string()
    }

    /// Whether a renderer string names a part that shares its memory with the
    /// CPU.
    ///
    /// Deliberately a POSITIVE test, and deliberately conservative. Everything
    /// this cannot name starts at four samples and is moved down by measurement
    /// if it turns out to need it; the cost of a false positive is a machine
    /// drawing a softer picture than it had to for a whole match, which nothing
    /// downstream will ever correct.
    ///
    /// The strings themselves come out of ANGLE and read like
    /// `ANGLE (Intel, Intel(R) UHD Graphics 620 Direct3D11 vs_5_0 ps_5_0, D3D11)`.
    fn is_integrated(renderer: &str) -> bool {
        let name = renderer.to_ascii_lowercase();

        // No hardware at all. Both of these draw perfectly correct frames at
        // perhaps five a second, and four samples is the last thing they need.
        if name.contains("swiftshader")
            || name.contains("llvmpipe")
            || name.contains("basic render")
            || name.contains("software")
        {
            return true;
        }

        // Every Intel part that has ever shipped in a laptop, minus the one
        // family that does not belong here: Arc is a discrete card that happens
        // to say Intel on it.
        if name.contains("intel") && !name.contains("arc") {
            return true;
        }

        // AMD names its integrated parts by what they are not: an APU reports
        // `Radeon(TM) Graphics` or `Radeon(TM) Vega 8 Graphics`, where a card
        // reports `Radeon RX 6700 XT` and stops. The trailing "graphics" is the
        // whole tell, and `rx` is the guard against the handful of discrete
        // Vegas that would otherwise match on the second clause.
        if name.contains("radeon") && !name.contains(" rx") {
            if name.contains("graphics") || name.contains("vega") {
                return true;
            }
        }

        // Phones and tablets, which are integrated by construction. Apple's own
        // parts are deliberately absent: they share memory too, and they have
        // the bandwidth of a discrete card to do it with.
        name.contains("adreno") || name.contains("mali") || name.contains("powervr")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laptop_parts_are_named_as_integrated() {
        for renderer in [
            "ANGLE (Intel, Intel(R) UHD Graphics 620 Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (Intel, Intel(R) Iris(R) Xe Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (AMD, AMD Radeon(TM) Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (AMD, AMD Radeon(TM) Vega 8 Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "Google SwiftShader",
            "Adreno (TM) 640",
            "Mali-G78",
        ] {
            assert!(
                Quality::is_integrated(renderer),
                "{renderer} should have been read as integrated"
            );
        }
    }

    /// The expensive mistake is the other one — a card that can afford four
    /// samples being docked them for a whole match, with nothing downstream to
    /// notice.
    #[test]
    fn cards_keep_their_samples() {
        for renderer in [
            "ANGLE (NVIDIA, NVIDIA GeForce RTX 3070 Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (AMD, AMD Radeon RX 6700 XT Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (Intel, Intel(R) Arc(TM) A770 Graphics Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)",
            "Apple GPU",
        ] {
            assert!(
                !Quality::is_integrated(renderer),
                "{renderer} should have been left at four samples"
            );
        }
    }

    /// **The band has to be able to contain a machine that is genuinely
    /// slow**, which is the bug this pair of constants had.
    ///
    /// At a ceiling of 100 ms a part managing six to nine frames a second —
    /// 110 to 170 ms — fell straight through the top of the band and was read
    /// as a stalled page, so the one mechanism that could have helped it
    /// declined to act. The numbers below are the frame times of machines
    /// this is FOR, and every one of them has to be inside.
    #[test]
    fn a_slow_machine_is_inside_the_band_it_is_judged_by() {
        for median in [24.5f32, 40.0, 80.0, 120.0, 170.0, 250.0] {
            assert!(
                (Quality::STRUGGLING_MS..Quality::STALLED_MS).contains(&median),
                "{median} ms is a machine that needs help and the band excludes it"
            );
        }
        // …and a page that has genuinely stopped is still outside it. A
        // four-second shader link or a backgrounded tab must not dock anybody.
        for median in [900.0f32, 4_000.0] {
            assert!(!(Quality::STRUGGLING_MS..Quality::STALLED_MS).contains(&median));
        }
        // A comfortable machine, on either common refresh rate, is below it.
        for median in [8.3f32, 16.7, 20.0] {
            assert!(median < Quality::STRUGGLING_MS, "{median} ms is not slow");
        }
    }

    /// Both controllers read the same figure and answer it the same way, so
    /// they have to agree about what the figure means — a machine that fails
    /// one should fail the other. Stated as a test because they live in two
    /// files and the numbers were kept in step by comment alone.
    #[test]
    fn the_two_controllers_judge_by_the_same_band() {
        use crate::app::stage::Stage;
        assert_eq!(Quality::STRUGGLING_MS, Stage::struggling_ms());
        assert_eq!(Quality::STALLED_MS, Stage::stalled_ms());
    }

    /// The window has to be wide enough for [`Quality::CONSECUTIVE`] readings
    /// at [`Quality::REVIEW`] apart, or requiring two of them means requiring
    /// something that cannot happen and the tier never moves at all.
    #[test]
    fn the_window_holds_the_readings_it_asks_for() {
        let window = Quality::SETTLE_BEFORE - Quality::SETTLE_AFTER;
        let needed = Quality::REVIEW * Quality::CONSECUTIVE as f32;
        assert!(
            window >= needed * 2.0,
            "a {window} s window cannot comfortably hold {} readings {} s apart",
            Quality::CONSECUTIVE,
            Quality::REVIEW
        );
    }
}
