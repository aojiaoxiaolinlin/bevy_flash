# Static vector UI: first implementation

Convert a symbol library with vatf:

```powershell
cargo run --release -- ui.swf --ui -o ui.vab
```

也可以通过 [Bevy 资产预处理](asset-processing.md) 直接加载 `.swf#导出名`，
在源 `.meta` 中选择 `StaticUi` 或 `AnimatedUi`。二者使用相同编译入口和渲染资产。

The explicit `--ui` mode uses ExportAssets names and only retains reachable
Shape/DefineSprite definitions and native DefineButton/DefineButton2 states. The root stage is ignored. Normal animation
conversion is unchanged. Every referenced Sprite must declare and contain one
frame. Recursive references, scripts, bitmap fills (including stroke fills),
bitmap characters, static text, morphs and other unsupported characters are rejected.
Editable text placements (`DefineEditText`) are omitted in UI mode; replacing a
shape with text removes the old shape at that depth. Exporting text itself is
rejected. Names and other live text should be supplied by Bevy UI.
Nested static groups, solid and gradient shapes use the existing renderer.

## Offline resource pruning

UI conversion first follows every ExportAssets entry and selects its source
dependencies before tessellation. A second pass, shared with ordinary animation
conversion, compacts resources from the final baked output just before writing
VAB. It walks every clip frame, every skin variant, nested masks/groups and every
exported UI frame (including button states and hit geometry).

Only referenced shapes, morph samples, meshes, vertices, indices, materials and
texture payloads are written. All dependent offsets are rebuilt together;
character IDs, export names, baked frames, transforms and playback timing stay
unchanged. No pruning is based on visibility, opacity or the current skin.
All exports are retained; selecting just one export is not part of this mode.

The CLI reports mesh counts and resource payload bytes before/after pruning.
The `convert_swf_*_with_report` functions and `write_vatf_with_report` expose
`ResourcePruningReport`; existing conversion functions keep their `Result<()>`
API. Payload bytes exclude chunk headers and unchanged baked-frame data.
Ordinary animation still parses/tessellates source definitions before this
final pass, so it reduces file size and runtime loading rather than avoiding
all compiler work. No format change is required; VAB remains version 1 and
existing files remain valid.

Names are nonempty, unique, contain no `#` or control characters, and cannot
start with the reserved `__vab/` prefix. Public labels have no extra namespace:

```rust,ignore
let graphic: Handle<VabGraphic> = asset_server.load("ui.vab#button_background");
let graphic: Handle<VabGraphic> = asset_server.load(
    VabAssetLabel::Graphic("button_background").from_asset("ui.vab"),
);
```

Types live in `bevy_flash::vab_graphic`. A VabGraphic contains original
geometric bounds and a shared-handle, single-frame rendering asset. Its geometry
is centered in Flash coordinates; the existing world renderer handles Y reversal.
The root animation's translation normalization is never used for UI symbols.
Geometric bounds do not include filter padding; image allocation must include
expanded render bounds if filters are present.

VAB v1 gains an optional UIGR chunk; ordinary animation files remain byte-format
compatible and do not need regeneration. New UI files need the updated loader.
Internal loader labels now start with `__vab/`; they are not public APIs.

## Cached UI rendering

The default `ui` Cargo feature exposes `VabUiPlugin`, `VabImageNode` and
`VabUiCacheSettings` in `bevy_flash::vab_ui`. Animation-only hosts can
set `default-features = false`. Add `VabUiPlugin` alongside `FlashPlayerPlugin`
and Bevy's UI plugins:

```rust,ignore
commands.spawn((
    VabImageNode::new(asset_server.load("nameplate3.vab#name_kuang")),
    Node { width: px(240), ..default() },
));
```

Layout uses the graphic's intrinsic visual bounds (including filter padding),
not the cached Image's physical resolution. Leaving width and height Auto uses
that natural size, subject to normal Bevy flex/grid constraints. Set only width
or only height for proportional sizing. Parent stretch, min/max constraints and
explicit aspect_ratio still follow Bevy layout rules.

When both dimensions are specified, the default VabImageFit::Contain preserves
the artwork's aspect ratio and centers it with transparent margins. Set
`node.fit = VabImageFit::Stretch` to explicitly allow deformation. Fit mode is
part of the raster cache key. The image is rasterized into the visual box
(ContentBox by default), so borders and padding are not counted as drawable size.

The component manages ImageNode.image and ImageNode.image_mode and installs its
own intrinsic ContentSize measurement. Do not override its atlas/rect or attach
another content-measure component (such as Text) to the same node. ImageNode tint
and flips, Bevy UI clipping, and child Text nodes remain supported. This is not
nine-slice scaling. UiTransform scales the existing raster; change Node layout
dimensions for higher-resolution rasterization.

After layout, physical pixel size (including DPI and UiScale), symbol and MSAA
select a shared cache entry. Same-size nodes share an Image; different sizes
rasterize independently. Internal jobs retry while GPU resources are pending.
A successful render sets a completion token and the job is removed, so a stable
image causes no subsequent extraction, preparation or offscreen drawing.
The final Image is still drawn by Bevy's normal UI renderer every frame.

Edits/removals to graphics, VAB payloads, meshes and materials invalidate the
cache conservatively; Image modifications also invalidate it. Invalidation
currently rebuilds all UI entries rather than tracking individual dependencies.
Cached images are not persisted across app runs.

Idle entries use an LRU byte budget (32 MiB) and a lifetime (120 frames).
Entries referenced by current nonzero-size nodes are retained even above that
budget. Raster dimensions default to a maximum of 2048 pixels; larger layouts
use a lower-resolution image. These controls are in VabUiCacheSettings. Byte
counts cover image payload, not backend allocation padding or external handles.

`cargo run --example ui_graphics` shows proportional width-only nameplates and Contain/Stretch comparisons. Editable text is omitted;
game labels should be separate Bevy Text nodes. No player or offscreen entity
needs to be created by the host. SWF asset preprocessing remains separate.
## Animated exported UI

Use `vatf input.swf --ui-animated -o ui.vab` to explicitly allow animated
pure-vector exports. The same file can contain static symbols. Static `--ui`
remains strict; animation assets without UIGR are unaffected. This updates the
unpublished UIGR schema in working VAB v1; old UI files must be regenerated.

A single-frame container persists while its descendants advance. Independent
persistent branches use the least common multiple of their periods. A multi-frame
container loops its own local timeline and resets descendant phases according
to the existing placement-based VAB timing contract, so its effective period is
its own frame count. Removing/replacing instances preserves their place_frame
phase. Exported names have no root animation-label or skin behavior. Scripts,
bitmaps and interactive button characters remain rejected; editable text is omitted.
Cycles are bounded to 4096 frames, with a one-million-node library bake budget;
excessive exports fail explicitly rather than truncating a loop.

Each export stores complete baked frames and SWF frame rate. All frames share
one geometry-center offset; the loader computes one union of their full visual
bounds including filter padding. UI size and raster origin remain fixed across
frames. `VabGraphic::visual_size()` returns the intrinsic visual size; `size()`
continues to return geometric size. There is no source timeline evaluation at runtime.

The same VabImageNode automatically loops an animated export. Its required
VabUiPlayback wraps VabPlayer: query it to call pause(), resume(), set_speed(),
or set_looping(false). Replacing the graphic resets the frame and fractional
progress while preserving speed/pause settings. Hidden or zero-size nodes stop
advancing. Returning to visibility resumes from their retained frame.

Frame index is part of the raster-cache key. Unchanged/paused frames reuse output,
matching node sizes and phases share images, and cached loop frames avoid new
rasterization. ImageNode only receives completed raster outputs. While a new
frame is uploading or rendering, the node retains its previous completed image;
initial loading remains transparent. Each node retains one pending request,
finishes it even if playback advances, then requests the current playback frame.
Earlier frames are idle entries subject to the same bounded cache
settings. Static/dynamic layer decomposition is not implemented; a cache miss
renders the complete baked frame. The synthetic six-frame fixture remains in animated_ui.vab. The example `cargo run --example animated_ui` now loads background551284.vab#sparkles, a real 24 FPS background with a 1224-frame (51-second) overall cycle.
Space pauses/resumes; left/right arrows change speed.

## Native vector buttons

Export a DefineButton or DefineButton2 using ExportAssets, for example
`login_button`. Type is inferred from the definition, with no naming prefix.
Convert using the same `--ui` option; ordinary Shape/Sprite exports can coexist.

```rust,ignore
let button = asset_server.load(
    VabAssetLabel::Button("login_button").from_asset("login.vab"),
);
commands.spawn((
    VabButtonNode::new(button),
    Node { width: px(180), ..default() },
));
```

`VabButton` lives in `vab_button`; `VabButtonNode` lives in `vab_ui`. The node
requires Bevy `ui_widgets::Button`, `picking::hover::Hovered` and VabImageNode.
The default, hovered and `ui::Pressed` states select up/over/down respectively.
`VabUiPlugin` installs `ButtonPlugin` unless the host already installed it.
Handle business actions with an observer for `ui_widgets::Activate`. Button records preserve depth, placement matrices,
color transforms, filters and blend modes, within the renderer's existing
blend support. All display states share one geometric registration and one
visual layout rectangle including filter padding, so state changes retain
position, aspect ratio and node size. Rasters are shared by state and size.
Like other UI graphics, a cold state retains the previous image until ready.
The individual static state graphics are also loadable as
`login.vab#login_button/up`, `/over`, `/down` and `/hit` when present.
Missing over/down records fall back to up. Up must contain valid geometry.

SWF HIT_TEST describes hit geometry, not a clicked or focused appearance. The
optional hit graphic is retained in VabButton.hit_test and aligned to the
button registration, but is not displayed or used for precise picking yet.
This version uses Bevy's complete node rectangle for interaction, including
transparent padding. Keyboard focus is separate and has no native SWF image.
Add Bevy's InteractionDisabled to retain up; application click handlers must
also respect that component. No disabled art is synthesized. ActionScript,
button sounds and track-as-menu behavior are not executed. States currently
require static vector Shape/Sprite subtrees; animated, bitmap and nested-button
states are unsupported.

VAB v1 uses an additional optional UIBT chunk for button state references. Existing
UIGR-only assets remain readable and do not need regeneration.
Run `cargo run --example ui_buttons` for the real login.swf fixture. Hover/press
the buttons; Space toggles disabled. Business handlers remain in the host app.
