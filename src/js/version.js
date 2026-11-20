/* 当前工程版本号升级:自适应识别项目中的版本文件(package.json / Cargo.toml /
   pyproject.toml / pubspec.yaml / tauri.conf.json / manifest.json 等),
   弹窗选择补丁/次版本/主版本级别,写回所有识别到的文件保持版本一致。 */
import { $, invoke, toast, setButtonLoading, repo } from "./state.js";
import { refresh } from "./panel.js";

// 前端预览用:与后端 parse_version 同规则(至少 major.minor,后缀原样保留)
function parseVersion(v) {
  const m = /^(\d+)(?:\.(\d+))?(?:\.(\d+))?(.*)$/.exec(String(v || ""));
  if (!m || m[2] === undefined) return null;
  return { major: +m[1], minor: +m[2], patch: +(m[3] || 0), suffix: m[4] || "" };
}

function bumpVersion(v, level) {
  const p = parseVersion(v);
  if (!p) return v;
  if (level === "major") return `${p.major + 1}.0.0${p.suffix}`;
  if (level === "minor") return `${p.major}.${p.minor + 1}.0${p.suffix}`;
  return `${p.major}.${p.minor}.${p.patch + 1}${p.suffix}`;
}

// 打开升级弹窗:先检测版本文件,再填充当前版本、文件清单与各级别预览
export async function openVersionBump() {
  if (!repo) { toast("请先打开一个项目", false); return; }
  let info;
  try {
    info = await invoke("project_version_detect", { repo });
  } catch (e) {
    toast("检测版本文件失败：" + e, false);
    return;
  }
  if (!info || !info.primary) {
    toast("未检测到版本文件（支持 package.json、Cargo.toml、pyproject.toml、pubspec.yaml、tauri.conf.json 等）", false);
    return;
  }

  const cur = info.primary.version;
  $("vb-current").textContent = cur + (info.files.length > 1 ? `（共 ${info.files.length} 个文件）` : "");

  const box = $("vb-files");
  box.innerHTML = "";
  for (const f of info.files) {
    const row = document.createElement("div");
    row.className = "vb-file";
    const code = document.createElement("code");
    code.textContent = f.path;
    const ver = document.createElement("span");
    ver.className = "vb-file-ver mono";
    ver.textContent = f.version;
    row.append(code, ver);
    box.appendChild(row);
  }

  for (const level of ["patch", "minor", "major"]) {
    $(`vb-ex-${level}`).textContent = `${cur} → ${bumpVersion(cur, level)}`;
  }
  const patchRadio = document.querySelector('input[name="vb-level"][value="patch"]');
  if (patchRadio) patchRadio.checked = true;
  $("dlg-version-bump").classList.remove("hidden");
}

// 确认升级:按所选级别写回,刷新仓库面板以反映新增的文件改动
export async function confirmVersionBump() {
  if (!repo) return;
  const level = document.querySelector('input[name="vb-level"]:checked')?.value || "patch";
  setButtonLoading($("btn-vb-confirm"), true, "升级中…");
  try {
    const r = await invoke("project_version_bump", { repo, level });
    $("dlg-version-bump").classList.add("hidden");
    toast(`版本已升级：${r.old_version} → ${r.new_version}（更新 ${r.updated.length} 个文件）`, true);
    await refresh();
  } catch (e) {
    toast("升级失败：" + e, false);
  } finally {
    setButtonLoading($("btn-vb-confirm"), false);
  }
}

export function bindVersionEvents() {
  $("btn-bump-version").addEventListener("click", openVersionBump);
  $("btn-vb-cancel").addEventListener("click", () => $("dlg-version-bump").classList.add("hidden"));
  $("btn-vb-confirm").addEventListener("click", confirmVersionBump);
}
