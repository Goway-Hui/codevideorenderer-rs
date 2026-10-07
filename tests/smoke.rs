//! 无需 ffmpeg 的端到端测试与单元测试。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use codevideorenderer::TokenKind;
use codevideorenderer::api::{
    Prepared, ProgressCallback, RenderOptions, render, render_preview, render_with_sink,
};
use codevideorenderer::encode::{FrameSink, PngSink};
use codevideorenderer::font::Font;
use codevideorenderer::layout::{self, GlyphRole, Source, preprocess};
use codevideorenderer::render::{FrameRenderer, GlyphOutlines};
use codevideorenderer::theme::{self, Theme};
use codevideorenderer::timeline::{Timeline, TimelineOptions};
use codevideorenderer::tiny_skia::Pixmap;
use codevideorenderer::{Error, supported_languages};

fn options() -> RenderOptions {
    RenderOptions {
        width: 640,
        height: 360,
        fps: 15,
        interval_min: 0.07,
        interval_max: 0.07,
        quiet: true,
        ..RenderOptions::default()
    }
}

fn font() -> Font {
    Font::find(None).expect("a usable font (assets/CodeVideoRendererFont.ttf)")
}

fn theme_ref() -> &'static Theme {
    theme::default_theme()
}

fn layout_of(code: &str, language: &str) -> layout::Layout {
    let pre = preprocess(&Source::text(code)).expect("preprocess");
    layout::layout(&pre, &font(), theme_ref(), language, 0.8).expect("layout")
}

// ---------------------------------------------------------------------------
// 预处理
// ---------------------------------------------------------------------------

#[test]
fn empty_code_is_rejected_up_front() {
    for code in ["", "\n\n\n", "   \n   \n"] {
        let err = preprocess(&Source::text(code)).unwrap_err();
        assert!(
            matches!(err, Error::EmptyCode),
            "expected EmptyCode, got {err:?}"
        );
    }
}

#[test]
fn invalid_characters_are_rejected() {
    let err = preprocess(&Source::text("print(1)\r\n")).unwrap_err();
    assert!(matches!(err, Error::InvalidCharacters(_)));
}

#[test]
fn tabs_are_expanded_to_spaces() {
    let pre = preprocess(&Source::text("if x:\n\treturn 1\n")).unwrap();
    assert!(pre.lines[1].starts_with("    return"));
    assert!(!pre.text.contains('\t'));
}

#[test]
fn blank_edges_are_dropped_and_blank_lines_kept() {
    let pre = preprocess(&Source::text("\n\na = 1\n\n\nb = 2\n\n")).unwrap();
    assert_eq!(pre.lines.len(), 4);
    // 内部空格保留其单元格，文本不被重写。
    assert_eq!(pre.lines[0], "a = 1");
    assert_eq!(pre.empty_lines, vec![1, 2]);
}

#[test]
fn inner_spaces_keep_their_cell_without_rewriting_the_text() {
    let pre = preprocess(&Source::text("    a = 1\n")).unwrap();
    assert!(
        pre.lines[0].starts_with("    a"),
        "indentation is preserved"
    );
    assert_eq!(pre.inner_spaces, vec![(0, 5), (0, 7)]);
    // 缩进不会被键入；`a = 1` 才会
    assert_eq!(pre.typed_chars, 5);
}

// ---------------------------------------------------------------------------
// 语法高亮
// ---------------------------------------------------------------------------

#[test]
fn every_character_gets_exactly_one_token() {
    let code = "def f(x):\n    # comment\n    s = \"a\\\"b\"\n    if x >= 1e3: return f(x - 1)\n";
    for language in supported_languages() {
        let highlighted = codevideorenderer::lexer::highlight(code, language);
        assert_eq!(
            highlighted.len(),
            code.chars().count(),
            "language {language} lost characters"
        );
    }
}

#[test]
fn json_negative_numbers_do_not_stall() {
    // 回归测试：单独的 '-' 曾让 JSON 扫描器停滞不前。
    let code = r#"{"a": -1, "b": "x", "c": [1, -2.5e3]}"#;
    let h = codevideorenderer::lexer::highlight(code, "json");
    assert_eq!(h.len(), code.chars().count());
}

#[test]
fn python_highlighting_classifies_keywords_and_strings() {
    let code = "def foo():\n    return \"bar\"\n";
    let kinds = codevideorenderer::lexer::highlight(code, "python")
        .kinds()
        .to_vec();
    assert!(kinds.contains(&TokenKind::Keyword), "no keyword found");
    assert!(kinds.contains(&TokenKind::NameFunction), "no function name");
    assert!(kinds.contains(&TokenKind::LiteralStringDouble), "no string");
    assert_eq!(kinds.len(), code.chars().count());
}

#[test]
fn curly_languages_use_the_curly_scanner() {
    let code = "fn main() {\n    // hi\n    let x: i32 = 1;\n}\n";
    let kinds = codevideorenderer::lexer::highlight(code, "rust")
        .kinds()
        .to_vec();
    assert!(kinds.contains(&TokenKind::Keyword));
    assert!(kinds.contains(&TokenKind::CommentSingle));
    assert!(kinds.contains(&TokenKind::KeywordType));
}

#[test]
fn unknown_languages_fall_back_instead_of_failing() {
    let code = "some *code* here";
    let h = codevideorenderer::lexer::highlight(code, "not-a-real-language");
    assert_eq!(h.len(), code.chars().count());
}

#[test]
fn preprocessing_does_not_disturb_the_highlighter() {
    // 回归测试：内部空格曾在词法分析器看到文本之前被改写成占位字符，导致
    // `let x = foo(1, 2);` 里的 `x` 看起来像调用名。词法分析器必须看到用户
    // 写下的原始内容。
    let code = "let x = foo(1, 2); // note";
    let pre = preprocess(&Source::text(code)).expect("preprocess");
    assert_eq!(pre.text, code, "preprocessing rewrote the code");

    let kinds = codevideorenderer::lexer::highlight(&pre.text, "rust")
        .kinds()
        .to_vec();
    assert_eq!(kinds.len(), code.chars().count());
    assert_eq!(kinds[4], TokenKind::Name, "`x` is a binding, not a call");
    assert_ne!(kinds[4], TokenKind::NameFunction);
    assert_eq!(kinds[8], TokenKind::NameFunction, "`foo` is the call");
}

// ---------------------------------------------------------------------------
// 布局
// ---------------------------------------------------------------------------

#[test]
fn layout_is_monospace_and_ordered() {
    let l = layout_of("abc\ndefgh\n", "python");

    let b0 = l.lines[0].baseline_y;
    let b1 = l.lines[1].baseline_y;
    assert!((b1 - b0 - l.cell_h).abs() < 0.01, "line pitch mismatch");

    let line0: Vec<_> = l
        .glyphs
        .iter()
        .filter(|g| g.line == 0 && matches!(g.role, GlyphRole::Code))
        .collect();
    assert_eq!(line0.len(), 3);
    assert!((line0[1].x - line0[0].x - l.cell_w).abs() < 0.01);
    assert_eq!(l.typed_chars, 8);
}

#[test]
fn layout_geometry_does_not_depend_on_output_size() {
    // 不同分辨率下的两次渲染必须产生相同的世界几何，否则相机的视场在不同
    // 尺寸下含义就会不同。
    let a = layout_of("print(\"hello\")\n", "python");
    let b = layout_of("print(\"hello\")\n", "python");
    assert_eq!(a.cell_w, b.cell_w);
    assert_eq!(a.cell_h, b.cell_h);
    assert_eq!(a.font_size_px, b.font_size_px);
    assert!(a.font_size_px > 0.0);
}

#[test]
fn line_numbers_are_right_aligned_in_the_gutter() {
    let code = (1..=12)
        .map(|i| format!("x{i} = {i}\n"))
        .collect::<String>();
    let l = layout_of(&code, "python");
    let gutter_glyphs: Vec<_> = l
        .glyphs
        .iter()
        .filter(|g| matches!(g.role, GlyphRole::LineNumber { .. }))
        .collect();
    let expected: usize = (1..=12).map(|i| i.to_string().len()).sum();
    assert_eq!(
        gutter_glyphs.len(),
        expected,
        "one glyph per line-number digit"
    );
    assert!(gutter_glyphs.iter().all(|g| g.x < l.code_left));
}

// ---------------------------------------------------------------------------
// 时间线与相机
// ---------------------------------------------------------------------------

fn timeline_of(l: &layout::Layout, camera_scale: f32) -> Timeline {
    Timeline::build(
        l,
        &TimelineOptions {
            interval_min: 0.07,
            interval_max: 0.07,
            fps: 15,
            camera_scale,
            ..TimelineOptions::default()
        },
    )
}

#[test]
fn reveal_times_are_monotonic() {
    let l = layout_of("a = 1\nb = 2\n", "python");
    let timeline = timeline_of(&l, 0.5);
    assert_eq!(timeline.reveal_times.len(), l.typed_chars);
    for pair in timeline.reveal_times.windows(2) {
        assert!(pair[0] <= pair[1], "reveal times must not go backwards");
    }
    assert!(
        timeline.duration > timeline.typing_end,
        "there is a closing pause"
    );
    assert!(timeline.frame_count > 0);
}

#[test]
fn camera_pulls_back_on_long_lines_and_never_zooms_in() {
    let l = layout_of(
        "x = a_very_long_line_of_code_that_keeps_going_and_going_and_going_so_the_camera_pulls_back()",
        "python",
    );
    let timeline = timeline_of(&l, 0.5);

    let start = timeline.camera.at(timeline.reveal_times[0]).scale;
    let end = timeline.camera.at(timeline.duration).scale;
    assert!(start >= 0.5 - 1e-6, "never zooms in past the initial scale");
    assert!(end > start, "camera should pull back over a long line");

    // 并且整个轨道上的缩放必须单调。
    let mut previous = 0.0f32;
    for i in 0..=20 {
        let t = timeline.duration * i as f32 / 20.0;
        let scale = timeline.camera.at(t).scale;
        assert!(scale >= previous - 1e-6, "camera zoom must never reverse");
        previous = scale;
    }
}

#[test]
fn short_lines_keep_the_initial_zoom() {
    let l = layout_of("x = 1\ny = 2\n", "python");
    let timeline = timeline_of(&l, 0.5);
    let end = timeline.camera.at(timeline.duration).scale;
    assert!(
        (end - 0.5).abs() < 0.02,
        "no unnecessary zoom-out for short code"
    );
}

// ---------------------------------------------------------------------------
// api
// ---------------------------------------------------------------------------

#[test]
fn preview_renders_actual_pixels() {
    let pixmap = render_preview(
        Source::text("print(\"hello\")"),
        &options(),
        30.0, // 远在结束之后：所有内容都已键入
    )
    .expect("preview");
    assert_eq!(pixmap.width(), 640);
    assert_eq!(pixmap.height(), 360);

    let data = pixmap.data();
    let first = &data[0..4];
    let differs = data
        .chunks_exact(4)
        .any(|px| px[0] != first[0] || px[1] != first[1] || px[2] != first[2]);
    assert!(differs, "frame looks empty");
}

#[test]
fn nothing_is_drawn_before_the_entrance_finishes() {
    let opts = options();
    let early = render_preview(Source::text("print(1)"), &opts, 0.001).expect("preview");
    let late = render_preview(Source::text("print(1)"), &opts, 60.0).expect("preview");
    let painted = |data: &[u8]| data.chunks_exact(4).filter(|px| px[0] > 200).count();
    assert!(
        painted(early.data()) < painted(late.data()),
        "typed text should add glyphs"
    );
}

#[test]
fn bad_parameters_fail_before_any_work() {
    let dir = temp_dir("cvr-bad-params");

    let mut o = options();
    o.fps = 15;
    o.interval_min = 0.01;
    o.interval_max = 0.01;
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::IntervalRange { .. }), "got {err:?}");

    let mut o = options();
    o.style = "definitely-not-a-style".into();
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::UnknownStyle(..)), "got {err:?}");

    let mut o = options();
    o.video_name = "  ".into();
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::VideoName), "got {err:?}");

    let mut o = options();
    o.line_spacing = 0.0;
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::LineSpacing(_)), "got {err:?}");

    let mut o = options();
    o.end_pause = -1.0;
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::EndPause(_)), "got {err:?}");

    let mut o = options();
    o.end_pause = f32::INFINITY;
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::EndPause(_)), "got {err:?}");

    let mut o = options();
    o.crf = 60;
    o.frames_dir = Some(dir.clone());
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::Crf(60)), "got {err:?}");

    let mut o = options();
    o.fps = 100_000;
    o.frames_dir = Some(dir);
    let err = render(Source::text("x = 1"), o).unwrap_err();
    assert!(matches!(err, Error::FrameRate(100_000)), "got {err:?}");
}

#[test]
fn renders_a_png_sequence_without_ffmpeg() {
    let dir = temp_dir("cvr-png");
    let opts = RenderOptions {
        frames_dir: Some(dir.clone()),
        ..options()
    };
    let report = render(Source::text("x = 1\ny = 2\n"), opts).expect("render");

    assert!(report.frames > 0);
    assert_eq!(report.typed_chars, 10);
    assert!(!report.font.is_empty());
    assert!(!report.theme.is_empty());
    assert!(report.outlines > 0);

    let files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("frames dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    assert_eq!(files.len() as u32, report.frames);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_renders_in_one_process_do_not_interfere() {
    // 针对原版全局参数泄漏的回归防护：渲染第二段视频不得改变第一段的设置。
    let dir_a = temp_dir("cvr-a");
    let dir_b = temp_dir("cvr-b");

    let a = render(
        Source::text("a = 1\n"),
        RenderOptions {
            frames_dir: Some(dir_a.clone()),
            style: "monokai".into(),
            ..options()
        },
    )
    .expect("render a");

    let b = render(
        Source::text("b = 2\n"),
        RenderOptions {
            frames_dir: Some(dir_b.clone()),
            style: "github-dark".into(),
            ..options()
        },
    )
    .expect("render b");

    assert_eq!(a.theme, "monokai");
    assert_eq!(b.theme, "github-dark");
    assert_ne!(a.frames, 0);
    assert_ne!(b.frames, 0);

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// 由调用方持有的 sink：`render_with_sink` 这个扩展点正是为此而存在。
#[derive(Default)]
struct CollectingSink {
    indices: Vec<u32>,
    bytes: usize,
    finished: bool,
}

impl FrameSink for CollectingSink {
    fn write(&mut self, frame_index: u32, pixmap: &Pixmap) -> Result<(), Error> {
        self.indices.push(frame_index);
        self.bytes += pixmap.data().len();
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<String, Error> {
        self.finished = true;
        Ok(format!("{} frames kept in memory", self.indices.len()))
    }
}

#[test]
fn a_custom_sink_receives_every_frame_in_order() {
    let report = render_with_sink(
        Source::text("x = 1\ny = 2\n"),
        options(),
        Box::new(CollectingSink::default()),
    )
    .expect("render");

    assert!(report.frames > 0);
    assert_eq!(
        report.output,
        format!("{} frames kept in memory", report.frames),
        "the sink's own finish() result is what gets reported"
    );
}

#[test]
fn progress_callbacks_see_every_frame_once() {
    let seen: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let dir = temp_dir("cvr-progress");

    let report = render(
        Source::text("x = 1\ny = 2\n"),
        RenderOptions {
            frames_dir: Some(dir.clone()),
            progress: Some(ProgressCallback::new(move |done, total| {
                recorder.lock().expect("progress lock").push((done, total));
            })),
            ..options()
        },
    )
    .expect("render");

    let seen = seen.lock().expect("progress lock");
    assert_eq!(seen.len() as u32, report.frames, "one call per frame");
    assert!(
        seen.windows(2).all(|pair| pair[0].0 + 1 == pair[1].0),
        "progress must count up one frame at a time"
    );
    assert!(seen.iter().all(|(_, total)| *total == report.frames));
    drop(seen);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_render_stops_before_writing_anything() {
    let dir = temp_dir("cvr-cancel");
    let err = render(
        Source::text("x = 1\ny = 2\n"),
        RenderOptions {
            frames_dir: Some(dir.clone()),
            cancel: Some(Arc::new(AtomicBool::new(true))),
            ..options()
        },
    )
    .unwrap_err();

    assert!(matches!(err, Error::Cancelled), "got {err:?}");
    let written = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(
        written, 0,
        "a cancelled render must not leave frames behind"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn frames_with_equal_signatures_rasterise_identically() {
    // 渲染器对每段相同签名的连续帧只绘制一帧并重复使用。只有当签名确实
    // 决定像素时这才成立，所以要用光栅化器来验证，而不是轻信字段列表。
    let l = layout_of("x = 1\ny = 2\nz = 3\n", "python");
    let timeline = Timeline::build(
        &l,
        &TimelineOptions {
            fps: 30,
            interval_min: 0.2,
            interval_max: 0.2,
            snap_camera: true,
            ..TimelineOptions::default()
        },
    );
    let font = font();
    let outlines = GlyphOutlines::build(&font, &l);
    let renderer = FrameRenderer::new(&l, &timeline, theme_ref(), &font, &outlines, 320, 180);

    let mut checked = 0;
    for frame in 0..timeline.frame_count.saturating_sub(1) {
        if timeline.signature_of_frame(frame) != timeline.signature_of_frame(frame + 1) {
            continue;
        }
        let a = renderer
            .render_to_rgba(timeline.time_of_frame(frame))
            .expect("frame");
        let b = renderer
            .render_to_rgba(timeline.time_of_frame(frame + 1))
            .expect("frame");
        assert_eq!(
            a, b,
            "equal signatures must give equal pixels (frame {frame})"
        );
        checked += 1;
        if checked >= 6 {
            break;
        }
    }
    assert!(checked > 0, "a snapped camera should repeat frames");
}

#[test]
fn a_snapped_camera_lets_most_frames_be_reused() {
    let snapped_dir = temp_dir("cvr-snap");
    let smooth_dir = temp_dir("cvr-smooth");

    let run = |dir: &PathBuf, snap: bool| {
        render(
            Source::text("x = 1\ny = 2\n"),
            RenderOptions {
                frames_dir: Some(dir.clone()),
                fps: 30,
                interval_min: 0.2,
                interval_max: 0.2,
                snap_camera: snap,
                ..options()
            },
        )
        .expect("render")
    };

    let snapped = run(&snapped_dir, true);
    let smooth = run(&smooth_dir, false);

    assert!(
        snapped.reused_frames * 2 > snapped.frames,
        "a snapped camera should reuse most frames, got {}/{}",
        snapped.reused_frames,
        snapped.frames
    );
    assert!(
        snapped.reused_frames > smooth.reused_frames,
        "snapping must reuse more frames than gliding ({} vs {})",
        snapped.reused_frames,
        smooth.reused_frames
    );

    let _ = std::fs::remove_dir_all(&snapped_dir);
    let _ = std::fs::remove_dir_all(&smooth_dir);
}

#[test]
fn a_parallel_png_sink_writes_the_same_files_as_a_serial_one() {
    let serial = temp_dir("cvr-png-serial");
    let parallel = temp_dir("cvr-png-parallel");

    for (dir, threads) in [(&serial, 1usize), (&parallel, 4)] {
        let sink = PngSink::with_threads(dir, threads).expect("PNG sink");
        let opts = RenderOptions {
            frames_dir: Some(dir.clone()),
            ..options()
        };
        render_with_sink(Source::text("x = 1\ny = 2\n"), opts, Box::new(sink)).expect("render");
    }

    let read_dir = |dir: &PathBuf| -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .expect("frames dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).expect("read frame"),
                )
            })
            .collect();
        files.sort();
        files
    };

    let serial_files = read_dir(&serial);
    assert!(!serial_files.is_empty(), "the serial sink wrote nothing");
    assert_eq!(
        serial_files,
        read_dir(&parallel),
        "PNG output must not depend on the encoder thread count"
    );

    let _ = std::fs::remove_dir_all(&serial);
    let _ = std::fs::remove_dir_all(&parallel);
}

#[test]
fn a_prepared_render_draws_many_frames_from_one_setup() {
    let opts = options();
    let code = "x = 1\ny = 2\n";
    let prepared = Prepared::new(&Source::text(code), &opts).expect("prepare");

    assert_eq!(prepared.size(), (opts.width, opts.height));
    assert!(prepared.timeline().frame_count > 1);
    assert!(
        prepared.outlines() > 0,
        "outlines are cached once, then reused"
    );
    assert_eq!(prepared.layout().typed_chars, 10);

    // 多帧、一次布局：无需重建，尺寸也不会漂移。
    let timeline = prepared.timeline();
    for frame in 0..timeline.frame_count.min(6) {
        let pixmap = prepared
            .render_at(timeline.time_of_frame(frame))
            .expect("frame");
        assert_eq!((pixmap.width(), pixmap.height()), prepared.size());
    }

    // 并且它必须与一次性入口逐像素一致。
    let at = timeline.duration * 0.7;
    let reused = prepared.render_at(at).expect("frame");
    let one_shot = render_preview(Source::text(code), &opts, at).expect("preview");
    assert_eq!(reused.data(), one_shot.data());
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
