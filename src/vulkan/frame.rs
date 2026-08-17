use super::VulkanApp;
use ash::vk;

impl VulkanApp {
    //
    // Recorded fresh every frame for the swapchain image that was just
    // acquired. The swapchain cycles through several images; recording
    // once against a single framebuffer would present unrendered images
    // and make the triangle blink.
    //

    pub(crate) unsafe fn record_command_buffer(&self, image_index: u32) {
        unsafe {
            self.context
                .device
                .begin_command_buffer(self.commands.buffer, &vk::CommandBufferBeginInfo::default())
                .expect("begin command buffer");

            self.pipeline.record(
                &self.context.device,
                self.commands.buffer,
                &self.swapchain,
                image_index,
            );

            self.context
                .device
                .end_command_buffer(self.commands.buffer)
                .expect("end command buffer");
        }
    }
}
