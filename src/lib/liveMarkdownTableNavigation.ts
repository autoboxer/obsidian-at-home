import { parseLiveMarkdownTables } from "./liveMarkdownTable";
import type {
  LiveMarkdownTable,
  LiveMarkdownTableRow,
} from "./liveMarkdownTable";

export type LiveMarkdownTableNavigation =
  | "next-cell"
  | "previous-cell";

export interface LiveMarkdownTableEdit {
  value: string;
  selectionStart: number;
  selectionEnd: number;
}

type TableCellLocation =
  | { columnIndex: number; role: "delimiter" }
  | { columnIndex: number; rowIndex: number; role: "editable" };

export function navigateLiveMarkdownTable(
  value: string,
  tables: readonly LiveMarkdownTable[],
  selectionStart: number,
  selectionEnd: number,
  navigation: LiveMarkdownTableNavigation,
): LiveMarkdownTableEdit | undefined {
  const table = tableContainingSelection(tables, selectionStart, selectionEnd);
  if (!table) {
    return undefined;
  }

  const startLocation = locateTableCell(table, selectionStart);
  const selectionLast = selectionEnd > selectionStart
    ? selectionEnd - 1
    : selectionEnd;
  const endLocation = locateTableCell(table, selectionLast);
  if (
    !startLocation ||
    !endLocation ||
    !sameCellLocation(startLocation, endLocation)
  ) {
    return undefined;
  }

  const target = adjacentCellTarget(
    table,
    startLocation,
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

function tableContainingSelection(
  tables: readonly LiveMarkdownTable[],
  selectionStart: number,
  selectionEnd: number,
): LiveMarkdownTable | undefined {
  const from = Math.min(selectionStart, selectionEnd);
  const to = Math.max(selectionStart, selectionEnd);

  return tables.find((table) => {
    const lastRow = table.rows.at(-1) ?? table.delimiter;

    return from >= table.from && to <= lastRow.to;
  });
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
  const cellIndex = row.cells.findIndex((cell) => position <= cell.to);
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

  return {
    value: nextValue,
    selectionStart: targetCell.from,
    selectionEnd: targetCell.to,
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
  return Math.max(cell.from, Math.min(position, cell.to));
}
