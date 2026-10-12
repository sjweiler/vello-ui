use ash::vk;
use vui_ash::EmbeddedRenderer;
use vui_core::{Color, PaintList, PaintOp, Rect, Size, UiFrame};

#[test]
fn renders_a_sampleable_image_without_taking_host_device_ownership() {
    // SAFETY: the test keeps the Vulkan loader, instance, physical device,
    // logical device, and queue alive in dependency order and destroys them
    // only after VUI has released every child resource.
    unsafe {
        let Ok(entry) = ash::Entry::load() else {
            eprintln!("host-device Vulkan test skipped: loader unavailable");
            return;
        };
        let application = vk::ApplicationInfo::default()
            .application_name(c"VUI host-device test")
            .engine_name(c"VUI")
            .api_version(vk::API_VERSION_1_1);
        let layers = validation_layers(&entry);
        let instance = entry
            .create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&application)
                    .enabled_layer_names(&layers),
                None,
            )
            .expect("create host Vulkan instance");
        let Some((physical, queue_family)) = instance
            .enumerate_physical_devices()
            .expect("enumerate host Vulkan devices")
            .into_iter()
            .find_map(|physical| {
                instance
                    .get_physical_device_queue_family_properties(physical)
                    .iter()
                    .position(|properties| {
                        properties.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    })
                    .and_then(|index| u32::try_from(index).ok())
                    .map(|family| (physical, family))
            })
        else {
            instance.destroy_instance(None);
            eprintln!("host-device Vulkan test skipped: no graphics device");
            return;
        };
        let priorities = [1.0_f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let device = instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
                None,
            )
            .expect("create host Vulkan device");
        let queue = device.get_device_queue(queue_family, 0);

        {
            let mut renderer = EmbeddedRenderer::from_host_device(
                &instance,
                physical,
                &device,
                queue,
                queue_family,
                Some(1),
            )
            .expect("create VUI renderer on host device");
            let first = renderer
                .render(
                    &frame(96.0, 64.0, Color::rgb(30, 120, 220)),
                    1.0,
                    Color::rgba(0, 0, 0, 0),
                )
                .expect("render first host image");
            assert_ne!(first.image, vk::Image::null());
            assert_ne!(first.view, vk::ImageView::null());
            assert_eq!((first.width, first.height), (96, 64));
            assert_eq!(first.format, vk::Format::R8G8B8A8_UNORM);
            assert_eq!(first.layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

            let second = renderer
                .render(
                    &frame(128.0, 80.0, Color::rgb(220, 80, 40)),
                    1.0,
                    Color::rgba(0, 0, 0, 0),
                )
                .expect("render resized host image");
            assert_eq!((second.width, second.height), (128, 80));
        }

        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("host device remains valid after VUI renderer drop");
        device.destroy_fence(fence, None);
        device.destroy_device(None);
        instance.destroy_instance(None);
    }
}

unsafe fn validation_layers(entry: &ash::Entry) -> Vec<*const std::ffi::c_char> {
    if std::env::var_os("VUI_VULKAN_VALIDATION").is_none() {
        return Vec::new();
    }
    let name = c"VK_LAYER_KHRONOS_validation";
    let available = unsafe { entry.enumerate_instance_layer_properties() }
        .expect("enumerate Vulkan layers")
        .iter()
        .any(|layer| unsafe { std::ffi::CStr::from_ptr(layer.layer_name.as_ptr()) == name });
    assert!(
        available,
        "Vulkan validation was requested but is unavailable"
    );
    vec![name.as_ptr()]
}

fn frame(width: f64, height: f64, color: Color) -> UiFrame {
    let mut paint = PaintList::default();
    paint.push(PaintOp::FillRect {
        rect: Rect::new(8.0, 8.0, width - 16.0, height - 16.0),
        color,
        radius: 6.0,
    });
    UiFrame {
        viewport: Size::new(width, height),
        damage: Vec::new(),
        paint,
        semantics: Vec::new(),
    }
}
