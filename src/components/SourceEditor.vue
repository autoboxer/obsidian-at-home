<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import AppIcon from "./AppIcon.vue";

const props = defineProps<{
  modelValue: string;
  noteTitles: string[];
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const textarea = ref<HTMLTextAreaElement>();
const gutter = ref<HTMLElement>();
const cursor = ref(0);
const suggestionIndex = ref(0);
const suggestionQuery = ref<string | null>(null);

const lineCount = computed(() => Math.max(1, props.modelValue.split("\n").length));
const lineNumbers = computed(() => Array.from({ length: lineCount.value }, (_, index) => index + 1).join("\n"));
const suggestions = computed(() => {
  if (suggestionQuery.value === null) return [];
  const query = suggestionQuery.value.toLocaleLowerCase();
  return props.noteTitles
    .filter((title) => !query || title.toLocaleLowerCase().includes(query))
    .sort((a, b) => {
      const aStarts = a.toLocaleLowerCase().startsWith(query);
      const bStarts = b.toLocaleLowerCase().startsWith(query);
      return Number(bStarts) - Number(aStarts) || a.localeCompare(b);
    })
    .slice(0, 6);
});

function onInput(event: Event): void {
  const element = event.target as HTMLTextAreaElement;
  cursor.value = element.selectionStart;
  emit("update:modelValue", element.value);
  updateSuggestions(element);
}

function onSelection(): void {
  if (!textarea.value) return;
  cursor.value = textarea.value.selectionStart;
  updateSuggestions(textarea.value);
}

function onScroll(): void {
  if (!textarea.value || !gutter.value) return;
  gutter.value.scrollTop = textarea.value.scrollTop;
}

function onKeydown(event: KeyboardEvent): void {
  const element = textarea.value;
  if (!element) return;

  if (suggestions.value.length) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      suggestionIndex.value = (suggestionIndex.value + 1) % suggestions.value.length;
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      suggestionIndex.value = (suggestionIndex.value - 1 + suggestions.value.length) % suggestions.value.length;
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      insertSuggestion(suggestions.value[suggestionIndex.value]!);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      suggestionQuery.value = null;
      return;
    }
  }

  if (event.key === "Tab") {
    event.preventDefault();
    replaceSelection("  ");
    return;
  }

  const modifier = event.metaKey || event.ctrlKey;
  if (modifier && event.key.toLocaleLowerCase() === "b") {
    event.preventDefault();
    wrapSelection("**", "**");
  } else if (modifier && event.key.toLocaleLowerCase() === "i") {
    event.preventDefault();
    wrapSelection("_", "_");
  }
}

function replaceSelection(value: string): void {
  const element = textarea.value;
  if (!element) return;
  const start = element.selectionStart;
  const end = element.selectionEnd;
  const next = `${props.modelValue.slice(0, start)}${value}${props.modelValue.slice(end)}`;
  emit("update:modelValue", next);
  nextTick(() => {
    element.focus();
    element.setSelectionRange(start + value.length, start + value.length);
    onSelection();
  });
}

function wrapSelection(before: string, after: string): void {
  const element = textarea.value;
  if (!element) return;
  const start = element.selectionStart;
  const end = element.selectionEnd;
  const selected = props.modelValue.slice(start, end);
  const next = `${props.modelValue.slice(0, start)}${before}${selected}${after}${props.modelValue.slice(end)}`;
  emit("update:modelValue", next);
  nextTick(() => {
    element.focus();
    const selectionStart = start + before.length;
    element.setSelectionRange(selectionStart, selectionStart + selected.length);
  });
}

function updateSuggestions(element: HTMLTextAreaElement): void {
  const beforeCursor = element.value.slice(0, element.selectionStart);
  const match = beforeCursor.match(/\[\[([^\]\n|#]*)$/);
  suggestionQuery.value = match ? match[1] ?? "" : null;
  suggestionIndex.value = 0;
}

function insertSuggestion(title: string): void {
  const element = textarea.value;
  if (!element || suggestionQuery.value === null) return;
  const queryLength = suggestionQuery.value.length;
  const start = element.selectionStart - queryLength;
  const replacement = `${title}]]`;
  const next = `${props.modelValue.slice(0, start)}${replacement}${props.modelValue.slice(element.selectionStart)}`;
  emit("update:modelValue", next);
  suggestionQuery.value = null;
  nextTick(() => {
    element.focus();
    const position = start + replacement.length;
    element.setSelectionRange(position, position);
  });
}
</script>

<template>
  <div class="source-editor">
    <div ref="gutter" class="editor-gutter" aria-hidden="true">
      <pre>{{ lineNumbers }}</pre>
    </div>
    <textarea
      ref="textarea"
      class="source-textarea"
      :value="modelValue"
      spellcheck="true"
      aria-label="Markdown source"
      @input="onInput"
      @click="onSelection"
      @keyup="onSelection"
      @scroll="onScroll"
      @keydown="onKeydown"
    />

    <div v-if="suggestions.length" class="wiki-suggestions" role="listbox">
      <div class="suggestion-kicker">Link a note</div>
      <button
        v-for="(title, index) in suggestions"
        :key="title"
        type="button"
        class="wiki-suggestion"
        :class="{ active: index === suggestionIndex }"
        @mousedown.prevent="insertSuggestion(title)"
      >
        <span class="suggestion-icon"><AppIcon name="link" :size="14" /></span>
        <span>{{ title }}</span>
        <AppIcon v-if="index === suggestionIndex" name="enter" :size="13" />
      </button>
    </div>

    <div class="editor-language-pill">MD</div>
  </div>
</template>
