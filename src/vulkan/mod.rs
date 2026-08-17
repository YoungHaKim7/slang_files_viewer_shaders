mod commands;
mod destroy;
mod device;
mod frame;
mod pipeline;
mod swapchain;
mod sync;

use ash::vk;
use commands::Commands;
use device::DeviceBundle;
use pipeline::Pipeline;
use swapchain::SwapchainBundle;
use sync::SyncObjects;
use winit::window::Window;

use crate::shader::CompiledShader;

/// Everything the viewer needs to present frames: one bundle per concern,
/// torn down in reverse creation order by destroy().
pub(crate) struct VulkanApp {
    context: DeviceBundle,
    swapchain: SwapchainBundle,
    pipeline: Pipeline,
    commands: Commands,
    sync: SyncObjects,
}

impl VulkanApp {
    pub(crate) unsafe fn new(window: &Window, compiled: &CompiledShader) -> Self {
        unsafe {
            let context = DeviceBundle::new(window);

            let swapchain = SwapchainBundle::new(&context);

            //
            // Pipeline for the compiled shader
            //

            let module_info = vk::ShaderModuleCreateInfo::default().code(&compiled.spirv);

            let shader_module = context
                .device
                .create_shader_module(&module_info, None)
                .expect("shader module");

            let pipeline = Pipeline::new(&context, &swapchain, shader_module, &compiled.mode);

            // Pipelines capture the entry point names; the module is no
            // longer needed.
            context.device.destroy_shader_module(shader_module, None);

            let commands = Commands::new(&context);

            let sync = SyncObjects::new(&context.device, swapchain.images.len());

            Self {
                context,
                swapchain,
                pipeline,
                commands,
                sync,
            }
        }
    }

    pub(crate) unsafe fn draw(&self) {
        unsafe {
            self.context
                .device
                .wait_for_fences(&[self.sync.in_flight], true, u64::MAX)
                .expect("wait fence");

            self.context
                .device
                .reset_fences(&[self.sync.in_flight])
                .expect("reset fence");

            let (image_index, _) = self
                .swapchain
                .loader
                .acquire_next_image(
                    self.swapchain.swapchain,
                    u64::MAX,
                    self.sync.image_available,
                    vk::Fence::null(),
                )
                .expect("acquire image");

            self.record_command_buffer(image_index);

            let wait_semaphores = [self.sync.image_available];

            let signal_semaphores = [self.sync.render_finished[image_index as usize]];

            let wait_stages = [self.pipeline.wait_stage()];

            let command_buffers = [self.commands.buffer];

            let submit_info = vk::SubmitInfo::default()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);

            self.context
                .device
                .queue_submit(self.context.queue, &[submit_info], self.sync.in_flight)
                .expect("queue submit");

            let swapchains = [self.swapchain.swapchain];

            let image_indices = [image_index];

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&signal_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            self.swapchain
                .loader
                .queue_present(self.context.queue, &present_info)
                .expect("queue present");
        }
    }
}
