#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "Vulkan layouts and viewport dimensions use fixed-width numeric types"
)]

use std::{
    marker::PhantomData,
    mem, ptr,
    time::{Duration, Instant},
};

use ash::vk;
use vui_core::{Color, UiFrame};

use crate::{
    device::{VulkanDeviceInfo, describe_host_device, select_device},
    gpu::{CommandKind, GpuScene},
    pipeline::{create_descriptor_resources, create_pipelines, create_render_pass},
    vulkan::{
        VulkanError, VulkanUploadStats, choose_depth_format, color_range, create_buffer,
        create_image, create_image_view, destroy_buffer,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffscreenError(String);

impl std::fmt::Display for OffscreenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for OffscreenError {}

impl From<vk::Result> for OffscreenError {
    fn from(error: vk::Result) -> Self {
        Self(format!("Vulkan error: {error:?}"))
    }
}

impl From<VulkanError> for OffscreenError {
    fn from(error: VulkanError) -> Self {
        Self(error.to_string())
    }
}

/// Packed premultiplied RGBA8 pixels read back from Vulkan.
pub struct OffscreenFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A sampleable image rendered on a host-owned Vulkan device.
///
/// The renderer owns both handles. They remain valid until the renderer is
/// dropped or a later render changes the target dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedImage<'a> {
    pub image: vk::Image,
    pub view: vk::ImageView,
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    pub layout: vk::ImageLayout,
    renderer: PhantomData<&'a OffscreenRenderer>,
}

/// Renders VUI frames into a persistent, sampleable image on a host-owned
/// Vulkan device.
pub struct EmbeddedRenderer {
    renderer: OffscreenRenderer,
    scenes: crate::GpuSceneBuilder,
}

impl EmbeddedRenderer {
    /// Creates an embedded renderer using a compositor's Vulkan context.
    ///
    /// # Safety
    ///
    /// The handle, lifetime, queue-family, and external-synchronization
    /// requirements of [`OffscreenRenderer::from_host_device`] apply.
    ///
    /// # Errors
    ///
    /// Returns an error if the host context is incompatible or renderer
    /// resources cannot be created.
    pub unsafe fn from_host_device(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        device: &ash::Device,
        queue: vk::Queue,
        queue_family: u32,
        requested_samples: Option<u32>,
    ) -> Result<Self, OffscreenError> {
        Ok(Self {
            renderer: unsafe {
                OffscreenRenderer::from_host_device(
                    instance,
                    physical,
                    device,
                    queue,
                    queue_family,
                    requested_samples,
                )?
            },
            scenes: crate::GpuSceneBuilder::new(),
        })
    }

    /// Renders a frame without a GPU-to-CPU copy and returns a sampleable image.
    ///
    /// The returned borrow prevents target recreation or renderer destruction
    /// while the host is inspecting its handles. The image is transparent where
    /// the frame emits no paint when `background` is transparent.
    ///
    /// # Errors
    ///
    /// Returns an error for scene construction, allocation, submission, or
    /// synchronization failure.
    pub fn render(
        &mut self,
        frame: &UiFrame,
        scale: f64,
        background: Color,
    ) -> Result<RenderedImage<'_>, OffscreenError> {
        let scene = self
            .scenes
            .build(frame, scale)
            .map_err(|error| OffscreenError(format!("build embedded Vulkan scene: {error}")))?;
        self.renderer.render_sampleable(&scene, background)
    }

    /// Details of the host adapter used by this renderer.
    #[must_use]
    pub const fn device_info(&self) -> &VulkanDeviceInfo {
        self.renderer.device_info()
    }

    /// Bytes uploaded while preparing the latest embedded frame.
    #[must_use]
    pub const fn last_upload_stats(&self) -> VulkanUploadStats {
        self.renderer.last_upload_stats()
    }

    /// Abandons Vulkan child handles without waiting for or calling the host
    /// device. This is only for recovery from a lost or stalled host device.
    pub fn abandon_device(&mut self) {
        self.renderer.abandon_device();
    }
}

/// CPU and GPU timings collected for the latest synchronous capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OffscreenTimingStats {
    pub resource_wait: Duration,
    pub prepare: Duration,
    pub gpu_wait: Duration,
    pub readback: Duration,
    pub gpu: Option<Duration>,
}

/// Reusable headless Vulkan renderer for snapshots and renderer tests.
pub struct OffscreenRenderer {
    _entry: Option<ash::Entry>,
    owns_context: bool,
    abandoned: bool,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
    device_info: VulkanDeviceInfo,
    device: ash::Device,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    timestamp_query: vk::QueryPool,
    timestamp_period: f32,
    sample_count: vk::SampleCountFlags,
    depth_format: vk::Format,
    render_pass: vk::RenderPass,
    descriptor_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    draw_pipeline: vk::Pipeline,
    bootstrap_pipeline: vk::Pipeline,
    clip_push_pipeline: vk::Pipeline,
    clip_pop_pipeline: vk::Pipeline,
    sampler: vk::Sampler,
    target: vk::Image,
    target_memory: vk::DeviceMemory,
    target_view: vk::ImageView,
    multisample: vk::Image,
    multisample_memory: vk::DeviceMemory,
    multisample_view: vk::ImageView,
    depth: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    atlas: vk::Image,
    atlas_memory: vk::DeviceMemory,
    atlas_view: vk::ImageView,
    staging: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    geometry: vk::Buffer,
    geometry_memory: vk::DeviceMemory,
    readback: vk::Buffer,
    readback_memory: vk::DeviceMemory,
    extent: vk::Extent2D,
    atlas_extent: vk::Extent2D,
    staging_capacity: u64,
    geometry_capacity: u64,
    readback_capacity: u64,
    atlas_revision: u64,
    geometry_revision: u64,
    last_upload_stats: VulkanUploadStats,
    last_timing_stats: OffscreenTimingStats,
}

impl OffscreenRenderer {
    /// Creates a headless Vulkan device and reusable graphics pipelines.
    ///
    /// # Errors
    ///
    /// Returns an error when no graphics-capable Vulkan device exists or a
    /// required renderer resource cannot be created.
    pub fn new() -> Result<Self, OffscreenError> {
        Self::new_with_requested_samples(None)
    }

    /// Creates a headless renderer requesting a specific MSAA sample count.
    /// Unsupported requests gracefully select the highest supported lower
    /// count. Valid requests are 1, 2, and 4.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid count or when Vulkan setup fails.
    pub fn new_with_sample_count(samples: u32) -> Result<Self, OffscreenError> {
        Self::new_with_requested_samples(Some(samples))
    }

    /// Creates a renderer on a compositor-owned Vulkan device and graphics queue.
    ///
    /// The renderer creates and owns its pipelines, buffers, and sampleable target
    /// image, but never destroys the supplied instance, device, or queue. Rendering
    /// is synchronized before returning so the host may immediately sample the
    /// returned image on the same logical device.
    ///
    /// # Safety
    ///
    /// `physical`, `device`, and `queue` must all belong to `instance`; `queue`
    /// must come from `queue_family`; and the host must keep every supplied handle
    /// alive until this renderer is dropped. The host must externally synchronize
    /// all other access to `queue`.
    ///
    /// # Errors
    ///
    /// Returns an error when the queue family is unsuitable, the sample request is
    /// invalid, or a Vulkan resource cannot be created.
    #[allow(clippy::too_many_lines)]
    pub unsafe fn from_host_device(
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        device: &ash::Device,
        queue: vk::Queue,
        queue_family: u32,
        requested_samples: Option<u32>,
    ) -> Result<Self, OffscreenError> {
        let selected =
            unsafe { describe_host_device(instance, physical, queue_family, requested_samples)? };
        let timestamp_query = if selected.info.timestamp_queries {
            unsafe {
                device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2),
                    None,
                )?
            }
        } else {
            vk::QueryPool::null()
        };
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?
        };
        let command_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?
        }[0];
        let fence = unsafe {
            device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?
        };
        let depth_format = choose_depth_format(instance, physical)?;
        let render_pass = unsafe {
            create_render_pass(
                device,
                vk::Format::R8G8B8A8_UNORM,
                depth_format,
                selected.sample_count,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            )?
        };
        let descriptors = unsafe { create_descriptor_resources(device)? };
        let pipelines = unsafe {
            create_pipelines(
                device,
                render_pass,
                descriptors.pipeline_layout,
                selected.sample_count,
            )?
        };
        Ok(Self {
            _entry: None,
            owns_context: false,
            abandoned: false,
            instance: instance.clone(),
            physical,
            device_info: selected.info,
            device: device.clone(),
            queue,
            command_pool,
            command_buffer,
            fence,
            timestamp_query,
            timestamp_period: selected.timestamp_period,
            sample_count: selected.sample_count,
            depth_format,
            render_pass,
            descriptor_layout: descriptors.layout,
            descriptor_pool: descriptors.pool,
            descriptor_set: descriptors.set,
            pipeline_layout: descriptors.pipeline_layout,
            draw_pipeline: pipelines.draw,
            bootstrap_pipeline: pipelines.bootstrap,
            clip_push_pipeline: pipelines.clip_push,
            clip_pop_pipeline: pipelines.clip_pop,
            sampler: descriptors.sampler,
            target: vk::Image::null(),
            target_memory: vk::DeviceMemory::null(),
            target_view: vk::ImageView::null(),
            multisample: vk::Image::null(),
            multisample_memory: vk::DeviceMemory::null(),
            multisample_view: vk::ImageView::null(),
            depth: vk::Image::null(),
            depth_memory: vk::DeviceMemory::null(),
            depth_view: vk::ImageView::null(),
            framebuffer: vk::Framebuffer::null(),
            atlas: vk::Image::null(),
            atlas_memory: vk::DeviceMemory::null(),
            atlas_view: vk::ImageView::null(),
            staging: vk::Buffer::null(),
            staging_memory: vk::DeviceMemory::null(),
            geometry: vk::Buffer::null(),
            geometry_memory: vk::DeviceMemory::null(),
            readback: vk::Buffer::null(),
            readback_memory: vk::DeviceMemory::null(),
            extent: vk::Extent2D::default(),
            atlas_extent: vk::Extent2D::default(),
            staging_capacity: 0,
            geometry_capacity: 0,
            readback_capacity: 0,
            atlas_revision: 0,
            geometry_revision: 0,
            last_upload_stats: VulkanUploadStats::default(),
            last_timing_stats: OffscreenTimingStats::default(),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn new_with_requested_samples(requested_samples: Option<u32>) -> Result<Self, OffscreenError> {
        // SAFETY: handles are created in dependency order and owned until Drop.
        unsafe {
            let entry = ash::Entry::load()
                .map_err(|error| OffscreenError(format!("load Vulkan loader: {error}")))?;
            let application = vk::ApplicationInfo::default()
                .application_name(c"VUI offscreen")
                .engine_name(c"VUI")
                .api_version(vk::API_VERSION_1_1);
            let validation_name = c"VK_LAYER_KHRONOS_validation";
            let validation_requested = std::env::var_os("VUI_VULKAN_VALIDATION").is_some();
            let validation_available =
                entry
                    .enumerate_instance_layer_properties()?
                    .iter()
                    .any(|layer| {
                        std::ffi::CStr::from_ptr(layer.layer_name.as_ptr()) == validation_name
                    });
            if validation_requested && !validation_available {
                return Err(OffscreenError(
                    "VUI_VULKAN_VALIDATION requested, but VK_LAYER_KHRONOS_validation is unavailable"
                        .to_owned(),
                ));
            }
            let layers = if validation_requested {
                vec![validation_name.as_ptr()]
            } else {
                Vec::new()
            };
            let instance = entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&application)
                    .enabled_layer_names(&layers),
                None,
            )?;
            let selected = select_device(
                &instance,
                "graphics requirements",
                requested_samples,
                |_physical, _index, properties| {
                    properties.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                },
            )?;
            let physical = selected.physical;
            let family = selected.queue_family;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&priorities)];
            let device = instance.create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                None,
            )?;
            let queue = device.get_device_queue(family, 0);
            let timestamp_query = if selected.info.timestamp_queries {
                device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2),
                    None,
                )?
            } else {
                vk::QueryPool::null()
            };
            let command_pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            let command_buffer = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )?[0];
            let fence = device.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?;
            let sample_count = selected.sample_count;
            let depth_format = choose_depth_format(&instance, physical)?;
            let render_pass = create_render_pass(
                &device,
                vk::Format::R8G8B8A8_UNORM,
                depth_format,
                sample_count,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            )?;
            let descriptors = create_descriptor_resources(&device)?;
            let pipelines = create_pipelines(
                &device,
                render_pass,
                descriptors.pipeline_layout,
                sample_count,
            )?;
            Ok(Self {
                _entry: Some(entry),
                owns_context: true,
                abandoned: false,
                instance,
                physical,
                device_info: selected.info,
                device,
                queue,
                command_pool,
                command_buffer,
                fence,
                timestamp_query,
                timestamp_period: selected.timestamp_period,
                sample_count,
                depth_format,
                render_pass,
                descriptor_layout: descriptors.layout,
                descriptor_pool: descriptors.pool,
                descriptor_set: descriptors.set,
                pipeline_layout: descriptors.pipeline_layout,
                draw_pipeline: pipelines.draw,
                bootstrap_pipeline: pipelines.bootstrap,
                clip_push_pipeline: pipelines.clip_push,
                clip_pop_pipeline: pipelines.clip_pop,
                sampler: descriptors.sampler,
                target: vk::Image::null(),
                target_memory: vk::DeviceMemory::null(),
                target_view: vk::ImageView::null(),
                multisample: vk::Image::null(),
                multisample_memory: vk::DeviceMemory::null(),
                multisample_view: vk::ImageView::null(),
                depth: vk::Image::null(),
                depth_memory: vk::DeviceMemory::null(),
                depth_view: vk::ImageView::null(),
                framebuffer: vk::Framebuffer::null(),
                atlas: vk::Image::null(),
                atlas_memory: vk::DeviceMemory::null(),
                atlas_view: vk::ImageView::null(),
                staging: vk::Buffer::null(),
                staging_memory: vk::DeviceMemory::null(),
                geometry: vk::Buffer::null(),
                geometry_memory: vk::DeviceMemory::null(),
                readback: vk::Buffer::null(),
                readback_memory: vk::DeviceMemory::null(),
                extent: vk::Extent2D::default(),
                atlas_extent: vk::Extent2D::default(),
                staging_capacity: 0,
                geometry_capacity: 0,
                readback_capacity: 0,
                atlas_revision: 0,
                geometry_revision: 0,
                last_upload_stats: VulkanUploadStats::default(),
                last_timing_stats: OffscreenTimingStats::default(),
            })
        }
    }

    /// Details of the physical adapter selected for offscreen rendering.
    #[must_use]
    pub const fn device_info(&self) -> &VulkanDeviceInfo {
        &self.device_info
    }

    /// Actual bytes uploaded while preparing the most recent capture.
    #[must_use]
    pub const fn last_upload_stats(&self) -> VulkanUploadStats {
        self.last_upload_stats
    }

    /// Prevents drop from touching a host device that is being abandoned.
    pub fn abandon_device(&mut self) {
        self.abandoned = true;
    }

    /// CPU phase timings and, when supported, GPU execution time for the most
    /// recent capture.
    #[must_use]
    pub const fn last_timing_stats(&self) -> OffscreenTimingStats {
        self.last_timing_stats
    }

    /// Renders and reads back one GPU scene.
    ///
    /// # Errors
    ///
    /// Returns an error for allocation, command submission, or readback failure.
    #[allow(clippy::too_many_lines)]
    pub fn render(
        &mut self,
        scene: &GpuScene,
        background: Color,
    ) -> Result<OffscreenFrame, OffscreenError> {
        self.render_internal(scene, background, true)?
            .ok_or_else(|| OffscreenError("Vulkan readback did not produce a frame".to_owned()))
    }

    /// Renders into a persistent image that the host can sample directly.
    ///
    /// No GPU-to-CPU copy is performed. The call waits for rendering to finish,
    /// returns the image in `SHADER_READ_ONLY_OPTIMAL`, and retains ownership of
    /// the image and view. This is intended for renderers created with
    /// [`Self::from_host_device`].
    ///
    /// # Errors
    ///
    /// Returns an error for allocation, command submission, or synchronization
    /// failure.
    pub fn render_sampleable(
        &mut self,
        scene: &GpuScene,
        background: Color,
    ) -> Result<RenderedImage<'_>, OffscreenError> {
        let _ = self.render_internal(scene, background, false)?;
        Ok(RenderedImage {
            image: self.target,
            view: self.target_view,
            width: scene.width,
            height: scene.height,
            format: vk::Format::R8G8B8A8_UNORM,
            layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            renderer: PhantomData,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn render_internal(
        &mut self,
        scene: &GpuScene,
        background: Color,
        read_pixels: bool,
    ) -> Result<Option<OffscreenFrame>, OffscreenError> {
        // SAFETY: the fence makes every owned per-capture resource idle before
        // destruction, mapping, or reuse. Copies use scene-derived byte sizes.
        unsafe {
            let wait_started = Instant::now();
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            let resource_wait = wait_started.elapsed();
            let prepare_started = Instant::now();
            let extent = vk::Extent2D {
                width: scene.width,
                height: scene.height,
            };
            let vertex_bytes: &[u8] = bytemuck::cast_slice(&scene.vertices);
            let index_bytes: &[u8] = bytemuck::cast_slice(&scene.indices);
            let instance_bytes: &[u8] = bytemuck::cast_slice(&scene.bootstrap_instances);
            let index_offset = (vertex_bytes.len() as u64).next_multiple_of(4);
            let instance_offset = (index_offset + index_bytes.len() as u64).next_multiple_of(4);
            let geometry_len = instance_offset + instance_bytes.len() as u64;
            self.ensure_dynamic(scene, extent, geometry_len)?;
            let atlas_initialized = self.atlas_revision != 0;
            let upload_atlas = self.atlas_revision != scene.atlas_revision;
            let reset_atlas = upload_atlas && scene.atlas_rebuilt;
            let full_atlas_upload = upload_atlas
                && !reset_atlas
                && (!atlas_initialized || self.atlas_revision != scene.atlas_base_revision);
            let atlas_len = if full_atlas_upload {
                scene.atlas.len() as u64
            } else if upload_atlas {
                scene
                    .atlas_dirty
                    .iter()
                    .map(|dirty| dirty.pixels.len() as u64)
                    .sum()
            } else {
                0
            };
            let upload_geometry = self.geometry_revision != scene.geometry_revision;
            let geometry_staging_offset = atlas_len.next_multiple_of(4);
            let staging_len =
                geometry_staging_offset + if upload_geometry { geometry_len } else { 0 };
            if staging_len > 0 {
                let mapped = self.device.map_memory(
                    self.staging_memory,
                    0,
                    staging_len,
                    vk::MemoryMapFlags::empty(),
                )?;
                if upload_atlas {
                    if full_atlas_upload {
                        ptr::copy_nonoverlapping(
                            scene.atlas.as_ptr(),
                            mapped.cast(),
                            scene.atlas.len(),
                        );
                    } else {
                        let mut offset = 0_usize;
                        for dirty in scene.atlas_dirty.iter() {
                            ptr::copy_nonoverlapping(
                                dirty.pixels.as_ptr(),
                                mapped.cast::<u8>().add(offset),
                                dirty.pixels.len(),
                            );
                            offset += dirty.pixels.len();
                        }
                    }
                }
                if upload_geometry && geometry_len > 0 {
                    let geometry_target = mapped.cast::<u8>().add(geometry_staging_offset as usize);
                    ptr::copy_nonoverlapping(
                        vertex_bytes.as_ptr(),
                        geometry_target,
                        vertex_bytes.len(),
                    );
                    ptr::copy_nonoverlapping(
                        index_bytes.as_ptr(),
                        geometry_target.add(index_offset as usize),
                        index_bytes.len(),
                    );
                    ptr::copy_nonoverlapping(
                        instance_bytes.as_ptr(),
                        geometry_target.add(instance_offset as usize),
                        instance_bytes.len(),
                    );
                }
                self.device.unmap_memory(self.staging_memory);
            }

            self.device.reset_fences(&[self.fence])?;
            self.device
                .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                self.command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if self.timestamp_query != vk::QueryPool::null() {
                self.device
                    .cmd_reset_query_pool(self.command_buffer, self.timestamp_query, 0, 2);
                self.device.cmd_write_timestamp(
                    self.command_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.timestamp_query,
                    0,
                );
            }
            if upload_geometry && geometry_len > 0 {
                self.device.cmd_copy_buffer(
                    self.command_buffer,
                    self.staging,
                    self.geometry,
                    &[vk::BufferCopy::default()
                        .src_offset(geometry_staging_offset)
                        .size(geometry_len)],
                );
                let geometry_ready = vk::BufferMemoryBarrier::default()
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(self.geometry)
                    .offset(0)
                    .size(geometry_len)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::VERTEX_ATTRIBUTE_READ | vk::AccessFlags::INDEX_READ,
                    );
                self.device.cmd_pipeline_barrier(
                    self.command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::VERTEX_INPUT,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[geometry_ready],
                    &[],
                );
            }
            if upload_atlas {
                let atlas_upload = vk::ImageMemoryBarrier::default()
                    .old_layout(if atlas_initialized {
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                    } else {
                        vk::ImageLayout::UNDEFINED
                    })
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.atlas)
                    .subresource_range(color_range())
                    .src_access_mask(if atlas_initialized {
                        vk::AccessFlags::SHADER_READ
                    } else {
                        vk::AccessFlags::empty()
                    })
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
                self.device.cmd_pipeline_barrier(
                    self.command_buffer,
                    if atlas_initialized {
                        vk::PipelineStageFlags::FRAGMENT_SHADER
                    } else {
                        vk::PipelineStageFlags::TOP_OF_PIPE
                    },
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[atlas_upload],
                );
                if reset_atlas {
                    self.device.cmd_clear_color_image(
                        self.command_buffer,
                        self.atlas,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &vk::ClearColorValue { uint32: [0; 4] },
                        &[color_range()],
                    );
                    let clear_ready = vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.atlas)
                        .subresource_range(color_range())
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
                    self.device.cmd_pipeline_barrier(
                        self.command_buffer,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[clear_ready],
                    );
                }
                let copies = if full_atlas_upload {
                    vec![
                        vk::BufferImageCopy::default()
                            .image_subresource(
                                vk::ImageSubresourceLayers::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .layer_count(1),
                            )
                            .image_extent(vk::Extent3D {
                                width: scene.atlas_width,
                                height: scene.atlas_height,
                                depth: 1,
                            }),
                    ]
                } else {
                    let mut offset = 0_u64;
                    scene
                        .atlas_dirty
                        .iter()
                        .map(|dirty| {
                            let copy = vk::BufferImageCopy::default()
                                .buffer_offset(offset)
                                .image_subresource(
                                    vk::ImageSubresourceLayers::default()
                                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                                        .layer_count(1),
                                )
                                .image_offset(vk::Offset3D {
                                    x: dirty.x.cast_signed(),
                                    y: dirty.y.cast_signed(),
                                    z: 0,
                                })
                                .image_extent(vk::Extent3D {
                                    width: dirty.width,
                                    height: dirty.height,
                                    depth: 1,
                                });
                            offset += dirty.pixels.len() as u64;
                            copy
                        })
                        .collect::<Vec<_>>()
                };
                if !copies.is_empty() {
                    self.device.cmd_copy_buffer_to_image(
                        self.command_buffer,
                        self.staging,
                        self.atlas,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &copies,
                    );
                }
                let atlas_ready = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.atlas)
                    .subresource_range(color_range())
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);
                self.device.cmd_pipeline_barrier(
                    self.command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[atlas_ready],
                );
            }

            let alpha = f32::from(background.a) / 255.0;
            let clears = [
                vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [
                            f32::from(background.r) / 255.0 * alpha,
                            f32::from(background.g) / 255.0 * alpha,
                            f32::from(background.b) / 255.0 * alpha,
                            alpha,
                        ],
                    },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
            ];
            self.device.cmd_begin_render_pass(
                self.command_buffer,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(self.framebuffer)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .clear_values(&clears),
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_set_viewport(
                self.command_buffer,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: extent.width as f32,
                    height: extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            self.device.cmd_set_scissor(
                self.command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                }],
            );
            self.device.cmd_bind_descriptor_sets(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );
            self.device
                .cmd_bind_vertex_buffers(self.command_buffer, 0, &[self.geometry], &[0]);
            self.device.cmd_bind_vertex_buffers(
                self.command_buffer,
                1,
                &[self.geometry],
                &[instance_offset],
            );
            self.device.cmd_bind_index_buffer(
                self.command_buffer,
                self.geometry,
                index_offset,
                vk::IndexType::UINT32,
            );
            let mut bound = vk::Pipeline::null();
            for command in scene.commands.iter() {
                let pipeline = match command.kind {
                    CommandKind::Draw => self.draw_pipeline,
                    CommandKind::Bootstrap => self.bootstrap_pipeline,
                    CommandKind::ClipPush => self.clip_push_pipeline,
                    CommandKind::ClipPop => self.clip_pop_pipeline,
                };
                if pipeline != bound {
                    self.device.cmd_bind_pipeline(
                        self.command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline,
                    );
                    bound = pipeline;
                }
                self.device.cmd_set_stencil_reference(
                    self.command_buffer,
                    vk::StencilFaceFlags::FRONT_AND_BACK,
                    command.stencil_reference,
                );
                self.device.cmd_draw_indexed(
                    self.command_buffer,
                    command.indices.end - command.indices.start,
                    command.instances.end - command.instances.start,
                    command.indices.start,
                    0,
                    command.instances.start,
                );
            }
            self.device.cmd_end_render_pass(self.command_buffer);
            if read_pixels {
                let copy = vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: extent.width,
                        height: extent.height,
                        depth: 1,
                    });
                self.device.cmd_copy_image_to_buffer(
                    self.command_buffer,
                    self.target,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    self.readback,
                    &[copy],
                );
            } else {
                let sampleable = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.target)
                    .subresource_range(color_range())
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);
                self.device.cmd_pipeline_barrier(
                    self.command_buffer,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[sampleable],
                );
            }
            if self.timestamp_query != vk::QueryPool::null() {
                self.device.cmd_write_timestamp(
                    self.command_buffer,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    self.timestamp_query,
                    1,
                );
            }
            self.device.end_command_buffer(self.command_buffer)?;
            self.device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&[self.command_buffer])],
                self.fence,
            )?;
            let prepare = prepare_started.elapsed();
            let gpu_wait_started = Instant::now();
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            let gpu_wait = gpu_wait_started.elapsed();
            let gpu = if self.timestamp_query == vk::QueryPool::null() {
                None
            } else {
                let mut timestamps = [0_u64; 2];
                self.device.get_query_pool_results(
                    self.timestamp_query,
                    0,
                    &mut timestamps,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )?;
                let ticks = timestamps[1].saturating_sub(timestamps[0]);
                let nanoseconds = ticks as f64 * f64::from(self.timestamp_period);
                Some(Duration::from_secs_f64(nanoseconds / 1_000_000_000.0))
            };
            let (frame, readback) = if read_pixels {
                let readback_started = Instant::now();
                let byte_len = u64::from(scene.width) * u64::from(scene.height) * 4;
                let mapped = self.device.map_memory(
                    self.readback_memory,
                    0,
                    byte_len,
                    vk::MemoryMapFlags::empty(),
                )?;
                let pixels =
                    std::slice::from_raw_parts(mapped.cast::<u8>(), byte_len as usize).to_vec();
                self.device.unmap_memory(self.readback_memory);
                (
                    Some(OffscreenFrame {
                        pixels,
                        width: scene.width,
                        height: scene.height,
                    }),
                    readback_started.elapsed(),
                )
            } else {
                (None, Duration::ZERO)
            };
            self.last_upload_stats = VulkanUploadStats {
                geometry_bytes: if upload_geometry { geometry_len } else { 0 },
                atlas_bytes: atlas_len,
            };
            if upload_geometry {
                self.geometry_revision = scene.geometry_revision;
            }
            if upload_atlas {
                self.atlas_revision = scene.atlas_revision;
            }
            self.last_timing_stats = OffscreenTimingStats {
                resource_wait,
                prepare,
                gpu_wait,
                readback,
                gpu,
            };
            Ok(frame)
        }
    }

    unsafe fn ensure_dynamic(
        &mut self,
        scene: &GpuScene,
        extent: vk::Extent2D,
        geometry_len: u64,
    ) -> Result<(), OffscreenError> {
        let atlas_extent = vk::Extent2D {
            width: scene.atlas_width,
            height: scene.atlas_height,
        };
        let staging_len = (scene.atlas.len() as u64).next_multiple_of(4) + geometry_len.max(4);
        let readback_len = u64::from(scene.width) * u64::from(scene.height) * 4;
        if self.extent == extent
            && self.atlas_extent == atlas_extent
            && self.staging_capacity >= staging_len
            && self.geometry_capacity >= geometry_len.max(4)
            && self.readback_capacity >= readback_len
        {
            return Ok(());
        }
        // An embedded host may still have compositor work sampling `target`
        // after our own render fence has signaled. Dynamic recreation is rare
        // (resize or capacity growth), so make that cross-renderer lifetime
        // boundary explicit before destroying the borrowed image/view.
        if !self.owns_context && self.target != vk::Image::null() {
            unsafe { self.device.device_wait_idle()? };
        }
        unsafe { self.destroy_dynamic() };
        unsafe { self.create_dynamic(scene, extent) }
    }

    unsafe fn create_dynamic(
        &mut self,
        scene: &GpuScene,
        extent: vk::Extent2D,
    ) -> Result<(), OffscreenError> {
        let (target, target_memory) = unsafe {
            create_image(
                &self.instance,
                self.physical,
                &self.device,
                extent,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::SAMPLED,
                vk::SampleCountFlags::TYPE_1,
            )?
        };
        self.target = target;
        self.target_memory = target_memory;
        self.target_view = unsafe {
            create_image_view(
                &self.device,
                target,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageAspectFlags::COLOR,
            )?
        };
        if self.sample_count != vk::SampleCountFlags::TYPE_1 {
            let (image, memory) = unsafe {
                create_image(
                    &self.instance,
                    self.physical,
                    &self.device,
                    extent,
                    vk::Format::R8G8B8A8_UNORM,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    self.sample_count,
                )?
            };
            self.multisample = image;
            self.multisample_memory = memory;
            self.multisample_view = unsafe {
                create_image_view(
                    &self.device,
                    image,
                    vk::Format::R8G8B8A8_UNORM,
                    vk::ImageAspectFlags::COLOR,
                )?
            };
        }
        let (depth, depth_memory) = unsafe {
            create_image(
                &self.instance,
                self.physical,
                &self.device,
                extent,
                self.depth_format,
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
                self.sample_count,
            )?
        };
        self.depth = depth;
        self.depth_memory = depth_memory;
        self.depth_view = unsafe {
            create_image_view(
                &self.device,
                depth,
                self.depth_format,
                vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL,
            )?
        };
        let views = if self.sample_count == vk::SampleCountFlags::TYPE_1 {
            vec![self.target_view, self.depth_view]
        } else {
            vec![self.multisample_view, self.depth_view, self.target_view]
        };
        self.framebuffer = unsafe {
            self.device.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(&views)
                    .width(extent.width)
                    .height(extent.height)
                    .layers(1),
                None,
            )?
        };
        let atlas_extent = vk::Extent2D {
            width: scene.atlas_width,
            height: scene.atlas_height,
        };
        let (atlas, atlas_memory) = unsafe {
            create_image(
                &self.instance,
                self.physical,
                &self.device,
                atlas_extent,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                vk::SampleCountFlags::TYPE_1,
            )?
        };
        self.atlas = atlas;
        self.atlas_memory = atlas_memory;
        self.atlas_view = unsafe {
            create_image_view(
                &self.device,
                atlas,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageAspectFlags::COLOR,
            )?
        };
        let atlas_info = [vk::DescriptorImageInfo::default()
            .image_view(self.atlas_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let sampler_info = [vk::DescriptorImageInfo::default().sampler(self.sampler)];
        unsafe {
            self.device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.descriptor_set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&atlas_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.descriptor_set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&sampler_info),
                ],
                &[],
            );
        };
        let atlas_len = scene.atlas.len() as u64;
        let vertex_len = mem::size_of_val(scene.vertices.as_ref()) as u64;
        let index_offset = vertex_len.next_multiple_of(4);
        let instance_offset =
            (index_offset + mem::size_of_val(scene.indices.as_ref()) as u64).next_multiple_of(4);
        let geometry_len =
            (instance_offset + mem::size_of_val(scene.bootstrap_instances.as_ref()) as u64).max(4);
        let staging_len = atlas_len.next_multiple_of(4) + geometry_len;
        (self.staging, self.staging_memory) = unsafe {
            create_buffer(
                &self.instance,
                self.physical,
                &self.device,
                staging_len.max(4),
                vk::BufferUsageFlags::TRANSFER_SRC,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?
        };
        (self.geometry, self.geometry_memory) = unsafe {
            create_buffer(
                &self.instance,
                self.physical,
                &self.device,
                geometry_len,
                vk::BufferUsageFlags::VERTEX_BUFFER
                    | vk::BufferUsageFlags::INDEX_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?
        };
        let readback_len = u64::from(scene.width) * u64::from(scene.height) * 4;
        (self.readback, self.readback_memory) = unsafe {
            create_buffer(
                &self.instance,
                self.physical,
                &self.device,
                readback_len,
                vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?
        };
        self.extent = extent;
        self.atlas_extent = atlas_extent;
        self.staging_capacity = staging_len.max(4);
        self.geometry_capacity = geometry_len;
        self.readback_capacity = readback_len;
        Ok(())
    }

    unsafe fn destroy_dynamic(&mut self) {
        if self.framebuffer != vk::Framebuffer::null() {
            unsafe { self.device.destroy_framebuffer(self.framebuffer, None) };
        }
        for view in [
            self.atlas_view,
            self.depth_view,
            self.multisample_view,
            self.target_view,
        ] {
            if view != vk::ImageView::null() {
                unsafe { self.device.destroy_image_view(view, None) };
            }
        }
        for image in [self.atlas, self.depth, self.multisample, self.target] {
            if image != vk::Image::null() {
                unsafe { self.device.destroy_image(image, None) };
            }
        }
        for memory in [
            self.atlas_memory,
            self.depth_memory,
            self.multisample_memory,
            self.target_memory,
        ] {
            if memory != vk::DeviceMemory::null() {
                unsafe { self.device.free_memory(memory, None) };
            }
        }
        unsafe { destroy_buffer(&self.device, &mut self.staging, &mut self.staging_memory) };
        unsafe { destroy_buffer(&self.device, &mut self.geometry, &mut self.geometry_memory) };
        unsafe { destroy_buffer(&self.device, &mut self.readback, &mut self.readback_memory) };
        self.framebuffer = vk::Framebuffer::null();
        self.atlas_view = vk::ImageView::null();
        self.depth_view = vk::ImageView::null();
        self.multisample_view = vk::ImageView::null();
        self.target_view = vk::ImageView::null();
        self.atlas = vk::Image::null();
        self.depth = vk::Image::null();
        self.multisample = vk::Image::null();
        self.target = vk::Image::null();
        self.atlas_memory = vk::DeviceMemory::null();
        self.depth_memory = vk::DeviceMemory::null();
        self.multisample_memory = vk::DeviceMemory::null();
        self.target_memory = vk::DeviceMemory::null();
        self.extent = vk::Extent2D::default();
        self.atlas_extent = vk::Extent2D::default();
        self.staging_capacity = 0;
        self.geometry_capacity = 0;
        self.readback_capacity = 0;
        self.atlas_revision = 0;
        self.geometry_revision = 0;
    }
}

impl Drop for OffscreenRenderer {
    fn drop(&mut self) {
        if self.abandoned {
            return;
        }
        // SAFETY: the device is idle and every handle is owned by this renderer.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.destroy_dynamic();
            for pipeline in [
                self.draw_pipeline,
                self.bootstrap_pipeline,
                self.clip_push_pipeline,
                self.clip_pop_pipeline,
            ] {
                self.device.destroy_pipeline(pipeline, None);
            }
            self.device.destroy_sampler(self.sampler, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_layout, None);
            self.device.destroy_render_pass(self.render_pass, None);
            if self.timestamp_query != vk::QueryPool::null() {
                self.device.destroy_query_pool(self.timestamp_query, None);
            }
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            if self.owns_context {
                self.device.destroy_device(None);
                self.instance.destroy_instance(None);
            }
        }
    }
}
