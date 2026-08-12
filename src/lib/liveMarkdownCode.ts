export interface LiveMarkdownCodeFence {
  from: number;
  to: number;
  openingLine: number;
  closingLine?: number;
  lineNumbers: number[];
  marker: "`" | "~";
  markerLength: number;
  info: LiveMarkdownCodeRange;
  language: string;
  languageRange?: LiveMarkdownCodeRange;
  code: string;
}

export interface LiveMarkdownCodeRange {
  from: number;
  to: number;
}

interface SourceLine {
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  source: string;
}

export function parseLiveMarkdownCodeFences(
  value: string,
): LiveMarkdownCodeFence[] {
  const lines = readSourceLines(value);
  const frontmatterEnd = findFrontmatterEnd(lines);
  const fences: LiveMarkdownCodeFence[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    if (frontmatterEnd !== undefined && index <= frontmatterEnd) {
      index = frontmatterEnd;

      continue;
    }

    const openingLine = lines[index]!;
    const opening = openingLine.source.match(/^ {0,3}(`{3,}|~{3,})([^\r\n]*)$/);
    if (!opening) {
      continue;
    }

    const marker = opening[1]!;
    const markerEnd = openingLine.from + opening[0].indexOf(marker) + marker.length;
    const contentLines: SourceLine[] = [];
    let closingLine: SourceLine | undefined;
    let cursor = index + 1;

    for (; cursor < lines.length; cursor += 1) {
      const candidate = lines[cursor]!;
      if (closesFence(candidate.source, marker)) {
        closingLine = candidate;

        break;
      }
      contentLines.push(candidate);
    }

    const finalLine = closingLine ?? contentLines.at(-1) ?? openingLine;
    const lineNumbers = lines
      .slice(index, (closingLine ? cursor : lines.length - 1) + 1)
      .map((line) => line.lineNumber);
    const infoSource = openingLine.source.slice(markerEnd - openingLine.from);
    const languageToken = infoSource.match(/\S+/);
    const language = (languageToken?.[0] ?? "")
      .replace(/[^a-zA-Z0-9_+-]/g, "")
      .slice(0, 40);
    const languageRange = languageToken?.index === undefined
      ? undefined
      : {
          from: markerEnd + languageToken.index,
          to: markerEnd + languageToken.index + languageToken[0].length,
        };

    fences.push({
      from: openingLine.from,
      to: finalLine.end,
      openingLine: openingLine.lineNumber,
      ...(closingLine ? { closingLine: closingLine.lineNumber } : {}),
      lineNumbers,
      marker: marker[0]! as "`" | "~",
      markerLength: marker.length,
      info: { from: markerEnd, to: openingLine.to },
      language,
      ...(languageRange ? { languageRange } : {}),
      code: contentLines.map((line) => line.source).join("\n"),
    });

    index = closingLine ? cursor : lines.length;
  }

  return fences;
}

function findFrontmatterEnd(lines: readonly SourceLine[]): number | undefined {
  const opening = lines[0]?.source.replace(/^\u{feff}/u, "").trimEnd();
  if (opening !== "---") {
    return undefined;
  }

  for (let index = 1; index < lines.length; index += 1) {
    const line = lines[index]!.source.trimEnd();
    if (line === "---" || line === "...") {
      return index;
    }
  }

  return undefined;
}

function closesFence(source: string, openingMarker: string): boolean {
  const closing = source.match(/^ {0,3}(`+|~+)\s*$/)?.[1];

  return Boolean(
    closing &&
    closing[0] === openingMarker[0] &&
    closing.length >= openingMarker.length
  );
}

function readSourceLines(value: string): SourceLine[] {
  const lines: SourceLine[] = [];
  const lineEnding = /\r\n|\n|\r/g;
  let from = 0;
  let lineNumber = 1;
  let match: RegExpExecArray | null;

  while ((match = lineEnding.exec(value))) {
    lines.push({
      lineNumber,
      from,
      to: match.index,
      end: lineEnding.lastIndex,
      source: value.slice(from, match.index),
    });
    from = lineEnding.lastIndex;
    lineNumber += 1;
  }

  lines.push({
    lineNumber,
    from,
    to: value.length,
    end: value.length,
    source: value.slice(from),
  });

  return lines;
}
