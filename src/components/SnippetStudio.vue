<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue';
import { deleteSnippet, notify, saveSnippet, vaultState } from '../stores/vault';
import type { CssSnippet } from '../types';
import AppIcon from './AppIcon.vue';

const activeId = ref( vaultState.snippets[ 0 ]?.id ?? null );
const draft = reactive({ name: '', description: '', css: '' });
const dirty = ref( false );
const referenceOpen = ref( false );
const referenceButton = ref<HTMLButtonElement>();
const referenceDialog = ref<HTMLElement>();

const activeSnippet = computed( () => vaultState.snippets.find( ( snippet ) => snippet.id === activeId.value ) );

watch( activeSnippet, ( snippet ) => loadDraft( snippet ), { immediate: true });

function loadDraft( snippet?: CssSnippet ): void {
  if ( !snippet ) {
    return;
  }
  Object.assign( draft, { name: snippet.name, description: snippet.description, css: snippet.css });
  dirty.value = false;
}

function markDirty(): void {
  dirty.value = true;
}

function save(): void {
  if ( !activeSnippet.value ) {
    return;
  }
  saveSnippet({ id: activeSnippet.value.id, ...draft });
  dirty.value = false;
  notify( 'CSS snippet saved', 'success' );
}

function create(): void {
  const snippet = saveSnippet({
    name: 'New snippet',
    description: 'A custom style for your vault.',
    css: `.source-editor {\n  /* Add your styles here */\n}\n`,
    enabled: true
  });
  activeId.value = snippet.id;
}

function toggle( snippet: CssSnippet ): void {
  snippet.enabled = !snippet.enabled;
}

function remove(): void {
  const snippet = activeSnippet.value;
  if ( !snippet || snippet.builtIn ) {
    return;
  }
  if ( window.confirm( `Delete the CSS snippet “${ snippet.name }”?` ) ) {
    deleteSnippet( snippet.id );
    activeId.value = vaultState.snippets[ 0 ]?.id ?? null;
  }
}

async function openReference(): Promise<void> {
  referenceOpen.value = true;
  await nextTick();
  referenceDialog.value?.focus();
}

async function closeReference(): Promise<void> {
  referenceOpen.value = false;
  await nextTick();
  referenceButton.value?.focus();
}
</script>

<template>
  <main class="snippets-workspace utility-workspace" data-ui-region="snippets">
    <div class="snippet-shell">
      <aside class="snippet-library" data-ui-region="snippet-library">
        <header>
          <div><span class="utility-eyebrow">Appearance</span><h1>CSS snippets</h1></div>
          <button
            type="button"
            class="icon-button"
            title="New snippet"
            @click="create"
          >
            <AppIcon name="plus" :size="16" />
          </button>
        </header>
        <p class="snippet-intro">
          Create and enable custom CSS. Enabled snippets apply immediately; edits apply when saved.
        </p>
        <div class="snippet-list">
          <button
            v-for="snippet in vaultState.snippets"
            :key="snippet.id"
            type="button"
            class="snippet-list-item"
            :class="{ active: activeId === snippet.id }"
            @click="activeId = snippet.id"
          >
            <span class="snippet-status" :class="{ enabled: snippet.enabled }"><span /></span>
            <span><strong>{{ snippet.name }}</strong><small>{{ snippet.builtIn ? "Built in" : "Custom" }}</small></span>
            <AppIcon name="chevron" :size="13" />
          </button>
        </div>
      </aside>

      <section
        v-if="activeSnippet"
        class="snippet-editor-area"
        data-ui-region="snippet-editor"
      >
        <header class="snippet-editor-header">
          <div>
            <span class="utility-eyebrow">CSS source</span>
            <h2>{{ activeSnippet.name }}</h2>
          </div>
          <div class="snippet-header-actions">
            <label class="toggle-control">
              <input
                type="checkbox"
                :checked="activeSnippet.enabled"
                @change="toggle( activeSnippet )"
              >
              <span><i /></span>
              {{ activeSnippet.enabled ? "Enabled" : "Disabled" }}
            </label>
            <button
              v-if="!activeSnippet.builtIn"
              type="button"
              class="icon-button danger-hover"
              title="Delete snippet"
              @click="remove"
            >
              <AppIcon name="trash" :size="16" />
            </button>
            <button
              type="button"
              class="primary-action-button small"
              :disabled="!dirty"
              @click="save"
            >
              <AppIcon name="check" :size="14" /> Save
            </button>
          </div>
        </header>

        <div class="snippet-fields">
          <label><span>Name</span><input v-model="draft.name" @input="markDirty"></label>
          <label><span>Description</span><input v-model="draft.description" @input="markDirty"></label>
        </div>

        <div class="css-editor-frame">
          <div class="css-editor-topbar">
            <span><span class="code-dot violet" /> CSS</span><span>{{ draft.css.split( '\n' ).length }} lines</span>
          </div>
          <div class="css-editor-body">
            <pre class="css-line-numbers" aria-hidden="true">{{ draft.css.split( '\n' ).map( ( _, index ) => index + 1 ).join( '\n' ) }}</pre>
            <textarea
              v-model="draft.css"
              spellcheck="false"
              aria-label="CSS source"
              @input="markDirty"
            />
          </div>
        </div>

        <footer class="snippet-help">
          <span><AppIcon name="info" :size="15" /> Target the unified note editor with <code>.source-editor</code> and its editable content with <code>.source-textarea</code>.</span>
          <button
            ref="referenceButton"
            type="button"
            class="snippet-reference-button"
            @click="openReference"
          >
            Selector reference
          </button>
        </footer>
      </section>
    </div>

    <Transition name="overlay-fade">
      <div
        v-if="referenceOpen"
        v-modal-scroll-lock
        class="modal-backdrop snippet-reference-backdrop"
        data-ui-region="selector-reference"
        @keydown.esc.stop="closeReference"
        @mousedown.self="closeReference"
      >
        <section
          ref="referenceDialog"
          class="editor-modal snippet-reference-modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="snippet-reference-title"
          tabindex="-1"
        >
          <header>
            <div>
              <span class="utility-eyebrow">CSS snippets</span>
              <h2 id="snippet-reference-title">
                Selector reference
              </h2>
            </div>
            <button
              type="button"
              class="icon-button"
              aria-label="Close selector reference"
              @click="closeReference"
            >
              <AppIcon name="x" :size="16" />
            </button>
          </header>

          <div class="snippet-reference-content" data-modal-scroll-region>
            <p>These selectors are stable. Other interface classes may change.</p>

            <details open>
              <summary>App views</summary>
              <div class="snippet-reference-grid">
                <code>[data-app-view="notes"]</code><span>Notes</span>
                <code>[data-app-view="search"]</code><span>Search</span>
                <code>[data-app-view="templates"]</code><span>Templates</span>
                <code>[data-app-view="snippets"]</code><span>CSS snippets</span>
                <code>[data-app-view="settings"]</code><span>Settings</span>
              </div>
            </details>

            <details>
              <summary>Interface regions</summary>
              <div class="snippet-reference-grid">
                <code>[data-ui-region="titlebar"]</code><span>Desktop titlebar</span>
                <code>[data-ui-region="activity-rail"]</code><span>Left navigation</span>
                <code>[data-ui-region="vault-panel"]</code><span>Vault and files panel</span>
                <code>[data-ui-region="editor"]</code><span>Note editor</span>
                <code>[data-ui-region="note-history"]</code><span>Back and Forward controls</span>
                <code>[data-ui-region="recently-deleted"]</code><span>Recently Deleted workspace</span>
                <code>[data-ui-region="recently-deleted-header"]</code><span>Recently Deleted header</span>
                <code>[data-ui-region="recently-deleted-list"]</code><span>Recently Deleted list</span>
                <code>[data-ui-region="recently-deleted-note"]</code><span>Deleted note card</span>
                <code>[data-ui-region="recently-deleted-actions"]</code><span>Deleted note actions</span>
                <code>[data-ui-region="note-title"]</code><span>Note title field</span>
                <code>[data-ui-region="document-search"]</code><span>Find-in-note bar</span>
                <code>[data-ui-region="context-panel"]</code><span>Links and note details</span>
                <code>[data-ui-region="search"]</code><span>Search page</span>
                <code>[data-ui-region="templates"]</code><span>Templates page</span>
                <code>[data-ui-region="snippets"]</code><span>CSS snippets page</span>
                <code>[data-ui-region="snippet-library"]</code><span>Snippet list</span>
                <code>[data-ui-region="snippet-editor"]</code><span>Snippet editor</span>
                <code>[data-ui-region="settings"]</code><span>Settings page</span>
                <code>[data-ui-region="quick-switcher"]</code><span>Quick switcher</span>
                <code>[data-ui-region="vault-chooser"]</code><span>Vault chooser</span>
                <code>[data-ui-region="template-dialog"]</code><span>Template editor</span>
                <code>[data-ui-region="keyboard-shortcuts"]</code><span>Keyboard shortcut reference</span>
                <code>[data-ui-region="selector-reference"]</code><span>This selector guide</span>
                <code>[data-ui-region="notification"]</code><span>Notifications</span>
              </div>
            </details>

            <details>
              <summary>Editor and context</summary>
              <div class="snippet-reference-grid">
                <code>[data-editor-view="live"]</code><span>Unified editor view</span>
                <code>[data-note-view="recently-deleted"]</code><span>Recently Deleted note view</span>
                <code>[data-editor-pane="live"]</code><span>Unified note pane</span>
                <code>[data-context-view="links"]</code><span>Links context tab</span>
                <code>[data-context-view="info"]</code><span>Info context tab</span>
                <code>.source-editor</code><span>Live Markdown editor</span>
                <code>.source-textarea</code><span>Unified editable content</span>
                <code>.document-search-bar</code><span>Find-in-note controls</span>
                <code>[data-ui-region="note-title"]</code><span>Note title</span>
                <code>.tag-chip</code><span>Note tags</span>
              </div>
            </details>

            <details>
              <summary>Actions</summary>
              <div class="snippet-reference-grid">
                <code>[data-note-action="navigate-back"]</code><span>Back through note history</span>
                <code>[data-note-action="navigate-forward"]</code><span>Forward through note history</span>
                <code>[data-note-action="toggle-frontmatter"]</code><span>Show or hide note frontmatter</span>
                <code>[data-recovery-action="restore"]</code><span>Restore a deleted note</span>
                <code>[data-recovery-action="delete"]</code><span>Permanently delete one note</span>
                <code>[data-recovery-action="empty"]</code><span>Empty Recently Deleted</span>
                <code>[data-recovery-action="load-more"]</code><span>Load more deleted notes</span>
              </div>
            </details>

            <details>
              <summary>Common interface elements</summary>
              <div class="snippet-reference-grid">
                <code>.rail-button</code><span>Navigation buttons</span>
                <code>.vault-tree-folder-row</code><span>Folder rows</span>
                <code>.vault-tree-note</code><span>Note rows</span>
                <code>.connection-card</code><span>Outgoing-link cards</span>
                <code>.backlink-card</code><span>Backlink cards</span>
                <code>.search-result-card</code><span>Search results</span>
                <code>.template-card</code><span>Template cards</span>
                <code>.snippet-list-item</code><span>CSS snippet rows</span>
                <code>.settings-section</code><span>Settings sections</span>
                <code>.popover-menu</code><span>Context menus</span>
                <code>.command-palette</code><span>Quick switcher dialog</span>
                <code>.vault-chooser-dialog</code><span>Vault chooser dialog</span>
                <code>.shortcut-reference-modal</code><span>Keyboard shortcut dialog</span>
                <code>.editor-modal</code><span>Editor dialogs</span>
                <code>.primary-action-button</code><span>Primary buttons</span>
                <code>.app-toast</code><span>Notifications</span>
              </div>
            </details>

            <details>
              <summary>Live Markdown</summary>
              <div class="snippet-reference-grid">
                <code>.live-markdown-block.is-heading</code><span>All headings</span>
                <code>.live-markdown-block.heading-level-1</code><span>Level-one headings</span>
                <code>.live-inline-segment.is-link</code><span>Markdown links</span>
                <code>.live-inline-segment.is-wiki-link</code><span>Wiki links</span>
                <code>.live-inline-segment.is-heading-link</code><span>Links to note headings</span>
                <code>.live-inline-segment.is-wiki-link.is-unresolved</code><span>Unresolved wiki links</span>
                <code>.live-inline-segment.is-strong</code><span>Bold text</span>
                <code>.live-inline-segment.is-emphasis</code><span>Italic text</span>
                <code>.live-inline-segment.is-strikethrough</code><span>Strikethrough text</span>
                <code>.live-inline-segment.is-code</code><span>Inline code</span>
                <code>.live-markdown-block.is-blockquote</code><span>Blockquotes</span>
                <code>.live-markdown-block.is-list</code><span>List items</span>
                <code>.live-list-marker</code><span>List bullets and numbers</span>
                <code>.live-markdown-block.is-code-content</code><span>Code block content</span>
                <code>.live-markdown-block.is-table-row</code><span>Table rows</span>
                <code>.live-table-cell</code><span>Table cells</span>
                <code>.live-markdown-block.is-task</code><span>Tasks</span>
                <code>.live-task-checkbox</code><span>Task checkboxes</span>
                <code>.live-code-language-button</code><span>Code language control</span>
                <code>.live-code-language-picker</code><span>Code language menu</span>
                <code>.is-code-content .hljs-keyword</code><span>Highlighted keywords</span>
              </div>
            </details>

            <div class="snippet-reference-example">
              <span>Example</span>
              <pre><code>[data-editor-view="live"] .source-editor {
  --source-editor-line-height: calc(var(--note-font-size) * 1.85);
}

.live-markdown-block.is-heading {
  color: #c9c1ff;
}

[data-app-view="settings"] [data-ui-region="titlebar"] {
  background: #111;
}</code></pre>
            </div>
          </div>

          <footer>
            <button
              type="button"
              class="primary-action-button small"
              @click="closeReference"
            >
              Done
            </button>
          </footer>
        </section>
      </div>
    </Transition>
  </main>
</template>
