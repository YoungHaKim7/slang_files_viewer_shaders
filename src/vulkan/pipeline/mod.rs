mod compute;
mod graphics;

use super::{device::DeviceBundle, swapchain::SwapchainBundle};
use crate::shader::RenderMode;
use ash::{Device, vk};

/// Mode-specific Vulkan objects created around the compiled shader module.
pub(crate) enum Pipeline {
    /// Classic vertex + fragment rendering through a render pass.
    Graphics(graphics::Graphics),
    /// Playground-style compute pass into an offscreen image that is
    /// blitted to the swapchain.
    Compute(compute::Compute),
}

impl Pipeline {
    pub(crate) unsafe fn new(
        context: &DeviceBundle,
        swapchain: &SwapchainBundle,
        shader_module: vk::ShaderModule,
        mode: &RenderMode,
    ) -> Self {
        unsafe {
            match mode {
                RenderMode::Graphics {
                    vertex_entry,
                    fragment_entry,
                } => Self::Graphics(graphics::Graphics::new(
                    context,
                    swapchain,
                    shader_module,
                    vertex_entry,
                    fragment_entry,
                )),

                RenderMode::Compute {
                    entry,
                    group_size,
                    parameters,
                } => Self::Compute(compute::Compute::new(
                    context,
                    swapchain,
                    shader_module,
                    entry,
                    group_size,
                    parameters,
                )),
            }
        }
    }

    /// Appends this pipeline's commands to the command buffer.
    pub(crate) unsafe fn record(
        &self,
        device: &Device,
        command_buffer: vk::CommandBuffer,
        swapchain: &SwapchainBundle,
        image_index: u32,
    ) {
        unsafe {
            match self {
                Self::Graphics(graphics) => {
                    graphics.record(device, command_buffer, swapchain, image_index)
                }
                Self::Compute(compute) => {
                    compute.record(device, command_buffer, swapchain, image_index)
                }
            }
        }
    }

    pub(crate) unsafe fn destroy(&self, device: &Device) {
        unsafe {
            match self {
                Self::Graphics(graphics) => graphics.destroy(device),
                Self::Compute(compute) => compute.destroy(device),
            }
        }
    }

    /// Stage at which the draw submission waits for the acquired image.
    /// Graphics waits before the render pass touches the color attachment;
    /// compute only needs the image by the blit.
    pub(crate) fn wait_stage(&self) -> vk::PipelineStageFlags {
        match self {
            Self::Graphics(_) => vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            Self::Compute(_) => vk::PipelineStageFlags::TRANSFER,
        }
    }
}
