# 2.1 快速集成指南

在国内网络环境下，安装和调试大型Rust依赖可能会面临一些挑战。本指南采用**本地离线插件配置方案**，帮助您更顺畅地完成插件的安装、运行和调试。

## 前置准备

### 1. 安装Rust环境

下载并安装Rust官方工具链：
- 访问 [Rust官网](https://www.rust-lang.org/zh-CN/)
- 根据您的操作系统选择对应的安装包
- 按照安装向导完成安装

### 2. 配置国内镜像加速

为了提升依赖下载速度，建议配置国内镜像源。在 PowerShell 中执行以下命令：

```powershell
$ENV:RUSTUP_DIST_SERVER='https://mirrors.ustc.edu.cn/rust-static'
$ENV:RUSTUP_UPDATE_ROOT='https://mirrors.ustc.edu.cn/rust-static/rustup'
```

### 3. 验证Rust安装

安装完成后，在命令行中执行以下命令验证Rust版本：

```powershell
rustc -V
```

预期输出类似：
```
rustc 1.75.0 (82e1608df 2023-12-21)
```

## 获取项目源码

### 1. 拉取Bevy-Flash插件

访问 [Bevy-Flash GitHub发布页](https://github.com/aojiaoxiaolinlin/bevy_flash/releases)，下载最新的0.15版本源码。

### 2. 拉取Ruffle依赖

访问 [Ruffle GitHub仓库](https://github.com/ruffle-rs/ruffle)，拉取对应的源码。

### 3. 项目目录结构

请确保将两个项目放置在同一个父目录下，形成如下结构：

```
project/
├── bevy_flash/
│   ├── Cargo.toml
│   ├── src/
│   └── ...
└── ruffle/
    ├── swf/
    └── ...
```

## 配置依赖

打开 `bevy_flash/Cargo.toml` 文件，确保依赖配置如下：

```toml
[package]
name = "bevy_flash"
version = "0.1.0"
edition = "2021"
authors = ["傲娇小霖霖"]
license = "MIT OR Apache-2.0"
description = "A Bevy plugin for Flash Animation"

[dependencies]
swf = { path = "../ruffle/swf"}
```

## 构建与运行

在`bevy_flash`目录下执行以下命令构建项目：

```powershell
cargo build
```

构建成功后，可以运行示例代码：

```powershell
cargo run --example sample
```

## 常见问题

如果您在安装过程中遇到问题，请参考[常见问题](appendix/faq.md)章节获取解决方案。