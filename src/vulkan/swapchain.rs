use crate::app::{HEIGHT, WIDTH};

use super::device::DeviceBundle;
use ash::{khr::swapchain, vk};

pub(crate) struct SwapchainBundle {
    pub(crate) loader: swapchain::Device,
    pub(crate) swapchain: vk::SwapchainKHR,

    // Owned by the swapchain; kept only to document what is present.
    pub(crate) images: Vec<vk::Image>,
    pub(crate) image_views: Vec<vk::ImageView>,
    pub(crate) extent: vk::Extent2D,
    /// The swapchain's color format; the graphics render pass must match it.
    pub(crate) format: vk::Format,
}

impl SwapchainBundle {
    pub(crate) unsafe fn new(context: &DeviceBundle) -> Self {
        unsafe {
            //
            // Surface capabilities
            //

            let capabilities = context
                .surface_loader
                .get_physical_device_surface_capabilities(context.physical_device, context.surface)
                .expect("surface capabilities");

            let formats = context
                .surface_loader
                .get_physical_device_surface_formats(context.physical_device, context.surface)
                .expect("surface formats");

            let surface_format = formats
                .iter()
                .copied()
                .find(|format| format.format == vk::Format::B8G8R8A8_UNORM)
                .unwrap_or(formats[0]);

            let extent = if capabilities.current_extent.width != u32::MAX {
                capabilities.current_extent
            } else {
                vk::Extent2D {
                    width: WIDTH,
                    height: HEIGHT,
                }
            };

            let present_modes = context
                .surface_loader
                .get_physical_device_surface_present_modes(context.physical_device, context.surface)
                .expect("present modes");

            let present_mode = present_modes
                .iter()
                .copied()
                .find(|&mode| mode == vk::PresentModeKHR::MAILBOX)
                .unwrap_or(vk::PresentModeKHR::FIFO);

            let image_count = capabilities.min_image_count + 1;

            let image_count = if capabilities.max_image_count > 0 {
                image_count.min(capabilities.max_image_count)
            } else {
                image_count
            };

            //
            // Swapchain
            //

            let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
                .surface(context.surface)
                .min_image_count(image_count)
                .image_format(surface_format.format)
                .image_color_space(surface_format.color_space)
                .image_extent(extent)
                .image_array_layers(1)
                // TRANSFER_DST: the compute path blits into the swapchain
                // images; COLOR_ATTACHMENT covers the graphics path.
                .image_usage(
                    vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST,
                )
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(capabilities.current_transform)
                .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(present_mode)
                .clipped(true);

            let loader = swapchain::Device::new(&context.instance, &context.device);

            let swapchain = loader
                .create_swapchain(&swapchain_create_info, None)
                .expect("failed to create swapchain");

            let images = loader
                .get_swapchain_images(swapchain)
                .expect("failed to get swapchain images");

            //
            // Image views
            //

            let image_views = images
                .iter()
                .map(|&image| {
                    let components = vk::ComponentMapping::default();

                    let subresource = vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1);

                    let info = vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(surface_format.format)
                        .components(components)
                        .subresource_range(subresource);

                    context
                        .device
                        .create_image_view(&info, None)
                        .expect("image view")
                })
                .collect::<Vec<_>>();

            Self {
                loader,
                swapchain,
                images,
                image_views,
                extent,
                format: surface_format.format,
            }
        }
    }

    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        unsafe {
            for &view in &self.image_views {
                device.destroy_image_view(view, None);
            }

            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}
