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
