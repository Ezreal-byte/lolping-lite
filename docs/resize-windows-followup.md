# Windows 设置窗口缩放：后续验证

截至 2026-10-20，已将缩略图解码移出 UI 线程，并限制解码队列、纹理上传和缓存。离屏对照支持“减少素材冷加载突刺”，**尚未证明修复了实测约 2.4 秒的窗口缩放长帧**。本轮不调整生产 vsync，也不关闭无障碍支持。已有 Windows 预览包供用户手动复测。

## 已确认

- 原始本地记录 `.test-data/resize/vsync-1.json` 有 944 次采样、218 次尺寸变化；eframe 耗时中位数 5.75 ms、P95 8.85 ms，5 次超过 100 ms，其中 3 次约 2.4 秒。该原始文件留在测试机器，未纳入仓库。
- 三次长帧分别为 2313.600、2436.890、2369.503 ms，占相邻采样间隔的 99.4%–99.8%。eframe 的计时明确暂停于 OpenGL context 切换和 `swap_buffers`，因此不能归因为单纯等待 vsync 或下一条 resize 事件。计时仍包含 UI、平台输出及 GL 调用，不是线程 CPU 使用率。
- 该次采样由 `.test-data/resize/launch.mjs` 单独启动 `--settings`，通过模拟管道提供 `stopped` 状态；没有应用主进程、输入 helper、音频或信号覆盖层参与。这个结论仅适用于该次采样。
- 设置 IPC 的输入解析、输出序列化和管道 flush 在工作线程；UI 只发送到无界队列并非阻塞地接收结果。拖动窗口边缘本身不会提交设置写入。文件选择也在工作线程，仅按钮触发。
- 字体只在创建设置界面时安装；窗口尺寸改变本身不重建字体。WGL surface 的 `resize` 方法为空，不能解释为应用每次重新分配整窗画布。

| 证据 | 固定版本源码位置 |
| --- | --- |
| IPC、非阻塞接收、文件选择 | `backend/src/ui.rs:107`、`:123`、`:209`、`:314`、`:402` |
| 应用逻辑与界面计时边界 | `backend/src/ui.rs:594`、`:644`；`backend/src/ui/resize_trace.rs:22` |
| context、swap 排除及最终计时 | `eframe-0.37.0/src/native/glow_integration.rs:792`、`:865`、`:936`、`:964` |
| 平台输出、三角化、GL 绘制 | 同文件 `:858`、`:861`、`:882` |
| AccessKit 更新锁与同步 UIA 通知 | `accesskit_windows-0.35.1/src/adapter.rs:484`、`:643`；`accesskit_winit-0.34.1/src/platform_impl/windows.rs:42` |
| 避免跨线程窗口标题查询的既有处理 | `accesskit_windows-0.35.1/src/util.rs:384` |

依赖源码位于 Cargo 缓存 `registry/src/<registry>/`；以上行号对应当前锁定版本。CPU 对照结果见 [本轮验证记录](resize-wayland-validation.md) 和 [对照数据](resize-cpu-comparison.json)。

## 尚未确定

现有长帧记录没有分阶段时间，不能区分应用逻辑、OpenGL 驱动调用、Windows 平台调用或 AccessKit 同步通知／树锁。AccessKit 的窗口标题读取已使用避免消息往返的实现，不能直接套用旧式跨线程 `GetWindowText` 死锁猜测。剪贴板不在普通 resize 路径，且其显式重试等待约为 25 ms；没有此次卡顿由剪贴板触发的证据。

上游 [winit #4708](https://github.com/rust-windowing/winit/issues/4708) 报告的是持续重绘时的标题栏移动卡顿，不能当作本次窗口边缘缩放的已确认根因。无障碍按需激活也意味着测试时是否有 UIA 客户端连接可能影响路径，但这仍需记录验证。[AccessKit 维护者说明](https://github.com/AccessKit/accesskit/discussions/502)

## 下一次用户手动采样

在现有 Windows 免安装预览包目录打开 PowerShell。以下命令创建独立测试数据目录，不读取正式版配置；保持默认 vsync，不设置 `LOLPING_UI_TEST_VSYNC`。

```powershell
$resizeTest = Join-Path $env:TEMP ('lolping-resize-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $resizeTest | Out-Null
$env:LOLPING_UI_TEST_RESIZE_TRACE = Join-Path $resizeTest 'resize.json'
& .\lolping-lite.exe --data-dir (Join-Path $resizeTest 'data')
```

1. 保持同一显示器，先在“触发”页拖动四边／四角，再在素材已加载的“轮盘”或“表情”页重复。记录卡顿页签、约发生在启动后第几秒、拖动方向及是否跨显示器；不要同时导入素材或打开文件选择器。
2. 记录 Windows 版本、GPU／驱动版本、显示器数量、各显示器缩放与刷新率，以及当时是否运行屏幕阅读器、UI 检查或桌面自动化工具。无需关闭用户所需的无障碍功能。
3. 正常关闭设置窗口，再从托盘退出这个测试实例。记录只在设置进程正常关闭时写入；强制结束可能没有文件。提供 `resize.json` 和上述场景信息。下一轮使用新的测试目录，避免覆盖记录。

当前新增 trace 有 `currentUiMs`，只覆盖 `App::ui` 中的绘制布局和对话框，不包含 `App::logic`、三角化或 GL。`previousEframeMs` 对应上一呈现帧，不能减去同一行的 `currentUiMs`；egui 多 pass 也可能产生多条 UI 记录。

若仍复现，下一步最小增量是记录 `logicMs`、`cumulative_frame_nr`、`current_pass_index`、`pixels_per_point`，以及 UI pass 内 `ctx.has_accesskit_node(egui::accesskit_root_id())` 的布尔值。这些字段用于精确关联阶段和确认无障碍树是否激活，**本轮尚未加入**，也不改变生产功能。
