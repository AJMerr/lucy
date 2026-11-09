// How a tool call reads to you, e.g. "run `git status`".

/** A tool call in a few words, e.g. "run `git status`" or "edit notes.md". */
export function describeCall(name: string, args: Record<string, unknown>): string {
  const arg = (key: string) => (typeof args[key] === "string" ? (args[key] as string) : "");
  switch (name) {
    case "run_command":
      return `run \`${arg("command")}\``;
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
    case "generate_image": {
      const prompt = arg("prompt");
      return `draw "${prompt.length > 80 ? `${prompt.slice(0, 79)}…` : prompt}"`;
    }
    default:
      return `use ${name}`;
  }
}
