# SWF 资产预处理

开发时可以直接加载 SWF 路径，由 Bevy 在首次导入或源文件变化时编译成 VAB。
播放、UI、按钮、渲染和资源裁剪仍使用原有实现。

## 接入

为本库启用 `asset_processor` feature，并在 `FlashPlayerPlugin` 后加入
`VabAssetProcessorPlugin`。默认不启用此 feature，已有 `.vab` 用法不受影响。

```rust,ignore
use bevy::{asset::AssetMode, prelude::*};
use bevy_flash::{FlashPlayerPlugin, asset_processing::{
    VabAssetProcessorPlugin, vab_processed_asset_path,
}};

App::new().add_plugins((
    DefaultPlugins.set(AssetPlugin {
        mode: AssetMode::Processed,
        processed_file_path: vab_processed_asset_path("imported_assets/game")
            .to_string_lossy().into_owned(),
        ..default()
    }),
    FlashPlayerPlugin,
    VabAssetProcessorPlugin,
));
```

普通动画没有 `.meta` 时默认按 `Animation` 转换，加载方式为
`assets.load::<VabAsset>("spirit.swf")`。动画根、时长及标签需要像 CLI
转换一样预先满足资源规范，预处理器不发现根、不补帧。

静态或动态 UI 通过 `xxx.swf.meta` 指定模式，例如：

```ron
(
    meta_format_version: "1.0",
    asset: Process(
        processor: "bevy_flash::asset_processing::SwfToVabProcessor",
        settings: (mode: AnimatedUi),
    ),
)
```

| 模式 | 对应 CLI | 内容 |
|---|---|---|
| `Animation` | 无 UI 参数 | 根动作、事件、换肤 |
| `StaticUi` | `--ui` | 静态纯矢量 ExportAssets 和原生按钮 |
| `AnimatedUi` | `--ui-animated` | 纯矢量 ExportAssets，离线展开子动画 |

按钮属于 UI 模式，由导出定义判断类型，不另加模式或命名前缀。
三种模式共用 `vatf::SwfCompileSettings`、`compile_swf` 与最终资源裁剪；
原先三个文件转换函数仍可调用。CLI 也使用这一共享入口。

加载路径保留源文件名字和扩展名：

```rust,ignore
let graphic = assets.load::<VabGraphic>("background.swf#sparkles");
let button = assets.load::<VabButton>("login.swf#login_button");
// VabAssetLabel::Graphic / Button 的 from_asset 同样支持 .swf 路径。
```

缓存中的 `.swf` 文件实际是 VAB 字节，生成的 `.swf.meta` 指定 `VabLoader`。
不能仅把 SWF 放进普通 Unprocessed 模式，或给它直接指定 VabLoader：
那样不会执行转换。原有 `.vab#导出名` 的手动转换流程继续可用。

## 缓存和热重载

Bevy 负责源字节与 `.meta` 的哈希、输出写入、处理事务和缓存复用。
SWF 自包含，此处理器没有外部编译依赖；CPU 编译交给
`AsyncComputeTaskPool`，不在 I/O 池中进行矢量细分。

启用宿主 Bevy 的 `file_watcher` feature 可以自动监视源 SWF、配置和处理后
资源。关闭监视时，下次启动仍会检查源文件和配置哈希。热重载复用已有
资产句柄，后续更新由已有资产事件和 UI 缓存失效机制处理。

Bevy 不对处理器程序代码做哈希。本库使用独立的 `COMPILER_REVISION`
和 `vab_processed_asset_path`：例如当前缓存位于
`imported_assets/game/vab-compiler-1`。改变编译语义或输出布局时必须增加
编译器修订号，从新目录重建，不能只依赖 VAB 格式版本或默认设置的变化。
VAB 仍为未发布的工作版本 1。自定义缓存路径而不使用这个辅助函数时，
宿主须自行管理编译器变化后的失效。

旧修订目录不会自动删除；确认不再需要后由宿主清理。修订路径作用于该
Bevy 资产源的全部处理后资源，也会让同一源内的其他类型重新导入。

## 发布

先运行开发版本，完成全部需要发布的资源预处理。打包对应缓存目录中的
处理后文件及生成的 `.meta`，保持相同目录布局；原始 SWF 无需随游戏分发。
运行时仍用相同的 `.swf` 逻辑路径加载。

发布应用保持 `AssetMode::Processed`，使用同一缓存路径，并设置
`use_asset_processor_override: Some(false)`。可以关闭本库的
`asset_processor` feature；显式 override 还能避免宿主的 Bevy feature
合并意外启用编译器。资源不完整时直接加载失败，不退回运行时编译 SWF。

预处理不改变当前的混合、UI 矢量范围及按钮静态状态限制。

## 示例与验证

```powershell
cargo run --features asset_processor --example processed_ui
cargo test --features asset_processor --test asset_processing
```

示例源目录为 `examples/processed_assets`，包含静态图、六帧动态 UI 和
原生登录按钮，以及各自的 `.meta`。无需先手动执行 vatf。
如需监视实际文件变化，可以额外启用 `bevy/file_watcher`。

集成测试覆盖共享字节/文件 API、三种模式、子资产、无变更不重写缓存、
源文件与配置失效、无源文件的发布加载，以及 Bevy 源/处理后监视事件触发
重编译和同句柄网格热重载。监视事件由测试注入以避免依赖操作系统通知时序。
