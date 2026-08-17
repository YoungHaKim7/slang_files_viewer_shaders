# divided & conquer code

- The decomposition design is ready. Here's the essence of it:

- Done. The divide & conquer split is complete, compiles clean, and renders verified on both paths.

## Final tree (was: 1405-line `lib.rs`)

```
src/
├── lib.rs              47  mod decls, WIDTH/HEIGHT, run()
├── app.rs              90  App + ApplicationHandler (verbatim + small ctor)
└── vulkan/
    ├── mod.rs         126  VulkanApp{context, swapchain, pipeline, commands, sync} + new() + draw()
    ├── device.rs      178  DeviceBundle (now stores queue_family_index)
    ├── swapchain.rs   150  SwapchainBundle (now stores format)
    ├── commands.rs     42  Commands{pool, buffer}
    ├── sync.rs         52  SyncObjects (per-image render_finished comment kept)
    ├── frame.rs        32  record_command_buffer (begin + dispatch + end)
    ├── destroy.rs      21  pure orchestration, reverse creation order
    └── pipeline/
        ├── mod.rs      91  Pipeline (tuple variants) + new/record/destroy/wait_stage
        ├── graphics.rs 247  Graphics::new/record/destroy
        └── compute.rs  577  Compute::new/record/destroy + private find_memory_type/fill_randoms
```

Two things I finished beyond the mechanical move, per `docs/README.md`'s plan: constructors take `(context, swapchain)` — fixing `make_compute`'s 8-arg `(instance, physical_device, device, ...)` leak (also kills the clippy `too_many_arguments` warning) — and visibility is uniform `pub(crate)` with `Graphics`/`Compute` fields private to their files. All `match Pipeline` sites collapsed into `pipeline/mod.rs`. Teardown order and the load-bearing X11 `take()`→`destroy()`→`exit()` pattern are byte-identical.

## Verification (per the plan's gate matrix)

| Case                          | Result                                                                                   |
| ----------------------------- | ---------------------------------------------------------------------------------------- |
| `triangle.slang` (graphics)   | gradient triangle renders, validation layer silent, clean close, exit 0                  |
| `2d_splatter.slang` (compute) | full-window splatter renders, validation silent, exit 0                                  |
| splatter via stdin            | exit 0, validation silent                                                                |
| 3 error fixtures              | all exit 1 with correct messages (incl. `error_not_supply` → the rewritten `run()` path) |

`cargo check`/`build` clean; clippy shows only the 3 pre-existing `shader.rs` `collapsible_if` warnings. Skipped: missing-slangc case (untouched `shader.rs` path).

Two notes:
- `rustfmt` recursed from `lib.rs` and reformatted `src/shader.rs` (3 formatting-only hunks, no semantic change). I tried reverting it but the permission guard blocked discarding the changes — say the word and you can `git checkout -- src/shader.rs` yourself if you want the diff minimal.
- Window discovery for the scripted close needed `_NET_WM_PID` tree search (mutter-x11-frames reparents client windows); saved to memory along with the harness location `/tmp/viewer_verify.py`.

Working tree is uncommitted (`M src/lib.rs`, `?? src/app.rs`, `src/vulkan/`, plus the `shader.rs` formatting) — tell me if you want it committed.

<hr />

# Plan: full decomposition of `src/lib.rs` (1438 → ~45 lines)

## Target tree

```
src/
├── lib.rs            ~45 lines: mod decls, WIDTH/HEIGHT, run()
├── app.rs            ~75: App + ApplicationHandler (moved verbatim)
└── vulkan/
    ├── mod.rs        ~170: VulkanApp {context, swapchain, pipeline, commands, sync} + new() + draw()
    ├── device.rs     ~200: DeviceBundle{entry, instance, surface_loader, surface, physical_device, device, queue, queue_family_index}
    ├── swapchain.rs  ~140: SwapchainBundle{loader, swapchain, images, image_views, extent, format}
    ├── commands.rs   ~45:  Commands{pool, buffer}
    ├── sync.rs       ~55:  SyncObjects{image_available, render_finished, in_flight}
    ├── frame.rs      ~35:  record_command_buffer (begin/end + dispatch)
    ├── destroy.rs    ~60:  teardown as pure orchestration
    └── pipeline/
        ├── mod.rs    ~80:  Pipeline enum (tuple variants) + new/record/destroy/wait_stage dispatch
        ├── graphics.rs ~215: Graphics::new/record/destroy
        └── compute.rs  ~330: Compute::new/record/destroy + find_memory_type + fill_randoms (private)
```

## Key decisions

- **All 4 `match Pipeline` sites collapse** into methods: `Pipeline::new` (build dispatch), `wait_stage()` (the COLOR_ATTACHMENT_OUTPUT vs TRANSFER choice in `draw`), `record()`, `destroy()`. The only remaining match is inside `pipeline/mod.rs`.
- **`make_compute`'s parameter leak is fixed**: instead of `(instance, physical_device, device)`, constructors take `(&DeviceBundle, &SwapchainBundle)` — so `queue_family_index` and the surface `format` become stored fields.
- **Uniform visibility**: bundles and `VulkanApp` are `pub(crate)`; `Graphics`/`Compute` fields fully private to their files (all their access stays local).
- **No `Drop` impls** — teardown stays a manual, explicitly ordered call chain, each bundle owning its contiguous slice of today's sequence, with `destroy.rs` reviewable as one artifact against current `lib.rs:751-831`. The load-bearing X11 `take()` → `destroy()` → `exit()` pattern moves byte-identical.
- ash 0.38's extension loaders are owned `Clone` structs (verified), so bundling them is borrow-safe; only the Vulkan-level destroy order matters, which the plan preserves exactly.

## Sequencing — 10 steps, each ends build-green

Extract in dependency order with runtime checks at the risky points: **app.rs → pipeline (verbatim move) → DeviceBundle → SwapchainBundle → sync → commands → variant conversion (the one semantic step; verified with validation layers on both render paths) → frame.rs → destroy.rs → final assembly.** Interactive runs (`assets/triangle.slang`, `assets/2d_splatter.slang`) + error fixtures gate steps 3, 4, 7, 9; the full matrix (graphics, compute, stdin, three error fixtures, missing slangc) gates the end.

Full detail — exact line-range→file mapping, every method signature, and the teardown-order checklist — is in the plan. Want me to start executing it? I'd begin with the baseline check (`cargo build && cargo clippy`, record warnings) and Step 1.

<hr />

# `cargo clippy`

- Fixed all three `collapsible_if` lints in `src/shader.rs` by collapsing into let-chains:

- `src/shader.rs:154` — plain-compile path, with the "nothing displayable" comment moved outside the collapsed `if`
- `src/shader.rs:187` — scaffold-retry path
- `src/shader.rs:310` — graphics-mode selection (`parameters.is_empty()` joined with `&&`)

`cargo fmt` + `cargo clippy --all-features --all-targets -- -D warnings` now pass clean, and `cargo test` builds without errors.

<hr />

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


