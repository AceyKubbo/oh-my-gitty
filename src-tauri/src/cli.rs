//! `gitty` 命令行入口支持。
//!
//! 分两部分:
//! 1. 应用侧解析 `--gitty-open-dir <目录>` 参数,把该目录加入仓库列表并设为当前项目;
//!    已运行的实例通过事件通知前端切换(配合 single-instance 插件)。
//! 2. 首次启动时把 `gitty` 启动脚本写入 PATH,做到安装后开箱即用(无需手动配置)。
//!    脚本把调用者的当前目录作为 `--gitty-open-dir` 参数传给应用。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tauri::Emitter;

/// 启动脚本传给应用的参数标记。
pub const OPEN_DIR_FLAG: &str = "--gitty-open-dir";
/// 通知已运行的前端切换到指定目录的事件名。
pub const OPEN_REPO_EVENT: &str = "gitty-open-repo";

/// 从参数序列里解析要打开的目录。
///
/// 取标记后**最后一个**存在的目录:脚本会先传当前目录 `"$PWD"`/`"%CD%"`,
/// 用户再附带 `gitty <path>` 时,显式路径排在后面,可覆盖当前目录。
pub fn open_dir_from_args<I, S>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut after_flag = false;
    let mut found: Option<PathBuf> = None;
    for arg in args {
        let s = arg.as_ref();
        if !after_flag {
            if s.to_str() == Some(OPEN_DIR_FLAG) {
                after_flag = true;
            }
            continue;
        }
        let candidate = PathBuf::from(s);
        if candidate.is_dir() {
            found = Some(candidate);
        }
    }
    found
}

/// 从当前进程参数解析(首次启动时使用;`args_os` 避免非 UTF-8 路径 panic)。
pub fn open_dir_from_env() -> Option<PathBuf> {
    open_dir_from_args(std::env::args_os().skip(1))
}

/// 把目录加入仓库列表、设为当前项目,并通知前端切换。
///
/// 返回是否成功。首次启动时事件可能在前端监听就绪前发出而丢失,
/// 但配置已落盘,前端 `settings_load` 会据此选中对应项目。
pub fn open_repo(app: &tauri::AppHandle, dir: &Path) -> bool {
    let Ok(canonical) = dir.canonicalize() else {
        return false;
    };
    let mut path = canonical.to_string_lossy().to_string();
    // Windows canonicalize 会返回 `\\?\C:\...` 形式,去掉前缀以免 git 命令拿到非法路径
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{rest}");
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        path = rest.to_string();
    }
    let store = crate::config::SettingsStore::new(app);
    let mut settings = store.load();
    // 从 worktree 中打开时,自动把同一仓库的其它 worktree 一并带入项目列表
    crate::expand_repo_paths(&mut settings.repos, &path);
    settings.last_repo = Some(path.clone());
    if store.save(&settings).is_err() {
        return false;
    }
    let _ = app.emit(OPEN_REPO_EVENT, path);
    true
}

/// 安装 `gitty` 启动脚本。开发模式下跳过(临时二进制不应成为系统命令)。
///
/// 每次启动都会重写脚本,确保应用搬迁/升级后脚本里的路径始终指向当前可执行文件。
pub fn ensure_installed() {
    if cfg!(debug_assertions) {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    #[cfg(target_os = "macos")]
    install_macos(&exe);

    #[cfg(target_os = "windows")]
    install_windows(&exe);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = exe;
}

/* ===== macOS ===== */

#[cfg(target_os = "macos")]
fn install_macos(exe: &Path) {
    let script = format!(
        "#!/bin/sh\n# Oh My Gitty CLI · 本文件由应用自动生成，请勿手动修改\nnohup \"{}\" {} \"$PWD\" \"$@\" >/dev/null 2>&1 &\n",
        shell_escape(&exe.to_string_lossy()),
        OPEN_DIR_FLAG,
    );

    let local_bin = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/bin"));

    // 优先 /usr/local/bin(常规 shell 默认在 PATH 中);无写权限时退到 ~/.local/bin
    if let Ok(()) = write_executable(&PathBuf::from("/usr/local/bin/gitty"), script.as_bytes()) {
        return;
    }
    if let Some(dir) = local_bin {
        let target = dir.join("gitty");
        if write_executable(&target, script.as_bytes()).is_ok() {
            ensure_path_in_shell_profiles();
        }
    }
}

/// 写入可执行文件(含 0755 权限),父目录不存在时创建。
#[cfg(unix)]
fn write_executable(path: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

/// 双引号 shell 字符串转义:反斜杠、双引号、美元符、反引号。
#[cfg(target_os = "macos")]
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// 兜底目录 ~/.local/bin 不一定在 PATH:给常见 shell 配置补一行 PATH 导出。
#[cfg(target_os = "macos")]
fn ensure_path_in_shell_profiles() {
    const SNIPPET: &str = "\n# Oh My Gitty CLI\nexport PATH=\"$HOME/.local/bin:$PATH\"\n";
    const MARK: &str = ".local/bin";
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    for name in [".zshrc", ".bash_profile", ".profile"] {
        let path = home.join(name);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.contains(MARK) {
            continue;
        }
        std::fs::write(&path, format!("{existing}{SNIPPET}")).ok();
    }
}

/* ===== Windows ===== */

/// 解析 `reg query HKCU\Environment /v Path` 的输出,取 REG(_EXPAND)_SZ 后的完整值。
/// 不能按空白切分:PATH 里常含 `C:\Program Files\...` 这类带空格的路径。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_reg_path(output: &str) -> String {
    for line in output.lines() {
        for key in ["REG_EXPAND_SZ", "REG_SZ"] {
            if let Some(i) = line.find(key) {
                return line[i + key.len()..].trim().to_string();
            }
        }
    }
    String::new()
}

#[cfg(target_os = "windows")]
fn install_windows(exe: &Path) {
    let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let dir = PathBuf::from(local_app_data).join("OhMyGitty").join("bin");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let script = format!(
        "@echo off\r\nREM Oh My Gitty CLI · 本文件由应用自动生成，请勿手动修改\r\nstart \"\" \"{}\" {} \"%CD%\" %*\r\n",
        exe.to_string_lossy(),
        OPEN_DIR_FLAG,
    );
    if std::fs::write(dir.join("gitty.cmd"), script).is_err() {
        return;
    }
    add_dir_to_user_path(&dir);
}

/// 把安装目录追加到用户级 PATH(写注册表 `HKCU\Environment`,无需管理员权限)。
/// 已存在则不重复添加;仅在追加后写回,避免污染原有系统 PATH。
#[cfg(target_os = "windows")]
fn add_dir_to_user_path(dir: &Path) {
    let query = crate::process::command("reg")
        .args(["query", "HKCU\\Environment", "/v", "Path"])
        .output();
    let mut current = match query {
        Ok(out) if out.status.success() => {
            parse_reg_path(&String::from_utf8_lossy(&out.stdout))
        }
        _ => String::new(),
    };

    let addition = dir.to_string_lossy();
    if current.to_lowercase().contains(&addition.to_lowercase()) {
        return;
    }
    if !current.is_empty() && !current.ends_with(';') {
        current.push(';');
    }
    current.push_str(&addition);
    let _ = crate::process::command("reg")
        .args([
            "add",
            "HKCU\\Environment",
            "/v",
            "Path",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
            &current,
            "/f",
        ])
        .output();
}

/* ===== 单元测试 ===== */

#[cfg(test)]
mod tests {
    use super::open_dir_from_args;

    #[test]
    fn parses_open_dir_after_flag() {
        let dir = std::env::temp_dir();
        let args = [
            "/path/to/oh-my-gitty",
            "--gitty-open-dir",
            dir.to_str().unwrap(),
        ];
        assert_eq!(open_dir_from_args(args.iter()), Some(dir));
    }

    #[test]
    fn last_directory_wins() {
        let base = std::env::temp_dir().join(format!("ohmygitty-cli-{}", std::process::id()));
        let sub = base.join("sub");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&sub).unwrap();
        let args = vec![
            "oh-my-gitty".to_string(),
            "--gitty-open-dir".to_string(),
            base.to_string_lossy().to_string(),
            sub.to_string_lossy().to_string(),
        ];
        assert_eq!(open_dir_from_args(args.iter()), Some(sub));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn ignores_missing_flag_or_nonexistent_dirs() {
        let args = ["oh-my-gitty", "/Users/me/project"];
        assert_eq!(open_dir_from_args(args.iter()), None);
        let args = ["oh-my-gitty", "--gitty-open-dir", "/definitely/not/a/real/dir"];
        assert_eq!(open_dir_from_args(args.iter()), None);
    }

    #[test]
    fn parses_reg_path_with_spaces() {
        let output = "\r\nHKEY_CURRENT_USER\\Environment\r\n    Path    REG_EXPAND_SZ    C:\\Program Files\\Git\\cmd;C:\\Users\\me\\bin\r\n";
        assert_eq!(
            super::parse_reg_path(output),
            r"C:\Program Files\Git\cmd;C:\Users\me\bin"
        );
        assert_eq!(super::parse_reg_path("no value here"), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shell_escapes_special_chars() {
        assert_eq!(
            super::shell_escape(r#"a\b"c$d`e"#),
            r#"a\\b\"c\$d\`e"#
        );
    }
}
