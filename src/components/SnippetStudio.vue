<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { deleteSnippet, notify, saveSnippet, vaultState } from "../stores/vault";
import type { CssSnippet } from "../types";
import AppIcon from "./AppIcon.vue";

const activeId = ref(vaultState.snippets[0]?.id ?? null);
const draft = reactive({ name: "", description: "", css: "" });
const dirty = ref(false);

const activeSnippet = computed(() => vaultState.snippets.find((snippet) => snippet.id === activeId.value));

watch(activeSnippet, (snippet) => loadDraft(snippet), { immediate: true });

function loadDraft(snippet?: CssSnippet): void {
  if (!snippet) return;
  Object.assign(draft, { name: snippet.name, description: snippet.description, css: snippet.css });
  dirty.value = false;
}

function markDirty(): void {
  dirty.value = true;
}

function save(): void {
  if (!activeSnippet.value) return;
  saveSnippet({ id: activeSnippet.value.id, ...draft });
  dirty.value = false;
  notify("CSS snippet saved", "success");
}

function create(): void {
  const snippet = saveSnippet({
    name: "New snippet",
    description: "A custom style for your notebook.",
    css: `.markdown-preview {\n  /* Add your styles here */\n}\n`,
    enabled: true,
  });
  activeId.value = snippet.id;
}

function toggle(snippet: CssSnippet): void {
  snippet.enabled = !snippet.enabled;
}

function remove(): void {
  const snippet = activeSnippet.value;
  if (!snippet || snippet.builtIn) return;
  if (window.confirm(`Delete the CSS snippet “${snippet.name}”?`)) {
    deleteSnippet(snippet.id);
    activeId.value = vaultState.snippets[0]?.id ?? null;
  }
}
</script>

<template>
  <main class="snippets-workspace utility-workspace">
    <div class="snippet-shell">
      <aside class="snippet-library">
        <header>
          <div><span class="utility-eyebrow">Appearance</span><h1>CSS snippets</h1></div>
          <button type="button" class="icon-button" title="New snippet" @click="create"><AppIcon name="plus" :size="16" /></button>
        </header>
        <p class="snippet-intro">Create and enable custom CSS. Changes apply locally and instantly.</p>
        <div class="snippet-list">
          <button v-for="snippet in vaultState.snippets" :key="snippet.id" type="button" class="snippet-list-item" :class="{ active: activeId === snippet.id }" @click="activeId = snippet.id">
            <span class="snippet-status" :class="{ enabled: snippet.enabled }"><span /></span>
            <span><strong>{{ snippet.name }}</strong><small>{{ snippet.builtIn ? "Built in" : "Custom" }}</small></span>
            <AppIcon name="chevron" :size="13" />
          </button>
        </div>
      </aside>

      <section v-if="activeSnippet" class="snippet-editor-area">
        <header class="snippet-editor-header">
          <div>
            <span class="utility-eyebrow">CSS source</span>
            <h2>{{ activeSnippet.name }}</h2>
          </div>
          <div class="snippet-header-actions">
            <label class="toggle-control">
              <input type="checkbox" :checked="activeSnippet.enabled" @change="toggle(activeSnippet)" />
              <span><i /></span>
              {{ activeSnippet.enabled ? "Enabled" : "Disabled" }}
            </label>
            <button v-if="!activeSnippet.builtIn" type="button" class="icon-button danger-hover" title="Delete snippet" @click="remove"><AppIcon name="trash" :size="16" /></button>
            <button type="button" class="primary-action-button small" :disabled="!dirty" @click="save"><AppIcon name="check" :size="14" /> Save</button>
          </div>
        </header>

        <div class="snippet-fields">
          <label><span>Name</span><input v-model="draft.name" @input="markDirty" /></label>
          <label><span>Description</span><input v-model="draft.description" @input="markDirty" /></label>
        </div>

        <div class="css-editor-frame">
          <div class="css-editor-topbar"><span><span class="code-dot violet" /> CSS</span><span>{{ draft.css.split('\n').length }} lines</span></div>
          <div class="css-editor-body">
            <pre class="css-line-numbers" aria-hidden="true">{{ draft.css.split('\n').map((_, index) => index + 1).join('\n') }}</pre>
            <textarea v-model="draft.css" spellcheck="false" aria-label="CSS source" @input="markDirty" />
          </div>
        </div>

        <footer class="snippet-help">
          <AppIcon name="info" :size="15" />
          <span>Try selectors like <code>.markdown-preview</code>, <code>.source-editor</code>, or CSS variables on <code>:root</code>.</span>
        </footer>
      </section>
    </div>
  </main>
</template>
