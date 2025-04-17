# 调试技巧
## 1.如何修改测试swf
在Vscode中，可以通过检索对应的参数值修改读取的内容

```
swf_movie: assert_server.load("frames.swf")
swf_movie: assert_server.load("131381-idle.swf")
```
## 2.如何调试指定的分支
以最新分支为例，比如 我们需要调试 blend_render_dev 分支，就可以点击拉取分支
然后配置如下的目录结构
```
Mode                 LastWriteTime         Length Name
----                 -------------         ------ ----
d-----         2025/4/14     13:17                bevy
d-----         2025/4/14     13:21                bevy_flash
d-----         2025/4/14     13:19                swf_animation
```
执行案例代码:
```
cargo run --example sample
```