export type LiveMarkdownBlockType =
  | "blank"
  | "code"
  | "frontmatter"
  | "heading"
  | "task"
  | "text";

export interface LiveMarkdownRange {
  from: number;
  to: number;
}

export interface LiveMarkdownTask {
  checked: boolean;
  marker: LiveMarkdownRange;
  check: LiveMarkdownRange;
}

export interface LiveMarkdownBlock {
  type: LiveMarkdownBlockType;
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  source: string;
  content: LiveMarkdownRange;
  syntax: LiveMarkdownRange[];
  headingLevel?: number;
  task?: LiveMarkdownTask;
}

interface SourceLine {
  lineNumber: number;
  from: number;
  to: number;
  end: number;
  source: string;
}

interface OpenFence {
  marker: "`" | "~";
  length: number;
}

export function parseLiveMarkdownBlocks(value: string): LiveMarkdownBlock[] {
  const lines = readSourceLines(value);
  const frontmatterEnd = findFrontmatterEnd(lines);
  const blocks: LiveMarkdownBlock[] = [];
  let fence: OpenFence | undefined;

  for (const [index, line] of lines.entries()) {
    if (frontmatterEnd !== undefined && index <= frontmatterEnd) {
      blocks.push(createPlainBlock(line, "frontmatter"));

      continue;
    }

    if (fence) {
      blocks.push(createPlainBlock(line, "code"));
      if (closesFence(line.source, fence)) {
        fence = undefined;
      }

      continue;
    }

    const openingFence = line.source.match(/^ {0,3}(`{3,}|~{3,})/);
    if (openingFence) {
      const marker = openingFence[1]!;
      fence = {
        marker: marker[0]! as "`" | "~",
        length: marker.length,
      };
      blocks.push(createPlainBlock(line, "code"));

      continue;
    }

    if (!line.source.trim()) {
      blocks.push(createPlainBlock(line, "blank"));

      continue;
    }

    const heading = parseHeading(line);
    if (heading) {
      blocks.push(heading);

      continue;
    }

    const task = parseTask(line);
    blocks.push(task ?? createPlainBlock(line, "text"));
  }

  return blocks;
}

export function findLiveMarkdownBlock(
  blocks: readonly LiveMarkdownBlock[],
  offset: number,
): LiveMarkdownBlock | undefined {
  if (!blocks.length) {
    return undefined;
  }

  const position = Math.max(0, offset);
  for (const [index, block] of blocks.entries()) {
    if (position < block.end || index === blocks.length - 1) {
      return block;
    }
  }

  return blocks.at(-1);
}

export function activeLiveMarkdownBlocks(
  blocks: readonly LiveMarkdownBlock[],
  anchor: number,
  head: number,
): LiveMarkdownBlock[] {
  const selectionFrom = Math.min(anchor, head);
  const selectionTo = Math.max(anchor, head);
  const first = findLiveMarkdownBlock(blocks, selectionFrom);
  const last = findLiveMarkdownBlock(
    blocks,
    selectionTo === selectionFrom ? selectionTo : selectionTo - 1,
  );
  if (!first || !last) {
    return [];
  }

  const firstIndex = blocks.indexOf(first);
  const lastIndex = blocks.indexOf(last);

  return blocks.slice(firstIndex, lastIndex + 1);
}

export function setLiveMarkdownTaskChecked(
  value: string,
  block: LiveMarkdownBlock,
  checked: boolean,
): string {
  if (block.type !== "task" || !block.task) {
    return value;
  }

  const { from, to } = block.task.check;
  if (to - from !== 1 || !/[ xX]/.test(value.slice(from, to))) {
    return value;
  }

  return `${value.slice(0, from)}${checked ? "x" : " "}${value.slice(to)}`;
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

function closesFence(source: string, fence: OpenFence): boolean {
  const closing = source.match(/^ {0,3}(`+|~+)\s*$/)?.[1];

  return Boolean(
    closing
    && closing[0] === fence.marker
    && closing.length >= fence.length,
  );
}

function parseHeading(line: SourceLine): LiveMarkdownBlock | undefined {
  const heading = line.source.match(/^( {0,3})(#{1,6})([\t ]+)(.*)$/);
  if (!heading) {
    return undefined;
  }

  const prefixLength = heading[1]!.length + heading[2]!.length + heading[3]!.length;
  const openingMarker = { from: line.from, to: line.from + prefixLength };
  const syntax = [openingMarker];
  const closingMarker = heading[4]!.match(/[\t ]+#+[\t ]*$/)?.[0];
  const contentTo = closingMarker ? line.to - closingMarker.length : line.to;
  if (closingMarker) {
    syntax.push({ from: contentTo, to: line.to });
  }

  return {
    ...line,
    type: "heading",
    content: { from: openingMarker.to, to: contentTo },
    syntax,
    headingLevel: heading[2]!.length,
  };
}

function parseTask(line: SourceLine): LiveMarkdownBlock | undefined {
  const task = line.source.match(
    /^([ \t]*)([-+*]|\d{1,9}[.)])([ \t]+)\[([ xX])\]([ \t]+)(.*)$/,
  );
  if (!task) {
    return undefined;
  }

  const indentLength = task[1]!.length;
  const markerStart = line.from + indentLength;
  const checkFrom = markerStart + task[2]!.length + task[3]!.length + 1;
  const contentFrom = checkFrom + 2 + task[5]!.length;
  const marker = { from: markerStart, to: contentFrom };

  return {
    ...line,
    type: "task",
    content: { from: contentFrom, to: line.to },
    syntax: [marker],
    task: {
      checked: task[4]!.toLocaleLowerCase() === "x",
      marker,
      check: { from: checkFrom, to: checkFrom + 1 },
    },
  };
}

function createPlainBlock(
  line: SourceLine,
  type: "blank" | "code" | "frontmatter" | "text",
): LiveMarkdownBlock {
  return {
    ...line,
    type,
    content: { from: line.from, to: line.to },
    syntax: [],
  };
}
