# Slang shader viewer

Opens a `.slang` file in a window and renders it with Vulkan. `slangc` from
the Vulkan SDK must be on `PATH`.

```bash
cargo run -- assets/triangle.slang        # vertex + fragment shader
cargo run -- assets/2d_splatter.slang     # playground-style compute shader
cat assets/2d_splatter.slang | cargo run  # same, source via stdin
```

## What it can display

- **Vertex + fragment** entry points with no resource parameters
  (e.g. `assets/triangle.slang`).
- **Playground-style compute** shaders that paint pixels through the Slang
  Playground's `drawPixel` (e.g. `assets/2d_splatter.slang`, the playground's
  gaussian-splat demo). Files saved from the playground without their
  `import playground; import rendering;` prelude still work: the viewer
  re-adds it (vendored in `assets/playground/`) and fills any
  `RWStructuredBuffer<float>` with random floats
  (`[playground::RAND(n)]` sets the count, default 131072).

Anything else (shaders needing meshes, textures, or URLs) is rejected with a
message.

## How rendering is chosen

The whole module is compiled once with `slangc -profile spirv_1_3
-fvk-use-entrypoint-name` plus `-reflection-json`; entry-point stages,
descriptor bindings, and the compute thread-group size are read from the
reflection. Vertex/fragment modules go through a render pass; compute
kernels dispatch into an offscreen `rgba8` storage image that is blitted to
the swapchain.
