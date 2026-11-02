# SPEC 2026-11-02: the cloud map as a real 3D view (orbit, camera zoom, fly to a group)

Status: build spec, agreed by the Punchable, iOS dev and iOS design experts before the build (both: "AGREED WITH
CHANGES", folded in below and marked [review]). Owner, 2026-11-02: "i was looking at the 3d cloud map, i think we
should make it more interactive and 3d like i can rotate it and zoom into different spots, this one gets blurry".
Tags: **CONFIRMED** = read at `file:line` or in the package source; **INFERRED** = judgement; **UNVERIFIED** = only a
device build can settle it. HIG pages are `https://developer.apple.com/design/human-interface-guidelines/<page>`
(written `HIG/<page>`).

**Claim: the blur is the view being magnified, not the dots. Zoom moves a camera instead, so Skia draws every dot and
label at device resolution at every zoom; a drag orbits the cloud in yaw and pitch; a group's chip or name flies the
camera to it; "Whole map" brings it back. One camera model in `rules/cloudMath.ts` drives the phone (UI thread) and
the web (SVG).**

## 0. Why it blurs today, and two defects found on the way (CONFIRMED unless marked)

- `lib/Cloud.native.tsx:135` wrapped the Skia `Canvas` in `lib/PanZoom` (`PanZoom.native.tsx:462`: a `scale`
  transform on an `Animated.View`). Skia rasterised at 1×; the view transform scaled the bitmap, so at 4× each dot was
  a 4× upscale. The cloud also turned about one axis only (`theta`): "rotate it" had no handle beyond ⟲ ⟳.
- **[review]** `wrapAngle` (`rules/cloudMath.ts`) had no `'worklet'` directive, yet the turn's frame callback called it
  (`Cloud.native.tsx:60`): the build-16 crash class ("Tried to synchronously call a Remote Function";
  `react-native-worklets/src/memory/serializable.native.ts:172`). In builds 17 and 18.
- **[review]** `useFrameCallback(cb, turning)` reads its second argument only when it first registers
  (`react-native-reanimated/src/hook/useFrameCallback.ts`: `isActive: autostart` kept in a ref; a re-registration
  reuses the stored value). `turning` is false on the first render (no cloud, no size yet), so on the phone the slow
  turn most likely **never ran**, which is also why the missing directive never crashed. INFERRED (source; no
  device). Now switched with `setActive(turning)`.

## 1. The camera (pure, `rules/cloudMath.ts`, every function `'worklet'`, node tests)

State `Cam = { yaw, pitch, tx, ty, tz, lz }`: an orbit around a target point `t` (world units, the cloud is about
[-1, 1]³), `lz` = ln(zoom) so zoom animates evenly. Home = all zeros.

| Piece | Rule |
|---|---|
| Rotation | `p − t`, then yaw about Y (`x' = x cos − z sin`, `z' = x sin + z cos`), then pitch about X (`y'' = y cos − z' sin`, `z'' = y sin + z' cos`). Pitch clamped to ±1.2 rad (~69°): up stays up. |
| Projection | Real perspective. Camera distance `dist = EYE / zoom`, `EYE = 4`; a point at camera depth `d = dist − z''` lands at `centre + (x'', −y'') · R · EYE / d`. At zoom 1 the target plane maps exactly as the old fit (`R` = half the padded square); the perspective spread at zoom 1 (≈0.8×-1.33×) is close to the old 0.82-1.18. |
| Zoom bounds | 0.6× to 12×. At 12× the view is inside the cloud: what was ~8 pt apart is ~100 pt apart. Whole map returns to 1×, not 0.6× **[review]**. |
| Near fade | Dots nearer the camera than 25 % of `dist` are not drawn (culled before the divide **[review]**), fully drawn from 60 %: zooming in passes through the dots in front. Relative to `dist`, so nothing is culled at zoom 1. |
| Depth cue | `depth = clamp((z'' + 1) / 2)`; alpha `0.3 + 0.7 · depth`, times the near fade. |
| Dot size | `R_MIN + (R_MAX − R_MIN) · depth`, times `clamp(√(EYE / d), 0.75, 2.6)`: dots grow with zoom, slower than the gaps between them, so a dense spot separates (8×: gaps 8×, dots under 3×). |
| Zoom about a point | `zoomAbout(cam, newLz, ox, oy, R)`: the world point under the pinch (on the target plane) stays under the fingers; the target moves along the camera's right/up vectors. |
| Pan | `panBy(cam, dx, dy, R)`: the target plane follows the fingers. The target is clamped to the cloud's box (±1.2) so part of the cloud is always on screen **[review]** (HIG/maps: "keep at least part of your indoor map visible onscreen at all times"). |
| Fly to a group | Target = the group's centre; zoom = `clamp(0.85 / spread, 1.6, 6)`, `spread` = RMS distance of its members from the centre (once per payload); yaw/pitch unchanged (nothing spins under him). 350 ms ease-out, cancelled by any touch **[review]**. |
| Hit test | `projectInto` (typed arrays, no object per dot) feeds the picture and the tap; a tap picks the nearest **drawn** dot within 18 pt of its edge in screen space (no `view.scale`). On the phone it runs in the Tap worklet and only the index crosses to JS **[review]** (reading six shared values on JS is six blocking `runOnUISync` calls, `mutables.native.ts:33-43`). |
| Group names | Alpha by depth and near fade; past 2× the names away from the screen centre fade, so a zoomed view shows the group you are in. A tap on a name flies to its group **[review]**: the tap box is 44 pt tall and as wide as the name (≤ 140 pt); the name of the group you are in (zoom > 2×, within 15 % of the centre) takes no tap. |
| Bit names | From 2.5× (full by 3.5×) the nearest dots get their short name, greedily on a 160 × 24 pt grid (a name also claims the cell to its right), near dots first, the selected dot first **[review]**, at most 30 (HIG/maps: "progressively add more detailed features and labels as the map is zoomed in"). 12 pt × min(font scale, 1.2) (HIG.md §3.10's cap), the system font semibold (Skia's `matchFont` sees system fonts; Inter comes through expo-font, which Skia's font manager does not see, INFERRED), fill `C.text` over a 5 pt `C.bg` halo with round joins for light and dark **[review]**. Names cut at 22 characters. A tap on a name selects the bit once names are at least half shown; the box is the measured width × 24 pt **[review]**. |

## 2. Gestures on the phone (react-native-gesture-handler 3.3 builder API + Reanimated 4.7, already in)

| Gesture | Does | Notes |
|---|---|---|
| One-finger drag | Orbit: right turns the front right, down tilts the top toward you; across the canvas width = 180°. | Release = momentum (`withDecay`; pitch with `clamp`; velocity × the same π / width). Off under Reduce Motion. **[review]** Deltas from `onChange`'s `changeX/Y`, only at one pointer and not while pinching; the first update after the pointer count changes re-bases (the touch centre jumps); no momentum straight out of a pinch. |
| Pinch | Zoom about the pinch point; the midpoint moving pans. | **[review]** Incremental: `scaleChange` into `zoomAbout`, the focal point's change since the last update into `panBy`, the focal reset in `onStart`. |
| Tap | **[review]** In this order (`resolveTap`, shared with the web): a dot within 6 pt is selected; then a bit's name; then a group's name flies there; then the nearest dot within 18 pt; else it clears. The card (Open · To a set · Done) is unchanged; the selected dot again opens it. | Waits for the double-tap window (Maps does the same). |
| Double tap | Zoom in 2× at that spot, animated; **at 12× back to the whole map** **[review]** (HIG/gestures: "Zoom in; zoom out if already zoomed in"). | |
| Double-tap-and-drag **[review]** | One-handed zoom: drag up = in, down = out, about the tapped spot (Maps, Photos). The one-handed zoom out. | A touch within 350 ms and 40 pt of a tap that never became a drag. |
| Two-finger tap | Zoom out 2× about the centre. | Maps convention. |
| Any touch | Stops the slow turn and cancels a flight or momentum at touch-down (`onBegin`, not `onStart`) **[review]** (HIG/motion: "Let people cancel motion"). The turn stops through a shared value the frame callback reads. | |

**[review] Composition and worklets** (iOS dev expert, from the package source): `Gesture.Race(Gesture.Simultaneous(orbit,
pinch), Gesture.Exclusive(twoFingerTap, doubleTap, tap))`, the two-finger tap first because a one-pointer tap also
succeeds with two fingers (`RNTapHandler.m:171`). RNGH 3.3 keeps the builder API (deprecated, `gestureObjects.ts`) and
the legacy detector (`v3/detectors/GestureDetector.tsx:20-25`); callbacks become worklets only when written inline in
a `Gesture.X()` chain (`worklets/plugin/index.js:264-276`) and mixing worklet and JS callbacks is an error
(`GestureDetector/utils.ts:91-99`): every callback is inline. The canvas is no longer inside `PanZoom` (Galaxy keeps it).

## 3. Chrome (`lib/CloudChrome.tsx`, shared)

- **[review]** A **bottom-trailing** column for the thumb (was top-right), above the card when a bit is selected:
  **+** and **−** (SF Symbols `plus`, `minus`; 44 pt circles with a 1 pt `C.borderStrong` stroke, HIG/maps "a thin
  stroke … to help a custom control stand out"; at 12× / 0.6× the button dims to 0.4 and is disabled **[review]**), then **Whole map** as a **text capsule** in a fixed slot, shown only off
  home (not `arrow.up.left.and.arrow.down.right`, which means full screen elsewhere in iOS). Kept on iPhone although
  Maps hides its zoom buttons: this view has no compass or other way back, and HIG/accessibility asks for "onscreen
  ways to achieve the same outcome". The ⟲ ⟳ turn buttons go: a drag replaces them, the toolbar's Turn and the chips
  stay, and the canvas keeps VoiceOver actions for turning.
- Chips: tap = fly to the group (was "turn it to the front"); a11y label "…, N bits. Zoom to it."
- Counts line hint: "drag to turn · pinch to zoom · double-tap to zoom in" **[review]**; VoiceOver reads the counts only
  (its own double-tap activates).
- Toolbar unchanged: Turn/Still, Links.
- VoiceOver **[review]**: the canvas is one `image` element, label "Cloud map: N bits in G groups", value "Whole map"
  or "Zoom 2×, near ‹group›", actions in this order: Zoom in, Zoom out, Whole map, Turn left, Turn right, Tilt up,
  Tilt down; each announces the new value (queued), and Zoom in / out at a limit say "Closest zoom" / "Widest zoom" **[review]**; hint "The Links button reads every bit as a list." (HIG/voiceover).
- Reduce Motion **[review]**: no slow turn, no momentum; flights, double-tap zoom and the buttons jump to the result;
  pinch and drag still follow the fingers ("Tracking animations directly with people's gestures" is on the HIG's list).
- Haptics **[review]**: selection on choosing a dot (as before); one selection tick per pinch or one-handed zoom that hits a zoom limit;
  none when a flight lands (HIG/playing-haptics: "Avoid overusing haptics").

## 4. Performance (phone)

- Per camera change: one `useDerivedValue` over the six camera shared values rebuilds the picture on the UI thread:
  projection into typed arrays allocated inside the worklet **[review]** (a typed array made on JS is copied to the UI
  thread once and later JS writes are never seen, `SerializableArrayBuffer.h:29`), the 16-bucket counting sort,
  ~1,600 `drawCircle`, ≤ 30 × 2 `drawText`. **[review]** One `SkPaint` per group × 16 alpha levels is made once on JS
  and captured (host objects pass by reference, `serializable.native.ts:180-182`): a dot is one native call instead
  of three plus a new Paint. `drawPoints` / `drawAtlas` would need ~1,600 new native objects per frame; not used.
- Crisp: the Skia surface is sized width × `UIScreen.scale` and the picture drawn at that density
  (`RNSkMetalCanvasProvider.mm:95-96`, `RNSkPictureView.h:147`). CONFIRMED.
- Group names stay `Animated.Text` with `useAnimatedStyle`, calling the same `'worklet'` projection (14 × 6 values).
  Nothing per frame on the JS thread; JS hears only gesture ends, flights (for Whole map and VoiceOver's value) and
  taps.
- Every function a worklet calls carries `'worklet'` (HANDOVER trap, build 16). **Every** function in
  `rules/cloudMath.ts` carries it, and `tests/cloudMath.test.mjs` fails on one that does not (with a planted-offender
  self-test), on a call through `M.` to anything cloudMath does not export, and on a helper in `Cloud.native.tsx`
  without it unless listed as JS-only.

## 5. Web (`lib/Cloud.tsx`, SVG)

Same camera math. A pointer drag orbits (momentum on release, decaying on a ~30 fps timer), the wheel zooms about the
cursor (a trackpad pinch arrives as ctrl + wheel: same path, finer), two touch pointers pinch and pan, a double tap
zooms in. The camera is a ref; the SVG renders a state snapshot of it at most every 33 ms (HANDOVER trap: a rAF loop
re-rendering 1,600 SVG dots starves the JS thread; React's rules: no ref reads during render). Flights ease out over
350 ms (`lerpCam`), momentum decays (`decayStep`). Bit names as SVG text from the same `pickLabels`. On the web a
single tap acts at once (no double-tap wait).

## 6. Tests and gates

**[review]** `tests/static.worklets.test.mjs` (new, in CI's static loop): compiles every worklet-bearing `src/rules` file
with babel-preset-expo and the worklets plugin, loads it, and fails on any `undefined` in a worklet's `__closure`
(with a planted-offender self-test). The plugin builds each worklet's closure when the module loads, so a worklet that
names a function declared further down captures `undefined`: `resolveTap` did (iOS dev review, second pass), which
would have thrown on the first tap off a dot on the phone's UI thread and on the web. Rule: a worklet names only what
is declared above it; no nested `'worklet'` (a nested one gets copies of the outer `let`s).


`tests/cloudMath.test.mjs`: home equals the old fit; yaw/pitch directions; pitch clamp; zoom bounds; `zoomAbout` keeps
the point under the fingers (200 random cameras); `panBy` moves the target plane with the fingers; the target box;
double tap in and back at 12×; Whole map; near fade culls nothing at 1× and the dot in front at 12×; a culled dot is
never hit; 8×: gaps 8×, dots under 3×; hit test against the current projection; `spreads`, `flyTo`, the label fades;
`pickLabels` (selected first, ≤ 30, no overlap, on screen); the web's batching, flights, momentum, spin; the
`'worklet'` checks above.

## 7. Only a device can confirm (UNVERIFIED until a build)

Smoothness at ~1,600 dots during a pinch on an iPhone 16; crispness at 12×; that the single tap's double-tap wait
feels fine; the momentum feel; the double-tap-and-drag; that the bit names are legible at 12× in light and dark; that
the slow turn now runs (and with it that the `'worklet'` fix holds on the UI thread).
