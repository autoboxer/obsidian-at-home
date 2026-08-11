export const DOCUMENT_SEARCH_MIN_LENGTH = 2;

export type DocumentSearchDirection = "next" | "previous";

export interface DocumentTextMatch {
  from: number;
  to: number;
}

export function normalizeDocumentSearchQuery(query: string): string | undefined {
  const normalized = query.trim();

  return Array.from(normalized).length >= DOCUMENT_SEARCH_MIN_LENGTH
    ? normalized
    : undefined;
}

export function findDocumentTextMatches(
  text: string,
  query: string,
): DocumentTextMatch[] {
  const normalizedQuery = normalizeDocumentSearchQuery(query);
  if (!normalizedQuery || !text) {
    return [];
  }

  const expression = new RegExp(escapeRegularExpression(normalizedQuery), "giu");

  return Array.from(text.matchAll(expression), (match) => ({
    from: match.index,
    to: match.index + match[0].length,
  }));
}

export function stepDocumentSearchIndex(
  currentIndex: number,
  matchCount: number,
  direction: DocumentSearchDirection,
): number {
  if (matchCount <= 0) {
    return -1;
  }
  if (currentIndex < 0 || currentIndex >= matchCount) {
    return direction === "next" ? 0 : matchCount - 1;
  }

  const step = direction === "next" ? 1 : -1;

  return (currentIndex + step + matchCount) % matchCount;
}

function escapeRegularExpression(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
