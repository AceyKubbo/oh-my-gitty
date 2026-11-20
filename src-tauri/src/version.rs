/*! 当前工程的版本号自适应识别与升级。
    识别项目根目录下常见的版本文件(package.json / Cargo.toml / pyproject.toml /
    pubspec.yaml / tauri.conf.json / manifest.json 等),定位其中的版本号并整体 +1。
    写回时只替换版本号字面量,不改动文件其他内容与排版。 */

use serde::Serialize;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Json,
    Toml,
    Yaml,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Json => "json",
            Kind::Toml => "toml",
            Kind::Yaml => "yaml",
        }
    }
}

#[derive(Serialize, Clone)]
pub struct VersionFile {
    /// 相对项目根目录的路径
    pub path: String,
    /// 文件格式(json / toml / yaml)
    pub format: &'static str,
    /// 当前版本号原文(可能带后缀,如 1.0.0+42)
    pub version: String,
}

#[derive(Serialize)]
pub struct VersionInfo {
    /// 识别到的全部版本文件(按优先级排列)
    pub files: Vec<VersionFile>,
    /// 作为基准的主版本文件(列表首个),未识别到则为 None
    pub primary: Option<VersionFile>,
}

#[derive(Serialize)]
pub struct VersionBumpResult {
    pub old_version: String,
    pub new_version: String,
    /// 实际写回的文件路径列表
    pub updated: Vec<String>,
    pub primary_path: String,
}

/// 候选版本文件(按优先级从上到下):同一项目存在多个时一起升级,保持版本一致。
const CANDIDATES: &[(&str, Kind)] = &[
    ("package.json", Kind::Json),
    ("Cargo.toml", Kind::Toml),
    ("src-tauri/Cargo.toml", Kind::Toml),
    ("pyproject.toml", Kind::Toml),
    ("pubspec.yaml", Kind::Yaml),
    ("tauri.conf.json", Kind::Json),
    ("src-tauri/tauri.conf.json", Kind::Json),
    ("manifest.json", Kind::Json),
];

/// 解析版本号开头的 major.minor.patch,返回 (major, minor, patch, 剩余后缀)。
/// 至少要求 major.minor 两段;后缀(如 +42、-beta.1)原样保留。
fn parse_version(v: &str) -> Option<(u64, u64, u64, String)> {
    let bytes = v.as_bytes();
    let mut i = 0;
    let mut nums = [0u64; 3];
    let mut count = 0;
    while count < 3 {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            break;
        }
        nums[count] = v[start..i].parse().ok()?;
        count += 1;
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    if count < 2 {
        return None;
    }
    Some((nums[0], nums[1], nums[2], v[i..].to_string()))
}

/// 在文件内容中定位版本号的字节区间 (start, end) 与原文。
fn find_version_span(content: &str, kind: Kind) -> Option<(usize, usize, String)> {
    match kind {
        Kind::Json => {
            // 只匹配独立的 "version" 键,不会误命中 "manifest_version"
            let key = "\"version\"";
            let mut from = 0;
            while let Some(pos) = content[from..].find(key) {
                let idx = from + pos;
                let mut p = idx + key.len();
                let b = content.as_bytes();
                while p < b.len() && b[p].is_ascii_whitespace() {
                    p += 1;
                }
                if p < b.len() && b[p] == b':' {
                    p += 1;
                    while p < b.len() && b[p].is_ascii_whitespace() {
                        p += 1;
                    }
                    if p < b.len() && b[p] == b'"' {
                        let start = p + 1;
                        if let Some(end_rel) = content[start..].find('"') {
                            let end = start + end_rel;
                            let v = &content[start..end];
                            if parse_version(v).is_some() {
                                return Some((start, end, v.to_string()));
                            }
                        }
                    }
                }
                from = idx + 1;
            }
            None
        }
        Kind::Toml => {
            let mut line_start = 0;
            for line in content.split_inclusive('\n') {
                let trimmed = line.trim();
                let abs = line_start + (line.len() - line.trim_start().len());
                if !trimmed.starts_with('#') && !trimmed.starts_with('[') {
                    if let Some(rest) = trimmed.strip_prefix("version") {
                        // "version" 后必须是空白或 '=',避免误命中 versioning / rust-version
                        if rest.starts_with(char::is_whitespace) || rest.starts_with('=') {
                            let b = content.as_bytes();
                            let mut p = abs + "version".len();
                            while p < b.len() && b[p].is_ascii_whitespace() {
                                p += 1;
                            }
                            if p < b.len() && b[p] == b'=' {
                                p += 1;
                                while p < b.len() && b[p].is_ascii_whitespace() {
                                    p += 1;
                                }
                                if p < b.len() && b[p] == b'"' {
                                    let start = p + 1;
                                    if let Some(end_rel) = content[start..].find('"') {
                                        let end = start + end_rel;
                                        let v = &content[start..end];
                                        if parse_version(v).is_some() {
                                            return Some((start, end, v.to_string()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                line_start += line.len();
            }
            None
        }
        Kind::Yaml => {
            let mut line_start = 0;
            for line in content.split_inclusive('\n') {
                let trimmed = line.trim();
                let abs = line_start + (line.len() - line.trim_start().len());
                if !trimmed.starts_with('#') {
                    if let Some(rest) = trimmed.strip_prefix("version:") {
                        // 只接受空白或引号作为值起点,避免误命中 version_code
                        if rest.starts_with(char::is_whitespace) || rest.starts_with('"') || rest.starts_with('\'') {
                            let b = content.as_bytes();
                            let mut p = abs + "version:".len();
                            while p < b.len() && b[p].is_ascii_whitespace() {
                                p += 1;
                            }
                            let quoted = p < b.len() && (b[p] == b'"' || b[p] == b'\'');
                            let quote = if quoted { b[p] } else { 0 };
                            if quoted {
                                p += 1;
                            }
                            let start = p;
                            let mut end = start;
                            while end < b.len() {
                                let c = b[end];
                                if c == b'\n' || c == b'\r' {
                                    break;
                                }
                                if quoted && c == quote {
                                    break;
                                }
                                end += 1;
                            }
                            if !quoted {
                                while end > start && b[end - 1].is_ascii_whitespace() {
                                    end -= 1;
                                }
                            }
                            let v = &content[start..end];
                            if parse_version(v).is_some() {
                                return Some((start, end, v.to_string()));
                            }
                        }
                    }
                }
                line_start += line.len();
            }
            None
        }
    }
}

/// 扫描项目根目录,收集所有可识别的版本文件
fn detect_version_files(repo: &str) -> Vec<VersionFile> {
    let root = std::path::Path::new(repo);
    let mut out = Vec::new();
    for (rel, kind) in CANDIDATES {
        let full = root.join(rel);
        let Ok(content) = std::fs::read_to_string(&full) else { continue };
        if let Some((_, _, v)) = find_version_span(&content, *kind) {
            out.push(VersionFile {
                path: (*rel).to_string(),
                format: kind.label(),
                version: v,
            });
        }
    }
    out
}

#[tauri::command]
pub fn project_version_detect(repo: String) -> VersionInfo {
    let files = detect_version_files(&repo);
    let primary = files.first().cloned();
    VersionInfo { files, primary }
}

#[tauri::command]
pub fn project_version_bump(repo: String, level: String) -> Result<VersionBumpResult, String> {
    let files = detect_version_files(&repo);
    let primary = files
        .first()
        .ok_or_else(|| "未检测到可识别的版本文件".to_string())?
        .clone();
    let (major, minor, patch, suffix) =
        parse_version(&primary.version).ok_or_else(|| "版本号格式无法识别".to_string())?;
    let (n_major, n_minor, n_patch) = match level.as_str() {
        "major" => (major + 1, 0, 0),
        "minor" => (major, minor + 1, 0),
        "patch" => (major, minor, patch + 1),
        _ => return Err("未知的升级级别".to_string()),
    };
    let new_version = format!("{}.{}.{}{}", n_major, n_minor, n_patch, suffix);

    let root = std::path::Path::new(&repo);
    let mut updated = Vec::new();
    for f in &files {
        let full = root.join(&f.path);
        let content = std::fs::read_to_string(&full)
            .map_err(|e| format!("读取 {} 失败：{}", f.path, e))?;
        let kind = match f.format {
            "json" => Kind::Json,
            "toml" => Kind::Toml,
            "yaml" => Kind::Yaml,
            _ => continue,
        };
        let (start, end, _) = find_version_span(&content, kind)
            .ok_or_else(|| format!("{} 中未找到版本号", f.path))?;
        let mut next = String::with_capacity(content.len());
        next.push_str(&content[..start]);
        next.push_str(&new_version);
        next.push_str(&content[end..]);
        std::fs::write(&full, next)
            .map_err(|e| format!("写入 {} 失败：{}", f.path, e))?;
        updated.push(f.path.clone());
    }

    Ok(VersionBumpResult {
        old_version: primary.version,
        new_version,
        updated,
        primary_path: primary.path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semver_with_suffix() {
        assert_eq!(parse_version("0.3.0"), Some((0, 3, 0, "".into())));
        assert_eq!(parse_version("1.2.3+42"), Some((1, 2, 3, "+42".into())));
        assert_eq!(parse_version("2.0-beta.1"), Some((2, 0, 0, "-beta.1".into())));
        assert_eq!(parse_version("abc"), None);
    }

    #[test]
    fn json_does_not_match_manifest_version() {
        let c = r#"{"name":"ext","manifest_version":3,"version":"1.4.0"}"#;
        assert_eq!(find_version_span(c, Kind::Json).map(|x| x.2), Some("1.4.0".into()));
    }

    #[test]
    fn toml_and_yaml_spans() {
        let t = "[package]\nname = \"x\"\nversion = \"0.1.2\"\n";
        assert_eq!(find_version_span(t, Kind::Toml).map(|x| x.2), Some("0.1.2".into()));
        let y = "name: x\nversion: 1.2.3+7\n";
        assert_eq!(find_version_span(y, Kind::Yaml).map(|x| x.2), Some("1.2.3+7".into()));
    }

    #[test]
    fn detect_real_tauri_project() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let files = detect_version_files(root.to_str().unwrap());
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"package.json"), "{:?}", paths);
        assert!(paths.contains(&"src-tauri/Cargo.toml"), "{:?}", paths);
        assert!(paths.contains(&"src-tauri/tauri.conf.json"), "{:?}", paths);
        assert!(files.iter().all(|f| parse_version(&f.version).is_some()));
    }

    #[test]
    fn bump_writes_back_all_detected_files_preserving_format() {
        let dir = std::env::temp_dir().join(format!("omg-version-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pkg = "{\n  \"name\": \"x\",\n  \"version\": \"1.2.3\"\n}\n";
        let cargo = "[package]\nname = \"x\"\nversion = \"1.2.3\"\n";
        std::fs::write(dir.join("package.json"), pkg).unwrap();
        std::fs::write(dir.join("Cargo.toml"), cargo).unwrap();

        let repo = dir.to_str().unwrap().to_string();
        let info = project_version_detect(repo.clone());
        assert_eq!(info.files.len(), 2);
        let r = project_version_bump(repo, "minor".into()).unwrap();
        assert_eq!(r.old_version, "1.2.3");
        assert_eq!(r.new_version, "1.3.0");
        assert_eq!(r.updated.len(), 2);

        let p = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(p.contains("\"version\": \"1.3.0\""));
        assert!(p.contains("\"name\": \"x\""), "其他内容被破坏: {}", p);
        let c = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(c.contains("version = \"1.3.0\""));
        assert!(c.starts_with("[package]"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
