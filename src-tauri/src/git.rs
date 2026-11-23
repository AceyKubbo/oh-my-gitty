use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crate::process::command;

pub const MAX_AI_DIFF_BYTES: usize = 60_000;

/// git 的空树对象:任何仓库中都存在,用作无父提交/无共同祖先时的对比基准
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Serialize, Clone, Default)]
pub struct FileEntry {
    pub path: String,
    pub orig_path: Option<String>,
    pub x: String, // index 状态字母
    pub y: String, // worktree 状态字母
    pub staged: bool,
    pub untracked: bool,
    pub conflict: bool,
    pub added: i32,   // 相对参照的新增行数(暂存↔HEAD / 更改↔index / 未跟踪=文件行数)
    pub deleted: i32, // 相对参照的删除行数
}

impl FileEntry {}

#[derive(Serialize, Clone, Default)]
pub struct RepoStatus {
    pub repo: String,
    pub is_repo: bool,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i32,
    pub behind: i32,
    pub unborn: bool,
    pub detached: bool,
    /// 当前 HEAD 的 oid(取自 status 的 branch.oid),unborn 时为 None
    pub head: Option<String>,
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub untracked: Vec<FileEntry>,
    pub conflicts: Vec<FileEntry>,
}

/// 子模块摘要(侧栏父项目下的可折叠叶子节点)
#[derive(Serialize, Clone, Default)]
pub struct SubmoduleInfo {
    /// .gitmodules 中配置的名字
    pub name: String,
    /// 相对父仓库的路径
    pub path: String,
    /// 工作区绝对路径(父仓库目录 + path)
    pub full_path: String,
    /// `git submodule status` 中记录的提交 oid
    pub head: String,
    /// 状态字符:' '(已同步) / '+'(子模块有新提交未同步到父仓库) / '-'(未初始化) / 'U'(冲突)
    pub state: String,
    /// 是否已初始化(工作区存在归属 .git)
    pub initialized: bool,
}

/// 子模块视角下父仓库的指针(gitlink)状态:父仓库是否已把本子模块的当前提交提交进去。
#[derive(Serialize, Clone, Default)]
pub struct SubmoduleParentState {
    /// 父仓库(superproject)工作区路径;当前仓库不是子模块时为 None
    pub parent: Option<String>,
    /// 子模块相对父仓库的路径
    pub rel_path: String,
    /// 父仓库索引中存在该子模块指针改动(已暂存)
    pub pointer_staged: bool,
    /// 父仓库工作区中存在该子模块指针改动(未暂存)
    pub pointer_unstaged: bool,
    /// 父仓库是否已提交当前子模块指针(true=父仓库没有待提交的指针改动)
    pub committed: bool,
    /// 子模块中领先于父仓库所记录指针的提交数
    pub commits_ahead: i32,
}

/// 单个 worktree 的摘要(git worktree list --porcelain 的一项)
#[derive(Serialize, Clone, Debug, Default)]
pub struct WorktreeEntry {
    /// 工作区绝对路径
    pub path: String,
    /// 检出的分支名(分离 HEAD 时为 None)
    pub branch: Option<String>,
    /// HEAD oid
    pub head: String,
    /// 是否为主 worktree(其 .git 是目录;其余 worktree 的 .git 是指向 gitdir 的文件)
    pub is_main: bool,
}

/// 某路径所属仓库的全部 worktree 关系。
/// 从任意 worktree 执行 `git worktree list --porcelain` 都能拿到整个仓库的全部工作树。
#[derive(Serialize, Clone, Default)]
pub struct WorktreeInfo {
    /// 当前路径是否为从属 worktree(linked worktree,而非主仓库)
    pub is_linked: bool,
    /// 主 worktree 的绝对路径
    pub main_path: String,
    /// 主 worktree 的项目名(目录名),供侧栏展示「来自 xxx」
    pub main_name: String,
    /// 当前仓库检出的分支(分离 HEAD 时为 None);不从 entries 按路径匹配,
    /// 避免规范化路径形式不同(如 macOS /var 与 /private/var)导致取不到分支
    pub current_branch: Option<String>,
    /// 全部 worktree(主在前),含各自分支
    pub entries: Vec<WorktreeEntry>,
}

/// 每个仓库一把互斥锁:同一仓库的 git 命令串行执行。
/// 暂存/提交/回退等写操作会创建 .git/index.lock,两个 git 进程并发写时,
/// 后者会误报"另一个 git 进程正在运行"。读命令也走同一把锁,避免读到
/// 写操作中途的不一致状态。
static REPO_GIT_LOCKS: Mutex<Option<HashMap<String, Arc<Mutex<()>>>>> = Mutex::new(None);

fn repo_git_lock(repo: &str) -> Arc<Mutex<()>> {
    let mut map = REPO_GIT_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    let map = map.get_or_insert_with(HashMap::new);
    map.entry(repo.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 判断 stderr 是否为 index 锁冲突(git 的"另一个 git 进程"提示)
fn is_lock_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("index.lock") || m.contains("another git process")
}

/// 删除过期的 index.lock 残留:上次 git 写操作被终止(崩溃/强杀)会留下 0 字节锁文件。
/// 锁文件协议只在最终 rename 时替换 index,删除残留锁不会损坏索引。
/// 超过 60 秒仍存在的锁才视为残留——正常写操作远快于此;太新的锁可能是
/// 其他工具正在进行中的操作,不处理。
fn remove_stale_index_lock(repo: &str) -> bool {
    let gd = match command("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--git-dir"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => return false,
    };
    let gd_path = std::path::Path::new(&gd);
    let git_dir = if gd_path.is_absolute() {
        gd_path.to_path_buf()
    } else {
        std::path::Path::new(repo).join(gd_path)
    };
    let lock = git_dir.join("index.lock");
    let stale = std::fs::metadata(&lock)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs() > 60)
        .unwrap_or(false);
    stale && std::fs::remove_file(&lock).is_ok()
}

pub fn run_git(repo: &str, args: &[&str]) -> Result<String, String> {
    run_git_impl(repo, args, false)
}

fn run_git_impl(repo: &str, args: &[&str], retried: bool) -> Result<String, String> {
    let lock = repo_git_lock(repo);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let res = run_git_inner(repo, args);
    if !retried {
        if let Err(e) = &res {
            if is_lock_error(e) && remove_stale_index_lock(repo) {
                drop(_guard);
                return run_git_impl(repo, args, true);
            }
        }
    }
    res
}

fn run_git_inner(repo: &str, args: &[&str]) -> Result<String, String> {
    let out = command("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 git： {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        }
        .trim()
        .to_string());
    }
    Ok(stdout)
}

fn git_stdin(repo: &str, args: &[&str], input: &str) -> Result<String, String> {
    git_stdin_impl(repo, args, input, false)
}

fn git_stdin_impl(repo: &str, args: &[&str], input: &str, retried: bool) -> Result<String, String> {
    let lock = repo_git_lock(repo);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let res = git_stdin_inner(repo, args, input);
    if !retried {
        if let Err(e) = &res {
            if is_lock_error(e) && remove_stale_index_lock(repo) {
                drop(_guard);
                return git_stdin_impl(repo, args, input, true);
            }
        }
    }
    res
}

fn git_stdin_inner(repo: &str, args: &[&str], input: &str) -> Result<String, String> {
    let mut child = command("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 git： {e}"))?;
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .map_err(|e| format!("写入失败： {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("等待 git 失败： {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        }
        .trim()
        .to_string());
    }
    Ok(stdout)
}

/// 解析 porcelain v2 输出,支持 C 风格引号路径(含八进制转义)。
/// 八进制转义是 UTF-8 的原始字节(如 带 = \345\270\246),须先按字节收集再整体解码。
fn unquote(s: &str) -> String {
    if !s.starts_with('"') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 1;
    // 处理到倒数第二个字节,最后一个字节是闭合引号
    while i < bytes.len() - 1 {
        let b = bytes[i];
        if b == b'\\' {
            let n = bytes[i + 1];
            match n {
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b't' => out.push(b'\t'),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b'0'..=b'7' => {
                    let mut val: u32 = 0;
                    let mut k = i + 1;
                    for _ in 0..3 {
                        if k < bytes.len() - 1 && (b'0'..=b'7').contains(&bytes[k]) {
                            val = val * 8 + (bytes[k] - b'0') as u32;
                            k += 1;
                        }
                    }
                    out.push(val as u8);
                    i = k;
                    continue;
                }
                _ => {
                    out.push(b);
                    out.push(n);
                }
            }
            i += 2;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn status(repo: &str) -> RepoStatus {
    let mut st = RepoStatus {
        repo: repo.to_string(),
        ..Default::default()
    };
    let out = match run_git(repo, &["status", "--porcelain=v2", "--branch"]) {
        Ok(o) => o,
        Err(_) => return st, // 不是 git 仓库
    };
    st.is_repo = true;

    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            st.unborn = rest == "(initial)";
            if !st.unborn {
                st.head = Some(rest.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.head ") {
            st.branch = if rest == "(detached)" {
                None
            } else {
                Some(rest.to_string())
            };
            st.detached = rest == "(detached)";
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.upstream ") {
            st.upstream = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("# branch.ab ") {
            let mut parts = rest.split_whitespace();
            if let Some(a) = parts.next() {
                st.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
            }
            if let Some(b) = parts.next() {
                st.behind = b.trim_start_matches('-').parse().unwrap_or(0);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("? ") {
            st.untracked.push(FileEntry {
                path: unquote(rest),
                x: "?".into(),
                y: "?".into(),
                untracked: true,
                ..Default::default()
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("u ") {
            // u <xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
            let mut parts = rest.splitn(10, ' ');
            let _ = parts.next(); // xy
            for _ in 0..8 {
                let _ = parts.next(); // sub, m1..m3, mW, h1..h3
            }
            let path = unquote(parts.next().unwrap_or(""));
            st.conflicts.push(FileEntry {
                path,
                x: "U".into(),
                y: "U".into(),
                conflict: true,
                ..Default::default()
            });
            continue;
        }

        // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
        // porcelain v2 中 '.' 表示未修改(而非空格)
        if let Some(rest) = line.strip_prefix("1 ") {
            let mut parts = rest.splitn(8, ' ');
            let xy = parts.next().unwrap_or("  ").to_string();
            let path = unquote(parts.nth(6).unwrap_or(""));
            let x = xy.chars().next().unwrap_or('.').to_string();
            let y = xy.chars().nth(1).unwrap_or('.').to_string();
            let e = FileEntry {
                path,
                x: x.clone(),
                y: y.clone(),
                ..Default::default()
            };
            if x != "." && x != "?" {
                st.staged.push(FileEntry {
                    staged: true,
                    ..e.clone()
                });
            }
            if y != "." {
                st.unstaged.push(e);
            }
            continue;
        }

        // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path><sep><origPath>
        if let Some(rest) = line.strip_prefix("2 ") {
            let mut parts = rest.splitn(9, ' ');
            let xy = parts.next().unwrap_or("  ").to_string();
            let path_and_orig = parts.nth(7).unwrap_or("").to_string();
            let (path, orig) = match path_and_orig.split_once('\t') {
                Some((p, o)) => (unquote(p), Some(unquote(o))),
                None => (unquote(&path_and_orig), None),
            };
            let x = xy.chars().next().unwrap_or('.').to_string();
            let y = xy.chars().nth(1).unwrap_or('.').to_string();
            let e = FileEntry {
                path,
                orig_path: orig,
                x: x.clone(),
                y: y.clone(),
                ..Default::default()
            };
            if x != "." && x != "?" {
                st.staged.push(FileEntry {
                    staged: true,
                    ..e.clone()
                });
            }
            if y != "." {
                st.unstaged.push(e);
            }
            continue;
        }
    }
    // 忽略规则生效后,命中文件会被移出跟踪(rm --cached),在提交前留下
    // "暂存的删除"记录;这些文件即将退出跟踪且已被忽略,不再显示在暂存列表
    let dels: Vec<String> = st
        .staged
        .iter()
        .filter(|e| e.x == "D" && e.y == ".")
        .map(|e| e.path.clone())
        .collect();
    if !dels.is_empty() {
        let mut args: Vec<&str> = vec!["check-ignore", "--"];
        args.extend(dels.iter().map(String::as_str));
        if let Ok(ign) = run_git(repo, &args) {
            let ignored: Vec<String> = ign.lines().map(unquote).collect();
            st.staged
                .retain(|e| !(e.x == "D" && e.y == "." && ignored.contains(&e.path)));
        }
    }
    // 增删行数:暂存取 index↔HEAD,更改(已跟踪)取 worktree↔index
    fill_numstat(repo, true, &mut st.staged);
    fill_numstat(repo, false, &mut st.unstaged);
    // 未跟踪文件:整文件视为新增(行数=文件行数;过大或二进制则留 0)
    for e in &mut st.untracked {
        let p = std::path::Path::new(repo).join(&e.path);
        if std::fs::metadata(&p)
            .map(|m| m.len() <= 512 * 1024)
            .unwrap_or(false)
        {
            if let Ok(s) = std::fs::read_to_string(&p) {
                e.added = s.lines().count() as i32;
            }
        }
    }
    st
}

/// 列出仓库的直接子模块。
/// `git submodule status` 给出已登记子模块及其当前状态(行首字符),
/// `.gitmodules` 提供每个路径对应的配置名。嵌套子模块不递归,只取直接子节点。
pub fn submodules(repo: &str) -> Vec<SubmoduleInfo> {
    let status_out = match run_git(repo, &["submodule", "status"]) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    // path -> name: git config --file <repo>/.gitmodules --get-regexp ^submodule\..*\.path$
    let mut names: HashMap<String, String> = HashMap::new();
    let gm = std::path::Path::new(repo).join(".gitmodules");
    if gm.exists() {
        let gm_arg = gm.to_string_lossy().to_string();
        if let Ok(cfg) = run_git(
            repo,
            &[
                "config",
                "--file",
                &gm_arg,
                "--get-regexp",
                r"^submodule\..*\.path$",
            ],
        ) {
            for line in cfg.lines() {
                let mut parts = line.splitn(2, ' ');
                let key = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("").trim().to_string();
                if let Some(name) = key
                    .strip_prefix("submodule.")
                    .and_then(|s| s.strip_suffix(".path"))
                {
                    names.insert(path, name.to_string());
                }
            }
        }
    }

    let mut out = Vec::new();
    for line in status_out.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // 行首状态字符后紧跟 oid(无空格):`<state><oid> <path> (<desc>)`
        let mut chars = line.chars();
        let first = chars.next().unwrap_or(' ');
        let (state, rest) = if matches!(first, '+' | '-' | 'U') {
            (first.to_string(), chars.as_str())
        } else {
            (" ".to_string(), line.trim_start())
        };
        let mut parts = rest.splitn(2, char::is_whitespace);
        let oid_tok = parts.next().unwrap_or("");
        let head: String = oid_tok
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        // 行尾可能带 ` (<描述>)`,去掉它只留路径(路径中若含括号,取最后一次出现的 ` (`)
        let raw_path = parts.next().unwrap_or("").trim();
        let raw_path = raw_path
            .rsplit_once(" (")
            .map(|(p, _)| p)
            .unwrap_or(raw_path);
        let path = unquote(raw_path.trim());
        if path.is_empty() {
            continue;
        }
        let full = std::path::Path::new(repo).join(&path);
        out.push(SubmoduleInfo {
            name: names.get(&path).cloned().unwrap_or_else(|| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone())
            }),
            full_path: full.to_string_lossy().to_string(),
            initialized: state != "-" && full.join(".git").exists(),
            path,
            head,
            state,
        });
    }
    out
}

/// 子模块视角:读取父仓库是否已提交本子模块的 gitlink 改动。
/// 检查父仓库中该子模块路径是否仍有暂存/未暂存的指针变更,并数出子模块领先的提交数。
pub fn submodule_parent_state(repo: &str) -> SubmoduleParentState {
    submodule_parent_state_hinted(repo, None, None)
}

/// 与 `submodule_parent_state` 等价,但允许调用方直接给出父仓库路径与子模块相对路径。
/// 侧栏摘要已发现子模块并带有这两项信息(`SubmoduleInfo.parent/path`),切库时传入可
/// 省掉 `rev-parse --show-superproject-working-tree`、`submodule status`、`.gitmodules`
/// 解析与 `canonicalize` 等发现步骤——每次少跑 3 个 git 子进程,是父子仓库切换卡顿的
/// 主要来源之一。无提示时(直接打开的子模块)回落到现场发现,行为与原先一致。
pub fn submodule_parent_state_hinted(
    repo: &str,
    parent_hint: Option<&str>,
    rel_hint: Option<&str>,
) -> SubmoduleParentState {
    let mut out = SubmoduleParentState {
        committed: true,
        ..Default::default()
    };
    // 解析父仓库路径与子模块相对路径:优先用调用方提示,否则现场发现。
    let (parent, rel, parent_view) = match (parent_hint, rel_hint) {
        (Some(p), Some(r)) if !p.is_empty() && !r.is_empty() => {
            (p.to_string(), r.to_string(), p.to_string())
        }
        _ => {
            let parent = match run_git(repo, &["rev-parse", "--show-superproject-working-tree"]) {
                Ok(s) => s.trim().to_string(),
                Err(_) => return out,
            };
            if parent.is_empty() {
                return out; // 不是子模块
            }
            // 相对路径:在父仓库子模块列表中按绝对路径匹配当前仓库。
            // 匹配成功后,父仓库路径取「当前仓库路径去掉相对路径后缀」——git 返回的
            // superproject 路径是规范化的(如 macOS 上 /private/var/...),可能与本应用
            // 保存的路径形式不同,派生出来的形式才能与侧栏项目路径精确对应。
            let matched = submodules(&parent)
                .into_iter()
                .find(|s| same_path(&s.full_path, repo));
            let (rel, parent_view) = match matched {
                Some(s) => {
                    let trimmed = repo.trim_end_matches(|c| c == '/' || c == '\\');
                    let mut derived = None;
                    for sep in ['/', '\\'] {
                        let suffix = format!("{}{}", sep, s.path.replace('/', &sep.to_string()));
                        if let Some(p) = trimmed.strip_suffix(&suffix) {
                            derived = Some(p.to_string());
                            break;
                        }
                    }
                    (s.path, derived.unwrap_or_else(|| parent.clone()))
                }
                None => {
                    let fallback = std::path::Path::new(repo)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    (fallback, parent.clone())
                }
            };
            (parent, rel, parent_view)
        }
    };
    out.parent = Some(parent_view);
    out.rel_path = rel.clone();

    // 父仓库 status --porcelain -- <rel>:X=索引状态,Y=工作区状态
    if rel.is_empty() {
        return out;
    }
    if let Ok(s) = run_git(&parent, &["status", "--porcelain", "--", &rel]) {
        for line in s.lines() {
            let x = line.chars().next().unwrap_or(' ');
            let y = line.chars().nth(1).unwrap_or(' ');
            if x != ' ' && x != '?' {
                out.pointer_staged = true;
            }
            if y != ' ' && y != '?' {
                out.pointer_unstaged = true;
            }
        }
    }
    out.committed = !out.pointer_staged && !out.pointer_unstaged;

    // 父仓库索引记录的指针(gitlink)oid → 子模块领先提交数
    let index_spec = format!(":{}", rel);
    if let Ok(oid) = run_git(&parent, &["rev-parse", &index_spec]) {
        let oid = oid.trim();
        if !oid.is_empty() {
            let range = format!("{}..HEAD", oid);
            if let Ok(n) = run_git(repo, &["rev-list", "--count", &range]) {
                out.commits_ahead = n.trim().parse().unwrap_or(0);
            }
        }
    }
    out
}

/// 两个路径是否指向同一位置(能规范化时按规范化结果比较,否则按原字符串)
fn same_path(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// 把解析中的一条 worktree 记录收尾写入 entries;跳过已失效(路径不存在)的工作树。
fn flush_worktree(
    entries: &mut Vec<WorktreeEntry>,
    path: &mut Option<String>,
    head: &mut String,
    branch: &mut Option<String>,
) {
    if let Some(p) = path.take() {
        if std::path::Path::new(&p).is_dir() {
            entries.push(WorktreeEntry {
                path: p,
                branch: branch.take(),
                head: std::mem::take(head),
                is_main: false,
            });
        } else {
            // prunable / 已被删除的工作区:整条丢弃,避免把无效路径加入项目列表
            *branch = None;
            head.clear();
        }
    }
}

/// 读取当前路径所属仓库的全部 worktree。
/// 主 worktree 判定:其 .git 是目录(linked worktree 的 .git 是文件),
/// 兼容 .git 为文件的情况(worktree/submodule 中 rev-parse --git-dir 同样可用)。
pub fn worktrees(repo: &str) -> WorktreeInfo {
    let mut out = WorktreeInfo::default();
    let raw = match run_git(repo, &["worktree", "list", "--porcelain"]) {
        Ok(o) => o,
        Err(_) => return out, // 非 git 仓库
    };

    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            flush_worktree(&mut entries, &mut path, &mut head, &mut branch);
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.trim().to_string());
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = h.trim().to_string();
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = Some(
                b.strip_prefix("refs/heads/")
                    .unwrap_or(b.trim())
                    .to_string(),
            );
        }
        // "detached" / "prunable ..." / "bare" 等行忽略(branch 保持 None)
    }
    flush_worktree(&mut entries, &mut path, &mut head, &mut branch);

    // 主 worktree:porcelain 输出中 .git 为目录者;兜底取第一个条目
    let mut main_idx = entries
        .iter()
        .position(|e| std::path::Path::new(&e.path).join(".git").is_dir());
    if main_idx.is_none() && !entries.is_empty() {
        main_idx = Some(0);
    }
    if let Some(i) = main_idx {
        entries[i].is_main = true;
        out.main_path = entries[i].path.clone();
        out.main_name = std::path::Path::new(&entries[i].path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
    }
    out.is_linked = !out.main_path.is_empty() && !same_path(repo, &out.main_path);
    // 当前分支:优先取解析出的对应条目,兜底直接问当前仓库(兼容路径形式不一致)
    out.current_branch = entries
        .iter()
        .find(|e| same_path(&e.path, repo))
        .and_then(|e| e.branch.clone())
        .or_else(|| {
            run_git(repo, &["branch", "--show-current"])
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    out.entries = entries;
    out
}

/// 用 git diff --numstat 填充每个文件相对参照的增删行数。
/// cached=true 取暂存区↔HEAD(对应暂存列表),false 取工作区↔暂存区(对应更改列表)。
fn fill_numstat(repo: &str, cached: bool, entries: &mut [FileEntry]) {
    if entries.is_empty() {
        return;
    }
    let out = match if cached {
        run_git(repo, &["diff", "--numstat", "--cached"])
    } else {
        run_git(repo, &["diff", "--numstat"])
    } {
        Ok(o) => o,
        Err(_) => return,
    };
    // numstat: "<added>\t<deleted>\t<path>",二进制文件为 "-\t-\t<path>"
    let mut map: HashMap<String, (i32, i32)> = HashMap::new();
    for line in out.lines() {
        let mut t = line.splitn(3, '\t');
        let a = t.next().unwrap_or("");
        let d = t.next().unwrap_or("");
        let p = t.next().unwrap_or("");
        let to_n = |s: &str| if s == "-" { 0 } else { s.parse().unwrap_or(0) };
        map.insert(unquote(p), (to_n(a), to_n(d)));
    }
    for e in entries.iter_mut() {
        if let Some(&(a, d)) = map.get(&e.path) {
            e.added = a;
            e.deleted = d;
        }
    }
}

pub fn stage_all(repo: &str) -> Result<(), String> {
    run_git(repo, &["add", "-A"]).map(|_| ())
}

pub fn unstage_all(repo: &str) -> Result<(), String> {
    let st = status(repo);
    if st.staged.is_empty() {
        return Ok(());
    }
    if st.unborn {
        // unborn 分支没有 HEAD,git reset 不可用;退化为从索引移除
        run_git(repo, &["rm", "-r", "--cached", "--quiet", "--", "."]).map(|_| ())
    } else {
        run_git(repo, &["reset", "--quiet"]).map(|_| ())
    }
}

pub fn stage_file(repo: &str, path: &str) -> Result<(), String> {
    run_git(repo, &["add", "--", path]).map(|_| ())
}

pub fn unstage_file(repo: &str, path: &str) -> Result<(), String> {
    let st = status(repo);
    if st.unborn {
        run_git(repo, &["rm", "--cached", "--quiet", "--", path]).map(|_| ())
    } else {
        run_git(repo, &["reset", "--quiet", "HEAD", "--", path]).map(|_| ())
    }
}

/// 批量暂存/取消暂存一组路径(文件树中「暂存/取消暂存该目录下所有文件」使用)。
/// 一次 git 调用完成,比逐个调用更省进程;unborn 分支沿用 unstage_file 的降级处理。
pub fn stage_paths(repo: &str, paths: &[String], stage: bool) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    if stage {
        let mut args = vec!["add", "--"];
        for p in paths {
            args.push(p);
        }
        run_git(repo, &args).map(|_| ())
    } else {
        let st = status(repo);
        let mut args = if st.unborn {
            vec!["rm", "--cached", "--quiet", "--"]
        } else {
            vec!["reset", "--quiet", "HEAD", "--"]
        };
        for p in paths {
            args.push(p);
        }
        run_git(repo, &args).map(|_| ())
    }
}

/// 把所有"已跟踪且命中忽略规则"的文件移出跟踪(仅删 index,保留工作区文件)。
/// .gitignore 对已跟踪/已暂存文件不生效,需移出跟踪才会被忽略;
/// ls-files -c -i --exclude-standard 一次取全(含目录/通配规则覆盖的其他文件)。
pub fn untrack_ignored(repo: &str) -> Result<(), String> {
    let out = run_git(repo, &["ls-files", "-c", "-i", "--exclude-standard", "-z"])?;
    let paths: Vec<&str> = out
        .split_terminator('\0')
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = vec!["rm", "--cached", "--quiet", "--"];
    args.extend(paths);
    run_git(repo, &args).map(|_| ())
}

// 放弃所有本地修改:把 HEAD 恢复到上次提交,并清理未跟踪文件。
// 这是确认框对应的全量破坏性操作;忽略文件不受影响。
pub fn discard_all_changes(repo: &str) -> Result<(), String> {
    let st = status(repo);
    if st.unborn {
        if st.staged.is_empty() && st.unstaged.is_empty() && st.untracked.is_empty() {
            return Ok(());
        }
        return run_git(repo, &["clean", "-fd", "--"]).map(|_| ());
    }
    if st.staged.is_empty() && st.unstaged.is_empty() && st.untracked.is_empty() {
        return Ok(());
    }
    run_git(repo, &["reset", "--hard", "HEAD"])?;
    run_git(repo, &["clean", "-fd", "--"]).map(|_| ())
}

/// 丢弃单个文件的更改:已跟踪 → git restore 还原工作区;未跟踪 → git clean 删除
pub fn discard_file(repo: &str, path: &str) -> Result<(), String> {
    if run_git(repo, &["ls-files", "--error-unmatch", "--", path]).is_ok() {
        run_git(repo, &["restore", "--", path]).map(|_| ())
    } else {
        run_git(repo, &["clean", "-fd", "--", path]).map(|_| ())
    }
}

pub fn commit(repo: &str, message: &str) -> Result<String, String> {
    git_stdin(repo, &["commit", "-F", "-"], message)
}

pub fn push(repo: &str) -> Result<String, String> {
    let st = status(repo);
    let result = if st.upstream.is_some() {
        run_git(repo, &["push"])
    } else if let Some(branch) = st.branch {
        run_git(repo, &["push", "-u", "origin", &branch])
    } else {
        Err("无法确定推送目标（仓库处于分离 HEAD 或没有分支）".into())
    };
    // git push 的进度写 stderr,而 run_git 成功只取 stdout → output 可能为空,前端 toast 会空白
    result.map(|o| if o.trim().is_empty() { "推送成功".into() } else { o })
}

pub fn pull(repo: &str) -> Result<String, String> {
    run_git(repo, &["pull"])
}

/// 当前 HEAD 推送到远程仓库的指定分支(不修改当前分支的上游)。
/// target 可写 "deploy"、"origin/deploy" 或 "refs/heads/deploy",统一规范化为分支名。
/// force=true 时用 --force-with-lease:仅当远程分支未被他人更新才覆盖,比 --force 安全。
pub fn push_to_branch(repo: &str, target: &str, force: bool) -> Result<String, String> {
    let remote = push_remote(repo)?;
    let mut branch = target.trim().replace('\\', "/");
    if let Some(rest) = branch.strip_prefix("refs/heads/") {
        branch = rest.to_string();
    }
    let prefix = format!("{remote}/");
    if let Some(rest) = branch.strip_prefix(&prefix) {
        branch = rest.to_string();
    }
    validate_push_branch(&branch)?;

    let spec = format!("HEAD:refs/heads/{branch}");
    let mut args: Vec<&str> = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    args.push(&remote);
    args.push(&spec);
    // 同上:git push 成功时 stdout 通常为空,给前端 toast 一个非空文案
    run_git(repo, &args).map(|o| if o.trim().is_empty() { format!("已推送到 {remote}/{branch}") } else { o })
}

/// 推送目标远程仓库:优先取当前分支上游所在的 remote,其次 origin,最后取第一个 remote。
fn push_remote(repo: &str) -> Result<String, String> {
    if let Some(up) = upstream(repo) {
        if let Some((r, _)) = up.split_once('/') {
            if !r.is_empty() {
                return Ok(r.to_string());
            }
        }
    }
    if run_git(repo, &["remote", "get-url", "origin"]).is_ok() {
        return Ok("origin".to_string());
    }
    run_git(repo, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "未配置远程仓库，无法推送".to_string())
}

/// 校验目标分支名,阻断注入与非法 refname(git check-ref-format 的子集)。
fn validate_push_branch(name: &str) -> Result<(), String> {
    let invalid = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("@{")
        || name.contains(' ')
        || name.contains('\t')
        || name.contains('~')
        || name.contains('^')
        || name.contains(':')
        || name.contains('?')
        || name.contains('*')
        || name.contains('[')
        || name.chars().any(|c| c.is_control())
        || name
            .split('/')
            .any(|c| c.is_empty() || c.starts_with('.') || c.ends_with('.'));
    if invalid {
        Err("分支名不合法（不可含空格、..、@{、~^:?*[ 等字符）".into())
    } else {
        Ok(())
    }
}

/// 静默更新远程跟踪分支(refs/remotes/...),不修改工作区。
/// 供前端后台 fetch:拉取最新远程状态后,ahead/behind 徽标与远程历史才有意义。
pub fn fetch(repo: &str) -> Result<String, String> {
    run_git(repo, &["fetch"])
}

/// 供 AI 生成提交信息用的差异文本。
/// staged_only = true 时只采集已暂存差异(提交语义:只提交已暂存内容,信息须与实际提交一致);
/// false 时采集暂存 + 未暂存 + 未跟踪文件清单。
pub fn diff_for_ai(repo: &str, staged_only: bool) -> Result<String, String> {
    let st = status(repo);
    let mut buf = String::new();
    if let Ok(s) = run_git(
        repo,
        &["diff", "--cached", "--no-color", "--no-ext-diff", "--stat"],
    ) {
        if !s.trim().is_empty() {
            buf.push_str("## 已暂存变更（stat）\n");
            buf.push_str(&s);
            buf.push('\n');
        }
    }
    if let Ok(s) = run_git(repo, &["diff", "--cached", "--no-color", "--no-ext-diff"]) {
        if !s.trim().is_empty() {
            buf.push_str("## 已暂存完整差异\n");
            buf.push_str(&s);
            buf.push('\n');
        }
    }
    if !staged_only {
        if let Ok(s) = run_git(repo, &["diff", "--no-color", "--no-ext-diff", "--stat"]) {
            if !s.trim().is_empty() {
                buf.push_str("## 未暂存变更（stat）\n");
                buf.push_str(&s);
                buf.push('\n');
            }
        }
        if let Ok(s) = run_git(repo, &["diff", "--no-color", "--no-ext-diff"]) {
            if !s.trim().is_empty() {
                buf.push_str("## 未暂存完整差异\n");
                buf.push_str(&s);
                buf.push('\n');
            }
        }
        if !st.untracked.is_empty() {
            buf.push_str("## 未跟踪的新文件（尚未有差异）\n");
            for f in &st.untracked {
                buf.push_str(&format!("- {}\n", f.path));
            }
        }
    }
    if buf.trim().is_empty() {
        return Err(if staged_only {
            "没有已暂存的更改".into()
        } else {
            "没有可提交的变更".into()
        });
    }
    // 超出上限时保留 stat 与尾部内容
    if buf.len() > MAX_AI_DIFF_BYTES {
        let keep = buf.len() - (buf.len() - MAX_AI_DIFF_BYTES);
        buf = buf[..keep].to_string();
        buf.push_str("\n...（差异过大已截断）\n");
    }
    Ok(buf)
}

/// 最近提交信息(供 AI 参考仓库既有风格)
pub fn recent_log(repo: &str, n: usize) -> Vec<String> {
    run_git(repo, &["log", &format!("-{n}"), "--format=%s"])
        .map(|o| o.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// 最近一次提交的 unix 时间戳(秒);非仓库/空仓库返回 None
pub fn last_commit_ts(repo: &str) -> Option<i64> {
    run_git(repo, &["log", "-1", "--format=%ct"])
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// 单个仓库在指定时间点之后的提交数，按作者日期聚合为本地日历日。
/// 总览只读取当前 HEAD，避免未检出的历史分支重复计入项目活跃度。
#[derive(Serialize, Clone)]
pub struct CommitDay {
    pub date: String,
    pub count: usize,
}

pub fn commit_days(repo: &str, since: i64) -> Vec<CommitDay> {
    let since = format!("--since=@{since}");
    run_git(
        repo,
        &["log", "HEAD", &since, "--date=short", "--format=%ad"],
    )
    .map(|out| {
        let mut counts = BTreeMap::<String, usize>::new();
        for date in out.lines().map(str::trim).filter(|date| !date.is_empty()) {
            *counts.entry(date.to_string()).or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(date, count)| CommitDay { date, count })
            .collect()
    })
    .unwrap_or_default()
}

/// 单个仓库在指定时间点之后的代码量(新增/删除行数),按作者日期聚合为本地日历日。
/// --numstat 逐文件给出 增\t删\t路径,二进制文件为 -\t-\t路径(不计入);
/// 与 --format=%ad 混排时日期行(无制表符)作为归属日的标记
#[derive(Serialize, Clone)]
pub struct CodeDay {
    pub date: String,
    pub add: usize,
    pub del: usize,
}

pub fn code_days(repo: &str, since: i64) -> Vec<CodeDay> {
    let since = format!("--since=@{since}");
    run_git(
        repo,
        &[
            "log", "HEAD", &since, "--date=short",
            "--pretty=format:%ad", "--numstat",
        ],
    )
    .map(|out| {
        let mut days: Vec<CodeDay> = Vec::new();
        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // 日期标记行:yyyy-mm-dd 且不含制表符(numstat 行必含两个制表符)
            if !line.contains('\t')
                && line.len() == 10
                && line.as_bytes()[4] == b'-'
                && line.as_bytes()[7] == b'-'
            {
                // 同日多笔提交:延续上一组,不新开
                if days.last().map(|d| d.date != line).unwrap_or(true) {
                    days.push(CodeDay { date: line.to_string(), add: 0, del: 0 });
                }
                continue;
            }
            let mut it = line.split('\t');
            let (add, del) = match (it.next(), it.next()) {
                (Some(a), Some(d)) => (a.parse::<usize>(), d.parse::<usize>()),
                _ => continue,
            };
            if let (Ok(a), Ok(d)) = (add, del) {
                if let Some(day) = days.last_mut() {
                    day.add += a;
                    day.del += d;
                }
            }
        }
        days
    })
    .unwrap_or_default()
}

#[derive(Serialize, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    /// 提交信息正文(去掉第一行 subject 后的剩余部分),可能为空
    pub body: String,
}

/// 历史列表(按时间倒序,HEAD 当前分支)
pub fn log(repo: &str, n: usize) -> Vec<CommitInfo> {
    log_ref(repo, "HEAD", n)
}

/// 指定 ref(分支/远程分支)的历史
/// 记录以 NUL 分隔、字段以 \t 分隔:提交信息正文含换行,按行分割会把一条提交拆成多条。
pub fn log_ref(repo: &str, r: &str, n: usize) -> Vec<CommitInfo> {
    run_git(
        repo,
        &[
            "log",
            &format!("-{n}"),
            r,
            "--format=%H%x09%h%x09%an%x09%at%x09%s%x09%b%x00",
        ],
    )
    .map(|o| {
        o.split('\0')
            .filter_map(|rec| {
                // git 在每条自定义 format 记录后仍追加换行；第二条起会以 \n（Windows 为 \r\n）开头。
                let rec = rec.trim_start_matches(['\r', '\n']);
                let mut p = rec.splitn(6, '\t');
                Some(CommitInfo {
                    hash: p.next()?.to_string(),
                    short: p.next()?.to_string(),
                    author: p.next()?.to_string(),
                    timestamp: p.next()?.parse().unwrap_or(0),
                    subject: p.next()?.to_string(),
                    body: p.next()?.trim().to_string(),
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// 当前分支的上游(远程跟踪分支),如 "origin/main";无上游返回 None
pub fn upstream(repo: &str) -> Option<String> {
    run_git(
        repo,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// 第一个远程跟踪分支(如 origin/master),排除符号引用 origin/HEAD;
/// 用于 upstream 未设置时兜底展示远程历史
pub fn first_remote_branch(repo: &str) -> Option<String> {
    run_git(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/remotes"],
    )
    .ok()?
    .lines()
    .map(|l| l.trim().to_string())
    .find(|l| !l.is_empty() && l != "origin/HEAD")
}

/// 本地领先远程的提交数(remote..HEAD)
pub fn ahead_count(repo: &str, remote_ref: &str) -> i64 {
    run_git(
        repo,
        &["rev-list", "--count", &format!("{remote_ref}..HEAD")],
    )
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .unwrap_or(0)
}

/// 远程领先本地的提交数(HEAD..remote)
pub fn behind_count(repo: &str, remote_ref: &str) -> i64 {
    run_git(
        repo,
        &["rev-list", "--count", &format!("HEAD..{remote_ref}")],
    )
    .ok()
    .and_then(|s| s.trim().parse().ok())
    .unwrap_or(0)
}

pub fn head_hash(repo: &str) -> Option<String> {
    run_git(repo, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
}

/// 强制回退到指定提交(丢弃之后的所有提交与未提交修改)
pub fn reset_hard(repo: &str, hash: &str) -> Result<String, String> {
    run_git(repo, &["reset", "--hard", hash])
}

/// 某提交的「上一版本」参照:第一个父提交(合并提交只看第一父);
/// 根提交无父,用 git 的空树对象代替(任何仓库中都存在),使新增文件也能正常对比。
fn commit_parent_or_empty_tree(repo: &str, hash: &str) -> Result<String, String> {
    let parents = run_git(repo, &["rev-list", "--parents", "-n", "1", hash])?;
    let mut tokens = parents.split_whitespace();
    let _ = tokens.next(); // 自身 hash
    Ok(tokens.next().unwrap_or(EMPTY_TREE).to_string())
}

/// 某个提交改动的文件清单(相对上一版本)。
/// status 为 A/M/D/T 等状态字母(--no-renames 下重命名拆成 A+D),
/// added/deleted 为增删行数(二进制文件记为 0)。供版本历史展开后展示。
#[derive(Serialize, Clone)]
pub struct CommitFile {
    pub path: String,
    pub status: String,
    pub added: i32,
    pub deleted: i32,
}

pub fn commit_files(repo: &str, hash: &str) -> Result<Vec<CommitFile>, String> {
    let parent = commit_parent_or_empty_tree(repo, hash)?;
    let base: Vec<&str> = vec![parent.as_str(), hash];

    let mut ns_args: Vec<&str> = vec![
        "diff-tree", "-r", "--no-commit-id", "--no-renames", "-z", "--name-status",
    ];
    ns_args.extend(base.iter().copied());
    let ns = run_git(repo, &ns_args)?;

    let mut num_args: Vec<&str> = vec![
        "diff-tree", "-r", "--no-commit-id", "--no-renames", "-z", "--numstat",
    ];
    num_args.extend(base.iter().copied());
    let num = run_git(repo, &num_args)?;

    Ok(parse_diff_files(&ns, &num))
}

/// 解析 diff 的 --name-status -z 与 --numstat -z 输出,合并成文件清单。
/// name-status: <状态>\0<路径>\0 ...;numstat: <增>\t<删>\t<路径>\0 ...
/// (-z 只把记录分隔符改成 NUL,记录内字段仍是 TAB;二进制文件增删记为 "-")
fn parse_diff_files(ns: &str, num: &str) -> Vec<CommitFile> {
    let mut statuses: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let fields: Vec<&str> = ns.split('\0').collect();
    let mut i = 0;
    while i + 1 < fields.len() {
        let st = fields[i].trim().to_string();
        let path = unquote(fields[i + 1]);
        if !st.is_empty() && !path.is_empty() {
            statuses.insert(path.clone(), st);
            order.push(path);
        }
        i += 2;
    }

    let mut counts: HashMap<String, (i32, i32)> = HashMap::new();
    let to_n = |s: &str| if s == "-" { 0 } else { s.parse().unwrap_or(0) };
    for rec in num.split('\0') {
        let mut f = rec.splitn(3, '\t');
        if let (Some(a), Some(d), Some(p)) = (f.next(), f.next(), f.next()) {
            counts.insert(unquote(p), (to_n(a), to_n(d)));
        }
    }

    order
        .into_iter()
        .map(|path| {
            let (added, deleted) = counts.get(&path).copied().unwrap_or((0, 0));
            CommitFile {
                status: statuses.get(&path).cloned().unwrap_or_default(),
                path,
                added,
                deleted,
            }
        })
        .collect()
}

/// 某个提交相对「上一版本」的差异:该文件在此提交中到底改了什么。
/// 减号行=上一版本(父提交)的内容,加号行=此提交的内容。
pub fn commit_file_diff(repo: &str, hash: &str, path: &str) -> Result<String, String> {
    let parent = commit_parent_or_empty_tree(repo, hash)?;
    run_git(
        repo,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            &parent,
            hash,
            "--",
            path,
        ],
    )
}

/// 本地分支列表
pub fn local_branches(repo: &str) -> Vec<String> {
    run_git(repo, &["branch", "--format=%(refname:short)"])
        .map(|o| {
            o.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 远程跟踪分支列表(排除 origin/HEAD 等符号引用)
pub fn remote_branches(repo: &str) -> Vec<String> {
    // 不用 --format=%(refname:short):它会把符号引用 origin/HEAD 输出成裸 "origin",
    // 使过滤失效。改用原始输出,跳过含 "->" 的符号引用行(如 origin/HEAD -> origin/main)。
    run_git(repo, &["branch", "-r"])
        .map(|o| {
            o.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.contains("->"))
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// 切换分支;远程分支(origin/xxx)自动创建对应本地分支跟踪
pub fn checkout(repo: &str, branch: &str) -> Result<String, String> {
    if let Some(remote) = branch.strip_prefix("origin/") {
        run_git(repo, &["checkout", "-b", remote, branch])
    } else {
        run_git(repo, &["checkout", branch])
    }
}

/// 把指定分支(可为本地分支或 origin/xxx 远程跟踪分支)合并到当前分支。
/// --no-edit 避免弹出编辑器;冲突时 git 返回错误,但冲突状态与 MERGE_HEAD 保留,
/// 前端据 status.conflicts 决定是否复用 AI 冲突解决流程。
pub fn merge(repo: &str, branch: &str) -> Result<String, String> {
    run_git(repo, &["merge", "--no-edit", branch])
}

/// 挑拣某个提交到当前分支。冲突时 git 返回错误,CHERRY_PICK_HEAD 保留,
/// 解决后由 continue_operation 续做,或 abort_operation 终止。
pub fn cherry_pick(repo: &str, hash: &str) -> Result<String, String> {
    run_git(repo, &["cherry-pick", hash])
}

/// 两个 ref 的对比结果:target 相对 base 的独有提交(可挑拣)、
/// base 相对 target 的独有提交数,以及自共同祖先起 target 引入的文件改动。
#[derive(Serialize, Clone)]
pub struct CompareResult {
    pub base: String,
    pub target: String,
    /// 在 target 上、不在 base 上的提交(新→旧),供逐条挑拣
    pub incoming: Vec<CommitInfo>,
    /// 在 base 上、不在 target 上的提交数(即"落后"数)
    pub outgoing: usize,
    pub files: Vec<CommitFile>,
}

pub fn compare(repo: &str, base: &str, target: &str) -> Result<CompareResult, String> {
    let incoming = log_ref(repo, &format!("{base}..{target}"), 50);
    let outgoing = run_git(repo, &["rev-list", "--count", &format!("{target}..{base}")])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    // A...B 的语义 = A 与 B 的共同祖先 → B,正是"合并 B 会给 A 带来什么"的视角。
    // 显式求 merge-base,使无共同祖先(无关历史)时退化为与空树比较而不是报错。
    let mb = run_git(repo, &["merge-base", base, target])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| EMPTY_TREE.to_string());
    let ns = run_git(repo, &["diff", "--name-status", "-z", "--no-renames", &mb, target])?;
    let num = run_git(repo, &["diff", "--numstat", "-z", "--no-renames", &mb, target])?;
    Ok(CompareResult {
        base: base.to_string(),
        target: target.to_string(),
        incoming,
        outgoing,
        files: parse_diff_files(&ns, &num),
    })
}

/// 对比视图中单个文件的差异(base...target 中该文件改了什么)
pub fn compare_file_diff(repo: &str, base: &str, target: &str, path: &str) -> Result<String, String> {
    let mb = run_git(repo, &["merge-base", base, target])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| EMPTY_TREE.to_string());
    run_git(
        repo,
        &["diff", "--no-color", "--no-ext-diff", &mb, target, "--", path],
    )
}

/// 解析仓库的 git 目录绝对路径(兼容 worktree 中 .git 为文件的情况)
fn git_dir(repo: &str) -> Option<std::path::PathBuf> {
    let gd = run_git(repo, &["rev-parse", "--git-dir"]).ok()?;
    let p = std::path::Path::new(gd.trim());
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::path::Path::new(repo).join(p)
    })
}

/// 当前进行中的 git 操作:none / merge / cherry-pick / revert
pub fn operation_state(repo: &str) -> &'static str {
    let Some(gd) = git_dir(repo) else {
        return "none";
    };
    if gd.join("CHERRY_PICK_HEAD").exists() {
        "cherry-pick"
    } else if gd.join("REVERT_HEAD").exists() {
        "revert"
    } else if gd.join("MERGE_HEAD").exists() {
        "merge"
    } else {
        "none"
    }
}

/// 冲突已解决后继续未完成的 git 操作。merge 走提交(复用 finish_merge),
/// cherry-pick/revert 走 --continue(-c core.editor=true 抑制编辑器)。
/// 返回 (是否确有操作被继续, 输出)。
pub fn continue_operation(repo: &str) -> Result<(bool, String), String> {
    match operation_state(repo) {
        "merge" => finish_merge(repo),
        "cherry-pick" => Ok((
            true,
            run_git(repo, &["-c", "core.editor=true", "cherry-pick", "--continue"])?,
        )),
        "revert" => Ok((
            true,
            run_git(repo, &["-c", "core.editor=true", "revert", "--continue"])?,
        )),
        _ => Ok((false, String::new())),
    }
}

/// 终止进行中的 git 操作(合并/挑拣/回退);无操作进行中返回 false。
pub fn abort_operation(repo: &str) -> Result<bool, String> {
    match operation_state(repo) {
        "merge" => run_git(repo, &["merge", "--abort"]).map(|_| true),
        "cherry-pick" => run_git(repo, &["cherry-pick", "--abort"]).map(|_| true),
        "revert" => run_git(repo, &["revert", "--abort"]).map(|_| true),
        _ => Ok(false),
    }
}

/// 冲突文件清单
pub fn conflict_files(repo: &str) -> Vec<String> {
    run_git(repo, &["diff", "--name-only", "--diff-filter=U"])
        .map(|o| o.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// 检测是否处于合并中(MERGE_HEAD 存在),若是则用 git 准备的 MERGE_MSG 完成合并提交。
/// 返回 (是否完成了合并, 提交输出);非合并状态返回 (false, "")。
pub fn finish_merge(repo: &str) -> Result<(bool, String), String> {
    let gd = git_dir(repo).ok_or_else(|| "无法定位 git 目录".to_string())?;
    if !gd.join("MERGE_HEAD").exists() {
        return Ok((false, String::new()));
    }
    let msg = std::fs::read_to_string(gd.join("MERGE_MSG")).unwrap_or_default();
    let msg = if msg.trim().is_empty() {
        "Merge".into()
    } else {
        msg.trim().to_string()
    };
    let out = commit(repo, &msg)?;
    Ok((true, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ohmygitty-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run_git(dir.to_str().unwrap(), &["init", "-q"]).unwrap();
        run_git(
            dir.to_str().unwrap(),
            &["config", "user.email", "test@example.com"],
        )
        .unwrap();
        run_git(dir.to_str().unwrap(), &["config", "user.name", "test"]).unwrap();
        // Windows 上 git 默认 autocrlf=true 会在检出时改写换行符,统一关闭保证内容断言跨平台一致
        run_git(
            dir.to_str().unwrap(),
            &["config", "core.autocrlf", "false"],
        )
        .unwrap();
        dir
    }

    #[test]
    fn parses_status_full_lifecycle() {
        let repo = temp_repo("lifecycle");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        std::fs::write(repo.join("b.txt"), "world").unwrap();

        // 未跟踪
        let st = status(r);
        assert!(st.is_repo && st.unborn);
        assert_eq!(st.untracked.len(), 2);

        // 全部暂存
        stage_all(r).unwrap();
        let st = status(r);
        assert_eq!(st.staged.len(), 2);
        assert!(st.staged.iter().all(|f| f.untracked == false));

        // 首次提交
        commit(r, "init: add files").unwrap();
        let st = status(r);
        assert!(!st.unborn);
        assert!(st.staged.is_empty() && st.untracked.is_empty());

        // 修改 + 部分暂存
        std::fs::write(repo.join("a.txt"), "hello2").unwrap();
        std::fs::write(repo.join("b.txt"), "world2").unwrap();
        stage_file(r, "a.txt").unwrap();
        let st = status(r);
        assert_eq!(st.staged.len(), 1);
        assert_eq!(st.unstaged.len(), 1);
        assert_eq!(st.staged[0].path, "a.txt");

        // 单文件取消暂存
        unstage_file(r, "a.txt").unwrap();
        let st = status(r);
        assert!(st.staged.is_empty());
        assert_eq!(st.unstaged.len(), 2);

        // 全部取消暂存(空操作不报错)
        unstage_all(r).unwrap();

        // 带空格与中文路径
        std::fs::write(repo.join("带 空格 文件.txt"), "x").unwrap();
        stage_all(r).unwrap();
        let st = status(r);
        assert!(st.staged.iter().any(|f| f.path == "带 空格 文件.txt"));

        // diff_for_ai 可用
        let d = diff_for_ai(r, false).unwrap();
        assert!(!d.is_empty());
        // staged-only:取消暂存 b.txt 后,暂存 diff 应含 a.txt 而不含 b.txt
        unstage_file(r, "b.txt").unwrap();
        let d_staged = diff_for_ai(r, true).unwrap();
        assert!(d_staged.contains("a.txt"));
        assert!(!d_staged.contains("b.txt"));

        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn last_commit_ts_tracks_head() {
        let repo = temp_repo("last-ts");
        let r = repo.to_str().unwrap();
        assert!(last_commit_ts(r).is_none(), "空仓库应返回 None");
        std::fs::write(repo.join("a.txt"), "hi").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();
        let ts = last_commit_ts(r).expect("有提交后应返回时间戳");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(
            ts > 0 && ts <= now + 5,
            "时间戳应落在合理区间: {ts} vs {now}"
        );
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn discard_all_restores_head_and_removes_untracked() {
        let repo = temp_repo("discard-all");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("tracked.txt"), "base\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "base").unwrap();

        // 同时制造已暂存、未暂存和未跟踪改动。
        std::fs::write(repo.join("tracked.txt"), "staged\n").unwrap();
        stage_all(r).unwrap();
        std::fs::write(repo.join("tracked.txt"), "unstaged\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        let before = status(r);
        assert!(!before.staged.is_empty());
        assert!(!before.unstaged.is_empty());
        assert!(!before.untracked.is_empty());

        discard_all_changes(r).unwrap();

        let after = status(r);
        assert!(after.staged.is_empty());
        assert!(after.unstaged.is_empty());
        assert!(after.untracked.is_empty());
        assert_eq!(
            std::fs::read_to_string(repo.join("tracked.txt")).unwrap(),
            "base\n"
        );
        assert!(!repo.join("new.txt").exists());
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn detects_conflicts() {
        let base = temp_repo("conflict");
        let r = base.to_str().unwrap();
        std::fs::write(base.join("f.txt"), "line1\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "base").unwrap();
        // 双方在相同位置修改同一行,制造真实冲突
        // 先记录主分支名(init.defaultBranch 可能是 master 或 main)
        let main_branch = run_git(r, &["symbolic-ref", "--short", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        run_git(r, &["checkout", "-q", "-b", "feature"]).unwrap();
        std::fs::write(base.join("f.txt"), "line1\nfeature\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "feature change").unwrap();
        run_git(r, &["checkout", "-q", &main_branch]).unwrap();
        std::fs::write(base.join("f.txt"), "line1\nother\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "main change").unwrap();
        let err = run_git(r, &["merge", "feature"]);
        assert!(err.is_err(), "merge 应产生冲突");
        let cf = conflict_files(r);
        assert!(cf.contains(&"f.txt".to_string()));

        // 手动解决后 finish_merge 应自动完成合并提交
        assert!(conflict_files(r).contains(&"f.txt".to_string()));
        std::fs::write(base.join("f.txt"), "line1\nmerged\nline2\n").unwrap();
        stage_all(r).unwrap();
        let (merged, _) = finish_merge(r).unwrap();
        assert!(merged, "存在 MERGE_HEAD 时应完成合并提交");
        assert!(conflict_files(r).is_empty(), "合并后不应再有冲突");
        // 非合并状态调用返回 false 而非报错
        let (merged2, _) = finish_merge(r).unwrap();
        assert!(!merged2);
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn finish_merge_noop_without_merge() {
        let repo = temp_repo("noopmerge");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();
        let (merged, _) = finish_merge(r).unwrap();
        assert!(!merged, "无 MERGE_HEAD 时不应合并");
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn log_and_reset_hard() {
        let repo = temp_repo("logreset");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "v1").unwrap();
        stage_all(r).unwrap();
        commit(r, "first").unwrap();
        std::fs::write(repo.join("a.txt"), "v2").unwrap();
        stage_all(r).unwrap();
        commit(r, "second").unwrap();

        let commits = log(r, 10);
        assert_eq!(commits.len(), 2, "应解析出 2 条提交");
        assert_eq!(commits[0].subject, "second", "按时间倒序");
        assert_eq!(commits[0].short.len(), 7, "短 hash 为 7 位");
        assert!(head_hash(r).as_deref() == Some(commits[0].hash.as_str()));

        // 强制回退到 first
        reset_hard(r, &commits[1].hash).unwrap();
        let commits2 = log(r, 10);
        assert_eq!(commits2.len(), 1);
        assert_eq!(commits2[0].subject, "first");
        assert_eq!(head_hash(r).as_deref(), Some(commits2[0].hash.as_str()));
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn commit_files_and_diff_against_worktree() {
        let repo = temp_repo("commit-files");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "v1\n").unwrap();
        std::fs::write(repo.join("keep.txt"), "keep\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();

        // 根提交:全部文件都是新增
        let root = log(r, 10).pop().unwrap();
        let root_files = commit_files(r, &root.hash).unwrap();
        assert_eq!(root_files.len(), 2);
        assert!(root_files.iter().all(|f| f.status == "A"));

        // 第二个提交:修改 a.txt,新增 b.txt
        std::fs::write(repo.join("a.txt"), "v2\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "second").unwrap();

        let head = log(r, 10)[0].clone();
        let files = commit_files(r, &head.hash).unwrap();
        assert_eq!(files.len(), 2, "应列出该提交改动的 2 个文件");
        let a = files.iter().find(|f| f.path == "a.txt").unwrap();
        let b = files.iter().find(|f| f.path == "b.txt").unwrap();
        assert_eq!(a.status, "M", "a.txt 应为修改");
        assert_eq!(b.status, "A", "b.txt 应为新增");
        assert_eq!((a.added, a.deleted), (1, 1));
        assert_eq!((b.added, b.deleted), (1, 0));

        // 差异=该提交相对上一版本改了什么:a.txt 的 v1→v2 应同时含 -v1/+v2
        let d = commit_file_diff(r, &head.hash, "a.txt").unwrap();
        assert!(d.contains("-v1"), "应含上一版本的旧行: {d}");
        assert!(d.contains("+v2"), "应含此提交的新行: {d}");

        // 新增文件 b.txt:相对上一版本只有新增行
        let db = commit_file_diff(r, &head.hash, "b.txt").unwrap();
        assert!(db.contains("+new"), "应含新增行: {db}");
        assert!(!db.contains("-new"), "新增文件不应有删除行: {db}");

        // 工作区后续改动不影响结果(对比基准是父提交,不是当前代码)
        std::fs::write(repo.join("a.txt"), "worktree\n").unwrap();
        let d2 = commit_file_diff(r, &head.hash, "a.txt").unwrap();
        assert_eq!(d, d2, "差异不应随工作区改动变化");

        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn upstream_and_remote_log() {
        // 建裸仓库作远程
        let bare =
            std::env::temp_dir().join(format!("ohmygitty-test-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        run_git(bare.to_str().unwrap(), &["init", "-q", "--bare"]).unwrap();

        let repo = temp_repo("upstream");
        let r = repo.to_str().unwrap();
        assert!(upstream(r).is_none(), "未设置上游时返回 None");

        std::fs::write(repo.join("a.txt"), "v1").unwrap();
        stage_all(r).unwrap();
        commit(r, "local commit").unwrap();
        run_git(r, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
        // 用当前分支(init.defaultBranch 可能是 master 或 main)
        let branch = run_git(r, &["symbolic-ref", "--short", "HEAD"]).unwrap();
        let branch = branch.trim().to_string();
        run_git(r, &["push", "-u", "-q", "origin", &branch]).unwrap();

        let remote_ref = format!("origin/{branch}");
        let up = upstream(r);
        assert_eq!(up.as_deref(), Some(remote_ref.as_str()));
        let remote_log = log_ref(r, &remote_ref, 10);
        assert_eq!(remote_log.len(), 1);
        assert_eq!(remote_log[0].subject, "local commit");
        // 兜底发现远程分支(upstream 缺失场景)
        assert_eq!(first_remote_branch(r).as_deref(), Some(remote_ref.as_str()));
        // ahead/behind:同步后均为 0
        assert_eq!(ahead_count(r, &remote_ref), 0);
        assert_eq!(behind_count(r, &remote_ref), 0);
        // 本地新增一个提交 → ahead = 1
        std::fs::write(repo.join("a.txt"), "v2").unwrap();
        stage_all(r).unwrap();
        commit(r, "local only").unwrap();
        assert_eq!(ahead_count(r, &remote_ref), 1);

        std::fs::remove_dir_all(repo).ok();
        std::fs::remove_dir_all(bare).ok();
    }

    #[test]
    fn branches_and_checkout() {
        let repo = temp_repo("branches");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();
        let main = run_git(r, &["branch", "--show-current"]).unwrap();
        let main = main.trim().to_string();
        // 建第二个本地分支
        run_git(r, &["checkout", "-q", "-b", "dev"]).unwrap();
        std::fs::write(repo.join("b.txt"), "y").unwrap();
        stage_all(r).unwrap();
        commit(r, "dev work").unwrap();
        run_git(r, &["checkout", "-q", &main]).unwrap();

        let locals = local_branches(r);
        assert!(locals.contains(&main) && locals.contains(&"dev".to_string()));
        // 切到 dev 并确认
        checkout(r, "dev").unwrap();
        let cur = run_git(r, &["branch", "--show-current"]).unwrap();
        assert_eq!(cur.trim(), "dev");
        // 切到不存在的分支报错
        assert!(checkout(r, "nope").is_err());
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn compare_branches_and_merge() {
        let repo = temp_repo("compare-merge");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();
        let main = run_git(r, &["branch", "--show-current"])
            .unwrap()
            .trim()
            .to_string();

        run_git(r, &["checkout", "-q", "-b", "feature"]).unwrap();
        std::fs::write(repo.join("base.txt"), "feature\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "feature work").unwrap();
        run_git(r, &["checkout", "-q", &main]).unwrap();

        let cmp = compare(r, &main, "feature").unwrap();
        assert_eq!(cmp.base, main);
        assert_eq!(cmp.target, "feature");
        assert_eq!(cmp.incoming.len(), 1, "feature 上有 1 个独有提交");
        assert_eq!(cmp.incoming[0].subject, "feature work");
        assert_eq!(cmp.outgoing, 0);
        let paths: Vec<&str> = cmp.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"base.txt") && paths.contains(&"new.txt"));
        // 单个文件差异来自共同祖先 → target
        let d = compare_file_diff(r, &main, "feature", "base.txt").unwrap();
        assert!(d.contains("+feature") && d.contains("-base"));

        merge(r, "feature").unwrap();
        assert_eq!(operation_state(r), "none");
        assert_eq!(std::fs::read_to_string(repo.join("base.txt")).unwrap(), "feature\n");
        assert!(repo.join("new.txt").exists());
        // 合并后 feature 的独有提交已并入当前分支
        assert_eq!(compare(r, &main, "feature").unwrap().incoming.len(), 0);
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn cherry_pick_apply_and_conflict() {
        let repo = temp_repo("cherry-pick");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("f.txt"), "line1\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "base").unwrap();

        // feature 新增文件,挑拣应干净应用
        run_git(r, &["checkout", "-q", "-b", "feature"]).unwrap();
        std::fs::write(repo.join("added.txt"), "added\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "add file").unwrap();
        let picked = log(r, 1)[0].hash.clone();
        run_git(r, &["checkout", "-q", "-"]).unwrap();
        cherry_pick(r, &picked).unwrap();
        assert!(repo.join("added.txt").exists());
        assert_eq!(operation_state(r), "none");

        // 制造冲突:双方改同一行,挑拣应停下并保留 CHERRY_PICK_HEAD
        run_git(r, &["checkout", "-q", "-b", "conflict"]).unwrap();
        std::fs::write(repo.join("f.txt"), "line1\nfrom-conflict\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "conflict change").unwrap();
        let conflict_hash = log(r, 1)[0].hash.clone();
        run_git(r, &["checkout", "-q", "-"]).unwrap();
        std::fs::write(repo.join("f.txt"), "line1\nfrom-main\nline2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "main change").unwrap();
        assert!(cherry_pick(r, &conflict_hash).is_err(), "应产生冲突");
        assert_eq!(operation_state(r), "cherry-pick");
        assert!(conflict_files(r).contains(&"f.txt".to_string()));

        // 解决并继续:挑拣提交应落到 HEAD,操作状态清空
        std::fs::write(repo.join("f.txt"), "line1\nresolved\nline2\n").unwrap();
        stage_all(r).unwrap();
        let (continued, _) = continue_operation(r).unwrap();
        assert!(continued);
        assert_eq!(operation_state(r), "none");
        assert_eq!(log(r, 1)[0].subject, "conflict change");
        assert!(conflict_files(r).is_empty());
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn merge_conflict_continue_and_abort() {
        let repo = temp_repo("merge-conflict-flow");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("f.txt"), "v0\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "base").unwrap();
        let main = run_git(r, &["branch", "--show-current"])
            .unwrap()
            .trim()
            .to_string();

        run_git(r, &["checkout", "-q", "-b", "topic"]).unwrap();
        std::fs::write(repo.join("f.txt"), "topic\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "topic").unwrap();
        run_git(r, &["checkout", "-q", &main]).unwrap();
        std::fs::write(repo.join("f.txt"), "main\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "main").unwrap();

        // 先验证终止:冲突后 abort 应回到合并前,HEAD 不变
        let before = head_hash(r);
        assert!(merge(r, "topic").is_err());
        assert_eq!(operation_state(r), "merge");
        assert!(abort_operation(r).unwrap());
        assert_eq!(operation_state(r), "none");
        assert_eq!(head_hash(r), before);

        // 再验证解决后继续
        assert!(merge(r, "topic").is_err());
        std::fs::write(repo.join("f.txt"), "merged\n").unwrap();
        stage_all(r).unwrap();
        let (continued, _) = continue_operation(r).unwrap();
        assert!(continued);
        assert_eq!(operation_state(r), "none");
        assert!(log(r, 1)[0].subject.starts_with("Merge"));
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn push_to_another_remote_branch() {
        let bare = std::env::temp_dir().join(format!("ohmygitty-push-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        run_git(bare.to_str().unwrap(), &["init", "-q", "--bare"]).unwrap();

        let repo = temp_repo("push-branch");
        let r = repo.to_str().unwrap();
        std::fs::write(repo.join("a.txt"), "v1\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "first").unwrap();
        run_git(r, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();

        // 推送当前 HEAD 到远程的 deploy 分支(远程原本没有该分支)
        let out = push_to_branch(r, "deploy", false).unwrap();
        assert!(!out.trim().is_empty(), "推送成功必须返回非空文案,否则前端 toast 为空白");
        let deploy = run_git(bare.to_str().unwrap(), &["rev-parse", "refs/heads/deploy"])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(Some(deploy.clone()), head_hash(r));
        assert_eq!(upstream(r), None, "不应改变当前分支的上游");

        // origin/ 前缀与 refs/heads/ 前缀都应规范化成同一分支
        std::fs::write(repo.join("a.txt"), "v2\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "second").unwrap();
        push_to_branch(r, "origin/deploy", false).unwrap();
        let deploy2 = run_git(bare.to_str().unwrap(), &["rev-parse", "refs/heads/deploy"])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(Some(deploy2.clone()), head_hash(r));
        assert_ne!(deploy, deploy2);

        // 非快进推送被拒绝,force(force-with-lease)可覆盖
        reset_hard(r, &deploy).unwrap();
        std::fs::write(repo.join("a.txt"), "diverged\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "diverged").unwrap();
        assert!(push_to_branch(r, "deploy", false).is_err(), "非快进应被拒绝");
        fetch(r).unwrap(); // 更新 origin/deploy 的 lease 基准
        push_to_branch(r, "refs/heads/deploy", true).unwrap();
        assert_eq!(
            run_git(bare.to_str().unwrap(), &["rev-parse", "refs/heads/deploy"])
                .unwrap()
                .trim(),
            head_hash(r).unwrap()
        );

        // 非法分支名直接拒绝
        assert!(push_to_branch(r, "bad..name", false).is_err());
        assert!(push_to_branch(r, "-x", false).is_err());
        assert!(push_to_branch(r, "has space", false).is_err());

        std::fs::remove_dir_all(repo).ok();
        std::fs::remove_dir_all(bare).ok();
    }

    #[test]
    fn submodule_discovery_and_parent_pointer_state() {
        let root = std::env::temp_dir().join(format!(
            "ohmygitty-submodule-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let child = root.join("child");
        let parent = root.join("parent");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(&parent).unwrap();

        // 子模块仓库:一次初始提交
        run_git(child.to_str().unwrap(), &["init", "-q"]).unwrap();
        run_git(child.to_str().unwrap(), &["config", "user.email", "t@e.com"]).unwrap();
        run_git(child.to_str().unwrap(), &["config", "user.name", "t"]).unwrap();
        std::fs::write(child.join("a.txt"), "one\n").unwrap();
        run_git(child.to_str().unwrap(), &["add", "."]).unwrap();
        run_git(child.to_str().unwrap(), &["commit", "-qm", "init"]).unwrap();

        // 父仓库:加入子模块并提交
        run_git(parent.to_str().unwrap(), &["init", "-q"]).unwrap();
        run_git(parent.to_str().unwrap(), &["config", "user.email", "t@e.com"]).unwrap();
        run_git(parent.to_str().unwrap(), &["config", "user.name", "t"]).unwrap();
        std::fs::write(parent.join("readme.md"), "parent\n").unwrap();
        run_git(parent.to_str().unwrap(), &["add", "."]).unwrap();
        run_git(parent.to_str().unwrap(), &["commit", "-qm", "init"]).unwrap();
        run_git(
            parent.to_str().unwrap(),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                child.to_str().unwrap(),
                "sub",
            ],
        )
        .unwrap();
        run_git(parent.to_str().unwrap(), &["commit", "-qm", "add submodule"]).unwrap();

        let parent_s = parent.to_str().unwrap();
        let sub = parent.join("sub");
        let sub_s = sub.to_str().unwrap();

        // 发现:一条子模块,路径 sub,已初始化
        let subs = submodules(parent_s);
        assert_eq!(subs.len(), 1, "应发现一个子模块");
        assert_eq!(subs[0].path, "sub");
        assert!(subs[0].initialized);

        // 初始状态:父仓库已提交指针,无待同步改动
        let st = submodule_parent_state(sub_s);
        assert_eq!(st.parent.as_deref(), Some(parent_s));
        assert_eq!(st.rel_path, "sub");
        assert!(st.committed, "初始应已提交");
        assert!(!st.pointer_staged && !st.pointer_unstaged);
        assert_eq!(st.commits_ahead, 0);
        // 快速路径:调用方已给出 parent/rel 时,结果与现场发现一致
        let fast = submodule_parent_state_hinted(sub_s, Some(parent_s), Some("sub"));
        assert_eq!(fast.parent.as_deref(), Some(parent_s));
        assert_eq!(fast.rel_path, "sub");
        assert!(fast.committed);
        assert_eq!(fast.commits_ahead, 0);

        // 子模块产生新提交 → 父仓库指针未提交(未暂存)
        std::fs::write(sub.join("a.txt"), "two\n").unwrap();
        run_git(sub_s, &["add", "."]).unwrap();
        run_git(sub_s, &["commit", "-qm", "sub update"]).unwrap();
        let st = submodule_parent_state(sub_s);
        assert!(!st.committed);
        assert!(st.pointer_unstaged, "父仓库指针应未暂存");
        assert!(st.commits_ahead >= 1, "应有一个新提交");

        // 父仓库暂存指针 → 已暂存但未提交
        run_git(parent_s, &["add", "sub"]).unwrap();
        let st = submodule_parent_state(sub_s);
        assert!(!st.committed);
        assert!(st.pointer_staged, "父仓库指针应已暂存");
        assert!(!st.pointer_unstaged);
        assert!(submodule_parent_state_hinted(sub_s, Some(parent_s), Some("sub")).pointer_staged);

        // 父仓库提交指针 → 已提交
        run_git(parent_s, &["commit", "-qm", "update submodule pointer"]).unwrap();
        let st = submodule_parent_state(sub_s);
        assert!(st.committed, "父仓库提交后应无待提交指针");
        assert!(!st.pointer_staged && !st.pointer_unstaged);
        assert_eq!(st.commits_ahead, 0);

        // 非子模块仓库:parent 为 None
        let plain = submodule_parent_state(parent_s);
        assert!(plain.parent.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detects_worktree_relationships() {
        let base = temp_repo("worktree-main");
        let r = base.to_str().unwrap();
        // worktree add 需要一个初始提交
        std::fs::write(base.join("README.md"), "init\n").unwrap();
        stage_all(r).unwrap();
        commit(r, "init").unwrap();

        let linked = std::env::temp_dir().join(format!(
            "ohmygitty-test-worktree-linked-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&linked);
        let l = linked.to_str().unwrap();
        run_git(r, &["worktree", "add", "-b", "feature-x", l]).unwrap();

        // 从主仓库看:不是 linked,包含 2 个工作树,主仓库被正确标记
        let from_main = worktrees(r);
        assert!(!from_main.is_linked);
        assert_eq!(from_main.entries.len(), 2, "{:?}", from_main.entries);
        assert!(same_path(&from_main.main_path, r));
        assert_eq!(
            from_main.main_name,
            base.file_name().unwrap().to_string_lossy().to_string()
        );
        assert_eq!(
            from_main.entries.iter().filter(|e| e.is_main).count(),
            1,
            "应恰有一个主 worktree"
        );
        assert!(
            from_main
                .entries
                .iter()
                .any(|e| e.branch.as_deref() == Some("feature-x")),
            "应解析出从属工作树的分支"
        );

        // 从从属工作树看:is_linked=true,且仍能拿到完整列表与主仓库路径
        let from_linked = worktrees(l);
        assert!(from_linked.is_linked);
        assert_eq!(from_linked.entries.len(), 2);
        assert!(same_path(&from_linked.main_path, r));

        // 非仓库路径:返回空信息而非 panic
        let empty = worktrees(std::env::temp_dir().to_str().unwrap());
        assert!(empty.entries.is_empty());
        assert!(!empty.is_linked);

        run_git(r, &["worktree", "remove", l]).ok();
        std::fs::remove_dir_all(&linked).ok();
        std::fs::remove_dir_all(&base).ok();
    }
}
