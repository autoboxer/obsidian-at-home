import { parseLiveMarkdownBlocks } from "./liveMarkdown";
import type { LiveMarkdownBlock } from "./liveMarkdown";

export type LiveMarkdownTableAlignment = "center" | "left" | "right";
export type LiveMarkdownTableLineRole = "body" | "delimiter" | "header";

export interface LiveMarkdownTableCell {
  from: number;
  to: number;
  source: string;
}

export interface LiveMarkdownTableRow {
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  cells: LiveMarkdownTableCell[];
}

export interface LiveMarkdownTable {
  from: number;
  to: number;
  lineNumbers: number[];
  columnCount: number;
  alignments: Array<LiveMarkdownTableAlignment | undefined>;
  header: LiveMarkdownTableRow;
  delimiter: LiveMarkdownTableRow;
  rows: LiveMarkdownTableRow[];
}

export interface LiveMarkdownTableLine {
  table: LiveMarkdownTable;
  role: LiveMarkdownTableLineRole;
}

export function parseLiveMarkdownTables(
  value: string,
  lines: readonly LiveMarkdownBlock[] = parseLiveMarkdownBlocks(value),
): LiveMarkdownTable[] {
  const tables: LiveMarkdownTable[] = [];

  for (let index = 0; index < lines.length - 1; index += 1) {
    const headerLine = lines[index]!;
    const delimiterLine = lines[index + 1]!;
    if (!lineCanStartTable(headerLine) || !lineCanStartTable(delimiterLine)) {
      continue;
    }

    const header = parseTableRow(headerLine);
    const delimiter = parseTableRow(delimiterLine);
    if (
      !header ||
      header.cells.length < 2 ||
      !delimiter ||
      !delimiterLine.source.includes("|") ||
      !delimiter.cells.length ||
      !delimiter.cells.every((cell) => /^:?-{3,}:?$/.test(cell.source))
    ) {
      continue;
    }

    const rows: LiveMarkdownTableRow[] = [];
    let cursor = index + 2;
    while (cursor < lines.length && lineCanBelongToTable(lines[cursor]!)) {
      const row = parseTableRow(lines[cursor]!);
      if (!row || row.cells.length < 2) {
        break;
      }

      rows.push(row);
      cursor += 1;
    }

    const tableRows = [header, delimiter, ...rows];
    tables.push({
      from: header.from,
      to: tableRows.at(-1)!.end,
      lineNumbers: tableRows.map((row) => row.lineNumber),
      columnCount: header.cells.length,
      alignments: delimiter.cells.map((cell) => tableAlignment(cell.source)),
      header,
      delimiter,
      rows,
    });

    index = cursor - 1;
  }

  return tables;
}

export function indexLiveMarkdownTableLines(
  tables: readonly LiveMarkdownTable[],
): Map<number, LiveMarkdownTableLine> {
  const lines = new Map<number, LiveMarkdownTableLine>();

  for (const table of tables) {
    lines.set(table.header.lineNumber, { table, role: "header" });
    lines.set(table.delimiter.lineNumber, { table, role: "delimiter" });
    for (const row of table.rows) {
      lines.set(row.lineNumber, { table, role: "body" });
    }
  }

  return lines;
}

function lineCanBelongToTable(line: LiveMarkdownBlock): boolean {
  return line.type !== "code" && line.type !== "frontmatter";
}

function lineCanStartTable(line: LiveMarkdownBlock): boolean {
  return line.type === "text";
}

function parseTableRow(
  line: LiveMarkdownBlock,
): LiveMarkdownTableRow | undefined {
  const bounds = trimmedBounds(line.source);
  if (!bounds || !line.source.includes("|")) {
    return undefined;
  }

  let { from, to } = bounds;
  if (line.source[from] === "|") {
    from += 1;
  }
  if (to > from && line.source[to - 1] === "|" && line.source[to - 2] !== "\\") {
    to -= 1;
  }

  const cellRanges: Array<{ from: number; to: number }> = [];
  let cellFrom = from;
  let codeDelimiterLength = 0;

  for (let index = from; index < to; index += 1) {
    const character = line.source[index]!;
    if (character === "\\" && line.source[index + 1] === "|") {
      index += 1;

      continue;
    }
    if (character === "`") {
      let runLength = 1;
      while (line.source[index + runLength] === "`") {
        runLength += 1;
      }
      if (!codeDelimiterLength) {
        codeDelimiterLength = runLength;
      } else if (codeDelimiterLength === runLength) {
        codeDelimiterLength = 0;
      }
      index += runLength - 1;

      continue;
    }
    if (character === "|" && !codeDelimiterLength) {
      cellRanges.push({ from: cellFrom, to: index });
      cellFrom = index + 1;
    }
  }
  cellRanges.push({ from: cellFrom, to });

  const cells = cellRanges.map((range) => {
    const trimmed = trimCellRange(line.source, range);
    const absoluteFrom = line.from + trimmed.from;
    const absoluteTo = line.from + trimmed.to;

    return {
      from: absoluteFrom,
      to: absoluteTo,
      source: line.source.slice(trimmed.from, trimmed.to),
    };
  });

  return {
    lineNumber: line.lineNumber,
    from: line.from,
    to: line.to,
    end: line.end,
    cells,
  };
}

function trimmedBounds(
  source: string,
): { from: number; to: number } | undefined {
  const from = source.search(/\S/);
  if (from < 0) {
    return undefined;
  }

  let to = source.length;
  while (to > from && /\s/.test(source[to - 1]!)) {
    to -= 1;
  }

  return { from, to };
}

function trimCellRange(
  source: string,
  range: { from: number; to: number },
): { from: number; to: number } {
  let { from, to } = range;
  while (from < to && /\s/.test(source[from]!)) {
    from += 1;
  }
  while (to > from && /\s/.test(source[to - 1]!)) {
    to -= 1;
  }

  return { from, to };
}

function tableAlignment(
  source: string,
): LiveMarkdownTableAlignment | undefined {
  if (source.startsWith(":") && source.endsWith(":")) {
    return "center";
  }
  if (source.endsWith(":")) {
    return "right";
  }
  if (source.startsWith(":")) {
    return "left";
  }

  return undefined;
}
