/* 生成 Release 更新说明:汇总上一个 tag 到当前 tag 之间的 Conventional Commits,
   按类型分组输出 Markdown。项目提交多为直推 main、没有 PR,GitHub 的 --generate-notes
   只会产出一行 Full Changelog 链接,因此这里自行生成有内容的说明。
   用法: node scripts/release_notes.mjs <tag> <owner/repo> <输出文件> */
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const [tag, repo, out] = process.argv.slice(2);
if (!tag || !repo || !out) {
  console.error("用法: node scripts/release_notes.mjs <tag> <owner/repo> <输出文件>");
  process.exit(1);
}

function git(args) {
  return execFileSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  }).trim();
}

// 上一个 tag:从当前 tag 的父提交定位。tag 此时已由 create 步骤创建。
let prev = "";
try {
  prev = git(["describe", "--tags", "--abbrev=0", `${tag}^`]);
} catch {
  // 首个版本没有上一个 tag
}

const range = prev ? `${prev}..${tag}` : tag;
const raw = git(["log", "--no-merges", "--pretty=format:%h%x09%s", range]);

const groups = [
  { re: /^feat(\(|:)/i, title: "✨ 新功能" },
  { re: /^fix(\(|:)/i, title: "🐛 问题修复" },
  { re: /^perf(\(|:)/i, title: "⚡ 性能优化" },
  { re: /^refactor(\(|:)/i, title: "♻️ 代码重构" },
  { re: /^docs?(\(|:)/i, title: "📝 文档" },
  { re: /^(style|ui)(\(|:)/i, title: "💄 界面与样式" },
  { re: /^(test|build|ci|chore)(\(|:)/i, title: "🔧 工程与维护" },
];

const commits = [];
for (const line of raw.split("\n")) {
  if (!line.trim()) continue;
  const [hash, ...rest] = line.trim().split("\t");
  const subject = rest.join("\t");
  // 版本提升提交不是面向用户的变更
  if (/^chore\(release\):\s*发布\s*v/i.test(subject)) continue;
  commits.push({ hash, subject });
}

const sections = [];
for (const group of groups) {
  const items = commits.filter((c) => group.re.test(c.subject));
  if (!items.length) continue;
  sections.push(
    `### ${group.title}\n` +
      items.map((c) => `- ${c.subject} (\`${c.hash}\`)`).join("\n"),
  );
}
const others = commits.filter((c) => !groups.some((g) => g.re.test(c.subject)));
if (others.length) {
  sections.push(
    "### 📦 其他变更\n" +
      others.map((c) => `- ${c.subject} (\`${c.hash}\`)`).join("\n"),
  );
}

let body = "## 更新内容\n\n";
body += sections.length ? sections.join("\n\n") : "- 常规维护与细节改进";
body += "\n\n---\n";
if (prev) {
  body += `**完整变更**：[${prev}...${tag}](https://github.com/${repo}/compare/${prev}...${tag})\n`;
} else {
  body += `**版本**：${tag}\n`;
}

writeFileSync(out, body + "\n");
console.log(`已生成 ${out}: ${commits.length} 条提交`);
