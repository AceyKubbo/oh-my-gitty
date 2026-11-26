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
| 总览与多仓库 | 仪表盘：KPI、提交热力图、代码量、项目分布、可搜索/筛选卡片；仓库打开/克隆/拖拽/排序、`git init`；递归发现子目录仓库；侧栏实时状态 +「只看待提交」 |
| 子模块 / Worktree | 子模块树导航 + gitlink 指针提示；同仓库 worktree 自动归组，主/从角标区分 |
| 工作区与代码查看 | 冲突/暂存/更改分组 + 目录批量操作；行内 diff、两步丢弃、`.gitignore` 编辑器；文件变化自动刷新；差异/代码/三向合并视图 + 独立暗黑模式 |
| 提交与 AI | 仅提交暂存内容；AI 流式生成 Conventional Commits（可编辑/自测/追加要求）；OpenAI 兼容 API + 多种本地 CLI；AI 解决冲突并自动合并 |
| 历史与分支 | 本地 + 远程时间线、逐文件 diff / 版本文件、确认回退；分支切换/合并/对比/挑拣，冲突可 AI 或手动解决 |
| 同步与工程版本 | 无上游自动处理 + 后台 fetch；refspec 推送（`--force-with-lease`）；一键批量升级多语言版本文件 |
| 运行与外观 | 启动命令/端口识别、应用内启停 + 实时日志；深浅主题 + 7 强调色、高 DPI、置顶与托盘 |
| 更新与终端入口 | 启动检查更新 + 签名校验安装；`gitty` 命令任意目录打开/聚焦并切换项目 |

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

- 生成提交信息时，超过 60 KB 的 diff 会被截断；冲突文件的 AI 处理上限默认 80 KB，可在「设置 → AI → AI 解决冲突文件大小上限」中按模型上下文窗口调整（8–2048 KB），超限文件需手动解决。调高上限后若模型上下文不足，AI 输出可能被截断，此时工具会拒绝写回并提示手动处理。
- 远端需要凭证时复用系统 git 凭据助手（macOS Keychain / Windows Credential Manager）；尊重仓库 hooks，不使用 `--no-verify`。

## 友链

- [LINUX DO](https://linux.do/) — 一个聚焦 AI 与技术的中文社区

## 贡献与协议

欢迎 Issue 和 PR，请先阅读 [贡献指南](CONTRIBUTING.md)。项目采用 [MIT](LICENSE) 协议 © Bavoch。
