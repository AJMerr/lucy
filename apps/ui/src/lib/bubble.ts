// Her reply as bubble HTML: a light Markdown subset (paragraphs, headings as bold lines,
// lists, code blocks, bold, italics, inline code; links keep only their text), always
// escaped, and capped to a few lines with "…" unless expanded. Works on a reply that's still
// streaming: unclosed markers stay text, an unclosed code block shows what's arrived.

export interface BubbleOptions {
  /** How many lines fit before "more…". */
  maxLines: number;
  /** Roughly how many characters fit on a line, for estimating wrapping. */
  charsPerLine: number;
  /** Show everything. */
  expanded: boolean;
}

export interface Shown {
  html: string;
  /** Some of the reply didn't fit. */
  more: boolean;
}

type Block =
  | { kind: "p" | "h"; text: string }
  | { kind: "ul" | "ol"; items: string[] }
  | { kind: "code"; lines: string[] };

const ELLIPSIS = "…";

export function renderBubble(text: string, options: BubbleOptions): Shown {
  const blocks = parse(text);
  if (options.expanded) {
    return { html: blocks.map(render).join(""), more: false };
  }
  const shown: Block[] = [];
  let left = options.maxLines;
  let more = false;
  for (const block of blocks) {
    const lines = height(block, options.charsPerLine);
    if (left <= 0) {
      more = true;
      break;
    }
    if (lines <= left) {
      shown.push(block);
      left -= lines;
      continue;
    }
    shown.push(cut(block, left, options.charsPerLine));
    more = true;
    left = 0;
    break;
  }
  if (more) {
    const last = shown.pop();
    if (last !== undefined) {
      shown.push(endWithEllipsis(last));
    }
  }
  return { html: shown.map(render).join(""), more };
}

function parse(text: string): Block[] {
  const blocks: Block[] = [];
  const lines = text.split("\n");
  let paragraph: string[] = [];
  const endParagraph = () => {
    if (paragraph.length > 0) {
      blocks.push({ kind: "p", text: paragraph.join(" ") });
      paragraph = [];
    }
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? "";
    if (line.trimStart().startsWith("```")) {
      endParagraph();
      const code: string[] = [];
      for (i++; i < lines.length && !(lines[i] ?? "").trimStart().startsWith("```"); i++) {
        code.push(lines[i] ?? "");
      }
      blocks.push({ kind: "code", lines: code });
      continue;
    }
    if (line.trim() === "") {
      endParagraph();
      continue;
    }
    const heading = /^#{1,6}\s+(.*)$/.exec(line);
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    const numbered = /^\s*\d+[.)]\s+(.*)$/.exec(line);
    if (heading !== null) {
      endParagraph();
      blocks.push({ kind: "h", text: heading[1] ?? "" });
    } else if (bullet !== null || numbered !== null) {
      endParagraph();
      const kind = bullet !== null ? "ul" : "ol";
      const item = (bullet ?? numbered)?.[1] ?? "";
      const previous = blocks[blocks.length - 1];
      if (previous !== undefined && previous.kind === kind) {
        previous.items.push(item);
      } else {
        blocks.push({ kind, items: [item] });
      }
    } else {
      paragraph.push(line.trim());
    }
  }
  endParagraph();
  return blocks;
}

/** Estimated lines a block takes when wrapped. */
function height(block: Block, charsPerLine: number): number {
  const wrapped = (text: string) => Math.max(1, Math.ceil(text.length / charsPerLine));
  switch (block.kind) {
    case "p":
    case "h":
      return wrapped(block.text);
    case "ul":
    case "ol":
      return block.items.reduce((sum, item) => sum + wrapped(`- ${item}`), 0);
    case "code":
      return block.lines.reduce((sum, line) => sum + wrapped(line), 0);
  }
}

/** The part of `block` that fits in `lines` lines. */
function cut(block: Block, lines: number, charsPerLine: number): Block {
  switch (block.kind) {
    case "p":
    case "h":
      return { kind: block.kind, text: cutText(block.text, lines * charsPerLine) };
    case "ul":
    case "ol": {
      const items: string[] = [];
      let left = lines;
      for (const item of block.items) {
        const needed = Math.max(1, Math.ceil((item.length + 2) / charsPerLine));
        if (needed > left) {
          if (left > 0) {
            items.push(cutText(item, left * charsPerLine - 2));
          }
          break;
        }
        items.push(item);
        left -= needed;
      }
      return { kind: block.kind, items };
    }
    case "code":
      return { kind: "code", lines: block.lines.slice(0, Math.max(1, lines)) };
  }
}

/** At most `chars` characters, ending at a word where possible (no ellipsis yet). */
function cutText(text: string, chars: number): string {
  if (text.length <= chars) {
    return text;
  }
  const window = text.slice(0, chars);
  const space = window.lastIndexOf(" ");
  return (space > chars / 2 ? window.slice(0, space) : window.slice(0, chars - 1)).trimEnd();
}

function endWithEllipsis(block: Block): Block {
  switch (block.kind) {
    case "p":
    case "h":
      return { ...block, text: block.text + ELLIPSIS };
    case "ul":
    case "ol":
      return { ...block, items: [...block.items.slice(0, -1), `${block.items.at(-1) ?? ""}${ELLIPSIS}`] };
    case "code":
      return { ...block, lines: [...block.lines, ELLIPSIS] };
  }
}

function render(block: Block): string {
  switch (block.kind) {
    case "p":
      return `<p>${inline(block.text)}</p>`;
    case "h":
      return `<p><strong>${inline(block.text)}</strong></p>`;
    case "ul":
    case "ol":
      return `<${block.kind}>${block.items.map((item) => `<li>${inline(item)}</li>`).join("")}</${block.kind}>`;
    case "code":
      return `<pre><code>${escape(block.lines.join("\n"))}</code></pre>`;
  }
}

/** Inline code first (its contents stay literal), then links, bold and italics. */
function inline(text: string): string {
  return text
    .split(/(`[^`]+`)/)
    .map((part) =>
      part.length > 1 && part.startsWith("`") && part.endsWith("`")
        ? `<code>${escape(part.slice(1, -1))}</code>`
        : emphasis(escape(part)),
    )
    .join("");
}

function emphasis(text: string): string {
  return text
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*\w])\*([^*\s][^*]*)\*(?!\*)/g, "$1<em>$2</em>")
    .replace(/(^|[^\w])_([^_\s][^_]*)_(?!\w)/g, "$1<em>$2</em>");
}

function escape(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
