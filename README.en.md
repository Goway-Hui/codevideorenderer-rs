# CodeVideoRenderer-rs

Render **"typing" code videos** in Rust: a snippet is animated character by
character while a camera follows the caret, then encoded to MP4.

> 中文文档：[README.md](README.md)

This is a from-scratch Rust reimplementation of
[CodeVideoRenderer](https://github.com/ExploreMaths/CodeVideoRenderer) (Python +
Manim). The visual language is deliberately the same — monospace grid, line
numbers, current-line bar, caret, camera follow/zoom-out, hand-held camera sway —
but the pipeline underneath is built so that it can (a) use every CPU core and
(b) be trusted in batch use.

```console
$ cvr --code-file script.py --language python --style github-dark --name Demo
cvr 0.1.0: python · github-dark · 1920x1080 @ 60fps
  font     assets/CodeVideoRendererFont.ttf
  theme    github-dark
  layout   12 lines · 245 typed characters · 71 glyph outlines
  frames   1523 @ 60fps (25.4s)
  render   3.41s (447 frames/s on 16 threads)
  output   Demo.mp4
```

---

## Why a rewrite

The Python version works, but four of its behaviours are structural rather than
cosmetic (all verified in its source):

1. **Quadratic rendering.** It called `scene.play()` once per typed character.
   Manim rebuilds a full "static layer" on every `play()` — and because the
   camera is animated while the scene's mobjects are static, every frame ended up
   re-rasterising *all* characters typed so far. Total work is O(N²) for N
   characters: ~125,000 glyph draws for a 500-character snippet.
2. **Serial frames.** Manim's frame loop has no threading at all; each of the
   ~9 frames per character is rendered and written one at a time.
3. **Two encode passes.** Manim writes an MP4, then MoviePy decodes it, blurs
   every frame in PIL and re-encodes — plus hundreds of partial movie files along
   the way (one per `play()`).
4. **Global state.** Parameters live in a module-level class, so a second
   `CameraFollowCursorCV` instance changes the parameters of the first; Manim's
   global config is mutated on construction and restored only after a successful
   render; and the output path is built with hard-coded Windows separators.

This crate restructures all four away. Nothing here is a line-by-line port.

## Architecture

```text
Source ──preprocess──▶ text ──lexer──▶ per-character token kinds
                                    │
                                    ▼
                              layout: glyphs + positions (computed once)
                                    │
                                    ▼
                        timeline: reveal times + baked camera keyframes
                                    │
                      frame(t) = pure function of time
                                    │
                    rayon ──▶ bounded batches ──▶ single ffmpeg pipe
```

* **One layout pass.** Glyph positions, colours and reveal order are computed
  once. A frame is a lookup, not a rebuild — the O(N²) term is gone.
* **Pure frames.** The camera is baked into keyframes and sampled by binary
  search, so `render_frame(t)` touches no shared mutable state and frames can be
  rendered in parallel. Memory stays flat because frames are produced in bounded
  batches.
* **Outlines cached in font units.** Glyph outlines are scale independent, so
  they are extracted once and re-filled at the current camera zoom — text stays
  crisp when the camera pulls back (the same guarantee Cairo gave the original).
* **Single encode.** Frames stream into one `ffmpeg` process; no intermediate
  MP4, no decode, no per-frame PIL pass.
* **No globals.** All state lives in `RenderOptions` / `Layout` / `Timeline`.
  Two renders in one process cannot interfere.

| Module | Responsibility |
|---|---|
| `config` | Defaults that mirror the original's constants |
| `error` | Every failure mode, as a typed error |
| `font` | Font discovery, metrics, outlines (`ttf-parser`) |
| `lexer` | Hand-written scanner → Pygments-compatible token kinds |
| `theme` / `theme_data` | 43 Pygments styles, plus the hand-written `midnight` default |
| `layout` | Preprocessing + monospace grid |
| `camera` | Keyframed camera: entrance, follow, zoom-out, sway |
| `timeline` | Reveal times, line breaks, frame count |
| `render` | Rasterisation (`tiny-skia`) |
| `encode` | ffmpeg pipe or PNG sequence, plus progress |
| `api` | `render(Source, RenderOptions) -> RenderReport` |

## Feature parity with the Python version

| Feature | Python + Manim | Rust |
|---|---|---|
| Code from string or file | ✅ | ✅ |
| Tab expansion, blank/edge-line handling | ✅ | ✅ |
| Invalid-character rejection | ✅ | ✅ |
| Line numbers + active line | ✅ | ✅ |
| Current-line highlight bar | ✅ | ✅ |
| Caret | ✅ | ✅ |
| Typing interval range | ✅ | ✅ (deterministic via `--seed`) |
| Camera entrance / follow / auto zoom-out | ✅ | ✅ |
| Hand-held camera sway | ✅ | ✅ |
| Syntax highlighting | Pygments, 600+ lexers | 19 hand-written grammars + generic fallback |
| Themes | 60+ Pygments styles | 43 Pygments styles + 1 hand-written default |
| Progress reporting | rich | built-in single-line progress |
| Resolution / frame-rate control | only via global Manim config | ✅ first-class options |
| Parallel frame rendering | ❌ serial | ✅ rayon |
| Batch-safe (many renders per process) | ❌ global parameters leak | ✅ |
| Works on Linux/macOS output paths | ❌ Windows separators | ✅ |
| Empty code | crashes at render time | rejected at construction |
| Glow post-processing | ✅ (forced, two encode passes) | ✅ `--glow` (one encode, off by default) |
| OpenGL renderer | ✅ (author notes it is slower) | not applicable |
| Runtime deps | manim, pygments, moviepy, PIL, numpy, rich, ffmpeg | tiny-skia, ttf-parser, rayon, ffmpeg |

**Not implemented yet:** CJK/emoji glyph fallback. The bundled font covers CJK;
system fonts may not, and a character with no glyph is a typed error naming the
codepoint rather than a silently missing box.

The glow post-process *is* implemented, as `--glow`. The original ran it as a
second pass over the finished MP4 (decode → blur every frame in PIL → re-encode);
here it is one ffmpeg filter on the way out, so it stays a single encode. It is
off by default because a per-frame filter costs encode time.

Two notes on how text is laid out, both inherited from the original:

* **Column alignment is metric-based, not cell-based.** The grid follows the
  font's real advances, so a CJK character (1.0 em in the bundled font) is not an
  exact multiple of the ASCII cell (0.55 em). Lines that mix the two do not line
  up column for column. The cursor stays correct because it accumulates the same
  advances.
* **A language without a dedicated grammar is highlighted generically.** The
  render still succeeds; the CLI prints a note to stderr (suppressed by
  `--quiet`) so a "why does `--language haskell` look plain?" question answers
  itself. `cvr --list-languages` shows the grammars that exist.

## Requirements

* Rust 1.85+
* `ffmpeg` on `PATH` — or use `--frames <DIR>` to write a PNG sequence instead
  (no external tools needed)

## Build

```console
$ cargo build --release
$ ./target/release/cvr --help
```

To bake the bundled font into the binary (single-file distribution):

```console
$ cargo build --release --features embed-font
```

### Package size

The published crate is ~13 MiB uncompressed (7 MiB on crates.io), and the
bundled `assets/CodeVideoRendererFont.ttf` is about 98% of it. That is the price
of shipping a font: CJK snippets render out of the box, and `--font` is only
needed when you want a different typeface.

## Usage

### Command line

```console
# Inline code, default theme
cvr --code 'print("hello")' --name Hello

# From a file, GitHub Dark, faster typing, 720p
cvr --code-file algorithm.py --style github-dark --interval 0.05:0.12 \
    --resolution 720p --name Algorithm

# Bigger code: zoom out more and hold the last frame longer
cvr --code-file big.py --camera-scale 0.35 --end-pause 3 --name BigDemo

# No ffmpeg? Write PNG frames instead
cvr --code-file script.py --frames frames/out

# Discovery
cvr --list-styles
cvr --list-languages
```

### Library

```rust
use codevideorenderer::{render, RenderOptions, Source};

// Builder-style, or set the fields directly — both work.
let options = RenderOptions::default()
    .with_language("python")
    .with_style("monokai")
    .with_fps(30)
    .with_interval(0.08, 0.2);

let report = render(Source::file("script.py"), options)?;
println!("{} frames in {:.1}s → {}", report.frames, report.elapsed, report.output);
# Ok::<(), codevideorenderer::Error>(())
```

Render a single frame without encoding anything (useful for previews):

```rust
use codevideorenderer::{render_preview, RenderOptions, Source};

let options = RenderOptions::default();
let pixmap = render_preview(Source::text("x = 1"), &options, 2.0)?;
pixmap.save_png("preview.png").unwrap();
# Ok::<(), codevideorenderer::Error>(())
```

Draw many frames from one setup — layout, timeline and the glyph outline cache are
built once, then every frame is just a rasterisation:

```rust
use codevideorenderer::{Prepared, RenderOptions, Source};

let options = RenderOptions::default().with_fps(30);
let prepared = Prepared::new(&Source::file("script.py"), &options)?;
let timeline = prepared.timeline();

for frame in 0..timeline.frame_count {
    let t = timeline.time_of_frame(frame);
    let pixmap = prepared.render_at(t)?;
    // ... save it, diff it, show it
    let _ = pixmap;
}
# Ok::<(), codevideorenderer::Error>(())
```

Send frames somewhere the built-in sinks cannot reach, watch progress, and stop a
render you no longer want:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use codevideorenderer::tiny_skia::Pixmap;
use codevideorenderer::{
    Error, FrameSink, ProgressCallback, RenderOptions, Source, render_with_sink,
};

/// A sink is anything that can take frames in order and finalise itself.
struct Uploader {
    bytes: usize,
}

impl FrameSink for Uploader {
    fn write(&mut self, _index: u32, pixmap: &Pixmap) -> Result<(), Error> {
        self.bytes += pixmap.data().len();
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<String, Error> {
        Ok(format!("uploaded {} bytes", self.bytes))
    }
}

let cancel = Arc::new(AtomicBool::new(false));

let options = RenderOptions::default()
    .with_progress(ProgressCallback::new(|done, total| {
        eprintln!("{done}/{total} frames");
    }))
    .with_cancel(Arc::clone(&cancel));

// ... set `cancel` from another thread to stop at the next batch boundary;
// the render then returns `Error::Cancelled` instead of a report.
let report = render_with_sink(Source::file("script.py"), options, Box::new(Uploader { bytes: 0 }))?;
println!("{}", report.output);
# Ok::<(), codevideorenderer::Error>(())
```

## Colour

The default theme is `midnight`. It is written by hand rather than exported from
Pygments, because an editor palette has to hold its own next to toolbars, tabs
and a gutter, while a code video has nothing on screen but the code:

* **Punctuation and operators stay close to the foreground**, so brackets and
  commas stop competing with the code. Material and One Dark paint them bright
  cyan, which is what makes a line of code look like confetti.
* **Six restrained hues, one job each** — violet for keywords, blue for function
  names, green for strings, teal for built-ins and types, amber for classes and
  numbers, and a muted red that only appears in errors.
* **Nothing is fully saturated, and the background is deep**, so the brighter
  colours stay readable after video compression instead of glowing.
* **Line numbers, the current-line bar and the caret are derived** from the
  theme's background and foreground rather than hard-coded, so they stay in tune
  in every theme — including the light ones, where a fixed grey bar would be
  invisible.

`cvr --list-styles` shows the other 43; `--style <name>` picks one.

## Options

| Flag | Default | Meaning |
|---|---|---|
| `--code <TEXT>` / `--code-file <PATH>` | built-in example | what to animate |
| `--language <NAME>` | `python` | language for syntax highlighting |
| `--style <NAME>` | `midnight` | Pygments style name, or the hand-written default |
| `--font <PATH>` | bundled → system | monospace TTF/OTF |
| `--line-spacing <F>` | `0.8` | extra spacing between lines (× font size) |
| `--interval MIN[:MAX]` | `0.15` | seconds between characters |
| `--camera-scale <F>` | `0.5` | initial zoom; smaller = closer |
| `--snap-camera` | off | hold the camera still between characters (frames repeat, so most are reused) |
| `--end-pause <F>` | `1.0` | hold on the finished code, seconds |
| `--seed <N>` | fixed | jitter seed (same seed → same video) |
| `--output <PATH>` | `<name>.mp4` | where the video goes |
| `--name <STEM>` | `CameraFollowCursorCV` | output name |
| `--frames <DIR>` | – | write PNG frames instead of a video |
| `--resolution <SPEC>` | `1080p` | `1080p`, `720p`, `480p`, `1440p`, `2160p`, `WxH` (≤ 8K) |
| `--width <N>` / `--height <N>` | – | explicit pixel size |
| `--fps <N>` | `60` | frame rate |
| `--crf <N>` / `--preset <NAME>` | `18` / `veryfast` | x264 settings |
| `--glow` | off | add the original's glow post-process (slower encode) |
| `--ffmpeg <PATH>` | `ffmpeg` | ffmpeg executable |
| `--quiet` | – | no progress output |
| `--list-styles` / `--list-languages` | – | discovery |
| `-h` / `-V` | – | help / version |

`--interval` has a floor of `1/fps`: at 60fps a character cannot be typed faster
than 16.7 ms. The renderer checks this up front and reports the exact constraint
rather than failing halfway through.

Frames that repeat the previous image are not rasterised again — that is free,
and it is what the closing pause costs. `--snap-camera` extends the same idea to
the typing itself: the camera holds each keyframe instead of gliding, so the
image only changes when a character appears. It is off by default because the
gliding camera is what the Python version looks like.

## Performance notes

Rendering cost is now proportional to `frames × visible glyphs`, not to the
square of the code length:

| | Python + Manim (measured behaviour of the original) | this crate |
|---|---|---|
| Glyph rasterisations for 500 chars | ≈ 125,000 (O(N²) static-layer rebuilds) | ≈ 500 typed + viewport-culled draws per frame |
| Frame rendering | 1 core, serial | all cores, bounded batches |
| Encode passes | 2 (Manim + MoviePy) + decode | 1 |
| Partial movie files | one per typed character | none |

Frame-parallel rendering means throughput scales with core count; the encoder
becomes the first real bottleneck, which is where it should be.

### Measured

Development machine: AMD Ryzen 9 7945HX (16C/32T), Windows.

**Rasterisation alone** — `cargo run --release --example bench` (1080p frames,
no encoding, no disk I/O, buffers reused):

| Threads | Wall clock (600 frames) | Throughput |
|---|---|---|
| 1 | 0.33 s | ~1,800 frames/s |
| 4 | 0.19 s | ~3,200 frames/s |
| 32 | 0.51 s | ~1,100 frames/s |

A single thread already renders ~30× faster than real time at 60fps, which is the
point: **rendering stops being the bottleneck**. What remains is encoding, and
that already runs multi-threaded inside ffmpeg.

**Repeated frames are free.** A frame whose `FrameSignature` — camera position,
current line, number of revealed characters — repeats the previous frame's is not
rasterised again. With the gliding camera (the default) that covers the closing
pause; with `--snap-camera` the camera holds still between characters, so most
frames repeat:

| Camera | fps | Unique frames | All frames | Unique only | Speed-up |
|---|---|---|---|---|---|
| gliding (default) | 30 | 1551 / 1580 | 2.14 s | 2.13 s | 1.00× |
| gliding (default) | 60 | 3101 / 3160 | 4.30 s | 4.36 s | 0.99× |
| `--snap-camera` | 30 | 342 / 1580 | 2.23 s | 0.48 s | **4.63×** |
| `--snap-camera` | 60 | 372 / 3160 | 4.46 s | 0.52 s | **8.63×** |

**Full pipeline** — 16-line snippet (311 typed characters, 790 frames), 480p at
15fps, PNG-sequence output (the slowest sink, since every frame is also
PNG-encoded):

| Scenario | Wall clock | Throughput |
|---|---|---|
| 854×480 @ 15fps → PNG sequence | 0.9 s | 836 frames/s |
| 1920×1080 @ 30fps → PNG sequence | 0.6 s | 213 frames/s |

PNG compression is the most expensive step on that path — 83–89% of the run — so
it happens on a pool of encoder threads rather than on the render thread. The
queue holds at most one frame per thread, which is what keeps memory bounded.

Four practical notes that came out of measuring:

* Always build with `--release` — `tiny-skia`'s rasteriser is ~20× slower
  unoptimised.
* **Reuse frame buffers.** Allocating a fresh 1080p buffer per frame cost about
  6× more than rasterising into it (250 → 1,500 frames/s), so the renderer
  recycles one buffer per worker and rasterises with
  `FrameRenderer::render_into_vec`.
* Frame-parallel rendering buys throughput only while rasterisation is the
  bottleneck. At 1080p the frame buffer alone is 8 MB, so a single thread already
  saturates a good slice of memory bandwidth — beyond ~4 threads the benchmark
  flattens out and then goes backwards on this CPU. The parallelism is there so
  that rendering never becomes the limiting factor, rather than to make an
  already-fast step faster.
* **Layout is linear in the code length.** A 1600-line file (73,470 characters)
  lays out in ~16 ms, and the per-character cost stays flat (~0.22 µs) across a
  35× range of input sizes.

## Testing

```console
$ cargo test
```

The suite covers preprocessing edge cases (empty code, blank lines, inner
spaces, tabs, invalid characters), highlighter invariants (one token kind per
character, for every supported language, and that preprocessing never rewrites
the text the highlighter sees), layout geometry, camera zoom-out, timeline
ordering, frame signatures (equal signatures must rasterise to equal pixels),
custom sinks, progress reporting, cancellation, the ffmpeg command line, and an
end-to-end PNG-sequence render.

## License

MIT, matching the original project.
