# 构建与发布

版本和工具均显式固定：Rust 1.96.0、fflate 0.8.2、NSIS 3.11、appimagetool 1.9.1、AppImage type2-runtime 20251108。外部打包工具的下载地址和 SHA-256 位于 `packaging/tools.json`，下载和缓存命中时都会检查哈希，不使用可变的 `latest` 或 `continuous` 资源。

NSIS 使用 Tauri 项目保存的固定版本镜像，校验值与 [NSIS 官方发布页](https://sourceforge.net/projects/nsis/files/NSIS%203/3.11/nsis-3.11.zip/download) 一致；AppImage 工具与运行时来自 AppImage 官方发布仓库。

## 平台环境

Windows：Visual Studio 2022 的 Desktop development with C++ 工作负载。NSIS 编译器由打包脚本自动下载并验证，不需要系统安装。安装器按当前用户安装，不要求管理员权限；覆盖升级和卸载会先通过 `--quit` 正常关闭同一用户的应用。

macOS：Apple Silicon、Xcode Command Line Tools。系统 `sips` / `iconutil` 生成图标，`codesign` 为 helper 和 app 执行 ad-hoc 签名，`hdiutil` 生成 DMG。没有开发者证书与公证时不要声称已签名分发或已公证。

Linux：Ubuntu 22.04 x64，安装以下依赖：

```sh
sudo apt-get install build-essential pkg-config libgtk-3-dev libasound2-dev libx11-dev libxi-dev libxtst-dev libxrandr-dev libxinerama-dev libxcursor-dev libxkbcommon-dev libxkbcommon-x11-dev libayatana-appindicator3-dev libgl1-mesa-dev libegl1-mesa-dev libfuse2 squashfs-tools desktop-file-utils xvfb xauth xcompmgr openbox dbus-x11
```

`.deb` 通过依赖声明使用系统库。AppImage 收集原生 GTK、ALSA、X11 与托盘库及其版权声明，保留系统 glibc 与显卡驱动；Linux 体积报告应把这些依赖打包成本单独说明。

## 命令与产物

```sh
npm ci --ignore-scripts
npm run build:resources
npm run test:helper
cargo test --locked
cargo build --release --locked
node scripts/package.mjs --stage-only
node scripts/standalone-smoke.mjs --no-desktop
npm run package
npm run verify:package
```

`ui/` 是自动生成的原生媒体目录，只有默认 9 个表情和信号素材，不含 HTML、JS、CSS。`.stage/` 是平台运行布局，`dist/` 是分发包、包验证报告、体积记录和 SHA-256 清单；这些生成文件不提交到 Git。

Windows 产物为用户安装器和免安装 ZIP；macOS 为 DMG；Linux 为 AppImage 和 Debian 包。扩展表情 ZIP 在三端构建时都产生，内容相同，发布时只保留一份并核验三端哈希一致。

Linux 的输入与透明覆盖层集成检查使用私有 Xvfb，不会操作用户桌面：

```sh
python3 native/hook-helper/tests/linux_x11_test.py
python3 scripts/test-linux-surface.py
```

完整桌面 smoke 需要可用的交互桌面。CI 的 Windows/macOS 使用 `--no-desktop`，Linux 在 Xvfb + 合成器中运行。macOS 辅助功能授权、全屏空间、真实 Linux 托盘兼容与 Windows 混合缩放仍需实机验收。

## 发布

先提交、push，等待 Windows、macOS、Linux 构建以及 `release-ready` 汇总任务全部通过。下载同一 commit 的 CI 产物，对照各平台 `SHA256SUMS-*.txt` 校验，再创建 `v0.1.0` Release。不要将后来本地重编译的文件替换该次 CI 产物。

随 Release 提供扩展包、SHA-256、第三方声明与实际性能报告。README 的内存降幅只写实测支持的结果，并说明旧版本、显示器、采样范围和 GPU 内存不计入的限制。

## 更新与卸载

Windows 安装器在覆盖升级和卸载前发送 `--quit`，等待主进程、设置及输入 helper 正常退出；卸载删除当前用户的开机自启项并保留用户配置与导入素材。macOS DMG、Linux AppImage 和 Debian 包管理器不会自动关闭用户会话中的进程：移除应用前先在设置关闭开机自启，再从托盘退出，然后删除应用或卸载 Debian 包。免安装版更新同样应先退出，再替换完整程序目录。
