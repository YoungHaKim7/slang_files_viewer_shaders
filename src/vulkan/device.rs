use ash::{
    Entry,
    khr::{surface, swapchain},
    vk,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::ffi::CString;
use winit::window::Window;

/// Instance-level and device-level Vulkan objects: everything created
/// before the swapchain and everything that outlives it.
pub(crate) struct DeviceBundle {
    // Held to keep the Vulkan loader alive for the instance lifetime.
    #[allow(dead_code)]
    pub(crate) entry: Entry,

    pub(crate) instance: ash::Instance,

    pub(crate) surface_loader: surface::Instance,
    pub(crate) surface: vk::SurfaceKHR,

    pub(crate) physical_device: vk::PhysicalDevice,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) queue_family_index: u32,
}

impl DeviceBundle {
    pub(crate) unsafe fn new(window: &Window) -> Self {
        unsafe {
            let entry = Entry::load().expect("failed to load Vulkan");

            //
            // Instance
            //

            let app_name = CString::new("Slang Viewer").unwrap();

            let app_info = vk::ApplicationInfo::default()
                .application_name(&app_name)
                .application_version(0)
                .engine_name(&app_name)
                .engine_version(0)
                // Vulkan 1.1 for PhysicalDeviceVulkan11Features (shaderDrawParameters).
                .api_version(vk::API_VERSION_1_1);

            let display = window.display_handle().expect("display handle").as_raw();

            let extension_names = ash_window::enumerate_required_extensions(display)
                .expect("required Vulkan extensions");

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .enabled_extension_names(extension_names);

            let instance = entry
                .create_instance(&create_info, None)
                .expect("failed to create Vulkan instance");

            //
            // Surface
            //

            let surface = ash_window::create_surface(
                &entry,
                &instance,
                display,
                window.window_handle().expect("window handle").as_raw(),
                None,
            )
            .expect("failed to create surface");

            let surface_loader = surface::Instance::new(&entry, &instance);

            //
            // Physical device
            //

            let physical_devices = instance
                .enumerate_physical_devices()
                .expect("failed to enumerate physical devices");

            let (physical_device, queue_family_index) = physical_devices
                .iter()
                .find_map(|&device| {
                    let families = instance.get_physical_device_queue_family_properties(device);

                    families.iter().enumerate().find_map(|(index, family)| {
                        let graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);

                        let present = surface_loader
                            .get_physical_device_surface_support(device, index as u32, surface)
                            .ok()?;

                        if graphics && present {
                            Some((device, index as u32))
                        } else {
                            None
                        }
                    })
                })
                .expect("no suitable Vulkan device");

            //
            // Logical device
            //

            // slangc declares an (unused) BuiltIn BaseVertex input for
            // SV_VertexID, which pulls in the DrawParameters SPIR-V
            // capability. That capability is only legal when the
            // shaderDrawParameters device feature (Vulkan 1.1) is enabled.
            assert!(
                instance
                    .get_physical_device_properties(physical_device)
                    .api_version
                    >= vk::API_VERSION_1_1,
                "Vulkan 1.1 is required for shaderDrawParameters"
            );

            let mut supported_vulkan11_features = vk::PhysicalDeviceVulkan11Features::default();

            let mut supported_features2 =
                vk::PhysicalDeviceFeatures2::default().push_next(&mut supported_vulkan11_features);

            instance.get_physical_device_features2(physical_device, &mut supported_features2);

            assert!(
                supported_vulkan11_features.shader_draw_parameters == vk::TRUE,
                "shaderDrawParameters feature is not supported"
            );

            let mut enabled_features =
                vk::PhysicalDeviceVulkan11Features::default().shader_draw_parameters(true);

            let priorities = [1.0_f32];

            let queue_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family_index)
                .queue_priorities(&priorities);

            let queue_infos = [queue_info];

            let device_extensions = [swapchain::NAME.as_ptr()];

            let device_create_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_infos)
                .enabled_extension_names(&device_extensions)
                .push_next(&mut enabled_features);

            let device = instance
                .create_device(physical_device, &device_create_info, None)
                .expect("failed to create logical device");

            let queue = device.get_device_queue(queue_family_index, 0);

            Self {
                entry,
                instance,
                surface_loader,
                surface,
                physical_device,
                device,
                queue,
                queue_family_index,
            }
        }
    }

    pub(crate) unsafe fn destroy(&self) {
        unsafe {
            self.device.destroy_device(None);

            self.surface_loader.destroy_surface(self.surface, None);

            self.instance.destroy_instance(None);
        }
    }
}
