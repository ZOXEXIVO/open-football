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

use crate::perf::FrameCost;
use bevy::anti_alias::fxaa::{Fxaa, Sensitivity};
use bevy::platform::time::Instant;
use bevy::prelude::*;
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
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Whether the tier has stopped moving.
    ///
    /// Read by [`crate::stage::Stage::fit`], which holds its own ladder until
    /// this is true so that one slow stretch does not get answered twice.
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
    /// Called once, by [`crate::bringup::Bringup`], when the stadium is up
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
        if cost.typical_frame_ms() < Self::STRUGGLING_MS {
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
    fn renderer() -> Option<String> {
        let canvas: HtmlCanvasElement = web_sys::window()?
            .document()?
            .create_element("canvas")
            .ok()?
            .dyn_into()
            .ok()?;
        let context: WebGl2RenderingContext =
            canvas.get_context("webgl2").ok()??.dyn_into().ok()?;
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
}
