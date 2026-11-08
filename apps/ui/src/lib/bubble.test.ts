// Turning a reply (light Markdown, possibly still streaming) into the bubble's HTML, capped to
// a few lines with "more…".

import { test } from "vitest";
import assert from "node:assert/strict";

import { renderBubble } from "./bubble.ts";

const roomy = { maxLines: 100, charsPerLine: 40, expanded: false };

function html(text: string): string {
  return renderBubble(text, roomy).html;
}

test("plain text becomes a paragraph", () => {
  assert.equal(html("Hey, choom."), "<p>Hey, choom.</p>");
});

test("HTML in a reply is shown, never run", () => {
  assert.equal(
    html(`<img src=x onerror="alert(1)"> & 'quotes'`),
    "<p>&lt;img src=x onerror=&quot;alert(1)&quot;&gt; &amp; &#39;quotes&#39;</p>",
  );
});

test("bold, italics and inline code", () => {
  assert.equal(
    html("**Run** *this*: `git <status>`"),
    "<p><strong>Run</strong> <em>this</em>: <code>git &lt;status&gt;</code></p>",
  );
});

test("an unclosed marker mid-stream stays as text", () => {
  assert.equal(html("This is **impor"), "<p>This is **impor</p>");
});

test("lists become lists", () => {
  assert.equal(
    html("Steps:\n- one\n- two\n\n1. first\n2. second"),
    "<p>Steps:</p><ul><li>one</li><li>two</li></ul><ol><li>first</li><li>second</li></ol>",
  );
});

test("code blocks are monospace and escaped", () => {
  assert.equal(
    html("Try:\n```sh\necho <hi>\n```\nDone."),
    "<p>Try:</p><pre><code>echo &lt;hi&gt;</code></pre><p>Done.</p>",
  );
});

test("a code block still streaming shows what's arrived so far", () => {
  assert.equal(html("```\nfn main() {"), "<pre><code>fn main() {</code></pre>");
});

test("headings are bold lines, and links keep only their text", () => {
  assert.equal(
    html("## Plan\nSee [the docs](https://example.com)."),
    "<p><strong>Plan</strong></p><p>See the docs.</p>",
  );
});

test("lines within a paragraph stay together", () => {
  assert.equal(html("one\ntwo\n\nthree"), "<p>one two</p><p>three</p>");
});

test("a short reply fits, with no \"more\"", () => {
  const shown = renderBubble("Short and sweet.", { maxLines: 8, charsPerLine: 40, expanded: false });

  assert.equal(shown.more, false);
  assert.equal(shown.html, "<p>Short and sweet.</p>");
});

test("a long reply is cut at a word, marked with an ellipsis, and offers more", () => {
  const text = "word ".repeat(40).trim(); // 199 characters: 5 lines of 40
  const shown = renderBubble(text, { maxLines: 2, charsPerLine: 40, expanded: false });

  assert.equal(shown.more, true);
  assert.match(shown.html, /^<p>(word ){15}word…<\/p>$/);
});

test("the cap counts whole blocks, so later paragraphs drop out", () => {
  const shown = renderBubble("First.\n\nSecond.\n\nThird.", {
    maxLines: 2,
    charsPerLine: 40,
    expanded: false,
  });

  assert.equal(shown.more, true);
  assert.equal(shown.html, "<p>First.</p><p>Second.…</p>");
});

test("expanded shows everything", () => {
  const text = "word ".repeat(40).trim();
  const shown = renderBubble(text, { maxLines: 2, charsPerLine: 40, expanded: true });

  assert.equal(shown.more, false);
  assert.equal(shown.html, `<p>${text}</p>`);
});
