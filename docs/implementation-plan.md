# VATF 游戏动画重构计划

## 当前工程状态（2026-10-05）

世界动画、根播放/事件/动作链、换肤、全部 VATF 滤镜和已声明的固定功能混合已实现；需采样相机颜色的精确混合仍延期。矢量 UI 已支持 ExportAssets 图形、含子动画的 UI 和 DefineButton/DefineButton2 静态按钮状态，见 [vector-ui.md](vector-ui.md)。VAB 未发布，工作版本为 1；以下带日期的验证记录保留历史版本信息。

本轮整理将 UI 子资产构建提取到 `src/vab_asset/ui.rs`，UI 按钮、布局、栅格任务和缓存分开维护，公开 API 保持不变。GPU 测试按功能分组，并复用测试应用与读回辅助代码。缓存复用逐帧临时容器，恢复显示时重新建立已经淘汰的未完成请求。

Bevy 资产预处理已接入：普通动画、静态 UI、动态 UI 共用 vatf 的编译设置与字节编译入口，处理产物交给 VabLoader，保留标签子资产路径；见 [asset-processing.md](asset-processing.md)。手动转换和 `.vab` 加载继续可用。vatf 的相关开发已合入其 `main` 分支。原点辅助 API、精确按钮命中及键盘焦点样式仍是独立待办。

文档入口与模块地图见 [README](../README.md) 和 [architecture.md](architecture.md)。

输入契约：根 MC 就是动画根。本工程不识别包装层、不补帧、不运行 ActionScript。带根动作标签的素材采用“每段动作至多一个根级控制对象”的结构，编译器会抵消每段动作首个非空帧的根放置平移；无根动作标签的普通场景保持原坐标与多对象结构。普通子 MC 由父帧确定；`skin_<slot>` 实例通过子时间轴的帧标签直接定义具名静态变体。

坐标与原点设计见 [`coordinates-and-origin.md`](coordinates-and-origin.md)。动作表的逐片段源布局平移已在编译期规范化；运行时只翻转 Y。跨资源共享语义原点所需的 `anchor_origin`、舞台 bounds 和 clip 联合 bounds 尚未实施。

## 阶段与验收

- [x] P1：确定输入、片段、事件、换肤、资源所有权契约。
- [x] P2：vatf 格式升级；离线帧求值；根片段与重复事件；实例名称；滤镜模式；材质采样；纹理去重；编译/读取验证。
- [x] P3：Bevy 加载新格式；根播放器（播放/暂停/速度/循环/seek/完成）；换肤；无 GPU 回归测试。
- [x] P4：纯色/位图/渐变进入 Bevy Camera2d/Transparent2d，完成实体排序、GPU 读回和 spirit 多帧图像验证。
- [x] P5：RAII 临时纹理借还；Blur/Glow；按视图像素范围；嵌套滤镜范围与同帧复用测试。
- [ ] P6：遮罩、固定功能混合和全部 VATF 滤镜已完成；待补滤镜与换肤组合及参考画面对比。需要读取相机颜色的复杂混合延期到 Bevy 提供合适的屏幕纹理输入，或项目引入明确的相机中间目标方案之后。
- [ ] P7：依据测量优化加载、帧去重、批处理、跨帧淘汰；记录 CPU/GPU/显存数据。

每阶段记录实际验证，未完成的视觉特性不得以结构测试通过替代验收。保留现有工作区修改，不提交或覆盖用户已有工作。

P5 已完成：不可 Clone 的 RAII Lease、按完整 TextureDescriptor 分池、同帧归还后复用、局部 ping-pong 状态、120 帧闲置桶淘汰、按相机/视口/实体变换计算像素比例、嵌套滤镜输出范围，以及 Blur→Glow GPU 图像与峰值临时纹理验证。

## 数据契约

1. 根时间轴中除 event_xxx 外的每个标签都定义动作 [start, next_start)；最后一个到根末尾。anim_xxx 兼容写法会去掉 anim_ 前缀，其余标签原名即动作名。无动作标签时使用 default 整段；有标签时第一个必须位于第 0 帧，规范化后重复/空动画名报错。
2. event_xxx 根标签保存为列表，允许同名多次出现，编译为片段局部帧号；子标签不参与播放控制。
3. 普通子帧遵循 (父局部帧 - place_frame) mod 子帧数，不重置子相位。带动作标签时，每段根帧至多有一个根级控制对象；取首个非空帧的 PlaceObject 平移 `(tx, ty)`，对该段所有烘焙结果统一左乘 `Translate(-tx, -ty)`。只移除源动作表中的舞台摆放位置，保留缩放、旋转、斜切以及后续帧相对于动作首帧的根运动。空片段偏移为零，多根对象报错。无动作标签的普通场景不执行此规则。
4. 离线展开普通 sprite 时间轴与变换；保留滤镜/混合/遮罩边界和 skin 槽，运行时不再解释普通 sprite 时间轴。
   静态 `DefineText/DefineText2` 同样在编译期按字体字形、文本矩阵、字号、颜色和 advance 展开为合成 Shape 与单帧子时间轴，运行时不保留字体或文本状态。
5. 换肤使用单一明确约定：PlaceObject 实例名 `skin_xxx` 定义槽 `xxx`，所引用子时间轴的帧标签直接定义具名变体，未标记帧不参与换肤。默认显示第一个具名变体；每实体按槽和名称设置，支持批量原子设置；未知名称返回错误。保留槽外部变换和分组，不生成变体的笛卡尔积。
6. 正常播放发出跨越帧的事件；暂停不重复；seek 不补发；play/replay 首帧事件一次；循环入口再次触发首帧事件；非循环保持末帧，完成一次。一次性动画可进入有序后继队列或循环 fallback，同一更新跨 clip 时继续消费剩余时间并保留事件来源；terminal 动画清空队列、抑制 fallback 并锁定普通切换。
7. VabAsset 保存共享不可变数据；VabPlayer/VabSkin 保存实例状态；GPU 临时资源只在 render world 管理。
8. 纹理池按完整兼容规格分池；借出即移出空闲列表，最后一次逻辑读取录入后归还；最终输出与临时结果分离。CPU 借还必须与 GPU 编码顺序一致。
9. POD 明确小端，输出排序确定。格式尚未发布时允许在工作版本 1 上迭代，但格式变化必须同步重生成全部测试资源；发布后不兼容变更才递增版本。普通动画的时间轴载荷为 BAKD，UI 另有可选 UIGR/UIBT；源时间轴仅存在于编译过程和 oracle 测试中，不写入旧的 ANIM 块。

## 验证记录

- 修改前：当前库 cargo test --offline：5 单元 + 8 合成 + 2 加载测试通过。
- 2026-09-13：当前库 cargo test：24 个普通测试通过（8 单元、4 播放、1 换肤、9 合成、2 加载）；严格 Clippy 通过。
- 2026-09-13：vatf cargo test --offline：17 单元 + 3 SWF oracle + 1 文档测试通过；严格 Clippy 通过。
- 2026-09-13：现有 bevy_flash/assets/spirit2159src.vab 已重新生成，版本 2，约 2.5 MB。
- 2026-09-13：无窗口 GPU 测试验证两个 VAB 实体按 GlobalTransform.z 排序；spirit2159src 的纯色 58、位图 59、渐变 39 个网格经 Camera2d 主路径输出 3 帧 PNG，均有有效像素。
- 2026-09-13：对照旧版材质 Shader 修正 UV 仿射矩阵读取：VATF 的 Mat3 平移位于嵌入 Mat4 的第三列，不能按普通 Mat4 的第四列平移解释。spirit 第 0/4/9 帧复验后，错位的紫色位图块已回到角色火焰/粒子位置。
- 2026-09-13：场景实例数据改为单帧共享 storage buffer；一个动画 RenderCommand 内仅合并相邻且 mesh/texture 相同的实例。GPU 测试用 3 个逻辑实例得到 2 个 DrawPacket 并保持实体 z 排序；spirit 抽样帧为 33 个实例、32 个包，说明该资源自然可合并项较少，未通过重排破坏 Flash 深度。
- 2026-09-13：位图/渐变 GPU bind group 按 Image AssetId 缓存，并用实际 TextureViewId/SamplerId 检测 GPU 资源替换；已移除逐形状逐帧创建。spirit 三帧验证只创建 17 个绑定并命中复用 1018 次。
- 2026-09-13：新增位图 GPU 回归，固定 UV 平移选取指定纹素；随后扩大并替换同一 Image，验证新像素生效且 bind group 随 GPU texture view 重建。
- 2026-09-13：实现第一条真实 GPU 滤镜链路：Blur 子树先渲染到池化目标，再按横向/纵向及 SWF pass 数在同一 TextureTarget 上 ping-pong，最后合成。64×64 GPU 像素读回验证模糊保留中心、扩散到原几何边界外，并受离线计算边界约束。
- 2026-09-13：最终 `Transparent2d` 绘制由每个 Camera2d 的 `Msaa` 决定，动画实体不重复保存该配置。滤镜子树默认跟随摄像机采样数，也可通过全局 `VabFilterMsaa::Off` 单独关闭其中间纹理 MSAA，以降低重滤镜场景的临时显存峰值；这可能降低滤镜内部矢量边缘质量。滤镜几何先 resolve 到单采样纹理，再执行 Blur 等后处理。GPU 回归覆盖默认 4x 和摄像机 4x／滤镜关闭的组合。
- 2026-09-13：场景提取保留 Draw/ApplyFilter/Blend 层级；滤镜子树在 `camera_driver` 前写入池化纹理。动画仍只产生一个 `Transparent2d` 项，其 RenderCommand 按原 Flash 深度交替提交网格包和滤镜纹理四边形。场景 Blur GPU 读回验证中心、扩散和单项排队。
- 2026-09-13：实现 Glow。原图保持只读，模糊结果在目标另一面与一个额外 scratch 间切换，Glow 输出通过交换 lease 成为主纹理，无需最终复制；Glow 后处理同时只需三张单采样工作纹理，另加相机 MSAA 决定的短期几何附件。GPU 读回验证白色源图保留和红色外发光。
- 2026-09-13：滤镜准备按 `(RetainedViewEntity, animation entity)` 独立进行。双相机 GPU 回归以 1.0/0.5 正交缩放观察同一实体，验证缩放视图获得更大的滤镜像素目标，不会错误复用另一相机的范围或分辨率。
- 2026-09-13：嵌套 Blur→Glow GPU 回归验证源图与外发光像素，同时记录临时池当前/峰值纹理数及字节数；断言内层结束后的 lease 会在同帧归还，并由外层继续复用。
- 2026-09-13：Glow Shader 与 `../bevy_flash`、`../ruffle/render/wgpu/shaders/filter/glow.wgsl` 逐分支校准，inner/knockout/composite_source、Fixed8 strength 和 sRGB→linear 规则一致；参数位换算由单元测试固定。
- 2026-09-13：进入 P6 并完成遮罩执行。提取阶段把 Push/Activate/Deactivate/Pop 的 stencil 风格命令区间恢复成嵌套 `Mask` 节点；GPU 将遮罩与内容子树分别写入池化目标，以遮罩 alpha 乘预乘内容后合成。嵌套双遮罩 GPU 回归验证横向、纵向裁切，并保持一个 `Transparent2d` 项；诊断记录每帧遮罩层数。
- 2026-09-13：实例准备改为从每个 Camera2d 的 `RenderVisibleEntities<VabAssetHandle>` 出发；隐藏实例不再采样、提取、准备或排队，无关相机也不再与全部 VAB 实例做笛卡尔积。
- 2026-09-13：加入按实体保存的帧采样缓存，键包含 VAB AssetId 与资源 revision、材质 revision、clip、frame 和完整 `VabSkin`。命中时直接共享解析后的 `Arc<[Op]>`；Asset/材质/动画/帧/皮肤变化会失效，根变换、相机和 MSAA 仅重新执行逐视图准备。GPU 回归验证两个静止实例均命中，单个实例切帧后仅该实例未命中；同一 Image 的 GPU 纹理替换仍能更新绑定。
- 2026-09-13：加入逐视图静态隔离层输出缓存。缓存槽为 `(RetainedViewEntity, animation entity, packet index)`，内容键包含帧采样 generation、Image/Mesh revision、视图像素比例、MSAA 和输出尺寸；平移不失效，缩放、切帧及 GPU 源资源变化会失效。每个槽只保存当前输出，不保存动画历史帧；默认硬预算 64 MiB，8 个未使用渲染帧后淘汰。超出预算的输出仅持有到当前相机绘制结束。Spirit 静止帧的 6 个隔离层全部命中，缓存占用约 1.56 MiB。
- 2026-09-14：按 Bevy 0.19 的 Queue → Prepare 调度顺序拆分场景系统。`QueueMeshes` 从可见集合生成逐视图 CPU 数据、专门化管线并加入唯一 `Transparent2d` 项；`PrepareResources` 上传共享实例 storage buffer；`PrepareBindGroups` 解析纹理、隔离层和 bind group。RenderCommand 只消费已经准备好的包。
- 2026-09-14：新增 `examples/spirit.rs`，使用 `assets/spirit2159src.vab` 演示真实窗口渲染、WAI/ATT 标签跳转、暂停/继续、帧事件和完成后返回待机动画。
- 2026-09-14：修复 Blur/Glow 全屏后处理四边形的纵向约定。离屏几何以输出纹理左上角为 `(0, 0)`，后处理现在把 `uv.y = 0` 映射到 NDC 顶部；旧实现中 Blur 的横纵两遍偶数次翻转掩盖了问题，Glow 的额外合成遍导致最终隔离层倒置。Glow GPU 回归改用仅占上半区的非对称源图，明确验证源图和发光不会镜像；Spirit 多帧重新生成并通过。
- 2026-09-14：完成优化清单第 5 项。场景实例 storage buffer 和 bind group 跨帧保留，仅在容量按 2 的幂增长时重建；CPU 数据每帧写入已有 buffer。场景滤镜按视图/实体/包槽位、独立离屏渲染按实体复用 uniform buffer 与其动态绑定，槽位消失时释放。诊断新增实例 buffer/bind group 与滤镜 uniform 的累计分配/复用计数；Spirit 多帧实测为实例 buffer `1` 次分配/至少 `12` 次复用、实例 bind group `1` 次创建/至少 `12` 次复用、滤镜 uniform `7` 次扩容分配/`17` 次复用。
- 2026-09-14：完成优化清单第 6 项。纹理池现在分别报告借出 live/peak、池内总驻留、空闲驻留、桶数及最大桶的纹理数量和字节数；总驻留严格等于 live + idle。字节按 descriptor 的压缩块、全部 mip、array/depth 和 MSAA sample 计算逻辑载荷；wgpu 不公开后端 heap 对齐和驱动元数据，因此不把估算值描述成驱动物理分配量。持有 live lease 的旧桶不会再被 120 帧淘汰，lease 归还后下一帧才能释放。Spirit 抽样三帧结束时池内为 `78` 张、约 `8.35 MiB`，其中 `72` 张、约 `6.80 MiB` 空闲，分布在 `18` 个桶。
- 2026-09-14：完成优化清单第 7 项的基准决策。新增可重复的无窗口 release GPU 基准和 Extract/Queue/PrepareResources/PrepareBindGroups CPU 计时。当前机器、256×256、MSAA Off、120 个稳定帧下，1 个无滤镜实例为 `1 draw / 447.6 µs App::update`；100 个同资源同帧实例为 `100 draw / 504.4 µs`，四段 VAB CPU 合计约 `43.8 µs`；100 个实例交错两个 packet 布局不同的帧为 `150 draw / 511.8 µs`，四段合计约 `48.8 µs`。这些是 CPU 提交墙钟值，不是 GPU 完成时间。99 个实例和 draw 增加约 `56.8 µs`，再增加 50 draw 约 `7.4 µs`，尚不足以证明应引入会改变透明排序模型的跨动画批处理；当前保留一个动画一个 `Transparent2d` 项。基准命令：`cargo test --release --test render_gpu cross_animation_batching_baseline -- --ignored --nocapture --test-threads=1`。
- 2026-09-14：用 `spirit3021src.swf` 开始优化清单第 8 项的真实资源测量。619,360 字节 SWF 在 release 下约 `91.0 ms` 转为 9,231,753 字节 VAB；包含 508 帧、517 个网格（212 纯色、17 位图、288 渐变）。单实例暂停帧 0/169/338 分别为 `0.520/0.653/0.544 ms App::update`，对应 `2/114/44` 个 packet；按 60 次渲染/30 个源帧推进时平均 `1.303 ms`、`36.6 packet`、`9.7` 个滤镜层、`4.8` 次滤镜缓存 miss、约 686,610 个滤镜输出像素。相同测法下 2159 为 `0.704 ms`、25 packet、6 个滤镜层、3 次 miss、约 205,848 像素。3021 的主要压力来自动态滤镜而非帧采样：Extract/Queue/PrepareBuffers/PrepareBindGroups 平均约 `12.3/8.9/12.3/62.8 µs`。测量还暴露精确尺寸纹理池的跨帧驻留问题：240 帧后 3021 池内为 1,461 张、约 `313.84 MiB`（286 桶），2159 也达到 361 张、约 `46.45 MiB`；不同帧的略微尺寸变化不断创建新桶，120 帧闲置淘汰无法约束快速播放时的峰值。后续需为临时池加入尺寸分级和总驻留预算，再复测动态滤镜。3021 当前还有 6 个未支持复杂层，因此数值不代表最终全部效果的成本。
- 2026-09-14：修复 3021 暴露的临时纹理池增长。完整滤镜逻辑边界按尺寸自适应归类：不超过 256 像素按 16、257–1024 按 32、更大按 64 像素向上取整，确保目标尺寸、几何 viewport、采样 UV 和 Blur texel 步长保持一致。新增公开 `TransientTexturePoolSettings`，默认总逻辑驻留预算 64 MiB、闲置寿命 120 帧；每帧开始按 bucket 最近使用时间清除空闲纹理，live lease 可临时超过预算。3021 release 复测降至 312 张、38 桶、约 `63.36 MiB`，相对原 1,461 张、286 桶、`313.84 MiB` 减少约 79.8%；连续播放由 `1.303 ms` 降至约 `1.17 ms`。尺寸分级使平均滤镜像素由 686,610 增至 744,179（约 8.4%），但复用改善抵消了额外像素。32 MiB 预算可降至 158 张、24 桶、`31.08 MiB`，纹理新建从每帧 4.6 次升至 6.4 次，因此保留 64 MiB 默认值，低显存宿主可显式改为 32 MiB。2× 实体缩放时平均滤镜像素增至约 2,891,691（约 3.89×），64 MiB 预算仍将池限制在 84 张、16 桶、约 `61.47 MiB`，其中 live 约 `11.03 MiB`；每帧纹理新建升至 12.6 次，说明预算有效但高倍率播放更依赖频繁分配。全部 11 个 GPU 像素/结构回归通过。
- 2026-09-14：实现 ColorMatrix 滤镜。单次池化 ping-pong pass 按 Flash/Ruffle 语义先把输入从预乘 Alpha 恢复为直色，应用 4×5 矩阵（常量列除以 255），clamp 后再输出预乘 Alpha；透明像素避免除零。GPU 像素回归验证 50% Alpha 红色准确变为绿色且 Alpha 不变。3021 的 2× release 复测从 6 个未支持层降为 0，说明该资源此前剩余的滤镜缺口全部是 ColorMatrix；连续播放约 `1.07 ms App::update`，池驻留仍为约 `61.47 MiB`。
- 2026-09-14：实现 DropShadow 与 Bevel。DropShadow 复用 Glow 的原图/模糊图双输入合成，按 angle/distance 偏移模糊 Alpha，并支持 inner、knockout 和 composite-source 标志；GPU 回归分别验证向右外阴影和相反边缘的内阴影。Bevel 对同一模糊 Alpha 作正反方向采样生成高光与阴影，支持 outer、inner、full/on-top 和 knockout；颜色先线性化再以预乘 Alpha 合成。Glow、DropShadow 和 Bevel 共享一条保留原图的三纹理执行函数，任意数量的模糊 pass 仍只需一个额外 scratch lease，最终结果直接提升为主纹理而不复制。14 项串行 GPU 回归全部通过。
- 2026-09-14：实现 Convolution。任意合法行列尺寸的卷积核保存在只读 storage buffer 中，并按尺寸和完整浮点位模式缓存，动态参数继续复用容量增长式 uniform buffer。单个 ping-pong pass 在直色线性空间计算矩阵、divisor 和以 255 为单位的 bias，再输出预乘 Alpha；支持 preserveAlpha、边缘像素 clamp 和 unclamped 默认颜色。零 divisor 按 Flash 文档回退为 1；矩阵尺寸、元素数量、有限浮点值及设备 storage binding 上限在准备阶段验证。GPU 像素回归覆盖左右移位核的两种边缘策略，以及半透明输入的 divisor、bias 和 Alpha 保留。
- 2026-09-15：实现 GradientGlow 与 GradientBevel。两者复用 Glow/Bevel 的原图、模糊图和 scratch 三纹理执行路径；SWF 渐变记录在 CPU 端展开为缓存的 256×1 sRGB 色带，Shader 以滤镜 Alpha 强度或正反方向 Alpha 差选择色带。支持 outer、inner、full/on-top、knockout 和 composite-source 标志。`vatf::filter_dest_rect` 同步加入 GradientGlow 的单向偏移范围和 GradientBevel 的双向对称范围，避免外部效果被裁切。单元测试验证 RGBA 色带插值与边界计算，GPU 像素回归验证绿色外发光及白色→透明→黑色内斜角；全部 16 项串行 GPU 回归通过。
- 2026-09-16：`wu_kong.swf` 的首轮试验暴露按变体序号选择会把不同部件的不同语义标签错误对齐，因此该临时约定在验收前撤回。当时曾把工作格式临时标为 v3；当前发布前版本已统一回到 1。只有 `skin_<slot>` 实例会成为皮肤槽，其子时间轴的标签原名直接产生携带名称的 `BakedSkinVariant`，运行时 `VabSkin` 按名称选择。编译器拒绝空名、重名、同帧多标签和完全没有标签的皮肤实例；`VabAsset` 提供槽及共同变体名称枚举。已按直接标签规则重新生成 `assets/wu_kong.vab`，真实资源 CPU 回归验证各皮肤槽保留 `skin_1`，`wu_kong_skin` 示例从资产读取共同名称并已通过编译。
- 2026-09-20～21：完成 Flash 注册点、bounds、根空间与人工 `anchor_origin` 的讨论。bounds 不能用作播放锚点，跨资源共享脚底等语义仍需未来的显式锚点。随后检查 `123620.swf` 和已提取的 `123620-idle.swf`，确认合并文件属于把各动作摆放在舞台不同位置的动作素材表，不能直接把这些舞台平移当作游戏世界坐标。
- 2026-09-21：播放器加入显式 `play_loop`、`play_once`、`play_once_and_hold`、后继队列、循环 fallback 和 terminal 锁定。一次更新可跨越多个 clip 并继续消费剩余时间，低层事件记录来源 clip，避免外部完成消息切换造成一帧停顿或事件误归属。`spirit` 示例改为 ATT 播完自动回 fallback WAI；回归测试覆盖多段跨越、原子名称校验和死亡动画禁止被普通播放覆盖。
- 2026-09-21：修复 `123620.swf` 的静态文本加载失败。character 8 是引用 DefineFont3 字形的 `DefineText`，旧编译器把它误留为没有 SHAP/MORP 映射的普通 Shape 节点。vatf 现按 Ruffle 的 `text_matrix * translate(pen) * scale(height/font_scale)` 顺序，将 DefineText/DefineText2 的嵌入字形离线生成确定排序的合成 Shape 和单帧子时间轴；重新生成的 `assets/123620.vab` 无 unresolved 引用。新增真实资源 AssetServer 回归，确认合成字形进入 BAKD 且全部 clip/frame 可采样。
- 2026-09-22：根标签规则改为除 `event_` 外全部生成动作，`anim_` 仅作为可选且会去掉的兼容前缀。带标签动作表的每段动作必须至多有一个根控制对象；编译器对整段统一抵消首个非空帧的根平移，保留后续相对根运动及完整线性变换。无标签普通场景保持多对象与原坐标。编译器新增 4 项单测；`123620` 回归确认 IDLE/ATTACK 可加载采样；所有示例 VAB 已重新生成且版本保持 1。
- 2026-09-23：对照 Ruffle `Surface::new`、bitmap/filter Shader 确认 Flash 工作画布故意移除 sRGB 格式，以 `Rgba8Unorm` 在编码 sRGB 数值上执行颜色变换、滤镜与混合，最后复制到显示表面时才转换。本插件此前把私有滤镜输入和滤镜颜色提前线性化，现已改为编码色彩工作空间；位图预乘数据、Glow/Bevel/Convolution/GradientFilter、清屏值和公开输出的最终转换保持同一契约。卷积 GPU 回归以 `blue / 2 + 0.2 = 0.7`（约 179）固定该语义。Camera2d 可显式添加 Bevy 的 `CompositingSpace::Srgb`，VAB 网格和隔离层 pipeline 已像 Sprite/Mesh2d 一样按该 view specialization 输出编码色值，由 Bevy 的最终 blit 转换显示；新增半透明中灰 GPU 回归固定这一行为。真实 `spirit2159src` 检查确认每帧含 1 个 Lighten、3 个 Add 和多个滤镜隔离层；剩余混合差异主要是 Lighten Max 近似以及尚未支持的目标颜色采样公式。
- 与 Flash/Ruffle 基准画面的整帧像素差异及完整 GPU 时间测量尚未完成。VATF 当前序列化的 Blur、Glow、ColorMatrix、DropShadow、Bevel、Convolution、GradientGlow 和 GradientBevel 均已执行。未支持的复杂混合保留隔离并暂按 Normal 合成；Lighten 使用已记录精度限制的 Max 近似。跳过的混合通过 `FlashRenderDiagnostics::unsupported_vab_layers` 计数。

## 当前迁移取舍

- 2026-10-05：接入可选 Bevy SWF 资产预处理。vatf 提供共享 `SwfCompileSettings`（Animation/StaticUi/AnimatedUi）、`compile_swf` 内存入口和 `VatfBuilder::to_vab_bytes`；CLI 与原文件转换 API 共用该路径。`SwfToVabProcessor` 在计算任务池编译，输出由 VabLoader 加载，逻辑路径保留 `.swf#导出名`。Bevy 负责源文件/配置哈希和事务，编译器修订缓存路径独立于 VAB 工作版本。新增 processed_ui 示例及五项集成测试，覆盖三模式、缓存复用/失效、子资产、错误传播、同句柄热重载和无源文件发布加载。热重载测试注入 Bevy 监视事件，不依赖 OS 通知时序。

- 2026-10-05：vatf 在写出前按最终 BAKD/UIGR 引用裁剪资源，覆盖所有动作帧、皮肤变体、嵌套遮罩/组和按钮全部状态。同步重建 SHAP/SHME/MORP、几何、材质和纹理偏移；共享纹理范围仅写一次，源 builder 不被重排。新增统计 API 保留原转换 API，VAB 版本仍为 1。`123620.swf` 网格 `534 → 472`，纹理载荷约 `774 → 595 KiB`；现有五个 UI 示例输出大小不变。UI/普通动画的多余定义回归验证裁剪后文件与无多余定义的文件逐字节相等。

- 运行时 VAB 不再保存 ANIM 块，只保存正式采样所需的 BAKD；源时间轴仍由 vatf 在编译期间构建并用于 oracle 验证。
- 纹理继续使用 TEXT 内 offset/length 引用，但写入按内容去重、加载按范围和 sampler 共享 Image；独立显式纹理表及跨采样设置共享 GPU 图像留待加载布局优化。
- 当前只支持小端主机；大端明确拒绝，尚未实现 POD 字节转换。
- 根动画时长来自预处理 SWF；没有重新识别动画根或修改子相位。
- 主场景渲染遵循 Extract → QueueMeshes 内准备 → Transparent2d Queue → RenderCommand。每个动画实体只有一个排序项，实体内部按 Flash 深度连续提交 draw call；因此 GlobalTransform.z 能和其他 2D 实体排序，动画内部不会被全局排序拆散。
- 2026-09-13：完成 Normal/Layer/Add/Subtract/Screen 固定功能混合。顶层隔离包把混合延迟到动画唯一的 `Transparent2d` RenderCommand，嵌套隔离层直接在动画离屏目标应用相同 blend state；避免 Subtract 先与透明色运算而丢失结果。GPU 回归在一个动画实体内验证 Add、Screen 和 destination-minus-source Subtract。spirit 抽样帧的未支持层由 4 降到 1。
- 2026-09-14：确认 spirit 剩余的未支持层是 Lighten，并按项目当前视觉需求暂时沿用 `../bevy_flash` 的 `BlendOperation::Max` 近似。VAB shader 输出预乘 Alpha，因此固定功能 `Max` 只对预乘后的源、目标附件值逐通道取最大值；它不能实现 Ruffle 的 `src * (1-dst.a) + dst * (1-src.a) + src.a * dst.a * max(src/src.a, dst/dst.a)`，在半透明滤镜和抗锯齿边缘存在已知偏差。Godot 示例的 `max(base, blend)` 同样要求 shader 已经取得两张输入纹理。当前实现继续保留 Lighten 隔离边界，之后可直接替换为相机颜色副本和 Ruffle 式双纹理合成 pass。
- P6 当前进度：遮罩、VATF 全部滤镜、固定功能混合及 Lighten 固定单元近似完成。下一步做滤镜与换肤组合测试和整帧参考图差异测试。Multiply/Darken/Difference/Invert/Overlay/HardLight 依照 Ruffle 属于需要目标颜色的复杂混合；Alpha/Erase 还需要正确的 Flash Layer 边界语义。这些模式继续保留隔离层和诊断，但暂按 Normal 显示，等 Bevy 提供合适的屏幕纹理输入，或本项目正式采用相机中间目标后再实现。

## 性能优化清单

按收益、确定性和后续滤镜放大成本排序：

1. [x] **按视图可见集合准备。** 提取先按 `ViewVisibility` 排除隐藏实例，准备再从每个 Camera2d 视图的 `RenderVisibleEntities<VabAssetHandle>` 出发，避免不可见实例及无关相机的采样、uniform、滤镜目标和 render pass。若要获得可靠的相机视锥裁剪，后续还需为动画实例维护准确的 `Aabb`。
2. [x] **动画帧采样缓存。** 仅当 Asset/材质/clip/frame/完整 skin 选择改变时重新生成 `CommandList` 和解析后的 `Op`；根 `GlobalTransform`、相机和 MSAA 的变化只触发对应的视图准备。相机仍每个渲染帧绘制，暂停动画不再重复递归采样相同帧。
3. [x] **静态滤镜结果短期缓存。** 在采样 generation、GPU 源资源、视图像素比例、MSAA 和输出尺寸未变化时复用最终隔离层结果。每个视图/实体/包只保存当前结果，默认 64 MiB 硬预算和 8 帧未使用淘汰；通过主世界的 `VabFilterCacheSettings` 调整。缓存纹理仍来自统一纹理池，淘汰后立即归还。
4. [x] **拆分 Bevy RenderSystems 阶段。** Bevy 0.19 的 Queue 在 Prepare 之前，因此逐视图 CPU 数据和 phase queue 位于 `QueueMeshes`；共享 GPU buffer 位于 `PrepareResources`；纹理、隔离层和 bind group 位于 `PrepareBindGroups`。各阶段通过明确的中间资源传递，不依赖逆序准备。
5. [x] **复用每帧 GPU 对象。** VAB instance 数据使用长期保留的 `RawBufferVec`，按 2 的幂扩容并通过 `RenderQueue::write_buffer` 覆写；storage bind group 仅在 buffer 扩容后重建。滤镜 uniform 保持 128 字节动态偏移布局，按离屏实体或 `(view, animation entity, packet)` 槽位保留容量增长式 buffer 与 bind group；Image bind group 继续按 GPU view/sampler 缓存。
6. [x] **补充真实显存指标。** 保留借出 lease 的 live/peak，同时增加池内总纹理/逻辑驻留字节、空闲纹理/字节、bucket count，以及最大桶的数量/字节；总驻留覆盖空闲和被持久滤镜缓存持有的纹理。字节包括 block、mip、layer/depth、MSAA sample，不包含 wgpu 未公开的后端对齐与驱动元数据。
7. [x] **以基准决定跨动画批处理。** 旧版可在多个相同 VAB、相同帧且排序相邻时跨实体合并相同 mesh/material。单动画内部通常没有相同网格，spirit 抽样 33 个实例形成 32 个包。release 基准中，从 1 个实例/1 draw 增至 100 个实例/100 draw 的 `App::update` 增量约为 `56.8 µs`；交错帧再增加 50 draw 的增量约为 `7.4 µs`。当前不实现跨动画合批，保持一个动画一个 `Transparent2d` 项及其原子透明排序。以后只有目标平台 GPU 时间或真实游戏基准确认瓶颈时，才重新评估无滤镜、无遮罩、无隔离层且 packet 布局完全一致的整动画实例批处理。
8. **建立对比基准。** 至少覆盖单个 spirit、100 个相同无滤镜实例、100 个不同帧实例、多滤镜实例、暂停实例和双相机，记录帧采样 CPU、Prepare CPU、draw call、filter pass、池驻留字节和 GPU frame time。
