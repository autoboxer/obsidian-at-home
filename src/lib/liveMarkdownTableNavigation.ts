import { parseLiveMarkdownTables } from "./liveMarkdownTable";
import type {
  LiveMarkdownTable,
  LiveMarkdownTableRow,
} from "./liveMarkdownTable";

export type LiveMarkdownTableNavigation =
  | "down-row"
  | "next-cell"
  | "previous-cell"
  | "up-row";

export interface LiveMarkdownTableEdit {
  value: string;
  selectionStart: number;
  selectionEnd: number;
}

export type LiveMarkdownTableCellBoundary = "end" | "start";
export type LiveMarkdownTableHorizontalDirection = "left" | "right";

export interface LiveMarkdownTableCursorTarget {
  assoc: -1 | 1;
  position: number;
}

type TableCellLocation =
  | { columnIndex: number; role: "delimiter" }
  | { columnIndex: number; rowIndex: number; role: "editable" };

export function navigateLiveMarkdownTable(
  value: string,
  tables: readonly LiveMarkdownTable[],
  position: number,
  navigation: LiveMarkdownTableNavigation,
): LiveMarkdownTableEdit | undefined {
  const table = tableContainingPosition(tables, position);
  if (!table) {
    return undefined;
  }

  const location = locateTableCell(table, position);
  if (!location) {
    return undefined;
  }

  if (navigation === "down-row" || navigation === "up-row") {
    return moveVertically(
      value,
      table,
      location,
      position,
      navigation === "down-row",
    );
  }

  const target = adjacentCellTarget(
    table,
    location,
    navigation === "previous-cell",
  );

  return moveToTableCell(value, table, target.rowIndex, target.columnIndex);
}

export function insertLiveMarkdownTableRow(
  value: string,
  tables: readonly LiveMarkdownTable[],
  position: number,
): LiveMarkdownTableEdit | undefined {
  const table = tableContainingPosition(tables, position);
  if (!table) {
    return undefined;
  }

  const location = locateTableCell(table, position);
  if (!location) {
    return undefined;
  }

  const rowIndex = location.role === "delimiter" || location.rowIndex === 0
    ? 0
    : location.rowIndex;
  const nextValue = insertEmptyTableRow(value, table, rowIndex);
  const nextTable = reparsedTable(nextValue, table);
  const targetCell = nextTable?.rows[rowIndex]?.cells[0];
  if (!targetCell) {
    return undefined;
  }

  return {
    value: nextValue,
    selectionStart: targetCell.from,
    selectionEnd: targetCell.from,
  };
}

export function insertLiveMarkdownTableLineBreak(
  value: string,
  tables: readonly LiveMarkdownTable[],
  anchor: number,
  head: number,
): LiveMarkdownTableEdit | undefined {
  const table = tableContainingPosition(tables, head);
  if (!table) {
    return undefined;
  }

  const location = locateTableCell(table, head);
  if (!location || location.role !== "editable") {
    return undefined;
  }

  const rows = [table.header, ...table.rows];
  const cell = rows[location.rowIndex]?.cells[location.columnIndex];
  if (!cell) {
    return undefined;
  }

  const anchorLocation = locateTableCell(table, anchor);
  const selectionStaysInCell = anchorLocation
    && sameCellLocation(anchorLocation, location);
  const selectionStart = selectionStaysInCell
    ? clampToCell(Math.min(anchor, head), cell)
    : clampToCell(head, cell);
  const selectionEnd = selectionStaysInCell
    ? clampToCell(Math.max(anchor, head), cell)
    : selectionStart;
  const lineBreak = "<br>";
  const nextValue = `${value.slice(0, selectionStart)}${lineBreak}${
    value.slice(selectionEnd)
  }`;
  const cursor = selectionStart + lineBreak.length;

  return {
    value: nextValue,
    selectionStart: cursor,
    selectionEnd: cursor,
  };
}

export function isLiveMarkdownTableCellBoundary(
  tables: readonly LiveMarkdownTable[],
  position: number,
  boundary: LiveMarkdownTableCellBoundary,
): boolean {
  return tables.some((table) =>
    [table.header, ...table.rows].some((row) =>
      row.cells.slice(0, table.columnCount).some((cell) =>
        boundary === "start"
          ? position === cell.editableFrom
          : position === cell.to || position === cell.editableTo
      )
    )
  );
}

export function moveAcrossLiveMarkdownTableCellBoundary(
  value: string,
  tables: readonly LiveMarkdownTable[],
  position: number,
  direction: LiveMarkdownTableHorizontalDirection,
): LiveMarkdownTableCursorTarget | undefined {
  const table = tableContainingPosition(tables, position);
  if (!table) {
    return undefined;
  }

  const location = locateTableCell(table, position);
  if (!location || location.role !== "editable") {
    return undefined;
  }

  const row = [table.header, ...table.rows][location.rowIndex];
  const cell = row?.cells[location.columnIndex];
  if (!row || !cell) {
    return undefined;
  }

  if (direction === "right") {
    const paddingFrom = trailingTableCellPaddingFrom(value, cell);
    if (
      position !== cell.editableTo &&
      (paddingFrom === undefined || position !== paddingFrom)
    ) {
      return undefined;
    }

    const nextCell = row.cells[location.columnIndex + 1];

    return nextCell
      ? { assoc: 1, position: nextCell.from }
      : {
        assoc: -1,
        position: paddingFrom ?? cell.to,
      };
  }

  if (position !== cell.from && position !== cell.editableFrom) {
    return undefined;
  }

  const previousCell = row.cells[location.columnIndex - 1];
  if (!previousCell) {
    return { assoc: 1, position: cell.from };
  }

  return {
    assoc: -1,
    position: trailingTableCellPaddingFrom(value, previousCell) ??
      previousCell.editableTo,
  };
}

function tableContainingPosition(
  tables: readonly LiveMarkdownTable[],
  position: number,
): LiveMarkdownTable | undefined {
  return tables.find((table) => {
    const lastRow = table.rows.at(-1) ?? table.delimiter;

    return position >= table.from && position <= lastRow.to;
  });
}

function moveVertically(
  value: string,
  table: LiveMarkdownTable,
  location: TableCellLocation,
  position: number,
  down: boolean,
): LiveMarkdownTableEdit | undefined {
  if (location.role === "delimiter") {
    if (!down) {
      return moveToTableCell(value, table, 0, location.columnIndex, 0);
    }

    return table.rows.length
      ? moveToTableCell(value, table, 1, location.columnIndex, 0)
      : moveBelowTable(value, table);
  }

  if (!down && location.rowIndex === 0) {
    return undefined;
  }
  if (down && location.rowIndex === table.rows.length) {
    return moveBelowTable(value, table);
  }

  const currentCell = tableCellAtLocation(table, location);
  if (!currentCell) {
    return undefined;
  }

  const targetRowIndex = location.rowIndex + (down ? 1 : -1);
  const offset = Math.max(0, position - currentCell.editableFrom);

  return moveToTableCell(
    value,
    table,
    targetRowIndex,
    location.columnIndex,
    offset,
  );
}

function tableCellAtLocation(
  table: LiveMarkdownTable,
  location: TableCellLocation,
): LiveMarkdownTableRow["cells"][number] | undefined {
  if (location.role === "delimiter") {
    return table.delimiter.cells[location.columnIndex];
  }

  return [table.header, ...table.rows][location.rowIndex]
    ?.cells[location.columnIndex];
}

function locateTableCell(
  table: LiveMarkdownTable,
  position: number,
): TableCellLocation | undefined {
  if (position >= table.delimiter.from && position <= table.delimiter.to) {
    return {
      role: "delimiter",
      columnIndex: cellIndexAtPosition(
        table.delimiter,
        position,
        table.columnCount,
      ),
    };
  }

  const editableRows = [table.header, ...table.rows];
  const rowIndex = editableRows.findIndex((row) =>
    position >= row.from && position <= row.to
  );
  if (rowIndex < 0) {
    return undefined;
  }

  return {
    role: "editable",
    rowIndex,
    columnIndex: cellIndexAtPosition(
      editableRows[rowIndex]!,
      position,
      table.columnCount,
    ),
  };
}

function cellIndexAtPosition(
  row: LiveMarkdownTableRow,
  position: number,
  columnCount: number,
): number {
  const cellIndex = row.cells.findIndex((cell) => position <= cell.editableTo);
  const nearestIndex = cellIndex < 0 ? row.cells.length - 1 : cellIndex;

  return Math.max(0, Math.min(nearestIndex, columnCount - 1));
}

function sameCellLocation(
  left: TableCellLocation,
  right: TableCellLocation,
): boolean {
  if (left.role !== right.role || left.columnIndex !== right.columnIndex) {
    return false;
  }

  return left.role === "delimiter" || (
    right.role === "editable" && left.rowIndex === right.rowIndex
  );
}

function adjacentCellTarget(
  table: LiveMarkdownTable,
  location: TableCellLocation,
  previous: boolean,
): { rowIndex: number; columnIndex: number } {
  if (location.role === "delimiter") {
    return previous
      ? { rowIndex: 0, columnIndex: location.columnIndex }
      : { rowIndex: 1, columnIndex: location.columnIndex };
  }

  const flatIndex = location.rowIndex * table.columnCount +
    location.columnIndex +
    (previous ? -1 : 1);
  const targetIndex = Math.max(0, flatIndex);

  return {
    rowIndex: Math.floor(targetIndex / table.columnCount),
    columnIndex: targetIndex % table.columnCount,
  };
}

function moveToTableCell(
  value: string,
  table: LiveMarkdownTable,
  rowIndex: number,
  columnIndex: number,
  selectionOffset?: number,
): LiveMarkdownTableEdit | undefined {
  let nextValue = value;
  let nextTable = table;
  let editableRows = [nextTable.header, ...nextTable.rows];

  if (rowIndex >= editableRows.length) {
    nextValue = appendEmptyTableRow(nextValue, nextTable);
    const reparsed = reparsedTable(nextValue, table);
    if (!reparsed) {
      return undefined;
    }
    nextTable = reparsed;
    editableRows = [nextTable.header, ...nextTable.rows];
  }

  let targetRow = editableRows[rowIndex];
  if (!targetRow) {
    return undefined;
  }
  if (!targetRow.cells[columnIndex]) {
    nextValue = expandTableRow(nextValue, nextTable, targetRow);
    const reparsed = reparsedTable(nextValue, table);
    if (!reparsed) {
      return undefined;
    }
    nextTable = reparsed;
    targetRow = [nextTable.header, ...nextTable.rows][rowIndex];
  }

  const targetCell = targetRow?.cells[columnIndex];
  if (!targetCell) {
    return undefined;
  }

  const selectionStart = selectionOffset === undefined
    ? targetCell.from
    : Math.min(
      targetCell.editableFrom + selectionOffset,
      targetCell.editableTo,
    );
  const selectionEnd = selectionOffset === undefined
    ? targetCell.to
    : selectionStart;

  return {
    value: nextValue,
    selectionStart,
    selectionEnd,
  };
}

function moveBelowTable(
  value: string,
  table: LiveMarkdownTable,
): LiveMarkdownTableEdit {
  const lastRow = table.rows.at(-1) ?? table.delimiter;
  if (lastRow.end > lastRow.to) {
    return {
      value,
      selectionStart: lastRow.end,
      selectionEnd: lastRow.end,
    };
  }

  const nextValue = `${value}${preferredLineEnding(value, table)}`;

  return {
    value: nextValue,
    selectionStart: nextValue.length,
    selectionEnd: nextValue.length,
  };
}

function insertEmptyTableRow(
  value: string,
  table: LiveMarkdownTable,
  rowIndex: number,
): string {
  const nextRow = table.rows[rowIndex];
  if (!nextRow) {
    return appendEmptyTableRow(value, table);
  }

  const row = emptyTableRow(value, table);
  const lineEnding = preferredLineEnding(value, table);

  return `${value.slice(0, nextRow.from)}${row}${lineEnding}${
    value.slice(nextRow.from)
  }`;
}

function appendEmptyTableRow(
  value: string,
  table: LiveMarkdownTable,
): string {
  const lastRow = table.rows.at(-1) ?? table.delimiter;
  const row = emptyTableRow(value, table);
  const existingLineEnding = value.slice(lastRow.to, lastRow.end);
  if (existingLineEnding) {
    return `${value.slice(0, lastRow.end)}${row}${existingLineEnding}${
      value.slice(lastRow.end)
    }`;
  }

  return `${value.slice(0, lastRow.to)}${preferredLineEnding(value, table)}${
    row
  }${value.slice(lastRow.to)}`;
}

function emptyTableRow(
  value: string,
  table: LiveMarkdownTable,
): string {
  return formatTableRow(
    Array.from({ length: table.columnCount }, () => ""),
    value.slice(table.header.from, table.header.to),
  );
}

function expandTableRow(
  value: string,
  table: LiveMarkdownTable,
  row: LiveMarkdownTableRow,
): string {
  const cells = Array.from(
    { length: table.columnCount },
    (_, index) => row.cells[index]?.source ?? "",
  );
  const source = value.slice(row.from, row.to);
  const replacement = formatTableRow(cells, source);

  return `${value.slice(0, row.from)}${replacement}${value.slice(row.to)}`;
}

function formatTableRow(cells: readonly string[], example: string): string {
  const indentation = example.match(/^\s*/)?.[0] ?? "";
  const trimmed = example.trim();
  const leadingPipe = trimmed.startsWith("|") || !cells[0];
  const trailingPipe = (
    trimmed.endsWith("|") && trimmed.at(-2) !== "\\"
  ) || !cells.at(-1);
  const content = cells.join(" | ");

  return `${indentation}${leadingPipe ? "| " : ""}${content}${
    trailingPipe ? " |" : ""
  }`;
}

function preferredLineEnding(
  value: string,
  table: LiveMarkdownTable,
): string {
  for (const row of [table.header, table.delimiter, ...table.rows]) {
    const lineEnding = value.slice(row.to, row.end);
    if (lineEnding) {
      return lineEnding;
    }
  }

  if (value.includes("\r\n")) {
    return "\r\n";
  }
  if (value.includes("\r")) {
    return "\r";
  }

  return "\n";
}

function reparsedTable(
  value: string,
  previous: LiveMarkdownTable,
): LiveMarkdownTable | undefined {
  return parseLiveMarkdownTables(value).find((table) =>
    table.header.from === previous.header.from
  );
}

function clampToCell(
  position: number,
  cell: LiveMarkdownTableRow["cells"][number],
): number {
  return Math.max(
    cell.editableFrom,
    Math.min(position, cell.editableTo),
  );
}

function trailingTableCellPaddingFrom(
  value: string,
  cell: LiveMarkdownTableRow["cells"][number],
): number | undefined {
  if (
    cell.to >= cell.editableTo ||
    !/\s/.test(value[cell.editableTo - 1] ?? "")
  ) {
    return undefined;
  }

  return cell.editableTo - 1;
}
