//! # Rust + Slang + Vulkan — Documentation Guide
//! - [https://vulkan-tutorial.com/](https://vulkan-tutorial.com/)
//! ## 1. What this program demonstrates
//!
//! This program is a minimal Vulkan renderer written in Rust using:
//!
//! - **winit** — creates the native application window and drives the event loop.
//! - **ash** — Rust bindings for the Vulkan API.
//! - **ash-window** — connects the winit window/display handles to a Vulkan surface.
//! - **Slang** — compiles the vertex and fragment shaders.
//! - **SPIR-V** — the shader intermediate representation consumed by Vulkan.
//!
//! The final rendering path is:
//!
//! ```text
//! winit Window
//!     │
//!     ▼
//! Vulkan Surface
//!     │
//!     ▼
//! Swapchain
//!     │
//!     ├── Swapchain Image
//!     │       │
//!     │       ▼
//!     │   Image View
//!     │       │
//!     │       ▼
//!     │   Framebuffer
//!     │
//!     ▼
//! Render Pass
//!     │
//!     ▼
//! Graphics Pipeline
//!     ├── Vertex Shader
//!     └── Fragment Shader
//!     │
//!     ▼
//! Command Buffer
//!     │
//!     ▼
//! Graphics Queue
//!     │
//!     ▼
//! Presentation
//! ```
//!
//! ## 2. The Vulkan object model
//!
//! A useful way to understand Vulkan is to separate **creation/discovery objects** from
//! **GPU execution resources**.
//!
//! ### `VkInstance`
//!
//! The instance is the application's connection to the Vulkan implementation.
//!
//! In this program:
//!
//! ```rust
//! let entry = Entry::load().expect("failed to load Vulkan");
//!
//! let instance = entry
//!     .create_instance(&create_info, None)
//!     .expect("failed to create Vulkan instance");
//! ```
//!
//! `Entry` loads the Vulkan loader. `Instance` is then used to enumerate physical
//! devices, create surfaces, and obtain other instance-level functionality.
//!
//! ### `VkPhysicalDevice`
//!
//! A physical device represents an actual GPU.
//!
//! ```rust
//! let physical_devices = instance
//!     .enumerate_physical_devices()
//!     .expect("failed to enumerate physical devices");
//! ```
//!
//! The program selects a device whose queue family supports both:
//!
//! ```text
//! GRAPHICS
//! +
//! PRESENT
//! ```
//!
//! That is important because this example submits graphics commands and presents
//! through the same queue.
//!
//! ### `VkDevice`
//!
//! The logical device is the application's interface to the selected physical GPU.
//!
//! ```rust
//! let device = instance
//!     .create_device(physical_device, &device_create_info, None)
//!     .expect("failed to create logical device");
//! ```
//!
//! A `VkDevice` owns access to device-level resources such as:
//!
//! - queues
//! - image views
//! - render passes
//! - pipelines
//! - command pools
//! - semaphores
//! - fences
//! - shader modules
//!
//! ### Queue
//!
//! A queue is where recorded GPU work is submitted.
//!
//! ```rust
//! let queue = device.get_device_queue(queue_family_index, 0);
//! ```
//!
//! The program later performs:
//!
//! ```rust
//! device.queue_submit(queue, ...);
//! ```
//!
//! and:
//!
//! ```rust
//! swapchain_loader.queue_present(queue, ...);
//! ```
//!
//! ## 3. Surface and swapchain
//!
//! A Vulkan surface represents the application's window as a presentation target.
//!
//! The surface itself does not contain the rendered pixels. Instead, the swapchain
//! provides a set of images that can be rendered to and presented.
//!
//! The program creates it with:
//!
//! ```rust
//! let swapchain = swapchain_loader
//! .create_swapchain(&swapchain_create_info, None)
//! .expect("failed to create swapchain");
//! ```
//!
//! Conceptually:
//!
//! ```text
//! Swapchain
//!    │
//!    ├── Image 0 ── ImageView 0 ── Framebuffer 0
//!    ├── Image 1 ── ImageView 1 ── Framebuffer 1
//!    └── Image 2 ── ImageView 2 ── Framebuffer 2
//! ```
//!
//! The exact number of images is selected from the surface capabilities.
//!
//! ### Why does `acquire_next_image()` return an index?
//!
//! Because Vulkan may give the application any currently available swapchain image:
//!
//! ```rust
//! let (image_index, _) = swapchain_loader.acquire_next_image(...)?;
//! ```
//!
//! The index is then used to select the matching framebuffer:
//!
//! ```rust
//! self.framebuffers[image_index as usize]
//! ```
//!
//! This is why the command buffer is recorded after acquiring the image.
//!
//! ## 4. Image vs. image view
//!
//! A Vulkan `Image` is the underlying GPU image resource.
//!
//! An `ImageView` describes how that image is accessed:
//!
//! ```rust
//! let info = vk::ImageViewCreateInfo::default()
//!     .image(image)
//!     .view_type(vk::ImageViewType::TYPE_2D)
//!     .format(surface_format.format)
//!     .subresource_range(subresource);
//! ```
//!
//! The render pass does not directly receive the raw swapchain image here. The
//! framebuffer contains the image view.
//!
//! ```text
//! VkImage
//!    │
//!    ▼
//! VkImageView
//!    │
//!    ▼
//! VkFramebuffer
//! ```
//!
//! ## 5. Render pass
//!
//! This example has one color attachment.
//!
//! The important settings are:
//!
//! ```rust
//! .load_op(vk::AttachmentLoadOp::CLEAR)
//! .store_op(vk::AttachmentStoreOp::STORE)
//! .initial_layout(vk::ImageLayout::UNDEFINED)
//! .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)
//! ```
//!
//! This means, conceptually:
//!
//! ```text
//! begin render pass
//!     │
//!     ├── discard/initialize old contents
//!     ├── clear the image
//!     ├── draw triangle
//!     └── make image suitable for presentation
//! ```
//!
//! The render pass also contains a subpass:
//!
//! ```rust
//! .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
//! .color_attachments(&color_refs)
//! ```
//!
//! A subpass describes how the graphics pipeline uses the attachment.
//!
//! ## 6. Shaders and SPIR-V
//!
//! The shaders are compiled by Slang before the Rust program runs.
//!
//! The Rust program includes the resulting binaries:
//!
//! ```rust
//! let vertex_code =
//!     include_bytes!(concat!(env!("OUT_DIR"), "/triangle.vert.spv"));
//!     
//! let fragment_code =
//!     include_bytes!(concat!(env!("OUT_DIR"), "/triangle.frag.spv"));
//! ```
//!
//! The flow is:
//!
//! ```text
//! Slang source
//!     │
//!     │ slangc
//!     ▼
//! SPIR-V
//!     │
//!     ▼
//! VkShaderModule
//!     │
//!     ▼
//! Graphics Pipeline
//! ```
//!
//! A shader module contains the compiled shader code. It is needed to create the
//! pipeline, but this program does not need to retain the modules afterward:
//!
//! ```rust
//! device.destroy_shader_module(vertex_module, None);
//! device.destroy_shader_module(fragment_module, None);
//! ```
//!
//! The resulting `graphics_pipeline` contains the pipeline state needed for later
//! execution.
//!
//! ## 7. Why there is no vertex buffer
//!
//! This is one of the most educational parts of the example.
//!
//! Normally a Vulkan application might do:
//!
//! ```text
//! vertex buffer
//!     │
//!     ▼
//! vertex input
//!     │
//!     ▼
//! vertex shader
//! ```
//!
//! This example deliberately avoids a vertex buffer.
//!
//! The shader obtains the vertex number through `SV_VertexID`.
//!
//! The Rust side simply submits:
//!
//! ```rust
//! device.cmd_draw(command_buffer, 3, 1, 0, 0);
//! ```
//!
//! The three vertices have IDs:
//!
//! ```text
//! vertex 0
//! vertex 1
//! vertex 2
//! ```
//!
//! The vertex shader converts those IDs into the triangle's positions.
//!
//! Therefore:
//!
//! ```text
//! CPU:
//!     vkCmdDraw(..., vertexCount = 3, ...)
//!     
//! GPU:
//!     vertex ID 0 -> triangle vertex 0
//!     vertex ID 1 -> triangle vertex 1
//!     vertex ID 2 -> triangle vertex 2
//! ```
//!
//! No vertex buffer is required.
//!
//! ## 8. Graphics pipeline
//!
//! The graphics pipeline combines many fixed rendering decisions.
//!
//! This example configures:
//!
//! ### Vertex input
//!
//! ```rust
//! let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
//! ```
//!
//! It is empty because there is no vertex buffer.
//!
//! ### Primitive assembly
//!
//! ```rust
//! .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
//! ```
//!
//! Three vertices form one triangle.
//!
//! ### Viewport
//!
//! ```rust
//! let viewport = vk::Viewport {
//!     x: 0.0,
//!     y: 0.0,
//!     width: extent.width as f32,
//!     height: extent.height as f32,
//!     min_depth: 0.0,
//!     max_depth: 1.0,
//! };
//! ```
//!
//! The viewport maps normalized/device coordinates to the framebuffer.
//!
//! ### Rasterization
//!
//! ```rust
//! .polygon_mode(vk::PolygonMode::FILL)
//! .cull_mode(vk::CullModeFlags::NONE)
//! ```
//!
//! The triangle is filled and back-face culling is disabled.
//!
//! ### Multisampling
//!
//! ```rust
//! .rasterization_samples(vk::SampleCountFlags::TYPE_1)
//! ```
//!
//! This example uses one sample per pixel.
//!
//! ### Color blending
//!
//! ```rust
//! .blend_enable(false)
//! ```
//!
//! The fragment color is written directly rather than alpha-blended with the
//! existing framebuffer color.
//!
//! ## 9. Command buffers
//!
//! Vulkan separates command recording from command execution.
//!
//! The program records:
//!
//! ```txt
//! begin command buffer
//!     │
//!     ├── begin render pass
//!     ├── bind graphics pipeline
//!     ├── draw 3 vertices
//!     └── end render pass
//! end command buffer
//! ```
//!
//! Only later does the queue execute it:
//!
//! ```rust
//! device.queue_submit(queue, &[submit_info], fence);
//! ```
//!
//! This separation is one of Vulkan's major design characteristics. It allows
//! applications to prepare GPU work explicitly and submit it efficiently.
//!
//! ## 10. Synchronization
//!
//! Synchronization is essential in Vulkan because CPU and GPU operations are
//! asynchronous.
//!
//! This example uses:
//!
//! ### Semaphore 1: `image_available`
//!
//! ```rust
//! image_available
//! ```
//!
//! It signals:
//!
//! > The swapchain image acquired by the application is ready for rendering.
//!
//! The queue waits for this semaphore before the color-output stage.
//!
//! ### Semaphore 2: `render_finished`
//!
//! ```rust
//! render_finished
//! ```
//!
//! It signals:
//!
//! > Rendering of the submitted command buffer has completed.
//!
//! Presentation waits for this semaphore.
//!
//! ### Fence: `in_flight`
//!
//! The fence is different because the CPU waits on it:
//!
//! ```rust
//! device.wait_for_fences(&[self.in_flight], true, u64::MAX);
//! ```
//!
//! It tells the CPU that the previous queue submission has finished.
//!
//! A useful mental model is:
//!
//! ```text
//! CPU
//!  │
//!  │ wait fence
//!  ▼
//! Acquire image
//!  │
//!  ▼
//! Record commands
//!  │
//!  ▼
//! Submit ────────────────► GPU queue
//!  │                         │
//!  │ image_available         │
//!  │────────────────────────►│
//!  │                         ▼
//!  │                    Render triangle
//!  │                         │
//!  │                         │ render_finished
//!  │                         ▼
//!  │                    Presentation
//!  │
//!  └──── fence signals when submission is complete
//! ```
//!
//! ## 11. Why the command buffer is recorded every frame
//!
//! The program has:
//!
//! ```rust
//! let (image_index, _) = acquire_next_image(...);
//! ```
//!
//! and then:
//!
//! ```rust
//! self.record_command_buffer(image_index);
//! ```
//!
//! This matters because the acquired image can change.
//!
//! If one command buffer permanently referenced only framebuffer 0, then presenting
//! framebuffer 1 or 2 would not automatically make the commands target that image.
//!
//! The example therefore records the command buffer using the framebuffer associated
//! with the newly acquired image.
//!
//! ## 12. Vulkan ownership and Rust ownership
//!
//! Vulkan itself uses explicit handle lifetime management.
//!
//! For example:
//!
//! ```rust
//! let image_view = device.create_image_view(...);
//! ```
//!
//! must eventually be paired with:
//!
//! ```rust
//! device.destroy_image_view(image_view, None);
//! ```
//!
//! Rust does not automatically know that a raw Vulkan handle needs that particular
//! destructor.
//!
//! This program therefore makes `VulkanApp` the practical owner of the resources:
//!
//! ```rust
//! struct VulkanApp {
//!     instance: ash::Instance,
//!     device: ash::Device,
//!     swapchain: vk::SwapchainKHR,
//!     ...
//! }
//! ```
//!
//! The `destroy()` method performs the cleanup.
//!
//! This resembles RAII in C++:
//!
//! ```cpp
//! class VulkanApp {
//! public:
//!     ~VulkanApp() {
//!         // destroy Vulkan objects
//!     }
//! };
//! ```
//!
//! but this example uses an explicit Rust method rather than relying on `Drop`.
//!
//! A production Rust Vulkan wrapper can instead implement `Drop` or use higher-level
//! RAII-style abstractions so resource cleanup happens automatically.
//!
//! ## 13. Correct destruction order
//!
//! The destruction order is important because Vulkan objects have dependencies.
//!
//! The program roughly performs:
//!
//! ```text
//! wait for GPU
//!     │
//!     ▼
//! semaphores / fence
//!     │
//!     ▼
//! command pool
//!     │
//!     ▼
//! framebuffers
//!     │
//!     ▼
//! graphics pipeline
//!     │
//!     ▼
//! pipeline layout
//!     │
//!     ▼
//! render pass
//!     │
//!     ▼
//! image views
//!     │
//!     ▼
//! swapchain
//!     │
//!     ▼
//! logical device
//!     │
//!     ▼
//! surface
//!     │
//!     ▼
//! instance
//! ```
//!
//! The general rule is:
//!
//! > Destroy dependent objects before the objects they depend on.
//!
//! For example, a framebuffer refers to an image view, so the framebuffer should
//! be destroyed before the image view.
//!
//! Likewise, device-owned resources should be destroyed before destroying the
//! logical device.
//!
//! ## 14. Why `device_wait_idle()` is used before destruction
//!
//! The program starts destruction with:
//!
//! ```rust
//! self.device.device_wait_idle().unwrap();
//! ```
//!
//! This establishes that submitted GPU work has finished before resources used by
//! that work are destroyed.
//!
//! Without an appropriate synchronization strategy, destroying an object while
//! the GPU is still using it would be invalid.
//!
//! ## 15. Vulkan + Rust safety boundary
//!
//! Most of the Vulkan operations in this example are inside:
//!
//! ```rust
//! unsafe {
//!     ...
//! }
//! ```
//!
//! This does **not** mean Vulkan is randomly unsafe.
//!
//! It means Rust cannot prove certain invariants from the types alone.
//!
//! Examples include:
//!
//! - a Vulkan handle must refer to a live object;
//! - an image must be used in an appropriate layout;
//! - synchronization must be correct;
//! - a command buffer must be in a valid recording/submission state;
//! - a Vulkan object must outlive resources that depend on it.
//!
//! The programmer is responsible for maintaining these invariants.
//!
//! Rust still provides valuable safety for ordinary program structure, references,
//! ownership of the `VulkanApp`, and other CPU-side data.
//!
//! ## 16. `shaderDrawParameters`
//!
//! This example explicitly enables:
//!
//! ```rust
//! .shader_draw_parameters(true);
//! ```
//!
//! The reason is documented in the source: the Slang-generated SPIR-V declares a
//! built-in associated with vertex drawing parameters, and the corresponding
//! Vulkan capability requires the Vulkan 1.1 feature.
//!
//! The initialization therefore:
//!
//! 1. requires Vulkan 1.1;
//! 2. queries `PhysicalDeviceVulkan11Features`;
//! 3. checks `shader_draw_parameters`;
//! 4. enables the feature.
//!
//! This is a good example of an important Vulkan rule:
//!
//! > A shader capability is not necessarily available merely because the GPU
//! > supports Vulkan. The required device feature must also be enabled.
//!
//! ## 17. Complete per-frame lifecycle
//!
//! The whole frame can be reduced to:
//!
//! ```text
//! wait for previous frame
//!         │
//!         ▼
//! acquire swapchain image
//!         │
//!         ▼
//! record command buffer
//!         │
//!         ├── begin render pass
//!         ├── clear color
//!         ├── bind pipeline
//!         ├── draw 3 vertices
//!         └── end render pass
//!         │
//!         ▼
//! submit to graphics queue
//!         │
//!         ▼
//! wait for rendering semaphore
//!         │
//!         ▼
//! present swapchain image
//! ```
//!
//! That is the central Vulkan rendering loop represented by this program.
//!
//! ## 18. The most important concepts to learn next
//!
//! After understanding this example, the next useful Vulkan topics are:
//!
//! 1. **Vulkan synchronization 2** — modern synchronization APIs.
//! 2. **Image layouts and barriers** — how Vulkan controls image usage transitions.
//! 3. **Descriptor sets** — passing buffers/textures/samplers to shaders.
//! 4. **Uniform buffers** — passing matrices and other per-frame data.
//! 5. **Vertex buffers** — replacing `SV_VertexID` with explicit vertex data.
//! 6. **Depth buffers** — adding depth testing.
//! 7. **Multiple frames in flight** — increasing CPU/GPU overlap.
//! 8. **Swapchain recreation** — handling window resizing and `OUT_OF_DATE_KHR`.
//! 9. **Command buffer reuse** — reducing per-frame recording overhead.
//! 10. **Vulkan validation layers** — catching incorrect API usage during development.
//!
//! ## 19. Suggested mental model
//!
//! When reading Vulkan code, ask these five questions:
//!
//! ```text
//! 1. What GPU resource is being created?
//! 2. What object owns or depends on that resource?
//! 3. What state/layout is the resource currently in?
//! 4. What synchronization makes the next operation legal?
//! 5. In what order must the resource be destroyed?
//! ```
//!
//! If those five questions are clear, most Vulkan code becomes substantially easier
//! to understand.

mod app;
pub mod shader;
pub mod vulkan;

pub use app::run;
