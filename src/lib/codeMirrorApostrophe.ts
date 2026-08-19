import {
  StateEffect,
  StateField,
  Transaction,
} from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { EditorSelection, Extension, Text } from "@codemirror/state";
import type { Command } from "@codemirror/view";

const STRAIGHT_APOSTROPHE = "'";
const SMART_APOSTROPHES = new Set(["‘", "’"]);

const rememberLiteralApostrophes = StateEffect.define<readonly number[]>();

const literalApostropheState = StateField.define<readonly number[]>({
  create: () => [],
  update(positions, transaction) {
    const mapped = positions
      .map((position) => transaction.changes.mapPos(position, -1))
      .filter((position) => isStraightApostropheAt(transaction.newDoc, position));
    const added = transaction.effects
      .filter((effect) => effect.is(rememberLiteralApostrophes))
      .flatMap((effect) => effect.value)
      .filter((position) => isStraightApostropheAt(transaction.newDoc, position));

    return [...new Set([...mapped, ...added])]
      .sort((first, second) => first - second)
      .filter((position) => literalApostropheIsPending(
        transaction.newDoc,
        transaction.newSelection,
        position,
      ));
  },
});

export const literalApostropheExtension: Extension = [
  literalApostropheState,
  EditorView.inputHandler.of(preserveLiteralApostrophes),
];

export const insertLiteralApostrophe: Command = (view) => {
  if (view.composing) {
    return false;
  }

  const transaction = view.state.update(
    view.state.replaceSelection(STRAIGHT_APOSTROPHE),
  );
  const positions = transaction.newSelection.ranges
    .map((range) => range.head - STRAIGHT_APOSTROPHE.length)
    .filter((position) => isStraightApostropheAt(transaction.newDoc, position));

  view.dispatch({
    changes: transaction.changes,
    selection: transaction.newSelection,
    effects: rememberLiteralApostrophes.of(positions),
    scrollIntoView: true,
    userEvent: "input.type",
  });

  return true;
};

function preserveLiteralApostrophes(
  view: EditorView,
  from: number,
  to: number,
  text: string,
  insert: () => Transaction,
): boolean {
  if (view.composing) {
    return false;
  }

  const protectedPositions = view.state
    .field(literalApostropheState)
    .filter((position) => position >= from && position < to);
  if (!protectedPositions.length) {
    return false;
  }

  const corrected = text.split("");
  let changed = false;

  for (const position of protectedPositions) {
    const offset = position - from;
    if (
      view.state.sliceDoc(position, position + 1) === STRAIGHT_APOSTROPHE &&
      SMART_APOSTROPHES.has(corrected[offset] ?? "")
    ) {
      corrected[offset] = STRAIGHT_APOSTROPHE;
      changed = true;
    }
  }
  if (!changed) {
    return false;
  }

  const defaultTransaction = insert();
  const correctedText = corrected.join("");
  const restoresExistingText = correctedText === view.state.sliceDoc(from, to);
  // Smart punctuation may rewrite a completed word behind the actual caret.
  const selection = restoresExistingText
    ? view.state.selection
    : defaultTransaction.newSelection;
  const userEvent =
    defaultTransaction.annotation(Transaction.userEvent) ?? "input.type";
  const addToHistory = defaultTransaction.annotation(Transaction.addToHistory);
  const remote = defaultTransaction.annotation(Transaction.remote);
  const annotations = [
    Transaction.userEvent.of(userEvent),
    ...(addToHistory === undefined
      ? []
      : [Transaction.addToHistory.of(addToHistory)]),
    ...(remote === undefined ? [] : [Transaction.remote.of(remote)]),
  ];

  view.dispatch({
    changes: { from, to, insert: correctedText },
    selection,
    effects: defaultTransaction.effects,
    annotations,
    scrollIntoView: defaultTransaction.scrollIntoView,
  });

  return true;
}

function literalApostropheIsPending(
  document: Text,
  selection: EditorSelection,
  position: number,
): boolean {
  const line = document.lineAt(position);

  return selection.ranges.some((range) => {
    if (!range.empty || range.head <= position || range.head > line.to) {
      return false;
    }

    return /^\S*\s?$/.test(document.sliceString(position + 1, range.head));
  });
}

function isStraightApostropheAt(document: Text, position: number): boolean {
  return document.sliceString(position, position + 1) === STRAIGHT_APOSTROPHE;
}
