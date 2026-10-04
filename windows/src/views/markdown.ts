// Markdown for chat answers, built node by node: the model's text never goes
// through innerHTML. Covers what chat answers actually use — paragraphs,
// headings, lists, quotes, fenced code with a copy button, inline code, bold,
// italic and links. Only http(s) links open, in the browser.

import { h } from "./dom";

type Block =
  | { kind: "p"; text: string }
  | { kind: "h"; level: number; text: string }
  | { kind: "quote"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "code"; lang: string; text: string };

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}\d{1,3}[.)]\s+(.*)$/;

/** Models that think out loud wrap it in <think>; the answer is what follows. */
export function stripThinking(text: string): string {
  return text.replace(/<think>[\s\S]*?(<\/think>|$)/g, "").trim();
}

export function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) blocks.push({ kind: "p", text: para.join("\n") });
    para = [];
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = FENCE.exec(line);
    if (fence) {
      flush();
      const body: string[] = [];
      for (i++; i < lines.length && !lines[i].trim().startsWith(fence[1]); i++) body.push(lines[i]);
      blocks.push({ kind: "code", lang: fence[2], text: body.join("\n") });
      continue;
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flush();
      blocks.push({ kind: "h", level: heading[1].length, text: heading[2] });
      continue;
    }
    if (QUOTE.test(line)) {
      flush();
      const body: string[] = [];
      for (; i < lines.length && QUOTE.test(lines[i]); i++) body.push(QUOTE.exec(lines[i])![1]);
      i--;
      blocks.push({ kind: "quote", text: body.join("\n") });
      continue;
    }
    const ordered = NUMBERED.test(line);
    if (ordered || BULLET.test(line)) {
      flush();
      const re = ordered ? NUMBERED : BULLET;
      const items: string[] = [];
      for (; i < lines.length; i++) {
        const m = re.exec(lines[i]);
        if (m) items.push(m[1]);
        // An indented line continues the item above it.
        else if (items.length && /^\s{2,}\S/.test(lines[i])) items[items.length - 1] += " " + lines[i].trim();
        else break;
      }
      i--;
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    para.push(line);
  }
  flush();
  return blocks;
}

const INLINE = /(`+)([\s\S]+?)\1|\*\*([\s\S]+?)\*\*|__([\s\S]+?)__|\*([^*\s][^*]*?)\*|(?<![\w])_([^_\s][^_]*?)_(?![\w])|\[([^\]]+)\]\(([^)\s]+)\)/g;

export function isWebLink(url: string): boolean {
  return /^https?:\/\/[^\s]+$/i.test(url);
}

function inline(text: string, openUrl: (url: string) => void): Node[] {
  const out: Node[] = [];
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) out.push(document.createTextNode(text.slice(last, at)));
    last = at + m[0].length;
    if (m[2] != null) out.push(h("code", { class: "md-code", text: m[2].trim() }));
    else if (m[3] != null || m[4] != null) out.push(h("strong", {}, ...inline(m[3] ?? m[4], openUrl)));
    else if (m[5] != null || m[6] != null) out.push(h("em", {}, ...inline(m[5] ?? m[6], openUrl)));
    else if (m[7] != null) {
      const url = m[8];
      if (isWebLink(url)) {
        const a = h("a", { class: "md-link", title: url }, ...inline(m[7], openUrl));
        a.addEventListener("click", (e) => {
          e.preventDefault();
          openUrl(url);
        });
        out.push(a);
      } else {
        out.push(document.createTextNode(m[7]));
      }
    }
  }
  if (last < text.length) out.push(document.createTextNode(text.slice(last)));
  return out;
}

function copyButton(text: string): HTMLElement {
  const b = h("button", { class: "md-copy", text: "Copy" });
  b.addEventListener("click", () => {
    void navigator.clipboard?.writeText(text).then(() => {
      b.textContent = "Copied";
      window.setTimeout(() => (b.textContent = "Copy"), 1200);
    });
  });
  return b;
}

export function renderMarkdown(source: string, openUrl: (url: string) => void): HTMLElement {
  const root = h("div", { class: "md" });
  for (const b of parseBlocks(stripThinking(source))) {
    switch (b.kind) {
      case "p":
        root.append(h("p", {}, ...inline(b.text, openUrl)));
        break;
      case "h":
        root.append(h("div", { class: `md-h md-h${Math.min(b.level, 3)}` }, ...inline(b.text, openUrl)));
        break;
      case "quote":
        root.append(h("blockquote", {}, ...inline(b.text, openUrl)));
        break;
      case "list": {
        const list = h(b.ordered ? "ol" : "ul", {});
        for (const item of b.items) list.append(h("li", {}, ...inline(item, openUrl)));
        root.append(list);
        break;
      }
      case "code":
        root.append(h("div", { class: "md-pre" },
          h("div", { class: "md-pre-head" }, h("span", { text: b.lang }), copyButton(b.text)),
          h("pre", { text: b.text })));
        break;
    }
  }
  return root;
}
