<p align="center">
  <img src="https://raw.githubusercontent.com/AceyKubbo/oh-my-gitty/main/src/favicon.png" alt="Oh My Gitty" width="120" />
</p>

# Oh My Gitty 🐱

[![Windows CI](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/windows.yml/badge.svg)](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/windows.yml)
[![Version Bump](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/version-bump.yml/badge.svg)](https://github.com/AceyKubbo/oh-my-gitty/actions/workflows/version-bump.yml)
[![Release](https://img.shields.io/github/v/release/AceyKubbo/oh-my-gitty)](https://github.com/AceyKubbo/oh-my-gitty/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[简体中文](README.md) | English

**A lightweight Git tool for solo AI developers**: manage repositories, generate commits, resolve conflicts, reset versions and run dev servers without opening an IDE. Built with Tauri 2 and a native frontend, ~5–8 MB installer; Git operations use your local CLI and repository content stays on your machine by default.

## Features at a glance

| Area | What you can do |
| --- | --- |
| Overview & repos | Dashboard: KPIs, commit heatmap, code volume, project breakdown, searchable/filterable cards; open/clone/drag/reorder repos, `git init`; recursive sub-repo discovery; live sidebar + "dirty only" filter |
| Submodules / worktrees | Submodule tree + gitlink pointer hint; worktrees of the same repo grouped automatically, main/linked badges |
| Workspace & code view | Conflicts/staged/changes with batch directory actions; inline diffs, two-step discard, `.gitignore` editor; auto status refresh; diff/code/three-way merge views + independent dark mode |
| Commits & AI | Stage-only commits; AI-streamed editable Conventional Commits (self-test/extra instructions); OpenAI-compatible APIs + local CLIs; AI conflict resolution with auto merge |
| History & branches | Local + remote timeline, per-file diffs / revision content, confirmed reset; branch switch/merge/compare/cherry-pick, AI or manual conflict handling |
| Sync & project versions | Upstream-less branch handling + silent fetch; refspec push (`--force-with-lease`); one-click batch bump of version files |
| Run & appearance | Command/port detection, in-app start/stop + live logs; dark/light themes with 7 accents, high DPI, always-on-top and tray |
| Updates & terminal | Startup update check + signature-verified install; `gitty` command opens/focuses the app from any directory |

## Quick start

### Install

Download from [Releases](https://github.com/AceyKubbo/oh-my-gitty/releases):

| Platform | File |
| --- | --- |
| macOS (Apple Silicon / M series) | `Oh.My.Gitty_<version>_aarch64.dmg` |
| Windows 10/11 (x64) | `Oh.My.Gitty_<version>_x64-setup.exe` |

macOS supports Apple Silicon only; Intel builds are no longer provided. The packages are not Apple-notarized / commercially code-signed yet: on first launch you may need to right-click the macOS app and choose Open, and Windows SmartScreen prompts should be confirmed against this repository before choosing Run anyway.

### Terminal command `gitty`

Installed into PATH on first launch (macOS: `/usr/local/bin` first; Windows: `%LOCALAPPDATA%\OhMyGitty\bin`). Reopen your terminal afterwards.

```bash
cd /path/to/your/project
gitty                          # open/focus the app, add the current directory and switch to it
gitty /path/to/another/project # or pass an explicit path
```

### Build from source

Requires [Node.js](https://nodejs.org/) 18+ and [Rust](https://www.rust-lang.org/). On Windows, also install the C++ Build Tools and WebView2 from the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#windows).

```bash
git clone https://github.com/AceyKubbo/oh-my-gitty.git
cd oh-my-gitty
npm ci
npm run dev      # development mode, auto-reloads on save
npm run build    # macOS: .dmg; Windows: NSIS setup.exe
```

After launching: pick a local repository or clone a remote → choose an AI API or local CLI in Settings → review changes in the workspace and commit → sync with Push / Pull → start a dev server in the Run panel.

## Privacy & security

- Git operations run through your local `git` CLI; repository content stays on your machine by default.
- API mode sends relevant diffs, recent commits or conflicted files to the configured AI endpoint. Local CLI mode passes the same content to the selected tool; whether it uploads data depends on that provider. Review its privacy policy.
- The AI API key is stored only in local config; the GitHub token is used solely to create remote repositories and push — never commit or share it.
- Report vulnerabilities privately via the [security policy](SECURITY.md); use [Issues](https://github.com/AceyKubbo/oh-my-gitty/issues) or email `hello@lumifold.top` for feedback.

## Tech stack & structure

Tauri 2 with a Rust backend wrapping the system git CLI, and a native HTML/CSS/JS frontend with no bundler. AI comes from OpenAI-compatible APIs or local CLIs. See the [design system](docs/ui-design-system.md) for UI conventions.

```
src/                Frontend static assets, no build step
src-tauri/src/      Backend
  git.rs            git CLI wrapper + porcelain v2 parsing
  ai.rs             AI commit messages / conflict resolution
  runner.rs         Dev server supervision / port probing
  watcher.rs        Repository watching / automatic status refresh
  version.rs        Project version detection & batch bump
  cli.rs            `gitty` command setup and directory arguments
  config.rs         Settings persistence
  lib.rs            Tauri command entry point
```

## Note

> ⚠️ This project is 100% AI-developed: architecture, implementation, iteration and fixes are all done by AI; humans only confirm requirements and accept results. It may contain uncovered edge cases or features without thorough real-world testing — use with caution in production and back up important data. Please open an [Issue](https://github.com/AceyKubbo/oh-my-gitty/issues) if you find a problem 🐱

- Diffs over 60 KB are truncated before being sent for AI commit messages; the AI conflict-file limit defaults to 80 KB and is configurable under Settings → AI → AI conflict file size limit (8–2048 KB) to match your model's context window; larger files need manual resolution. If the limit exceeds the model's context, AI output may be truncated — in that case the tool refuses to write it back and asks for manual resolution.
- Remote credentials are delegated to the system git helper (macOS Keychain / Windows Credential Manager); repository hooks are respected and `--no-verify` is never used.

## Friendly links

- [LINUX DO](https://linux.do/) — a Chinese community focused on AI and technology

## Contributing & license

Issues and PRs are welcome; please read the [contributing guide](CONTRIBUTING.en.md) first. Licensed under [MIT](LICENSE) © Bavoch.
