# 2.1 快速集成指南
在国内网络中，安装调试如此巨大的rust的依赖并不是一个容易的事情，这里我们使用的是**本地离线插件配置方案**
来进行插件的安装、运行、调试
### 包体下载
下载Rust官方包体
https://www.rust-lang.org/zh-CN/
### 镜像配置
在 Powershell 中执行加速命令:
```
$ENV:RUSTUP_DIST_SERVER='https://mirrors.ustc.edu.cn/rust-static'
$ENV:RUSTUP_UPDATE_ROOT='https://mirrors.ustc.edu.cn/rust-static/rustup'
```
### Rust测试
在命令行中执行下面的命令查看对应的Rust版本（效果请查看image-demo1）
```
rustc -V
```
### image-demo 1:
![20250411-130925.png](https://img.picui.cn/free/2025/04/11/67f8a42fe07d0.png)

### 代码拉取
请访问**https://github.com/aojiaoxiaolinlin/bevy_flash/releases**

拉取0.15版本的插件项目

### 路径设计与修改

拉取Ruffle项目 和我们的插件放置到同一个目录
目录结构关系如下:
```
project/
├── bevy_flash-main/
│   ├── xxx.xxx
│   └── xxx.xxx
├──  ruffle/
│   └── xxx.xxx
```

### 最终依赖配置表
```
[package]
name = "bevy_flash"
version = "0.1.0"
edition = "2021"
authors = ["傲娇小霖霖"]
license = "MIT OR Apache-2.0"
description = "A Bevy plugin for Flash Animation"

[dependencies]
swf = { path = "../ruffle/swf"}
ruffle_render = { path = "../ruffle/render" }
ruffle_render_wgpu = { path = "../ruffle/render/wgpu" }
ruffle_macros = { path = "../ruffle/core/macros" }
swf_macro = { path = "./swf_macro" }
smallvec = { version = "1.13.2", features = ["union"] }
wgpu = { version = "23", default-features = false }

bevy = { version = "0.15", default-features = false, features = [
    "bevy_asset",
    "bevy_sprite",
] }

thiserror = "1.0"
anyhow = "1.0"
bitflags = "2.5"
lyon_tessellation = "1.0"
indexmap = "2.7"
copyless = "0.1.5"

uuid = "1.10"
enum-map = "2.7.3"

[dev-dependencies]
bevy = { version = "0.15" }


[profile.dev]
opt-level = 1
```
主要修改的部分是
```
swf = { path = "../ruffle/swf"}
ruffle_render = { path = "../ruffle/render" }
ruffle_render_wgpu = { path = "../ruffle/render/wgpu" }
ruffle_macros = { path = "../ruffle/core/macros" }
swf_macro = { path = "./swf_macro" }
```

### 项目运行demo
```
cargo run --example sample
```

### image_demo_2
![Snipaste_2025-04-11_13-35-58.png](https://img.picui.cn/free/2025/04/11/67f8aa4dae200.png)