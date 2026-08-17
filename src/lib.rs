use ash::{
    Entry,
    khr::{surface, swapchain},
    vk,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::{ffi::CString, mem::size_of};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

struct VulkanApp {
    // Held to keep the Vulkan loader alive for the instance lifetime.
    #[allow(dead_code)]
    entry: Entry,
    instance: ash::Instance,

    surface_loader: surface::Instance,
    surface: vk::SurfaceKHR,

    #[allow(dead_code)]
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,

    swapchain_loader: swapchain::Device,
    swapchain: vk::SwapchainKHR,

    // Owned by the swapchain; kept only to document what is present.
    #[allow(dead_code)]
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_extent: vk::Extent2D,

    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,

    framebuffers: Vec<vk::Framebuffer>,

    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,

    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
}

impl VulkanApp {
    unsafe fn new(window: &Window) -> Self {
        unsafe {
            let entry = Entry::load().expect("failed to load Vulkan");

            //
            // Instance
            //

            let app_name = CString::new("Slang Triangle").unwrap();

            let app_info = vk::ApplicationInfo::default()
                .application_name(&app_name)
                .application_version(0)
                .engine_name(&app_name)
                .engine_version(0)
                // Vulkan 1.1 for PhysicalDeviceVulkan11Features (shaderDrawParameters).
                .api_version(vk::API_VERSION_1_1);

            let display = window.display_handle().expect("display handle").as_raw();

            let extension_names = ash_window::enumerate_required_extensions(display)
                .expect("required Vulkan extensions");

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .enabled_extension_names(extension_names);

            let instance = entry
                .create_instance(&create_info, None)
                .expect("failed to create Vulkan instance");

            //
            // Surface
            //

            let surface = ash_window::create_surface(
                &entry,
                &instance,
                display,
                window.window_handle().expect("window handle").as_raw(),
                None,
            )
            .expect("failed to create surface");

            let surface_loader = surface::Instance::new(&entry, &instance);

            //
            // Physical device
            //

            let physical_devices = instance
                .enumerate_physical_devices()
                .expect("failed to enumerate physical devices");

            let (physical_device, queue_family_index) = physical_devices
                .iter()
                .find_map(|&device| {
                    let families = instance.get_physical_device_queue_family_properties(device);

                    families.iter().enumerate().find_map(|(index, family)| {
                        let graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);

                        let present = surface_loader
                            .get_physical_device_surface_support(device, index as u32, surface)
                            .ok()?;

                        if graphics && present {
                            Some((device, index as u32))
                        } else {
                            None
                        }
                    })
                })
                .expect("no suitable Vulkan device");

            //
            // Logical device
            //

            // slangc declares an (unused) BuiltIn BaseVertex input for
            // SV_VertexID, which pulls in the DrawParameters SPIR-V
            // capability. That capability is only legal when the
            // shaderDrawParameters device feature (Vulkan 1.1) is enabled.
            assert!(
                instance
                    .get_physical_device_properties(physical_device)
                    .api_version
                    >= vk::API_VERSION_1_1,
                "Vulkan 1.1 is required for shaderDrawParameters"
            );

            let mut supported_vulkan11_features = vk::PhysicalDeviceVulkan11Features::default();

            let mut supported_features2 =
                vk::PhysicalDeviceFeatures2::default().push_next(&mut supported_vulkan11_features);

            instance.get_physical_device_features2(physical_device, &mut supported_features2);

            assert!(
                supported_vulkan11_features.shader_draw_parameters == vk::TRUE,
                "shaderDrawParameters feature is not supported"
            );

            let mut enabled_features =
                vk::PhysicalDeviceVulkan11Features::default().shader_draw_parameters(true);

            let priorities = [1.0_f32];

            let queue_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family_index)
                .queue_priorities(&priorities);

            let queue_infos = [queue_info];

            let device_extensions = [swapchain::NAME.as_ptr()];

            let device_create_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_infos)
                .enabled_extension_names(&device_extensions)
                .push_next(&mut enabled_features);

            let device = instance
                .create_device(physical_device, &device_create_info, None)
                .expect("failed to create logical device");

            let queue = device.get_device_queue(queue_family_index, 0);

            //
            // Surface capabilities
            //

            let capabilities = surface_loader
                .get_physical_device_surface_capabilities(physical_device, surface)
                .expect("surface capabilities");

            let formats = surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
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

            let present_modes = surface_loader
                .get_physical_device_surface_present_modes(physical_device, surface)
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
                .surface(surface)
                .min_image_count(image_count)
                .image_format(surface_format.format)
                .image_color_space(surface_format.color_space)
                .image_extent(extent)
                .image_array_layers(1)
                .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(capabilities.current_transform)
                .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(present_mode)
                .clipped(true);

            let swapchain_loader = swapchain::Device::new(&instance, &device);

            let swapchain = swapchain_loader
                .create_swapchain(&swapchain_create_info, None)
                .expect("failed to create swapchain");

            let swapchain_images = swapchain_loader
                .get_swapchain_images(swapchain)
                .expect("failed to get swapchain images");

            //
            // Image views
            //

            let swapchain_image_views = swapchain_images
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

                    device.create_image_view(&info, None).expect("image view")
                })
                .collect::<Vec<_>>();

            //
            // Render pass
            //

            let color_attachment = vk::AttachmentDescription::default()
                .format(surface_format.format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

            let color_ref = vk::AttachmentReference::default()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

            let color_refs = [color_ref];

            let subpass = vk::SubpassDescription::default()
                .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
                .color_attachments(&color_refs);

            let attachments = [color_attachment];
            let subpasses = [subpass];

            let render_pass_info = vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses);

            let render_pass = device
                .create_render_pass(&render_pass_info, None)
                .expect("render pass");

            //
            // Load Slang-generated SPIR-V
            //

            let vertex_code = include_bytes!(concat!(env!("OUT_DIR"), "/triangle.vert.spv"));

            let fragment_code = include_bytes!(concat!(env!("OUT_DIR"), "/triangle.frag.spv"));

            let vertex_words = std::slice::from_raw_parts(
                vertex_code.as_ptr() as *const u32,
                vertex_code.len() / size_of::<u32>(),
            );

            let fragment_words = std::slice::from_raw_parts(
                fragment_code.as_ptr() as *const u32,
                fragment_code.len() / size_of::<u32>(),
            );

            let vertex_module_info = vk::ShaderModuleCreateInfo::default().code(vertex_words);

            let fragment_module_info = vk::ShaderModuleCreateInfo::default().code(fragment_words);

            let vertex_module = device
                .create_shader_module(&vertex_module_info, None)
                .expect("vertex shader module");

            let fragment_module = device
                .create_shader_module(&fragment_module_info, None)
                .expect("fragment shader module");

            //
            // Pipeline
            //

            let main_name = CString::new("vertMain").unwrap();

            let vertex_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_module)
                .name(&main_name);

            let main_name_frag = CString::new("fragMain").unwrap();

            let fragment_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_module)
                .name(&main_name_frag);

            let stages = [vertex_stage, fragment_stage];

            //
            // IMPORTANT:
            //
            // There are NO vertex attributes.
            //
            // SV_VertexID supplies the vertex number.
            //

            let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();

            let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .primitive_restart_enable(false);

            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };

            let scissors = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            };

            let viewports = [viewport];
            let scissors_array = [scissors];

            let viewport_state = vk::PipelineViewportStateCreateInfo::default()
                .viewports(&viewports)
                .scissors(&scissors_array);

            let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(vk::PolygonMode::FILL)
                .line_width(1.0)
                .cull_mode(vk::CullModeFlags::NONE)
                .front_face(vk::FrontFace::COUNTER_CLOCKWISE);

            let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1);

            let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false);

            let color_blend_attachments = [color_blend_attachment];

            let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
                .logic_op_enable(false)
                .attachments(&color_blend_attachments);

            let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default();

            let pipeline_layout = device
                .create_pipeline_layout(&pipeline_layout_info, None)
                .expect("pipeline layout");

            let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&stages)
                .vertex_input_state(&vertex_input)
                .input_assembly_state(&input_assembly)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterizer)
                .multisample_state(&multisampling)
                .color_blend_state(&color_blending)
                .layout(pipeline_layout)
                .render_pass(render_pass)
                .subpass(0);

            let graphics_pipeline = device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
                .expect("graphics pipeline")[0];

            device.destroy_shader_module(vertex_module, None);

            device.destroy_shader_module(fragment_module, None);

            //
            // Framebuffers
            //

            let framebuffers = swapchain_image_views
                .iter()
                .map(|&view| {
                    let attachments = [view];

                    let info = vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&attachments)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1);

                    device.create_framebuffer(&info, None).expect("framebuffer")
                })
                .collect::<Vec<_>>();

            //
            // Command pool
            //

            // RESET_COMMAND_BUFFER lets draw() reset and re-record the
            // command buffer every frame for the acquired swapchain image.
            let command_pool_info = vk::CommandPoolCreateInfo::default()
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                .queue_family_index(queue_family_index);

            let command_pool = device
                .create_command_pool(&command_pool_info, None)
                .expect("command pool");

            let command_buffer_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);

            let command_buffer = device
                .allocate_command_buffers(&command_buffer_info)
                .expect("command buffer")[0];

            //
            // Synchronization
            //

            let semaphore_info = vk::SemaphoreCreateInfo::default();

            let image_available = device
                .create_semaphore(&semaphore_info, None)
                .expect("semaphore");

            let render_finished = device
                .create_semaphore(&semaphore_info, None)
                .expect("semaphore");

            let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

            let in_flight = device.create_fence(&fence_info, None).expect("fence");

            Self {
                entry,
                instance,
                surface_loader,
                surface,
                physical_device,
                device,
                queue,
                swapchain_loader,
                swapchain,
                swapchain_images,
                swapchain_image_views,
                swapchain_extent: extent,
                render_pass,
                pipeline_layout,
                graphics_pipeline,
                framebuffers,
                command_pool,
                command_buffer,
                image_available,
                render_finished,
                in_flight,
            }
        }
    }

    //
    // Recorded fresh every frame for the swapchain image that was just
    // acquired. The swapchain cycles through several images; recording
    // once against a single framebuffer would present unrendered images
    // and make the triangle blink.
    //

    unsafe fn record_command_buffer(&self, image_index: u32) {
        unsafe {
            self.device
                .begin_command_buffer(self.command_buffer, &vk::CommandBufferBeginInfo::default())
                .expect("begin command buffer");

            let clear_value = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.05, 0.05, 0.05, 1.0],
                },
            };

            let clear_values = [clear_value];

            let render_begin = vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(self.framebuffers[image_index as usize])
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: self.swapchain_extent,
                })
                .clear_values(&clear_values);

            self.device.cmd_begin_render_pass(
                self.command_buffer,
                &render_begin,
                vk::SubpassContents::INLINE,
            );

            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.graphics_pipeline,
            );

            //
            // HERE!
            //
            // No vertex buffer.
            //
            // Draw 3 vertices.
            //

            self.device.cmd_draw(self.command_buffer, 3, 1, 0, 0);

            self.device.cmd_end_render_pass(self.command_buffer);

            self.device
                .end_command_buffer(self.command_buffer)
                .expect("end command buffer");
        }
    }

    unsafe fn draw(&self) {
        unsafe {
            self.device
                .wait_for_fences(&[self.in_flight], true, u64::MAX)
                .expect("wait fence");

            self.device
                .reset_fences(&[self.in_flight])
                .expect("reset fence");

            let (image_index, _) = self
                .swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    self.image_available,
                    vk::Fence::null(),
                )
                .expect("acquire image");

            self.record_command_buffer(image_index);

            let wait_semaphores = [self.image_available];

            let signal_semaphores = [self.render_finished];

            let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

            let command_buffers = [self.command_buffer];

            let submit_info = vk::SubmitInfo::default()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);

            self.device
                .queue_submit(self.queue, &[submit_info], self.in_flight)
                .expect("queue submit");

            let swapchains = [self.swapchain];

            let image_indices = [image_index];

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&signal_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            self.swapchain_loader
                .queue_present(self.queue, &present_info)
                .expect("queue present");
        }
    }

    unsafe fn destroy(&self) {
        unsafe {
            self.device.device_wait_idle().unwrap();

            self.device.destroy_semaphore(self.image_available, None);

            self.device.destroy_semaphore(self.render_finished, None);

            self.device.destroy_fence(self.in_flight, None);

            self.device.destroy_command_pool(self.command_pool, None);

            for &framebuffer in &self.framebuffers {
                self.device.destroy_framebuffer(framebuffer, None);
            }

            self.device.destroy_pipeline(self.graphics_pipeline, None);

            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);

            self.device.destroy_render_pass(self.render_pass, None);

            for &view in &self.swapchain_image_views {
                self.device.destroy_image_view(view, None);
            }

            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);

            self.device.destroy_device(None);

            self.surface_loader.destroy_surface(self.surface, None);

            self.instance.destroy_instance(None);
        }
    }
}

struct App {
    window: Option<Window>,
    vulkan: Option<VulkanApp>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = WindowAttributes::default()
            .with_title("Rust + Slang + Vulkan")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT));

        let window = event_loop.create_window(attributes).expect("window");

        let vulkan = unsafe { VulkanApp::new(&window) };

        self.window = Some(window);
        self.vulkan = Some(vulkan);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(vulkan) = &self.vulkan {
                    unsafe {
                        vulkan.destroy();
                    }
                }

                event_loop.exit();
            }

            WindowEvent::RedrawRequested => {
                if let Some(vulkan) = &self.vulkan {
                    unsafe {
                        vulkan.draw();
                    }
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Creates the event loop and runs the application until the window closes.
pub fn run() {
    let event_loop = EventLoop::new().expect("event loop");

    let mut app = App {
        window: None,
        vulkan: None,
    };

    event_loop.run_app(&mut app).expect("event loop error");
}
