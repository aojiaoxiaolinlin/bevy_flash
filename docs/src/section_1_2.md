# 2.2 示例SWF文件

Bevy-Flash插件仓库中包含了多个示例SWF文件，您可以使用这些文件来测试插件的功能和效果。本节将介绍如何获取和使用这些示例文件。

## 获取示例文件

### 1. 从GitHub仓库获取

示例SWF文件位于Bevy-Flash仓库的`assets`目录中。您可以通过以下方式获取：

1. 克隆或下载整个[Bevy-Flash仓库](https://github.com/aojiaoxiaolinlin/bevy_flash)
2. 进入仓库的`assets`目录查看所有可用的示例文件

### 2. 部分示例文件列表

仓库中包含了多种类型的SWF文件，涵盖不同的动画效果和功能测试：

- **角色动画**：如`123620-idle.swf`、`attack.swf`、`wu_kong.swf`等
- **特效动画**：如`bloodEffect.swf`、`blur.swf`、`glow.swf`等
- **滤镜测试**：如`filter_blend.swf`、`filter_test.swf`等
- **混合模式**：如`blend_add.swf`、`blend_demo.swf`、`blend_screen.swf`等
- **位图测试**：如`bitmap.swf`
- **文本测试**：如`graphic_text.swf`

## 使用示例文件

### 1. 基本用法

您可以通过以下步骤在示例代码中使用这些SWF文件：

1. 确保SWF文件位于您项目的`assets`目录下
2. 在代码中使用`asset_server.load`加载SWF文件
3. 创建Flash组件并添加到实体

示例代码：

```rust
use bevy::prelude::*;
use bevy_flash::*;

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // 加载SWF文件
    let swf_handle = asset_server.load("123620-idle.swf");
    
    // 创建Flash实体
    commands.spawn((
        Flash::new(swf_handle),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(FlashPlugin)
        .add_systems(Startup, setup)
        .run();
}
```

### 2. 测试特定功能

如果您想测试特定功能，可以选择相应的示例文件：

- 测试滤镜效果：使用`filter_test.swf`、`glow.swf`、`blur.swf`等
- 测试混合模式：使用`blend_demo.swf`及其相关文件
- 测试动画控制：使用`mc.swf`、`frames.swf`等

## 添加自定义SWF文件

除了使用自带的示例文件外，您还可以添加自己的SWF文件进行测试：

1. 将您的SWF文件复制到项目的`assets`目录
2. 按照上述基本用法加载并使用
3. 如果遇到兼容性问题，请参考[常见问题](appendix/faq.md)章节

## 注意事项

1. SWF文件的版本：建议使用ActionScript 2.0或3.0的SWF文件，以获得最佳兼容性
2. 文件大小：过大的SWF文件可能会影响加载性能
3. 资源引用：SWF文件中引用的外部资源可能需要单独处理

如果您在使用示例文件时遇到任何问题，请参考[调试技巧](development/debugging.md)章节获取帮助。