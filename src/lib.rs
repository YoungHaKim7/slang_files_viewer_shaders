mod app;
pub mod shader;
pub mod vulkan;

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
