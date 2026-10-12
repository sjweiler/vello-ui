use std::{cmp::Ordering, ffi::CStr};

use ash::vk;

use crate::vulkan::VulkanError;

/// Broad hardware class reported by Vulkan for the selected adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VulkanDeviceType {
    DiscreteGpu,
    IntegratedGpu,
    VirtualGpu,
    Cpu,
    Other,
}

impl std::fmt::Display for VulkanDeviceType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::DiscreteGpu => "discrete GPU",
            Self::IntegratedGpu => "integrated GPU",
            Self::VirtualGpu => "virtual GPU",
            Self::Cpu => "CPU",
            Self::Other => "other",
        })
    }
}

/// Observable details about the adapter used by a renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VulkanDeviceInfo {
    pub enumeration_index: usize,
    pub name: String,
    pub device_type: VulkanDeviceType,
    pub vendor_id: u32,
    pub device_id: u32,
    pub api_version: u32,
    pub driver_version: u32,
    pub requested_msaa_samples: Option<u32>,
    pub msaa_samples: u32,
    pub timestamp_queries: bool,
}

impl std::fmt::Display for VulkanDeviceInfo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let samples = self.requested_msaa_samples.map_or_else(
            || format!("{}x MSAA", self.msaa_samples),
            |requested| {
                format!(
                    "{requested}x MSAA requested/{}x selected",
                    self.msaa_samples
                )
            },
        );
        write!(
            formatter,
            "{} ({}, Vulkan {}.{}.{}, driver {}, {}, timestamps {})",
            self.name,
            self.device_type,
            vk::api_version_major(self.api_version),
            vk::api_version_minor(self.api_version),
            vk::api_version_patch(self.api_version),
            self.driver_version,
            samples,
            if self.timestamp_queries {
                "supported"
            } else {
                "unavailable"
            }
        )
    }
}

pub(crate) struct SelectedDevice {
    pub physical: vk::PhysicalDevice,
    pub queue_family: u32,
    pub sample_count: vk::SampleCountFlags,
    pub timestamp_period: f32,
    pub info: VulkanDeviceInfo,
}

pub(crate) unsafe fn describe_host_device(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    queue_family: u32,
    requested_samples: Option<u32>,
) -> Result<SelectedDevice, VulkanError> {
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let queue_properties =
        unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let queue = queue_properties
        .get(queue_family as usize)
        .ok_or_else(|| VulkanError(format!("Vulkan queue family {queue_family} does not exist")))?;
    if !queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
        return Err(VulkanError(format!(
            "Vulkan queue family {queue_family} does not support graphics"
        )));
    }
    let requested_samples = requested_sample_count(requested_samples)?;
    let sample_count = choose_sample_count(&properties.limits, requested_samples);
    Ok(SelectedDevice {
        physical,
        queue_family,
        sample_count,
        timestamp_period: properties.limits.timestamp_period,
        info: VulkanDeviceInfo {
            enumeration_index: 0,
            name: device_name(&properties),
            device_type: public_device_type(properties.device_type),
            vendor_id: properties.vendor_id,
            device_id: properties.device_id,
            api_version: properties.api_version,
            driver_version: properties.driver_version,
            requested_msaa_samples: requested_samples,
            msaa_samples: sample_count.as_raw(),
            timestamp_queries: queue.timestamp_valid_bits > 0,
        },
    })
}

struct Candidate {
    physical: vk::PhysicalDevice,
    queue_family: u32,
    properties: vk::PhysicalDeviceProperties,
    enumeration_index: usize,
    timestamp_queries: bool,
}

/// Selects a compatible adapter deterministically. The optional
/// `VUI_VULKAN_DEVICE` value is either a Vulkan enumeration index or a
/// case-insensitive device-name match.
pub(crate) unsafe fn select_device(
    instance: &ash::Instance,
    requirement: &str,
    requested_samples: Option<u32>,
    mut queue_compatible: impl FnMut(vk::PhysicalDevice, u32, &vk::QueueFamilyProperties) -> bool,
) -> Result<SelectedDevice, VulkanError> {
    let physical_devices = unsafe { instance.enumerate_physical_devices()? };
    let mut candidates = Vec::new();
    let mut all_names = Vec::new();
    for (enumeration_index, physical) in physical_devices.into_iter().enumerate() {
        let properties = unsafe { instance.get_physical_device_properties(physical) };
        let name = device_name(&properties);
        all_names.push(format!("{enumeration_index}: {name}"));
        let queue_properties =
            unsafe { instance.get_physical_device_queue_family_properties(physical) };
        if let Some((queue_index, queue)) =
            queue_properties
                .iter()
                .enumerate()
                .find(|(index, properties)| {
                    u32::try_from(*index)
                        .is_ok_and(|index| queue_compatible(physical, index, properties))
                })
        {
            let queue_family = u32::try_from(queue_index)
                .map_err(|_| VulkanError("Vulkan queue-family index overflow".to_owned()))?;
            candidates.push(Candidate {
                physical,
                queue_family,
                properties,
                enumeration_index,
                timestamp_queries: queue.timestamp_valid_bits > 0,
            });
        }
    }
    candidates.sort_by(compare_candidates);
    let selected = if let Some(requested) = std::env::var_os("VUI_VULKAN_DEVICE") {
        let requested = requested.to_string_lossy();
        choose_override(&candidates, requested.trim()).map_err(|message| {
            VulkanError(format!(
                "{message}; available Vulkan devices: {}",
                if all_names.is_empty() {
                    "none".to_owned()
                } else {
                    all_names.join(", ")
                }
            ))
        })?
    } else {
        candidates.first().ok_or_else(|| {
            VulkanError(format!(
                "no Vulkan device satisfies {requirement}; detected devices: {}",
                if all_names.is_empty() {
                    "none".to_owned()
                } else {
                    all_names.join(", ")
                }
            ))
        })?
    };
    let requested_samples = requested_sample_count(requested_samples)?;
    let sample_count = choose_sample_count(&selected.properties.limits, requested_samples);
    Ok(SelectedDevice {
        physical: selected.physical,
        queue_family: selected.queue_family,
        sample_count,
        timestamp_period: selected.properties.limits.timestamp_period,
        info: VulkanDeviceInfo {
            enumeration_index: selected.enumeration_index,
            name: device_name(&selected.properties),
            device_type: public_device_type(selected.properties.device_type),
            vendor_id: selected.properties.vendor_id,
            device_id: selected.properties.device_id,
            api_version: selected.properties.api_version,
            driver_version: selected.properties.driver_version,
            requested_msaa_samples: requested_samples,
            msaa_samples: sample_count.as_raw(),
            timestamp_queries: selected.timestamp_queries,
        },
    })
}

fn compare_candidates(left: &Candidate, right: &Candidate) -> Ordering {
    device_score(right.properties.device_type)
        .cmp(&device_score(left.properties.device_type))
        .then_with(|| device_name(&left.properties).cmp(&device_name(&right.properties)))
        .then_with(|| left.enumeration_index.cmp(&right.enumeration_index))
}

fn choose_override<'a>(
    candidates: &'a [Candidate],
    requested: &str,
) -> Result<&'a Candidate, String> {
    if requested.is_empty() {
        return Err("VUI_VULKAN_DEVICE is empty".to_owned());
    }
    if let Ok(index) = requested.parse::<usize>() {
        return candidates
            .iter()
            .find(|candidate| candidate.enumeration_index == index)
            .ok_or_else(|| {
                format!("VUI_VULKAN_DEVICE index {index} is unavailable or incompatible")
            });
    }
    let requested = requested.to_lowercase();
    let exact: Vec<_> = candidates
        .iter()
        .filter(|candidate| device_name(&candidate.properties).to_lowercase() == requested)
        .collect();
    if let Some(candidate) = exact.first() {
        return Ok(candidate);
    }
    let partial: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            device_name(&candidate.properties)
                .to_lowercase()
                .contains(&requested)
        })
        .collect();
    match partial.as_slice() {
        [candidate] => Ok(candidate),
        [] => Err(format!(
            "VUI_VULKAN_DEVICE `{requested}` did not match a compatible device"
        )),
        matches => Err(format!(
            "VUI_VULKAN_DEVICE `{requested}` is ambiguous (matches {})",
            matches
                .iter()
                .map(|candidate| device_name(&candidate.properties))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn device_name(properties: &vk::PhysicalDeviceProperties) -> String {
    // SAFETY: Vulkan guarantees a nul-terminated device_name array.
    unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn device_score(device_type: vk::PhysicalDeviceType) -> u8 {
    match device_type {
        vk::PhysicalDeviceType::DISCRETE_GPU => 4,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
        vk::PhysicalDeviceType::CPU => 1,
        _ => 0,
    }
}

fn public_device_type(device_type: vk::PhysicalDeviceType) -> VulkanDeviceType {
    match device_type {
        vk::PhysicalDeviceType::DISCRETE_GPU => VulkanDeviceType::DiscreteGpu,
        vk::PhysicalDeviceType::INTEGRATED_GPU => VulkanDeviceType::IntegratedGpu,
        vk::PhysicalDeviceType::VIRTUAL_GPU => VulkanDeviceType::VirtualGpu,
        vk::PhysicalDeviceType::CPU => VulkanDeviceType::Cpu,
        _ => VulkanDeviceType::Other,
    }
}

fn requested_sample_count(explicit: Option<u32>) -> Result<Option<u32>, VulkanError> {
    let requested = explicit.or_else(|| {
        std::env::var_os("VUI_VULKAN_SAMPLES")
            .map(|value| value.to_string_lossy().parse::<u32>().unwrap_or_default())
    });
    if requested.is_some_and(|samples| !matches!(samples, 1 | 2 | 4)) {
        return Err(VulkanError(
            "VUI_VULKAN_SAMPLES must be 1, 2, or 4".to_owned(),
        ));
    }
    Ok(requested)
}

fn choose_sample_count(
    limits: &vk::PhysicalDeviceLimits,
    requested: Option<u32>,
) -> vk::SampleCountFlags {
    let supported = limits.framebuffer_color_sample_counts & limits.framebuffer_depth_sample_counts;
    let preferred = [
        vk::SampleCountFlags::TYPE_4,
        vk::SampleCountFlags::TYPE_2,
        vk::SampleCountFlags::TYPE_1,
    ];
    preferred
        .into_iter()
        .filter(|count| requested.is_none_or(|limit| count.as_raw() <= limit))
        .find(|count| supported.contains(*count))
        .unwrap_or(vk::SampleCountFlags::TYPE_1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_selection_honors_request_and_falls_back() {
        let limits = vk::PhysicalDeviceLimits {
            framebuffer_color_sample_counts: vk::SampleCountFlags::TYPE_1
                | vk::SampleCountFlags::TYPE_2,
            framebuffer_depth_sample_counts: vk::SampleCountFlags::TYPE_1
                | vk::SampleCountFlags::TYPE_2,
            ..vk::PhysicalDeviceLimits::default()
        };
        assert_eq!(
            choose_sample_count(&limits, None),
            vk::SampleCountFlags::TYPE_2
        );
        assert_eq!(
            choose_sample_count(&limits, Some(4)),
            vk::SampleCountFlags::TYPE_2
        );
        assert_eq!(
            choose_sample_count(&limits, Some(1)),
            vk::SampleCountFlags::TYPE_1
        );
    }

    #[test]
    fn explicit_sample_selection_is_validated() {
        assert_eq!(requested_sample_count(Some(2)).unwrap(), Some(2));
        assert!(requested_sample_count(Some(8)).is_err());
    }
}
