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

/// Owns the Vulkan objects required to render one triangle.
///
/// # Vulkan object hierarchy
///
/// The important lifetime relationships are roughly:
///
/// `Entry -> Instance -> PhysicalDevice -> Device -> Queue`
///
/// and, for presentation:
///
/// `Instance -> Surface -> Swapchain -> Images -> ImageViews -> Framebuffers`
///
/// The graphics pipeline depends on the render pass, while command buffers
/// refer to the pipeline and the framebuffer selected for the acquired
/// swapchain image.
///
/// Vulkan handles are generally lightweight, non-owning values. The Rust
/// struct therefore acts as the owner of the corresponding Vulkan resources,
/// and `destroy()` releases them in dependency-safe reverse order.
///
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
    /// Creates all Vulkan state needed by the triangle renderer.
    ///
    /// Vulkan exposes a relatively explicit initialization model. In broad
    /// terms this function performs these steps:
    ///
    /// 1. Load the Vulkan loader (`Entry`).
    /// 2. Create a Vulkan `Instance`.
    /// 3. Create a window `Surface` that Vulkan can present to.
    /// 4. Select a physical GPU and a queue family supporting graphics and presentation.
    /// 5. Create a logical `Device` and obtain a graphics queue.
    /// 6. Query surface capabilities and create a `Swapchain`.
    /// 7. Create image views for the swapchain images.
    /// 8. Create a render pass describing the color attachment.
    /// 9. Load Slang-generated SPIR-V and create shader modules.
    /// 10. Build the graphics pipeline.
    /// 11. Create framebuffers, command infrastructure, and synchronization.
    ///
    /// Most Vulkan functions are `unsafe` here because Vulkan's C API cannot
    /// express resource validity, synchronization, or lifetime dependencies
    /// in its type system. The surrounding Rust code establishes those
    /// invariants manually.
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

    /// Executes one complete frame.
    ///
    /// The CPU/GPU sequence is:
    ///
    /// 1. Wait until the previous use of our reusable command buffer is done.
    /// 2. Acquire a swapchain image.
    /// 3. Record commands targeting that image's framebuffer.
    /// 4. Submit those commands to the graphics queue.
    /// 5. Present the same swapchain image after rendering finishes.
    ///
    /// The semaphores establish GPU-to-GPU ordering; the fence establishes
    /// CPU-to-GPU reuse ordering.
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

            // The semaphore wait is consumed before the color-attachment
            // output stage. In other words, the GPU must not start writing
            // the acquired swapchain image until image acquisition signals
            // `image_available`.
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
