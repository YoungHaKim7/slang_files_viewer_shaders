use ash::vk;

pub(crate) struct SyncObjects {
    pub(crate) image_available: vk::Semaphore,
    // One per swapchain image: a present operation's semaphore wait is not
    // covered by the in-flight fence, so a single semaphore could be
    // signaled again while a previous present still uses it.
    pub(crate) render_finished: Vec<vk::Semaphore>,
    pub(crate) in_flight: vk::Fence,
}

impl SyncObjects {
    pub(crate) unsafe fn new(device: &ash::Device, image_count: usize) -> Self {
        unsafe {
            //
            // ------------------------------------------------------------
            // GPU Synchronization
            // ------------------------------------------------------------
            //
            // Semaphores synchronize GPU operations: one says that the
            // acquired swapchain image is ready, and the other says that
            // rendering has completed before presentation.
            //
            // The fence lets the CPU know that the previous submission has
            // completed before the single reusable command buffer and its
            // synchronization objects are reused.
            //
            let semaphore_info = vk::SemaphoreCreateInfo::default();

            let image_available = device
                .create_semaphore(&semaphore_info, None)
                .expect("semaphore");

            let render_finished = (0..image_count)
                .map(|_| {
                    device
                        .create_semaphore(&semaphore_info, None)
                        .expect("semaphore")
                })
                .collect::<Vec<_>>();

            let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

            let in_flight = device.create_fence(&fence_info, None).expect("fence");

            Self {
                image_available,
                render_finished,
                in_flight,
            }
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
            device.destroy_semaphore(self.image_available, None);

            for &semaphore in &self.render_finished {
                device.destroy_semaphore(semaphore, None);
            }

            device.destroy_fence(self.in_flight, None);
        }
    }
}
