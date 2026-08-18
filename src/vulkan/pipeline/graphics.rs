use crate::vulkan::{device::DeviceBundle, swapchain::SwapchainBundle};

use ash::{Device, vk};

use std::ffi::CString;

/// Classic vertex + fragment rendering through a render pass.
pub(crate) struct Graphics {
    render_pass: vk::RenderPass,
    pipeline_layout: vk::PipelineLayout,
    graphics_pipeline: vk::Pipeline,
    framebuffers: Vec<vk::Framebuffer>,
}

impl Graphics {
    //
    // Graphics pipeline: render pass + framebuffers + the vertex/fragment
    // stages, matching the previous build-time triangle setup.
    //

    pub(in crate::vulkan::pipeline) unsafe fn new(
        context: &DeviceBundle,
        swapchain: &SwapchainBundle,
        shader_module: vk::ShaderModule,
        vertex_entry: &str,
        fragment_entry: &str,
    ) -> Self {
        unsafe {
            let device = &context.device;

            let surface_format = swapchain.format;

            let swapchain_image_views = &swapchain.image_views[..];

            let extent = swapchain.extent;

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

            Self {
                render_pass,
                pipeline_layout,
                graphics_pipeline,
                framebuffers,
            }
        }
    }

    pub(in crate::vulkan::pipeline) unsafe fn record(
        &self,
        device: &Device,
        command_buffer: vk::CommandBuffer,
        swapchain: &SwapchainBundle,
        image_index: u32,
    ) {
        unsafe {
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
                    extent: swapchain.extent,
                })
                .clear_values(&clear_values);

            device.cmd_begin_render_pass(
                command_buffer,
                &render_begin,
                vk::SubpassContents::INLINE,
            );

            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                self.graphics_pipeline,
            );

            // No vertex buffer: SV_VertexID supplies the corner.
            device.cmd_draw(command_buffer, 3, 1, 0, 0);

            device.cmd_end_render_pass(command_buffer);
        }
    }

    pub(in crate::vulkan::pipeline) unsafe fn destroy(&self, device: &Device) {
        unsafe {
            for &framebuffer in &self.framebuffers {
                device.destroy_framebuffer(framebuffer, None);
            }

            device.destroy_pipeline(self.graphics_pipeline, None);

            device.destroy_pipeline_layout(self.pipeline_layout, None);

            device.destroy_render_pass(self.render_pass, None);
        }
    }
}
