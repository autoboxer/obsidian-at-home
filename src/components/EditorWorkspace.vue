<script setup lang="ts">
import { openUrl } from '@tauri-apps/plugin-opener';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  useAssetEmbedding,
  type AssetEmbeddingEditor
} from '../composables/useAssetEmbedding';
import { leadingFrontmatterEnd } from '../lib/frontmatter';
import { findMarkdownHeading, markdownHeadingSuggestions } from '../lib/headingLinks';
import { formatCommandShortcut } from '../lib/keyboard';
import { parseMarkdownNoteTarget, type MarkdownNoteTarget } from '../lib/markdownLinks';
import { resolveNoteLink, resolveWikiLink, wikiLinkSuggestions, type WikiLinkTarget } from '../lib/wikiLinks';
import {
  editorPositionVaultId,
  getNoteEditorPosition,
  setNoteEditorPosition
} from '../stores/editorPositions';
import {
  canEditVault,
  activeNote,
  activeNoteTab,
  activateNoteTab,
  backNavigationNote,
  canNavigateBack,
  canNavigateForward,
  closeNoteTab,
  createFolder,
  createLinkedNote,
  createNote,
  createNoteTab,
  deleteNote,
  folderPath,
  forwardNavigationNote,
  navigateBack,
  navigateForward,
  moveNoteToFolder,
  notify,
  noteTabsState,
  noteLinkPaths,
  openQuickSearch,
  renameVaultAttachment,
  selectNote,
  togglePinned,
  uiState,
  updateNote,
  vaultSession,
  vaultState
} from '../stores/vault';
import type {
  NoteEditorPosition
} from '../types';
import AppIcon from './AppIcon.vue';
import SourceEditor from './SourceEditor.vue';

const createNoteShortcut = formatCommandShortcut( 'N' );
const newTabShortcut = formatCommandShortcut( 'T' );
const closeTabShortcut = formatCommandShortcut( 'W' );
const openNoteShortcut = formatCommandShortcut( 'O' );
const tabList = ref<HTMLElement>();
const emptyTabOpenButton = ref<HTMLButtonElement>();
const keepTabFocus = ref( false );
let tabResizeObserver: ResizeObserver | undefined;
const noteTabs = computed( () => {
  const notes = new Map( vaultState.notes.map( ( note ) => [ note.id, note ]) );

  return noteTabsState.tabs.map( ( tab ) => {
    const note = tab.noteId ? notes.get( tab.noteId ) : undefined;
    const title = note ? note.title.trim() || 'Untitled note' : 'New tab';

    return {
      id: tab.id,
      title,
      path: note?.relativePath || ( note ? [ folderPath( note.folderId ), title ].filter( Boolean ).join( '/' ) : title )
    };
  });
});

function tabButton( id: string ): HTMLButtonElement | undefined {
  return Array.from( tabList.value?.querySelectorAll<HTMLButtonElement>( '[role="tab"]' ) ?? [])
    .find( ( button ) => button.dataset.noteTabId === id );
}

function revealActiveTab(): void {
  const list = tabList.value;
  const tab = tabButton( activeNoteTab.value.id )?.parentElement;
  if ( !list || !tab ) {
    return;
  }
  const bounds = list.getBoundingClientRect();
  const item = tab.getBoundingClientRect();
  if ( item.left < bounds.left ) {
    list.scrollLeft += item.left - bounds.left;
  } else if ( item.right > bounds.right ) {
    list.scrollLeft += item.right - bounds.right;
  }
}

function focusEmptyTab(): void {
  if ( !activeNote.value && !keepTabFocus.value ) {
    emptyTabOpenButton.value?.focus({ preventScroll: true });
  }
}

async function selectTab( id: string, focusTab = false ): Promise<void> {
  keepTabFocus.value = focusTab;
  activateNoteTab( id );
  await nextTick();
  if ( activeNoteTab.value.id === id ) {
    if ( focusTab ) {
      tabButton( id )?.focus({ preventScroll: true });
    } else if ( activeNote.value ) {
      // Selecting the already active tab does not remount its editor.
      document.querySelector<HTMLElement>( '.source-editor .cm-content' )?.focus({ preventScroll: true });
    } else {
      focusEmptyTab();
    }
  }
  keepTabFocus.value = false;
}

async function closeTab( id: string ): Promise<void> {
  const focusTab = Boolean( tabList.value?.contains( document.activeElement ) );
  keepTabFocus.value = focusTab;
  closeNoteTab( id );
  await nextTick();
  if ( focusTab ) {
    tabButton( activeNoteTab.value.id )?.focus({ preventScroll: true });
  }
  keepTabFocus.value = false;
}

function handleTabKeydown( event: KeyboardEvent ): void {
  if ( event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.isComposing ) {
    return;
  }
  const target = event.target;
  const id = target instanceof HTMLElement ? target.closest<HTMLElement>( '[data-note-tab-id]' )?.dataset.noteTabId : undefined;
  const index = noteTabs.value.findIndex( ( tab ) => tab.id === id );
  if ( index < 0 ) {
    return;
  }
  const last = noteTabs.value.length - 1;
  const next = event.key === 'ArrowRight' ? ( index + 1 ) % ( last + 1 )
    : event.key === 'ArrowLeft' ? ( index + last ) % ( last + 1 )
      : event.key === 'Home' ? 0 : event.key === 'End' ? last : undefined;
  if ( next !== undefined ) {
    event.preventDefault();
    void selectTab( noteTabs.value[ next ]!.id, true );
  } else if ( event.key === 'Delete' ) {
    event.preventDefault();
    void closeTab( id! );
  }
}

function scrollTabs( event: WheelEvent ): void {
  const list = tabList.value;
  if ( !list || event.ctrlKey || event.shiftKey || event.deltaX || list.scrollWidth <= list.clientWidth ) {
    return;
  }
  event.preventDefault();
  list.scrollLeft += event.deltaY * ( event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? list.clientWidth : 1 );
}

watch([ () => activeNoteTab.value.id, noteTabs ], () => {
  revealActiveTab();
}, { flush: 'post' });
watch( () => activeNoteTab.value.id, focusEmptyTab, { flush: 'post' });
onMounted( () => {
  tabResizeObserver = new ResizeObserver( revealActiveTab );
  if ( tabList.value ) {
    tabResizeObserver.observe( tabList.value );
  }
  revealActiveTab();
  focusEmptyTab();
});
onBeforeUnmount( () => tabResizeObserver?.disconnect() );
const embedAttachmentShortcut = formatCommandShortcut( 'Shift+A' );
const embedImageShortcut = formatCommandShortcut( 'Shift+I' );
const tagInputOpen = ref( false );
const tagInput = ref( '' );
const tagField = ref<HTMLInputElement>();
const tagSuggestionIndex = ref( -1 );
const noteMenuOpen = ref( false );
const quickFolderOpen = ref( false );
const quickFolderName = ref( '' );
const quickFolderField = ref<HTMLInputElement>();
const quickFolderButton = ref<HTMLButtonElement>();
const sourceEditor = ref<AssetEmbeddingEditor & {
  focusDocumentOffset: ( offset: number ) => boolean;
}>();
const {
  activateEmbeddedAttachment,
  attachmentEmbedBusy,
  embedAttachmentFromFile,
  embedAttachmentFromVault,
  embedExternalFiles,
  embedImageFromClipboard,
  embedImageFromFile,
  embedImageFromVault,
  imageEmbedBusy,
  nativeAvailable,
  requestAttachmentFromToolbar,
  requestImageFromToolbar,
  revealEmbeddedAttachmentInTree,
  storeAndInsertAttachment,
  showEmbeddedAttachmentInFolder
} = useAssetEmbedding( sourceEditor );

defineExpose({ openNoteLink, storeAndInsertAttachment });

const noteLinkTargets = computed( () => wikiLinkSuggestions( vaultState.notes, noteLinkPaths.value ) );

function noteLinkHeadings( target: string ): string[] {
  const note = resolveWikiLink({ target }, vaultState.notes, activeNote.value, noteLinkPaths.value );

  return note ? markdownHeadingSuggestions( note.content ) : [];
}

function noteLinkForId( noteId: string ): string | undefined {
  const path = noteLinkPaths.value.get( noteId );
  if ( !path ) {
    return undefined;
  }
  // A dropped file identifies an exact note, even among duplicate names
  // or notes whose only difference is their Markdown extension.
  return '[[/' + path.replace( /[\\|#[\]]/g, '\\$&' ) + ']]';
}

const wikiLinkIsResolved = computed( () => {
  const paths = noteLinkPaths.value;
  const sourceNote = activeNote.value;
  const resolvedTargets = new Map<string, boolean>();

  // Render updates reuse results until the source note or vault paths change.
  return ( target: string ): boolean => {
    const cached = resolvedTargets.get( target );
    if ( cached !== undefined ) {
      return cached;
    }
    const resolved = Boolean( resolveWikiLink({ target }, vaultState.notes, sourceNote, paths ) );
    resolvedTargets.set( target, resolved );

    return resolved;
  };
});
const positionVaultId = computed( () => editorPositionVaultId( vaultSession.backend, vaultSession.path ) );
// Access changes rebuild the editor and its widget controls.
const editorKey = computed( () => JSON.stringify([
  positionVaultId.value,
  activeNoteTab.value.id,
  canEditVault.value,
  activeNote.value?.id ?? null
]) );
const sortedFolders = computed( () => [ ...vaultState.folders ].sort( ( a, b ) => folderPath( a.id ).localeCompare( folderPath( b.id ) ) ) );
const tagSuggestions = computed( () => {
  const query = normalizeTag( tagInput.value ).toLocaleLowerCase();
  const appliedTags = new Set( activeNote.value?.tags.map( ( tag ) => tag.toLocaleLowerCase() ) ?? []);
  const uniqueTags = new Map<string, string>();

  for ( const note of vaultState.notes ) {
    for ( const tag of note.tags ) {
      if ( !uniqueTags.has( tag.toLocaleLowerCase() ) ) {
        uniqueTags.set( tag.toLocaleLowerCase(), tag );
      }
    }
  }

  return [ ...uniqueTags.values() ]
    .filter( ( tag ) => !appliedTags.has( tag.toLocaleLowerCase() ) )
    .filter( ( tag ) => !query || tag.toLocaleLowerCase().includes( query ) )
    .sort( ( a, b ) => {
      const aStartsWithQuery = query && a.toLocaleLowerCase().startsWith( query ) ? 0 : 1;
      const bStartsWithQuery = query && b.toLocaleLowerCase().startsWith( query ) ? 0 : 1;

      return aStartsWithQuery - bStartsWithQuery || a.localeCompare( b );
    })
    .slice( 0, 7 );
});
const wordCount = computed( () => {
  const content = activeNote.value?.content ?? '';

  return content.trim() ? content.trim().split( /\s+/ ).length : 0;
});
const characterCount = computed( () => activeNote.value?.content.length ?? 0 );
const hasFrontmatter = computed( () => (
  activeNote.value
    ? leadingFrontmatterEnd( activeNote.value.content ) !== undefined
    : false
) );
const backNavigationLabel = computed( () => backNavigationNote.value
  ? `Back to “${ backNavigationNote.value.title.trim() || 'Untitled note' }”`
  : 'No previous note' );
const forwardNavigationLabel = computed( () => forwardNavigationNote.value
  ? `Forward to “${ forwardNavigationNote.value.title.trim() || 'Untitled note' }”`
  : 'No next note' );

function setTitle( event: Event ): void {
  if ( !activeNote.value ) {
    return;
  }
  updateNote( activeNote.value.id, { title: ( event.target as HTMLInputElement ).value });
}

function resetTitle( event: FocusEvent ): void {
  // A colliding draft can be completed while typing; leaving the field shows
  // the last accepted name instead of implying that a rejected rename saved.
  ( event.target as HTMLInputElement ).value = activeNote.value?.title ?? '';
}

function setContent( content: string ): void {
  if ( activeNote.value ) {
    updateNote( activeNote.value.id, { content });
  }
}

function savedEditorPosition( noteId: string, content: string ): NoteEditorPosition | undefined {
  return getNoteEditorPosition( positionVaultId.value, noteId, content );
}

function rememberEditorPosition(
  vaultId: string,
  noteId: string,
  position: NoteEditorPosition
): void {
  if (
    vaultId === positionVaultId.value
    && !vaultState.notes.some( ( note ) => note.id === noteId )
  ) {
    return;
  }

  setNoteEditorPosition( vaultId, noteId, position );
}

async function openRenderedLink( href: string ): Promise<void> {
  const target = parseMarkdownNoteTarget( href );
  if ( target ) {
    await openNoteLink( target );

    return;
  }
  if ( !/^(?:https?|mailto):/i.test( href ) ) {
    return;
  }

  try {
    await openUrl( href );
  } catch {
    notify( 'Could not open that link', 'warning' );
  }
}

async function openWikiLink( target: string, heading?: string ): Promise<void> {
  if ( !heading ) {
    const note = createLinkedNote({ target });
    if ( note ) {
      await focusLinkedNote( note.id, 0 );
    }

    return;
  }

  await openNoteLink({ target, heading });
}

async function openNoteLink( link: WikiLinkTarget | MarkdownNoteTarget ): Promise<void> {
  const note = resolveNoteLink( link, vaultState.notes, activeNote.value, noteLinkPaths.value );
  if ( !note ) {
    notify( `Could not find note “${ linkedNoteLabel( link.target ) }”`, 'warning' );

    return;
  }
  const heading = link.heading;
  const match = heading ? findMarkdownHeading( note.content, heading ) : undefined;
  if ( heading && !match ) {
    notify( `Could not find heading “${ heading }” in “${ note.title }”`, 'warning' );

    return;
  }

  await focusLinkedNote( note.id, match?.contentFrom ?? 0 );
}

async function focusLinkedNote( noteId: string, offset: number ): Promise<void> {
  const vaultId = positionVaultId.value;
  const tabId = activeNoteTab.value.id;
  if ( noteId !== activeNote.value?.id ) {
    selectNote( noteId );
  }
  // Link destinations override the restored position after the editor mounts,
  // including self-links and notes just selected by createLinkedNote.
  await nextTick();
  if ( positionVaultId.value !== vaultId || activeNoteTab.value.id !== tabId || activeNote.value?.id !== noteId ) {
    return;
  }

  if ( !sourceEditor.value?.focusDocumentOffset( offset ) ) {
    notify( 'Could not move to that location', 'warning' );
  }
}

function linkedNoteLabel( target: string ): string {
  return target.trim().replace( /\.md$/i, '' ) || 'current note';
}

async function setFolder( event: Event ): Promise<void> {
  if ( !activeNote.value ) {
    return;
  }
  const value = ( event.target as HTMLSelectElement ).value;
  await moveNoteToFolder( activeNote.value.id, value || null );
}

function openTagInput(): void {
  if ( !canEditVault.value ) {
    return;
  }
  tagInput.value = '';
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = true;
  nextTick( () => {
    tagField.value?.focus();
    tagField.value?.select();
  });
}

function normalizeTag( value: string ): string {
  return value.trim().replace( /^#/, '' ).replace( /\s+/g, '-' );
}

function addTag( suggestedTag?: string ): void {
  if ( !canEditVault.value ) {
    return;
  }
  if ( !activeNote.value ) {
    return;
  }

  const tag = normalizeTag( suggestedTag ?? tagInput.value );
  const alreadyApplied = activeNote.value.tags.some(
    ( candidate ) => candidate.toLocaleLowerCase() === tag.toLocaleLowerCase()
  );
  if ( tag && !alreadyApplied ) {
    if ( !updateNote( activeNote.value.id, { tags: [ ...activeNote.value.tags, tag ] }) ) {
      return;
    }
  }

  tagInput.value = '';
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function submitTag(): void {
  const suggestion = tagSuggestionIndex.value >= 0
    ? tagSuggestions.value[ tagSuggestionIndex.value ]
    : undefined;
  addTag( suggestion );
}

function cancelTagInput(): void {
  tagInput.value = '';
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function handleTagKeydown( event: KeyboardEvent ): void {
  if ( event.key === 'ArrowDown' && tagSuggestions.value.length ) {
    event.preventDefault();
    tagSuggestionIndex.value = ( tagSuggestionIndex.value + 1 ) % tagSuggestions.value.length;
  } else if ( event.key === 'ArrowUp' && tagSuggestions.value.length ) {
    event.preventDefault();
    tagSuggestionIndex.value = tagSuggestionIndex.value <= 0
      ? tagSuggestions.value.length - 1
      : tagSuggestionIndex.value - 1;
  }
}

function removeTag( tag: string ): void {
  if ( activeNote.value ) {
    updateNote( activeNote.value.id, { tags: activeNote.value.tags.filter( ( candidate ) => candidate !== tag ) });
  }
}

function requestDelete(): void {
  if ( !canEditVault.value ) {
    return;
  }
  if ( !activeNote.value ) {
    return;
  }
  noteMenuOpen.value = false;
  if ( window.confirm( `Delete “${ activeNote.value.title || 'Untitled note' }”? It will remain in Recently Deleted for seven days.` ) ) {
    void deleteNote( activeNote.value.id );
  }
}

function toggleFrontmatter(): void {
  uiState.frontmatterVisible = !uiState.frontmatterVisible;
}

function openQuickFolder(): void {
  if ( !canEditVault.value ) {
    return;
  }
  quickFolderOpen.value = true;
  nextTick( () => quickFolderField.value?.focus() );
}

function closeQuickFolder( restoreFocus = false ): void {
  quickFolderOpen.value = false;
  quickFolderName.value = '';
  if ( restoreFocus ) {
    nextTick( () => quickFolderButton.value?.focus() );
  }
}

function submitQuickFolder(): void {
  const name = quickFolderName.value.trim();
  if ( name ) {
    createFolder( name );
  }
  closeQuickFolder( true );
}

function handleQuickFolderFocusOut( event: FocusEvent ): void {
  const form = event.currentTarget as HTMLElement;
  const next = event.relatedTarget;
  if ( !( next instanceof Node ) || !form.contains( next ) ) {
    closeQuickFolder();
  }
}

watch(
  () => uiState.explorerOpen,
  ( open ) => {
    if ( open ) {
      closeQuickFolder();
    }
  }
);

watch(
  [ () => activeNote.value?.id, () => activeNoteTab.value.id ],
  () => {
    noteMenuOpen.value = false;
    cancelTagInput();
    closeQuickFolder();
  }
);

watch( canEditVault, ( editable ) => {
  if ( !editable ) {
    cancelTagInput();
    closeQuickFolder();
  }
});

watch( tagInput, () => {
  tagSuggestionIndex.value = -1;
});
</script>

<template>
  <main
    class="editor-workspace"
    data-ui-region="editor"
    data-editor-view="live"
  >
    <div class="note-tabs" data-ui-region="note-tabs">
      <div
        ref="tabList"
        class="note-tab-list"
        role="tablist"
        aria-label="Open notes"
        @keydown="handleTabKeydown"
        @wheel="scrollTabs"
      >
        <div
          v-for="tab in noteTabs"
          :key="tab.id"
          class="note-tab"
          :class="{ active: tab.id === activeNoteTab.id }"
          role="presentation"
          @mousedown.middle.prevent
          @mouseup.middle.prevent="closeTab( tab.id )"
        >
          <button
            :id="`note-tab-${tab.id}`"
            class="note-tab-select"
            type="button"
            role="tab"
            :data-note-tab-id="tab.id"
            :aria-selected="tab.id === activeNoteTab.id"
            aria-controls="note-tab-panel"
            :tabindex="tab.id === activeNoteTab.id ? 0 : -1"
            :title="tab.path"
            @click.left="selectTab( tab.id )"
          >
            <AppIcon name="file-text" :size="14" />
            <span>{{ tab.title }}</span>
          </button>
          <button
            class="note-tab-close icon-button subtle"
            type="button"
            tabindex="-1"
            :aria-label="`Close ${tab.title} tab`"
            :title="`Close tab · ${closeTabShortcut}`"
            @mousedown.prevent
            @click.left="closeTab( tab.id )"
          >
            <AppIcon name="x" :size="12" />
          </button>
        </div>
      </div>
      <button
        class="icon-button new-note-tab"
        type="button"
        data-note-action="new-tab"
        aria-label="New tab"
        :title="`New tab · ${newTabShortcut}`"
        @click.left="createNoteTab()"
      >
        <AppIcon name="plus" :size="16" />
      </button>
    </div>
    <header class="editor-toolbar">
      <div class="editor-crumbs">
        <button
          class="icon-button explorer-toggle"
          type="button"
          :class="{ active: uiState.explorerOpen }"
          :title="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-label="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-pressed="uiState.explorerOpen"
          @click="uiState.explorerOpen = !uiState.explorerOpen"
        >
          <AppIcon name="sidebar" :size="17" />
        </button>
        <div
          v-if="activeNote"
          class="note-navigation"
          data-ui-region="note-history"
          role="group"
          aria-label="Note history"
        >
          <button
            class="icon-button subtle"
            type="button"
            data-note-action="navigate-back"
            :disabled="!canNavigateBack"
            :title="backNavigationLabel"
            :aria-label="backNavigationLabel"
            @click="navigateBack"
          >
            <AppIcon name="history-back" :size="15" />
          </button>
          <button
            class="icon-button subtle"
            type="button"
            data-note-action="navigate-forward"
            :disabled="!canNavigateForward"
            :title="forwardNavigationLabel"
            :aria-label="forwardNavigationLabel"
            @click="navigateForward"
          >
            <AppIcon name="history-forward" :size="15" />
          </button>
        </div>
        <Transition name="chip-swap">
          <div v-if="!uiState.explorerOpen" class="vault-hidden-actions">
            <button
              :disabled="!canEditVault"
              type="button"
              class="icon-button subtle"
              aria-label="Create note"
              :title="`Create note · ${createNoteShortcut}`"
              @click="createNote()"
            >
              <AppIcon name="file-plus" :size="15" />
            </button>
            <div class="menu-anchor">
              <button
                ref="quickFolderButton"
                :disabled="!canEditVault"
                type="button"
                class="icon-button subtle"
                aria-label="Create folder"
                title="Create folder"
                :aria-expanded="quickFolderOpen"
                @mousedown.prevent
                @click="quickFolderOpen ? closeQuickFolder( true ) : openQuickFolder()"
              >
                <AppIcon name="folder-plus" :size="15" />
              </button>
              <Transition name="popover-fade">
                <form
                  v-if="quickFolderOpen"
                  class="popover-menu quick-folder-popover"
                  @submit.prevent="submitQuickFolder"
                  @focusout="handleQuickFolderFocusOut"
                  @keydown.esc.prevent="closeQuickFolder( true )"
                >
                  <strong>New folder</strong>
                  <div class="quick-folder-entry">
                    <input
                      ref="quickFolderField"
                      v-model="quickFolderName"
                      type="text"
                      maxlength="120"
                      autocomplete="off"
                      aria-label="Folder name"
                      placeholder="Folder name"
                    >
                    <button
                      type="submit"
                      :disabled="!canEditVault || !quickFolderName.trim()"
                      aria-label="Create folder"
                    >
                      <AppIcon name="arrow" :size="14" />
                    </button>
                  </div>
                </form>
              </Transition>
            </div>
          </div>
        </Transition>
        <template v-if="activeNote">
          <span class="crumb-vault">{{ vaultState.name }}</span>
          <AppIcon name="chevron" :size="12" />
          <span v-if="activeNote.folderId" class="crumb-folder">{{ folderPath( activeNote.folderId ) }}</span>
          <AppIcon
            v-if="activeNote.folderId"
            name="chevron"
            :size="12"
          />
          <span class="crumb-note">{{ activeNote.title || "Untitled note" }}</span>
        </template>
      </div>

      <div v-if="activeNote" class="editor-toolbar-actions">
        <button
          class="icon-button"
          type="button"
          data-note-action="embed-attachment"
          :disabled="!canEditVault || !nativeAvailable || attachmentEmbedBusy || imageEmbedBusy || vaultSession.busy"
          aria-label="Embed file"
          :title="`Embed file · ${embedAttachmentShortcut}`"
          @click="requestAttachmentFromToolbar"
        >
          <AppIcon name="paperclip" :size="16" />
        </button>
        <button
          class="icon-button"
          type="button"
          data-note-action="embed-image"
          :disabled="!canEditVault || !nativeAvailable || imageEmbedBusy || attachmentEmbedBusy || vaultSession.busy"
          aria-label="Embed image"
          :title="`Embed image · ${embedImageShortcut}`"
          @click="requestImageFromToolbar"
        >
          <AppIcon name="image" :size="16" />
        </button>
        <button
          :disabled="!canEditVault"
          class="icon-button"
          type="button"
          :class="{ active: activeNote.pinned }"
          :aria-label="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          :title="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          @click="togglePinned( activeNote.id )"
        >
          <AppIcon name="star" :size="16" />
        </button>
        <button
          v-if="hasFrontmatter || uiState.frontmatterVisible"
          class="icon-button frontmatter-toggle"
          type="button"
          data-note-action="toggle-frontmatter"
          :class="{ active: uiState.frontmatterVisible }"
          :aria-label="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :title="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :aria-pressed="uiState.frontmatterVisible"
          @click="toggleFrontmatter"
        >
          <AppIcon name="code" :size="16" />
        </button>
        <button
          class="icon-button context-toggle"
          type="button"
          :class="{ active: uiState.contextOpen }"
          :aria-label="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          :title="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          @click="uiState.contextOpen = !uiState.contextOpen"
        >
          <AppIcon name="panel-right" :size="17" />
        </button>
        <div class="menu-anchor">
          <button
            class="icon-button"
            type="button"
            title="More actions"
            @click="noteMenuOpen = !noteMenuOpen"
          >
            <AppIcon name="more" :size="18" />
          </button>
          <Transition name="popover-fade">
            <div v-if="noteMenuOpen" class="popover-menu compact-menu">
              <button
                :disabled="!canEditVault"
                type="button"
                class="danger"
                @click="requestDelete"
              >
                <AppIcon name="trash" :size="15" /> Delete note
              </button>
            </div>
          </Transition>
        </div>
      </div>
    </header>

    <template v-if="activeNote">
      <section
        id="note-tab-panel"
        class="editor-document"
        role="tabpanel"
        :aria-labelledby="`note-tab-${activeNoteTab.id}`"
      >
        <div class="document-heading">
          <input
            class="note-title-input"
            data-ui-region="note-title"
            :readonly="!canEditVault"
            :value="activeNote.title"
            aria-label="Note title"
            placeholder="Untitled note"
            @input="setTitle"
            @blur="resetTitle"
          >
          <div class="note-properties">
            <label class="property-control folder-property">
              <AppIcon name="folder" :size="14" />
              <select
                :disabled="!canEditVault"
                :value="activeNote.folderId ?? ''"
                aria-label="Move note to folder"
                @change="setFolder"
              >
                <option value="">Vault root</option>
                <option
                  v-for="folder in sortedFolders"
                  :key="folder.id"
                  :value="folder.id"
                >
                  {{ folderPath( folder.id ) }}
                </option>
              </select>
            </label>

            <span
              v-for="tag in activeNote.tags"
              :key="tag"
              class="tag-chip"
            >
              <span>#</span>{{ tag }}
              <button
                :disabled="!canEditVault"
                type="button"
                :aria-label="`Remove ${tag} tag`"
                @click="removeTag( tag )"
              >
                <AppIcon name="x" :size="10" />
              </button>
            </span>
            <Transition name="chip-swap" mode="out-in">
              <form
                v-if="tagInputOpen"
                key="tag-input"
                class="inline-tag-form"
                @submit.prevent="submitTag"
              >
                <span>#</span>
                <input
                  ref="tagField"
                  v-model="tagInput"
                  placeholder="tag"
                  autocomplete="off"
                  autocapitalize="none"
                  autocorrect="off"
                  spellcheck="false"
                  role="combobox"
                  aria-autocomplete="list"
                  aria-controls="tag-suggestions"
                  :aria-expanded="tagSuggestions.length > 0"
                  :aria-activedescendant="tagSuggestionIndex >= 0 ? `tag-suggestion-${tagSuggestionIndex}` : undefined"
                  @blur="addTag()"
                  @keydown="handleTagKeydown"
                  @keydown.esc.prevent="cancelTagInput"
                >
                <div
                  v-if="tagSuggestions.length"
                  id="tag-suggestions"
                  class="tag-suggestions"
                  role="listbox"
                >
                  <button
                    v-for="( tag, index ) in tagSuggestions"
                    :id="`tag-suggestion-${index}`"
                    :key="tag"
                    :disabled="!canEditVault"
                    type="button"
                    role="option"
                    :aria-selected="index === tagSuggestionIndex"
                    :class="{ active: index === tagSuggestionIndex }"
                    @mouseenter="tagSuggestionIndex = index"
                    @mousedown.prevent="addTag( tag )"
                  >
                    <span>#</span>{{ tag }}
                  </button>
                </div>
              </form>
              <button
                v-else
                key="tag-button"
                :disabled="!canEditVault"
                type="button"
                class="add-tag-button"
                @click="openTagInput"
              >
                <AppIcon name="plus" :size="12" /> Add tag
              </button>
            </Transition>
          </div>
        </div>

        <div class="editor-canvas" data-editor-pane="live">
          <SourceEditor
            :key="editorKey"
            ref="sourceEditor"
            :tab-id="activeNoteTab.id"
            :focus-on-mount="!keepTabFocus && uiState.noteTitleFocusId !== activeNote.id"
            :initial-position="savedEditorPosition( activeNote.id, activeNote.content )"
            :attachment-files="vaultState.attachmentFiles"
            :attachment-refresh-token="uiState.attachmentRefreshToken"
            :embedded-attachments="vaultState.embeddedAttachments"
            :embedded-images="vaultState.embeddedImages"
            :image-refresh-token="uiState.imageRefreshToken"
            :model-value="activeNote.content"
            :note-id="activeNote.id"
            :note-relative-path="activeNote.relativePath"
            :note-link-targets="noteLinkTargets"
            :note-link-headings="noteLinkHeadings"
            :note-link-for-id="noteLinkForId"
            :wiki-link-is-resolved="wikiLinkIsResolved"
            :rename-attachment="renameVaultAttachment"
            :read-only="!canEditVault"
            :show-frontmatter="uiState.frontmatterVisible"
            :vault-id="positionVaultId"
            :vault-path="vaultSession.path"
            @editor-position="rememberEditorPosition"
            @activate-attachment="activateEmbeddedAttachment"
            @external-file-drop="embedExternalFiles"
            @open-link="openRenderedLink"
            @open-wiki="openWikiLink"
            @paste-image="embedImageFromClipboard"
            @reveal-attachment-in-tree="revealEmbeddedAttachmentInTree"
            @request-embed-attachment="embedAttachmentFromFile"
            @vault-attachment-drop="embedAttachmentFromVault"
            @vault-image-drop="embedImageFromVault"
            @request-embed-image="embedImageFromFile"
            @show-attachment-in-folder="showEmbeddedAttachmentInFolder"
            @update:model-value="setContent"
          />
        </div>
      </section>

      <footer class="editor-statusbar">
        <div>
          <span class="status-dot" :class="uiState.saveStatus" />
          <span v-if="!canEditVault">Read only</span>
          <span v-else-if="uiState.saveStatus === 'saving'">Saving…</span>
          <span v-else-if="uiState.saveStatus === 'error'">Couldn’t save</span>
          <span v-else>Saved</span>
        </div>
        <div class="status-stats">
          <span>Ln {{ activeNote.content.slice( 0, activeNote.content.length ).split( '\n' ).length }}</span>
          <span>{{ wordCount }} words</span>
          <span>{{ characterCount.toLocaleString() }} characters</span>
          <span>Markdown</span>
        </div>
      </footer>
    </template>

    <div
      v-else
      id="note-tab-panel"
      class="empty-editor"
      role="tabpanel"
      :aria-labelledby="`note-tab-${activeNoteTab.id}`"
    >
      <div class="empty-glyph">
        <AppIcon name="file-plus" :size="28" />
      </div>
      <h2>No note selected</h2>
      <p>{{ canEditVault ? "Select a note or create a new one." : "Select a note to read." }}</p>
      <div class="empty-tab-actions">
        <button
          ref="emptyTabOpenButton"
          class="secondary-button"
          type="button"
          :title="`Open a note · ${openNoteShortcut}`"
          @click="openQuickSearch()"
        >
          <AppIcon name="search" :size="15" /> Open a note
        </button>
        <button
          v-if="canEditVault"
          class="primary-action-button"
          type="button"
          :title="`Create a note · ${createNoteShortcut}`"
          @click="createNote()"
        >
          <AppIcon name="file-plus" :size="15" /> Create a note
        </button>
      </div>
    </div>
  </main>
</template>
