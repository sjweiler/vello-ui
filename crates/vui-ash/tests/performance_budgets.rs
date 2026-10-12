use vui_ash::GpuSceneBuilder;
use vui_core::{Color, PaintList, PaintOp, Point, RasterImage, Rect, Size, TextSystem, UiFrame};
use vui_text::ParleyTextSystem;

const NOTO_SANS: &[u8] = include_bytes!("../../vui-snapshot/fonts/NotoSans-wdth-wght.ttf");
const NOTO_SANS_ARABIC: &[u8] =
    include_bytes!("../../vui-snapshot/fonts/NotoSansArabic-wdth-wght.ttf");

#[test]
fn representative_scene_stays_within_structural_budgets() {
    let first_image = RasterImage::new(32, 32, vec![255; 32 * 32 * 4]).unwrap();
    let mut paint = PaintList::default();
    for row in 0_u32..20 {
        for column in 0_u32..20 {
            paint.push(PaintOp::FillRect {
                rect: Rect::new(f64::from(column) * 20.0, f64::from(row) * 20.0, 18.0, 18.0),
                color: Color::WHITE,
                radius: 3.0,
            });
        }
    }
    paint.push(PaintOp::Image {
        image: first_image,
        rect: Rect::new(420.0, 0.0, 64.0, 64.0),
    });
    let frame = UiFrame {
        viewport: Size::new(512.0, 512.0),
        damage: Vec::new(),
        paint,
        semantics: Vec::new(),
    };
    let mut builder = GpuSceneBuilder::new();
    let first = builder.build(&frame, 1.0).unwrap();
    let first_stats = first.stats();
    assert!(
        first_stats.vertices <= 20_000,
        "vertex budget regressed: {first_stats:?}"
    );
    assert!(
        first_stats.indices <= 40_000,
        "index budget regressed: {first_stats:?}"
    );
    assert!(
        first_stats.draw_commands <= 3,
        "batching budget regressed: {first_stats:?}"
    );
    assert!(
        first_stats.bootstrap_instances <= 12_000,
        "bootstrap instance budget regressed: {first_stats:?}"
    );
    assert!(first_stats.atlas_bytes <= 4 * 1024 * 1024);
    assert_eq!(first_stats.atlas_dirty_bytes, 32 * 32 * 4);
    assert_eq!(first_stats.atlas_occupancy_bytes, 32 * 32 * 4);

    let mut changed = frame.clone();
    changed.paint.push(PaintOp::Image {
        image: RasterImage::new(16, 16, vec![127; 16 * 16 * 4]).unwrap(),
        rect: Rect::new(420.0, 80.0, 32.0, 32.0),
    });
    let changed = builder.build(&changed, 1.0).unwrap();
    let changed_stats = changed.stats();
    assert_eq!(changed_stats.atlas_dirty_bytes, 16 * 16 * 4);
    assert_eq!(changed_stats.atlas_evictions, 0);
    assert_eq!(changed_stats.atlas_rebuilds, first_stats.atlas_rebuilds);
}

#[test]
fn shaped_text_stays_within_structural_budgets() {
    let mut text = ParleyTextSystem::with_embedded_fonts([NOTO_SANS, NOTO_SANS_ARABIC]).unwrap();
    let samples = [
        ("Office affinity — AVATAR", 12.0_f32),
        ("مرحبا بالعالم", 16.0_f32),
        ("Small labels and 0123456789", 10.0_f32),
        ("Large shaped typography", 24.0_f32),
    ];
    let mut paint = PaintList::default();
    for line in 0_u32..16 {
        let (content, size) = samples[line as usize % samples.len()];
        for run in text.layout(content, size, Some(760.0)).runs {
            paint.push(PaintOp::GlyphRun {
                origin: Point::new(12.0, 28.0 + f64::from(line) * 36.0),
                font: run.font,
                font_size: run.font_size,
                normalized_coords: run.normalized_coords,
                glyphs: run.glyphs,
                color: Color::WHITE,
            });
        }
    }
    let scene = GpuSceneBuilder::new()
        .build(
            &UiFrame {
                viewport: Size::new(800.0, 640.0),
                damage: Vec::new(),
                paint,
                semantics: Vec::new(),
            },
            1.0,
        )
        .unwrap();
    let stats = scene.stats();
    assert!(
        stats.vertices <= 10_000,
        "vertex budget regressed: {stats:?}"
    );
    assert!(stats.indices <= 30_000, "index budget regressed: {stats:?}");
    assert!(stats.draw_commands <= 2, "batching regressed: {stats:?}");
    assert_eq!(stats.bootstrap_instances, 0);
}
