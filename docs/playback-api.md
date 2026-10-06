# VAB v1 播放接口

输入 SWF 的根 MC 必须是动画根。根时间轴中除 `event_` 外的标签都定义动作；带标签的动作表由编译器移除各动作的初始根放置平移。插件已完成加载、离线帧采样、事件、换肤，以及通过 Camera2d/Transparent2d 的实体渲染、遮罩、Blur、Glow 和固定功能混合。

## 加载与实例

宿主已有 AssetPlugin（渲染宿主一般使用 DefaultPlugins），再添加 FlashPlayerPlugin。
组件 VabAssetHandle(Handle<VabAsset>)、VabPlayer、可选 VabSkin 放在同一实体。
FlashPlayerPlugin 注册 VabAsset、GradientMaterial、BitmapMaterial、加载器及播放器消息。
Image/Mesh 基础资产由 Bevy 的相应插件注册；无渲染的宿主需要手动 init_asset。

```rust,ignore
let asset = assets.get(&handle.0).unwrap();
player.set_fallback_loop(asset, "idle")?;
player.play_once(asset, "attack")?; // 播完自动回到 idle
player
    .play_once(asset, "skill_start")?
    .then_once(asset, "skill_end")?
    .then_loop(asset, "idle")?;
player.play_terminal(asset, "death")?; // 播放一次、保持末帧并锁定
player.reset_terminal();               // 复活时显式解锁
player.play_loop(asset, "idle")?;
player.set_speed(1.5)?;
player.pause();
player.resume();
player.seek(asset, 3)?;        // 片段内 0-based 帧，不补发中间事件
skin.set(asset, "hand", "red_armor")?;
let slots = asset.skin_slots();
let variants = asset.skin_variant_names("hand")?;
let commands = asset.sample(player.clip(), player.current_frame, &skin, Vec3::ONE)?;
```

sample 的 scale 为正的输出像素缩放；几何与滤镜范围使用同一输出像素坐标。翻转/旋转属于后续宿主显示变换，不用负的滤镜缩放表达。

带动作标签的资产以每段动作首个非空帧的唯一根对象平移为源布局偏移，编译时从整段统一减去；无动作标签的普通场景保留 SWF 根局部坐标。场景渲染只翻转 Y，不根据 bounds 或视口自动平移。跨资源统一脚底等语义原点的人工 `anchor_origin` 与舞台适配仍是未实施设计，完整契约见 [`coordinates-and-origin.md`](coordinates-and-origin.md)。

## 消息

使用 Bevy MessageReader<VabFrameEvent> / MessageReader<VabCompleteEvent> 消费消息。
帧事件包含 entity、animation、frame、name；name 不含 event_。
一次更新跨过多帧或多个循环时，仍按顺序发出沿途事件。同名事件不去重。
非循环动画完整播放 N/fps 秒后保持末帧，完成消息只发一次。
暂停不推进；seek 不触发目标帧或途中事件；play/replay 在下次推进时发首帧事件。
只支持非负播放速度；逆向播放及动画间权重混合不在当前契约内。

## 动画链、回退与终止播放

`play_once` 播完后优先进入 `then_once` / `then_loop` 队列；队列耗尽后进入
`set_fallback_loop` 设置的常驻动画。`play_once_then` 是“动作播放一次，然后循环待机”的快捷接口。
播放器在同一次更新内把越过动画边界的剩余时间继续用于后继动画，因此不会在动作末帧额外停一帧，跨 clip 的帧事件和完成消息仍携带各自正确的动画名。

`play_once_and_hold` 忽略 fallback 并保持末帧。`play_terminal` 还会清空队列并锁定播放器，适合死亡或退场；锁定后普通播放请求返回错误，必须先调用 `reset_terminal`。终止动画仍发送完成消息，实体销毁等游戏行为由宿主决定。

## 示例

`cargo run --example spirit` 加载示例 VAB。Space 播放一次 ATT，播放器随后自动回到 fallback WAI；R 立即返回循环 WAI，P 暂停或继续。示例不再监听完成消息来手动切换待机。

`cargo run --example show_demo` 加载 `123620.vab`，列出 IDLE、STB、BTS、APPEAR、ATTACK 等全部根标签动作；点击列表或使用键盘即可切换，适合检查逐动作根平移规范化后的定位。

`cargo run --example wu_kong_skin` 加载 `assets/wu_kong.vab`，启动时列出共同具名变体，数字键 1–9 选择。当前测试资源直接保留 `skin_1` 等源帧标签。

## 编译数据与限制

- 普通 sprite 已离线展开为 BakedNode；sample 不读取源 animations。
- 静态 DefineText/DefineText2 在 vatf 编译时展开为带颜色和排版变换的字形 Shape；运行时不加载字体或解释文本记录。
- PlaceObject 实例名必须是 `skin_<slot>`；其子时间轴中有标签的帧会成为皮肤变体，标签原名就是运行时变体名，例如 `default`、`red_armor`。
- 槽名会去掉 `skin_` 前缀；变体名不再要求额外前缀，也不依赖帧序号。重复或空标签、同帧多个标签、皮肤 Sprite 完全没有标签都会在编译期报错。
- skin 变体静态采样，变体内部普通子 sprite 固定第 0 帧。
- 同槽名的所有实例共享该实体的同一选择，但不同实体之间不共享选择。
- set 校验该槽所有符号的变体数量；set_many 失败时保持原选择。
- VAB 当前仍处于发布前设计阶段，格式版本为 1。运行时文件只保存 BAKD；构建 BAKD 所需的源时间轴是编译器内部数据，不再写入 ANIM 块。
- 纹理载荷按内容去重；加载时相同载荷范围和采样设置共享 Image。不同采样设置目前仍创建独立 Image。
- POD chunk 在小端主机直接读取，大端主机明确拒绝；没有声称已实现跨端字节转换。
- 编译器拒绝交叉遮罩深度区间；常规嵌套遮罩通过池化 alpha 目标执行。
- 无动作标签使用 default；根时间轴中除 event_ 外的标签都定义动作。anim_ 前缀可选且会从运行时动作名中去掉；第一个动作标签必须在根帧 0。

## 渲染接入

主场景路径自动执行 sample、Extract、Queue、Prepare 和 RenderCommand。每个 VAB 动画实体只加入一个 `Transparent2d` 项，内部绘制保持 Flash 深度顺序。运行时采样只读取 BAKD 烘焙帧。

用于复现 Flash/Ruffle 色彩合成的 Camera2d 应显式添加 `CompositingSpace::Srgb`：

```rust
commands.spawn((Camera2d, CompositingSpace::Srgb));
```

这个组件属于相机；`Camera2d` 不会自动插入它。VAB 的网格与隔离层 pipeline 会读取相机配置并使用 Bevy 的 `SRGB_COMPOSITING` 约定，在 `Rgba8Unorm` 主目标上按编码 sRGB 数值混合，最后由 Bevy 转换到显示表面。没有该组件或显式使用 `CompositingSpace::Linear` 时，VAB 会按 Bevy 默认的线性空间参与场景合成。

Lighten 当前使用 WGPU `BlendOperation::Max` 作为过渡实现，与旧版 `bevy_flash` 一致。它比较预乘 Alpha 的附件颜色，纯不透明区域符合逐通道 Lighten，半透明及抗锯齿边缘与 Flash/Ruffle 的双纹理公式存在已知偏差；隔离边界已保留，后续取得相机颜色输入后可替换为精确实现。

Flash/Ruffle 的私有工作画布同样使用 `Rgba8Unorm`，在编码 sRGB 数值上执行位图颜色变换、滤镜和混合。本插件的滤镜与隔离层遵循该行为。`CompositingSpace::Srgb` 已覆盖普通透明、Add、Subtract、Screen 以及当前 Lighten Max 近似的相机级 gamma-space 合成；它不会提供可采样的目标颜色，因此精确 Lighten 及 Multiply、Overlay 等复杂双纹理公式仍需 Bevy 提供屏幕纹理或由插件接管相机中间目标。
