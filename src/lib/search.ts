import type { Note, SearchResult, SearchScope } from "../types";

export type FolderNameLookup =
  | ReadonlyMap<string, string>
  | Readonly<Record<string, string>>
  | ((folderId: string | null) => string | undefined);

export interface SearchOptions {
  folderNames?: FolderNameLookup;
  limit?: number;
  snippetLength?: number;
  scope?: SearchScope;
  exactTag?: string;
}

interface ScoredField {
  score: number;
  matchedTerms: Set<string>;
}

/**
 * A small, deterministic full-text search tuned for a local notes collection.
 * Titles carry the most weight, followed by tags, folders and body text.
 */
export function searchNotes(
  notes: readonly Note[],
  query: string,
  optionsOrFolders: SearchOptions | FolderNameLookup | number = {},
  explicitLimit?: number,
): SearchResult[] {
  const options = toOptions(optionsOrFolders, explicitLimit);
  const terms = parseSearchTerms(query);
  const phrase = terms.join(" ");
  if (!phrase || terms.length === 0) {
    return [];
  }

  const results: Array<SearchResult & { updatedAt: number }> = [];
  const exactTag = normalizeSearchText(options.exactTag ?? "");

  for (const note of notes) {
    const title = normalizeSearchText(note.title);
    const content = normalizeSearchText(note.content);
    const tags = note.tags.map(normalizeSearchText);
    const folderLabel = normalizeSearchText(
      folderName(note.folderId, options.folderNames) ?? "",
    );
    if (exactTag && !tags.includes(exactTag)) {
      continue;
    }

    const titleScore = scoreTitle(title, phrase, terms);
    const contentScore = scoreContent(content, phrase, terms);
    const tagScore = scoreTags(tags, phrase, terms);
    const folderScore = scoreFolder(folderLabel, phrase, terms);
    const allFields: Array<[SearchResult["reason"], ScoredField]> = [
      ["title", titleScore],
      ["tag", tagScore],
      ["folder", folderScore],
      ["content", contentScore],
    ];
    const scopedReason = options.scope === "titles"
      ? "title"
      : options.scope === "tags"
        ? "tag"
        : options.scope;
    const fields = options.scope === "all"
      ? allFields
      : allFields.filter(([reason]) => reason === scopedReason);
    const matched = new Set(fields.flatMap(([, field]) => [...field.matchedTerms]));

    // Multi-word searches are intentionally conjunctive across the selected
    // fields. This keeps "meeting api" useful without a heavyweight index.
    if (terms.some((term) => !matched.has(term))) {
      continue;
    }

    const [reason] = fields.reduce((best, current) =>
      current[1].score > best[1].score ? current : best,
    );
    const score = fields.reduce((total, [, field]) => total + field.score, 0);

    if (score <= 0) {
      continue;
    }

    results.push({
      note,
      score: Math.round(score * 100) / 100,
      snippet: createSearchSnippet(
        note.content,
        [query, ...terms],
        options.snippetLength,
      ),
      reason,
      updatedAt: note.updatedAt,
    });
  }

  return results
    .sort(
      (a, b) =>
        b.score - a.score ||
        b.updatedAt - a.updatedAt ||
        a.note.title.localeCompare(b.note.title),
    )
    .slice(0, options.limit)
    .map(({ updatedAt: _updatedAt, ...result }) => result);
}

/** Build a plain-text, context-aware excerpt around the earliest query hit. */
export function createSearchSnippet(
  markdown: string,
  queryOrTerms: string | readonly string[],
  maxLength = 170,
): string {
  const safeMaxLength = Math.max(40, maxLength);
  const text = markdownToSearchText(markdown);
  if (!text) {
    return "";
  }

  const terms = (typeof queryOrTerms === "string"
    ? parseSearchTerms(queryOrTerms)
    : queryOrTerms.flatMap(parseSearchTerms)
  )
    .map(normalizeSearchText)
    .filter(Boolean)
    .sort((a, b) => b.length - a.length);
  const comparableText = normalizeSearchText(text);

  let hitIndex = -1;
  let hitLength = 0;
  for (const term of terms) {
    const index = comparableText.indexOf(term);
    if (index >= 0 && (hitIndex < 0 || index < hitIndex)) {
      hitIndex = index;
      hitLength = term.length;
    }
  }

  if (text.length <= safeMaxLength) {
    return text;
  }
  if (hitIndex < 0) {
    return `${trimAtWord(text, safeMaxLength)}…`;
  }

  const desiredStart = Math.max(0, hitIndex - Math.floor((safeMaxLength - hitLength) / 2));
  let start = desiredStart;
  if (start > 0) {
    const nextSpace = text.indexOf(" ", start);
    if (nextSpace >= 0 && nextSpace - start < 24) {
      start = nextSpace + 1;
    }
  }

  let excerpt = text.slice(start, start + safeMaxLength);
  if (start + safeMaxLength < text.length) {
    excerpt = trimAtWord(excerpt, excerpt.length);
  }

  return `${start > 0 ? "…" : ""}${excerpt.trim()}${
    start + excerpt.length < text.length ? "…" : ""
  }`;
}

export function parseSearchTerms(query: string): string[] {
  const terms: string[] = [];
  const seen = new Set<string>();
  const tokenPattern = /"([^"]+)"|'([^']+)'|([^\s]+)/g;
  let match: RegExpExecArray | null;

  while ((match = tokenPattern.exec(query)) !== null) {
    const term = normalizeSearchText(match[1] ?? match[2] ?? match[3] ?? "");
    if (!term || seen.has(term)) {
      continue;
    }
    seen.add(term);
    terms.push(term);
  }

  return terms;
}

function scoreTitle(title: string, phrase: string, terms: readonly string[]): ScoredField {
  const result = scoreFieldMatches(title, terms, 34, 22);
  if (title === phrase) {
    result.score += 180;
  } else if (title.startsWith(phrase)) {
    result.score += 95;
  } else if (title.includes(phrase)) {
    result.score += 58;
  }

  return result;
}

function scoreContent(content: string, phrase: string, terms: readonly string[]): ScoredField {
  const result = scoreFieldMatches(content, terms, 5, 3);
  const phraseIndex = content.indexOf(phrase);
  if (phraseIndex >= 0) {
    result.score += 18;
  }

  for (const term of terms) {
    const occurrences = countOccurrences(content, term, 8);
    result.score += Math.min(occurrences, 8) * 1.25;
  }

  return result;
}

function scoreTags(tags: readonly string[], phrase: string, terms: readonly string[]): ScoredField {
  const matchedTerms = new Set<string>();
  let score = 0;

  for (const tag of tags) {
    if (tag === phrase) {
      score += 55;
    } else if (tag.includes(phrase)) {
      score += 24;
    }

    for (const term of terms) {
      if (tag === term) {
        score += 35;
        matchedTerms.add(term);
      } else if (tag.includes(term)) {
        score += 16;
        matchedTerms.add(term);
      }
    }
  }

  return { score, matchedTerms };
}

function scoreFolder(folder: string, phrase: string, terms: readonly string[]): ScoredField {
  const result = scoreFieldMatches(folder, terms, 14, 8);
  if (folder === phrase) {
    result.score += 30;
  } else if (folder.includes(phrase)) {
    result.score += 15;
  }

  return result;
}

function scoreFieldMatches(
  field: string,
  terms: readonly string[],
  wholeWordWeight: number,
  partialWeight: number,
): ScoredField {
  const matchedTerms = new Set<string>();
  let score = 0;

  for (const term of terms) {
    const index = field.indexOf(term);
    if (index < 0) {
      continue;
    }
    matchedTerms.add(term);
    score += hasWordAt(field, term, index) ? wholeWordWeight : partialWeight;
    if (index === 0) {
      score += wholeWordWeight * 0.25;
    }
  }

  return { score, matchedTerms };
}

function hasWordAt(field: string, term: string, index: number): boolean {
  const before = index === 0 ? "" : field[index - 1]!;
  const after = field[index + term.length] ?? "";

  return (!before || /[^\p{L}\p{N}_]/u.test(before)) &&
    (!after || /[^\p{L}\p{N}_]/u.test(after));
}

function countOccurrences(value: string, needle: string, limit: number): number {
  let count = 0;
  let cursor = 0;
  while (count < limit) {
    const index = value.indexOf(needle, cursor);
    if (index < 0) {
      break;
    }
    count += 1;
    cursor = index + Math.max(1, needle.length);
  }

  return count;
}

function folderName(
  folderId: string | null,
  lookup?: FolderNameLookup,
): string | undefined {
  if (!folderId || !lookup) {
    return undefined;
  }
  if (typeof lookup === "function") {
    return lookup(folderId);
  }
  if ("get" in lookup && typeof lookup.get === "function") {
    return lookup.get(folderId);
  }

  return (lookup as Readonly<Record<string, string>>)[folderId];
}

function toOptions(
  value: SearchOptions | FolderNameLookup | number,
  explicitLimit?: number,
): Required<Pick<SearchOptions, "limit" | "snippetLength">> &
  Required<Pick<SearchOptions, "scope">> &
  Pick<SearchOptions, "folderNames" | "exactTag"> {
  if (typeof value === "number") {
    return {
      folderNames: undefined,
      limit: Math.max(1, value),
      snippetLength: 170,
      scope: "all",
      exactTag: undefined,
    };
  }

  const looksLikeOptions =
    typeof value === "object" &&
    value !== null &&
    !(value instanceof Map) &&
    (
      "folderNames" in value
      || "limit" in value
      || "snippetLength" in value
      || "scope" in value
      || "exactTag" in value
    );
  const options = looksLikeOptions ? value as SearchOptions : { folderNames: value as FolderNameLookup };

  return {
    folderNames: options.folderNames,
    limit: Math.max(1, explicitLimit ?? options.limit ?? 50),
    snippetLength: Math.max(40, options.snippetLength ?? 170),
    scope: options.scope ?? "all",
    exactTag: options.exactTag,
  };
}

function markdownToSearchText(markdown: string): string {
  return markdown
    .replace(/^ {0,3}(`{3,}|~{3,})[^\n]*\n[\s\S]*?^ {0,3}\1\s*$/gm, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!?(?:\[\[)([^\]|#]+)(?:#[^\]|]+)?(?:\|([^\]]+))?\]\]/g, "$2 $1")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}(?:#{1,6}|>|[-+*]|\d+[.)])\s+/gm, "")
    .replace(/[>*_~#]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function trimAtWord(value: string, maxLength: number): string {
  const sliced = value.slice(0, maxLength);
  const lastSpace = sliced.lastIndexOf(" ");

  return lastSpace > maxLength * 0.65 ? sliced.slice(0, lastSpace) : sliced;
}

function normalizeSearchText(value: string): string {
  return value
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLocaleLowerCase()
    .replace(/\s+/g, " ")
    .trim();
}
