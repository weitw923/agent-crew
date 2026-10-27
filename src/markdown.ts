// Markdown 渲染：仅用于展示 Agent 产出的设计文档 / 摘要 / 技术栈 / 契约等。
// 关键安全约束：html: false —— 原始 HTML 会被转义，避免模型输出注入脚本。
import MarkdownIt from "markdown-it";

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
  typographer: false
});

// 图片不外链加载：避免 Agent 输出远程图片导致 WebView 主动发起请求（隐私 / 追踪）。
md.renderer.rules.image = (tokens, idx) => {
  const alt = tokens[idx].content || "图片";
  return `<span class="md-image-alt">[图片：${md.utils.escapeHtml(alt)}]</span>`;
};

// 外链统一新窗口打开；markdown-it 默认已拦截 javascript: 等危险协议。
const defaultLinkOpen =
  md.renderer.rules.link_open ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  const href = String(token.attrGet("href") ?? "");
  if (/^https?:\/\//i.test(href)) {
    token.attrSet("target", "_blank");
    token.attrSet("rel", "noreferrer noopener");
  }
  return defaultLinkOpen(tokens, idx, options, env, self);
};

// 表格外包一层横向滚动容器：宽表格只在面板内滚动，不会把文字顶出窗口。
const defaultTableOpen =
  md.renderer.rules.table_open ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.table_open = (tokens, idx, options, env, self) =>
  `<div class="md-table-wrap">${defaultTableOpen(tokens, idx, options, env, self)}`;
const defaultTableClose =
  md.renderer.rules.table_close ??
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.table_close = (tokens, idx, options, env, self) =>
  `${defaultTableClose(tokens, idx, options, env, self)}</div>`;

/** 渲染 Markdown 为安全 HTML 字符串。空内容返回空串。 */
export function renderMarkdown(raw: string): string {
  if (!raw || !raw.trim()) return "";
  return md.render(raw);
}

/**
 * 若正文以与标题完全相同的 H1 开头（例如 `# 技术栈`），去掉这行避免标题重复。
 * 只在完全一致时处理，`# 模块设计方案：xxx` 这类带补充信息的标题会保留。
 */
export function stripDuplicateHeading(raw: string, title: string): string {
  if (!raw) return raw;
  const lines = raw.split("\n");
  const firstIdx = lines.findIndex((l) => l.trim() !== "");
  if (firstIdx < 0) return raw;
  const match = /^#{1,6}\s+(.+?)\s*#*\s*$/.exec(lines[firstIdx].trim());
  if (!match) return raw;
  const headingText = match[1].trim();
  if (headingText.toLowerCase() !== title.trim().toLowerCase()) return raw;
  lines.splice(firstIdx, 1);
  return lines.join("\n").replace(/^\n+/, "");
}

/** 把 Markdown 压成一行纯文本摘要（用于对话时间线里的卡片预览）。 */
export function markdownToSnippet(raw: string, limit = 260): string {
  if (!raw) return "";
  let text = raw
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/^\s{0,3}#{1,6}\s+/gm, "")
    .replace(/^\s{0,3}[-*+]\s+/gm, "· ")
    .replace(/^\s{0,3}\d+\.\s+/gm, "")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/~~([^~]+)~~/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^\s{0,3}>\s?/gm, "")
    .replace(/^\s{0,3}([-*_])\s*\1\s*\1[\s\S]*$/gm, " ")
    .replace(/\n{2,}/g, "\n")
    .replace(/[ \t]+/g, " ")
    .trim();
  if (text.length > limit) text = `${text.slice(0, limit)}…`;
  return text;
}
