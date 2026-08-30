# Prompt: make the match viewer fast (reported ~25 fps, worst during camera movement)

Paste this whole document as the task for a session working in `D:\Projects\open-football`.

---

## The job

The 3D match replay (`src/match`, package `match_viewer` — Bevy 0.19, `wasm32-unknown-unknown`,
WebGL2 backend, single-threaded) runs at ~25 fps on the machine it was reported from, and the
judder is most visible when the camera moves (orbit drag, flight, the broadcast pan). Bring it
to a steady 60 fps during camera movement without degrading the picture at the top quality tier.

Work measurement-first: every claim below was verified in code on 2026-08-30, but the *sizes*
were measured on an RTX 3080 Ti and do not transfer. Re-measure on the affected setup before
picking work items, and re-measure after each one.

## The instrument (use it before touching anything)

`src/match/src/app/perf.rs` — `FrameCost` is always collected and prints to the console every 2 s:

```
match viewer — 300 fps · frame 3.3 ms (p95 4.2, worst 141) = update 0.2 + rest of main 0.7 + outside 2.4 · 464/698 meshes drawn
```

- `update` = this crate's systems. `rest of main` = Bevy main world: transform propagation,
  visibility, **UI layout, text shaping**. `outside` = render sub-app + browser: extract,
  batch, **draw submission**, GPU wait. Single-threaded wasm, so the arithmetic is exact.
- Reading it: large `outside` + high `drawn` = per-entity submission bound. Large `outside` +
  low `drawn` = fill/sample bound. Large `rest of main` = UI/text/propagation churn.
- `worst` is a separate figure from p95 on purpose — one-off stalls (chunk parses, pipeline
  compiles) fall through a 2 s median.
- Debug overlay on the transport bar shows the same line plus the resolution-ladder rung
  (`debug` on in `ViewerConfig`).
- Measure in headless Chrome with the REAL GPU: `--enable-gpu --disable-gpu-vsync
  --disable-frame-rate-limit`. NEVER the SwiftShader/angle flags from the screenshot recipe —
  SwiftShader is fill-bound where the real thing is per-entity-bound and reports 1 fps for a
  300 fps scene. Without `--disable-gpu-vsync` you measure the refresh rate, not the scene.
- `.dev/match` is the harness that serves a fixture match; `recording/loader.rs` logs
  per-chunk parse cost. `fit_canvas_to_parent` ignores `devicePixelRatio` — DPR is not a variable.

### Step 0 — reproduce and bucket the 25 fps

On the affected setup: open `chrome://gpu` first and confirm no SwiftShader/software fallback
(a blocklisted driver alone produces exactly this symptom and no code change fixes it). Then
capture the FrameCost line three ways: broadcast framing at rest, during a long orbit drag, and
during free flight pointed down the length of the ground (worst case: nearly all ~870 mesh
entities in frustum). The bucket that grows is the work list that applies below.

## Known cost structure (measured Aug 2026, RTX 3080 Ti, 1080p — ratios travel, numbers don't)

- Frame is **per-entity bound on discrete GPUs**: 3.9 ms at 720p and at 4K identically.
  Stadium-only = 130 entities @ 1.6 ms; full scene = ~870 entities, ~550 drawn @ 3.9 ms.
- The 22 players are ~60 % of the frame.
- On integrated GPUs the frame flips to **sample-bound**; two mechanisms already exist for that
  and only that: the MSAA→FXAA tier (`app/quality.rs`, probe + one-way `relent` in a 6–20 s
  window) and the resolution ladder (`app/stage.rs`, `SCALES = [1.0, 0.87, 0.75, 0.65, 0.55]`,
  measurement-driven, one-way). **A submission-bound machine has no mechanism at all** — both
  existing ones spend picture quality and buy nothing there.

## Verified hot places

1. **Player rigs: ~656 of ~870 mesh entities.** `Footballer::assemble`
   (`src/match/src/players/body.rs:5858`): ~28 mesh entities per outfielder, ~48 per keeper —
   each keeper hand alone is 9 mesh entities (thumb, 4 fingers, 4 fingertips, each its own
   `Joint`). Every part pays the full per-entity toll every frame: `Transform` write in
   `Actors::animate` (`players/actors.rs:2300`), propagation through a 5–6-deep subtree,
   per-entity visibility, per-entity extract/queue, and WebGL2 draw submission (every draw a
   validated call into the browser's GL).
2. **~12 of those parts per player have no `Joint` of their own** — number, name, front plate,
   collar, hair, sleeve ×2, cuff ×2, shorts-leg ×2, sock-top ×2. They are never posed
   independently; they exist as entities only because they wear a different material
   (trim/shirt/shorts/socks) than their parent.
3. **Batching is already half-won, so don't redo it:** part meshes are shared through
   `Res<BodyParts>` (built once, `players/actors.rs:1340`) and team materials are shared per
   kit / per complexion ramp (`Wardrobe::outfit`, `players/kit.rs:577`). Per-player materials
   are only face, number, name, front plate. The cap on batching is the ~15 distinct part
   meshes × material combos, not material duplication.
4. **Camera-movement-specific main-world cost:** the 22 name plates are `bevy_ui` nodes.
   `Actors::place_labels` (`players/actors.rs:3301`) is change-gated and pixel-rounded, so a
   still camera writes nothing — but any pan moves every plate every frame → 22 `Node` writes →
   taffy relayout + text measure. (The seek fill also writes every frame while playing —
   `ui/timeline.rs:1046`.)
5. **Hitches, not steady rate:** (a) WebGL2 compiles a render pipeline the first frame a
   material/mesh combo is visible — bring-up staggers the stadium (`app/bringup.rs`), but a
   combo first revealed *by a camera flight* compiles mid-pan, a several-hundred-ms stall;
   (b) chunk parse: 4.5 MB JSON through `serde_json` on the only thread, 34–46 ms each, three
   requested at once as read-ahead (`ChunkLoader::request_chunk`, `recording/loader.rs`) —
   recorded `worst 141`.
6. **Already clean — do not spend time there:** netting deforms only while absorbing a strike
   (`scene/net.rs:648`); pitch markings are one merged mesh (`scene/pitch.rs` `LineMesh`);
   crowd is one mesh per bank (`scene/crowd.rs`); `Bank::cull` and shadow discs are
   change-gated; shadow maps are off; the turf/fill path is resolution-insensitive on discrete.

## Work items, ranked

### WI-1 — Collapse the player rig (the big lever, ~60 % of the frame)

Two stages; land A first, it is low-risk and mechanical:

**A. Bake the ~12 non-jointed parts into their jointed parents.** Merge sleeve+cuff into the
upper-arm mesh, keeper sleeve+cuff into the forearm, shorts-leg into the thigh, sock-top into
the shin, collar into the torso. The only reason they are entities is per-part materials, so
give the merged mesh either vertex colors or a small per-team atlas so one material survives.
~28 → ~16 entities per outfielder (−250 total). Keeper hands: one glove mesh per hand posed as
a whole (keep 2–3 grip poses if the dive needs them) instead of 9 jointed digit entities
(−16 per keeper). Decals (number/name/front plate) can stay separate quads — they are 3 per
player and per-player materials anyway.

**B. If A is not enough on the affected machine: one skinned mesh per body.** Bevy skinned
meshes work on WebGL2. Rigid weights, one bone per current `Limb`; the existing `Joint::pose`
math becomes the bone-transform writer unchanged; joints become `Transform`-only entities with
no `Mesh3d` (no extraction, no draw). Player = body + head/face + hair + decals ≈ 5 entities.
Needs a per-player or per-team texture atlas for the body; keep the face its own mesh+material —
the portrait pipeline (`players/portrait.rs`) paints the face material in place and must not be
disturbed.

Acceptance: mesh census in the FrameCost line drops accordingly; `outside` shrinks roughly in
proportion at the same framing; the lineup ceremony (front plates), aftermath poses, dives,
kits, prints, and portraits all survive a screenshot pass.

### WI-2 — Warm every pipeline behind the loading readout

During bring-up, render one throwaway frame (or a sequence of them) with every material/mesh
combo in frustum — stands, crowd, sky, net, a dressed player of each kind — so WebGL2 pipeline
compiles all land behind the loading readout instead of mid-flight. Acceptance: `worst` during a
first full flight around a fresh ground stays within ~2× the median.

### WI-3 — Get the chunk parse off the frame

A plain Web Worker (no atomics, no COOP/COEP — those would break the cross-origin player
photos) that fetches and parses the chunk and posts back a compact typed structure; or switch
the chunk encoding to a binary format (the recording writer lives in this repo, so both ends
can move together). Acceptance: no 34–46 ms parse frames in the loader log during playback;
`worst` under load ≤ ~2× median.

### WI-4 — Plates without taffy (only if Step 0 shows `rest of main` matters)

Pre-rasterize each surname plate once — `typeface::Stencil` already rasterizes these exact
strings for shirt backs — and draw plates as billboard quads (or Text2d) on the overlay camera.
A `Transform` move costs no UI relayout and no text measure, so a pan writes nothing into
`bevy_ui`.

### WI-5 — Verify the sample-bound mechanisms actually fired (integrated GPUs only)

If the affected machine is integrated: the debug overlay must show the FXAA tier and/or a
ladder rung engaged. The probe is silent on Firefox/Safari and `relent` fires once inside a
6–20 s window — if the struggle starts later (first camera flight!), the window has closed.
Consider re-arming `relent` on sustained struggle rather than widening defaults blindly.
Measure first.

### Dead ends — do not spend time on these

- Cheaper turf/pitch shading, texture size, MSAA on discrete GPUs: the frame is
  resolution-insensitive there; this was measured, twice.
- wasm threads for rendering: needs nightly against the stable-toolchain rule, COOP/COEP that
  breaks player photos, and WebGL2 renders from one thread regardless.
- A WebGPU backend build is a real lever on submission cost but a product decision (second
  wasm artifact + fallback); raise it, don't do it unprompted.

## Guardrails

- **Never commit.** The user edits concurrently — re-read files before editing them. The user
  runs and eyeballs the app themselves; ask for a check rather than assuming.
- Stable toolchain only. `wasm-bindgen` stays pinned `=0.2.127`. The build recipe is
  `src/match/builder` (nested release build → wasm-bindgen library → gzip → stage) — both
  `src/web/build.rs` and `.dev/match/build.rs` call it; don't add CLI tooling.
- `src/match` is not fmt-clean — never `cargo fmt --all`; format only what you touched.
- Keep `FrameCost` always-collected, keep the per-chunk parse log, keep the debug overlay
  fields meaningful (mesh census, rung).
- Both quality tiers stay antialiased and nothing is culled from the top-tier scene: the fix is
  doing the same picture cheaper, not a cheaper picture.
- Match the crate's style: heavy narrative doc comments explaining *why*, `struct` + method
  organization, no free functions.
- Record before/after FrameCost lines (same fixture, same camera path: rest, orbit, length-of-
  ground flight) for every work item in your final summary.
