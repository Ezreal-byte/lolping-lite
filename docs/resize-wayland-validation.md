# 设置窗口拖边与 Ubuntu Wayland 排查记录

目标：独立版 LOLPing Lite，基于 v0.2.0。用户环境为 Windows 拖动设置窗口边缘调整大小，以及 Ubuntu 24.04.3 LTS / GNOME Wayland。

## 已确认的原因与边界

- Ubuntu 设置能够打开，但启动失败，原实现的 `global_input_available` 明确拒绝 Wayland/XWayland。只移除拒绝条件无法让 X11 helper 获得原生 Wayland 应用的全局输入，也无法让 X11 覆盖层变成 Wayland 原生覆盖层。
- Linux 动画另外存在定时唤醒缺失：Tao 0.35 的 GTK `WaitUntil` 没有注册 GLib 定时源，在没有外部输入时会阻塞。GNOME 集成截图暴露了信号停在首帧的问题；先前定期查询 diagnostics 恰好推动了动画，不能作为自主播放的证据。新增单次 GLib 定时唤醒及“600 ms 内完全不发 RPC／输入、帧数仍增长”的集成断言；空闲仍按原 500 ms 周期等待，不使用忙轮询。修复仅编译到 Linux。
- Ubuntu 24.04 仓库实际安装的 GJS `1.80.2-1build2` 存在 [incremental GC 错误](https://github.com/GNOME/gjs/commit/4e53510fbcf9472c4c1fa43661a7f409a012511d)。逐帧传递大 Uint8Array 时，嵌套和原生虚拟 GNOME 都复现了整个 Shell 的 JavaScript 回调被错误阻断，继而 Present 超时。实现改为原生 Cairo 解码无损 PNG、显式释放旧图像；CI 保留系统默认 GJS 和原有超时，通过持续至少 300 帧及普通输入断言验证，未用升级 CI 库或放宽断言代替兼容性修复。
- Windows 设置的缩略图缓存缺失时原先在 UI 线程同步读取和解码，FIFO 缓存也不反映最近访问；拖边导致可见内容变化时会出现额外阻塞。新实现使用一个有界后台解码线程和 LRU 缓存，限制同时排队任务及每帧纹理上传。
- eframe 0.37 在 Windows resize 回调中同步绘制。原始 944 次采样中有 218 次尺寸变化，eframe 中位耗时 5.75 ms、p95 8.85 ms；3 次约 2.4 秒的长帧占相邻采样间隔的 99.4%–99.8%。这段计时排除了交换缓冲和 OpenGL context 切换，仍需区分应用逻辑、绘制、驱动调用和平台输出。保留生产默认设置，添加仅测试用的帧记录与 vsync A/B 参数。
- 已取得的原版拖边记录有长帧；随后桌面控制收到用户 Escape 停止信号，实机 A/B 未完成，不能据此宣传“拖边卡顿已完全消除”。之后只执行代码与非桌面验证。

## 验证状态

本地完整 Rust 测试 57 项通过、2 项默认忽略（真实桌面和按需性能基准），其中 UI 24 项通过；GNOME 协议／打包边界 23 项通过。UI 回归包含解码任务去重、坏图不重复解码、关闭后线程退出，以及 3840×2560 完整素材库预热后连续 100 次 resize 不产生新的解码任务。

缩略图缓存最多 96 张，覆盖当前合法素材上限 81 张；最大 RGBA 纹理数据约 6 MiB，依然仅加载可见内容。单个解码线程、有界任务队列以及每次 UI pass 最多上传 2 张小纹理，避免大量资源同时阻塞界面。

## Windows 离屏对照

同一 release 测试可执行文件，交替运行同步／异步解码各 3 轮；每页 20 个冷帧、300 次测量 resize，表情页含 68 个测试条目。同步模式保留同一缓存实现，仅改变解码调度，以隔离这次改动的影响。统计布局与细分的墙钟耗时，不含 OpenGL、交换缓冲、显卡驱动及桌面合成，不能替代真实拖边测试。

| 页面 | 同步冷首帧 | 异步冷首帧 | 同步预热后 p95 | 异步预热后 p95 |
| --- | ---: | ---: | ---: | ---: |
| 触发 | 4.64 ms | 4.46 ms | 0.53 ms | 0.58 ms |
| 信号与声音 | 8.91 ms | 3.51 ms | 0.60 ms | 0.61 ms |
| 轮盘 | 9.83 ms | 5.04 ms | 0.66 ms | 0.69 ms |
| 表情库 | 5.19 ms | 4.95 ms | 2.45 ms | 2.55 ms |

结果支持“减少冷加载解码突刺”；预热后的布局耗时基本持平，不能解释原始桌面 trace 中约 2.4 秒的 eframe 长帧。该 eframe 指标包含绘制与 GL 调用，但排除了交换缓冲等待；不能直接归因为 vsync。生产 vsync 设置保持原值，未将尚未验证的显卡同步猜测作为修复。[汇总数据](resize-cpu-comparison.json)

可用 `LOLPING_UI_TEST_RESIZE_TRACE=<JSON 路径>` 记录真实窗口帧：包含上一帧 eframe 耗时和当前次 UI 绘制耗时，关闭设置时写出文件；正常运行不采样、不写日志。`previousEframeMs` 属于前一呈现帧，不能减去同一行的 `currentUiMs`；后者不包含 `App::logic`，egui 还可能在一帧内执行多次 UI pass。`LOLPING_UI_TEST_VSYNC=0/1` 仅供对照诊断。复现离屏基准使用 `cargo test --release --locked resize_layout_cpu_profile -- --ignored --nocapture`，测试专用 `LOLPING_UI_TEST_SYNC_THUMBNAILS=1` 切换同步对照，`LOLPING_UI_TEST_CPU_REPORT` 指定输出文件。

## 尚待完成

PNG 编码采用显式 `Compression::Fast`，保留无损质量。在本机 Windows / Core Ultra 5 125H 的 release 离屏测试中，各模式热身 3 次、采样 30 次：600×600 真实轮盘的默认 Balanced 编码中位数 22.60 ms、p95 27.95 ms，Fast 为 4.95 ms / 7.54 ms；400×400 信号的 Fast 为 1.48 ms / 2.76 ms。包含复制、反预乘、分配和压缩，不包含文件写入、Cairo 解码和合成，不能据此宣称端到端 60 FPS。[完整基准](wayland-png-encoding-benchmark.json)

真实 GNOME 46 原生虚拟会话将在同提交 Linux CI 产物上验证启动、原生帧、组合快捷键、释放/取消、暂停恢复、断开清理以及未知 D-Bus 发送者拒绝。这里只在结果确认后登记通过项。

Windows 实际拖边 A/B、Ubuntu 实机多屏/缩放/全屏/锁屏仍需单独验收。
