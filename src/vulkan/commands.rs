use super::device::DeviceBundle;
use ash::vk;

pub(crate) struct Commands {
    pub(crate) pool: vk::CommandPool,
    pub(crate) buffer: vk::CommandBuffer,
}

impl Commands {
    pub(crate) unsafe fn new(context: &DeviceBundle) -> Self {
        unsafe {
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

    pub(crate) unsafe fn destroy(&self, device: &ash::Device) {
        unsafe {
            device.destroy_command_pool(self.pool, None);
        }
    }
}
