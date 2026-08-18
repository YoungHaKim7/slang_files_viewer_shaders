use crate::{
    shader::{DEFAULT_RAND_COUNT, ParamKind, ShaderParam},
    vulkan::{device::DeviceBundle, swapchain::SwapchainBundle},
};
use ash::{Device, Instance, vk};
use std::{
    ffi::CString,
    time::{SystemTime, UNIX_EPOCH},
};

/// Playground-style compute pass into an offscreen image that is blitted
/// to the swapchain.
pub(crate) struct Compute {
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
}

impl Compute {
    //
    // Compute pipeline: offscreen storage image + random buffer + the
    // descriptor set the kernel's parameters bind to.
    //

    pub(in crate::vulkan::pipeline) unsafe fn new(
        context: &DeviceBundle,
        swapchain: &SwapchainBundle,
        shader_module: vk::ShaderModule,
        entry: &str,
        group_size: &[u32; 3],
        parameters: &[ShaderParam],
    ) -> Self {
        unsafe {
            let instance = &context.instance;

            let physical_device = context.physical_device;

            let device = &context.device;

            let extent = swapchain.extent;

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

            let image = device
                .create_image(&image_info, None)
                .expect("storage image");

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

            device
                .bind_image_memory(image, image_memory, 0)
                .expect("bind image memory");

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
                let count = param.rand_count.unwrap_or(DEFAULT_RAND_COUNT) as usize;

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
                        ParamKind::RandomFloatBuffer => vk::DescriptorType::STORAGE_BUFFER,
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
                .buffer(
                    rand_buffer
                        .map(|(buffer, _)| buffer)
                        .unwrap_or(vk::Buffer::null()),
                )
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
                    &vk::PipelineLayoutCreateInfo::default().set_layouts(&[descriptor_set_layout]),
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

            Self {
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

    pub(in crate::vulkan::pipeline) unsafe fn record(
        &self,
        device: &Device,
        command_buffer: vk::CommandBuffer,
        swapchain: &SwapchainBundle,
        image_index: u32,
    ) {
        unsafe {
            let extent = swapchain.extent;

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
                .image(self.image)
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

            device.cmd_pipeline_barrier(
                command_buffer,
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

            device.cmd_bind_pipeline(
                command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.compute_pipeline,
            );

            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.descriptor_set],
                &[],
            );

            device.cmd_dispatch(
                command_buffer,
                self.group_count[0],
                self.group_count[1],
                self.group_count[2],
            );

            //
            // Offscreen: general -> transfer source
            //

            let to_transfer_src = vk::ImageMemoryBarrier::default()
                .image(self.image)
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
                .image(swapchain.images[image_index as usize])
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

            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer_src, to_transfer_dst],
            );

            //
            // Blit handles the format conversion between the shader's
            // rgba8 image and the swapchain format. The core 1.0
            // vkCmdBlitImage is used (the *2 variant needs Vulkan 1.3 or
            // KHR_copy_commands2).
            //

            let blit = vk::ImageBlit::default()
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

            device.cmd_blit_image(
                command_buffer,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                swapchain.images[image_index as usize],
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &blit_regions,
                vk::Filter::LINEAR,
            );

            //
            // Swapchain image: transfer destination -> present
            //

            let to_present = vk::ImageMemoryBarrier::default()
                .image(swapchain.images[image_index as usize])
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

            device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present],
            );
        }
    }

    pub(in crate::vulkan::pipeline) unsafe fn destroy(&self, device: &Device) {
        unsafe {
            device.destroy_pipeline(self.compute_pipeline, None);

            device.destroy_pipeline_layout(self.pipeline_layout, None);

            device.destroy_descriptor_pool(self.descriptor_pool, None);

            device.destroy_descriptor_set_layout(self.descriptor_set_layout, None);

            device.destroy_image_view(self.image_view, None);

            device.destroy_image(self.image, None);

            device.free_memory(self.image_memory, None);

            if let Some((buffer, memory)) = &self.rand_buffer {
                device.destroy_buffer(*buffer, None);

                device.free_memory(*memory, None);
            }
        }
    }
}

unsafe fn find_memory_type(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> u32 {
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };

    (0..memory_properties.memory_type_count)
        .find(|&index| {
            let memory_type = memory_properties.memory_types[index as usize];

            type_filter & (1 << index) != 0 && memory_type.property_flags.contains(properties)
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
