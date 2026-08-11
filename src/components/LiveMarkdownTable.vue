<script setup lang="ts">
import { computed } from "vue";
import { parseLiveMarkdownInline } from "../lib/liveMarkdownInline";
import type { LiveMarkdownInlineSegment } from "../lib/liveMarkdownInline";
import type {
  LiveMarkdownTable,
  LiveMarkdownTableAlignment,
  LiveMarkdownTableRow,
} from "../lib/liveMarkdownTable";
import LiveMarkdownInline from "./LiveMarkdownInline.vue";

interface RenderedTableCell {
  alignment?: LiveMarkdownTableAlignment;
  segments: LiveMarkdownInlineSegment[];
}

const props = defineProps<{
  table: LiveMarkdownTable;
  height?: number;
  resolveWikiLink?: (target: string, heading?: string) => boolean;
}>();

const blockStyle = computed(() => ({
  ...(props.height ? { "--live-table-height": `${props.height}px` } : {}),
  "--live-table-lines": String(props.table.lineNumbers.length),
}));
const headerCells = computed(() => renderRow(props.table.header));
const bodyRows = computed(() => props.table.rows.map(renderRow));

function renderRow(row: LiveMarkdownTableRow): RenderedTableCell[] {
  return Array.from({ length: props.table.columnCount }, (_, index) => ({
    ...(props.table.alignments[index]
      ? { alignment: props.table.alignments[index] }
      : {}),
    segments: parseLiveMarkdownInline(row.cells[index]?.source ?? "", {
      ...(props.resolveWikiLink
        ? { resolveWikiLink: props.resolveWikiLink }
        : {}),
    }),
  }));
}

function alignmentClass(
  alignment: LiveMarkdownTableAlignment | undefined,
): string | undefined {
  return alignment ? `align-${alignment}` : undefined;
}
</script>

<template>
  <div
    class="live-table-block"
    :style="blockStyle"
  >
    <table role="presentation">
      <thead>
        <tr>
          <th
            v-for="(cell, index) in headerCells"
            :key="index"
            :class="alignmentClass(cell.alignment)"
          >
            <LiveMarkdownInline :segments="cell.segments" />
          </th>
        </tr>
      </thead>
      <tbody v-if="bodyRows.length">
        <tr
          v-for="(row, rowIndex) in bodyRows"
          :key="rowIndex"
        >
          <td
            v-for="(cell, cellIndex) in row"
            :key="cellIndex"
            :class="alignmentClass(cell.alignment)"
          >
            <LiveMarkdownInline :segments="cell.segments" />
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
