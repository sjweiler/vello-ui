use std::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ash::{khr, vk};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::{
    RasterFrame,
    device::{VulkanDeviceInfo, select_device},
    gpu::{CommandKind, GpuScene},
    pipeline::{create_descriptor_resources, create_pipelines, create_render_pass},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanError(pub(crate) String);

impl std::fmt::Display for VulkanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for VulkanError {}

impl VulkanError {
    /// Whether Vulkan reported that the logical device can no longer be used.
    #[must_use]
    pub fn is_device_lost(&self) -> bool {
        self.0.contains("ERROR_DEVICE_LOST")
    }
}

impl From<vk::Result> for VulkanError {
    fn from(error: vk::Result) -> Self {
        Self(format!("Vulkan error: {error:?}"))
    }
}

/// Bytes transferred to device-local renderer resources by the latest frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VulkanUploadStats {
    pub geometry_bytes: u64,
    pub atlas_bytes: u64,
}

/// Draws VUI GPU scenes or presents CPU fallback frames through Vulkan.
///
/// Two independently synchronized frames may be in flight. Swapchain-sized
/// multisample attachments are recreated on resize, while scene geometry and
/// image atlases stay resident until their revisions change.
pub struct VulkanPresenter {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_loader: khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
    device_info: VulkanDeviceInfo,
    device: ash::Device,
    queue: vk::Queue,
    swapchain_loader: khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    format: vk::Format,
    extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    sample_count: vk::SampleCountFlags,
    multisample_image: vk::Image,
    multisample_memory: vk::DeviceMemory,
    multisample_view: vk::ImageView,
    depth_format: vk::Format,
    depth_image: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    descriptor_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    draw_pipeline: vk::Pipeline,
    bootstrap_pipeline: vk::Pipeline,
    clip_push_pipeline: vk::Pipeline,
    clip_pop_pipeline: vk::Pipeline,
    sampler: vk::Sampler,
    atlas_image: vk::Image,
    atlas_memory: vk::DeviceMemory,
    atlas_view: vk::ImageView,
    atlas_extent: vk::Extent2D,
    atlas_revision: u64,
    geometry: [vk::Buffer; 2],
    geometry_memory: [vk::DeviceMemory; 2],
    geometry_capacity: [vk::DeviceSize; 2],
    geometry_revision: [u64; 2],
    command_pool: vk::CommandPool,
    command_buffers: [vk::CommandBuffer; 2],
    image_available: [vk::Semaphore; 2],
    render_finished: Vec<vk::Semaphore>,
    fences: [vk::Fence; 2],
    staging: [vk::Buffer; 2],
    staging_memory: [vk::DeviceMemory; 2],
    staging_capacity: [vk::DeviceSize; 2],
    current_frame: usize,
    timestamp_queries: [vk::QueryPool; 2],
    timestamp_pending: [bool; 2],
    timestamp_period: f32,
    last_gpu_time: Option<Duration>,
    last_upload_stats: VulkanUploadStats,
}

static FAULT_INJECTED: AtomicBool = AtomicBool::new(false);

fn inject_device_loss(stage: &str) -> Result<(), VulkanError> {
    if std::env::var("VUI_VULKAN_FAULT").as_deref() == Ok(stage)
        && !FAULT_INJECTED.swap(true, Ordering::Relaxed)
    {
        return Err(vk::Result::ERROR_DEVICE_LOST.into());
    }
    Ok(())
}

impl VulkanPresenter {
    /// Creates a Vulkan instance, surface, device, and swapchain for `window`.
    ///
    /// The window and display handles must remain valid until this presenter is
    /// dropped. A `winit::Window` satisfies that contract when both are fields
    /// of the same host window state.
    ///
    /// # Errors
    ///
    /// Returns an error if Vulkan is unavailable or instance, surface, device,
    /// synchronization, or swapchain creation fails.
    #[allow(clippy::too_many_lines)]
    pub fn new<W>(window: &W, width: u32, height: u32) -> Result<Self, VulkanError>
    where
        W: HasDisplayHandle + HasWindowHandle,
    {
        // SAFETY: all Vulkan handles are created in parent-before-child order,
        // checked after each call, stored by the presenter, and destroyed in
        // the reverse order in Drop.
        unsafe {
            let entry = ash::Entry::load()
                .map_err(|error| VulkanError(format!("load Vulkan loader: {error}")))?;
            let display = window
                .display_handle()
                .map_err(|error| VulkanError(format!("get display handle: {error}")))?;
            let window_handle = window
                .window_handle()
                .map_err(|error| VulkanError(format!("get window handle: {error}")))?;
            #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(unused_mut))]
            let mut extensions =
                ash_window::enumerate_required_extensions(display.as_raw())?.to_vec();
            #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(unused_mut))]
            let mut instance_flags = vk::InstanceCreateFlags::empty();
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                extensions.push(khr::portability_enumeration::NAME.as_ptr());
                instance_flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
            }
            let app_name = c"VUI";
            let application = vk::ApplicationInfo::default()
                .application_name(app_name)
                .application_version(1)
                .engine_name(app_name)
                .engine_version(1)
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
                return Err(VulkanError(
                    "VUI_VULKAN_VALIDATION requested, but VK_LAYER_KHRONOS_validation is unavailable"
                        .to_owned(),
                ));
            }
            let layers = if validation_requested {
                vec![validation_name.as_ptr()]
            } else {
                Vec::new()
            };
            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&application)
                .enabled_extension_names(&extensions)
                .enabled_layer_names(&layers)
                .flags(instance_flags);
            let instance = entry.create_instance(&create_info, None)?;
            let surface_loader = khr::surface::Instance::new(&entry, &instance);
            let surface = match ash_window::create_surface(
                &entry,
                &instance,
                display.as_raw(),
                window_handle.as_raw(),
                None,
            ) {
                Ok(surface) => surface,
                Err(error) => {
                    instance.destroy_instance(None);
                    return Err(error.into());
                }
            };

            let selected = match select_device(
                &instance,
                "graphics/transfer and presentation requirements",
                None,
                |physical, index, properties| {
                    let graphics = properties.queue_flags.contains(vk::QueueFlags::GRAPHICS);
                    let present = surface_loader
                        .get_physical_device_surface_support(physical, index, surface)
                        .unwrap_or(false);
                    graphics && present
                },
            ) {
                Ok(selected) => selected,
                Err(error) => {
                    surface_loader.destroy_surface(surface, None);
                    instance.destroy_instance(None);
                    return Err(error);
                }
            };
            let physical_device = selected.physical;
            let queue_family = selected.queue_family;

            let priorities = [1.0_f32];
            let queue_info = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family)
                .queue_priorities(&priorities)];
            #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(unused_mut))]
            let mut device_extensions = vec![khr::swapchain::NAME.as_ptr()];
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            device_extensions.push(khr::portability_subset::NAME.as_ptr());
            let device_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_info)
                .enabled_extension_names(&device_extensions);
            let device = match instance.create_device(physical_device, &device_info, None) {
                Ok(device) => device,
                Err(error) => {
                    surface_loader.destroy_surface(surface, None);
                    instance.destroy_instance(None);
                    return Err(error.into());
                }
            };
            let queue = device.get_device_queue(queue_family, 0);
            let timestamps_supported = selected.info.timestamp_queries;
            let timestamp_period = selected.timestamp_period;
            let swapchain_loader = khr::swapchain::Device::new(&instance, &device);
            let command_pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            let command_buffers: [vk::CommandBuffer; 2] = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(2),
                )?
                .try_into()
                .map_err(|_| {
                    VulkanError("Vulkan returned an invalid command-buffer count".to_owned())
                })?;
            let image_available = [
                device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
                device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
            ];
            let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
            let fences = [
                device.create_fence(&fence_info, None)?,
                device.create_fence(&fence_info, None)?,
            ];
            let timestamp_queries = if timestamps_supported {
                let info = vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::TIMESTAMP)
                    .query_count(2);
                [
                    device.create_query_pool(&info, None)?,
                    device.create_query_pool(&info, None)?,
                ]
            } else {
                [vk::QueryPool::null(); 2]
            };

            let depth_format = choose_depth_format(&instance, physical_device)?;
            let sample_count = selected.sample_count;
            let mut presenter = Self {
                _entry: entry,
                instance,
                surface_loader,
                surface,
                physical_device,
                device_info: selected.info,
                device,
                queue,
                swapchain_loader,
                swapchain: vk::SwapchainKHR::null(),
                images: Vec::new(),
                image_views: Vec::new(),
                framebuffers: Vec::new(),
                format: vk::Format::UNDEFINED,
                extent: vk::Extent2D::default(),
                render_pass: vk::RenderPass::null(),
                sample_count,
                multisample_image: vk::Image::null(),
                multisample_memory: vk::DeviceMemory::null(),
                multisample_view: vk::ImageView::null(),
                depth_format,
                depth_image: vk::Image::null(),
                depth_memory: vk::DeviceMemory::null(),
                depth_view: vk::ImageView::null(),
                descriptor_layout: vk::DescriptorSetLayout::null(),
                descriptor_pool: vk::DescriptorPool::null(),
                descriptor_set: vk::DescriptorSet::null(),
                pipeline_layout: vk::PipelineLayout::null(),
                draw_pipeline: vk::Pipeline::null(),
                bootstrap_pipeline: vk::Pipeline::null(),
                clip_push_pipeline: vk::Pipeline::null(),
                clip_pop_pipeline: vk::Pipeline::null(),
                sampler: vk::Sampler::null(),
                atlas_image: vk::Image::null(),
                atlas_memory: vk::DeviceMemory::null(),
                atlas_view: vk::ImageView::null(),
                atlas_extent: vk::Extent2D::default(),
                atlas_revision: 0,
                geometry: [vk::Buffer::null(); 2],
                geometry_memory: [vk::DeviceMemory::null(); 2],
                geometry_capacity: [0; 2],
                geometry_revision: [0; 2],
                command_pool,
                command_buffers,
                image_available,
                render_finished: Vec::new(),
                fences,
                staging: [vk::Buffer::null(); 2],
                staging_memory: [vk::DeviceMemory::null(); 2],
                staging_capacity: [0; 2],
                current_frame: 0,
                timestamp_queries,
                timestamp_pending: [false; 2],
                timestamp_period,
                last_gpu_time: None,
                last_upload_stats: VulkanUploadStats::default(),
            };
            presenter.create_gpu_static()?;
            presenter.recreate_swapchain(width.max(1), height.max(1))?;
            Ok(presenter)
        }
    }

    /// Duration of the most recently completed GPU frame, when timestamp
    /// queries are supported by the selected graphics queue.
    #[must_use]
    pub const fn last_gpu_time(&self) -> Option<Duration> {
        self.last_gpu_time
    }

    /// Details of the physical adapter selected for this presenter.
    #[must_use]
    pub const fn device_info(&self) -> &VulkanDeviceInfo {
        &self.device_info
    }

    /// Actual bytes uploaded while preparing the most recently submitted frame.
    #[must_use]
    pub const fn last_upload_stats(&self) -> VulkanUploadStats {
        self.last_upload_stats
    }

    /// Uploads and presents one premultiplied RGBA8 frame.
    ///
    /// # Errors
    ///
    /// Returns an error if staging allocation, command submission, swapchain
    /// acquisition, or presentation fails.
    #[allow(clippy::too_many_lines)]
    pub fn present(&mut self, frame: &RasterFrame) -> Result<(), VulkanError> {
        if frame.width == 0 || frame.height == 0 {
            return Ok(());
        }
        // SAFETY: synchronization serializes reuse of all per-frame resources;
        // mapped writes stay within the allocation, and image barriers bracket
        // the transfer before presentation.
        unsafe {
            let frame_slot = self.current_frame;
            let command_buffer = self.command_buffers[frame_slot];
            let image_available = self.image_available[frame_slot];
            let fence = self.fences[frame_slot];
            self.device.wait_for_fences(&[fence], true, u64::MAX)?;
            if self.extent.width != frame.width || self.extent.height != frame.height {
                self.recreate_swapchain(frame.width, frame.height)?;
            }
            let byte_len = u64::try_from(frame.pixels.len())
                .map_err(|_| VulkanError("frame byte count exceeds Vulkan limits".to_owned()))?;
            self.ensure_staging(frame_slot, byte_len)?;
            let mapped = self.device.map_memory(
                self.staging_memory[frame_slot],
                0,
                byte_len,
                vk::MemoryMapFlags::empty(),
            )?;
            if matches!(
                self.format,
                vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB
            ) {
                let destination =
                    std::slice::from_raw_parts_mut(mapped.cast::<u8>(), frame.pixels.len());
                for (source, target) in frame
                    .pixels
                    .chunks_exact(4)
                    .zip(destination.chunks_exact_mut(4))
                {
                    target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
                }
            } else {
                ptr::copy_nonoverlapping(
                    frame.pixels.as_ptr(),
                    mapped.cast::<u8>(),
                    frame.pixels.len(),
                );
            }
            self.device.unmap_memory(self.staging_memory[frame_slot]);

            inject_device_loss("acquire-device-lost-once")?;
            let (image_index, suboptimal) = match self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                image_available,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.recreate_swapchain(frame.width, frame.height)?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            let render_finished = self.render_finished[image_index as usize];
            self.device.reset_fences(&[fence])?;
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            let image = self.images[image_index as usize];
            let to_transfer = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(color_range())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer],
            );
            let copy = vk::BufferImageCopy::default()
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(0)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: frame.width,
                    height: frame.height,
                    depth: 1,
                });
            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                self.staging[frame_slot],
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[copy],
            );
            let to_present = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(color_range())
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE);
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present],
            );
            self.device.end_command_buffer(command_buffer)?;
            let waits = [image_available];
            let wait_stages = [vk::PipelineStageFlags::TRANSFER];
            let commands = [command_buffer];
            let signals = [render_finished];
            let submit = [vk::SubmitInfo::default()
                .wait_semaphores(&waits)
                .wait_dst_stage_mask(&wait_stages)
                .command_buffers(&commands)
                .signal_semaphores(&signals)];
            self.device.queue_submit(self.queue, &submit, fence)?;
            let swapchains = [self.swapchain];
            let indices = [image_index];
            let present = vk::PresentInfoKHR::default()
                .wait_semaphores(&signals)
                .swapchains(&swapchains)
                .image_indices(&indices);
            match self.swapchain_loader.queue_present(self.queue, &present) {
                Ok(present_suboptimal) if suboptimal || present_suboptimal => {
                    self.device.wait_for_fences(&[fence], true, u64::MAX)?;
                    self.recreate_swapchain(frame.width, frame.height)?;
                }
                Ok(_) => {}
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    self.device.wait_for_fences(&[fence], true, u64::MAX)?;
                    self.recreate_swapchain(frame.width, frame.height)?;
                }
                Err(error) => return Err(error.into()),
            }
            self.current_frame = (frame_slot + 1) % self.command_buffers.len();
        }
        Ok(())
    }

    /// Draws tessellated VUI geometry directly with Vulkan and presents it.
    ///
    /// # Errors
    ///
    /// Returns an error if GPU resource allocation, recording, submission, or
    /// presentation fails.
    #[allow(
        clippy::cast_precision_loss,
        clippy::too_many_lines,
        reason = "Vulkan viewport dimensions are represented as f32"
    )]
    pub fn present_gpu(&mut self, scene: &GpuScene) -> Result<(), VulkanError> {
        if scene.width == 0 || scene.height == 0 {
            return Ok(());
        }
        // SAFETY: one fence serializes every mutable per-frame resource. All
        // mapped ranges and draw ranges derive from owned scene vectors.
        unsafe {
            let frame_slot = self.current_frame;
            let command_buffer = self.command_buffers[frame_slot];
            let image_available = self.image_available[frame_slot];
            let fence = self.fences[frame_slot];
            self.device.wait_for_fences(&[fence], true, u64::MAX)?;
            self.update_gpu_time(frame_slot)?;
            if self.extent.width != scene.width || self.extent.height != scene.height {
                self.recreate_swapchain(scene.width, scene.height)?;
            }
            let atlas_changed = self.atlas_revision != scene.atlas_revision
                || self.atlas_extent.width != scene.atlas_width
                || self.atlas_extent.height != scene.atlas_height;
            if atlas_changed {
                // The atlas is shared by both frame slots. Wait only when it
                // changes so no in-flight fragment shader can sample a region
                // while transfer writes update it.
                self.device.device_wait_idle()?;
            }
            self.ensure_atlas(scene.atlas_width, scene.atlas_height)?;
            let atlas_initialized = self.atlas_revision != 0;
            let upload_atlas = self.atlas_revision != scene.atlas_revision;
            let reset_atlas = upload_atlas && scene.atlas_rebuilt;
            let full_atlas_upload = upload_atlas
                && !reset_atlas
                && (!atlas_initialized || self.atlas_revision != scene.atlas_base_revision);
            let vertex_bytes: &[u8] = bytemuck::cast_slice(&scene.vertices);
            let index_bytes: &[u8] = bytemuck::cast_slice(&scene.indices);
            let instance_bytes: &[u8] = bytemuck::cast_slice(&scene.bootstrap_instances);
            let index_offset = align_up(vertex_bytes.len() as u64, 4);
            let index_offset_usize = usize::try_from(index_offset)
                .map_err(|_| VulkanError("geometry offset exceeds address space".to_owned()))?;
            let instance_offset = align_up(index_offset + index_bytes.len() as u64, 4);
            let instance_offset_usize = usize::try_from(instance_offset)
                .map_err(|_| VulkanError("instance offset exceeds address space".to_owned()))?;
            let geometry_len = instance_offset + instance_bytes.len() as u64;
            self.ensure_geometry(frame_slot, geometry_len.max(4))?;
            let upload_geometry = self.geometry_revision[frame_slot] != scene.geometry_revision;
            let atlas_len = if full_atlas_upload {
                scene.atlas.len() as vk::DeviceSize
            } else if upload_atlas {
                scene
                    .atlas_dirty
                    .iter()
                    .map(|dirty| dirty.pixels.len() as u64)
                    .sum()
            } else {
                0
            };
            let geometry_staging_offset = align_up(atlas_len, 4);
            let geometry_staging_offset_usize = usize::try_from(geometry_staging_offset)
                .map_err(|_| VulkanError("staging offset exceeds address space".to_owned()))?;
            let staging_len =
                geometry_staging_offset + if upload_geometry { geometry_len } else { 0 };
            if staging_len > 0 {
                self.ensure_staging(frame_slot, staging_len)?;
                let mapped = self.device.map_memory(
                    self.staging_memory[frame_slot],
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
                    let geometry_target = mapped.cast::<u8>().add(geometry_staging_offset_usize);
                    ptr::copy_nonoverlapping(
                        vertex_bytes.as_ptr(),
                        geometry_target,
                        vertex_bytes.len(),
                    );
                    ptr::copy_nonoverlapping(
                        index_bytes.as_ptr(),
                        geometry_target.add(index_offset_usize),
                        index_bytes.len(),
                    );
                    ptr::copy_nonoverlapping(
                        instance_bytes.as_ptr(),
                        geometry_target.add(instance_offset_usize),
                        instance_bytes.len(),
                    );
                }
                self.device.unmap_memory(self.staging_memory[frame_slot]);
            }

            let (image_index, suboptimal) = match self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                image_available,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    self.recreate_swapchain(scene.width, scene.height)?;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            };
            let render_finished = self.render_finished[image_index as usize];
            self.device.reset_fences(&[fence])?;
            self.device
                .reset_command_buffer(command_buffer, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if self.timestamp_queries[frame_slot] != vk::QueryPool::null() {
                self.device.cmd_reset_query_pool(
                    command_buffer,
                    self.timestamp_queries[frame_slot],
                    0,
                    2,
                );
                self.device.cmd_write_timestamp(
                    command_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.timestamp_queries[frame_slot],
                    0,
                );
            }
            if upload_geometry && geometry_len > 0 {
                self.device.cmd_copy_buffer(
                    command_buffer,
                    self.staging[frame_slot],
                    self.geometry[frame_slot],
                    &[vk::BufferCopy::default()
                        .src_offset(geometry_staging_offset)
                        .size(geometry_len)],
                );
                let geometry_ready = vk::BufferMemoryBarrier::default()
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(self.geometry[frame_slot])
                    .offset(0)
                    .size(geometry_len)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::VERTEX_ATTRIBUTE_READ | vk::AccessFlags::INDEX_READ,
                    );
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::VERTEX_INPUT,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[geometry_ready],
                    &[],
                );
            }
            if upload_atlas {
                let atlas_barrier = vk::ImageMemoryBarrier::default()
                    .old_layout(if atlas_initialized {
                        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                    } else {
                        vk::ImageLayout::UNDEFINED
                    })
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.atlas_image)
                    .subresource_range(color_range())
                    .src_access_mask(if atlas_initialized {
                        vk::AccessFlags::SHADER_READ
                    } else {
                        vk::AccessFlags::empty()
                    })
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    if atlas_initialized {
                        vk::PipelineStageFlags::FRAGMENT_SHADER
                    } else {
                        vk::PipelineStageFlags::TOP_OF_PIPE
                    },
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[atlas_barrier],
                );
                if reset_atlas {
                    self.device.cmd_clear_color_image(
                        command_buffer,
                        self.atlas_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &vk::ClearColorValue { uint32: [0; 4] },
                        &[color_range()],
                    );
                    let clear_ready = vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(self.atlas_image)
                        .subresource_range(color_range())
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
                    self.device.cmd_pipeline_barrier(
                        command_buffer,
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
                        .collect()
                };
                if !copies.is_empty() {
                    self.device.cmd_copy_buffer_to_image(
                        command_buffer,
                        self.staging[frame_slot],
                        self.atlas_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &copies,
                    );
                }
                let atlas_ready = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(self.atlas_image)
                    .subresource_range(color_range())
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);
                self.device.cmd_pipeline_barrier(
                    command_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[atlas_ready],
                );
            }

            let clear = [
                vk::ClearValue {
                    color: vk::ClearColorValue { float32: [0.0; 4] },
                },
                vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 1.0,
                        stencil: 0,
                    },
                },
            ];
            self.device.cmd_begin_render_pass(
                command_buffer,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(self.render_pass)
                    .framebuffer(self.framebuffers[image_index as usize])
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent: self.extent,
                    })
                    .clear_values(&clear),
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_set_viewport(
                command_buffer,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.extent.width as f32,
                    height: self.extent.height as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            self.device.cmd_set_scissor(
                command_buffer,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent: self.extent,
                }],
            );
            self.device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );
            self.device.cmd_bind_vertex_buffers(
                command_buffer,
                0,
                &[self.geometry[frame_slot]],
                &[0],
            );
            self.device.cmd_bind_vertex_buffers(
                command_buffer,
                1,
                &[self.geometry[frame_slot]],
                &[instance_offset],
            );
            self.device.cmd_bind_index_buffer(
                command_buffer,
                self.geometry[frame_slot],
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
                        command_buffer,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline,
                    );
                    bound = pipeline;
                }
                self.device.cmd_set_stencil_reference(
                    command_buffer,
                    vk::StencilFaceFlags::FRONT_AND_BACK,
                    command.stencil_reference,
                );
                self.device.cmd_draw_indexed(
                    command_buffer,
                    command.indices.end - command.indices.start,
                    command.instances.end - command.instances.start,
                    command.indices.start,
                    0,
                    command.instances.start,
                );
            }
            self.device.cmd_end_render_pass(command_buffer);
            if self.timestamp_queries[frame_slot] != vk::QueryPool::null() {
                self.device.cmd_write_timestamp(
                    command_buffer,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    self.timestamp_queries[frame_slot],
                    1,
                );
            }
            self.device.end_command_buffer(command_buffer)?;
            let waits = [image_available];
            let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let commands = [command_buffer];
            let signals = [render_finished];
            inject_device_loss("submit-device-lost-once")?;
            self.device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default()
                    .wait_semaphores(&waits)
                    .wait_dst_stage_mask(&wait_stages)
                    .command_buffers(&commands)
                    .signal_semaphores(&signals)],
                fence,
            )?;
            if upload_geometry {
                self.geometry_revision[frame_slot] = scene.geometry_revision;
            }
            if upload_atlas {
                self.atlas_revision = scene.atlas_revision;
            }
            self.last_upload_stats = VulkanUploadStats {
                geometry_bytes: if upload_geometry { geometry_len } else { 0 },
                atlas_bytes: atlas_len,
            };
            self.timestamp_pending[frame_slot] =
                self.timestamp_queries[frame_slot] != vk::QueryPool::null();
            let swapchains = [self.swapchain];
            let indices = [image_index];
            let present = vk::PresentInfoKHR::default()
                .wait_semaphores(&signals)
                .swapchains(&swapchains)
                .image_indices(&indices);
            inject_device_loss("present-device-lost-once")?;
            match self.swapchain_loader.queue_present(self.queue, &present) {
                Ok(was_suboptimal) if suboptimal || was_suboptimal => {
                    self.device.wait_for_fences(&[fence], true, u64::MAX)?;
                    self.recreate_swapchain(scene.width, scene.height)?;
                }
                Ok(_) => {}
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    self.device.wait_for_fences(&[fence], true, u64::MAX)?;
                    self.recreate_swapchain(scene.width, scene.height)?;
                }
                Err(error) => return Err(error.into()),
            }
            self.current_frame = (frame_slot + 1) % self.command_buffers.len();
        }
        Ok(())
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "timestamp duration intentionally converts hardware ticks to floating-point seconds"
    )]
    unsafe fn update_gpu_time(&mut self, frame: usize) -> Result<(), VulkanError> {
        if !self.timestamp_pending[frame] {
            return Ok(());
        }
        let mut timestamps = [0_u64; 2];
        unsafe {
            self.device.get_query_pool_results(
                self.timestamp_queries[frame],
                0,
                &mut timestamps,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
            )?;
        }
        let ticks = timestamps[1].saturating_sub(timestamps[0]);
        let nanoseconds = ticks as f64 * f64::from(self.timestamp_period);
        self.last_gpu_time = Some(Duration::from_secs_f64(nanoseconds / 1_000_000_000.0));
        self.timestamp_pending[frame] = false;
        Ok(())
    }

    unsafe fn create_gpu_static(&mut self) -> Result<(), VulkanError> {
        let resources = unsafe { create_descriptor_resources(&self.device)? };
        self.descriptor_layout = resources.layout;
        self.descriptor_pool = resources.pool;
        self.descriptor_set = resources.set;
        self.pipeline_layout = resources.pipeline_layout;
        self.sampler = resources.sampler;
        Ok(())
    }

    unsafe fn ensure_geometry(
        &mut self,
        frame: usize,
        required: vk::DeviceSize,
    ) -> Result<(), VulkanError> {
        if self.geometry_capacity[frame] >= required {
            return Ok(());
        }
        unsafe {
            destroy_buffer(
                &self.device,
                &mut self.geometry[frame],
                &mut self.geometry_memory[frame],
            );
        };
        let (buffer, memory) = unsafe {
            create_buffer(
                &self.instance,
                self.physical_device,
                &self.device,
                required.next_power_of_two(),
                vk::BufferUsageFlags::VERTEX_BUFFER
                    | vk::BufferUsageFlags::INDEX_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            )?
        };
        self.geometry[frame] = buffer;
        self.geometry_memory[frame] = memory;
        self.geometry_capacity[frame] = required.next_power_of_two();
        self.geometry_revision[frame] = 0;
        Ok(())
    }

    unsafe fn ensure_atlas(&mut self, width: u32, height: u32) -> Result<(), VulkanError> {
        let extent = vk::Extent2D { width, height };
        if self.atlas_extent == extent {
            return Ok(());
        }
        unsafe { self.destroy_atlas() };
        let (image, memory) = unsafe {
            create_image(
                &self.instance,
                self.physical_device,
                &self.device,
                extent,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
                vk::SampleCountFlags::TYPE_1,
            )?
        };
        self.atlas_image = image;
        self.atlas_memory = memory;
        self.atlas_view = unsafe {
            create_image_view(
                &self.device,
                image,
                vk::Format::R8G8B8A8_UNORM,
                vk::ImageAspectFlags::COLOR,
            )?
        };
        self.atlas_extent = extent;
        self.atlas_revision = 0;
        let image_info = [vk::DescriptorImageInfo::default()
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
                        .image_info(&image_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(self.descriptor_set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&sampler_info),
                ],
                &[],
            );
        };
        Ok(())
    }

    unsafe fn destroy_atlas(&mut self) {
        if self.atlas_view != vk::ImageView::null() {
            unsafe { self.device.destroy_image_view(self.atlas_view, None) };
        }
        if self.atlas_image != vk::Image::null() {
            unsafe { self.device.destroy_image(self.atlas_image, None) };
        }
        if self.atlas_memory != vk::DeviceMemory::null() {
            unsafe { self.device.free_memory(self.atlas_memory, None) };
        }
        self.atlas_view = vk::ImageView::null();
        self.atlas_image = vk::Image::null();
        self.atlas_memory = vk::DeviceMemory::null();
        self.atlas_extent = vk::Extent2D::default();
        self.atlas_revision = 0;
    }

    unsafe fn recreate_swapchain(&mut self, width: u32, height: u32) -> Result<(), VulkanError> {
        // SAFETY: caller ensures the device is externally synchronized.
        unsafe { self.device.device_wait_idle()? };
        let capabilities = unsafe {
            self.surface_loader
                .get_physical_device_surface_capabilities(self.physical_device, self.surface)?
        };
        let formats = unsafe {
            self.surface_loader
                .get_physical_device_surface_formats(self.physical_device, self.surface)?
        };
        let chosen = formats
            .iter()
            .copied()
            .find(|format| {
                matches!(
                    format.format,
                    vk::Format::R8G8B8A8_UNORM | vk::Format::B8G8R8A8_UNORM
                )
            })
            .or_else(|| {
                formats.iter().copied().find(|format| {
                    matches!(
                        format.format,
                        vk::Format::R8G8B8A8_SRGB | vk::Format::B8G8R8A8_SRGB
                    )
                })
            })
            .ok_or_else(|| VulkanError("surface has no supported 8-bit RGBA format".to_owned()))?;
        let required_usage =
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT;
        if !capabilities.supported_usage_flags.contains(required_usage) {
            return Err(VulkanError(
                "surface images cannot be transfer destinations and color attachments".to_owned(),
            ));
        }
        let extent = if capabilities.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: width.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: height.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        } else {
            capabilities.current_extent
        };
        let image_count = capabilities.min_image_count.saturating_add(1).min(
            if capabilities.max_image_count == 0 {
                u32::MAX
            } else {
                capabilities.max_image_count
            },
        );
        let composite_alpha = [
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::INHERIT,
        ]
        .into_iter()
        .find(|mode| capabilities.supported_composite_alpha.contains(*mode))
        .ok_or_else(|| VulkanError("surface has no composite alpha mode".to_owned()))?;
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(chosen.format)
            .image_color_space(chosen.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(required_usage)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(composite_alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(self.swapchain);
        let (replacement, images) = unsafe { self.create_replacement_swapchain(&info)? };
        if let Err(error) = unsafe { self.replace_render_finished_semaphores(images.len()) } {
            unsafe { self.swapchain_loader.destroy_swapchain(replacement, None) };
            return Err(error);
        }
        unsafe { self.destroy_swapchain_graphics() };
        if self.swapchain != vk::SwapchainKHR::null() {
            unsafe {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            };
        }
        self.swapchain = replacement;
        self.images = images;
        self.format = chosen.format;
        self.extent = extent;
        unsafe { self.create_swapchain_graphics()? };
        Ok(())
    }

    unsafe fn create_replacement_swapchain(
        &self,
        info: &vk::SwapchainCreateInfoKHR<'_>,
    ) -> Result<(vk::SwapchainKHR, Vec<vk::Image>), VulkanError> {
        let swapchain = unsafe { self.swapchain_loader.create_swapchain(info, None)? };
        match unsafe { self.swapchain_loader.get_swapchain_images(swapchain) } {
            Ok(images) => Ok((swapchain, images)),
            Err(error) => {
                unsafe { self.swapchain_loader.destroy_swapchain(swapchain, None) };
                Err(error.into())
            }
        }
    }

    unsafe fn replace_render_finished_semaphores(
        &mut self,
        count: usize,
    ) -> Result<(), VulkanError> {
        let replacement = unsafe { create_semaphores(&self.device, count)? };
        for semaphore in std::mem::replace(&mut self.render_finished, replacement) {
            unsafe { self.device.destroy_semaphore(semaphore, None) };
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    unsafe fn create_swapchain_graphics(&mut self) -> Result<(), VulkanError> {
        self.image_views = self
            .images
            .iter()
            .copied()
            .map(|image| unsafe {
                create_image_view(
                    &self.device,
                    image,
                    self.format,
                    vk::ImageAspectFlags::COLOR,
                )
            })
            .collect::<Result<_, _>>()?;
        self.render_pass = unsafe {
            create_render_pass(
                &self.device,
                self.format,
                self.depth_format,
                self.sample_count,
                vk::ImageLayout::PRESENT_SRC_KHR,
            )?
        };
        let multisampled = self.sample_count != vk::SampleCountFlags::TYPE_1;
        let (depth_image, depth_memory) = unsafe {
            create_image(
                &self.instance,
                self.physical_device,
                &self.device,
                self.extent,
                self.depth_format,
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
                self.sample_count,
            )?
        };
        self.depth_image = depth_image;
        self.depth_memory = depth_memory;
        self.depth_view = unsafe {
            create_image_view(
                &self.device,
                self.depth_image,
                self.depth_format,
                vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL,
            )?
        };
        if multisampled {
            let (image, memory) = unsafe {
                create_image(
                    &self.instance,
                    self.physical_device,
                    &self.device,
                    self.extent,
                    self.format,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT,
                    self.sample_count,
                )?
            };
            self.multisample_image = image;
            self.multisample_memory = memory;
            self.multisample_view = unsafe {
                create_image_view(
                    &self.device,
                    image,
                    self.format,
                    vk::ImageAspectFlags::COLOR,
                )?
            };
        }
        self.framebuffers = self
            .image_views
            .iter()
            .copied()
            .map(|view| {
                let views = if multisampled {
                    vec![self.multisample_view, self.depth_view, view]
                } else {
                    vec![view, self.depth_view]
                };
                unsafe {
                    self.device.create_framebuffer(
                        &vk::FramebufferCreateInfo::default()
                            .render_pass(self.render_pass)
                            .attachments(&views)
                            .width(self.extent.width)
                            .height(self.extent.height)
                            .layers(1),
                        None,
                    )
                }
                .map_err(VulkanError::from)
            })
            .collect::<Result<_, _>>()?;
        let pipelines = unsafe {
            create_pipelines(
                &self.device,
                self.render_pass,
                self.pipeline_layout,
                self.sample_count,
            )?
        };
        self.draw_pipeline = pipelines.draw;
        self.bootstrap_pipeline = pipelines.bootstrap;
        self.clip_push_pipeline = pipelines.clip_push;
        self.clip_pop_pipeline = pipelines.clip_pop;
        Ok(())
    }

    unsafe fn destroy_swapchain_graphics(&mut self) {
        for framebuffer in self.framebuffers.drain(..) {
            unsafe { self.device.destroy_framebuffer(framebuffer, None) };
        }
        for pipeline in [
            self.draw_pipeline,
            self.bootstrap_pipeline,
            self.clip_push_pipeline,
            self.clip_pop_pipeline,
        ] {
            if pipeline != vk::Pipeline::null() {
                unsafe { self.device.destroy_pipeline(pipeline, None) };
            }
        }
        self.draw_pipeline = vk::Pipeline::null();
        self.bootstrap_pipeline = vk::Pipeline::null();
        self.clip_push_pipeline = vk::Pipeline::null();
        self.clip_pop_pipeline = vk::Pipeline::null();
        if self.depth_view != vk::ImageView::null() {
            unsafe { self.device.destroy_image_view(self.depth_view, None) };
        }
        if self.depth_image != vk::Image::null() {
            unsafe { self.device.destroy_image(self.depth_image, None) };
        }
        if self.depth_memory != vk::DeviceMemory::null() {
            unsafe { self.device.free_memory(self.depth_memory, None) };
        }
        self.depth_view = vk::ImageView::null();
        self.depth_image = vk::Image::null();
        self.depth_memory = vk::DeviceMemory::null();
        if self.multisample_view != vk::ImageView::null() {
            unsafe { self.device.destroy_image_view(self.multisample_view, None) };
        }
        if self.multisample_image != vk::Image::null() {
            unsafe { self.device.destroy_image(self.multisample_image, None) };
        }
        if self.multisample_memory != vk::DeviceMemory::null() {
            unsafe { self.device.free_memory(self.multisample_memory, None) };
        }
        self.multisample_view = vk::ImageView::null();
        self.multisample_image = vk::Image::null();
        self.multisample_memory = vk::DeviceMemory::null();
        if self.render_pass != vk::RenderPass::null() {
            unsafe { self.device.destroy_render_pass(self.render_pass, None) };
        }
        self.render_pass = vk::RenderPass::null();
        for view in self.image_views.drain(..) {
            unsafe { self.device.destroy_image_view(view, None) };
        }
    }

    unsafe fn ensure_staging(
        &mut self,
        frame: usize,
        required: vk::DeviceSize,
    ) -> Result<(), VulkanError> {
        if self.staging_capacity[frame] >= required {
            return Ok(());
        }
        if self.staging[frame] != vk::Buffer::null() {
            unsafe { self.device.destroy_buffer(self.staging[frame], None) };
            unsafe { self.device.free_memory(self.staging_memory[frame], None) };
        }
        let info = vk::BufferCreateInfo::default()
            .size(required)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        self.staging[frame] = unsafe { self.device.create_buffer(&info, None)? };
        let requirements = unsafe {
            self.device
                .get_buffer_memory_requirements(self.staging[frame])
        };
        let properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        let memory_type = (0..properties.memory_type_count)
            .find(|index| {
                requirements.memory_type_bits & (1 << index) != 0
                    && properties.memory_types[*index as usize]
                        .property_flags
                        .contains(
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
            })
            .ok_or_else(|| VulkanError("no host-visible coherent Vulkan memory type".to_owned()))?;
        self.staging_memory[frame] = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )?
        };
        unsafe {
            self.device
                .bind_buffer_memory(self.staging[frame], self.staging_memory[frame], 0)?;
        };
        self.staging_capacity[frame] = required;
        Ok(())
    }
}

pub(crate) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}

unsafe fn create_semaphores(
    device: &ash::Device,
    count: usize,
) -> Result<Vec<vk::Semaphore>, VulkanError> {
    let mut semaphores = Vec::with_capacity(count);
    for _ in 0..count {
        match unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) } {
            Ok(semaphore) => semaphores.push(semaphore),
            Err(error) => {
                for semaphore in semaphores.drain(..) {
                    unsafe { device.destroy_semaphore(semaphore, None) };
                }
                return Err(error.into());
            }
        }
    }
    Ok(semaphores)
}

fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

pub(crate) fn choose_depth_format(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Result<vk::Format, VulkanError> {
    [
        vk::Format::D24_UNORM_S8_UINT,
        vk::Format::D32_SFLOAT_S8_UINT,
    ]
    .into_iter()
    .find(|format| {
        // SAFETY: the physical device belongs to this instance.
        let properties =
            unsafe { instance.get_physical_device_format_properties(physical, *format) };
        properties
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
    })
    .ok_or_else(|| VulkanError("device has no depth-stencil attachment format".to_owned()))
}

pub(crate) unsafe fn create_image(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    extent: vk::Extent2D,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    samples: vk::SampleCountFlags,
) -> Result<(vk::Image, vk::DeviceMemory), VulkanError> {
    let image = unsafe {
        device.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )?
    };
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    let memory_type = memory_type(
        instance,
        physical,
        requirements.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;
    let memory = match unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type),
            None,
        )
    } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe { device.destroy_image(image, None) };
            return Err(error.into());
        }
    };
    if let Err(error) = unsafe { device.bind_image_memory(image, memory, 0) } {
        unsafe {
            device.free_memory(memory, None);
            device.destroy_image(image, None);
        }
        return Err(error.into());
    }
    Ok((image, memory))
}

pub(crate) unsafe fn create_image_view(
    device: &ash::Device,
    image: vk::Image,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
) -> Result<vk::ImageView, VulkanError> {
    Ok(unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(aspect)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                ),
            None,
        )?
    })
}

pub(crate) unsafe fn create_buffer(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    flags: vk::MemoryPropertyFlags,
) -> Result<(vk::Buffer, vk::DeviceMemory), VulkanError> {
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?
    };
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
    let memory_type = memory_type(instance, physical, requirements.memory_type_bits, flags)?;
    let memory = match unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type),
            None,
        )
    } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe { device.destroy_buffer(buffer, None) };
            return Err(error.into());
        }
    };
    if let Err(error) = unsafe { device.bind_buffer_memory(buffer, memory, 0) } {
        unsafe {
            device.free_memory(memory, None);
            device.destroy_buffer(buffer, None);
        }
        return Err(error.into());
    }
    Ok((buffer, memory))
}

pub(crate) unsafe fn destroy_buffer(
    device: &ash::Device,
    buffer: &mut vk::Buffer,
    memory: &mut vk::DeviceMemory,
) {
    if *buffer != vk::Buffer::null() {
        unsafe { device.destroy_buffer(*buffer, None) };
    }
    if *memory != vk::DeviceMemory::null() {
        unsafe { device.free_memory(*memory, None) };
    }
    *buffer = vk::Buffer::null();
    *memory = vk::DeviceMemory::null();
}

fn memory_type(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    allowed: u32,
    required: vk::MemoryPropertyFlags,
) -> Result<u32, VulkanError> {
    // SAFETY: the physical device belongs to this instance.
    let properties = unsafe { instance.get_physical_device_memory_properties(physical) };
    (0..properties.memory_type_count)
        .find(|index| {
            allowed & (1 << index) != 0
                && properties.memory_types[*index as usize]
                    .property_flags
                    .contains(required)
        })
        .ok_or_else(|| VulkanError(format!("no Vulkan memory type supports {required:?}")))
}

impl Drop for VulkanPresenter {
    fn drop(&mut self) {
        // SAFETY: all handles belong to this presenter and device idle ensures
        // none remain in use. Destruction follows Vulkan parent-child order.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.destroy_swapchain_graphics();
            self.destroy_atlas();
            for frame in 0..self.command_buffers.len() {
                destroy_buffer(
                    &self.device,
                    &mut self.geometry[frame],
                    &mut self.geometry_memory[frame],
                );
                if self.staging[frame] != vk::Buffer::null() {
                    self.device.destroy_buffer(self.staging[frame], None);
                    self.device.free_memory(self.staging_memory[frame], None);
                }
                self.device.destroy_fence(self.fences[frame], None);
                self.device
                    .destroy_semaphore(self.image_available[frame], None);
                if self.timestamp_queries[frame] != vk::QueryPool::null() {
                    self.device
                        .destroy_query_pool(self.timestamp_queries[frame], None);
                }
            }
            for semaphore in self.render_finished.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.device.destroy_command_pool(self.command_pool, None);
            if self.sampler != vk::Sampler::null() {
                self.device.destroy_sampler(self.sampler, None);
            }
            if self.descriptor_pool != vk::DescriptorPool::null() {
                self.device
                    .destroy_descriptor_pool(self.descriptor_pool, None);
            }
            if self.pipeline_layout != vk::PipelineLayout::null() {
                self.device
                    .destroy_pipeline_layout(self.pipeline_layout, None);
            }
            if self.descriptor_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .destroy_descriptor_set_layout(self.descriptor_layout, None);
            }
            if self.swapchain != vk::SwapchainKHR::null() {
                self.swapchain_loader
                    .destroy_swapchain(self.swapchain, None);
            }
            self.device.destroy_device(None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.instance.destroy_instance(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VulkanError;
    use ash::vk;

    #[test]
    fn classifies_device_loss_without_hiding_other_errors() {
        assert!(VulkanError::from(vk::Result::ERROR_DEVICE_LOST).is_device_lost());
        assert!(!VulkanError::from(vk::Result::ERROR_OUT_OF_DATE_KHR).is_device_lost());
    }
}
