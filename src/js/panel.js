/* 仓库面板视图:刷新主区(状态+历史)、空态、文件列表(暂存/更改/冲突)、diff 视图、
   忽略规则对话框、「更多」菜单 */
import { $, invoke, toast, setButtonLoading, STATUS_CHARS, repo, view, repos, settings, lastShipStatus, setLastShipStatus, setNumBadge } from "./state.js";
import { updateProjectHeader, renderRepoList, removeRepo } from "./sidebar.js";
import { renderHistory } from "./history.js";
import { toggleStage, stagePaths, discardFile, resolveOne, stageAll, discardAll, invalidateBranchCache, syncOpBanner } from "./git-ops.js";

const sectionUserSet = new Set(); // 用户手动切换过的分组:自动收起规则不再覆盖其状态

/* ===== 主区刷新 ===== */
export async function refresh() {
  if (!repo) return;
  const targetRepo = repo;
  // 切换了仓库:右侧 diff 面板属于旧仓库,关闭
  if (diffRepo && diffRepo !== targetRepo) closeDiffPanel();
  invalidateBranchCache(); // 仓库状态已变(切分支/推送/拉取等),分支缓存作废
  updateProjectHeader(); // 顶部标题栏:当前项目图标 + 名称
  // 一次拉取合并的 status + history(后端复用 status 的 branch 信息,省去重复 git 进程)
  const data = await invoke("git_refresh", { repo: targetRepo });
  // 总览/切换项目期间,旧请求返回的数据不能重新显示项目级工具栏和面板
  if (view !== "repo" || repo !== targetRepo) return;
  const st = data.status, hist = data.history;
  // 进行中的合并/挑拣/回退:常驻提示条(冲突解决后可继续或终止)
  syncOpBanner(data.operation, st.conflicts.length);
  // 就地同步侧栏摘要:角标(未暂存+未跟踪+冲突)/领先落后数/最近提交时间随面板操作即时更新,
  // 不必等全量重扫(暂存一批更改后左侧数字立刻变化);字段口径与后端 summarize 一致
  const sum = repos.find((r) => r.path === targetRepo);
  if (sum) {
    sum.branch = st.branch || null;
    sum.ahead = st.ahead;
    sum.behind = st.behind;
    sum.staged = st.staged.length;
    sum.unstaged = st.unstaged.length + st.untracked.length;
    sum.conflicts = st.conflicts.length;
    sum.is_repo = st.is_repo;
    const head = hist && hist[0];
    if (head && head.timestamp) sum.last_commit_ts = head.timestamp;
    renderRepoList();
  }
  if (!st.is_repo) {
    showEmpty(true);
    return;
  }
  showPanel(st);
  renderHistory(hist, st.branch);
}

// 切换仓库的加载占位:延迟 150ms 显示(毫秒级切换不闪 spinner),一旦显示至少停留 250ms(避免抖一下)
let refreshingTimer = null;
let refreshingShownAt = 0;
const REFRESH_DELAY_MS = 150;
const REFRESH_MIN_MS = 250;

export function showRefreshing() {
  clearTimeout(refreshingTimer);
  refreshingTimer = setTimeout(() => {
    refreshingShownAt = Date.now();
    $("empty-state").classList.add("hidden");
    $("panel").classList.add("hidden");
    $("refreshing-state").classList.remove("hidden");
  }, REFRESH_DELAY_MS);
}

// 藏起占位:从未显示则直接返回;显示不足最短时长则延后,resolve 时占位已彻底隐藏
function hideRefreshing() {
  clearTimeout(refreshingTimer);
  const el = $("refreshing-state");
  if (el.classList.contains("hidden")) return Promise.resolve();
  const wait = REFRESH_MIN_MS - (Date.now() - refreshingShownAt);
  if (wait <= 0) { el.classList.add("hidden"); return Promise.resolve(); }
  return new Promise((resolve) => setTimeout(() => { el.classList.add("hidden"); resolve(); }, wait));
}

export async function showEmpty(showInit) {
  const { leaveOverview } = await import("./dashboard.js");
  leaveOverview();
  closeDiffPanel(); // 离开仓库视图,右侧编辑器不再有归属
  $("sec-history").classList.add("hidden"); // 空态不展示历史
  syncEditorToggle();
  // 无项目时隐藏标题栏;当前选中项不是 Git 仓库时仍保留(项目已选定)
  $("project-header").classList.toggle("hidden", !repo);
  $("toolbar").classList.add("hidden"); // 空态无 Git 操作,隐藏顶部工具栏
  $("panel").classList.add("hidden");
  await hideRefreshing();
  $("empty-state").classList.remove("hidden");
  if (showInit) {
    // 当前选中的不是 Git 仓库:简洁提示 + 仅保留初始化入口
    $("empty-title").textContent = "不是 Git 仓库";
    $("empty-desc").textContent = "此文件夹尚未初始化为 Git 仓库";
    $("btn-init").classList.remove("hidden");
    $("btn-open2").classList.add("hidden");
    $("btn-clone").classList.add("hidden");
  } else {
    // 尚未打开任何仓库
    $("empty-title").textContent = "打开一个项目";
    $("empty-desc").textContent = "选择本地文件夹，或从远程仓库克隆";
    $("btn-init").classList.add("hidden");
    $("btn-open2").classList.remove("hidden");
    $("btn-clone").classList.remove("hidden");
  }
  $("branch-name").textContent = "—";
  $("branch-name").title = "";
  // 无仓库:提交/推送/拉取按钮复位为次要态(不禁用,点击有 toast 提示)
  setLastShipStatus(null);
  for (const id of ["btn-commit", "btn-push", "btn-pull"]) {
    const b = $(id);
    if (b) b.classList.remove("primary");
  }
  if ($("push-count")) {
    setNumBadge($("push-count"), 0);
    $("push-count").classList.add("hidden");
  }
  if ($("pull-count")) $("pull-count").classList.add("hidden");
}

export async function showPanel(st) {
  $("empty-state").classList.add("hidden");
  $("toolbar").classList.remove("hidden"); // 仓库面板恢复顶部工具栏
  await hideRefreshing();
  $("panel").classList.remove("hidden");
  $("sec-history").classList.remove("hidden"); // 版本历史位于侧栏底部,仓库视图显示

  renderList("conflict-list", st.conflicts, "conflict", $("conflict-count"));
  // 暂存计数弱化显示:灰字无胶囊(暂存是中间态,提示强度低于实际更改)
  renderList("staged-list", st.staged, "staged", $("staged-count"), "dim");
  renderList("unstaged-list", [...st.unstaged, ...st.untracked], "unstaged", $("unstaged-count"));

  // 列表为空时禁用对应整组操作按钮:取消全部暂存←暂存数;取消修改←全部本地改动;全部暂存←未暂存改动
  $("btn-unstage-all").disabled = st.staged.length === 0;
  $("btn-discard-all").disabled = st.staged.length + st.unstaged.length + st.untracked.length === 0;
  $("btn-stage-all").disabled = st.unstaged.length + st.untracked.length === 0;

  $("sec-conflicts").classList.toggle("hidden", st.conflicts.length === 0);
  $("btn-ai-resolve").disabled = st.conflicts.length === 0;
  // 分组默认展开/收起:历史始终收起;暂存/更改按是否有内容决定。
  // 仅作为默认展示规则——用户手动切换过的分组保持其状态,刷新不覆盖。
  if (!sectionUserSet.has("sec-staged")) setSectionOpen("sec-staged", "staged-list", st.staged.length > 0);
  if (!sectionUserSet.has("sec-unstaged")) setSectionOpen("sec-unstaged", "unstaged-list", st.unstaged.length + st.untracked.length > 0);
  if (!sectionUserSet.has("sec-history")) setSectionOpen("sec-history", "history-container", true);
  syncEditorToggle(); // 右侧编辑器可能曾被 ESC/总览关闭:按实际显隐复位开关
  // 右侧面板显示当前分支
  $("branch-name").textContent = st.detached ? "（分离）" : (st.branch || "（无分支）");
  $("branch-name").title = st.branch || "";
  setLastShipStatus(st);
  setCommitButton(st);
  setPushButton(st);
  // 拉取按钮同规则:有落后 → 主按钮样式;已同步 → 正常次要样式(永不禁用,点击有 toast 提示)
  const pb = $("btn-pull"), lc = $("pull-count");
  if (pb && !pb.classList.contains("loading")) {
    const dirty = st.behind > 0;
    pb.disabled = false;
    pb.title = dirty ? "拉取远程新提交" : "没有可拉取的提交";
    pb.classList.toggle("primary", dirty);
  }
  if (lc) {
    if (st.behind > 0) { setNumBadge(lc, st.behind, "red"); lc.classList.remove("hidden"); }
    else lc.classList.add("hidden");
  }
}

// 设置分组展开/收起(open=true 展开):同步 .collapsed 与目标列表显隐
function setSectionOpen(secId, targetId, open) {
  $(secId).classList.toggle("collapsed", !open);
  $(targetId).classList.toggle("hidden", !open);
}

// 提交按钮:有已暂存更改 → 主按钮样式;未暂存/干净 → 正常次要样式(永不禁用,空操作走 toast)
function setCommitButton(st) {
  const btn = $("btn-commit");
  if (!btn || btn.classList.contains("loading")) return; // loading 中不重建,避免打断操作
  const pending = st.staged.length + st.unstaged.length + st.untracked.length;
  btn.disabled = false;
  btn.title = st.staged.length > 0 ? "提交已暂存的更改"
    : (pending > 0 ? "请先暂存更改" : "没有可提交的更改");
  btn.classList.toggle("primary", st.staged.length > 0);
}

// 推送按钮:本地领先 → 主按钮样式;无领先 → 正常次要样式(永不禁用,空操作走 toast)
function setPushButton(st) {
  const btn = $("btn-push");
  if (!btn || btn.classList.contains("loading")) return;
  const count = $("push-count");
  const dirty = st.ahead > 0;
  btn.disabled = false;
  btn.title = dirty ? "推送本地领先的提交" : "没有待推送的提交";
  btn.classList.toggle("primary", dirty);
  if (count) {
    if (dirty) { setNumBadge(count, st.ahead); count.classList.remove("hidden"); }
    else count.classList.add("hidden");
  }
}

// 操作完成后的 loading 复位会还原按钮旧 DOM,这里按最新状态重建,避免显示态错乱
export function refreshShipButtons() {
  if (lastShipStatus) {
    setCommitButton(lastShipStatus);
    setPushButton(lastShipStatus);
  }
}

/* ===== 文件树(暂存/更改/冲突) ===== */
// 目录默认展开;用户收起的目录记入此集合(key = kind + ":" + 目录路径),刷新后保留状态
const folderClosed = new Set();

// 由扁平的文件列表构建目录树:目录节点 children 为 Map,文件节点保存原始 entry
function buildTree(entries) {
  const root = { type: "dir", children: new Map(), count: 0, depth: -1 };
  for (const e of entries) {
    const parts = String(e.path || "").split("/");
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const seg = parts[i];
      if (!seg) continue;
      let next = node.children.get(seg);
      if (!next) {
        next = { type: "dir", name: seg, path: parts.slice(0, i + 1).join("/"), depth: i, children: new Map(), count: 0 };
        node.children.set(seg, next);
      }
      next.count++;
      node = next;
    }
    // 文件以完整路径为 key,避免同一目录下同名文件相互覆盖
    node.children.set("f:" + e.path, { type: "file", entry: e });
  }
  return root;
}

// 目录在前、文件在后,同类按名称排序
function sortedChildren(node) {
  return [...node.children.values()].sort((a, b) => {
    if (a.type !== b.type) return a.type === "dir" ? -1 : 1;
    const an = a.type === "dir" ? a.name : a.entry.path;
    const bn = b.type === "dir" ? b.name : b.entry.path;
    return an.localeCompare(bn);
  });
}

const CARET_SVG = '<svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>';
const FOLDER_SVG = '<svg viewBox="0 0 24 24" width="13" height="13" fill="currentColor" stroke="none"><path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2Z"/></svg>';
const STAGE_FOLDER_SVG = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="M5 12h14"/></svg>';
const UNSTAGE_FOLDER_SVG = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/></svg>';
const IGNORE_FOLDER_SVG = '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M5.6 5.6l12.8 12.8"/></svg>';

// 递归收集目录(含子目录)下所有文件路径,供目录节点的批量按钮使用
function collectFilePaths(dir) {
  const paths = [];
  for (const child of dir.children.values()) {
    if (child.type === "dir") paths.push(...collectFilePaths(child));
    else paths.push(child.entry.path);
  }
  return paths;
}

// 目录节点:点击标题展开/折叠其子列表;标题最右侧可批量暂存/取消暂存整个目录
function folderNode(dir, kind) {
  const children = sortedChildren(dir);
  const li = document.createElement("li");
  li.className = "tree-folder";

  const head = document.createElement("div");
  head.className = "tree-folder-head";
  head.style.setProperty("--depth", dir.depth);
  head.dataset.path = dir.path;
  head.dataset.kind = kind;
  const key = kind + ":" + dir.path;
  const open = !folderClosed.has(key);
  head.classList.toggle("closed", !open);

  const caret = document.createElement("span");
  caret.className = "caret";
  caret.innerHTML = CARET_SVG;
  const ico = document.createElement("span");
  ico.className = "folder-ic";
  ico.innerHTML = FOLDER_SVG;
  const label = document.createElement("span");
  label.className = "folder-name";
  label.textContent = dir.name;
  const num = document.createElement("span");
  setNumBadge(num, dir.count);
  head.append(caret, ico, label, num);

  // 父节点最右侧:批量暂存(更改)/取消暂存(暂存)该目录下所有文件
  if (kind === "staged" || kind === "unstaged") {
    const stage = kind === "unstaged";
    const btn = document.createElement("button");
    btn.className = "folder-batch";
    btn.innerHTML = stage ? STAGE_FOLDER_SVG : UNSTAGE_FOLDER_SVG;
    btn.title = stage ? "暂存此目录下所有更改" : "取消暂存此目录下所有文件";
    btn.setAttribute("aria-label", btn.title);
    btn.addEventListener("click", (ev) => {
      ev.stopPropagation(); // 不触发目录的展开/折叠
      stagePaths(collectFilePaths(dir), stage);
    });
    head.appendChild(btn);

    // 批量忽略:一条目录规则覆盖该目录下所有文件(含子目录)
    const ib = document.createElement("button");
    ib.className = "folder-batch folder-ignore";
    ib.innerHTML = IGNORE_FOLDER_SVG;
    ib.title = "忽略此目录（含子目录）";
    ib.setAttribute("aria-label", ib.title);
    ib.addEventListener("click", (ev) => {
      ev.stopPropagation();
      askIgnoreDir(dir.path);
    });
    head.appendChild(ib);
  }
  li.appendChild(head);

  const ul = document.createElement("ul");
  ul.className = "file-list tree-children";
  ul.classList.toggle("hidden", !open);
  for (const child of children) {
    ul.appendChild(child.type === "dir"
      ? folderNode(child, kind)
      : fileRow(child.entry, kind, dir.depth + 1));
  }
  li.appendChild(ul);

  head.addEventListener("click", (ev) => {
    ev.stopPropagation();
    const closed = head.classList.toggle("closed");
    ul.classList.toggle("hidden", closed);
    if (closed) folderClosed.add(key); else folderClosed.delete(key);
  });
  return li;
}

// 文件行:沿用原有操作(暂存/忽略/丢弃/AI 解决),点击打开右侧编辑器
function fileRow(e, kind, depth) {
  const li = document.createElement("li");
  li.className = "file-row tree-file" + (kind === "conflict" ? " conflict" : "");
  li.style.setProperty("--depth", depth);

  // 仅冲突列表保留状态字母徽标;暂存/更改列表不显示
  if (kind === "conflict") {
    const st = document.createElement("span");
    st.className = "st st-" + stChar(e, kind);
    st.textContent = stChar(e, kind);
    li.appendChild(st);
  }

  const path = document.createElement("span");
  path.className = "path";
  // 目录已分层展示,行内只显示文件名(重命名同时显示原名)
  const name = String(e.path).split("/").pop();
  path.textContent = e.orig_path ? `${name} → ${String(e.orig_path).split("/").pop()}` : name;
  path.title = e.path;
  li.appendChild(path);

  // 增删行数(仅暂存/更改;无变动不显示),增=绿、删=红
  if (kind !== "conflict" && (e.added > 0 || e.deleted > 0)) {
    const stat = document.createElement("span");
    stat.className = "lines";
    const a = document.createElement("span"); a.className = "add"; a.textContent = "+" + e.added;
    const d = document.createElement("span"); d.className = "del"; d.textContent = "-" + e.deleted;
    stat.append(a, d);
    li.appendChild(stat);
  }

  const actions = document.createElement("span");
  actions.className = "row-actions";

  if (kind === "conflict") {
    const b = document.createElement("button");
    b.className = "resolve";
    b.textContent = "AI 解决";
    b.addEventListener("click", (ev) => { ev.stopPropagation(); resolveOne(e.path); });
    actions.appendChild(b);
  } else {
    const b = document.createElement("button");
    b.textContent = kind === "staged" ? "取消暂存" : "暂存";
    b.addEventListener("click", (ev) => {
      ev.stopPropagation();
      toggleStage(e.path, kind === "staged");
    });
    actions.appendChild(b);
    const ig = document.createElement("button");
    ig.textContent = "忽略";
    ig.addEventListener("click", (ev) => { ev.stopPropagation(); askIgnore(e.path); });
    actions.appendChild(ig);
    // 仅未暂存/未跟踪行提供「丢弃」(已跟踪→还原,未跟踪→删除)。
    // 两步确认:第一次点击变「✓」待确认态,再点一次才丢弃;
    // 光标一旦离开该行立即复原——避免从其他行回来时仍是对勾(误以为仍待确认)
    if (kind === "unstaged") {
      const di = document.createElement("button");
      di.textContent = "丢弃";
      let armed = false;
      const reset = () => {
        if (!armed) return;
        armed = false;
        di.textContent = "丢弃";
        di.classList.remove("armed");
      };
      di.addEventListener("click", (ev) => {
        ev.stopPropagation();
        if (!armed) {
          armed = true; di.textContent = "✓"; di.classList.add("armed");
          return;
        }
        reset();
        discardFile(e.path);
      });
      di.addEventListener("mouseleave", reset); // 移开光标立即复原
      actions.appendChild(di);
    }
  }
  li.appendChild(actions);

  // 点击任意文件(含冲突)打开右侧编辑器;冲突文件默认落到「合并」页,
  // 真正解决仍走行内的「AI 解决」按钮,避免误触
  li.addEventListener("click", () => openEditor(e, kind));
  return li;
}

// mods:数值 >0 时附加的修饰类(如 "dim" 弱化);zero:数值为 0 时的回落形态
function renderList(listId, entries, kind, countEl, mods = "") {
  const ul = $(listId);
  ul.innerHTML = "";
  ul.classList.add("file-tree");
  setNumBadge(countEl, entries.length, mods);
  const tree = buildTree(entries);
  for (const child of sortedChildren(tree)) {
    ul.appendChild(child.type === "dir"
      ? folderNode(child, kind)
      : fileRow(child.entry, kind, 0));
  }
}

// 展开/折叠全部目录:任一目录处于收起态则全部展开,否则全部折叠
function setAllFoldersOpen(listId, open) {
  for (const head of $(listId).querySelectorAll(".tree-folder-head")) {
    const key = head.dataset.kind + ":" + head.dataset.path;
    if (open) folderClosed.delete(key); else folderClosed.add(key);
    head.classList.toggle("closed", !open);
    const child = head.nextElementSibling;
    if (child && child.classList.contains("tree-children")) child.classList.toggle("hidden", !open);
  }
}

function stChar(e, kind) {
  if (kind === "conflict") return "C";
  if (e.untracked) return "?";
  if (kind === "staged") return STATUS_CHARS[e.x] || "M";
  return STATUS_CHARS[e.y] || "M";
}

// 忽略对话框:渲染候选规则(默认勾选第一条),确认逻辑统一在 btn-ignore-confirm
function showIgnoreDialog(opts) {
  const box = $("ignore-options");
  box.innerHTML = "";
  opts.forEach((o, i) => {
    const lab = document.createElement("label");
    lab.className = "ignore-opt";
    const r = document.createElement("input");
    r.type = "checkbox"; r.value = o.rule;
    r.className = "ig-check";
    if (i === 0) r.checked = true;
    const name = document.createElement("span");
    name.className = "ig-name";
    name.textContent = o.label;
    const code = document.createElement("code");
    code.className = "ig-rule";
    code.textContent = o.rule;
    lab.append(r, name, code);
    box.appendChild(lab);
  });
  $("dlg-ignore").classList.remove("hidden");
}

// 单文件忽略:文件名 / 扩展名 / 所在目录 三个候选规则
function askIgnore(path) {
  const base = path.split("/").pop();
  const dot = base.lastIndexOf(".");
  const ext = dot > 0 ? "*" + base.slice(dot) : null;
  const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/") + 1) : null;
  const opts = [{ label: "此文件", rule: base }];
  if (ext) opts.push({ label: "此扩展名", rule: ext });
  if (dir) opts.push({ label: "此目录", rule: dir });
  showIgnoreDialog(opts);
}

// 目录批量忽略:一条规则覆盖该目录及所有子目录,复用同一个确认对话框
function askIgnoreDir(dirPath) {
  const rule = String(dirPath).replace(/\/+$/, "") + "/";
  showIgnoreDialog([{ label: "此目录（含子目录）", rule }]);
}

/* ===== 右侧代码/对比编辑器 =====
   三个标签:差异(git diff)/ 代码(文件内容)/ 合并(BASE·OURS·THEIRS 三方对比)。
   面板可整体隐藏,切换仓库、空态、总览、ESC 均统一收起。 */
let diffRepo = null;        // 编辑器当前所属仓库,切换仓库时自动关闭
let editorEntry = null;     // 当前文件条目
let editorKind = null;      // staged | unstaged | conflict | commit(来自版本历史)
let editorCommit = null;    // 版本历史联动:{ hash, short },非空表示查看的是「某提交的文件」
let editorTab = "diff";     // diff | code | merge
let editorLoadSeq = 0;      // 异步加载序号:丢弃过期结果

const escapeHtml = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const SUN_SVG = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>';
const MOON_SVG = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z"/></svg>';

// 编辑器专属暗黑模式:缺省开启,仅作用于右侧代码区,不影响应用整体主题
function syncEditorTheme() {
  const dark = settings.editor_dark !== false;
  $("diff-panel").classList.toggle("code-dark", dark);
  const btn = $("btn-editor-theme");
  if (btn) {
    btn.innerHTML = dark ? SUN_SVG : MOON_SVG;
    btn.title = dark ? "切换到浅色代码区" : "切换到暗黑代码区";
  }
}

// 打开文件:设置标题与可用标签,展示面板并加载当前标签内容
function openEditor(e, kind) {
  diffRepo = repo;
  editorEntry = e;
  editorKind = kind;
  editorCommit = null;
  // 冲突文件总是从「合并」页进入;普通文件若停在「合并」页则回到「差异」页
  if (kind === "conflict") editorTab = "merge";
  else if (editorTab === "merge") editorTab = "diff";
  $("diff-title").textContent = e.path;
  $("diff-title").title = e.path;
  $("diff-context").classList.add("hidden");
  // 合并页只对冲突文件开放
  document.querySelector('.editor-tab[data-tab="merge"]').classList.toggle("hidden", kind !== "conflict");
  setEditorVisible(true);
  setEditorTab(editorTab);
}

// 从版本历史打开某个提交改动的文件:右侧差异页展示「该提交 vs 上一版本」,
// 即这个提交里到底改了什么。由 history.js 在展开提交的文件清单后调用。
export function openCommitFile(hash, short, file) {
  diffRepo = repo;
  editorCommit = { hash, short };
  editorEntry = file;
  editorKind = "commit";
  if (editorTab === "merge") editorTab = "diff"; // 合并页只对冲突开放
  $("diff-title").textContent = file.path;
  $("diff-title").title = `${short} · ${file.path} · 与上一版本的差异`;
  const ctx = $("diff-context");
  ctx.textContent = `对比 上一版 → ${short}`;
  ctx.classList.remove("hidden");
  document.querySelector('.editor-tab[data-tab="merge"]').classList.add("hidden");
  setEditorVisible(true);
  setEditorTab(editorTab);
}

// 切换标签并加载对应内容
function setEditorTab(tab) {
  editorTab = tab;
  document.querySelectorAll(".editor-tab").forEach((b) => b.classList.toggle("active", b.dataset.tab === tab));
  loadEditorContent();
}

async function loadEditorContent() {
  if (!editorEntry || !repo) return;
  const seq = ++editorLoadSeq;
  const e = editorEntry, kind = editorKind, tab = editorTab, targetRepo = repo, cm = editorCommit;
  const box = $("diff-content");
  box.innerHTML = '<span class="d-load">加载中…</span>';
  try {
    let html;
    if (tab === "diff") {
      if (cm) {
        // 版本历史联动:该文件在此提交中相对上一版本的改动
        const diff = await invoke("git_commit_file_diff", { repo: targetRepo, hash: cm.hash, path: e.path });
        html = diff ? renderDiff(diff) : '<span class="d-empty">该提交未改动此文件</span>';
      } else {
        const diff = await invoke("git_diff", { repo: targetRepo, path: e.path, staged: kind === "staged" });
        html = diff ? renderDiff(diff) : '<span class="d-empty">无差异内容</span>';
      }
    } else if (tab === "code") {
      // 版本历史联动看该提交里的文件内容;否则已暂存看索引,其余看工作区
      const rev = cm ? cm.hash : (kind === "staged" ? "INDEX" : "WORKTREE");
      const text = await invoke("git_file_content", { repo: targetRepo, path: e.path, rev });
      html = renderCode(text);
    } else {
      // 合并:并排展示 BASE / OURS(当前分支) / THEIRS(传入分支)
      const [base, ours, theirs] = await Promise.all([
        invoke("git_file_content", { repo: targetRepo, path: e.path, rev: "BASE" }),
        invoke("git_file_content", { repo: targetRepo, path: e.path, rev: "OURS" }),
        invoke("git_file_content", { repo: targetRepo, path: e.path, rev: "THEIRS" }),
      ]);
      html = renderMerge(base, ours, theirs);
    }
    if (seq !== editorLoadSeq || diffRepo !== repo || $("diff-panel").classList.contains("hidden")) return;
    box.innerHTML = html;
  } catch (err) {
    if (seq !== editorLoadSeq || diffRepo !== repo || $("diff-panel").classList.contains("hidden")) return;
    box.innerHTML = '<span class="d-empty">' + escapeHtml(String(err)) + "</span>";
  }
}

// 带行号的代码视图
function renderCode(text) {
  if (text == null || text === "") return '<span class="d-empty">文件为空或无法预览</span>';
  const lines = String(text).replace(/\n$/, "").split("\n");
  return '<div class="code-view">' + lines.map((l, i) =>
    `<div class="c-row"><span class="c-num">${i + 1}</span><span class="c-text">${escapeHtml(l)}</span></div>`
  ).join("") + "</div>";
}

// 三方合并对比:BASE / OURS / THEIRS 三栏并排
function renderMerge(base, ours, theirs) {
  const col = (title, text, cls) =>
    `<div class="merge-col ${cls}"><div class="merge-col-head">${escapeHtml(title)}</div><div class="merge-col-body">${renderCode(text)}</div></div>`;
  return '<div class="merge-grid">' +
    col("基础版本 BASE", base, "m-base") +
    col("当前分支 OURS", ours, "m-ours") +
    col("传入分支 THEIRS", theirs, "m-theirs") +
    "</div>";
}

// 显示/隐藏右侧编辑器(切换仓库、空态、总览、ESC、关闭按钮统一走这里)
function setEditorVisible(visible) {
  $("diff-panel").classList.toggle("hidden", !visible);
  if (!visible) {
    editorEntry = null; editorCommit = null; diffRepo = null; editorLoadSeq++;
    $("diff-content").innerHTML = "";
    $("diff-context").classList.add("hidden");
  }
  syncEditorToggle();
}

// 隐藏编辑器(供 main.js 的 ESC、dashboard 的总览调用)
export function hideEditor() { setEditorVisible(false); }

// 工具栏开关与按钮态:与实际面板显隐保持一致
export function syncEditorToggle() {
  const btn = $("btn-toggle-editor");
  if (!btn) return;
  const visible = !$("diff-panel").classList.contains("hidden");
  btn.classList.toggle("active", visible);
  btn.title = visible ? "隐藏代码对比面板" : "显示代码对比面板";
  const close = $("btn-diff-close");
  if (close) close.title = visible ? "关闭代码面板" : "显示代码面板";
}

function closeDiffPanel() { setEditorVisible(false); }

/* ===== 并排差异视图 =====
   左栏=上一版本(父提交),右栏=此提交。
   unified diff 先按 hunk 解析成行,再按「连续改动 / 连续未改动」分段;
   未改动段超过 CONTEXT 行时只保留首尾若干行,中间折叠成可点击的展开条。*/
const DIFF_CONTEXT_LINES = 3;

function isDiffMeta(line) {
  return line.startsWith("diff --git ")
    || line.startsWith("index ")
    || line.startsWith("new file mode ")
    || line.startsWith("deleted file mode ")
    || line.startsWith("old mode ")
    || line.startsWith("new mode ")
    || line.startsWith("similarity index ")
    || line.startsWith("rename from ")
    || line.startsWith("rename to ")
    || line.startsWith("Binary files ")
    || line.startsWith("GIT binary patch")
    || line.startsWith("--- ")
    || line.startsWith("+++ ")
    || line.startsWith("\\");
}

// 解析成段:{t:"ctx",rows} | {t:"change",dels,adds} | {t:"gap",count}
function parseDiffSegments(text) {
  const hunks = [];
  let cur = null, oldNo = 0, newNo = 0;
  for (const raw of String(text).split("\n")) {
    const line = raw.replace(/\r$/, "");
    if (line.startsWith("@@")) {
      if (cur) cur.oldEnd = oldNo;
      const m = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
      oldNo = m ? Number(m[1]) : 0;
      newNo = m ? Number(m[3]) : 0;
      cur = { oldStart: oldNo, rows: [] };
      hunks.push(cur);
      continue;
    }
    if (!cur || isDiffMeta(line)) continue;
    if (line.startsWith("+")) cur.rows.push({ t: "add", text: line.slice(1), n: newNo++ });
    else if (line.startsWith("-")) cur.rows.push({ t: "del", text: line.slice(1), n: oldNo++ });
    else if (line.startsWith(" ")) cur.rows.push({ t: "ctx", text: line.slice(1), o: oldNo++, n: newNo++ });
  }
  if (cur) cur.oldEnd = oldNo;

  const segments = [];
  let prevOldEnd = null;
  for (const h of hunks) {
    // hunk 之间被 git 省略的未改动行
    if (prevOldEnd !== null && h.oldStart > prevOldEnd) {
      segments.push({ t: "gap", count: h.oldStart - prevOldEnd });
    }
    let i = 0;
    while (i < h.rows.length) {
      if (h.rows[i].t === "ctx") {
        const run = [];
        while (i < h.rows.length && h.rows[i].t === "ctx") run.push(h.rows[i++]);
        segments.push({ t: "ctx", rows: run });
      } else {
        const dels = [], adds = [];
        while (i < h.rows.length && h.rows[i].t !== "ctx") {
          const x = h.rows[i++];
          (x.t === "del" ? dels : adds).push(x);
        }
        segments.push({ t: "change", dels, adds });
      }
    }
    prevOldEnd = h.oldEnd;
  }
  return segments;
}

function renderDiff(text) {
  const segments = parseDiffSegments(text);
  if (!segments.length) return '<span class="d-empty">无文本差异（可能为二进制文件）</span>';
  const num = (n, cls) => `<span class="sd-num ${cls || ""}">${n == null ? "" : n}</span>`;
  const code = (txt, cls) => `<span class="sd-code ${cls || ""}">${escapeHtml(txt)}</span>`;
  const ctxRow = (r) =>
    `<div class="sd-row sd-ctx">${num(r.o)}${code(r.text)}${num(r.n)}${code(r.text)}</div>`;

  let uid = 0;
  const collapse = (count, rows) => {
    const id = "sd-hidden-" + (++uid);
    return `<div class="sd-collapse" data-target="${id}">⋯ 展开 ${count} 行未更改内容 ⋯</div>`
      + `<div class="sd-hidden" id="${id}" data-count="${count}">${rows.map(ctxRow).join("")}</div>`;
  };

  let html = '<div class="side-diff">'
    + '<div class="sd-head"><span></span><span class="sd-side">上一版本</span>'
    + '<span></span><span class="sd-side">此提交<button class="sd-toggle-all">全部展开</button></span></div>'
    + '<div class="sd-body">';
  for (const seg of segments) {
    if (seg.t === "gap" || seg.t === "ctx") {
      const rows = seg.t === "ctx" ? seg.rows : [];
      if (seg.t === "ctx" && rows.length <= DIFF_CONTEXT_LINES * 2) {
        html += rows.map(ctxRow).join("");
      } else {
        const head = rows.slice(0, DIFF_CONTEXT_LINES);
        const tail = rows.slice(-DIFF_CONTEXT_LINES);
        const middle = rows.slice(DIFF_CONTEXT_LINES, rows.length - DIFF_CONTEXT_LINES);
        const count = seg.t === "gap" ? seg.count : middle.length;
        html += head.map(ctxRow).join("");
        if (count > 0) {
          // hunk 之间被 git 省略的行没有内容可展开,只做信息提示
          html += seg.t === "gap"
            ? `<div class="sd-collapse sd-gap">⋯ 此处省略 ${count} 行未更改内容 ⋯</div>`
            : collapse(count, middle);
        }
        html += tail.map(ctxRow).join("");
      }
    } else {
      const { dels, adds } = seg;
      const len = Math.max(dels.length, adds.length);
      for (let i = 0; i < len; i++) {
        const d = dels[i], a = adds[i];
        html += '<div class="sd-row sd-chg">'
          + num(d ? d.n : "") + code(d ? d.text : "", d ? "sd-del" : "sd-empty")
          + num(a ? a.n : "") + code(a ? a.text : "", a ? "sd-add" : "sd-empty")
          + "</div>";
      }
    }
  }
  return html + "</div></div>";
}

// 规范化 .gitignore 排版:注释段落前空一行(跟在规则行后时)、连续空行合一、
// 去行尾空白、统一以单个换行结尾。只动空白,不改动注释与规则的内容和顺序。
function normalizeGitignore(text) {
  const out = [];
  for (const raw of text.split("\n")) {
    const l = raw.replace(/\s+$/, ""); // 去行尾空白
    const prev = out[out.length - 1];
    const isComment = l.trimStart().startsWith("#");
    if (isComment && prev !== undefined && prev.trim() !== "" && !prev.trimStart().startsWith("#")) {
      out.push(""); // 规则行后直接跟注释:空行分组
    }
    if (l.trim() === "" && (prev === undefined || prev.trim() === "")) continue; // 开头/连续空行
    out.push(l);
  }
  const joined = out.join("\n").replace(/\s+$/, "");
  return joined ? joined + "\n" : "";
}

/* ===== 事件绑定 ===== */
export function bindPanelEvents() {
  $("btn-diff-close").addEventListener("click", closeDiffPanel);

  // 代码区暗黑模式:初始按记忆值应用,点击切换并持久化
  syncEditorTheme();
  $("btn-editor-theme").addEventListener("click", () => {
    settings.editor_dark = !(settings.editor_dark !== false);
    syncEditorTheme();
    invoke("settings_save", { settings }).catch(() => {});
  });

  // 工具栏开关:显示/隐藏右侧代码对比面板
  $("btn-toggle-editor").addEventListener("click", () => {
    if ($("diff-panel").classList.contains("hidden")) {
      setEditorVisible(true);
      if (!editorEntry) $("diff-content").innerHTML = '<span class="d-empty">点击左侧文件查看代码、差异或合并视图</span>';
    } else {
      setEditorVisible(false);
    }
  });
  // 编辑器标签切换
  document.querySelectorAll(".editor-tab").forEach((b) =>
    b.addEventListener("click", () => setEditorTab(b.dataset.tab))
  );
  // 并排差异:折叠的未改动块按需展开/收起(事件委托,内容异步替换仍生效)
  $("diff-content").addEventListener("click", (ev) => {
    const setBar = (bar, open) => {
      const target = document.getElementById(bar.dataset.target);
      if (!target) return false;
      target.classList.toggle("open", open);
      bar.classList.toggle("open", open);
      bar.textContent = `⋯ ${open ? "收起" : "展开"} ${target.dataset.count} 行未更改内容 ⋯`;
      return true;
    };
    const bar = ev.target.closest(".sd-collapse");
    if (bar) {
      setBar(bar, !bar.classList.contains("open"));
      return;
    }
    const all = ev.target.closest(".sd-toggle-all");
    if (all) {
      const bars = [...all.closest(".side-diff").querySelectorAll(".sd-collapse")];
      const open = bars.some((b) => !b.classList.contains("open"));
      bars.forEach((b) => setBar(b, open));
      all.textContent = open ? "全部收起" : "全部展开";
    }
  });
  // 文件树:展开/折叠全部目录
  document.querySelectorAll(".tree-toggle-all").forEach((b) =>
    b.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const heads = $(b.dataset.treeTarget).querySelectorAll(".tree-folder-head");
      const anyClosed = [...heads].some((h) => h.classList.contains("closed"));
      setAllFoldersOpen(b.dataset.treeTarget, anyClosed); // 有收起则全展开,否则全折叠
    })
  );

  // 右侧编辑器宽度拖拽调整(与侧栏 resizer 同款交互)
  const DIFF_W_MIN = 240, DIFF_W_MAX = 900;
  // 左侧暂存/更改面板必须保留的最小宽度(与 CSS --main-min-width 同源),
  // 拖宽 diff 时不能把它压到该值以下,否则文件名会被右侧行内操作按钮遮挡
  const mainMinWidth = () =>
    parseInt(getComputedStyle(document.documentElement).getPropertyValue("--main-min-width"), 10) || 300;
  let diffDragStartX = 0, diffDragStartW = 0;
  const onDiffDrag = (e) => {
    // 拖手柄向右 → 面板变宽(手柄在面板左缘,增量取反)
    const room = $("main-row").clientWidth - mainMinWidth();
    const upper = Math.max(DIFF_W_MIN, Math.min(DIFF_W_MAX, room));
    const w = Math.min(upper, Math.max(DIFF_W_MIN, diffDragStartW + diffDragStartX - e.clientX));
    $("diff-panel").style.width = w + "px";
  };
  const endDiffDrag = () => {
    document.body.classList.remove("resizing");
    document.removeEventListener("pointermove", onDiffDrag);
    settings.diff_width = parseInt($("diff-panel").style.width, 10);
    invoke("settings_save", { settings }).catch(() => {});
  };
  $("diff-resizer").addEventListener("pointerdown", (e) => {
    e.preventDefault();
    diffDragStartX = e.clientX;
    diffDragStartW = $("diff-panel").getBoundingClientRect().width;
    document.body.classList.add("resizing");
    document.addEventListener("pointermove", onDiffDrag);
    document.addEventListener("pointerup", endDiffDrag, { once: true });
  });

  $("btn-stage-all").addEventListener("click", () => stageAll(true));
  $("btn-unstage-all").addEventListener("click", () => stageAll(false));
  $("btn-discard-all").addEventListener("click", discardAll);

  // 更多菜单
  $("btn-more").addEventListener("click", (e) => {
    e.stopPropagation();
    $("more-menu").classList.toggle("hidden");
  });
  $("btn-more-open-dir").addEventListener("click", () => {
    $("more-menu").classList.add("hidden");
    if (repo) invoke("open_local_dir", { path: repo }).catch((e) => toast(String(e), false));
  });
  $("btn-more-remove").addEventListener("click", () => {
    $("more-menu").classList.add("hidden");
    if (repo) removeRepo(repo);
  });
  $("btn-more-gitignore").addEventListener("click", async () => {
    $("more-menu").classList.add("hidden");
    if (!repo) return;
    try {
      $("gitignore-content").value = await invoke("gitignore_read", { repo });
    } catch (e) { toast(String(e), false); return; }
    $("dlg-gitignore").classList.remove("hidden");
  });
  $("btn-gitignore-save").addEventListener("click", async () => {
    setButtonLoading($("btn-gitignore-save"), true, "保存中…");
    try {
      await invoke("gitignore_write", { repo, content: normalizeGitignore($("gitignore-content").value) });
      // 同忽略弹窗:新规则命中的已跟踪文件立即移出跟踪,列表即时生效
      try { await invoke("git_untrack_ignored", { repo }); } catch (_) {}
      $("dlg-gitignore").classList.add("hidden");
      toast("已保存 .gitignore", true);
      await refresh();
    } catch (e) { toast(String(e), false); }
    finally { setButtonLoading($("btn-gitignore-save"), false); }
  });
  $("btn-gitignore-cancel").addEventListener("click", () => $("dlg-gitignore").classList.add("hidden"));
  $("btn-ignore-confirm").addEventListener("click", async () => {
    const rules = [...document.querySelectorAll("#ignore-options input:checked")].map((r) => r.value);
    if (!rules.length) return;
    setButtonLoading($("btn-ignore-confirm"), true, "添加中…");
    try {
      const content = (await invoke("gitignore_read", { repo })) || "";
      // 去重比较用归一化形式:目录规则的尾斜杠不参与(dist 与 dist/ 对 git 等价,避免重复追加)
      const norm = (s) => s.trim().replace(/\/+$/, "");
      const existing = new Set(content.split("\n").map(norm).filter(Boolean));
      const add = rules.filter((r) => !existing.has(norm(r)));
      if (add.length) {
        const base = normalizeGitignore(content); // 顺带整理既有排版(段落空行统一)
        // 与已有内容空行分隔,避免粘进别人的注释/分组;
        // 尾部已是空行隔开的独立非注释规则块(上次弹窗追加的)则直接续接,不产生碎块
        const tailIsRuleBlock = /\n\s*\n(?!#)[^\n]*$/.test("\n" + base);
        const sep = base ? (tailIsRuleBlock ? "\n" : "\n\n") : "";
        await invoke("gitignore_write", { repo, content: base + sep + add.join("\n") + "\n" });
      }
      // 让被忽略文件立即从列表消失:.gitignore 对已跟踪/已暂存文件不生效,
      // 需把命中规则的所有文件(含同目录/同扩展名的其他文件)移出 git 跟踪
      try { await invoke("git_untrack_ignored", { repo }); } catch (_) {}
      $("dlg-ignore").classList.add("hidden");
      toast(add.length ? "已添加到 .gitignore： " + add.join("、") : "所选规则已存在，无需添加", add.length > 0);
      await refresh();
    } catch (e) { toast(String(e), false); }
    finally { setButtonLoading($("btn-ignore-confirm"), false); }
  });
  $("btn-ignore-cancel").addEventListener("click", () => $("dlg-ignore").classList.add("hidden"));

  document.querySelectorAll(".section-head").forEach((h) =>
    h.addEventListener("click", () => {
      const sec = h.closest(".section");
      sectionUserSet.add(sec.id); // 用户手动改过:后续刷新不再套用默认收起规则
      sec.classList.toggle("collapsed");
      $(h.dataset.target).classList.toggle("hidden");
    })
  );
}
