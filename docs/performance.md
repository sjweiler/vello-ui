# Renderer performance

Run the optimized Ash benchmark with:

```sh
cargo bench -p vui-ash --bench renderer
```

It reports average time and throughput for cold scene construction, retained
scene cache hits, image-atlas churn, and Vulkan offscreen rendering. The GPU
case also prints the selected device, actual geometry/atlas upload volume, and
separate command-preparation, GPU-wait, readback, and hardware-timestamp phases.
The text cases cover both the deterministic bootstrap font and embedded Noto
Sans/Noto Sans Arabic shaping with ligatures, RTL text, and mixed sizes.
If Vulkan is unavailable, CPU-side measurements still run and the GPU case is
reported as skipped.

The windowed swapchain stress test is opt-in so ordinary headless workspace
tests do not require a display server. Run it under an X11 or Wayland session
with `VUI_RUN_WINDOWED_SWAPCHAIN=1 cargo test -p vui-ash --test
windowed_swapchain`; CI supplies that variable inside Xvfb with Vulkan
validation enabled.

Timing output is informational because host load and GPU drivers vary. CI uses
`performance_budgets.rs` for deterministic regression gates instead: the
representative scene has upper bounds for vertices, indices, draw batches, and
atlas allocation, while an incremental image update must remain a single dirty
region with no eviction or atlas rebuild. CI additionally compiles the release
benchmark so benchmark-only code cannot decay.

The current llvmpipe reference from October 2026 is:

- cold scene build: 2.3 ms;
- cold shapes only: 0.41 ms;
- cold bootstrap text only: 0.023 ms;
- cold shaped text: 0.30 ms;
- retained cache hit: 0.69 µs;
- retained shaped-text cache hit: 0.36 µs;
- atlas-churn scene build: 0.50 ms;
- cached offscreen Vulkan render at 4x MSAA: 13.1 ms;
- shapes-only offscreen render: 4.8 ms;
- bootstrap-text-only offscreen render: 10.0 ms;
- shaped-text-only offscreen render: 4.1 ms;
- cold offscreen upload: 462,480 geometry bytes and 4,096 atlas bytes;
- cached offscreen upload: zero bytes.

## Optimization decision

Phase-isolated measurements showed the old bootstrap text path consuming 9.82
ms of a 13.53 ms cold build, while shape tessellation consumed only 0.40 ms.
Each repeated bitmap-font character had been tessellating every lit cell through
Lyon. Retaining the exact generated glyph geometry by character and size reduced
bootstrap text to 0.68 ms, cold builds to 4.07 ms (3.3× faster), and atlas-churn
builds to 1.03 ms (10.7× faster) without changing snapshot output.

Parallel tessellation is deferred because the remaining independently
tessellatable shape phase is below half a millisecond in this workload, where
thread scheduling and geometry merging would have little headroom. Indirect
drawing is also deferred because the structural budget fixture batches the
representative scene into at most three draw commands. Reconsider either
choice only if future profiles show shape tessellation or draw-call submission
becoming material.

The offscreen renderer now retains same-sized render targets, depth/MSAA
attachments, framebuffer, staging, geometry, and readback buffers between
captures. Geometry and atlas revisions suppress unchanged uploads. Atlas
rebuilds clear unused texels on the GPU and upload only occupied regions, so the
representative first atlas transfer fell from 4 MiB to 4 KiB and cached captures
transfer nothing. On the same llvmpipe setup this reduced cached offscreen time
from the original 16.7 ms into an approximately 11–16 ms range, depending on
host load, without changing snapshot output.

## Readback decision

Hardware timestamps and CPU phase timers show that asynchronous readback is not
the next useful optimization for this workload. A representative cached capture
spends about 11.8 ms in GPU execution, 0.015 ms preparing commands, and 0.26 ms
copying mapped readback memory. A readback ring would target roughly two percent
of the total while complicating the synchronous snapshot API.

Phase-isolated GPU cases identified bootstrap text as the next target. Bootstrap
glyph cells now use a dedicated instanced pipeline: each font size contributes
one exact Lyon-rounded cell mesh, while compact position/color instances place
the lit cells. The representative scene fell from 83,908 vertices and 190,950
indices to 4,812 vertices, 12,984 indices, and 9,888 instances. Cold geometry
transfer fell from 3,784,488 bytes to 462,480 bytes, and bootstrap scene building
fell from about 0.68 ms to 0.023 ms, with unchanged pixel snapshots.

The software Vulkan GPU timing remains host-load-sensitive and did not show a
repeatable raster-time improvement from instancing, so no GPU-time speedup is
claimed. The structural, transfer, and CPU improvements are deterministic. The
live swapchain presenter uses the same instanced pipeline, already exposes GPU
timestamps, and does not perform framebuffer readback.

## Shaped-text decision

The embedded-font workload produces 8,368 vertices, 23,964 indices, and one
draw command across sixteen Latin, ligature, Arabic RTL, numeric, and mixed-size
lines. Cold outline construction takes about 0.30 ms, retained builds about
0.36 µs, and its llvmpipe GPU time is only around 0.55 ms above the full-frame
clear/MSAA baseline. A glyph-mask atlas is therefore deferred: it would add
raster-cache sizing, scale invalidation, and another antialiasing path to target
a phase that is not currently material. `performance_budgets.rs` locks the
shaped workload below 10,000 vertices, 30,000 indices, and two draw commands so
future growth makes that decision visible.

## MSAA decision

`VUI_VULKAN_SAMPLES=1`, `2`, or `4` controls the requested sample count for live
and offscreen renderers. The unset default remains the highest mutually
supported color/depth count up to 4x. A valid unsupported request falls back to
the highest supported lower count, and `VulkanDeviceInfo` reports requested and
selected values. `OffscreenRenderer::new_with_sample_count` provides the same
selection without mutating process environment, allowing one benchmark process
to compare all modes.

On the current llvmpipe device, 4x averages about 13.1 ms and 1x about 6.2 ms
for the representative scene. The device lacks a mutually supported 2x
color/depth mode, so a 2x request correctly selects 1x and performs similarly.
The default remains 4x to preserve rendering quality and snapshot baselines;
software-rendered deployments can explicitly select 1x when throughput matters
more than edge quality.

Compare measurements only when the device, build profile, scene, and validation
configuration match. Hardware GPU results should not be compared directly with
the llvmpipe reference.
