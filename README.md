# LOLPing Lite

把信号轮盘和表情带到桌面，保持轻巧。

LOLPing Lite 是 [lolPing](https://github.com/darrenprx/lolPing) 的本地精简版，基于已经验证的 [DBX 原生插件](https://github.com/Ezreal-byte/dbx-lolping-plugin) 独立实现。覆盖层、动画、音效和设置界面均使用原生进程，不依赖 DBX、Electron 或 WebView2。

设置使用 Rust + egui/Glow 绘制；打开时启动单独的设置进程，关闭即释放界面、字体与图形资源。后台只保留打信号所需的核心与输入监听。

## 下载

从 [GitHub Releases](https://github.com/Ezreal-byte/lolping-lite/releases) 获取安装包。

| 平台 | 下载文件 | 要求 |
| --- | --- | --- |
| Windows x64 | `LOLPing-Lite-版本-windows-x64-Setup.exe` 或 `.zip` | Windows 10 / 11；安装器按当前用户安装 |
| macOS Apple Silicon | `LOLPing-Lite-版本-macos-arm64.dmg` | macOS 13+；需辅助功能授权 |
| Linux x64 | `LOLPing-Lite-版本-linux-x64.AppImage` 或 `.deb` | Ubuntu 22.04 / glibc 2.35+；X11 与桌面合成器 |

Windows 安装器尚未商业签名，macOS 使用 ad-hoc 签名、尚未公证。macOS 下载后按系统提示在“系统设置 → 隐私与安全性”允许打开，并给应用授予辅助功能权限。Linux AppImage 首次运行需添加可执行权限；AppImage 不自带显卡驱动。

**Wayland 暂不支持全局手势。** 应用会明确提示，不会将 XWayland 局部能力当作完整支持。macOS 全屏空间和不同 Linux 桌面环境仍需真实设备验收；CI 通过不能替代这些检查。

## 使用

- 启动后驻留托盘，首次自动打开设置。再次启动会打开已有实例的设置。
- 默认按住 `Alt`，鼠标左键拖动选择信号；按住 `Ctrl` 拖动选择表情。默认 `Ctrl + Alt + P` 暂停或恢复。
- 可在设置中调整触发键、快捷键、轮盘布局、大小、持续时间、音量和语言；支持自定义图片与声音。
- 关闭设置后仍能打信号；从托盘打开设置、暂停／恢复或退出。开机自启默认关闭。
- 主包内置 9 个默认表情，其他 35 个下载 `lolping-extra-emotes-1.zip` 后在设置中导入。扩展包不降低原素材质量。
- 应用仅在本机工作，不包含房间、网络中继、成员同步或自动下载更新器。

配置和导入素材保存在独立用户数据目录；不会自动读写 DBX 插件的数据。Windows 卸载保留这些设置与素材，便于下次安装恢复。

## 轻量设计

- 原生绘制，不启动浏览器渲染进程。
- 设置按交互重绘，关闭设置后结束其子进程。
- 动画与图像缓存有上限；空闲时回收覆盖层缓冲。
- 延续 DBX 插件 0.2.2 的屏幕边缘修复，避免边缘触发后后续轮盘残缺或消失；保留原来的光标位置和手势判断。

**Windows 同机实测，后台工作集从 448.9 MiB 降至 45.0 MiB，降低 90.0%。** 相比旧 WebView 插件 0.1.1，原生版超过了“后台内存降低约 80%”的优化目标。打开原生设置约 164.4 MiB，关闭后回到约 44.6 MiB，让界面资源随用随释放。

| 场景 | 旧 WebView 插件 0.1.1 | LOLPing Lite 0.1.0 |
| --- | ---: | ---: |
| 后台空闲 | 448.9 MiB | **45.0 MiB** |
| 连续信号（5 次/秒） | 512.7 MiB | **47.4 MiB** |
| 动态表情 | 591.3 MiB | **49.7 MiB** |

上述为 Windows 11、Core Ultra 5 125H、2880×1800 / 150% 缩放下，各重复三次的完整应用进程树工作集中位数。两方案静音并通过实际渲染器的预览接口产生相同负载；旧版不计共享 DBX 宿主，GPU 内存也不计入。它们不是峰值上限，也不保证其他机器或平台得到同一结果。详细 CPU、私有提交、方法与匿名原始数据见 [实测报告](docs/performance-results-windows.md) 和 [测试说明](docs/performance.md)。

**Windows 免安装 ZIP 与 macOS DMG 约 21 MiB**，原版 Electron [v0.5.1](https://github.com/darrenprx/lolPing/releases/tag/v0.5.1) 的对应安装包约为 158 / 175 MiB。主包只内置 9 个默认表情，其余 35 个为约 39.67 MiB 的无损离线扩展包；这项体积对比包含资源范围变化，不能全部归因于渲染框架。Linux AppImage 另含 GTK 等依赖，大小单独统计。各包准确大小以 Release 为准。

测试通过范围与待完成的实机项目见 [验收记录](docs/validation.md)。

## 本地构建

需要 Rust **1.96.0**、Node.js **22**（仅用于开发与打包）以及平台 C++ 工具链。应用运行不需要 Node.js。

```sh
npm ci --ignore-scripts
npm run build:resources
npm run test:helper
cargo test --locked
cargo build --release --locked
npm run package
npm run verify:package
```

构建步骤、平台依赖及发布检查见 [打包文档](docs/building.md)。三端 GitHub Actions 会分别生成安装包与 SHA-256 清单，通过全部校验后再发布同一提交的 CI 产物。

## 来源与许可

- 独立版本来源：[Ezreal-byte/dbx-lolping-plugin](https://github.com/Ezreal-byte/dbx-lolping-plugin)，固定导入提交 [`5d9eaf15e88f82586a5fc0d958bc6f77a10f7157`](https://github.com/Ezreal-byte/dbx-lolping-plugin/commit/5d9eaf15e88f82586a5fc0d958bc6f77a10f7157)。
- 原始项目：[darrenprx/lolPing](https://github.com/darrenprx/lolPing)，固定上游提交 [`b947aab8d379015687117544191160bd82141954`](https://github.com/darrenprx/lolPing/commit/b947aab8d379015687117544191160bd82141954)。感谢原作者的设计、资源整理和输入处理实现。
- 软件代码遵循 [MIT](LICENSE)。游戏图片、动画和音效属于其各自权利人，不属于 MIT 授权范围；本项目非 Riot Games 官方产品。
- 第三方组件及字体说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 与 `licenses/`。资源原始哈希记录在 `resources/upstream.json`。
