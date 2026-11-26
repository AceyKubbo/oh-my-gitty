/* 侧栏:项目列表(渲染/拖拽排序/右键菜单/宽度模式) + 项目管理(打开/添加/克隆/移除/初始化)
   + 仓库摘要加载(loadRepos) + 后台 fetch + 仓库切换 */
import { $, invoke, toast, setButtonLoading, repoAvatarColor, settings, repos, repo, view, setRepos, setRepo, setNumBadge } from "./state.js";
import { refresh, showRefreshing, beginSwitchLoading, endSwitchLoading } from "./panel.js";
import { syncRunPanel, runningRepos } from "./run-panel.js";
import { syncRepoFileWatcher } from "./file-watcher.js";
import { refreshTip } from "./tooltip.js";

const SIDEBAR_MIN = 48, SIDEBAR_MAX = 420; // 侧栏拖拽宽度范围
const SIDEBAR_COMPACT_MAX = 96; // 简洁展示的宽度上限(含);更宽自动切全面展示

let fetching = false; // 后台 fetch 防重入:定时器与手动刷新共用
let ctxRepo = null; // 右键菜单当前目标项目路径(仅菜单打开期间有效)
let dragPath = null; // 正在拖拽的项目路径
let dragState = null; // pointerdown 待定状态(移动超过阈值前不视为拖拽)
let dragGhost = null; // 跟随指针的拖拽缩略图(克隆被拖项)
let suppressClick = false; // 拖拽结束后抑制一次合成 click,避免误切仓库

/* ===== 子模块树(父项目可折叠,子模块为叶子节点) ===== */
const normPath = (p) => String(p || "").replace(/\/+$/, ""); // 路径末尾斜杠归一,便于比较

// 已展开的父项目集合(持久化在 settings.submodule_expanded)
function expandedSet() {
  if (!Array.isArray(settings.submodule_expanded)) settings.submodule_expanded = [];
  return new Set(settings.submodule_expanded);
}

function toggleSubmoduleGroup(parentPath) {
  const set = expandedSet();
  if (set.has(parentPath)) set.delete(parentPath);
  else set.add(parentPath);
  settings.submodule_expanded = [...set];
  invoke("settings_save", { settings }).catch(() => {});
  renderRepoList(true); // 保持滚动位置:仅展开/折叠,不跳动
}

// 按路径查找侧栏条目:顶层项目本身,或其子模块/递归发现子仓库叶子(标题栏/高亮用)
export function findRepoEntry(path) {
  if (!path) return null;
  const target = normPath(path);
  for (const r of repos) {
    if (normPath(r.path) === target) return r;
    for (const s of r.submodules || []) {
      if (normPath(s.full_path) === target) {
        return { path: s.full_path, name: s.name, icon: null, parent: r.path, submodule: true, rel_path: s.path };
      }
    }
    for (const s of r.folder_children || []) {
      if (normPath(s.full_path) === target) {
        // auto_discovered:交互同子模块,但不展示父仓库 gitlink 指针提示
        return { path: s.full_path, name: s.name, icon: null, parent: r.path, submodule: true, auto_discovered: true, rel_path: s.path };
      }
    }
  }
  return null;
}

const SUBMODULE_GIT_SVG = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="2.5"/><circle cx="6" cy="18" r="2.5"/><circle cx="18" cy="12" r="2.5"/><path d="M8.5 6H14a4 4 0 0 1 4 4v0"/><path d="M8.5 18H14a4 4 0 0 0 4-4v0"/></svg>';
const SUBMODULE_CARET_SVG = '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6"/></svg>';

// worktree 标记:重叠方框表示「同一仓库的多个工作区」
const WORKTREE_SVG = '<svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="13" height="13" rx="1.5"/><path d="M8 21h10a3 3 0 0 0 3-3V8"/></svg>';

// 计算 worktree 关系在侧栏的展示信息;非 worktree 仓库或只有一个工作树时返回 null。
// 主仓库标出工作树数量,从属工作树标出自己的分支和来源主仓库。
function worktreeDisplay(r) {
  const wt = r.worktree;
  if (!wt || !Array.isArray(wt.entries) || wt.entries.length < 2) return null;
  const linked = !!wt.is_linked;
  const branch = wt.current_branch || null;
  const others = wt.entries.length - 1;
  const mainName = wt.main_name || "主仓库";
  return {
    linked,
    sub: linked
      ? `↳ ${branch || "detached"} · ${mainName}`
      : `主仓库 · ${others} 个工作树`,
    tip: linked
      ? `从属工作树 · 分支 ${branch || "分离 HEAD"} · 主仓库：${wt.main_path || mainName}`
      : `主仓库 · ${others} 个工作树：${wt.entries.filter((e) => !e.is_main).map((e) => e.branch || "detached").join("、")}`,
  };
}

/* ===== 项目管理 ===== */
export async function openLocalRepo() {
  const dir = await invoke("pick_folder");
  if (!dir) return;
  await addRepoByPath(dir);
}

// 按路径添加项目并切换(供打开本地/拖拽复用)
export async function addRepoByPath(dir) {
  // 同步切换到加载占位:隐藏空态 + 立即显示 refreshing(不等 150ms 延迟)。
  // 占位是绝对定位填满 #main,不参与布局——leaveOverview 显示 toolbar/run-panel
  // 压缩内容区时,占位始终填满,无空态位移/空白闪烁
  $("empty-state").classList.add("hidden");
  $("refreshing-state").classList.remove("hidden");
  const { leaveOverview } = await import("./dashboard.js"); // 拖拽添加可能发生在总览视图:切回仓库视图
  leaveOverview();
  let result;
  try {
    result = await invoke("repos_add", { path: dir });
    settings.repos = result.repos;
    // 后端可能刚写入虚拟分组;必须同步到前端 settings,否则随后的 settings_save
    // 会用旧值覆盖掉 folder_groups,导致子仓库叶子消失
    if (result.folder_groups) settings.folder_groups = result.folder_groups;
  } catch (e) {
    toast("添加失败：" + e, false);
    // 失败:收起占位,回到空态
    $("refreshing-state").classList.add("hidden");
    $("empty-state").classList.remove("hidden");
    return;
  }
  // 虚拟分组:根目录不是仓库,展开并切换到第一个子仓库,避免出现无内容的空态
  let target = result.path;
  if (result.is_folder_group) {
    const sums = await invoke("repos_status_all");
    const group = sums.find((r) => normPath(r.path) === normPath(result.path));
    const first = group && (group.folder_children || [])[0];
    if (first && first.initialized) target = first.full_path;
    if (group && !expandedSet().has(group.path)) {
      expandedSet().add(group.path);
      settings.submodule_expanded = [...expandedSet()];
      invoke("settings_save", { settings }).catch(() => {});
    }
  }
  setRepo(target);
  settings.last_repo = target;
  try { await invoke("repos_set_current", { path: target }); } catch (_) {}
  await loadRepos();
  await refresh();
  await syncRepoFileWatcher();
  syncRunPanel(); // 运行栏回填新项目的命令/日志(与 switchRepo 一致,添加也是一次切换)
  fetchRemote(); // 新添加的项目后台核对一次远程状态
}

// 拖拽文件夹到窗口添加项目
export function setupDragDrop() {
  const getWin = window.__TAURI__?.window?.getCurrentWindow;
  if (!getWin) return;
  getWin().onDragDropEvent((event) => {
    const p = event?.payload || event || {};
    if (p.type === "drop" && Array.isArray(p.paths) && p.paths.length) {
      addRepoByPath(p.paths[0]);
    }
  }).catch(() => { /* 拖拽不可用,静默 */ });
}

export function cloneRepo() {
  $("clone-url").value = "";
  $("clone-dest").value = "";
  $("dlg-clone").classList.remove("hidden");
  $("clone-url").focus();
}

export async function doClone() {
  const url = $("clone-url").value.trim();
  const dest = $("clone-dest").value.trim();
  if (!url) { toast("请输入仓库地址", false); return; }
  if (!dest) { toast("请选择本地目录", false); return; }
  setButtonLoading($("btn-clone-confirm"), true, "克隆中…");
  try {
    await invoke("git_clone", { url, dest });
    $("dlg-clone").classList.add("hidden");
    const result = await invoke("repos_add", { path: dest });
    settings.repos = result.repos;
    if (result.folder_groups) settings.folder_groups = result.folder_groups;
    setRepo(result.path);
    settings.last_repo = result.path;
    try { await invoke("repos_set_current", { path: result.path }); } catch (_) {}
    await loadRepos();
    await refresh();
    await syncRepoFileWatcher();
    syncRunPanel(); // 运行栏回填新项目的命令/日志(与 switchRepo 一致,克隆也是一次切换)
    toast("克隆成功", true);
    fetchRemote(); // 克隆后核对远程跟踪分支,徽标立即可用
  } catch (e) { toast(String(e), false); }
  finally { setButtonLoading($("btn-clone-confirm"), false); }
}

export async function loadRepos() {
  try { setRepos(await invoke("repos_status_all")); }
  catch (_) { /* 扫描失败时保留现有列表,不清空 */ }
  syncSidebarVisibility(); // 无项目时隐藏侧栏,有项目恢复
  renderRepoList();
  if (view === "overview") {
    const { renderDashboard } = await import("./dashboard.js");
    renderDashboard(); // 总览态:摘要更新后总览行同步重渲染
  }
}

// 无项目时隐藏侧栏与底部运行栏(初始空态界面更聚焦),添加项目后自动恢复
export function syncSidebarVisibility() {
  const empty = repos.length === 0;
  $("sidebar").classList.toggle("hidden", empty);
  // 首页没有当前项目,即使项目列表存在也不显示项目级运行栏
  $("run-panel").classList.toggle("hidden", empty || view === "overview");
}

/* ===== 后台 fetch ===== */
// 后台静默 fetch:更新本地远程跟踪分支,让 push/pull 徽标与远程历史反映最新状态。
// 失败静默(无远程/无网络/未配置凭据),不打断用户操作;完成后就地刷新当前仓库。
export async function fetchRemote(targetRepo) {
  // 可指定目标仓库:切库后延迟触发时,即使期间又切了库也不会把旧库数据刷到新界面。
  const target = targetRepo || repo;
  if (!target || fetching) return;
  fetching = true;
  try {
    const r = await invoke("git_fetch", { repo: target });
    if (r && r.ok === false) return; // 无远程/无网络/认证失败等:静默忽略
    if (repo !== target) return;
    // fetch 后重读状态,更新 ahead/behind 徽标;总览视图下改刷摘要(总览行经 loadRepos 重渲染)
    if (view === "repo") await refresh();
    else await loadRepos();
  } catch (_) { /* 静默 */ }
  finally { fetching = false; }
}

/* ===== 仓库切换 ===== */
export async function switchRepo(path) {
  if (path === repo && view === "repo") return;
  // 一个整体加载态覆盖下面所有步骤;异常时也保证收起,不会把界面卡在 loading。
  beginSwitchLoading();
  try {
    const { leaveOverview } = await import("./dashboard.js"); // 从总览点进项目:恢复仓库级界面
    leaveOverview();
    setRepo(path);
    settings.last_repo = path;
    try { await invoke("repos_set_current", { path }); } catch (_) {}
    // 目标是子模块时,若父项目分组处于收起态则自动展开,保证叶子高亮可见
    const entry = findRepoEntry(path);
    if (entry && entry.submodule && entry.parent && !expandedSet().has(entry.parent)) {
      const set = expandedSet();
      set.add(entry.parent);
      settings.submodule_expanded = [...set];
      invoke("settings_save", { settings }).catch(() => {});
      renderRepoList(true);
    }
    updateSidebarActive(path); // 只切高亮与滚动,不重建列表(重建会让全部状态行闪烁)
    await refresh();
    await syncRepoFileWatcher();
    syncRunPanel(); // 切到新仓库:回填其运行命令、刷新日志与运行态
    // 提交面板跟随项目:切走隐藏,切回恢复(动态 import 断开与 git-ops 的循环依赖)
    import("./git-ops.js").then(({ syncCommitPop }) => syncCommitPop()).catch(() => {});
    // 后台核对远程状态延后触发:避免 fetch + 二次 refresh 与切库本身的 git 进程
    // 抢占同一仓库锁,也不会在加载动画刚消失时立刻引起一次整屏重绘。
    setTimeout(() => fetchRemote(path), 600);
  } finally {
    await endSwitchLoading();
  }
}

export async function removeRepo(path) {
  try { settings.repos = await invoke("repos_remove", { path }); }
  catch (e) { toast("关闭失败：" + e, false); return; }
  if (repo === path) {
    setRepo(settings.repos[0] || null);
    settings.last_repo = repo;
    await syncRepoFileWatcher();
  }
  await loadRepos(); // 总览视图下:列表与总览行已同步更新,主区保持总览不动
  if (view === "overview") return;
  if (repo) {
    await refresh(); // 关闭的是当前项目:自动切到首个剩余项目
    syncRunPanel();  // 运行栏跟随切换(命令/日志/运行态不再残留已关闭项目)
  } else {
    syncRunPanel(); // 已无项目:清空运行栏残留的旧项目命令与日志
    const { showEmpty } = await import("./panel.js");
    await showEmpty(false);
  }
}

export async function initRepo() {
  if (!repo) return;
  setButtonLoading($("btn-init"), true, "初始化…");
  try {
    await invoke("git_init", { repo });
    toast("仓库已初始化", true);
  } catch (e) {
    toast(String(e), false);
  } finally {
    setButtonLoading($("btn-init"), false);
  }
  await refresh();
  await loadRepos();
}

/* ===== 列表渲染 ===== */
export function renderRepoList(keepScroll = false) {
  const ul = $("repo-list");
  const scrollTop = ul.scrollTop; // 整表重建会重置滚动,保存并在渲染后恢复(拖拽排序/后台刷新不跳动)
  ul.innerHTML = "";
  // 过滤:只看待提交(更改+冲突;与图标角标同口径)。数据仍用完整列表,过滤仅影响渲染,
  // 拖拽排序/切库/总览均基于全量,不受影响
  const dirtyOnly = !!settings.sidebar_dirty_only;
  const list = dirtyOnly ? repos.filter((r) => (r.unstaged || 0) + (r.conflicts || 0) > 0) : repos;
  const empty = $("sidebar-empty");
  empty.textContent = repos.length === 0 ? "暂无项目，点击 + 添加" : "没有待提交的项目";
  empty.classList.toggle("hidden", !(repos.length === 0 || (dirtyOnly && list.length === 0)));
  $("btn-dirty-only").classList.toggle("on", dirtyOnly);
  $("btn-dirty-only").dataset.tip = dirtyOnly ? "显示全部项目" : "只显示待提交项目";
  refreshTip($("btn-dirty-only")); // 悬停中切换过滤:提示文案随状态就地更新
  const running = new Set(runningRepos().map((x) => x.repo)); // 运行中项目集(绿点标识)
  for (const r of list) {
    const li = document.createElement("li");
    // 真实子模块 + 递归发现的子仓库统一作为叶子节点(共用折叠树交互)
    const subs = [...(r.submodules || []), ...(r.folder_children || [])];
    const expanded = subs.length > 0 && expandedSet().has(r.path);
    const isFolderGroup = !r.is_repo && (r.folder_children || []).length > 0;
    // 总览视图下不标记任何项目为选中(选中态属于「总览」入口),避免重渲染时残留旧高亮
    li.className = "repo-item" + (view === "repo" && normPath(r.path) === normPath(repo) ? " active" : "");
    li.title = isFolderGroup
      ? r.path + ` · 文件夹（含 ${subs.length} 个子仓库）`
      : (r.is_repo ? r.path : r.path + " · 不是 Git 仓库");
    li.dataset.path = r.path; // 拖拽落点定位用

    const ic = document.createElement("span");
    ic.className = "repo-icon-wrap";
    if (r.icon) {
      const img = document.createElement("img");
      img.className = "repo-icon";
      img.src = r.icon;
      img.alt = "";
      img.draggable = false; // 避免拖到图标时触发图片原生拖拽
      ic.appendChild(img);
    } else {
      // 无真实图标:取项目名首字符做字母头像,配色按路径哈希稳定分配(同项目颜色不变)
      const c = repoAvatarColor(r.path);
      const av = document.createElement("span");
      av.className = "repo-avatar";
      av.style.background = c.bg;
      av.style.color = c.fg;
      av.textContent = ((r.name || r.path).trim().charAt(0) || "?").toUpperCase();
      ic.appendChild(av);
    }
    // 图标右上角更改数角标(更改+冲突;暂存不计;0 不显示)
    const dirty = (r.unstaged || 0) + (r.conflicts || 0);
    if (dirty > 0) {
      const badge = document.createElement("span");
      ic.appendChild(badge);
      setNumBadge(badge, dirty, "repo-icon-badge");
    }

    const info = document.createElement("div");
    info.className = "repo-info";
    const name = document.createElement("span");
    name.className = "repo-name";
    name.textContent = r.name;
    info.appendChild(name);

    // worktree 关系:图标角标 + 名称下方的关系副标题(简洁模式仅保留角标)
    const wtInfo = worktreeDisplay(r);
    if (wtInfo) {
      const wbadge = document.createElement("span");
      wbadge.className = "worktree-badge" + (wtInfo.linked ? " linked" : " main");
      wbadge.innerHTML = WORKTREE_SVG;
      wbadge.dataset.tip = wtInfo.tip;
      ic.appendChild(wbadge);
      const sub = document.createElement("span");
      sub.className = "repo-sublabel";
      sub.textContent = wtInfo.sub;
      sub.dataset.tip = wtInfo.tip;
      info.appendChild(sub);
      li.classList.add(wtInfo.linked ? "is-worktree" : "has-worktrees");
    }

    // 运行中项目:行尾静态绿点(隐藏用 .hidden 占位,运行态变化由 updateRepoRunDots 就地切换)
    const dot = document.createElement("span");
    dot.className = "repo-run-dot" + (running.has(r.path) ? "" : " hidden");
    dot.title = "运行中";

    li.append(ic, info, dot);
    // 有子模块:行尾加展开/折叠按钮(点击不切换仓库)
    if (subs.length > 0) {
      li.classList.add("has-submodules");
      const tg = document.createElement("button");
      tg.className = "submodule-toggle" + (expanded ? " open" : "");
      tg.type = "button";
      tg.title = `${expanded ? "收起" : "展开"} ${subs.length} 个${isFolderGroup ? "子仓库" : "子模块"}`;
      tg.innerHTML = SUBMODULE_CARET_SVG;
      tg.addEventListener("click", (e) => { e.stopPropagation(); toggleSubmoduleGroup(r.path); });
      li.appendChild(tg);
    }
    li.addEventListener("click", () => {
      if (suppressClick) { suppressClick = false; return; } // 拖拽刚结束:吞掉合成 click
      // 文件夹分组本身不是仓库:展开分组并切换到第一个子仓库,避免出现无内容的空态
      if (isFolderGroup) {
        if (!expandedSet().has(r.path)) toggleSubmoduleGroup(r.path);
        const first = (r.folder_children || [])[0];
        if (first && first.initialized) { switchRepo(first.full_path); return; }
      }
      switchRepo(r.path);
    });
    // 右键:弹出项目菜单(目前仅「关闭项目」,不切换选中)
    li.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      openCtxMenu(e.clientX, e.clientY, r.path);
    });
    // pointer 实现拖拽:按下记录起点,移动超阈值才拖(普通点击仍可切换仓库)
    li.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return; // 仅左键
      suppressClick = false;
      dragState = { path: r.path, li, x: e.clientX, y: e.clientY };
    });
    ul.appendChild(li);
    // 展开态:子模块作为叶子节点跟在父项目下方,点击切换到该子模块的 Git 仓库
    if (expanded) {
      for (const s of subs) {
        const leaf = document.createElement("li");
        leaf.className = "repo-item submodule-item" + (view === "repo" && normPath(s.full_path) === normPath(repo) ? " active" : "");
        leaf.dataset.path = s.full_path;
        leaf.dataset.submodule = "1";
        if (s.auto_discovered) leaf.dataset.folderChild = "1";
        leaf.title = s.full_path + (s.initialized ? "" : " · 未初始化");
        const gi = document.createElement("span");
        gi.className = "submodule-git-icon";
        gi.innerHTML = SUBMODULE_GIT_SVG;
        const si = document.createElement("div");
        si.className = "repo-info";
        const sn = document.createElement("span");
        sn.className = "repo-name";
        sn.textContent = s.name || s.path;
        si.appendChild(sn);
        leaf.append(gi, si);
        // 父仓库指针落后于子模块('+'=有新提交未同步,'U'=冲突):橙色提示点。
        // 递归发现的子仓库不涉及父仓库 gitlink,不展示该提示点。
        if (!s.auto_discovered && (s.state === "+" || s.state === "U")) {
          const sync = document.createElement("span");
          sync.className = "submodule-sync-dot";
          sync.title = "子模块有改动尚未在父仓库提交";
          leaf.appendChild(sync);
        }
        leaf.addEventListener("click", () => {
          if (!s.initialized) { toast("该子模块尚未初始化，请先在终端执行 git submodule update --init", false); return; }
          switchRepo(s.full_path);
        });
        ul.appendChild(leaf);
      }
    }
  }
  ul.scrollTop = scrollTop; // 恢复原滚动位置(整表重建会重置)
  // 拖拽重排(keepScroll):严格停在原位置,不做当前项滚动(选中项不在视口时会拉走视图)
  if (keepScroll) return;
  // 注意:不做选中项 scrollIntoView——选中项滚动由 updateSidebarActive(切换仓库)负责。
  // 后台刷新(fetch/状态轮询)重建列表时若强制拉回选中项,会打断用户主动滚动(闪烁/无法滚到底部)
}

// 就地同步项目列表行尾「运行中」绿点:运行态由 run-panel 维护,其状态变化时调用,不重建列表
export function updateRepoRunDots() {
  const running = new Set(runningRepos().map((x) => x.repo));
  for (const li of $("repo-list").children) {
    const dot = li.querySelector(".repo-run-dot");
    if (dot) dot.classList.toggle("hidden", !running.has(li.dataset.path));
  }
}

// 按当前宽度自动切换简洁(仅图标)与全面(名称+路径)展示。
// 简洁时「添加项目」按钮移到项目列表下方,全面时回到头部。
export function applySidebarMode() {
  const sb = $("sidebar");
  const w = parseInt(sb.style.width, 10);
  const compact = !isNaN(w) && w <= SIDEBAR_COMPACT_MAX;
  sb.classList.toggle("collapsed", compact);
  if (compact) {
    $("repo-list").after($("btn-add-repo"));
  } else {
    $("sidebar-head").appendChild($("btn-add-repo"));
  }
  $("btn-toggle-sidebar").title = compact ? "展开侧边栏" : "收起侧边栏";
  return compact;
}

// 切换仓库时只更新高亮与滚动,不重建列表
export function updateSidebarActive(path) {
  for (const li of $("repo-list").children) {
    li.classList.toggle("active", normPath(li.dataset.path) === normPath(path));
  }
  const active = $("repo-list").querySelector(".repo-item.active");
  if (active) active.scrollIntoView({ block: "nearest" });
}

// 顶部标题栏:当前项目图标 + 名称(数据复用侧栏仓库摘要,无需额外请求)
export function updateProjectHeader() {
  const hdr = $("project-header");
  const cur = findRepoEntry(repo); // 顶层项目或其子模块叶子
  if (!cur) { hdr.classList.add("hidden"); return; }
  hdr.classList.remove("hidden");
  const wrap = $("proj-icon-wrap");
  wrap.innerHTML = "";
  if (cur.icon) {
    const img = document.createElement("img");
    img.className = "proj-icon";
    img.src = cur.icon;
    img.alt = "";
    wrap.appendChild(img);
  } else {
    // 无真实图标:取项目名首字符做字母头像,配色与侧栏一致(同项目颜色不变)
    const c = repoAvatarColor(cur.path);
    const av = document.createElement("span");
    av.className = "proj-avatar";
    av.style.background = c.bg;
    av.style.color = c.fg;
    av.textContent = ((cur.name || cur.path).trim().charAt(0) || "?").toUpperCase();
    wrap.appendChild(av);
  }
  $("proj-name").textContent = cur.name || cur.path;
  $("proj-path").textContent = cur.path;
  $("proj-path").title = cur.path;
}

/* ===== 右键菜单 ===== */
// 在鼠标位置弹出侧栏右键菜单,位置钳制在视口内(卡片「更多」按钮也复用此菜单)
export function openCtxMenu(x, y, path) {
  const menu = $("ctx-menu");
  ctxRepo = path;
  menu.classList.remove("hidden");
  // 清掉弹窗前可能残留的文本选区(侧栏虽禁用选择,但右键拖选仍可能在部分平台留下选区)
  const sel = window.getSelection();
  if (sel && !sel.isCollapsed) sel.removeAllRanges();
  const r = menu.getBoundingClientRect();
  menu.style.left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4)) + "px";
  menu.style.top = Math.max(4, Math.min(y, window.innerHeight - r.height - 4)) + "px";
}
export function closeCtxMenu() {
  $("ctx-menu").classList.add("hidden");
  ctxRepo = null;
}

/* ===== 拖拽排序 ===== */
// 清除所有项目的落点指示线
function clearDropIndicators() {
  for (const el of $("repo-list").querySelectorAll(".drop-above,.drop-below")) {
    el.classList.remove("drop-above", "drop-below");
  }
}

// 根据纵坐标定位指针下的项目元素
function repoItemAtY(y) {
  // 子模块叶子不参与顶层项目的拖拽排序
  for (const it of $("repo-list").querySelectorAll(".repo-item:not(.submodule-item)")) {
    const r = it.getBoundingClientRect();
    if (y >= r.top && y <= r.bottom) return it;
  }
  return null;
}

// 拖拽移动:超过阈值才进入拖拽,之后实时刷新落点指示(Tauri 默认拦截 HTML5 drop,故改用 pointer)
function onRepoDragMove(e) {
  if (!dragState) return;
  if (!dragPath) {
    const dx = e.clientX - dragState.x, dy = e.clientY - dragState.y;
    if (dx * dx + dy * dy < 16) return; // 4px 阈值:与点击区分
    dragPath = dragState.path;
    dragState.li.classList.add("dragging");
    createDragGhost();
  }
  e.preventDefault(); // 抑制拖动过程中浏览器的默认文本选中
  moveDragGhost(e.clientX, e.clientY);
  clearDropIndicators();
  const target = repoItemAtY(e.clientY);
  if (target && target !== dragState.li) {
    const rect = target.getBoundingClientRect();
    target.classList.add(e.clientY - rect.top < rect.height / 2 ? "drop-above" : "drop-below");
  }
}

// 缩略图:克隆被拖项为实底卡片跟随指针;挂在 body 下不参与落点计算(pointer-events: none)
function createDragGhost() {
  dragGhost = dragState.li.cloneNode(true);
  dragGhost.className = "repo-item repo-drag-ghost"; // 重置类:不带 dragging/选中态
  dragGhost.style.width = dragState.li.offsetWidth + "px";
  document.body.appendChild(dragGhost);
}
function moveDragGhost(x, y) {
  if (!dragGhost) return;
  dragGhost.style.left = x + "px";
  dragGhost.style.top = y + "px";
}

// 拖拽松手:按落点重排,并复位状态
function onRepoDragUp(e) {
  if (dragPath && dragState) {
    const target = repoItemAtY(e.clientY);
    if (target && target !== dragState.li) {
      const rect = target.getBoundingClientRect();
      reorderRepo(dragPath, target.dataset.path, e.clientY - rect.top < rect.height / 2);
    }
    dragState.li.classList.remove("dragging");
    clearDropIndicators();
    suppressClick = true;
    dragPath = null;
  }
  if (dragGhost) { dragGhost.remove(); dragGhost = null; }
  dragState = null;
}

// 拖拽落定:把 src 移到 target 之前(before=true)/之后,就地重排两份列表并持久化
function reorderRepo(srcPath, targetPath, before) {
  const srcIdx = repos.findIndex((r) => r.path === srcPath);
  if (srcIdx < 0) return;
  const [moved] = repos.splice(srcIdx, 1);
  let dstIdx = repos.findIndex((r) => r.path === targetPath);
  if (dstIdx < 0) repos.push(moved);
  else {
    if (!before) dstIdx += 1;
    repos.splice(dstIdx, 0, moved);
  }
  // settings.repos 仅存路径,顺序即展示顺序;重排后持久化,下次加载沿用
  settings.repos = repos.map((r) => r.path);
  invoke("settings_save", { settings }).catch(() => {});
  renderRepoList(true); // 保持滚动位置:拖拽落定视图不跳动
}

/* ===== 事件绑定 ===== */
export function bindSidebarEvents() {
  $("btn-open2").addEventListener("click", openLocalRepo);
  $("btn-add-repo").addEventListener("click", async () => {
    setRepo(null); // 进入「新建项目」态:取消旧选中,标题栏与工具栏隐藏,右侧面板显示空态
    renderRepoList(); // 清掉列表旧选中高亮
    syncRunPanel(); // 清空底部运行栏(旧项目的命令与日志)
    const { showEmpty } = await import("./panel.js");
    showEmpty(false);
  });

  // 侧栏过滤:只看待提交项目(状态持久化,后台刷新时列表自动跟随最新更改)
  $("btn-dirty-only").addEventListener("click", () => {
    settings.sidebar_dirty_only = !settings.sidebar_dirty_only;
    invoke("settings_save", { settings }).catch(() => {});
    renderRepoList(true); // 保持滚动位置:切换过滤视图不跳动
  });
  $("btn-clone").addEventListener("click", cloneRepo);
  $("clone-dest").addEventListener("click", async () => {
    const dir = await invoke("pick_folder");
    if (dir) $("clone-dest").value = dir;
  });
  $("btn-clone-confirm").addEventListener("click", doClone);
  $("btn-clone-cancel").addEventListener("click", () => $("dlg-clone").classList.add("hidden"));
  $("btn-init").addEventListener("click", initRepo);
  // 侧栏右键菜单:打开本地目录(文件管理器)
  $("ctx-open-dir").addEventListener("click", () => {
    const path = ctxRepo;
    closeCtxMenu();
    if (path) invoke("open_local_dir", { path }).catch((e) => toast(String(e), false));
  });
  // 侧栏右键菜单:关闭当前右键目标项目
  $("ctx-close").addEventListener("click", () => {
    const path = ctxRepo;
    closeCtxMenu();
    if (path) removeRepo(path);
  });

  // 侧栏宽度拖拽调整(宽度同时决定简洁/全面展示,见 applySidebarMode)
  let sidebarDragStartX = 0, sidebarDragStartW = 0;
  const onSidebarDrag = (e) => {
    const w = Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, sidebarDragStartW + e.clientX - sidebarDragStartX));
    $("sidebar").style.width = w + "px";
  };
  const endSidebarDrag = () => {
    document.body.classList.remove("resizing");
    document.removeEventListener("pointermove", onSidebarDrag);
    settings.sidebar_width = parseInt($("sidebar").style.width, 10);
    settings.sidebar_collapsed = $("sidebar").classList.contains("collapsed");
    invoke("settings_save", { settings }).catch(() => {});
    window.dispatchEvent(new Event("dashboard-layout-change"));
  };
  // 侧栏收起/展开:在简洁(48px)与上次宽度间切换
  $("btn-toggle-sidebar").addEventListener("click", () => {
    const sb = $("sidebar");
    if (sb.classList.contains("collapsed")) {
      // 展开:恢复到记忆宽度,至少为默认 172(避免从拖窄的简洁态展开到过窄宽度)
      sb.style.width = Math.max(settings.sidebar_width || 172, 172) + "px";
    } else {
      settings.sidebar_width = parseInt(sb.style.width, 10) || 172;
      sb.style.width = "48px";
    }
    applySidebarMode();
    settings.sidebar_collapsed = sb.classList.contains("collapsed");
    invoke("settings_save", { settings }).catch(() => {});
    window.dispatchEvent(new Event("dashboard-layout-change"));
  });

  $("sidebar-resizer").addEventListener("pointerdown", (e) => {
    e.preventDefault();
    sidebarDragStartX = e.clientX;
    sidebarDragStartW = $("sidebar").getBoundingClientRect().width;
    document.body.classList.add("resizing");
    document.addEventListener("pointermove", onSidebarDrag);
    document.addEventListener("pointerup", endSidebarDrag, { once: true });
  });

  // 侧栏宽度变化(拖拽调整、收起切换、初始布局)时自动切换简洁/全面展示
  new ResizeObserver(() => { applySidebarMode(); }).observe($("sidebar"));

  // 侧栏拖拽排序:按下在各 li,移动/松手由文档统一接收(指针移出原项也能持续追踪)
  document.addEventListener("pointermove", onRepoDragMove);
  document.addEventListener("pointerup", onRepoDragUp);
}
