// Turning her replies into speech: the prose of her Markdown (code, tables and link addresses
// stay in the chat), split into sentences that are spoken as soon as each is complete.

const CODE = "I've put the code in the chat.";
const TABLE = "There's a table in the chat.";

/** Her reply as something to say aloud. */
export function speakable(markdown: string): string {
  const parts: string[] = [];
  // Between ``` fences is code (odd pieces); one still open at the end isn't said at all.
  const pieces = markdown.split("```");
  pieces.forEach((piece, i) => {
    if (i % 2 === 0) {
      prose(piece, parts);
    } else if (i < pieces.length - 1) {
      parts.push(CODE);
    }
  });
  return parts.join(" ").replace(/\s+/g, " ").trim();
}

/** Text outside code blocks, a line at a time, into `parts`. */
function prose(text: string, parts: string[]): void {
  let inTable = false;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("|")) {
      if (!inTable) {
        parts.push(TABLE);
      }
      inTable = true;
      continue;
    }
    inTable = false;
    const said = inline(line);
    if (said === "") {
      continue;
    }
    // Headings and list items are sentences of their own.
    const block = /^(#{1,6}\s|[-*+]\s|\d+[.)]\s)/.test(line);
    parts.push(block && !/[.!?:…]$/.test(said) ? `${said}.` : said);
  }
}

/** One line's prose: markup and addresses gone. */
function inline(line: string): string {
  return (
    line
      .replace(/^(#{1,6}\s+|[-*+]\s+|\d+[.)]\s+|>\s*)/, "")
      .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/https?:\/\/\S+?(?=[.,;:!?)]*(\s|$))/g, "a link")
      .replace(/`([^`]*)`/g, "$1")
      .replace(/\*\*|__|\*/g, "")
      .trim()
  );
}

/** Sentences in `text`: ending in . ! ? or … (with any closing quote). */
function sentences(text: string): string[] {
  return text.match(/[^.!?…]+(?:[.!?…]+["'”’)]*|$)/g)?.map((s) => s.trim()).filter((s) => s !== "") ?? [];
}

/**
 * Speech for a reply that's still arriving: give it the whole text so far, and it returns the
 * sentences that are now complete and haven't been spoken yet.
 */
export class SpeechStream {
  private spoken = 0;

  feed(markdown: string): string[] {
    // An unclosed code block is still being written: speak only up to it.
    const fences = (markdown.match(/```/g) ?? []).length;
    const settled = fences % 2 === 1 ? markdown.slice(0, markdown.lastIndexOf("```")) : markdown;
    const all = sentences(speakable(settled));
    // The last one may still be growing, unless the text ends with a code block.
    const complete = /[.!?…]["'”’)]*$/.test(all.at(-1) ?? "") ? all : all.slice(0, -1);
    return this.take(complete);
  }

  /** The reply is done: whatever hasn't been spoken. */
  finish(markdown: string): string[] {
    return this.take(sentences(speakable(markdown)));
  }

  private take(all: string[]): string[] {
    const fresh = all.slice(this.spoken);
    this.spoken = Math.max(this.spoken, all.length);
    return fresh;
  }
}
