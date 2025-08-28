<!-- 保存为 README.zh-CN.md -->
<div align="center">
    <h1>bevy_flash</h1>
    <span><a href="./README.md">English</a> | 中文</span>
    <p><em>将 Flash 动画引入 Bevy 引擎，兼容 WASM！</em></p>

    <a href="https://github.com/ruffle-rs/ruffle">
        <img alt="参考实现" src="https://img.shields.io/badge/参考-Ruffle-blue" />
    </a>
    <a href="https://aojiaoxiaolinlin.github.io/bevy_flash_demo/">
        <img alt="在线 Demo" src="https://img.shields.io/badge/在线-Demo-green" />
    </a>
    <br/>
    <a href="LICENSE">
        <img alt="License" src="https://img.shields.io/badge/License-MIT%2FApache--2.0-lightgrey" />
    </a>
</div>

---

## 目标

我希望将 Flash 动画引入 Bevy 游戏引擎，焕发新生！

## ✨ 特性
- ✅ 动画播放控制（暂停/跳转/循环等）  
- 🟡 Blend 渲染（开发中，部分支持）  
- 🟡 滤镜渲染（开发中，部分支持，预览版见 `filter_render_dev` 分支）

## 📸 预览

> 在线 [Demo](https://aojiaoxiaolinlin.github.io/bevy_flash_demo/)

![](./docs/Readme/xiao_hai_shen_long.png)
![](./docs/Readme/bevy_flash_sample.gif)
![](./docs/Readme/filter_effect.gif)


## 🚀 快速开始

### 1. 运行示例
```bash
git clone https://github.com/aojiaoxiaolinlin/bevy_flash.git
cd bevy_flash
cargo run --example sample
```

### 2. 在项目中使用
```toml
[dependencies]
bevy_flash = { git = "https://github.com/aojiaoxiaolinlin/bevy_flash.git" }
```
最小使用示例
```rust
fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        Name::new("冲霄"),
        Flash(assert_server.load("spirit2159src.swf")),
        FlashPlayer::from_animation_name("WAI"),
        Transform::from_scale(Vec3::splat(2.0)),
    ));

    commands.spawn((
        Flash(assert_server.load("埃及太阳神.swf")),
        Transform::from_scale(Vec3::splat(2.0)),
    ));

    commands.spawn(Flash(assert_server.load("loading_event_test.swf")));
}
```

> [!TIP]
> 滤镜效果当前在 `filter_render_dev` 分支。由于我修改了部分源码，你需要拉取我的 [分支](https://github.com/aojiaoxiaolinlin/bevy/tree/bevy_flash_modify)。

> [!WARNING]
> 该项目目前仍处于开发的早期阶段。

## 🤝 贡献

欢迎 Issue、PR、讨论！
所有贡献默认接受 MIT / Apache-2.0 双许可证，无需额外署名。

## 📄 许可证
MIT 或 Apache-2.0 任选其一，详见 LICENSE。