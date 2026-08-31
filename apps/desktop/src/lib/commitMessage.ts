// Assembling and splitting commit messages (commit box ↔ message log).
// Pure functions — covered by Vitest (commitMessage.test.ts).

export interface ParsedCommitMessage {
  summary: string;
  description: string;
  /** Comma-separated co-authors ("Name <mail>, …"), as in the input field. */
  coAuthors: string;
}

/** Builds the full commit message: subject, blank line, body, Co-authored-by trailer. */
export function buildCommitMessage(
  summary: string,
  description: string,
  coAuthors: string,
): string {
  let message = description.trim() ? `${summary.trim()}\n\n${description.trim()}` : summary.trim();
  // Append the Co-authored-by trailer (GitHub convention).
  const authors = coAuthors
    .split(/[,;\n]/)
    .map((s) => s.trim())
    .filter((s) => s.includes("@"));
  if (authors.length > 0) {
    message += "\n\n" + authors.map((a) => `Co-authored-by: ${a}`).join("\n");
  }
  return message;
}

/** Splits a message back into subject/body/co-authors (the inverse of build). */
export function parseCommitMessage(msg: string): ParsedCommitMessage {
  const idx = msg.indexOf("\n\n");
  const summary = idx === -1 ? msg : msg.slice(0, idx);
  let body = idx === -1 ? "" : msg.slice(idx + 2);
  // Lift the Co-authored-by trailer back into its own field.
  const authors: string[] = [];
  body = body
    .split("\n")
    .filter((line) => {
      const m = line.match(/^co-authored-by:\s*(.+)$/i);
      if (m) {
        authors.push(m[1].trim());
        return false;
      }
      return true;
    })
    .join("\n")
    .trim();
  return { summary, description: body, coAuthors: authors.join(", ") };
}
