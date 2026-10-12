//! VUI's direct Vulkan backend.
//!
//! The crate deliberately isolates unsafe Vulkan calls from the safe UI core.
//! Paint lists are tessellated safely, then drawn with Vulkan pipelines using
//! stencil clips and a retained image atlas. The same pipelines also support
//! deterministic headless rendering and readback for visual regression tests.

mod device;
mod gpu;
mod offscreen;
mod pipeline;
mod raster;
mod vulkan;

pub use device::{VulkanDeviceInfo, VulkanDeviceType};
pub use gpu::{GpuScene, GpuSceneBuilder, GpuSceneStats};
pub use offscreen::{
    EmbeddedRenderer, OffscreenError, OffscreenFrame, OffscreenRenderer, OffscreenTimingStats,
    RenderedImage,
};
pub use raster::RasterFrame;
pub use vulkan::{VulkanError, VulkanPresenter, VulkanUploadStats};
