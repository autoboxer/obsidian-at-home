<script setup lang="ts">
import { nextTick, ref } from "vue";
import { shortcutCommandKey } from "../lib/keyboard";
import AppIcon from "./AppIcon.vue";

interface Shortcut {
  label: string;
  detail?: string;
  keys: string[];
}

interface ShortcutGroup {
  id: "workspace" | "editor" | "view";
  title: string;
  shortcuts: Shortcut[];
}

const referenceOpen = ref(false);
const referenceButton = ref<HTMLButtonElement>();
const closeButton = ref<HTMLButtonElement>();
const referenceDialog = ref<HTMLElement>();
const commandKey = shortcutCommandKey();
const shortcutGroups: ShortcutGroup[] = [
  {
    id: "workspace",
    title: "Workspace",
    shortcuts: [
      { label: "Quick search", keys: [commandKey, "O"] },
      { label: "Create a note", keys: [commandKey, "N"] },
      { label: "Open templates", keys: [commandKey, "Shift", "T"] },
      { label: "Toggle the vault panel", keys: [commandKey, "\\"] },
      { label: "Focus the vault filter", detail: "Outside a text field", keys: ["/"] },
    ],
  },
  {
    id: "editor",
    title: "Editor",
    shortcuts: [
      { label: "Toggle bold", keys: [commandKey, "B"] },
      { label: "Toggle italic", keys: [commandKey, "I"] },
      { label: "Toggle strikethrough", keys: [commandKey, "Shift", "X"] },
      { label: "Create a Markdown link", keys: [commandKey, "K"] },
      { label: "Embed an image", detail: "Choose an image file", keys: [commandKey, "Shift", "I"] },
      { label: "Embed a clipboard image", detail: "When the clipboard contains an image", keys: [commandKey, "V"] },
      { label: "Wrap as inline code", detail: "With text selected", keys: ["Backtick"] },
      { label: "Find in the current note", keys: [commandKey, "F"] },
      { label: "Next find result", detail: "Find field: Enter also works. Anywhere: F3", keys: ["Tab"] },
      { label: "Previous find result", detail: "Find field: Shift+Enter also works. Anywhere: Shift+F3", keys: ["Shift", "Tab"] },
      { label: "Indent", detail: "Current list item or selected lines", keys: ["Tab"] },
      { label: "Outdent", detail: "Current list item or selected lines", keys: ["Shift", "Tab"] },
      { label: "Continue a list", detail: "Also completes an opening code fence", keys: ["Enter"] },
      { label: "Move to the next table cell", detail: "Creates a row after the final cell", keys: ["Tab"] },
      { label: "Move to the previous table cell", keys: ["Shift", "Tab"] },
      { label: "Insert a table row", detail: "Below the current row; moves to its first cell", keys: ["Enter"] },
      { label: "Delete an empty table row", detail: "From its first cell only", keys: ["Backspace"] },
      { label: "Add a line break in a table cell", keys: ["Shift", "Enter"] },
      { label: "Move up a table column", keys: ["↑"] },
      { label: "Move down a table column", detail: "Exits below the final row", keys: ["↓"] },
    ],
  },
  {
    id: "view",
    title: "View",
    shortcuts: [
      { label: "Zoom in", keys: [commandKey, "+"] },
      { label: "Zoom out", keys: [commandKey, "−"] },
      { label: "Reset zoom", keys: [commandKey, "0"] },
    ],
  },
];

async function openReference(): Promise<void> {
  referenceOpen.value = true;
  await nextTick();
  closeButton.value?.focus();
}

async function closeReference(): Promise<void> {
  referenceOpen.value = false;
  await nextTick();
  referenceButton.value?.focus();
}

function handleDialogKeydown(event: KeyboardEvent): void {
  event.stopPropagation();

  if (event.key === "Escape") {
    event.preventDefault();
    void closeReference();

    return;
  }
  if (event.key !== "Tab" || !referenceDialog.value) {
    return;
  }

  const focusable = Array.from(referenceDialog.value.querySelectorAll<HTMLElement>(
    "button:not(:disabled), [href], [tabindex]:not([tabindex='-1'])",
  )).filter((element) => !element.hasAttribute("hidden"));
  const first = focusable[0];
  const last = focusable[focusable.length - 1];

  if (!first || !last) {
    event.preventDefault();
    referenceDialog.value.focus();
  } else if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}
</script>

<template>
  <button
    ref="referenceButton"
    type="button"
    class="rail-button"
    :class="{ active: referenceOpen }"
    :aria-expanded="referenceOpen"
    aria-haspopup="dialog"
    aria-label="Keyboard shortcuts"
    title="Keyboard shortcuts"
    @click="openReference"
  >
    <AppIcon name="keyboard" :size="18" />
    <span class="rail-tooltip">Keyboard shortcuts</span>
  </button>

  <Transition name="overlay-fade">
    <div
      v-if="referenceOpen"
      class="modal-backdrop shortcut-reference-backdrop"
      data-ui-region="keyboard-shortcuts"
      @mousedown.self="closeReference"
    >
      <section
        ref="referenceDialog"
        class="editor-modal shortcut-reference-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcut-reference-title"
        aria-describedby="shortcut-reference-description"
        tabindex="-1"
        @keydown="handleDialogKeydown"
      >
        <header>
          <div>
            <span class="utility-eyebrow">Reference</span>
            <h2 id="shortcut-reference-title">Keyboard shortcuts</h2>
          </div>
          <button
            ref="closeButton"
            type="button"
            class="icon-button"
            aria-label="Close keyboard shortcuts"
            @click="closeReference"
          >
            <AppIcon name="x" :size="16" />
          </button>
        </header>

        <div class="shortcut-reference-content">
          <p id="shortcut-reference-description">
            Work with your vault and format Markdown without leaving the keyboard.
          </p>

          <section
            v-for="group in shortcutGroups"
            :key="group.id"
            class="shortcut-reference-group"
            :class="`group-${group.id}`"
          >
            <h3>{{ group.title }}</h3>
            <dl>
              <div v-for="shortcut in group.shortcuts" :key="shortcut.label" class="shortcut-reference-row">
                <dt>
                  <strong>{{ shortcut.label }}</strong>
                  <small v-if="shortcut.detail">{{ shortcut.detail }}</small>
                </dt>
                <dd class="shortcut-reference-keys" role="group" :aria-label="shortcut.keys.join(' plus ')">
                  <template v-for="(key, index) in shortcut.keys" :key="`${shortcut.label}-${key}`">
                    <span v-if="index" aria-hidden="true">+</span>
                    <kbd aria-hidden="true">{{ key }}</kbd>
                  </template>
                </dd>
              </div>
            </dl>
          </section>
        </div>

        <footer>
          <button type="button" class="primary-action-button small" @click="closeReference">Done</button>
        </footer>
      </section>
    </div>
  </Transition>
</template>
