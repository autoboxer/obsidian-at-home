<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import { defaultKeymap, history, historyField, historyKeymap } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { Annotation, Compartment, EditorSelection, EditorState, Prec, Transaction } from '@codemirror/state';
import { drawSelection, dropCursor, EditorView, highlightSpecialChars, keymap } from '@codemirror/view';
import {
  useCodeMirrorAssets,
  type CodeMirrorAssetEvents
} from '../composables/useCodeMirrorAssets';
import {
  codeMirrorDocumentSearchExtension,
  useCodeMirrorDocumentSearch
} from '../composables/useCodeMirrorDocumentSearch';
import { useCodeMirrorViewport } from '../composables/useCodeMirrorViewport';
import {
  insertLiteralApostrophe,
  insertLiteralDoubleQuote,
  literalApostropheExtension
} from '../lib/codeMirrorApostrophe';
import { tableDelimiterHyphenExtension } from '../lib/codeMirrorTableDelimiter';
import { codeMirrorMultiCursorExtension } from '../lib/codeMirrorMultiCursor';
import {
  handleEnter,
  handleShiftEnter,
  handleTab,
  handleShiftTab,
  handleTableArrowDown,
  handleTableArrowUp,
  deleteTableBackward,
  deleteTableForward,
  deleteTableWordBackward,
  deleteTableWordForward,
  deleteToTableCellTextStart,
  deleteToTableCellTextEnd,
  moveAcrossTableCellLeft,
  moveAcrossTableCellRight,
  protectTableCellSelectionLeft,
  protectTableCellSelectionRight,
  moveToTableCellTextStart,
  selectToTableCellTextStart,
  moveToTableCellTextEnd,
  selectToTableCellTextEnd,
  moveToTableCellTextLeft,
  selectToTableCellTextLeft,
  moveToTableCellTextRight,
  selectToTableCellTextRight,
  toggleBold,
  toggleItalic,
  toggleStrikethrough,
  insertLiteralHyphen,
  wrapSelectionAsInlineCode,
  wrapSelectionAsMarkdownLink
} from '../lib/codeMirrorEditingCommands';
import {
  minimalDocumentChange,
  preferredLineEnding,
  normalizeDocumentText,
  projectEditableDocument,
  restorableEditorHistory,
  frontmatterVisibilitySelection,
  changeTouchesLeadingFrontmatter,
  normalizeInitialPosition,
  restoreLineEndings,
  editorLineNumbers
} from '../lib/codeMirrorDocument';
import {
  liveMarkdownExtension,
  orderedListRenumberingExtension,
  refreshLiveMarkdownEffect
} from '../lib/liveMarkdownCodeMirror';
import { joinLeadingFrontmatter, markdownBodyStart } from '../lib/frontmatter';
import { openNoteEditorHistory } from '../stores/editorHistories';
import type { Extension } from '@codemirror/state';
import type { Command } from '@codemirror/view';
import type { MarkdownAttachmentRenameTarget } from '../lib/markdownAttachments';
import type { EmbeddedAttachment, EmbeddedImage, NoteEditorPosition, VaultAttachmentFile } from '../types';
import AppIcon from './AppIcon.vue';

const props = defineProps<{
  initialPosition?: NoteEditorPosition;
  attachmentFiles: VaultAttachmentFile[];
  attachmentRefreshToken: number;
  embeddedAttachments: EmbeddedAttachment[];
  embeddedImages: EmbeddedImage[];
  imageRefreshToken: number;
  modelValue: string;
  noteId: string;
  noteRelativePath: string;
  noteLinkTargets: string[];
  noteLinkForId: ( noteId: string ) => string | undefined;
  wikiLinkIsResolved: ( target: string ) => boolean;
  renameAttachment: (
    target: MarkdownAttachmentRenameTarget,
    fileName: string
  ) => Promise<boolean>;
  readOnly: boolean;
  showFrontmatter: boolean;
  vaultId: string;
  vaultPath: string | null;
}>();

const emit = defineEmits<CodeMirrorAssetEvents & {
  editorPosition: [vaultId: string, noteId: string, position: NoteEditorPosition];
  openLink: [href: string];
  openWiki: [target: string, heading?: string];
  'update:modelValue': [value: string];
}>();

const editorHost = ref<HTMLElement>();
const editorView = shallowRef<EditorView>();
const suggestionIndex = ref( 0 );
const suggestionQuery = ref<string | null>( null );
const externalUpdate = Annotation.define<boolean>();
const historyCompartment = new Compartment();
const lineNumbersCompartment = new Compartment();
let outputLineEnding = preferredLineEnding( props.modelValue );
let frontmatterHistoryChanged = false;
let frontmatterLineOffset = 0;
let frontmatterPrefix = '';
let closeEditorHistory: ReturnType<typeof openNoteEditorHistory>[ 'close' ] | undefined;

const {
  disposeEditorPosition,
  editorRenderReady,
  focusDocumentOffset,
  registerEditorPosition,
  restoreEditorPosition,
  scheduleEditorRenderReady,
  schedulePositionCapture
} = useCodeMirrorViewport( props, emit, editorView, () => frontmatterPrefix );

const {
  activateLiveMarkdownAttachment,
  blockUnavailableEditorDrag,
  cancelAttachmentInsertion,
  cancelImageInsertion,
  captureAttachmentInsertion,
  captureImageInsertion,
  clearExternalFileDrag,
  clearLiveMarkdownImageSources,
  disposeAssets,
  externalFileDragActive,
  handleSourceEditorDragEnter,
  handleSourceEditorDragLeave,
  handleSourceEditorDragOver,
  handleSourceEditorDrop,
  handleSourceEditorPaste,
  insertEmbeddedAttachment,
  insertEmbeddedImage,
  isKnownExtensionlessAttachment,
  mapPendingImageInsertions,
  requestAttachmentEmbed,
  requestImageEmbed,
  resolveLiveMarkdownAttachmentMetadata,
  resolveLiveMarkdownImageSource,
  revealLiveMarkdownAttachmentInTree,
  showLiveMarkdownAttachmentInFolder
} = useCodeMirrorAssets( props, emit, editorView, editorRenderReady, blockPendingEditorInteraction );

const suggestions = computed( () => {
  if ( suggestionQuery.value === null ) {
    return [];
  }
  const query = suggestionQuery.value.toLocaleLowerCase();

  return props.noteLinkTargets
    .filter( ( title ) => !query || title.toLocaleLowerCase().includes( query ) )
    .sort( ( a, b ) => {
      const aStarts = a.toLocaleLowerCase().startsWith( query );
      const bStarts = b.toLocaleLowerCase().startsWith( query );

      return Number( bStarts ) - Number( aStarts ) || a.localeCompare( b );
    })
    .slice( 0, 6 );
});

const {
  closeSearch: closeDocumentSearch,
  handleEditorSearchKeydown: handleDocumentSearchKeydown,
  handleSearchInputKeydown: handleDocumentSearchInputKeydown,
  isOpen: documentSearchOpen,
  matchCount: documentSearchMatchCount,
  moveToMatch: moveToDocumentSearchMatch,
  openSearch: openDocumentSearch,
  query: documentSearchQuery,
  refreshSearch: refreshDocumentSearch,
  searchInput: documentSearchInput,
  statusText: documentSearchStatus
} = useCodeMirrorDocumentSearch( editorView );

const suggestionDown: Command = ( view ) => {
  if ( view.composing || !suggestions.value.length ) {
    return false;
  }
  suggestionIndex.value = ( suggestionIndex.value + 1 ) % suggestions.value.length;

  return true;
};

const suggestionUp: Command = ( view ) => {
  if ( view.composing || !suggestions.value.length ) {
    return false;
  }
  suggestionIndex.value = (
    suggestionIndex.value - 1 + suggestions.value.length
  ) % suggestions.value.length;

  return true;
};

const acceptSuggestion: Command = ( view ) => {
  if ( view.composing || !suggestions.value.length ) {
    return false;
  }

  insertSuggestion( suggestions.value[ suggestionIndex.value ]! );

  return true;
};

const closeSuggestions: Command = ( view ) => {
  if ( view.composing || suggestionQuery.value === null ) {
    return false;
  }
  suggestionQuery.value = null;

  return true;
};

onMounted( () => {
  const host = editorHost.value;
  if ( !host ) {
    return;
  }

  const editableDocument = projectEditableDocument(
    normalizeDocumentText( props.modelValue ),
    props.showFrontmatter
  );
  frontmatterLineOffset = editableDocument.lineNumberOffset;
  frontmatterPrefix = editableDocument.prefix;
  const initialPosition = normalizeInitialPosition(
    props.initialPosition,
    editableDocument.body.length,
    editableDocument.bodyStart
  );
  const historySession = openNoteEditorHistory(
    props.vaultId,
    props.noteId,
    () => {
      frontmatterHistoryChanged = false;
      const view = editorView.value;
      if ( view ) {
        view.dispatch({ effects: historyCompartment.reconfigure([]) });
        view.dispatch({ effects: historyCompartment.reconfigure( history() ) });
      }
    }
  );
  closeEditorHistory = historySession.close;
  const savedState = restorableEditorHistory(
    historySession.snapshot,
    normalizeDocumentText( props.modelValue ),
    editableDocument
  );
  if ( historySession.snapshot && !savedState ) {
    historySession.discard();
  }
  // Keep the restored field inside the compartment so it can actually be reset.
  // fromJSON with the full editor extensions would install another copy outside it.
  const restoredState = savedState
    ? EditorState.fromJSON( savedState, {}, { history: historyField })
    : undefined;
  const extensions: Extension[] = [
    EditorState.readOnly.of( props.readOnly ),
    EditorView.editable.of( !props.readOnly ),
    // Hydration and frontmatter visibility may replace the projected document.
    EditorState.transactionFilter.of( ( transaction ) => (
      props.readOnly && transaction.docChanged && !transaction.annotation( externalUpdate )
        ? []
        : transaction
    ) ),
    lineNumbersCompartment.of( editorLineNumbers( frontmatterLineOffset ) ),
    highlightSpecialChars(),
    historyCompartment.of([
      history(),
      ...( restoredState
        ? [ historyField.init( () => restoredState.field( historyField ) ) ]
        : [])
    ]),
    drawSelection(),
    dropCursor(),
    // Let the drop-cursor observers run before a custom drop consumes the event.
    EditorView.domEventHandlers({
      dragover: ( event ) => {
        handleSourceEditorDragOver( event );

        return event.defaultPrevented;
      },
      drop: ( event ) => {
        handleSourceEditorDrop( event );

        return event.defaultPrevented;
      }
    }),
    codeMirrorMultiCursorExtension,
    EditorState.tabSize.of( 4 ),
    EditorView.lineWrapping,
    EditorView.contentAttributes.of({
      'aria-label': 'Markdown source',
      'aria-readonly': String( props.readOnly ),
      tabindex: '0',
      autocapitalize: 'off',
      autocorrect: 'off',
      class: 'source-textarea',
      spellcheck: 'true',
      writingsuggestions: 'false'
    }),
    markdown({
      addKeymap: false,
      base: markdownLanguage,
      completeHTMLTags: false,
      pasteURLAsLink: false
    }),
    literalApostropheExtension,
    tableDelimiterHyphenExtension,
    orderedListRenumberingExtension( ( transaction ) => Boolean( transaction.annotation( externalUpdate ) ) ),
    liveMarkdownExtension({
      acceptExtensionlessAttachment: isKnownExtensionlessAttachment,
      activateAttachment: activateLiveMarkdownAttachment,
      documentId: `${ props.vaultId }\u0000${ props.noteId }`,
      openLink: openLiveMarkdownLink,
      openWiki: openLiveMarkdownWikiLink,
      renameAttachment: ( target, fileName ) => props.readOnly
        ? Promise.resolve( false )
        : props.renameAttachment( target, fileName ),
      revealAttachmentInTree: revealLiveMarkdownAttachmentInTree,
      resolveAttachmentMetadata: resolveLiveMarkdownAttachmentMetadata,
      resolveImageSource: resolveLiveMarkdownImageSource,
      showAttachmentInFolder: showLiveMarkdownAttachmentInFolder,
      wikiLinkIsResolved: ( target ) => props.wikiLinkIsResolved( target )
    }),
    codeMirrorDocumentSearchExtension,
    Prec.high( keymap.of([
      { key: 'ArrowDown', run: suggestionDown },
      { key: 'ArrowUp', run: suggestionUp },
      { key: 'ArrowDown', run: handleTableArrowDown },
      { key: 'ArrowUp', run: handleTableArrowUp },
      { key: 'Enter', run: acceptSuggestion },
      { key: 'Escape', run: closeSuggestions },
      { key: 'Shift-Enter', run: handleShiftEnter },
      { key: 'Enter', run: handleEnter },
      { key: 'Tab', run: handleTab },
      { key: 'Shift-Tab', run: handleShiftTab },
      { key: 'Backspace', run: deleteTableBackward, shift: deleteTableBackward },
      { key: 'Delete', run: deleteTableForward },
      {
        key: 'Mod-Backspace',
        mac: 'Alt-Backspace',
        run: deleteTableWordBackward
      },
      {
        key: 'Mod-Delete',
        mac: 'Alt-Delete',
        run: deleteTableWordForward
      },
      { mac: 'Mod-Backspace', run: deleteToTableCellTextStart },
      { mac: 'Mod-Delete', run: deleteToTableCellTextEnd },
      {
        key: 'ArrowLeft',
        run: moveAcrossTableCellLeft,
        shift: protectTableCellSelectionLeft
      },
      {
        key: 'ArrowRight',
        run: moveAcrossTableCellRight,
        shift: protectTableCellSelectionRight
      },
      {
        key: 'Home',
        run: moveToTableCellTextStart,
        shift: selectToTableCellTextStart
      },
      {
        key: 'End',
        run: moveToTableCellTextEnd,
        shift: selectToTableCellTextEnd
      },
      {
        mac: 'Cmd-ArrowLeft',
        run: moveToTableCellTextLeft,
        shift: selectToTableCellTextLeft
      },
      {
        mac: 'Cmd-ArrowRight',
        run: moveToTableCellTextRight,
        shift: selectToTableCellTextRight
      },
      { key: 'Mod-b', run: toggleBold },
      { key: 'Mod-Shift-a', run: requestAttachmentEmbed },
      { key: 'Mod-Shift-i', run: requestImageEmbed },
      { key: 'Mod-i', run: toggleItalic },
      { key: 'Mod-k', run: wrapSelectionAsMarkdownLink },
      { key: 'Mod-Shift-x', run: toggleStrikethrough },
      { key: "'", run: insertLiteralApostrophe },
      { key: '"', run: insertLiteralDoubleQuote },
      { key: '-', run: insertLiteralHyphen },
      { key: '`', run: wrapSelectionAsInlineCode }
    ]) ),
    keymap.of([ ...defaultKeymap, ...historyKeymap ]),
    EditorView.updateListener.of( ( update ) => {
      if ( update.docChanged ) {
        mapPendingImageInsertions( update );
      }
      const localDocumentChange = update.docChanged && update.transactions.some(
        ( transaction ) => transaction.docChanged && !transaction.annotation( externalUpdate )
      );
      if ( localDocumentChange ) {
        if (
          props.showFrontmatter
          && changeTouchesLeadingFrontmatter( update )
        ) {
          frontmatterHistoryChanged = true;
        }
        emit(
          'update:modelValue',
          restoreLineEndings(
            joinLeadingFrontmatter(
              frontmatterPrefix,
              update.state.doc.toString()
            ),
            outputLineEnding
          )
        );
        refreshDocumentSearch();
      }
      if ( update.docChanged || update.selectionSet ) {
        updateSuggestions( update.view );
        schedulePositionCapture( update.view );
      }
      scheduleEditorRenderReady( update.view );
    })
  ];
  let state = EditorState.create({
    doc: restoredState?.doc ?? editableDocument.body,
    selection: restoredState?.selection ?? ( initialPosition
      ? EditorSelection.range( initialPosition.selection.anchor, initialPosition.selection.head )
      : undefined ),
    extensions
  });
  if ( savedState && savedState.doc !== editableDocument.body ) {
    const currentBodyStart = markdownBodyStart( savedState.prefix, savedState.doc );
    state = state.update({
      annotations: [
        externalUpdate.of( true ),
        Transaction.addToHistory.of( false )
      ],
      changes: minimalDocumentChange( savedState.doc, editableDocument.body ),
      selection: frontmatterVisibilitySelection(
        state.selection,
        currentBodyStart,
        editableDocument
      )
    }).state;
  }
  frontmatterHistoryChanged = savedState?.frontmatterHistoryChanged ?? false;
  const view = new EditorView({
    parent: host,
    state,
    scrollTo: initialPosition
      ? EditorView.scrollIntoView( initialPosition.viewport.anchor, { x: 'start', y: 'start' })
      : undefined
  });
  editorView.value = view;
  registerEditorPosition( view );
  updateSuggestions( view );
  view.focus();

  restoreEditorPosition( view, initialPosition );
  window.requestAnimationFrame( () => {
    if ( editorView.value !== view || !view.dom.isConnected ) {
      return;
    }

    const scrollLeft = view.scrollDOM.scrollLeft;
    const scrollTop = view.scrollDOM.scrollTop;
    view.focus();
    view.scrollDOM.scrollLeft = scrollLeft;
    view.scrollDOM.scrollTop = scrollTop;
  });
});

onBeforeUnmount( () => {
  disposeAssets();
  const view = editorView.value;
  disposeEditorPosition();
  if ( view ) {
    const serializedState = view.state.toJSON({ history: historyField });
    closeEditorHistory?.({
      doc: serializedState.doc,
      frontmatterHistoryChanged,
      history: serializedState.history,
      prefix: frontmatterPrefix,
      selection: serializedState.selection
    });
    closeEditorHistory = undefined;
    view.destroy();
  }
  editorView.value = undefined;
});

watch(
  [ () => props.modelValue, () => props.showFrontmatter ],
  ([ value, showFrontmatter ], [ , previouslyShowingFrontmatter ]) => {
    const view = editorView.value;
    outputLineEnding = preferredLineEnding( value );
    const visibilityChanged = showFrontmatter !== previouslyShowingFrontmatter;
    const normalizedValue = visibilityChanged && view
      ? joinLeadingFrontmatter(
        frontmatterPrefix,
        view.state.doc.toString()
      )
      : normalizeDocumentText( value );

    const editableDocument = projectEditableDocument(
      normalizedValue,
      showFrontmatter
    );
    const prefixChanged = editableDocument.prefix !== frontmatterPrefix;
    const lineOffsetChanged = (
      editableDocument.lineNumberOffset !== frontmatterLineOffset
    );
    if ( !view ) {
      frontmatterPrefix = editableDocument.prefix;
      frontmatterLineOffset = editableDocument.lineNumberOffset;

      return;
    }

    const currentBody = view.state.doc.toString();
    const currentBodyStart = markdownBodyStart( frontmatterPrefix, currentBody );
    const bodyChanged = editableDocument.body !== currentBody;
    const coordinateSpaceChanged = bodyChanged
      && editableDocument.bodyStart !== currentBodyStart;
    const resetHistory = coordinateSpaceChanged && (
      !visibilityChanged
      || ( previouslyShowingFrontmatter && frontmatterHistoryChanged )
    );
    const selection = visibilityChanged
      ? frontmatterVisibilitySelection(
        view.state.selection,
        currentBodyStart,
        editableDocument
      )
      : undefined;
    frontmatterPrefix = editableDocument.prefix;
    frontmatterLineOffset = editableDocument.lineNumberOffset;
    if ( resetHistory ) {
      // Projection changes invalidate history entries that target the hidden prefix
      view.dispatch({
        effects: historyCompartment.reconfigure([])
      });
    }
    if ( bodyChanged || lineOffsetChanged ) {
      view.dispatch({
        ...( bodyChanged
          ? {
            changes: minimalDocumentChange( currentBody, editableDocument.body ),
            annotations: [
              externalUpdate.of( true ),
              Transaction.addToHistory.of( false )
            ]
          }
          : {}),
        ...( lineOffsetChanged
          ? {
            effects: lineNumbersCompartment.reconfigure(
              editorLineNumbers( frontmatterLineOffset )
            )
          }
          : {}),
        ...( selection ? { selection } : {}),
        scrollIntoView: visibilityChanged
      });
    }
    if ( bodyChanged ) {
      refreshDocumentSearch();
    }
    if ( prefixChanged ) {
      schedulePositionCapture( view );
    }
    if ( resetHistory ) {
      view.dispatch({
        effects: historyCompartment.reconfigure( history() )
      });
    }
    if ( visibilityChanged ) {
      frontmatterHistoryChanged = false;
      view.focus();
    }
  }
);

watch(
  () => props.noteLinkTargets,
  () => {
    editorView.value?.dispatch({
      effects: refreshLiveMarkdownEffect.of( null )
    });
  },
  { deep: true }
);

watch(
  () => props.imageRefreshToken,
  () => {
    clearLiveMarkdownImageSources();
    editorView.value?.dispatch({
      effects: refreshLiveMarkdownEffect.of( null )
    });
  }
);

watch(
  () => props.attachmentRefreshToken,
  () => {
    editorView.value?.dispatch({
      effects: refreshLiveMarkdownEffect.of( null )
    });
  }
);

function openLiveMarkdownLink( href: string ): void {
  emit( 'openLink', href );
}

function openLiveMarkdownWikiLink( target: string, heading?: string ): void {
  emit( 'openWiki', target, heading );
}

defineExpose({
  cancelAttachmentInsertion,
  cancelImageInsertion,
  captureAttachmentInsertion,
  captureImageInsertion,
  focusDocumentOffset,
  insertEmbeddedAttachment,
  insertEmbeddedImage
});

function updateSuggestions( view: EditorView ): void {
  const selection = view.state.selection.main;
  if ( props.readOnly || view.state.selection.ranges.length !== 1 || !selection.empty ) {
    suggestionQuery.value = null;

    return;
  }

  const line = view.state.doc.lineAt( selection.head );
  const beforeCursor = view.state.doc.sliceString( line.from, selection.head );
  const match = beforeCursor.match( /\[\[([^\]\n|#]*)$/ );
  suggestionQuery.value = match ? match[ 1 ] ?? '' : null;
  suggestionIndex.value = 0;
}

function insertSuggestion( title: string ): void {
  const view = editorView.value;
  if ( props.readOnly || !view || view.state.selection.ranges.length !== 1 || suggestionQuery.value === null ) {
    return;
  }

  const cursor = view.state.selection.main.head;
  const start = cursor - suggestionQuery.value.length;
  const replacement = `${ title }]]`;
  suggestionQuery.value = null;
  view.dispatch({
    changes: { from: start, to: cursor, insert: replacement },
    selection: EditorSelection.cursor( start + replacement.length ),
    scrollIntoView: true,
    userEvent: 'input.complete'
  });
  view.focus();
}

function blockPendingEditorInteraction( event: Event ): void {
  if ( !editorRenderReady.value || ( props.readOnly && event.type !== 'keydown' ) ) {
    event.preventDefault();
    event.stopImmediatePropagation();
  }
}

function handleSourceEditorKeydown( event: KeyboardEvent ): void {
  if ( !editorRenderReady.value ) {
    blockPendingEditorInteraction( event );

    return;
  }

  handleDocumentSearchKeydown( event );
}
</script>

<template>
  <div
    class="source-editor"
    :class="{
      'is-external-file-dragging': externalFileDragActive,
      'is-render-pending': !editorRenderReady,
      'is-searching': documentSearchOpen,
    }"
    :aria-busy="!editorRenderReady"
    @beforeinput.capture="blockPendingEditorInteraction"
    @compositionstart.capture="blockPendingEditorInteraction"
    @keydown.capture="handleSourceEditorKeydown"
    @paste.capture="handleSourceEditorPaste"
    @dragenter.capture="handleSourceEditorDragEnter"
    @dragleave.capture="handleSourceEditorDragLeave"
    @dragover.capture="blockUnavailableEditorDrag"
    @dragover="handleSourceEditorDragOver"
    @dragend.capture="clearExternalFileDrag"
    @drop.capture="blockUnavailableEditorDrag"
    @drop="handleSourceEditorDrop"
  >
    <div ref="editorHost" class="code-mirror-host" />

    <Transition name="popover-fade">
      <form
        v-if="documentSearchOpen"
        class="document-search-bar"
        data-ui-region="document-search"
        role="search"
        aria-label="Find in note"
        @submit.prevent="moveToDocumentSearchMatch( 'next' )"
      >
        <label class="document-search-field">
          <AppIcon name="search" :size="14" />
          <input
            ref="documentSearchInput"
            v-model="documentSearchQuery"
            type="search"
            placeholder="Find in note"
            aria-label="Find in note"
            aria-describedby="document-search-status"
            autocomplete="off"
            autocapitalize="none"
            spellcheck="false"
            @keydown="handleDocumentSearchInputKeydown"
          >
        </label>
        <span
          id="document-search-status"
          class="document-search-status"
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >{{ documentSearchStatus }}</span>
        <button
          type="button"
          class="document-search-button previous"
          :disabled="!documentSearchMatchCount"
          aria-label="Previous match"
          aria-keyshortcuts="Shift+Tab Shift+F3"
          title="Previous match (Shift+Tab)"
          @mousedown.prevent
          @click="moveToDocumentSearchMatch( 'previous' )"
        >
          <AppIcon name="chevron-down" :size="13" />
        </button>
        <button
          type="button"
          class="document-search-button"
          :disabled="!documentSearchMatchCount"
          aria-label="Next match"
          aria-keyshortcuts="Tab F3"
          title="Next match (Tab)"
          @mousedown.prevent
          @click="moveToDocumentSearchMatch( 'next' )"
        >
          <AppIcon name="chevron-down" :size="13" />
        </button>
        <button
          type="button"
          class="document-search-button close"
          aria-label="Close find in note"
          title="Close (Escape)"
          @mousedown.prevent
          @click="closeDocumentSearch"
        >
          <AppIcon name="x" :size="13" />
        </button>
      </form>

      <button
        v-else
        type="button"
        class="document-search-trigger"
        aria-label="Find in note"
        aria-keyshortcuts="Control+F Meta+F"
        title="Find in note"
        @click="openDocumentSearch"
      >
        <AppIcon name="search" :size="14" />
      </button>
    </Transition>

    <Transition name="popover-fade">
      <div
        v-if="suggestions.length"
        class="wiki-suggestions"
        role="listbox"
      >
        <div class="suggestion-kicker">
          Link a note
        </div>
        <button
          v-for="( title, index ) in suggestions"
          :key="title"
          type="button"
          class="wiki-suggestion"
          :class="{ active: index === suggestionIndex }"
          @mousedown.prevent="insertSuggestion( title )"
        >
          <span class="suggestion-icon"><AppIcon name="link" :size="14" /></span>
          <span>{{ title }}</span>
          <AppIcon
            v-if="index === suggestionIndex"
            name="enter"
            :size="13"
          />
        </button>
      </div>
    </Transition>

    <div class="editor-language-pill">
      MD
    </div>
  </div>
</template>
