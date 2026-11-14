/**
 * Minimal Markdown subset built directly into React nodes (no `innerHTML`, so
 * raw HTML in model replies renders literally). Supports bold, italic, code,
 * headings, lists, blockquotes, rules, and `http(s)` links.
 */

import { type ReactNode } from "react";

import { api } from "../../lib/ipc/client";

const ALLOWED_LINK = /^https?:\/\//i;

/** Only http(s) links pass; credentials in the URL are rejected. */
export function safeHref(raw: string): string | null {
  const trimmed = raw.trim();
  if (!ALLOWED_LINK.test(trimmed)) return null;
  try {
    const url = new URL(trimmed);
    if (!["http:", "https:"].includes(url.protocol)) return null;
    if (url.username || url.password) return null;
    return url.href;
  } catch {
    return null;
  }
}

function matchMarkdownLink(
  rest: string,
): { full: string; label: string; url: string } | null {
  const head = /^\[([^\]\n]+)\]\(/.exec(rest);
  if (!head) return null;
  const label = head[1] ?? "";
  let pos = head[0].length;
  let depth = 0;
  while (pos < rest.length) {
    const ch = rest[pos]!;
    if (ch === "\n" || /\s/.test(ch)) return null;
    if (ch === "(") {
      depth += 1;
    } else if (ch === ")") {
      if (depth === 0) {
        const url = rest.slice(head[0].length, pos);
        if (!url) return null;
        return { full: rest.slice(0, pos + 1), label, url };
      }
      depth -= 1;
    }
    pos += 1;
  }
  return null;
}

const CJK_CHAR = /[\u3000-\u9FFF\uF900-\uFAFF\uFF00-\uFFEF]/;

function joinParagraphLines(lines: string[]): string {
  let out = "";
  for (const part of lines) {
    if (!out) {
      out = part;
      continue;
    }
    const prev = out[out.length - 1] ?? "";
    const next = part[0] ?? "";
    if (CJK_CHAR.test(prev) && CJK_CHAR.test(next)) {
      out += part;
    } else {
      out += ` ${part}`;
    }
  }
  return out;
}

/**
 * Inline spans for one line of text. Unmatched or nested-by-one-character
 * delimiters degrade to literal text rather than swallowing the rest of a reply:
 * an unterminated `**` is common in a cancelled answer.
 */
function inline(text: string, keyPrefix: string): ReactNode[] {
  const out: ReactNode[] = [];
  let index = 0;

  const pushText = (value: string) => {
    if (value) out.push(value);
  };

  while (index < text.length) {
    const rest = text.slice(index);
    // Code first: its contents must never be interpreted as bold or links.
    const code = /^`([^`\n]+)`/.exec(rest);
    if (code) {
      out.push(
        <code key={`${keyPrefix}-c${index}`} className="md-code">
          {code[1] ?? ""}
        </code>,
      );
      index += code[0].length;
      continue;
    }
    const link = matchMarkdownLink(rest);
    if (link) {
      const href = safeHref(link.url);
      if (href) {
        // Use a button backed by Rust's scheme allowlist; target="_blank" anchors navigate inside Android WebView.
        out.push(
          <button
            key={`${keyPrefix}-a${index}`}
            type="button"
            className="linkbtn"
            onClick={() => {
              void api.openExternal(href).catch(() => undefined);
            }}
          >
            {inline(link.label, `${keyPrefix}-a${index}`)}
          </button>,
        );
        index += link.full.length;
        continue;
      }
      // Not a safe link: fall through so it renders as the literal source.
    }
    const bold = /^\*\*([^*\n]+(?:\*[^*\n]+)*)\*\*/.exec(rest);
    if (bold) {
      out.push(
        <strong key={`${keyPrefix}-b${index}`}>
          {inline(bold[1] ?? "", `${keyPrefix}-b${index}`)}
        </strong>,
      );
      index += bold[0].length;
      continue;
    }
    const starItalic = /^\*([^*\n]+)\*/.exec(rest);
    if (starItalic) {
      out.push(
        <em key={`${keyPrefix}-i${index}`}>
          {inline(starItalic[1] ?? "", `${keyPrefix}-i${index}`)}
        </em>,
      );
      index += starItalic[0].length;
      continue;
    }
    const underItalic = /^_([^_\n]+)_/.exec(rest);
    if (
      underItalic &&
      (index === 0 || !/[a-zA-Z0-9_]/.test(text[index - 1] ?? "")) &&
      !/^[a-zA-Z0-9_]/.test(rest.slice(underItalic[0].length))
    ) {
      out.push(
        <em key={`${keyPrefix}-i${index}`}>
          {inline(underItalic[1] ?? "", `${keyPrefix}-i${index}`)}
        </em>,
      );
      index += underItalic[0].length;
      continue;
    }
    pushText(text[index] ?? "");
    index += 1;
  }
  return out;
}

const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}(\d{1,3})[.)]\s+(.*)$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*)$/;
const RULE = /^\s{0,3}(?:-{3,}|\*{3,}|_{3,})\s*$/;
const FENCE = /^\s{0,3}```([\w-]*)\s*$/;

export function renderMarkdown(source: string): ReactNode {
  if (!source) return null;
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let paragraph: string[] = [];
  let index = 0;

  const flush = () => {
    if (!paragraph.length) return;
    const key = `p${blocks.length}`;
    blocks.push(
      <p key={key} className="md-p">
        {inline(joinParagraphLines(paragraph), key)}
      </p>,
    );
    paragraph = [];
  };

  while (index < lines.length) {
    const line = lines[index] ?? "";
    const fence = FENCE.exec(line);
    if (fence) {
      flush();
      const lang = fence[1] ?? "";
      const codeLines: string[] = [];
      index += 1;
      while (index < lines.length) {
        const current = lines[index] ?? "";
        if (/^\s{0,3}```\s*$/.test(current)) {
          index += 1;
          break;
        }
        codeLines.push(current);
        index += 1;
      }
      blocks.push(
        <pre
          key={`code${blocks.length}`}
          className="md-pre"
          data-lang={lang || undefined}
        >
          <code>{codeLines.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    if (!line.trim()) {
      flush();
      index += 1;
      continue;
    }
    if (RULE.test(line)) {
      flush();
      blocks.push(<hr key={`hr${blocks.length}`} className="md-hr" />);
      index += 1;
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flush();
      const level = Math.min((heading[1] ?? "#").length + 2, 6);
      const Tag = `h${level}` as "h3" | "h4" | "h5" | "h6";
      blocks.push(
        <Tag key={`h${blocks.length}`} className={`md-h${level}`}>
          {inline(heading[2] ?? "", `h${blocks.length}`)}
        </Tag>,
      );
      index += 1;
      continue;
    }
    if (BULLET.test(line) || NUMBERED.test(line)) {
      flush();
      const ordered = !BULLET.test(line);
      const items: string[] = [];
      let start = 1;
      while (index < lines.length) {
        const current = lines[index] ?? "";
        if (!ordered) {
          const bullet = BULLET.exec(current);
          if (!bullet) break;
          items.push(bullet[1] ?? "");
        } else {
          const numbered = NUMBERED.exec(current);
          if (!numbered) break;
          if (items.length === 0) start = Number(numbered[1]);
          items.push(numbered[2] ?? "");
        }
        index += 1;
      }
      const key = `l${blocks.length}`;
      const children = items.map((item, position) => (
        <li key={`${key}-${position}`}>{inline(item, `${key}-${position}`)}</li>
      ));
      blocks.push(
        ordered ? (
          <ol key={key} className="md-list" start={start}>
            {children}
          </ol>
        ) : (
          <ul key={key} className="md-list">
            {children}
          </ul>
        ),
      );
      continue;
    }
    const quote = /^\s{0,3}>\s?(.*)$/.exec(line);
    if (quote) {
      flush();
      const quoted: string[] = [];
      while (index < lines.length) {
        const next = /^\s{0,3}>\s?(.*)$/.exec(lines[index] ?? "");
        if (!next) break;
        quoted.push(next[1] ?? "");
        index += 1;
      }
      const key = `q${blocks.length}`;
      blocks.push(
        <blockquote key={key} className="md-quote">
          {quoted.map((text, position) => (
            <p key={`${key}-${position}`} className="md-p">
              {inline(text, `${key}-${position}`)}
            </p>
          ))}
        </blockquote>,
      );
      continue;
    }
    paragraph.push(line.trim());
    index += 1;
  }
  flush();
  return blocks;
}
