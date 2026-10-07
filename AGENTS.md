# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

A Bevy library crate that loads and plays **VAB** (VATF) assets — animation files compiled from SWF by the `vatf` crate in `../vatf`. It is a port of the `../swf_player` (Ruffle-derived) player architecture onto pre-baked assets + Bevy. Supported engine/library combinations are listed in the README's Compatibility table; keep version support there rather than fixing it in this introduction.

This is a library crate with no `[[bin]]`. A host app adds `FlashPlayerPlugin`; `examples/spirit.rs` is the runnable playback/control reference.

The main VAB instance renderer draws solid, bitmap, gradient, nested alpha masks, Blur, Glow, ColorMatrix, DropShadow, Bevel, Convolution, GradientGlow, GradientBevel, the Normal/Layer/Add/Subtract/Screen fixed-function blend modes, and a documented fixed-function Lighten approximation through Bevy's Camera2d `Transparent2d` phase. Every filter variant currently serialized by VATF is implemented; exact blends that sample camera color are still outstanding. VAB v1 offline frame baking, root playback/events, playback queues/fallback/terminal locking, named skin sampling, and transient texture pooling are implemented. Follow `docs/implementation-plan.md` and `docs/playback-api.md` for current contracts and outstanding work.

## Commands

```bash
cargo test                       # CPU tests run by default; GPU tests are ignored
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo run --example spirit
```

Toolchain: edition 2024; use a Rust toolchain satisfying the supported Bevy checkout's `rust-version`.

Path dependencies must exist as siblings of this repo:
- `../bevy` — a Bevy checkout matching the README Compatibility table (`default-features = false`; adding a Bevy feature usually means adding it to `Cargo.toml` first)
- `../vatf` — the `vatf` crate: VAB reader, animation model, filter math, and the SWF→VAB compiler

The vatf UI and shared compiler APIs are merged into its `main` branch. There is no requirement to use the former UI development branch. Keep dependency APIs and generated VAB assets in sync when updating it.

vatf compacts resource chunks from final baked references before writing VAB.
All clip frames, skin variants, masks/groups and exported UI frames are roots;
do not infer liveness from the current skin, transparency or camera visibility.
Mesh, geometry, material, texture and morph offsets are rebuilt together while
character IDs and baked nodes stay unchanged. UI additionally selects source
ExportAssets dependencies before tessellation. `*_with_report` conversion APIs
return `ResourcePruningReport`; existing `Result<()>` APIs remain available.

`asset_processing::VabAssetProcessorPlugin` optionally registers SWF→VAB with
Bevy's Process API. The `asset_processor` feature enables Bevy's background
processor; AssetPlugin must use Processed mode. Metadata selects Animation
(default), StaticUi or AnimatedUi through shared `vatf::SwfCompileSettings`.
Processed `.swf` paths contain VAB bytes and generated VabLoader metadata;
`.swf#export` labels keep their normal meaning. Use `vab_processed_asset_path`
for compiler-revision cache isolation. Increment vatf COMPILER_REVISION for
compiler behavior changes, independently of the unpublished VAB working version.
Published apps disable processing and distribute processed bytes + metadata.

`[dev-dependencies]` enables bevy's `2d` feature, so `cargo test` compiles a heavier feature set than `cargo check`. Use `cargo test --lib` / `cargo test --test vab_load` to iterate faster.

The VAB format is still unpublished and its working version is 1; design changes may replace that format in place, but every checked-in `.vab` must then be regenerated together. Once a format is published, incompatible changes require a version increment. `VabReader` rejects files whose version or length doesn't match. The root MC is the animation root and clip/child frame lengths are aligned. Every non-`event_` root label defines a clip; `anim_` remains an optional stripped prefix. RootTranslationPolicy::Preserve is the compiler default. Explicit NormalizeClipStart requires at most one controlling root object per frame and removes each clip's initial root placement translation while retaining relative root motion. Unlabelled roots remain general scenes and keep their coordinates. Do not add root discovery or padding.

```bash
cd ../vatf
cargo test
cargo run --release -- fixtures/spirit2159src.swf \
    -o target/examples/spirit2159src.vab
```

## Architecture

### Data flow

Code navigation and current module boundaries are documented in `docs/architecture.md`.
Vector UI contracts are in `docs/vector-ui.md`; `VECTOR_UI.md` is a compatibility pointer.
UI internals are under `src/vab_ui/{button,layout,raster,cache}.rs` with stable public
re-exports from `src/vab_ui/mod.rs`. UI sub-assets are built by `src/vab_asset/ui.rs`.
GPU tests retain `tests/render_gpu.rs` as their harness and use `tests/gpu/` modules.

```
.vab bytes ──VabLoader──> VabAsset ──VabPlayer(clip/frame) + VabSkin──> sample()
                                                            └─> CommandList
                                                                 └─> Extract → Prepare → Transparent2d Queue → RenderCommand
```

**`src/vab_asset.rs`** — the heart of the crate.

- `VabLoader` declares `extensions() = &["vab"]`; without that, `AssetServer::load` cannot resolve it (Bevy resolves loaders by file extension).
- At load time it builds one `Mesh` + `MeshMaterial` per VAB ShapeMesh, registered as labeled sub-assets (`mesh_{i}`, `gmat_{i}`, `btex_{i}`, …). Embedded textures are WebP.
- `ShapeHandle` is a plain `usize` index into `render_meshes` — positional, so the loader **errors out** rather than skipping a mesh it can't classify (`unknown material type`). `shape_map`/`morph_map` ranges are validated against `render_meshes`.
- Morph meshes are resolved through each `MorphEntry`'s own vertex/index range, not a positional guess.
- `baked: BakedMovie` contains pre-evaluated ordinary sprite frames, clip-local events, and shared named skin variants. It is the only animation payload stored in VAB; compiler-side source timelines are not serialized as an `ANIM` chunk. Static `DefineText`/`DefineText2` records are flattened by vatf into synthetic colored glyph shapes and one-frame timelines, so the runtime has no font/text state. `sample()` in `src/sampling.rs` never evaluates ordinary source sprite timelines.
- Every non-`event_` root label defines a clip. `anim_foo` is exposed as `foo`; any other label keeps its full name. When NormalizeClipStart is explicitly selected for labelled action sheets, vatf subtracts the first populated frame's sole root-object translation from every frame in that clip. This preserves scale/rotation and motion relative to clip start. Empty clips use zero offset; multiple root objects are rejected. Unlabelled `default` scenes are neither restricted nor translated.
- A skin sprite must be placed with a `skin_<slot>` instance name. Its selectable frames use their unique, non-empty frame labels directly as variant names; unlabelled frames are excluded. `BakedSkinVariant` preserves the name and baked nodes, and `VabSkin` selects by name rather than frame ordinal.
- Sub-sprite timing: an instance records `place_frame` (the parent timeline frame it was placed on). Its child frame is `(parent_frame - place_frame).rem_euclid(child_frame_count)` — loops in phase with placement. **The parent frame is threaded down the recursion**; it is not the root frame.
- Filter/blend isolation mirrors swf_player's `render_base`: non-impotent filters or a non-`Normal` blend mode lift the object's commands into a sub-`CommandList`, wrapped as `VabCommand::ApplyFilter { commands, filters, bounds }` and/or `VabCommand::Blend(..)`. **Blend is applied independently of filtering** — a filtered object still gets its blend mode.
- `bounds` for `ApplyFilter` comes from `compute_command_bounds`; nested filters contribute their expanded output bounds, not their original geometry. New sampling expresses geometry and filter bounds in output pixel space. The **offscreen target itself is the renderer's job**; commands carry no texture handle.
- Filter radii are scaled by the view `scale` before `filter_dest_rect`, matching `swf_player/src/render.rs`.
- Vertex decode: `i16` positions normalized by `32767.0`, scaled by `bounds_half_x/y` + `bounds_center_x/y`; colors are sRGB bytes converted to **linear** for `Mesh::ATTRIBUTE_COLOR`.
- All range/offset errors are `anyhow::Error`s naming the mesh index and offending range — no `.unwrap()` on asset data.

**`src/vab_player.rs`** — one root player with clip-local frame, fractional-frame timer, speed, pause/loop/completion state. Explicit loop/once methods, queued successors, a looping fallback, and terminal locking support action chains without external completion-message control. `advance` carries remaining time across clip boundaries and preserves each event's source clip. The ECS system emits `VabFrameEvent` and `VabCompleteEvent` messages; skipped frames and loop crossings retain event order. Requires `VabAssetHandle` on the same entity. See `tests/playback.rs`.

**`src/render/instance.rs`** — the normal host-scene path. `VabAssetHandle` participates in Bevy visibility; hidden instances are excluded during extraction and QueueMeshes starts from each Camera2d view's visible VAB set. Sampling and parsed `Op` trees are cached per entity until the asset/material revision, clip, frame, or complete skin selection changes. Static top-level isolation outputs are cached by retained view/entity/packet while the sample generation, Image/Mesh revision, pixel scale, MSAA, and dimensions remain unchanged. Each slot retains only its current output; `VabFilterCacheSettings` defaults to a 64 MiB hard limit and eight unused frames. Bevy 0.19 orders Queue before Prepare: QueueMeshes collects CPU instance data, specializes pipelines, and inserts the sole phase item; PrepareResources overwrites a capacity-growing shared instance `RawBufferVec`; PrepareBindGroups resolves textures, filtered layers, and bind groups. The instance storage bind group survives until that buffer grows. Filter uniform buffers and their dynamic bind groups are retained per view/entity/packet slot, while independent offscreen rendering retains one slot per entity. Each view projects the entity's local axes through its camera and viewport, then scales geometry, filter radii, and nested filter bounds into output pixels before allocating intermediate targets. Before `camera_driver`, supported filter subtrees render into pooled textures. Each visible animation still adds exactly one `Transparent2d` item at its `GlobalTransform.z`; its RenderCommand alternates ordered mesh DrawPackets and procedural filtered-texture quads. Adjacent mesh instances merge only when mesh/texture match, so whole animations sort without interleaving their parts. Unsupported blend modes retain isolation but currently composite as Normal and increment `unsupported_vab_layers`; Lighten uses the documented fixed-function Max approximation.

MSAA and compositing space belong to the camera/view, not the VAB animation entity. VAB instance pipelines specialize from that view's `Msaa`; filter geometry uses the same sample count and resolves to a single-sampled texture before post-processing. Bevy's required-component default for `Camera` is `Msaa::Sample4`. Attach `CompositingSpace::Srgb` to a Camera2d for Flash/Ruffle-style gamma-space scene compositing; the custom mesh and filtered-quad pipelines then use Bevy's `SRGB_COMPOSITING` convention. Without that component, VAB output follows the camera's normal linear compositing contract.

**`src/render.rs` / `src/render/gpu.rs`** — optional `OffscreenViewTarget` rendering plus the pre-camera isolation graph shared by offscreen and scene paths. Private Flash targets deliberately use `Rgba8Unorm`: bitmap/color-transform/filter arithmetic and blending operate on premultiplied encoded-sRGB channel values like Ruffle's legacy working framebuffer. When Camera2d has `CompositingSpace::Srgb`, isolated and direct VAB output stays encoded for Bevy's `Rgba8Unorm` main target and Bevy converts it in the final blit. Linear cameras instead receive converted linear premultiplied output. Public `Rgba8UnormSrgb` output Images are converted and unpremultiplied on their final copy. Multi-pass Blur uses the target ping-pong pair. Glow and DropShadow keep the original and blurred images live with one additional single-sample scratch texture; Bevel samples that blurred alpha at opposite offsets for its highlight and shadow. GradientGlow and GradientBevel reuse that execution path and sample a cached 256×1 encoded-color ramp built from the SWF gradient records. These filters promote the result by swapping leases without a final copy. ColorMatrix applies its 4×5 transform to straight encoded color and returns premultiplied output in one ping-pong pass. Convolution keeps arbitrary validated kernels in cached read-only storage buffers and implements divisor, bias, preserve-alpha, clamped edges, and default edge color in one ping-pong pass. Mask command intervals are parsed into nested `Op::Mask` nodes; mask and content subtrees render to pooled targets and an alpha-mask pass multiplies premultiplied content by mask alpha. P6 only implements fixed-function blend modes that do not sample camera color. Ruffle-style complex blends are deferred until Bevy exposes a suitable screen texture input or this crate adopts an explicit camera intermediate target. The VAB instance extractor excludes `RenderOffscreenTexture` entities to prevent duplicate rendering.

**`src/render/texture_cache.rs`** — `FrameInternalTextureCache` buckets keyed by full `TextureDescriptor` (including MSAA sample count). Filter bounds use adaptive 16/32/64-pixel dimension classes so nearby animation frames reuse buckets without desynchronizing target size, UVs, and blur texel steps. Non-Clone RAII leases (`src/render/pool.rs`) are removed from free lists while borrowed and returned on Drop, enabling same-frame reuse. Ping-pong state is local to each `TextureTarget`. `TransientTexturePoolSettings` defaults to a 64 MiB total logical-resident budget and 120 unused frames; LRU idle textures are released first, while live leases may temporarily exceed the budget. `FlashRenderDiagnostics` separates borrowed live/peak textures from all pool-resident and idle textures, bytes, bucket count, and largest-bucket metrics. Byte totals are descriptor-derived logical payload including blocks, mips, layers/depth and samples; wgpu does not expose backend heap padding or driver metadata. Nested Blur→Glow GPU coverage verifies lease returns and resident = live + idle.

**`src/material.rs`** — `GradientMaterial` / `BitmapMaterial` assets hold their texture and texture transform. The VAB instance shader resolves these during extraction and renders all three material classes with Flash color transforms and premultiplied-alpha output.

### Tests

- `src/vab_asset.rs` (`#[cfg(test)]`) — pure helpers: `sub_frame` phasing, vertex dequantization, color linearization, decode error messages.
- `tests/synthetic_animation.rs` — builds a `VabAsset` in memory and asserts exact command output: transform/color accumulation, `place_frame` phasing and looping, filter scaling, blend-preserved-with-filter, nested bounds. No GPU, no `App`.
- `tests/vab_load.rs` — compiles `../bevy_flash/assets/spirit2159src.swf` and the static-text fixture `123620.swf` to temporary `.vab` files, loads them through a headless `AssetServer`, and cross-checks mesh counts, id resolution, per-frame sampling, and synthetic glyph references. Regenerating the compiler therefore can never leave the test asset stale.
- `../vatf/tests/swf_oracle.rs` — independently walks the source SWF's `PlaceObject`/`RemoveObject`/`ShowFrame` stream and compares it **frame by frame** against the compiler's baked frame data. This is the proof that every frame reproduces the source animation; the source timeline itself is not serialized into VAB.

### Conventions worth knowing

- Library-owned WGSL lives under `src/render/shaders`. `vab_instance.wgsl` is registered through Bevy's `embedded_asset!`; the private offscreen/filter shaders are compiled from `include_str!`. The root `assets` directory is reserved for runnable example assets.
- Color formats are mixed deliberately: private Flash targets and decoded premultiplied WebP payloads use `Rgba8Unorm` so their encoded-sRGB numeric values are preserved; public output Images use `Rgba8UnormSrgb`. A `CompositingSpace::Srgb` camera also uses a `Rgba8Unorm` main target followed by Bevy's conversion blit. Clear colors enter private targets as premultiplied `Srgba` channel values without linearization.
- Offscreen pixel space and sampled texture UVs both use the top-left as `(0, 0)`. Fullscreen post-process vertices must map `uv.y = 0` to NDC `y = 1`; otherwise an odd number of passes vertically mirrors the result.
- `RenderMeshGroup::local_bounds` is `[x_min, y_min, x_max, y_max]`; `compute_command_bounds` returns `[offset_x, offset_y, width, height]`.
- Comments cite the corresponding swf_player construct — when the port is ambiguous, `../swf_player/src/{command.rs,render.rs,display_object/movie_clip.rs,filter.rs}` is the reference implementation.
- VAB v1 preserves nested mask groups and sampling emits stencil-style command intervals. Extraction converts those intervals to nested alpha-mask nodes; crossing mask depth intervals remain rejected by the compiler.
