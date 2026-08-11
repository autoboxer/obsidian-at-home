<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: number; strokeWidth?: number }>(), {
  size: 18,
  strokeWidth: 1.8,
});

const paths: Record<string, string> = {
  notes: '<path d="M5 3.5h10.5a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5.5a2 2 0 0 1 2-2Z"/><path d="M7 8h7M7 12h7M7 16h4"/>',
  search: '<circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 4 4"/>',
  templates: '<path d="M6.5 3.5h8l3 3V20H6.5a2 2 0 0 1-2-2V5.5a2 2 0 0 1 2-2Z"/><path d="M14.5 3.5v4h3M8 12h6M8 15.5h4"/>',
  snippets: '<path d="m8.5 7-4 5 4 5M15.5 7l4 5-4 5M13.5 4l-3 16"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.08A1.7 1.7 0 0 0 8.95 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.56-1H3v-4h.08A1.7 1.7 0 0 0 4.6 8.95a1.7 1.7 0 0 0-.34-1.88L4.2 7l2.83-2.83.06.06A1.7 1.7 0 0 0 8.95 4.6 1.7 1.7 0 0 0 10 3.08V3h4v.08a1.7 1.7 0 0 0 1 1.53 1.7 1.7 0 0 0 1.88-.34l.06-.06L19.77 7l-.06.06a1.7 1.7 0 0 0-.34 1.88A1.7 1.7 0 0 0 20.92 10H21v4h-.08A1.7 1.7 0 0 0 19.4 15Z"/>',
  keyboard: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10h.01M10.5 10h.01M14 10h.01M17.5 10h.01M7 13h.01M10.5 13h.01M14 13h.01M17.5 13h.01M8 16h8"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  minus: '<path d="M5 12h14"/>',
  "zoom-in": '<circle cx="10.8" cy="10.8" r="6.8"/><path d="M10.8 7.8v6M7.8 10.8h6m2.2 5.2 4 4"/>',
  folder: '<path d="M3.5 6.5A2.5 2.5 0 0 1 6 4h4l2 2h6a2.5 2.5 0 0 1 2.5 2.5v8A2.5 2.5 0 0 1 18 19H6a2.5 2.5 0 0 1-2.5-2.5Z"/>',
  "folder-plus": '<path d="M3.5 6.5A2.5 2.5 0 0 1 6 4h4l2 2h6a2.5 2.5 0 0 1 2.5 2.5v8A2.5 2.5 0 0 1 18 19H6a2.5 2.5 0 0 1-2.5-2.5Z"/><path d="M12 11v6M9 14h6"/>',
  "folder-open": '<path d="M3.5 8V6.5A2.5 2.5 0 0 1 6 4h4l2 2h6a2.5 2.5 0 0 1 2.5 2.5V10"/><path d="M4.5 9.5h16l-2 8a2 2 0 0 1-2 1.5H6a2 2 0 0 1-2-1.5l-1-5.5a2 2 0 0 1 1.5-2.5Z"/>',
  chevron: '<path d="m9 6 6 6-6 6"/>',
  "chevron-down": '<path d="m6 9 6 6 6-6"/>',
  pin: '<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3-1-5ZM12 14v7"/>',
  link: '<path d="M10 13a4.5 4.5 0 0 0 6.6.1l2-2a4.5 4.5 0 0 0-6.4-6.3l-1.1 1.1"/><path d="M14 11a4.5 4.5 0 0 0-6.6-.1l-2 2a4.5 4.5 0 0 0 6.4 6.3l1.1-1.1"/>',
  backlinks: '<path d="M9 7H6a4 4 0 0 0 0 8h3M15 7h3a4 4 0 0 1 0 8h-3M8 11h8M8 3 4 7l4 4"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  more: '<circle cx="5" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="19" cy="12" r="1" fill="currentColor" stroke="none"/>',
  sidebar: '<rect x="3.5" y="4" width="17" height="16" rx="2"/><path d="M9 4v16"/>',
  "panel-right": '<rect x="3.5" y="4" width="17" height="16" rx="2"/><path d="M15 4v16"/>',
  code: '<path d="m8.5 7-4 5 4 5M15.5 7l4 5-4 5"/>',
  columns: '<rect x="3.5" y="4" width="17" height="16" rx="2"/><path d="M12 4v16"/>',
  eye: '<path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z"/><circle cx="12" cy="12" r="2.5"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  trash: '<path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5"/>',
  star: '<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9L12 3Z"/>',
  command: '<path d="M9 7V5a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v14a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3Z"/>',
  arrow: '<path d="M5 12h14M14 7l5 5-5 5"/>',
  enter: '<path d="M5 5v4a4 4 0 0 0 4 4h10M15 9l4 4-4 4"/>',
  calendar: '<rect x="3.5" y="5" width="17" height="15" rx="2"/><path d="M8 3v4M16 3v4M3.5 10h17M8 14h.01M12 14h.01M16 14h.01"/>',
  users: '<path d="M16 20v-1.5a4 4 0 0 0-4-4H7a4 4 0 0 0-4 4V20M9.5 10.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7ZM17 11a3 3 0 0 0 0-6M20.5 20v-1.5a4 4 0 0 0-2.5-3.7"/>',
  briefcase: '<rect x="3" y="7" width="18" height="13" rx="2"/><path d="M8 7V4h8v3M3 12h18M10 12v2h4v-2"/>',
  "file-plus": '<path d="M6 3.5h8l4 4V20H6a2 2 0 0 1-2-2V5.5a2 2 0 0 1 2-2Z"/><path d="M14 3.5v4h4M8 13h6M11 10v6"/>',
  "file-text": '<path d="M6 3.5h8l4 4V20H6a2 2 0 0 1-2-2V5.5a2 2 0 0 1 2-2Z"/><path d="M14 3.5v4h4M8 12h6M8 15.5h6"/>',
  import: '<path d="M12 3v12M7 10l5 5 5-5M5 20h14"/>',
  export: '<path d="M12 16V4M7 9l5-5 5 5M5 20h14"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  hash: '<path d="M10 3 8 21M16 3l-2 18M4 9h16M3 15h16"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  edit: '<path d="m4 16-.8 4 4-.8L18 8.4 14.6 5 4 16ZM12.8 6.8l3.4 3.4"/>',
  tag: '<path d="M20 13 13 20l-9-9V4h7l9 9Z"/><circle cx="8" cy="8" r="1"/>',
  sparkles: '<path d="m12 3 1.2 3.8L17 8l-3.8 1.2L12 13l-1.2-3.8L7 8l3.8-1.2L12 3ZM18.5 14l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8.8-2.2ZM5.5 13l.8 2.2 2.2.8-2.2.8L5.5 19l-.8-2.2-2.2-.8 2.2-.8.8-2.2Z"/>',
  archive: '<rect x="3" y="5" width="18" height="4" rx="1"/><path d="M5 9v10h14V9M9 13h6"/>',
  refresh: '<path d="M20 7v5h-5M4 17v-5h5M6.1 8a7 7 0 0 1 11.5-1L20 12M4 12l2.4 5a7 7 0 0 0 11.5-1"/>',
};
</script>

<template>
  <svg
    class="app-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    :stroke-width="strokeWidth"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    v-html="paths[name] ?? paths.notes"
  />
</template>
