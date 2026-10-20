# Ubuntu 24.04 / GNOME 46 Wayland

本分支新增 GNOME Wayland 桥接。设置、轮盘、信号、表情动画与音频仍由原生 Rust 进程处理；GNOME Shell 扩展负责桌面合成、全局组合键以及轮盘打开期间的临时输入抓取。不启动 X11 输入 helper，不使用 WebView、root、`/dev/input` 或屏幕录制权限。

## 安装与启动

1. 在 Ubuntu 24.04 的 GNOME Wayland 会话中启动本分支构建的应用。
2. 在设置里的 Wayland 提示卡点击“安装 GNOME 扩展”。只安装到当前用户目录，不需要管理员权限。
3. 首次安装或升级扩展后，按提示注销并重新登录。不要在 Wayland 下尝试用 `Alt+F2 → r` 重启 Shell。
4. 在 GNOME“扩展”应用中确认 **LOLPing Lite Wayland Bridge** 已启用，再打开 LOLPing Lite 点击“重试启动”。

也可以手动安装包中的 `LOLPing-Lite-<版本>-gnome46.shell-extension.zip`：

```sh
gnome-extensions install --force ./LOLPing-Lite-<版本>-gnome46.shell-extension.zip
# 首次安装后注销并重新登录，再运行：
gnome-extensions enable lolping-lite@ezreal-byte.github.io
```

用户级安装路径为 `~/.local/share/gnome-shell/extensions/lolping-lite@ezreal-byte.github.io`。卸载桥接可运行 `gnome-extensions uninstall lolping-lite@ezreal-byte.github.io`。应用启动不会自动安装或修改扩展；必须主动点击设置按钮。

## Wayland 操作方式

| 操作 | GNOME Wayland 快捷键 |
| --- | --- |
| 信号轮盘 | 按住 `Super + Alt + Q`，移动鼠标选择，释放 `Q` 确认 |
| 表情轮盘 | 按住 `Super + Alt + E`，移动鼠标选择，释放 `E` 确认 |
| 另一种确认方式 | 轮盘打开后点击鼠标左键 |
| 取消 | `Esc`、鼠标右键或在轮盘中心释放 |
| 暂停／恢复 | `Super + Alt + P` |

`Super` 通常是 Windows 标志键。快捷键冲突会明确报错，不会覆盖系统或其他扩展的绑定。轮盘以外的信号与表情显示不抢焦点、不接收鼠标；最长 15 秒后自动取消未完成的轮盘。

GNOME 不允许普通应用以原来的方式全局监听 `Alt/Ctrl + 鼠标拖动`。Wayland 设置页因此显示实际生效的组合键，隐藏原触发键、拖动阈值及触发键单击选项；原配置保留，回到 X11/Windows/macOS 后仍使用原手势。表情可单独关闭，轮盘编辑、音量、持续时间、本地素材导入和真实预览仍使用同一套设置。

## 支持范围

- 首个目标是 **Ubuntu 24.04 / GNOME Shell 46**。扩展元数据仅声明 GNOME 46，其他版本不会假装兼容。
- KDE Plasma、sway、其他 Wayland 合成器还没有此桥接；不能安装 GNOME 扩展来代替它们的原生适配。
- X11 继续使用现有 C++ 输入 helper 和透明 Cairo 覆盖层。
- 当前每个显示器合并传输一块覆盖区域，限制为单边 4096 物理像素、32 MiB。超宽屏或高缩放下，在相距很远的位置同时显示多个信号可能超过限制并停止引擎，需要点击启动重试；尚未实现大画面的分块传输。本测试版不能据普通分辨率的结果声称已覆盖这一场景。
- 锁屏、概览、休眠、显示器变更和扩展禁用都会释放输入抓取并清理覆盖层。GNOME 在锁屏或重载时可能卸载用户扩展；若应用显示连接断开，重新启用扩展并点击启动。
- 目前的实现与测试范围记录在 [排查与验证记录](resize-wayland-validation.md)。隔离的原生虚拟 Wayland 桌面测试不能替代真实显示器、显卡、锁屏和全屏游戏验收。

## 实现与资源边界

控制和输入事件走用户会话 D-Bus。扩展绑定一个唯一应用发送者；其他发送者无权绘制或控制，不提供任意 JavaScript 执行、输入注入、任意文件读取或网络端口。

Rust 将绘制结果无损编码为 PNG，写到自身 `XDG_RUNTIME_DIR` 下的私有目录，目录权限 `0700`、文件 `0600`。Shell 校验路径、文件类型、所有者、长度及固定 33 字节 PNG 头，再由原生 Cairo 读取图片；解码后再次核验尺寸、权限及文件长度。每个显示器最多一帧在传输中，原生解码确认后才能重写。解码后的帧限制为 4096 像素边长与 32 MiB，编码文件另允许 64 KiB 格式开销。

每个显示器复用一个 `St.DrawingArea`，只保留当前原生 Cairo 图像；替换或清理时显式 `finish()` 释放旧图像的像素，每次重绘立即释放临时绘图上下文。大块像素不转换为 JavaScript 数组，也不依赖其垃圾回收释放。这避免了 Ubuntu 24.04 默认 GJS 1.80.2 的 [incremental GC 回调错误](https://github.com/GNOME/gjs/commit/4e53510fbcf9472c4c1fa43661a7f409a012511d) 所对应的逐帧大数组分配路径，不要求用户替换系统 GJS。

Rust 在空闲、停止或正常退出时释放自身绘制缓冲与临时文件，Shell 同时移除不再使用的覆盖层。主进程异常退出后，Shell 根据 D-Bus owner 消失释放所有输入与覆盖层；残留临时文件由登录会话的运行目录生命周期清理。帧传输协议现为 2；如果安装过本分支早期测试扩展，需要在设置中重新安装，并按提示注销重登录。

本次不把此前 Windows 的内存数字当作 Wayland 保证值。GNOME compositor 内的纹理也会占用资源，需要在真实 Ubuntu 设备上独立测量。

当前 `wayland-preview` 是测试候选：三端构建／打包通过，Ubuntu 24.04 虚拟 GNOME 已显示超过 300 帧；普通输入基线未完成，完整集成工作流没有通过。软件渲染环境下 Shell RSS 在 308 帧后增长约 155 MiB，原因仍待定位，不能宣称内存稳定。用户将进行 Ubuntu 真机验收，测试时请同时观察 LOLPing Lite 与 GNOME Shell。详情与源提交见上述排查记录。

官方机制参考：[GNOME Shell modal 与生命周期](https://github.com/GNOME/gnome-shell/blob/gnome-46/js/ui/main.js)、[GNOME Shell 快捷键与扩展 D-Bus](https://github.com/GNOME/gnome-shell/blob/gnome-46/js/ui/shellDBus.js)、[GNOME 扩展安装说明](https://gjs.guide/extensions/development/creating.html)。
