export interface EditableMarkdownDocument {
  body: string;
  bodyStart: number;
  lineNumberOffset: number;
  prefix: string;
}

interface SourceLine {
  end: number;
  source: string;
  terminated: boolean;
}

export function splitLeadingFrontmatter(
  markdown: string,
): EditableMarkdownDocument {
  const bodyStart = leadingFrontmatterEnd(markdown);
  if (bodyStart === undefined) {
    return {
      body: markdown,
      bodyStart: 0,
      lineNumberOffset: 0,
      prefix: "",
    };
  }

  const prefix = markdown.slice(0, bodyStart);

  return {
    body: markdown.slice(bodyStart),
    bodyStart,
    lineNumberOffset: frontmatterLineCount(prefix),
    prefix,
  };
}

export function joinLeadingFrontmatter(prefix: string, body: string): string {
  if (!prefix || !body || endsWithLineEnding(prefix)) {
    return `${prefix}${body}`;
  }

  return `${prefix}\n${body}`;
}

export function markdownBodyStart(prefix: string, body: string): number {
  const separatorLength = prefix && body && !endsWithLineEnding(prefix) ? 1 : 0;

  return prefix.length + separatorLength;
}

export function leadingFrontmatterEnd(markdown: string): number | undefined {
  const opening = readSourceLine(markdown, 0);
  const openingSource = opening.source.replace(/^\u{feff}/u, "").trimEnd();
  if (openingSource !== "---" || !opening.terminated) {
    return undefined;
  }

  let lineStart = opening.end;
  while (lineStart <= markdown.length) {
    const line = readSourceLine(markdown, lineStart);
    const source = line.source.trimEnd();
    if (source === "---" || source === "...") {
      return line.end;
    }
    if (!line.terminated) {
      return undefined;
    }
    lineStart = line.end;
  }

  return undefined;
}

function readSourceLine(markdown: string, from: number): SourceLine {
  for (let index = from; index < markdown.length; index += 1) {
    const character = markdown[index];
    if (character === "\n") {
      return {
        end: index + 1,
        source: markdown.slice(from, index),
        terminated: true,
      };
    }
    if (character === "\r") {
      return {
        end: markdown[index + 1] === "\n" ? index + 2 : index + 1,
        source: markdown.slice(from, index),
        terminated: true,
      };
    }
  }

  return {
    end: markdown.length,
    source: markdown.slice(from),
    terminated: false,
  };
}

function frontmatterLineCount(prefix: string): number {
  const lineEndings = prefix.match(/\r\n|\n|\r/g)?.length ?? 0;

  return endsWithLineEnding(prefix) ? lineEndings : lineEndings + 1;
}

function endsWithLineEnding(value: string): boolean {
  return /(?:\r\n|\n|\r)$/.test(value);
}
