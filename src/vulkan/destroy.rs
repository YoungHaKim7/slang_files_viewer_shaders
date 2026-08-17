use super::VulkanApp;

impl VulkanApp {
    /// Tears the bundles down in reverse creation order; each bundle
    /// destroys its own objects.
    pub(crate) unsafe fn destroy(&self) {
        unsafe {
            self.context.device.device_wait_idle().unwrap();

            self.sync.destroy(&self.context.device);

            self.commands.destroy(&self.context.device);

            self.pipeline.destroy(&self.context.device);

            self.swapchain.destroy(&self.context.device);

            self.context.destroy();
        }
    }
}
