<p align="center">
  <img src="https://raw.githubusercontent.com/AceyKubbo/oh-my-gitty/main/src/favicon.png" alt="Oh My Gitty" width="120" />
</p>

# Oh My Gitty 🐱

[![Windows CI](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/windows.yml/badge.svg)](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/windows.yml)
[![Version Bump](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/version-bump.yml/badge.svg)](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/version-bump.yml)
[![Release](https://img.shields.io/github/v/release/AceyKubbo/oh-my-gitty)](https://github.com/AceyKubbo/oh-my-gitty/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

简体中文 | [English](README.en.md)

**为单人 AI 开发者打造的轻量 Git 工具**：不用打开 IDE，就能集中管理仓库、生成提交、解决冲突、回退版本和启停开发服务器。基于 Tauri 2 + 原生前端，安装包约 5–8 MB；Git 操作走本机 CLI，仓库内容默认留在本机。

## 功能一览

| 场景 | 能做什么 |
| --- | --- |
| 总览与多仓库 | 仪表盘提供 KPI 卡片、近一年提交热力图、近 7 日代码量、项目类型分布与可搜索/排序/筛选的项目卡片；仓库可打开、克隆、拖拽添加、拖动排序，普通文件夹可一键 `git init`；打开非仓库文件夹时递归扫描子目录，把发现的 Git 仓库以类似子模块的折叠树加入（无需关注父仓库），侧栏实时显示分支、领先/落后与改动数，支持折叠、宽度记忆和「只看待提交项目」 |
| 子模块 / Worktree | 子模块树可展开导航，并提示父仓库 gitlink 指针是否已提交；打开任一工作树会自动带入同一仓库的全部 worktree，侧栏用角标区分主仓库与从属工作树 |
| 工作区与代码查看 | 冲突/暂存/更改三分组、文件树化展示（目录批量暂存/取消暂存/忽略）、行内 diff、单文件两步确认丢弃、完整 `.gitignore` 编辑器并自动取消跟踪已忽略文件；本地文件变化会自动刷新状态；右侧编辑器有「差异 / 代码 / 合并（三向）」三个视图，代码区可独立切换暗黑模式，宽度可拖拽 |
| 提交与 AI | 只提交已暂存内容；AI 流式生成 Conventional Commits（可编辑、可选直接提交或确认后提交、可追加自定义要求），支持配置自测；可用 OpenAI 兼容 API 或 Claude Code、Codex、Gemini、Qwen、Ollama、Crush、OpenCode 等本地 CLI；冲突文件支持单个或批量 AI 解决并自动完成合并 |
| 历史与分支 | 本地 + 远程提交合并时间线，展开可看改动文件并逐文件查看 diff 或该版本文件内容，确认后可回退到任意版本；分支可切换、合并、对比和逐条挑拣提交，冲突支持 AI 或手动解决后继续/终止 |
| 同步与工程版本 | 自动处理无上游分支、后台静默 fetch；支持 refspec 将当前分支推送到远程其他分支（可选 `--force-with-lease`）；一键识别并升级项目版本文件（package.json / Cargo.toml / pyproject.toml / pubspec.yaml / tauri.conf.json 等），patch/minor/major 多文件保持一致 |
| 运行与外观 | 运行面板自动识别启动命令与端口（npm/django/make/cargo/go…），应用内启停、逐行日志、外部运行检测与安全停止；深浅主题 + 7 种强调色自动适配明暗、高 DPI 自动缩放、窗口置顶与托盘常驻 |
| 更新与终端入口 | 启动自动检查更新，签名校验后一步安装（也可在「设置 → 关于与更新」手动检查）；`gitty` 命令可从任意目录打开/聚焦应用并切换到当前项目 |

## 快速开始

### 安装

从 [Releases](https://github.com/AceyKubbo/oh-my-gitty/releases) 下载：

| 平台 | 文件 |
| --- | --- |
| macOS（Apple Silicon / M 系列） | `Oh.My.Gitty_<版本>_aarch64.dmg` |
| Windows 10/11（x64） | `Oh.My.Gitty_<版本>_x64-setup.exe` |

macOS 仅支持 Apple Silicon，不再提供 Intel 版本。安装包尚未 Apple 公证 / 使用商业代码签名：首次启动 macOS 应用可能需右键选「打开」，Windows 的 SmartScreen 提示请确认来源后选择「仍要运行」。

### 终端命令 `gitty`

首次启动时自动安装到 PATH（macOS 优先 `/usr/local/bin`，Windows 写入 `%LOCALAPPDATA%\OhMyGitty\bin`）；安装后需重新打开终端。

```bash
cd /path/to/your/project
gitty                          # 打开/聚焦应用，并把当前目录加入项目列表并切换过去
gitty /path/to/another/project # 也可显式指定路径
```

### 从源码构建

需要 [Node.js](https://nodejs.org/) 18+ 和 [Rust](https://www.rust-lang.org/)；Windows 另需 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#windows) 中的 C++ Build Tools 与 WebView2。

```bash
git clone https://github.com/AceyKubbo/oh-my-gitty.git
cd oh-my-gitty
npm ci
npm run dev      # 开发模式，保存后自动重载
npm run build    # macOS 输出 .dmg，Windows 输出 NSIS setup.exe
```

打开应用后：选择本地仓库或克隆远程 → 在设置中选择 AI 接口或本地 CLI → 在工作区查看改动并提交 → 用「推送 / 拉取」同步 → 在「运行」面板启动开发服务器。

## 隐私与安全

- Git 操作通过本机 `git` CLI 完成，仓库内容默认只留在本机。
- API 模式会把相关 diff、近期提交或冲突文件发往你配置的 AI 接口；本地 CLI 模式把同类内容交给所选工具，是否上传取决于该工具提供方。请自行确认其隐私政策。
- AI API Key 只保存在本机配置；GitHub Token 仅用于创建远程仓库和推送，请勿提交或分享。
- 发现漏洞请按 [安全策略](SECURITY.md) 私密报告；问题与建议走 [Issues](https://github.com/AceyKubbo/oh-my-gitty/issues) 或邮件 `hello@lumifold.top`。

## 技术栈与结构

Tauri 2 + Rust 后端（封装系统 git CLI），原生 HTML/CSS/JS 前端、无打包器；AI 走 OpenAI 兼容接口或本地 CLI。UI 规范见 [设计系统](docs/ui-design-system.md)。

```
src/                前端静态资源，无需构建
src-tauri/src/      后端
  git.rs            git CLI 封装 + porcelain v2 解析
  ai.rs             AI 提交信息生成 / 冲突解决
  runner.rs         开发服务器进程托管 / 端口探测
  watcher.rs        仓库文件监听 / 状态自动刷新
  version.rs        项目版本文件识别与批量升级
  cli.rs            gitty 命令安装与目录参数
  config.rs         设置持久化
  lib.rs            Tauri 命令入口
```

## 注意

> ⚠️ 这是一个 100% 由 AI 开发的项目：从架构到实现、迭代与修复均由 AI 完成，人工只参与需求确认与验收。可能有未覆盖的边界或未经真实场景验证的功能，请谨慎用于生产环境并备份重要数据。遇到问题欢迎提 [Issue](https://github.com/AceyKubbo/oh-my-gitty/issues) 🐱

- 生成提交信息时，超过 60 KB 的 diff 会被截断；冲突文件超过 80 KB 需手动解决。
- 远端需要凭证时复用系统 git 凭据助手（macOS Keychain / Windows Credential Manager）；尊重仓库 hooks，不使用 `--no-verify`。

## 贡献与协议

欢迎 Issue 和 PR，请先阅读 [贡献指南](CONTRIBUTING.md)。项目采用 [MIT](LICENSE) 协议 © Bavoch。
