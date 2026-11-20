---
name: desktop-build
description: 本地构建和验证 Cursor BYOK 桌面安装包。用于打包、生成 macOS DMG 或 Windows NSIS 安装包，以及排查本地 Tauri 打包失败。GitHub Release、发布标签和正式更新签名使用 release 技能。
---

# Cursor BYOK 本地打包

按用户要的系统、架构和产物执行本地构建。用户问“如何打包”时给出对应命令和产物路径。用户要求打包时运行构建并验证产物。

## 桌面安装包

1. 在仓库根目录确认当前平台、架构及 Node.js、npm、Cargo 和 Make 可用。macOS 可用 `xcode-select -p` 检查 Xcode 工具链。Linux 的系统依赖见[发布工作流](../../../.github/workflows/release.yml)中的 Linux 依赖安装步骤。
2. 首次构建、前端依赖缺失或锁文件变更时，使用 `npm ci` 安装锁定的版本。使用仓库的 Make 目标构建当前平台的安装包。

   ```bash
   npm --prefix apps/desktop ci
   make build-desktop
   ```

3. [Makefile](../../../Makefile) 会生成或复用 `.tauri/cursor-byok.local.key` 来签名本地更新产物，Windows 分支明确使用 NSIS。Tauri 根据[项目配置](../../../apps/desktop/src-tauri/tauri.conf.json)自动构建前端并编译内嵌的 Rust 服务。Apple Silicon 原生工具链默认构建 arm64。
4. 构建结束后，在 Cargo 目标目录的 `release/bundle/` 下核对本次生成的文件。默认目标目录为仓库根目录的 `target/`，如果环境覆盖了它，用 `cargo metadata --no-deps --format-version 1` 查看 `target_directory`。指定目标三元组时，产物位于目标目录的 `<target>/release/bundle/`。

| 平台 | 默认安装包位置 |
| --- | --- |
| macOS | `target/release/bundle/dmg/*.dmg`，应用包在 `target/release/bundle/macos/*.app` |
| Windows | `target/release/bundle/nsis/*-setup.exe` |
| Linux | `target/release/bundle/` 下对应的 `deb/`、`rpm/` 或 `appimage/` 目录 |

确认构建命令成功退出，目标安装包存在且非空。报告平台、架构和产物的绝对路径。失败时定位报错对应的依赖安装、前端构建、Rust 编译、系统打包或签名阶段，从该阶段处理问题。`make -n build-desktop` 可核对 Make 将执行的命令，实际产物仍须通过完整构建验证。

## 独立构建目标

用户只需要某个组件时，使用 [Makefile](../../../Makefile) 的对应目标。

| 产物 | 命令 | 默认输出 |
| --- | --- | --- |
| Rust 服务 | `make build-server` | `target/release/cursor-server`（Windows 带 `.exe`） |
| 管理前端 | `make build-web` | `apps/desktop/dist/` |

GitHub Release、多平台 CI 发布和正式更新签名按 [release 技能](../release/SKILL.md)处理。
