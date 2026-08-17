# fix

- Done. Both problems are fixed and verified end-to-end.

## What was wrong

**1. The SIGSEGV (close-time crash).** winit's X11 loop drains the close event (running your `destroy()`) and *then* still dispatches the pending `RedrawRequested` — with no exit check in between. Since `about_to_wait` requests a redraw every iteration, `draw()` always ran once on already-destroyed Vulkan objects. Fixed in `src/lib.rs` by `self.vulkan.take()` in `CloseRequested`, so the late redraw finds `None`.

**2. The file was never read.** `build.rs` baked `assets/triangle.slang` into the binary at compile time; stdin/arguments were ignored. Now everything compiles at runtime.

## What it does now

```bash
cargo run -- assets/2d_splatter.slang     # the file you tried
cargo run -- assets/triangle.slang        # vertex/fragment shaders
cat assets/2d_splatter.slang | cargo run  # your original stdin form works too
```

`2d_splatter.slang` is the playground's `gsplat2d` demo with its scaffolding stripped (`drawPixel` and `[playground::...]` come from a prelude the web playground injects). The viewer now vendors that prelude (`assets/playground/`), re-adds the imports when a file doesn't compile standalone, fills `randBuffer` with random floats, and picks the render path from `slangc -reflection-json` (entry stages, descriptor bindings, thread-group size). Compute kernels dispatch into an offscreen `rgba8` image blitted to the swapchain.

## Verification (via the SDK's screenshot + validation layers, since X11 capture is broken under your compositor)

- Triangle: pixel-exact (red/green/blue corners, clear color), **0 validation errors**, exit 0
- Splatter: renders the full gaussian-splat field, 0 validation errors, exit 0 — plus a synthetic color-ramp compute shader matched expected RGB values exactly, proving the compute→blit channel mapping
- Validation also caught and I fixed three real bugs: SPIR-V 1.5 vs Vulkan 1.1 (now `-profile spirv_1_3`), missing `TRANSFER_DST` on the swapchain, `vkCmdBlitImage2` unavailable on 1.1 (switched to core `vkCmdBlitImage`), plus a pre-existing single-semaphore present race (now one semaphore per swapchain image)
- Error paths: bad file → slangc diagnostics; missing file → exit 2; unsupported shaders (like `shader.slang`, which needs meshes) → clear message

I saw your `a0822f0` commit — the small remaining diff on top of it is the validation-layer fixes + README. Window is non-resizable for now (no swapchain-recreation logic); live-reload and playground attributes like `TIME`/`MOUSE_POSITION` would be natural next steps.
