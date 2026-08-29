export interface ParsedInlineMarkdownLink {
  destination: string;
  end: number;
  label: string;
  raw: string;
  start: number;
  title?: string;
}

/** Parse the shared inline-link grammar used by Markdown links and images. */
export function parseInlineMarkdownLinkAt(
  source: string,
  start: number,
  image = false,
): ParsedInlineMarkdownLink | undefined {
  const opening = image ? "![" : "[";
  if (!source.startsWith(opening, start)) {
    return undefined;
  }

  const labelStart = start + opening.length;
  const labelEnd = findClosingLabel(source, labelStart);
  if (labelEnd < 0 || source[labelEnd + 1] !== "(") {
    return undefined;
  }

  const destinationEnd = findClosingDestination(source, labelEnd + 2);
  if (destinationEnd < 0) {
    return undefined;
  }

  const destinationParts = parseDestination(
    source.slice(labelEnd + 2, destinationEnd).trim(),
  );
  if (!destinationParts) {
    return undefined;
  }

  return {
    destination: destinationParts.destination,
    end: destinationEnd,
    label: unescapeMarkdownPunctuation(source.slice(labelStart, labelEnd)),
    raw: source.slice(start, destinationEnd + 1),
    start,
    ...(destinationParts.title ? { title: destinationParts.title } : {}),
  };
}

function findClosingLabel(source: string, start: number): number {
  let depth = 1;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\n" || character === "\r") {
      return -1;
    }
    if (character === "\\") {
      index += 1;
    } else if (character === "[") {
      depth += 1;
    } else if (character === "]") {
      depth -= 1;
      if (!depth) {
        return index;
      }
    }
  }

  return -1;
}

function findClosingDestination(source: string, start: number): number {
  let depth = 1;
  let angleDestination = source[start] === "<";
  let quote: "\"" | "'" | undefined;

  for (let index = start; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\n" || character === "\r") {
      return -1;
    }
    if (character === "\\") {
      index += 1;
      continue;
    }
    if (angleDestination) {
      if (character === ">") {
        angleDestination = false;
      }
      continue;
    }
    if (quote) {
      if (character === quote) {
        quote = undefined;
      }
      continue;
    }
    const previousCharacter = source[index - 1];
    const followsTitleSeparator = index > start
      && (previousCharacter === " " || previousCharacter === "\t");
    if (
      (character === "\"" || character === "'")
      && depth === 1
      && followsTitleSeparator
    ) {
      quote = character;
    } else if (character === "(") {
      depth += 1;
    } else if (character === ")") {
      depth -= 1;
      if (!depth) {
        return index;
      }
    }
  }

  return -1;
}

function parseDestination(
  raw: string,
): { destination: string; title?: string } | undefined {
  const match = raw.match(
    /^(<(?:\\.|[^>\\])+>|\S+?)(?:\s+(?:"((?:\\.|[^"\\])*)"|'((?:\\.|[^'\\])*)'|\(((?:\\.|[^)\\])*)\)))?$/,
  );
  if (!match) {
    return undefined;
  }

  const destination = match[1]!.replace(/^<|>$/g, "");
  const title = match[2] ?? match[3] ?? match[4];

  return {
    destination,
    ...(title === undefined ? {} : { title: unescapeMarkdownPunctuation(title) }),
  };
}

function unescapeMarkdownPunctuation(value: string): string {
  return value.replace(
    /\\([!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~])/g,
    "$1",
  );
}
