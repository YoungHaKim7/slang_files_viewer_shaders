/// Program entry point.
///
/// winit owns the event loop. The application creates its window and Vulkan
/// state from the `resumed` callback, renders whenever a redraw is requested,
/// and explicitly destroys Vulkan resources when the window closes.
///
fn main() {
    slang_files_viewer_shaders::run();
}
