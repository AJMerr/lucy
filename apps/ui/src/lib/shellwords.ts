// Splitting a pasted command line the way a shell would: words, quotes and backslashes, with
// leading NAME=value words as environment variables.

export interface Split {
  env: Record<string, string>;
  words: string[];
}

export function splitCommand(line: string): Split {
  const words: string[] = [];
  let word = "";
  let inWord = false;
  let quote: "'" | '"' | null = null;
  for (let i = 0; i < line.length; i++) {
    const c = line[i] as string;
    if (quote === "'") {
      if (c === "'") {
        quote = null;
      } else {
        word += c;
      }
    } else if (quote === '"') {
      if (c === '"') {
        quote = null;
      } else if (c === "\\" && i + 1 < line.length && /["\\$`]/.test(line[i + 1] as string)) {
        word += line[++i];
      } else {
        word += c;
      }
    } else if (c === "'" || c === '"') {
      quote = c;
      inWord = true;
    } else if (c === "\\" && i + 1 < line.length) {
      word += line[++i];
      inWord = true;
    } else if (/\s/.test(c)) {
      if (inWord) {
        words.push(word);
        word = "";
        inWord = false;
      }
    } else {
      word += c;
      inWord = true;
    }
  }
  if (quote !== null) {
    throw new Error(`an unclosed ${quote} quote`);
  }
  if (inWord) {
    words.push(word);
  }
  const env: Record<string, string> = {};
  while (words.length > 0 && /^[A-Za-z_][A-Za-z0-9_]*=/.test(words[0] as string)) {
    const assignment = words.shift() as string;
    const at = assignment.indexOf("=");
    env[assignment.slice(0, at)] = assignment.slice(at + 1);
  }
  return { env, words };
}
