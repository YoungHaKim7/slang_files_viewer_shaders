use super::device::DeviceBundle;
use ash::vk;

pub(crate) struct Commands {
    pub(crate) pool: vk::CommandPool,
    pub(crate) buffer: vk::CommandBuffer,
}

impl Commands {
    pub(crate) unsafe fn new(context: &DeviceBundle) -> Self {
        unsafe {
            //
            // ------------------------------------------------------------
            // Command Pool and Command Buffer
            // ------------------------------------------------------------
            //
            // Vulkan rendering is normally submitted through recorded command
            // buffers. The command pool controls allocation/reset of command
            // buffers for a particular queue family.
            //

            // RESET_COMMAND_BUFFER lets draw() reset and re-record the
            // command buffer every frame for the acquired swapchain image.
            let command_pool_info = vk::CommandPoolCreateInfo::default()
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                .queue_family_index(context.queue_family_index);

            let pool = context
                .device
                .create_command_pool(&command_pool_info, None)
                .expect("command pool");

            let command_buffer_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);

            let buffer = context
                .device
                .allocate_command_buffers(&command_buffer_info)
                .expect("command buffer")[0];

            Self { pool, buffer }
        }
    }

    /// Releases Vulkan resources in dependency-safe reverse order.
    ///
    /// Vulkan does not automatically destroy handles merely because a Rust
    /// variable goes out of scope. Every created Vulkan object must be
    /// explicitly destroyed (or wrapped in an RAII abstraction that performs
    /// the same operation).
    ///
    /// Destruction must respect dependencies. For example, framebuffers use
    /// image views and a render pass, so they are destroyed before those
    /// objects. The device is destroyed only after device-owned resources are
    /// gone, and the instance is destroyed last.
    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_command_pool(self.pool, None);
        }
    }
}
