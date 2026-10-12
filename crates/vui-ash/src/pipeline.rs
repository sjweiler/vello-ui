use std::mem;

use ash::vk;

use crate::{
    gpu::{BootstrapInstance, Vertex},
    vulkan::VulkanError,
};

pub(crate) struct DescriptorResources {
    pub layout: vk::DescriptorSetLayout,
    pub pool: vk::DescriptorPool,
    pub set: vk::DescriptorSet,
    pub pipeline_layout: vk::PipelineLayout,
    pub sampler: vk::Sampler,
}

pub(crate) struct Pipelines {
    pub draw: vk::Pipeline,
    pub bootstrap: vk::Pipeline,
    pub clip_push: vk::Pipeline,
    pub clip_pop: vk::Pipeline,
}

struct CompiledShaders {
    vertex: Vec<u32>,
    bootstrap_vertex: Vec<u32>,
    fragment: Vec<u32>,
}

pub(crate) unsafe fn create_descriptor_resources(
    device: &ash::Device,
) -> Result<DescriptorResources, VulkanError> {
    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
    ];
    let layout = unsafe {
        device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )?
    };
    let layouts = [layout];
    let pipeline_layout = unsafe {
        device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
            None,
        )?
    };
    let sizes = [
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1),
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::SAMPLER)
            .descriptor_count(1),
    ];
    let pool = unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&sizes),
            None,
        )?
    };
    let set = unsafe {
        device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&layouts),
        )?
    }[0];
    let sampler = unsafe {
        device.create_sampler(
            &vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE),
            None,
        )?
    };
    Ok(DescriptorResources {
        layout,
        pool,
        set,
        pipeline_layout,
        sampler,
    })
}

pub(crate) unsafe fn create_render_pass(
    device: &ash::Device,
    color_format: vk::Format,
    depth_format: vk::Format,
    samples: vk::SampleCountFlags,
    final_layout: vk::ImageLayout,
) -> Result<vk::RenderPass, VulkanError> {
    let multisampled = samples != vk::SampleCountFlags::TYPE_1;
    let mut attachments = vec![
        vk::AttachmentDescription::default()
            .format(color_format)
            .samples(samples)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(if multisampled {
                vk::AttachmentStoreOp::DONT_CARE
            } else {
                vk::AttachmentStoreOp::STORE
            })
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(if multisampled {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            } else {
                final_layout
            }),
        vk::AttachmentDescription::default()
            .format(depth_format)
            .samples(samples)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::CLEAR)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL),
    ];
    if multisampled {
        attachments.push(
            vk::AttachmentDescription::default()
                .format(color_format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(final_layout),
        );
    }
    let colors = [vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let depth = vk::AttachmentReference::default()
        .attachment(1)
        .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
    let resolves = [vk::AttachmentReference::default()
        .attachment(2)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let mut subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&colors)
        .depth_stencil_attachment(&depth);
    if multisampled {
        subpass = subpass.resolve_attachments(&resolves);
    }
    let subpasses = [subpass];
    let dependencies = [vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
    Ok(unsafe {
        device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies),
            None,
        )?
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) unsafe fn create_pipelines(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    samples: vk::SampleCountFlags,
) -> Result<Pipelines, VulkanError> {
    let shaders = compiled_shaders()?;
    let vertex = unsafe {
        device.create_shader_module(
            &vk::ShaderModuleCreateInfo::default().code(&shaders.vertex),
            None,
        )?
    };
    let fragment = unsafe {
        device.create_shader_module(
            &vk::ShaderModuleCreateInfo::default().code(&shaders.fragment),
            None,
        )?
    };
    let bootstrap_vertex = unsafe {
        device.create_shader_module(
            &vk::ShaderModuleCreateInfo::default().code(&shaders.bootstrap_vertex),
            None,
        )?
    };
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex)
            .name(c"vs_main"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment)
            .name(c"fs_main"),
    ];
    let bootstrap_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(bootstrap_vertex)
            .name(c"vs_bootstrap"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment)
            .name(c"fs_main"),
    ];
    let bindings = [vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(u32::try_from(mem::size_of::<Vertex>()).expect("vertex stride fits u32"))
        .input_rate(vk::VertexInputRate::VERTEX)];
    let attributes = [
        vk::VertexInputAttributeDescription::default()
            .location(0)
            .binding(0)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(1)
            .binding(0)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(8),
        vk::VertexInputAttributeDescription::default()
            .location(2)
            .binding(0)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(24),
        vk::VertexInputAttributeDescription::default()
            .location(3)
            .binding(0)
            .format(vk::Format::R32_SFLOAT)
            .offset(32),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);
    let bootstrap_bindings = [
        bindings[0],
        vk::VertexInputBindingDescription::default()
            .binding(1)
            .stride(
                u32::try_from(mem::size_of::<BootstrapInstance>())
                    .expect("instance stride fits u32"),
            )
            .input_rate(vk::VertexInputRate::INSTANCE),
    ];
    let bootstrap_attributes = [
        attributes[0],
        attributes[1],
        attributes[2],
        attributes[3],
        vk::VertexInputAttributeDescription::default()
            .location(4)
            .binding(1)
            .format(vk::Format::R32G32_SFLOAT)
            .offset(0),
        vk::VertexInputAttributeDescription::default()
            .location(5)
            .binding(1)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .offset(8),
    ];
    let bootstrap_vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bootstrap_bindings)
        .vertex_attribute_descriptions(&bootstrap_attributes);
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample =
        vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(samples);
    let color = [vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
    let no_color = [vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::empty())];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&color);
    let clip_blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&no_color);
    let dynamic_values = [
        vk::DynamicState::VIEWPORT,
        vk::DynamicState::SCISSOR,
        vk::DynamicState::STENCIL_REFERENCE,
    ];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_values);
    let stencil = |op| {
        vk::StencilOpState::default()
            .pass_op(op)
            .fail_op(vk::StencilOp::KEEP)
            .depth_fail_op(vk::StencilOp::KEEP)
            .compare_op(vk::CompareOp::EQUAL)
            .compare_mask(0xff)
            .write_mask(0xff)
    };
    let draw_depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .stencil_test_enable(true)
        .front(stencil(vk::StencilOp::KEEP))
        .back(stencil(vk::StencilOp::KEEP));
    let push_depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .stencil_test_enable(true)
        .front(stencil(vk::StencilOp::INCREMENT_AND_CLAMP))
        .back(stencil(vk::StencilOp::INCREMENT_AND_CLAMP));
    let pop_depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .stencil_test_enable(true)
        .front(stencil(vk::StencilOp::DECREMENT_AND_CLAMP))
        .back(stencil(vk::StencilOp::DECREMENT_AND_CLAMP));
    let common = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .dynamic_state(&dynamic)
        .layout(layout)
        .render_pass(render_pass)
        .subpass(0);
    let bootstrap_common = common
        .stages(&bootstrap_stages)
        .vertex_input_state(&bootstrap_vertex_input);
    let infos = [
        common
            .depth_stencil_state(&draw_depth)
            .color_blend_state(&blend),
        bootstrap_common
            .depth_stencil_state(&draw_depth)
            .color_blend_state(&blend),
        common
            .depth_stencil_state(&push_depth)
            .color_blend_state(&clip_blend),
        common
            .depth_stencil_state(&pop_depth)
            .color_blend_state(&clip_blend),
    ];
    let pipelines =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None) }
            .map_err(|(_, error)| VulkanError::from(error));
    unsafe {
        device.destroy_shader_module(fragment, None);
        device.destroy_shader_module(vertex, None);
        device.destroy_shader_module(bootstrap_vertex, None);
    }
    let pipelines = pipelines?;
    Ok(Pipelines {
        draw: pipelines[0],
        bootstrap: pipelines[1],
        clip_push: pipelines[2],
        clip_pop: pipelines[3],
    })
}

fn compiled_shaders() -> Result<CompiledShaders, VulkanError> {
    const VERTEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ui.vert.spv"));
    const BOOTSTRAP_VERTEX: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/ui.bootstrap.vert.spv"));
    const FRAGMENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ui.frag.spv"));
    Ok(CompiledShaders {
        vertex: shader_words(VERTEX)?,
        bootstrap_vertex: shader_words(BOOTSTRAP_VERTEX)?,
        fragment: shader_words(FRAGMENT)?,
    })
}

fn shader_words(bytes: &[u8]) -> Result<Vec<u32>, VulkanError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(VulkanError(
            "compiled shader has an invalid length".to_owned(),
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}
