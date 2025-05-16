

# bevy_flash

Rendering flash animation in Bevy.WASM compatible!
> Reference [Ruffle](https://github.com/ruffle-rs/ruffle/);

## docs

访问 [中文文档](http://49.232.132.44/bevy-flash2/ "中文文档") | [文档源码](/docs "文档源码") 

## Goals

I want to bring Flash animations into the game engine to reuse old resources and thereby reconstruct Flash web games!


## Support

- [x] MovieClip Animation
- [x] Control Animation
- [ ] Blend Render
- [ ] Filter Render

## Example

[See online demo](https://aojiaoxiaolinlin.github.io/bevy_flash_demo/)

- run example

```bash
cargo run --example sample
```

- static example

    ![show_case](./docs/Readme/xiao_hai_shen_long.png)

- dynamic example

    ![bevy_flash_sample](https://github.com/user-attachments/assets/8bf354d0-0c7b-4bce-bd2f-65fb0fcbc590)

- filter effect example

    ![effect](./docs/Readme/filter_effect.gif)

> [!TIP]
> The filter effects are currently available in the `filter_render_dev` branch. Since I've modified some of the source code, you'll need to pull my [branch](https://github.com/aojiaoxiaolinlin/bevy/tree/bevy_flash_modify).

> [!IMPORTANT]
> Currently, the animation control still follows Ruffle's implementation, which is quite cumbersome. In the future, I may refer to the design of dedicated animation resources in other game engines for changes.

> [!WARNING]
> This project is still in the early stages of development.

## Getting Started


```rust
fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(Camera2d::default());
    commands.spawn((
        FlashAnimation {
            name: Some(String::from("name")),
            swf_movie: assert_server.load("spirit2724src.swf"),
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    commands.spawn((
        FlashAnimation {
            name: Some(String::from("another name")),
            swf_movie: assert_server.load("131381-idle.swf"),
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(-800.0, 200.0, 0.0)).with_scale(Vec3::splat(6.0)),
    ));
}
```

## Contributing
If you also want to complete this plugin, you are welcome to submit a Pull Request (PR) or raise an issue.  

## License

This code is licensed under dual MIT / Apache-2.0 but with no attribution necessary. All contributions must agree to this licensing.
