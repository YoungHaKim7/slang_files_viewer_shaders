pub mod shader;

use ash::{
    Entry,
    khr::{surface, swapchain},
    vk,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use shader::{CompiledShader, ParamKind, RenderMode};
use std::{
    ffi::CString,
    time::{SystemTime, UNIX_EPOCH},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// Mode-specific Vulkan objects created around the compiled shader module.
enum Pipeline {
    /// Classic vertex + fragment rendering through a render pass.
    Graphics {
        render_pass: vk::RenderPass,
        pipeline_layout: vk::PipelineLayout,
        graphics_pipeline: vk::Pipeline,
        framebuffers: Vec<vk::Framebuffer>,
    },
    /// Playground-style compute pass into an offscreen image that is
    /// blitted to the swapchain.
    Compute {
        pipeline_layout: vk::PipelineLayout,
        compute_pipeline: vk::Pipeline,

        descriptor_pool: vk::DescriptorPool,
        descriptor_set_layout: vk::DescriptorSetLayout,
        descriptor_set: vk::DescriptorSet,

        image: vk::Image,
        image_memory: vk::DeviceMemory,
        image_view: vk::ImageView,

        /// The shader's random-float buffer, when it declares one.
        rand_buffer: Option<(vk::Buffer, vk::DeviceMemory)>,

        /// Work groups to dispatch; derived from threadGroupSize and the
        /// image extent.
        group_count: [u32; 3],
    },
}

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

    pipeline: Pipeline,

    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,

    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
}

impl VulkanApp {
    unsafe fn new(window: &Window, compiled: &CompiledShader) -> Self {
        unsafe {
            let entry = Entry::load().expect("failed to load Vulkan");

            //
            // Instance
            //

            let app_name = CString::new("Slang Viewer").unwrap();

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
            // Pipeline for the compiled shader
            //

            let module_info = vk::ShaderModuleCreateInfo::default().code(&compiled.spirv);

            let shader_module = device
                .create_shader_module(&module_info, None)
                .expect("shader module");

            let pipeline = match &compiled.mode {
                RenderMode::Graphics {
                    vertex_entry,
                    fragment_entry,
                } => Pipeline::make_graphics(
                    &device,
                    surface_format.format,
                    &swapchain_image_views,
                    extent,
                    shader_module,
                    vertex_entry,
                    fragment_entry,
                ),

                RenderMode::Compute {
                    entry,
                    group_size,
                    parameters,
                } => Pipeline::make_compute(
                    &instance,
                    physical_device,
                    &device,
                    extent,
                    shader_module,
                    entry,
                    group_size,
                    parameters,
                ),
            };

            // Pipelines capture the entry point names; the module is no
            // longer needed.
            device.destroy_shader_module(shader_module, None);

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
                pipeline,
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

            match &self.pipeline {
                Pipeline::Graphics {
                    render_pass,
                    graphics_pipeline,
                    framebuffers,
                    ..
                } => {
                    let clear_value = vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: [0.05, 0.05, 0.05, 1.0],
                        },
                    };

                    let clear_values = [clear_value];

                    let render_begin = vk::RenderPassBeginInfo::default()
                        .render_pass(*render_pass)
                        .framebuffer(framebuffers[image_index as usize])
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
                        *graphics_pipeline,
                    );

                    // No vertex buffer: SV_VertexID supplies the corner.
                    self.device.cmd_draw(self.command_buffer, 3, 1, 0, 0);

                    self.device.cmd_end_render_pass(self.command_buffer);
                }

                Pipeline::Compute {
                    pipeline_layout,
                    compute_pipeline,
                    descriptor_set,
                    image,
                    group_count,
                    ..
                } => {
                    let extent = self.swapchain_extent;

                    let subresource = || {
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1)
                    };

                    //
                    // Offscreen image: undefined -> general (compute write)
                    //

                    let to_general = vk::ImageMemoryBarrier::default()
                        .image(*image)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        )
                        .src_access_mask(vk::AccessFlags::empty())
                        .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::GENERAL);

                    self.device.cmd_pipeline_barrier(
                        self.command_buffer,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[to_general],
                    );

                    //
                    // Dispatch the kernel over the whole image
                    //

                    self.device.cmd_bind_pipeline(
                        self.command_buffer,
                        vk::PipelineBindPoint::COMPUTE,
                        *compute_pipeline,
                    );

                    self.device.cmd_bind_descriptor_sets(
                        self.command_buffer,
                        vk::PipelineBindPoint::COMPUTE,
                        *pipeline_layout,
                        0,
                        &[*descriptor_set],
                        &[],
                    );

                    self.device.cmd_dispatch(
                        self.command_buffer,
                        group_count[0],
                        group_count[1],
                        group_count[2],
                    );

                    //
                    // Offscreen: general -> transfer source
                    //

                    let to_transfer_src = vk::ImageMemoryBarrier::default()
                        .image(*image)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        )
                        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);

                    //
                    // Swapchain image: undefined -> transfer destination
                    //

                    let to_transfer_dst = vk::ImageMemoryBarrier::default()
                        .image(self.swapchain_images[image_index as usize])
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        )
                        .src_access_mask(vk::AccessFlags::empty())
                        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .old_layout(vk::ImageLayout::UNDEFINED)
                        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL);

                    self.device.cmd_pipeline_barrier(
                        self.command_buffer,
                        vk::PipelineStageFlags::COMPUTE_SHADER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[to_transfer_src, to_transfer_dst],
                    );

                    //
                    // Blit handles the format conversion between the
                    // shader's rgba8 image and the swapchain format.
                    //

                    let blit = vk::ImageBlit2::default()
                        .src_subresource(subresource())
                        .src_offsets([
                            vk::Offset3D { x: 0, y: 0, z: 0 },
                            vk::Offset3D {
                                x: extent.width as i32,
                                y: extent.height as i32,
                                z: 1,
                            },
                        ])
                        .dst_subresource(subresource())
                        .dst_offsets([
                            vk::Offset3D { x: 0, y: 0, z: 0 },
                            vk::Offset3D {
                                x: extent.width as i32,
                                y: extent.height as i32,
                                z: 1,
                            },
                        ]);

                    let blit_regions = [blit];

                    let blit_info = vk::BlitImageInfo2::default()
                        .src_image(*image)
                        .src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .dst_image(self.swapchain_images[image_index as usize])
                        .dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .regions(&blit_regions)
                        .filter(vk::Filter::LINEAR);

                    self.device.cmd_blit_image2(self.command_buffer, &blit_info);

                    //
                    // Swapchain image: transfer destination -> present
                    //

                    let to_present = vk::ImageMemoryBarrier::default()
                        .image(self.swapchain_images[image_index as usize])
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        )
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::empty())
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR);

                    self.device.cmd_pipeline_barrier(
                        self.command_buffer,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[to_present],
                    );
                }
            }

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

            // Graphics waits before the render pass touches the color
            // attachment; compute only needs the image by the blit.
            let wait_stage = match &self.pipeline {
                Pipeline::Graphics { .. } => vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                Pipeline::Compute { .. } => vk::PipelineStageFlags::TRANSFER,
            };

            let wait_stages = [wait_stage];

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

            match &self.pipeline {
                Pipeline::Graphics {
                    render_pass,
                    pipeline_layout,
                    graphics_pipeline,
                    framebuffers,
                } => {
                    for &framebuffer in framebuffers {
                        self.device.destroy_framebuffer(framebuffer, None);
                    }

                    self.device.destroy_pipeline(*graphics_pipeline, None);

                    self.device.destroy_pipeline_layout(*pipeline_layout, None);

                    self.device.destroy_render_pass(*render_pass, None);
                }

                Pipeline::Compute {
                    pipeline_layout,
                    compute_pipeline,
                    descriptor_pool,
                    descriptor_set_layout,
                    image,
                    image_memory,
                    image_view,
                    rand_buffer,
                    ..
                } => {
                    self.device.destroy_pipeline(*compute_pipeline, None);

                    self.device.destroy_pipeline_layout(*pipeline_layout, None);

                    self.device
                        .destroy_descriptor_pool(*descriptor_pool, None);

                    self.device
                        .destroy_descriptor_set_layout(*descriptor_set_layout, None);

                    self.device.destroy_image_view(*image_view, None);

                    self.device.destroy_image(*image, None);

                    self.device.free_memory(*image_memory, None);

                    if let Some((buffer, memory)) = rand_buffer {
                        self.device.destroy_buffer(*buffer, None);

                        self.device.free_memory(*memory, None);
                    }
                }
            }

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

impl Pipeline {
    //
    // Graphics pipeline: render pass + framebuffers + the vertex/fragment
    // stages, matching the previous build-time triangle setup.
    //

    unsafe fn make_graphics(
        device: &ash::Device,
        surface_format: vk::Format,
        swapchain_image_views: &[vk::ImageView],
        extent: vk::Extent2D,
        shader_module: vk::ShaderModule,
        vertex_entry: &str,
        fragment_entry: &str,
    ) -> Self {
        unsafe {

            //
            // Render pass
            //

            let color_attachment = vk::AttachmentDescription::default()
                .format(surface_format)
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
            // Pipeline
            //

            let vertex_name = CString::new(vertex_entry).unwrap();

            let vertex_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(shader_module)
                .name(&vertex_name);

            let fragment_name = CString::new(fragment_entry).unwrap();

            let fragment_stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(shader_module)
                .name(&fragment_name);

            let stages = [vertex_stage, fragment_stage];

            // There are NO vertex attributes: SV_VertexID supplies the
            // vertex number.
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

            Pipeline::Graphics {
                render_pass,
                pipeline_layout,
                graphics_pipeline,
                framebuffers,
            }
        }
    }

    //
    // Compute pipeline: offscreen storage image + random buffer + the
    // descriptor set the kernel's parameters bind to.
    //

    unsafe fn make_compute(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        extent: vk::Extent2D,
        shader_module: vk::ShaderModule,
        entry: &str,
        group_size: &[u32; 3],
        parameters: &[shader::ShaderParam],
    ) -> Self {
        unsafe {

            //
            // Offscreen image the kernel writes to. rgba8 matches the
            // [format("rgba8")] on the playground's outputTexture; the
            // blit to the swapchain handles any format difference.
            //

            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::R8G8B8A8_UNORM)
                .extent(vk::Extent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);

            let image = device.create_image(&image_info, None).expect("storage image");

            let memory_requirements = device.get_image_memory_requirements(image);

            let image_memory = device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(memory_requirements.size)
                        .memory_type_index(find_memory_type(
                            instance,
                            physical_device,
                            memory_requirements.memory_type_bits,
                            vk::MemoryPropertyFlags::DEVICE_LOCAL,
                        )),
                    None,
                )
                .expect("allocate image memory");

            device.bind_image_memory(image, image_memory, 0).expect("bind image memory");

            let image_view = device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::R8G8B8A8_UNORM)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        ),
                    None,
                )
                .expect("storage image view");

            //
            // Random-float buffer, when the kernel declares one. Uploaded
            // through host-visible memory; a viewer does not need a
            // staging pass.
            //

            let rand_param = parameters
                .iter()
                .find(|param| matches!(param.kind, ParamKind::RandomFloatBuffer));

            let rand_buffer = rand_param.map(|param| {
                let count = param
                    .rand_count
                    .unwrap_or(shader::DEFAULT_RAND_COUNT) as usize;

                let buffer_info = vk::BufferCreateInfo::default()
                    .size((count * std::mem::size_of::<f32>()) as vk::DeviceSize)
                    .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);

                let buffer = device
                    .create_buffer(&buffer_info, None)
                    .expect("random buffer");

                let memory_requirements = device.get_buffer_memory_requirements(buffer);

                let memory = device
                    .allocate_memory(
                        &vk::MemoryAllocateInfo::default()
                            .allocation_size(memory_requirements.size)
                            .memory_type_index(find_memory_type(
                                instance,
                                physical_device,
                                memory_requirements.memory_type_bits,
                                vk::MemoryPropertyFlags::HOST_VISIBLE
                                    | vk::MemoryPropertyFlags::HOST_COHERENT,
                            )),
                        None,
                    )
                    .expect("allocate random buffer memory");

                device
                    .bind_buffer_memory(buffer, memory, 0)
                    .expect("bind random buffer memory");

                let randoms = fill_randoms(count);

                let mapped = device
                    .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
                    .expect("map random buffer") as *mut f32;

                for (index, value) in randoms.iter().enumerate() {
                    mapped.add(index).write(*value);
                }

                device.unmap_memory(memory);

                (buffer, memory)
            });

            //
            // Descriptors: one binding per reflection parameter, at the
            // binding index slangc assigned.
            //

            let bindings = parameters
                .iter()
                .map(|param| {
                    let descriptor_type = match param.kind {
                        ParamKind::RandomFloatBuffer => {
                            vk::DescriptorType::STORAGE_BUFFER
                        }
                        ParamKind::OutputTexture => vk::DescriptorType::STORAGE_IMAGE,
                        ParamKind::Unsupported(_) => unreachable!("validated before init"),
                    };

                    vk::DescriptorSetLayoutBinding::default()
                        .binding(param.binding)
                        .descriptor_type(descriptor_type)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::COMPUTE)
                })
                .collect::<Vec<_>>();

            let descriptor_set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .expect("descriptor set layout");

            let pool_sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(bindings.len() as u32),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_IMAGE)
                    .descriptor_count(bindings.len() as u32),
            ];

            let descriptor_pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&pool_sizes),
                    None,
                )
                .expect("descriptor pool");

            let descriptor_set = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(descriptor_pool)
                        .set_layouts(&[descriptor_set_layout]),
                )
                .expect("descriptor set")[0];

            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(rand_buffer.map(|(buffer, _)| buffer).unwrap_or(vk::Buffer::null()))
                .offset(0)
                .range(vk::WHOLE_SIZE);

            let image_info = vk::DescriptorImageInfo::default()
                .image_view(image_view)
                .image_layout(vk::ImageLayout::GENERAL);

            let writes = parameters
                .iter()
                .map(|param| {
                    let write = vk::WriteDescriptorSet::default()
                        .dst_set(descriptor_set)
                        .dst_binding(param.binding)
                        .descriptor_type(match param.kind {
                            ParamKind::RandomFloatBuffer => vk::DescriptorType::STORAGE_BUFFER,
                            ParamKind::OutputTexture => vk::DescriptorType::STORAGE_IMAGE,
                            ParamKind::Unsupported(_) => unreachable!("validated before init"),
                        });

                    match param.kind {
                        ParamKind::RandomFloatBuffer => {
                            write.buffer_info(std::slice::from_ref(&buffer_info))
                        }
                        ParamKind::OutputTexture => {
                            write.image_info(std::slice::from_ref(&image_info))
                        }
                        ParamKind::Unsupported(_) => unreachable!("validated before init"),
                    }
                })
                .collect::<Vec<_>>();

            device.update_descriptor_sets(&writes, &[]);

            //
            // Compute pipeline
            //

            let pipeline_layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&[descriptor_set_layout]),
                    None,
                )
                .expect("compute pipeline layout");

            let entry_name = CString::new(entry).unwrap();

            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader_module)
                .name(&entry_name);

            let compute_pipeline = device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage)
                        .layout(pipeline_layout)],
                    None,
                )
                .expect("compute pipeline")[0];

            //
            // Cover the whole image with the kernel's thread group size.
            //

            let group_count = [
                extent.width.div_ceil(group_size[0].max(1)),
                extent.height.div_ceil(group_size[1].max(1)),
                1,
            ];

            Pipeline::Compute {
                pipeline_layout,
                compute_pipeline,
                descriptor_pool,
                descriptor_set_layout,
                descriptor_set,
                image,
                image_memory,
                image_view,
                rand_buffer,
                group_count,
            }
        }
    }
}

unsafe fn find_memory_type(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> u32 {
    let memory_properties = unsafe {
        instance.get_physical_device_memory_properties(physical_device)
    };

    (0..memory_properties.memory_type_count)
        .find(|&index| {
            let memory_type = memory_properties.memory_types[index as usize];

            type_filter & (1 << index) != 0
                && memory_type.property_flags.contains(properties)
        })
        .expect("no memory type with the requested properties")
}

/// Uniform randoms in [0, 1) from a xorshift64* generator. The playground
/// fills its RAND buffers the same way (host-side, once at startup).
fn fill_randoms(count: usize) -> Vec<f32> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);

    let mut state = nanos | 1;

    (0..count)
        .map(|_| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;

            let mixed = state.wrapping_mul(0x2545_F491_4F6C_DD1D);

            (mixed >> 40) as f32 / (1u64 << 24) as f32
        })
        .collect()
}

struct App {
    window: Option<Window>,
    vulkan: Option<VulkanApp>,

    /// File name shown in the window title.
    shader_name: String,
    compiled: Option<CompiledShader>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = WindowAttributes::default()
            .with_title(format!("Slang Viewer — {}", self.shader_name))
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            // The viewer does not recreate the swapchain on resize yet.
            .with_resizable(false);

        let window = event_loop.create_window(attributes).expect("window");

        let compiled = self
            .compiled
            .as_ref()
            .expect("shader must be compiled before the window opens");

        let vulkan = unsafe { VulkanApp::new(&window, compiled) };

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
                // take() clears the field: winit still delivers a pending
                // RedrawRequested after this handler on X11, and it must
                // not touch the destroyed Vulkan objects.
                if let Some(vulkan) = self.vulkan.take() {
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
    let workdir = shader::create_workdir();

    let source = shader::resolve_source(&workdir);

    let compiled = shader::compile(&workdir, &source);

    // The viewer can only supply random buffers and the output texture;
    // reject anything else before any window or device exists.
    if let RenderMode::Compute { parameters, .. } = &compiled.mode {
        for param in parameters {
            if let ParamKind::Unsupported(what) = &param.kind {
                eprintln!(
                    "error: parameter '{}' is {what}; the viewer can only supply \
                     random float buffers and the output texture",
                    param.name
                );

                std::process::exit(1);
            }
        }
    }

    let event_loop = EventLoop::new().expect("event loop");

    let mut app = App {
        window: None,
        vulkan: None,
        shader_name: source.display_name.clone(),
        compiled: Some(compiled),
    };

    let result = event_loop.run_app(&mut app);

    // Scratch files are no longer needed once the app is done.
    let _ = std::fs::remove_dir_all(workdir);

    result.expect("event loop error");
}
