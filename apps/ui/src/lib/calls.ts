// How a tool call reads to you, e.g. "run `git status`".

/** Characters that hide or rearrange text, or only look like a space. */
const HIDING =
  /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u00a0\u061c\u1680\u180e\u2000-\u200f\u202a-\u202f\u205f-\u2069\u2800\u3000\u3164\ufeff\uffa0]/gu;

/** `text` with every character that could hide or reorder part of it shown as `<U+202E>`. */
export function visibleText(text: string): string {
  return text.replace(HIDING, (c) => `<U+${c.codePointAt(0)?.toString(16).toUpperCase().padStart(4, "0")}>`);
}

/** Text that's too long, or has hidden characters, to approve from a glance at the sprite. */
export function needsReview(text: string): boolean {
  return text.length > 200 || text.split("\n").length > 4 || visibleText(text) !== text;
}

/** What a call would do, in full, for approving it: the whole command, or every argument of a
 * tool without a summary of its own. `undefined` when the summary says it all. */
export function callDetails(name: string, args: Record<string, unknown>): string | undefined {
  if (name === "run_command") {
    const command = typeof args.command === "string" ? args.command : "";
    const shown = visibleText(command);
    // A short, plain command reads in full in the summary already.
    return shown !== command || command.length > 80 || command.includes("\n") ? shown : undefined;
  }
  if (describeCall(name, args).startsWith("use ")) {
    return visibleText(JSON.stringify(args, null, 2));
  }
  return undefined;
}

/** A tool call in a few words, e.g. "run `git status`" or "edit notes.md". */
export function describeCall(name: string, args: Record<string, unknown>): string {
  const arg = (key: string) => (typeof args[key] === "string" ? visibleText(args[key] as string) : "");
  switch (name) {
    case "run_command": {
      const command = arg("command");
      return `run \`${command.length > 80 ? `${command.slice(0, 79)}…` : command}\``;
    }
    case "write_file":
      return `write ${arg("path")}`;
    case "edit_file":
      return `edit ${arg("path")}`;
    case "read_file":
      return `read ${arg("path")}`;
    case "list_dir":
      return `look in ${arg("path") || "."}`;
    case "search_files":
      return `search for "${arg("pattern")}"`;
    case "fetch_url":
      return `open ${arg("url")}`;
    case "remember":
      return `remember "${arg("fact")}"`;
    case "forget":
      return `forget "${arg("fact")}"`;
    case "recall":
      return `search her memory for "${arg("query")}"`;
    case "schedule":
      return `schedule "${arg("title") || arg("what")}"`;
    case "list_schedules":
      return "look at what's scheduled";
    case "cancel_schedule":
      return `cancel "${arg("which")}"`;
    case "generate_image": {
      const prompt = arg("prompt");
      return `draw "${prompt.length > 80 ? `${prompt.slice(0, 79)}…` : prompt}"`;
    }
    default:
      return `use ${name}`;
  }
}
