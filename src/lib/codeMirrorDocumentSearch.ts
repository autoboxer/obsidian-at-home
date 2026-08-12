import { StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView } from "@codemirror/view";
import type { ChangeDesc, EditorState, Extension } from "@codemirror/state";
import type { DecorationSet } from "@codemirror/view";
import type { DocumentTextMatch } from "./documentSearch";

interface DocumentSearchState {
  activeIndex: number;
  decorations: DecorationSet;
  matches: readonly DocumentTextMatch[];
}

export interface DocumentSearchUpdate {
  activeIndex: number;
  matches: readonly DocumentTextMatch[];
}

export const setDocumentSearchMatches = StateEffect.define<DocumentSearchUpdate>();

const documentSearchState = StateField.define<DocumentSearchState>({
  create: () => createDocumentSearchState([], -1),
  update(value, transaction) {
    let activeIndex = value.activeIndex;
    let matches = transaction.docChanged
      ? mapDocumentSearchMatches(value.matches, transaction.changes)
      : value.matches;

    for (const effect of transaction.effects) {
      if (effect.is(setDocumentSearchMatches)) {
        activeIndex = effect.value.activeIndex;
        matches = effect.value.matches;
      }
    }

    const unchanged = matches === value.matches && activeIndex === value.activeIndex;

    return unchanged
      ? value
      : createDocumentSearchState(matches, activeIndex);
  },
  provide: (field) => EditorView.decorations.from(
    field,
    (value) => value.decorations,
  ),
});

export const codeMirrorDocumentSearchExtension: Extension = documentSearchState;

export function documentSearchMatches(
  state: EditorState,
): readonly DocumentTextMatch[] {
  return state.field(documentSearchState, false)?.matches ?? [];
}

function createDocumentSearchState(
  matches: readonly DocumentTextMatch[],
  activeIndex: number,
): DocumentSearchState {
  return {
    activeIndex,
    decorations: Decoration.set(
      matches.map((match, index) => Decoration.mark({
        class: index === activeIndex
          ? "document-search-match document-search-active"
          : "document-search-match",
      }).range(match.from, match.to)),
      true,
    ),
    matches,
  };
}

function mapDocumentSearchMatches(
  matches: readonly DocumentTextMatch[],
  changes: ChangeDesc,
): DocumentTextMatch[] {
  return matches.flatMap((match) => {
    const from = changes.mapPos(match.from, 1);
    const to = changes.mapPos(match.to, -1);

    return from < to ? [{ from, to }] : [];
  });
}
