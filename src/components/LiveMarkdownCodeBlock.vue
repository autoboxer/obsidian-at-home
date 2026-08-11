<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from "vue";
import {
  CODE_LANGUAGE_OPTIONS,
  findCodeLanguageOption,
  highlightCode,
} from "../lib/highlight";
import type { CodeLanguageOption } from "../lib/highlight";
import type { LiveMarkdownCodeFence } from "../lib/liveMarkdownCode";
import AppIcon from "./AppIcon.vue";

const props = defineProps<{
  fence: LiveMarkdownCodeFence;
  height?: number;
}>();

const emit = defineEmits<{
  "update:language": [language: string];
}>();

const root = ref<HTMLElement>();
const languageButton = ref<HTMLButtonElement>();
const searchInput = ref<HTMLInputElement>();
const pickerOpen = ref(false);
const query = ref("");
const activeOptionIndex = ref(0);
const languageListboxId = `${useId()}-language-listbox`;

const selectedLanguage = computed(() => findCodeLanguageOption(props.fence.language));
const languageLabel = computed(() =>
  selectedLanguage.value?.label || props.fence.language || "Plain text"
);
const highlightedCode = computed(() =>
  props.fence.language
    ? highlightCode(props.fence.code, props.fence.language)
    : undefined
);
const filteredLanguages = computed(() => {
  const search = query.value.trim().toLocaleLowerCase();
  if (!search) {
    return CODE_LANGUAGE_OPTIONS;
  }

  return CODE_LANGUAGE_OPTIONS.filter((option) =>
    [option.label, option.value, ...option.aliases]
      .join(" ")
      .toLocaleLowerCase()
      .includes(search)
  );
});
const activeOptionId = computed(() =>
  filteredLanguages.value[activeOptionIndex.value]
    ? languageOptionId(activeOptionIndex.value)
    : undefined
);
const blockStyle = computed(() => ({
  ...(props.height ? { "--live-code-height": `${props.height}px` } : {}),
  "--live-code-lines": String(props.fence.lineNumbers.length),
  "--live-code-closing-lines": props.fence.closingLine === undefined ? "0" : "1",
}));

watch(query, () => {
  resetActiveOption();
});

function languageOptionId(index: number): string {
  return `${languageListboxId}-option-${index}`;
}

function resetActiveOption(): void {
  const selectedValue = selectedLanguage.value?.value;
  const selectedIndex = query.value.trim()
    ? -1
    : filteredLanguages.value.findIndex((option) => option.value === selectedValue);

  setActiveOption(Math.max(0, selectedIndex));
}

function setActiveOption(index: number): void {
  activeOptionIndex.value = index;
  nextTick(() => {
    const id = activeOptionId.value;
    if (id) {
      document.getElementById(id)?.scrollIntoView({
        block: "nearest",
        inline: "nearest",
      });
    }
  });
}

function togglePicker(): void {
  pickerOpen.value = !pickerOpen.value;
  query.value = "";
  if (pickerOpen.value) {
    resetActiveOption();
    nextTick(() => searchInput.value?.focus());
  }
}

function closePicker(): void {
  pickerOpen.value = false;
  query.value = "";
}

function selectLanguage(option: CodeLanguageOption): void {
  emit("update:language", option.value);
  closePicker();
  nextTick(() => languageButton.value?.focus());
}

function onPickerKeydown(event: KeyboardEvent): void {
  if (event.isComposing) {
    return;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    closePicker();
    nextTick(() => languageButton.value?.focus());

    return;
  }
  if (!filteredLanguages.value.length) {
    return;
  }
  if (event.key === "ArrowDown") {
    event.preventDefault();
    const nextIndex = (
      activeOptionIndex.value + 1
    ) % filteredLanguages.value.length;
    setActiveOption(nextIndex);

    return;
  }
  if (event.key === "ArrowUp") {
    event.preventDefault();
    const previousIndex = (
      activeOptionIndex.value - 1 + filteredLanguages.value.length
    ) % filteredLanguages.value.length;
    setActiveOption(previousIndex);

    return;
  }
  if (event.key === "Home") {
    event.preventDefault();
    setActiveOption(0);

    return;
  }
  if (event.key === "End") {
    event.preventDefault();
    setActiveOption(filteredLanguages.value.length - 1);

    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    selectLanguage(filteredLanguages.value[activeOptionIndex.value]!);
  }
}

function onFocusout(event: FocusEvent): void {
  const nextTarget = event.relatedTarget;
  if (nextTarget instanceof Node && root.value?.contains(nextTarget)) {
    return;
  }

  closePicker();
}
</script>

<template>
  <div
    ref="root"
    class="live-code-block"
    :style="blockStyle"
    @focusout="onFocusout"
  >
    <div class="live-code-header">
      <button
        ref="languageButton"
        type="button"
        class="live-code-language-button"
        aria-haspopup="listbox"
        :aria-expanded="pickerOpen"
        @mousedown.prevent
        @click.stop="togglePicker"
      >
        <span>{{ languageLabel }}</span>
        <AppIcon name="chevron-down" :size="11" />
      </button>

      <Transition name="popover-fade">
        <div
          v-if="pickerOpen"
          class="live-code-language-picker"
          @keydown="onPickerKeydown"
          @mousedown.stop
        >
          <label class="live-code-language-search">
            <AppIcon name="search" :size="13" />
            <input
              ref="searchInput"
              v-model="query"
              type="search"
              role="combobox"
              placeholder="Filter languages…"
              aria-label="Filter code languages"
              aria-autocomplete="list"
              :aria-activedescendant="activeOptionId"
              :aria-controls="languageListboxId"
              :aria-expanded="pickerOpen"
            />
          </label>
          <div
            :id="languageListboxId"
            class="live-code-language-options"
            role="listbox"
            aria-label="Code language"
          >
            <button
              v-for="(option, index) in filteredLanguages"
              :key="option.value || 'plain-text'"
              :id="languageOptionId(index)"
              type="button"
              role="option"
              :aria-selected="index === activeOptionIndex"
              :class="{ active: index === activeOptionIndex }"
              tabindex="-1"
              @mouseenter="activeOptionIndex = index"
              @click="selectLanguage(option)"
            >
              <span>{{ option.label }}</span>
              <code>{{ option.value || 'text' }}</code>
              <AppIcon
                v-if="selectedLanguage?.value === option.value"
                name="check"
                :size="12"
              />
            </button>
            <div
              v-if="!filteredLanguages.length"
              class="live-code-language-empty"
              role="status"
            >
              No matching languages
            </div>
          </div>
        </div>
      </Transition>
    </div>

    <pre class="live-code-body" aria-hidden="true"><code
      v-if="highlightedCode"
      class="hljs"
      v-html="highlightedCode"
    /><code v-else>{{ fence.code || '\u00a0' }}</code></pre>
  </div>
</template>
