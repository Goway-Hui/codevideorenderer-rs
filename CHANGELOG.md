# Changelog

Notable changes to `codevideorenderer`. The grouping follows the original
project's `docs/changelog.rst`: **Additions**, **Changes**, **Deletions**.

## Unreleased

### Additions

- `--glow` and `RenderOptions::glow`: the original's glow post-process, applied
  as one ffmpeg filter on the way out instead of a second encode pass.
- `--snap-camera` and `RenderOptions::snap_camera` (off by default): hold the
  camera still between characters, so most frames repeat and are reused instead
  of rasterised again (measured 4.6×–8.6× faster rasterisation at 1080p).
- `FrameSink` trait and `render_with_sink()`: frames can go anywhere — a stream,
  an object store, a caller-owned encoder — not just to ffmpeg or a PNG
  directory.
- `RenderOptions::progress` (`ProgressCallback`) for per-frame progress, and
  `RenderOptions::cancel` (`Arc<AtomicBool>`) for cooperative cancellation. A
  cancelled render returns `Error::Cancelled` at the next batch boundary.
- A complete `RenderOptions` builder: `with_*` for every field.
- A process-wide font cache. A second render no longer re-reads (13 MB) and
  re-parses the font, and the bytes are no longer leaked per call.
- `RenderReport::fps` (the frame rate you asked for) and
  `RenderReport::reused_frames`.
- `Timeline::signature_at` / `signature_of_frame` and `FrameSignature`: the whole
  input state of a frame, which is what makes frame reuse provably safe.
- `lexer::is_supported()`. The CLI now prints a note when a language falls back
  to the generic highlighter instead of degrading silently.
- `README`: notes on metric-based column alignment with CJK text, and on the
  fallback highlighting behaviour.
- `Prepared`: build the font, layout, timeline and outline cache once, then draw
  as many frames as you like. `render_preview` still rebuilds everything per
  call, which is what you want for one frame and not for a timeline scrubber.
- `PngSink::with_threads()`: PNG frames are compressed by a pool of encoder
  threads (one per core by default) instead of on the render thread.
- `midnight`, a hand-written theme, is now the default, and the UI colours around
  the code are derived from the theme instead of hard-coded: the current-line
  bar, the line numbers and the caret all follow the theme's background and
  foreground, which is what makes the light Pygments styles usable at all.

### Changes

- The published crate is named `codevideorenderer` (it was
  `codevideorenderer-rs`), so depending on it is just `codevideorenderer = "0.1"`
  — no `package = "..."` rename needed. The binary is still `cvr`, and the
  repository keeps the `-rs` suffix to mark it as the Rust port.
- `Cargo.toml` excludes the demo output (`*.mp4`, `frame_*.png`, …) and the
  project's own docs from the package. Without it the demo videos pushed the
  tarball to 22 MiB, past crates.io's 10 MiB limit — `cargo` only honours
  `.gitignore` inside a git repository, and this directory is not one yet.
- `README.md` is now the Simplified Chinese manual, and it documents every option
  in full: a grouped flag reference with defaults and ranges, scenario examples,
  the video-length formula, the `--interval` floor, a reading guide for the
  report output, four library recipes, an FAQ, and the known limits. The English
  original is kept verbatim as `README.en.md`.
- All source comments are now in Simplified Chinese. The code itself is
  untouched: identifiers, `#[error("...")]` messages, the `USAGE`/`EXAMPLE`
  text, the `GLOW_FILTER` chain and every doctest body are byte-for-byte what
  they were, and the 18 files were verified line by line after stripping
  comments.
- The default `--style` is `midnight` (was `material`). In it, punctuation and
  operators take a muted colour close to the foreground, which is the main visual
  difference: brackets and commas stop competing with keywords and strings.
- `midnight`'s text colours were then brightened by 15–20% (foreground
  `#C7D0E0` → `#E6ECF7`; the brightest pixel in a frame went 194 → 218) after the
  first version read as too dim. Hues and the punctuation-recedes rule are
  unchanged.
- `--glow`'s filter chain pins its pixel format (`format=gbrp,...`). Without it,
  ffmpeg's own negotiation between `eq` and `blend` turned the whole frame
  magenta — found by rendering a video with ffmpeg 8.0 and diffing it against the
  same render without `--glow`, and now covered by a regression test.
- PNG sequence output is 4.5–5.7× faster end to end (480p/15fps, 790 frames:
  5.2 s → 0.94 s) now that compression happens off the render thread.
- `layout()` is ~33% faster (1600 lines: 19.5 ms → 13.1 ms): no per-character
  `HashSet` lookup, one `cmap` lookup per character instead of two, and a
  per-token colour cache.
- `render_preview` and the full render share one pipeline construction path.
- Frames are rasterised into recycled per-worker buffers
  (`FrameRenderer::render_into_vec`) rather than a fresh `Pixmap` per frame, and
  a frame whose signature repeats the previous one is not rasterised again.
  End-to-end at 480p/15fps (790 frames): 5.84 s → 5.21 s.
- `layout()` is linear in the code length: the per-character O(N²) index lookup
  is gone. A 1600-line file lays out in ~16 ms instead of ~439 ms, and the
  per-character cost no longer grows with file size.
- The highlighter scans the user's text, unmodified. Inner spaces used to be
  rewritten to a placeholder character before the lexer saw them, which could
  mis-classify tokens (`let x = foo(1, 2);` had its `x` read as a function name).
- `validate()` rejects frames larger than 8K, a negative or non-finite
  `end_pause` (which used to yield a one-frame video), a `crf` above x264's 51, a
  frame rate above 1000, and non-finite `line_spacing`, `camera_scale` or
  interval values — typed errors instead of an allocation abort or a rejected
  ffmpeg invocation. The CLI range-checks `--resolution` while parsing.
- `FfmpegSink::spawn` no longer probes `ffmpeg -version` first: spawning is the
  check, and a missing binary maps onto `Error::FfmpegMissing`.
- The CLI reports the frame rate you asked for, not `frames / duration`
  (`790 @ 15.004708fps` → `790 @ 15fps`).

### Deletions

- `Sink`, the closed enum — replaced by the `FrameSink` trait.
- `config::CURRENT_LINE_HIGHLIGHT`, `config::LINE_NUMBER_COLOR`,
  `config::LINE_NUMBER_ACTIVE_COLOR` and `config::CURSOR_COLOR` — the theme
  derives them from its own background and foreground now.
- `config::OCCUPY_CHARACTER` (inner spaces keep their cell through
  `Preprocessed::inner_spaces`), `config::CODE_BACKGROUND` (the renderer uses
  `theme.background`), `config::FRAME_WIDTH_UNITS`,
  `config::DEFAULT_DEDUP_TOLERANCE`.
- `Font::bytes()`, `Theme::is_dark()`, `Timeline::seconds_per_frame()`,
  `Timeline::reference_width()`, `CameraTrack::first()`,
  `FrameRenderer::glyph_count()`, `Sink::ffmpeg_available()`.
- The always-true `elapsed.is_finite()` check that ran after a render finished.

## 0.1.0

- First release: preprocessing, 19 hand-written grammars with
  Pygments-compatible token kinds, 43 exported Pygments styles, a keyframed
  camera (entrance, follow, zoom-out, sway), rayon-parallel frame rendering into
  a single ffmpeg pipe (or a PNG sequence without ffmpeg).
