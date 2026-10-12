use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use vui_ash::{GpuScene, GpuSceneBuilder, OffscreenRenderer};
use vui_core::{Color, PaintList, PaintOp, Point, RasterImage, Rect, Size, TextSystem, UiFrame};
use vui_text::ParleyTextSystem;

const NOTO_SANS: &[u8] = include_bytes!("../../vui-snapshot/fonts/NotoSans-wdth-wght.ttf");
const NOTO_SANS_ARABIC: &[u8] =
    include_bytes!("../../vui-snapshot/fonts/NotoSansArabic-wdth-wght.ttf");

fn main() {
    if cfg!(debug_assertions) {
        println!("renderer benchmark skipped in debug mode; run `cargo bench -p vui-ash`");
        return;
    }
    let frame = representative_frame(0);
    let primitives = shapes_frame();
    let text = text_frame();
    let font_text = shaped_text_frame();

    measure("scene/cold", 30, || {
        let mut builder = GpuSceneBuilder::new();
        black_box(builder.build(black_box(&frame), 1.25).unwrap());
    });
    measure("scene/cold-shapes", 30, || {
        let mut builder = GpuSceneBuilder::new();
        black_box(builder.build(black_box(&primitives), 1.25).unwrap());
    });
    measure("scene/cold-bootstrap-text", 30, || {
        let mut builder = GpuSceneBuilder::new();
        black_box(builder.build(black_box(&text), 1.25).unwrap());
    });
    measure("scene/cold-shaped-text", 30, || {
        let mut builder = GpuSceneBuilder::new();
        black_box(builder.build(black_box(&font_text), 1.25).unwrap());
    });

    let mut cached_builder = GpuSceneBuilder::new();
    cached_builder.build(&frame, 1.25).unwrap();
    measure("scene/cached", 20_000, || {
        black_box(cached_builder.build(black_box(&frame), 1.25).unwrap());
    });
    let mut font_builder = GpuSceneBuilder::new();
    font_builder.build(&font_text, 1.25).unwrap();
    measure("scene/cached-shaped-text", 20_000, || {
        black_box(font_builder.build(black_box(&font_text), 1.25).unwrap());
    });

    let churn_frames: Vec<_> = (0..32).map(representative_frame).collect();
    let mut churn_builder = GpuSceneBuilder::new();
    let mut churn_index = 0_usize;
    measure("scene/atlas-churn", 100, || {
        let frame = &churn_frames[churn_index % churn_frames.len()];
        churn_index += 1;
        black_box(churn_builder.build(black_box(frame), 1.0).unwrap());
    });

    match OffscreenRenderer::new_with_sample_count(4) {
        Ok(mut renderer) => {
            let mut builder = GpuSceneBuilder::new();
            let scene = builder.build(&frame, 1.0).unwrap();
            let primitive_scene = builder.build(&primitives, 1.0).unwrap();
            let text_scene = builder.build(&text, 1.0).unwrap();
            let font_scene = builder.build(&font_text, 1.0).unwrap();
            println!("metric scene {:?}", scene.stats());
            println!("metric shaped-scene {:?}", font_scene.stats());
            renderer.render(&scene, Color::BLACK).unwrap();
            let cold_uploads = renderer.last_upload_stats();
            measure_offscreen("gpu/offscreen-4x", &mut renderer, &scene);
            measure_offscreen("gpu/offscreen-shapes", &mut renderer, &primitive_scene);
            measure_offscreen("gpu/offscreen-text", &mut renderer, &text_scene);
            measure_offscreen("gpu/offscreen-shaped-text", &mut renderer, &font_scene);
            let cached_uploads = renderer.last_upload_stats();
            println!(
                "metric upload/cold geometry_bytes={} atlas_bytes={}",
                cold_uploads.geometry_bytes, cold_uploads.atlas_bytes
            );
            println!(
                "metric upload/cached geometry_bytes={} atlas_bytes={}",
                cached_uploads.geometry_bytes, cached_uploads.atlas_bytes
            );
            println!("device {}", renderer.device_info());
        }
        Err(error) => println!("gpu/offscreen skipped: {error}"),
    }
    for samples in [1, 2] {
        match OffscreenRenderer::new_with_sample_count(samples) {
            Ok(mut renderer) => {
                let mut builder = GpuSceneBuilder::new();
                let scene = builder.build(&frame, 1.0).unwrap();
                renderer.render(&scene, Color::BLACK).unwrap();
                measure_offscreen(&format!("gpu/offscreen-{samples}x"), &mut renderer, &scene);
                println!("device/{samples}x {}", renderer.device_info());
            }
            Err(error) => println!("gpu/offscreen-{samples}x skipped: {error}"),
        }
    }
}

fn measure_offscreen(name: &str, renderer: &mut OffscreenRenderer, scene: &GpuScene) {
    const ITERATIONS: u32 = 20;
    for _ in 0..10 {
        black_box(renderer.render(black_box(scene), Color::BLACK).unwrap());
    }
    let mut resource_wait = 0_u128;
    let mut prepare = 0_u128;
    let mut gpu_wait = 0_u128;
    let mut readback = 0_u128;
    let mut gpu = 0_u128;
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(renderer.render(black_box(scene), Color::BLACK).unwrap());
        let timings = renderer.last_timing_stats();
        resource_wait += timings.resource_wait.as_nanos();
        prepare += timings.prepare.as_nanos();
        gpu_wait += timings.gpu_wait.as_nanos();
        readback += timings.readback.as_nanos();
        gpu += timings.gpu.map_or(0, |duration| duration.as_nanos());
    }
    report(name, ITERATIONS, started.elapsed());
    let count = u128::from(ITERATIONS);
    println!(
        "metric phase/{name} resource_wait_ns={} prepare_ns={} gpu_wait_ns={} readback_ns={} gpu_ns={}",
        resource_wait / count,
        prepare / count,
        gpu_wait / count,
        readback / count,
        gpu / count,
    );
}

fn measure(name: &str, iterations: u32, mut operation: impl FnMut()) {
    for _ in 0..iterations.min(10) {
        operation();
    }
    let started = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    report(name, iterations, started.elapsed());
}

fn report(name: &str, iterations: u32, elapsed: Duration) {
    let nanos = elapsed.as_nanos() / u128::from(iterations);
    let per_second = f64::from(iterations) / elapsed.as_secs_f64();
    println!(
        "benchmark {name} iterations={iterations} ns_per_iteration={nanos} operations_per_second={per_second:.2}"
    );
}

fn representative_frame(variant: usize) -> UiFrame {
    let mut paint = shapes_frame().paint;
    let variant = u8::try_from(variant % 255).unwrap();
    for line in 0_u32..20 {
        paint.push(PaintOp::Text {
            origin: Point::new(12.0, 320.0 + f64::from(line) * 15.0),
            text: format!("Vulkan retained UI benchmark line {line:02}"),
            color: Color::WHITE,
            size: 12.0,
        });
    }
    let image = RasterImage::new(
        32,
        32,
        [variant.wrapping_mul(13), 120, 220, 255].repeat(32 * 32),
    )
    .unwrap();
    paint.push(PaintOp::Image {
        image,
        rect: Rect::new(650.0, 16.0, 96.0, 96.0),
    });
    frame(paint)
}

fn shapes_frame() -> UiFrame {
    let mut paint = PaintList::default();
    for row in 0_u32..12 {
        for column in 0_u32..20 {
            let index = row * 20 + column;
            paint.push(PaintOp::FillRect {
                rect: Rect::new(
                    8.0 + f64::from(column) * 31.0,
                    8.0 + f64::from(row) * 24.0,
                    27.0,
                    20.0,
                ),
                color: Color::rgba(
                    u8::try_from((index * 17) % 255).unwrap(),
                    u8::try_from((index * 31) % 255).unwrap(),
                    u8::try_from((index * 47) % 255).unwrap(),
                    255,
                ),
                radius: 4.0,
            });
        }
    }
    frame(paint)
}

fn text_frame() -> UiFrame {
    let mut paint = PaintList::default();
    for line in 0_u32..20 {
        paint.push(PaintOp::Text {
            origin: Point::new(12.0, 320.0 + f64::from(line) * 15.0),
            text: format!("Vulkan retained UI benchmark line {line:02}"),
            color: Color::WHITE,
            size: 12.0,
        });
    }
    frame(paint)
}

fn shaped_text_frame() -> UiFrame {
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
        let layout = text.layout(content, size, Some(760.0));
        for run in layout.runs {
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
    frame(paint)
}

fn frame(paint: PaintList) -> UiFrame {
    UiFrame {
        viewport: Size::new(800.0, 640.0),
        damage: Vec::new(),
        paint,
        semantics: Vec::new(),
    }
}
