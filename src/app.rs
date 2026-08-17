use crate::shader::CompiledShader;
use crate::vulkan::VulkanApp;
use crate::{HEIGHT, WIDTH};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes, WindowId},
};

pub(crate) struct App {
    window: Option<Window>,
    vulkan: Option<VulkanApp>,

    /// File name shown in the window title.
    shader_name: String,
    compiled: Option<CompiledShader>,
}

impl App {
    pub(crate) fn new(shader_name: String, compiled: CompiledShader) -> Self {
        Self {
            window: None,
            vulkan: None,
            shader_name,
            compiled: Some(compiled),
        }
    }
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
