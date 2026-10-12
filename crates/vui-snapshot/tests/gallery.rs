use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard},
    time::Duration,
};
use vui_core::{
    Button, ButtonVariant, Checkbox, Column, Event, Insets, Key, Label, Modifier, Modifiers,
    Padding, Point, ProgressBar, Size, TextArea, TextField, TextSystem, Theme, Widget,
};
use vui_snapshot::{
    SnapshotConfig, SnapshotPaths, SnapshotRenderer, SnapshotTolerance, verify_snapshot,
};
use vui_testing::TestHarness;
use vui_text::ParleyTextSystem;

const NOTO_SANS: &[u8] = include_bytes!("../fonts/NotoSans-wdth-wght.ttf");
const NOTO_SANS_ARABIC: &[u8] = include_bytes!("../fonts/NotoSansArabic-wdth-wght.ttf");

const GALLERY_TOLERANCE: SnapshotTolerance = SnapshotTolerance {
    channel_delta: 4,
    max_changed_pixels: 700,
    max_changed_ratio: 0.01,
};

// Keep Vulkan contexts mutually exclusive so software drivers do not compete
// for memory or make snapshot timing needlessly noisy.
static GPU_SNAPSHOT_LOCK: Mutex<()> = Mutex::new(());

fn lock_gpu_snapshots() -> MutexGuard<'static, ()> {
    GPU_SNAPSHOT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn gallery() -> impl Widget {
    Padding::new(
        Insets::all(24.0),
        Column::new([
            Box::new(Label::new("VUI COMPONENT GALLERY").text_size(22.0)) as Box<dyn Widget>,
            Box::new(Button::new("Primary action", || {})),
            Box::new(Button::new("Secondary action", || {}).variant(ButtonVariant::Secondary)),
            Box::new(Button::new("Destructive action", || {}).variant(ButtonVariant::Destructive)),
            Box::new(Button::new("Disabled action", || {}).disabled(true)),
            Box::new(Checkbox::new("Selected option", true, |_| {})),
            Box::new(Checkbox::new("Available option", false, |_| {})),
            Box::new(ProgressBar::new(0.68).label("68 percent")),
            Box::new(Label::new("UNICODE FALLBACK: مرحبا بالعالم")),
        ])
        .gap(12.0),
    )
}

fn snapshot_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn embedded_text() -> ParleyTextSystem {
    ParleyTextSystem::with_embedded_fonts([NOTO_SANS, NOTO_SANS_ARABIC])
        .expect("checked-in Noto fixtures are valid")
}

#[test]
fn embedded_fonts_shape_ligatures_and_rtl_without_fallback() {
    let mut text = embedded_text();
    let ligature = text.layout("office", 18.0, None);
    let ligature_glyphs = ligature
        .runs
        .iter()
        .map(|run| run.glyphs.len())
        .sum::<usize>();
    assert!(ligature.fallback_text.is_none());
    assert!(ligature_glyphs < "office".chars().count());

    let rtl = text.layout("مرحبًا بالعالم", 18.0, None);
    assert!(rtl.fallback_text.is_none());
    assert!(!rtl.runs.is_empty());
    assert!(rtl.carets.windows(2).any(|pair| pair[1].x < pair[0].x));
}

fn typography_gallery() -> impl Widget {
    Padding::new(
        Insets::all(24.0),
        Column::new([
            Box::new(Label::new("DETERMINISTIC TYPOGRAPHY").text_size(24.0)) as Box<dyn Widget>,
            Box::new(Label::new("Ligatures: office affine efficient")),
            Box::new(Label::new("Combining: Cafe\u{301} — nai\u{308}ve — A\u{30a}ngstro\u{308}m")),
            Box::new(Label::new("العربية: مرحبًا بالعالم")),
            Box::new(Label::new("Mixed bidi: VUI 2026 — مرحبًا — ready")),
            Box::new(
                TextArea::new(
                    "Wrapping uses the same embedded fonts across every machine. العربية تلتف أيضًا داخل المساحة المحددة.",
                    |_| {},
                )
                .min_lines(4),
            ),
        ])
        .gap(14.0),
    )
}

#[test]
fn component_gallery_visual_states() {
    if std::env::var_os("VUI_SKIP_GPU_SNAPSHOTS").is_some() {
        return;
    }
    let _gpu_guard = lock_gpu_snapshots();
    let mut renderer = match SnapshotRenderer::new() {
        Ok(renderer) => renderer,
        Err(error) if std::env::var_os("VUI_REQUIRE_GPU_SNAPSHOTS").is_none() => {
            eprintln!("skipping GPU gallery snapshots: {error}");
            return;
        }
        Err(error) => panic!("GPU snapshots are required: {error}"),
    };
    let paths = SnapshotPaths::new(snapshot_root());
    for (name, theme) in [
        ("components_light", Theme::light()),
        ("components_dark", Theme::dark()),
        ("components_high_contrast", Theme::high_contrast()),
    ] {
        let image = renderer
            .capture_widget(
                gallery(),
                SnapshotConfig::new(Size::new(520.0, 480.0), theme),
            )
            .expect("render component gallery");
        verify_snapshot(name, &image, &paths, GALLERY_TOLERANCE)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    let theme = Theme::light();
    let mut interaction = TestHarness::new(
        Padding::new(Insets::all(24.0), Button::new("Focused and hovered", || {})),
        Size::new(320.0, 100.0),
    );
    interaction.set_theme(theme);
    interaction.frame();
    interaction.event(&Event::KeyDown(Key::Tab));
    interaction.event(&Event::PointerMoved(Point::new(30.0, 30.0)));
    interaction.advance_by(Duration::from_millis(1));
    let image = renderer
        .capture_harness(
            &mut interaction,
            SnapshotConfig::new(Size::new(320.0, 100.0), theme),
        )
        .expect("render interaction states");
    verify_snapshot("interaction_focus_hover", &image, &paths, GALLERY_TOLERANCE)
        .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn embedded_font_typography_and_editor_states() {
    if std::env::var_os("VUI_SKIP_GPU_SNAPSHOTS").is_some() {
        return;
    }
    let _gpu_guard = lock_gpu_snapshots();
    let mut renderer = match SnapshotRenderer::new() {
        Ok(renderer) => renderer,
        Err(error) if std::env::var_os("VUI_REQUIRE_GPU_SNAPSHOTS").is_none() => {
            eprintln!("skipping GPU typography snapshots: {error}");
            return;
        }
        Err(error) => panic!("GPU snapshots are required: {error}"),
    };
    let paths = SnapshotPaths::new(snapshot_root());
    let theme = Theme::light();

    for (name, scale) in [
        ("typography_1x", 1.0),
        ("typography_1_25x", 1.25),
        ("typography_2x", 2.0),
    ] {
        let size = Size::new(640.0, 400.0);
        let mut harness =
            TestHarness::with_text_system(typography_gallery(), size, embedded_text());
        let image = renderer
            .capture_harness(
                &mut harness,
                SnapshotConfig::new(size, theme).scale_factor(scale),
            )
            .expect("render embedded-font typography gallery");
        verify_snapshot(name, &image, &paths, GALLERY_TOLERANCE)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    let selection_size = Size::new(520.0, 100.0);
    let selection = Padding::new(
        Insets::all(24.0),
        TextField::new("Select combining text: Cafe\u{301}", |_| {}),
    );
    let mut selection = TestHarness::with_text_system(selection, selection_size, embedded_text());
    let _ = selection.frame();
    assert!(selection.event(&Event::KeyDown(Key::Tab)));
    let mut shift = Modifiers::default();
    shift.set(Modifier::Shift, true);
    let _ = selection.event(&Event::ModifiersChanged(shift));
    for _ in 0..5 {
        let _ = selection.event(&Event::KeyDown(Key::ArrowLeft));
    }
    let image = renderer
        .capture_harness(&mut selection, SnapshotConfig::new(selection_size, theme))
        .expect("render shaped selection state");
    verify_snapshot("editor_selection", &image, &paths, GALLERY_TOLERANCE)
        .unwrap_or_else(|error| panic!("{error}"));

    let ime_size = Size::new(520.0, 180.0);
    let ime = Padding::new(
        Insets::all(24.0),
        TextArea::new("IME composition: ", |_| {}).min_lines(3),
    );
    let mut ime = TestHarness::with_text_system(ime, ime_size, embedded_text());
    let _ = ime.frame();
    assert!(ime.event(&Event::KeyDown(Key::Tab)));
    assert!(ime.event(&Event::ImePreedit {
        text: "مَرْحَبًا".to_owned(),
        cursor: Some((0, "مَرْحَبًا".len())),
    }));
    let image = renderer
        .capture_harness(
            &mut ime,
            SnapshotConfig::new(ime_size, theme).scale_factor(1.25),
        )
        .expect("render shaped IME composition state");
    verify_snapshot("editor_ime_1_25x", &image, &paths, GALLERY_TOLERANCE)
        .unwrap_or_else(|error| panic!("{error}"));
}
