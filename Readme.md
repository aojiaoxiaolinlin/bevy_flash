# bevy_flash

## Rendering flash animation in bevy engine

> 部分技术参考[ruffle](https://github.com/ruffle-rs/ruffle/)项目

## 目前实现的功能

- [x] DefineShape & GradientFill
- [x] MovieClip Animation
- [ ] Control Animation

<!-- 插入图片 -->
## 初步实现渲染`Shape`

- `Color`填充
![展示](./assets/docs/shape.png)

- `Gradient`填充
![展示](./assets/docs/shape_gradient.png)

- example
![展示](./assets/docs/image.png)

> **note:** 目前依赖的`ruffle-render`版本是nightly-2024-08-06。后续版本wgpu版本不兼容。
