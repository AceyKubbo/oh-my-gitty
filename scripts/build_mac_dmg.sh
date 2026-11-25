#!/usr/bin/env bash
# macOS DMG 打包重试封装。
# Tauri 的 bundle_dmg.sh 在 CI 上偶发 hdiutil "Resource busy"/"Operation not permitted"
# 失败（tauri#3055、tauri-action#801），首次失败后清理残留挂载与临时镜像重试通常即可成功。
# --verbose 用于让 CLI 2.11.x 打印脚本真实输出（该版本默认丢弃 stderr，报错只剩 "failed to run"）。
# 用法: bash scripts/build_mac_dmg.sh [target triple, 例如 aarch64-apple-darwin]
set -uo pipefail

target=${1:-}
max_attempts=3

args=(--bundles app,dmg --verbose)
if [[ -n "$target" ]]; then
  args+=(--target "$target")
  bundle_dir="src-tauri/target/$target/release/bundle/macos"
else
  bundle_dir="src-tauri/target/release/bundle/macos"
fi

for ((attempt = 1; attempt <= max_attempts; attempt++)); do
  echo "::group::tauri build (attempt $attempt/$max_attempts)"
  if npx tauri build "${args[@]}"; then
    echo "::endgroup::"
    exit 0
  fi
  echo "::endgroup::"

  if ((attempt == max_attempts)); then
    echo "::error::DMG 打包连续 ${max_attempts} 次失败"
    exit 1
  fi

  echo "::warning::DMG 打包失败，清理残留挂载与临时镜像后重试..."
  # bundle_dmg.sh 用 -mountrandom 挂载中间卷，路径是随机的，不能按卷名卸载。
  # 从 hdiutil info 里找出镜像路径含 rw. 的设备（image-path 行在设备行之前）。
  while IFS= read -r dev; do
    [[ -n "$dev" ]] || continue
    hdiutil detach "$dev" -force >/dev/null 2>&1 || true
  done < <(hdiutil info 2>/dev/null | awk '
    /^image-path/ { want = ($0 ~ /rw\./) }
    /^\/dev\/disk/ && want { d = $1; sub(/s[0-9]+$/, "", d); print d }
  ' | sort -u)
  hdiutil detach "/Volumes/Oh My Gitty" -force >/dev/null 2>&1 || true
  rm -f "$bundle_dir"/rw.*.dmg
  sleep 5
done
