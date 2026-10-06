# README 渲染截图

这些 PNG 是已有 GPU 测试的直接输出，复制入文档目录后未裁剪、调色或合成。
它们保留透明背景，来自测试资源，不是 SWF 编辑器导出的截图。
动态图只展示其中一帧，截图不是像素一致性或性能证明。

| 文件 | 来源 |
|---|---|
| `spirit2159src.png` | `benchmarks::spirit_frames_to_png` 的 `spirit2159src_0009.png` |
| `nameplate.png` | `ui::nameplate_ui_renders_without_editable_text` 的 `vab_nameplate_ui.png` |
| `animated-background.png` | `ui::background551284_export_animates_and_renders` 的 `background551284_408.png` |
| `button-up.png`、`button-over.png`、`button-down.png` | `ui::native_login_button_states_render_different_pixels` 的三种状态 |

重新生成前，需要让本库与同级 vatf 的接口/分支配套。运行：

```powershell
cargo test --test render_gpu spirit_frames_to_png -- --ignored --nocapture --test-threads=1
cargo test --test render_gpu nameplate_ui_renders_without_editable_text -- --ignored --nocapture --test-threads=1
cargo test --test render_gpu background551284_export_animates_and_renders -- --ignored --nocapture --test-threads=1
cargo test --test render_gpu native_login_button_states_render_different_pixels -- --ignored --nocapture --test-threads=1
```

角色截图写到 `target/render-validation`；UI 截图写到操作系统临时目录。
测试通过后，将需要展示的帧复制到本目录，保持上述文件名即可更新 README。
角色测试需要 `../bevy_flash/assets/spirit2159src.swf`；UI 源文件位于本库 `assets/`。
