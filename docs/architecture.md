# 架构与代码入口

## 数据与所有权

```text
SWF ── vatf 离线编译 ── VAB
                         │
                    VabLoader
                         │
         ┌───────────────┴────────────────┐
      VabAsset                       VabGraphic / VabButton
         │                                │
  VabPlayer + VabSkin                VabImageNode / VabButtonNode
         │                                │
   烘焙帧采样                          UI 布局与栅格缓存
         │                                │
  Extract / Queue / Prepare          一次性离屏任务
         │                                │
 Transparent2d                       完成后的 ImageNode
```

vatf 负责源格式解析、时间轴展开与 VAB 序列化。运行时不重新解释普通 Sprite 时间轴。可选的 `SwfToVabProcessor` 调用共享内存编译入口，在 Bevy 的处理流程中输出 VAB 字节，输出仍交给同一个 VAB 加载器。源文件与配置失效由 Bevy 管理，编译器变化通过修订缓存路径失效。

编译器写出前按最终烘焙帧做资源引用裁剪：遍历全部动作、皮肤变体、遮罩和 UI 导出帧，只保留引用到的 Shape/Morph、网格、材质与纹理，并同步重建偏移。UI 另有编译前的 ExportAssets 依赖筛选。裁剪不改帧、命名、坐标或格式版本，编译统计通过 `ResourcePruningReport` 返回；普通动画的最终裁剪尚不省去前面的解析和曲面细分。

资产保存共享不可变数据，实体组件保存播放和换肤状态；Render World 拥有 GPU 缓冲与临时纹理。UI 缓存额外持有公开输出 Image，使用完成标记发布图片，临时滤镜纹理仍由公共渲染池管理。

## 模块职责

| 模块 | 职责 |
|---|---|
| `vab_asset.rs` | 主资产、渲染命令、网格/材质解码和加载器入口 |
| `vab_asset/ui.rs` | 图形/按钮子资产校验、共享渲染资产构建和状态范围统一 |
| `asset_processing.rs` | 可选的 SWF→VAB Process、插件和编译器修订缓存路径 |
| `vab_graphic.rs`、`vab_button.rs` | 公开 UI 资产与具名加载标签 |
| `vab_player.rs`、`sampling.rs` | 根播放状态、事件、动作链和烘焙帧采样 |
| `vab_ui/mod.rs` | 公开 UI 组件、配置和插件调度 |
| `vab_ui/button.rs` | Bevy Interaction 到原生按钮状态的选择 |
| `vab_ui/layout.rs` | 资源固有尺寸与 Bevy 布局测量 |
| `vab_ui/raster.rs` | 可见 UI 播放推进、离屏任务创建和图片发布 |
| `vab_ui/cache.rs` | 缓存键、失效、待完成请求、任务回收及空闲淘汰 |
| `render/instance.rs` | 世界实例的可见性、采样缓存、阶段准备和原子排序 |
| `render/gpu.rs` | 离屏目标、滤镜/遮罩执行和 GPU 绘制 |
| `render/pool.rs`、`render/texture_cache.rs` | RAII 借还、尺寸分级、预算和驻留指标 |
| `render/diagnostics.rs` | 可选诊断插件 |

公开路径继续使用 `vab_ui::VabImageNode`、`vab_ui::VabButtonNode` 等，内部拆分不改变宿主调用方式。世界渲染、离屏渲染和 UI 栅格任务共用底层渲染器，不为按钮建立另一套 Shader 或滤镜流程。

## 调度与缓存

按钮选择发生在布局测量之前；资源尺寸测量在 Bevy Content 之后、Layout 之前执行；栅格调度在 Layout 和可见性传播之后执行。用户可通过 `VabUiSystems` 对自己的系统排序。

布局使用资源固定联合范围，不使用某帧纹理的分辨率。Stretch 图片的固有测量会被 Bevy 清除，因此每次必要时恢复测量；这避免自动高度归零和 DPI 反馈。

每个 UI 节点最多保留一个待完成请求。播放可以继续推进，上一张完成图片仍保持可见；请求完成后发布它并请求最新帧。来源、尺寸、MSAA 或适配方式变化时替换请求。隐藏节点暂停推进，空闲缓存可以淘汰；恢复显示时，已被淘汰的请求会重新建立。

缓存复用逐帧已用集合和淘汰候选数组，避免每帧重建这些临时容器。保留容量不代表继续持有已淘汰图片：图片句柄仅由缓存项、节点和待完成请求持有。

## 验证组织

普通播放、换肤、采样和加载测试仍位于 `tests/`。`tests/render_gpu.rs` 是单一 GPU 集成测试入口，具体用例分组到 `tests/gpu/`：

- `support.rs`：共用的无窗口应用、输出目标和 GPU 读回。
- `instances.rs`：透明排序、材质、混合、颜色空间及视图准备。
- `filters.rs`：各滤镜的输出像素。
- `ui.rs`：静态/动态 UI、按钮、布局、缓存及连续发布。
- `diagnostics.rs`：诊断开关。
- `benchmarks.rs`：真实动画读回与性能基准。

大型渲染模块尚未全面拆分。后续只有职责明确且回归覆盖充分时再拆滤镜执行和实例准备，避免在接入资产预处理时同时改变 GPU 生命周期。
