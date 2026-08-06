<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import {
  createFromTemplate,
  notify,
  saveTemplate,
  uiState,
  vaultState,
} from "../stores/vault";
import type { NoteTemplate } from "../types";
import AppIcon from "./AppIcon.vue";

const modalOpen = ref(false);
const editingId = ref<string | null>(null);
const draft = reactive({ name: "", description: "", titlePattern: "", content: "" });
const filter = ref("");

const filteredTemplates = computed(() => {
  const query = filter.value.toLocaleLowerCase().trim();
  if (!query) return vaultState.templates;
  return vaultState.templates.filter((template) =>
    `${template.name} ${template.description}`.toLocaleLowerCase().includes(query),
  );
});

function useTemplate(id: string): void {
  const note = createFromTemplate(id);
  if (note) uiState.tool = "notes";
}

function openCreate(): void {
  editingId.value = null;
  Object.assign(draft, {
    name: "",
    description: "",
    titlePattern: "{{date}} — Note",
    content: "# {{title}}\n\n## Notes\n\n",
  });
  modalOpen.value = true;
}

function openEdit(template: NoteTemplate): void {
  editingId.value = template.builtIn ? null : template.id;
  Object.assign(draft, {
    name: template.builtIn ? `${template.name} copy` : template.name,
    description: template.description,
    titlePattern: template.titlePattern,
    content: template.content,
  });
  modalOpen.value = true;
}

function submitTemplate(): void {
  if (!draft.name.trim() || !draft.content.trim()) return;
  saveTemplate({
    id: editingId.value ?? undefined,
    name: draft.name,
    description: draft.description,
    titlePattern: draft.titlePattern,
    content: draft.content,
  });
  modalOpen.value = false;
  notify(editingId.value ? "Template updated" : "Template saved", "success");
}
</script>

<template>
  <main class="templates-workspace utility-workspace">
    <div class="utility-page templates-page">
      <header class="utility-header-row">
        <div class="utility-hero compact">
          <span class="utility-eyebrow">Templates</span>
          <h1>Create notes from templates</h1>
          <p>Templates can use <code v-pre>{{date}}</code>, <code v-pre>{{time}}</code>, and <code v-pre>{{title}}</code>.</p>
        </div>
        <button type="button" class="primary-action-button" @click="openCreate">
          <AppIcon name="plus" :size="16" /> New template
        </button>
      </header>

      <div class="library-toolbar">
        <label><AppIcon name="search" :size="15" /><input v-model="filter" placeholder="Filter templates…" /></label>
        <span>{{ filteredTemplates.length }} templates</span>
      </div>

      <section class="template-grid">
        <article v-for="template in filteredTemplates" :key="template.id" class="template-card">
          <div class="template-card-preview">
            <span class="template-glyph"><AppIcon :name="template.glyph" :size="21" /></span>
            <div class="paper-lines">
              <span /><span /><span /><span />
            </div>
            <span v-if="template.builtIn" class="built-in-badge">Built in</span>
          </div>
          <div class="template-card-body">
            <strong>{{ template.name }}</strong>
            <p>{{ template.description }}</p>
            <code>{{ template.titlePattern }}</code>
          </div>
          <footer>
            <button type="button" class="template-edit" @click="openEdit(template)">
              <AppIcon :name="template.builtIn ? 'copy' : 'edit'" :size="14" />
              {{ template.builtIn ? "Duplicate" : "Edit" }}
            </button>
            <button type="button" class="template-use" @click="useTemplate(template.id)">
              Use template <AppIcon name="arrow" :size="14" />
            </button>
          </footer>
        </article>
      </section>
    </div>

    <div v-if="modalOpen" class="modal-backdrop" @mousedown.self="modalOpen = false">
      <form class="editor-modal template-editor-modal" @submit.prevent="submitTemplate">
        <header>
          <div><span class="utility-eyebrow">Template editor</span><h2>{{ editingId ? "Edit template" : "Create a template" }}</h2></div>
          <button type="button" class="icon-button" @click="modalOpen = false"><AppIcon name="x" :size="16" /></button>
        </header>
        <div class="modal-fields two-column-fields">
          <label><span>Name</span><input v-model="draft.name" required placeholder="Weekly review" /></label>
          <label><span>Title pattern</span><input v-model="draft.titlePattern" placeholder="Weekly review — {{date}}" /></label>
          <label class="full-field"><span>Description</span><input v-model="draft.description" placeholder="A short explanation of when to use this." /></label>
          <label class="full-field"><span>Markdown source</span><textarea v-model="draft.content" required spellcheck="false" /></label>
        </div>
        <footer><button type="button" class="secondary-button" @click="modalOpen = false">Cancel</button><button type="submit" class="primary-action-button"><AppIcon name="check" :size="15" /> Save template</button></footer>
      </form>
    </div>
  </main>
</template>
