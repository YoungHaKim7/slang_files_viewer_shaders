use crate::{
    app,
    shader::{self, CompiledShader},
    vulkan::VulkanApp,
};

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes, WindowId},
};

use shader::{ParamKind, RenderMode};
use winit::event_loop::EventLoop;

/// Window size; also the swapchain's fallback extent when the surface
/// does not report one.
pub(crate) const WIDTH: u32 = 800;
pub(crate) const HEIGHT: u32 = 600;

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

    let mut app = app::App::new(source.display_name.clone(), compiled);

    let result = event_loop.run_app(&mut app);

    // Scratch files are no longer needed once the app is done.
    let _ = std::fs::remove_dir_all(workdir);

    result.expect("event loop error");
}

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
