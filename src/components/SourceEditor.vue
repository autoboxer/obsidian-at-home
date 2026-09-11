<script setup lang="ts">
import {
  computed,
  onBeforeUnmount,
  onMounted,
  ref,
  shallowRef,
  watch
} from 'vue';
import {
  defaultKeymap,
  history,
  historyField,
  historyKeymap,
  isolateHistory
} from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import {
  getIndentation,
  getIndentUnit,
  IndentContext,
  indentString,
  syntaxTree,
  syntaxTreeAvailable
} from '@codemirror/language';
import { NodeProp } from '@lezer/common';
import {
  Annotation,
  Compartment,
  countColumn,
  EditorSelection,
  EditorState,
  findClusterBreak,
  Prec,
  Transaction
} from '@codemirror/state';
import {
  Direction,
  drawSelection,
  dropCursor,
  EditorView,
  highlightSpecialChars,
  keymap,
  lineNumbers
} from '@codemirror/view';
import {
  codeMirrorDocumentSearchExtension,
  useCodeMirrorDocumentSearch
} from '../composables/useCodeMirrorDocumentSearch';
import {
  insertLiteralApostrophe,
  insertLiteralDoubleQuote,
  literalApostropheExtension
} from '../lib/codeMirrorApostrophe';
import { tableDelimiterHyphenExtension } from '../lib/codeMirrorTableDelimiter';
import {
  codeMirrorMultiCursorExtension,
  multiCursorChanges,
  type MultiCursorEdit
} from '../lib/codeMirrorMultiCursor';
import { notify } from '../stores/vault';
import {
  liveMarkdownExtension,
  orderedListRenumberingExtension,
  refreshLiveMarkdownEffect
} from '../lib/liveMarkdownCodeMirror';
import { liveMarkdownDocumentModel } from '../lib/liveMarkdownDocumentModel';
import {
  joinLeadingFrontmatter,
  leadingFrontmatterEnd,
  markdownBodyStart,
  splitLeadingFrontmatter
} from '../lib/frontmatter';
import { registerNoteEditorPositionCapture } from '../stores/editorPositions';
import {
  openNoteEditorHistory,
  type NoteEditorHistorySnapshot
} from '../stores/editorHistories';
import {
  deleteEmptyLiveMarkdownTableRow,
  insertLiveMarkdownTableLineBreak,
  insertLiveMarkdownTableRow,
  isLiveMarkdownTableCellBoundary,
  liveMarkdownTableCellTextBounds,
  moveAcrossLiveMarkdownTableCellBoundary,
  navigateLiveMarkdownTable,
  type LiveMarkdownTableNavigation
} from '../lib/liveMarkdownTableNavigation';
import {
  toggleInlineFormatting,
  wrapInlineCode,
  wrapMarkdownLink
} from '../lib/markdownFormatting';
import {
  decodeMarkdownImageDestination,
  imageMediaTypeForPath,
  NOTE_IMAGE_DRAG_MIME,
  resolveMarkdownImagePath,
  VAULT_IMAGE_DRAG_MIME
} from '../lib/imageEmbeds';
import { sanitizeImageUrl } from '../lib/markdown';
import type {
  MarkdownAttachmentMetadata,
  MarkdownAttachmentRenameTarget,
  ParsedMarkdownAttachment
} from '../lib/markdownAttachments';
import { VAULT_ATTACHMENT_DRAG_MIME } from '../lib/markdownAttachments';
import { parseMarkdownImageAt } from '../lib/markdownImages';
import { isTauri, readWorkspaceImage } from '../services/native';
import type { Extension, SelectionRange } from '@codemirror/state';
import type { Command, ViewUpdate } from '@codemirror/view';
import type { LiveMarkdownTextEdit } from '../lib/liveMarkdown';
import type { MarkdownSelectionEdit } from '../lib/markdownFormatting';
import type { ParsedMarkdownImage } from '../lib/markdownImages';
import type {
  AttachmentInsertionCapture,
  EmbeddedAttachment,
  EmbeddedImage,
  ImageInsertionCapture,
  NoteEditorPosition,
  VaultAttachmentFile
} from '../types';
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

const emit = defineEmits<{
  activateAttachment: [
    assetId: string | undefined,
    relativePath: string,
    mediaType: string | undefined,
    openingDisabled: boolean | undefined
  ];
  editorPosition: [vaultId: string, noteId: string, position: NoteEditorPosition];
  externalFileDrop: [
    capture: AttachmentInsertionCapture,
    files: File[],
    rejectedCount: number
  ];
  openLink: [href: string];
  openWiki: [target: string, heading?: string];
  pasteImage: [capture: ImageInsertionCapture, file?: File];
  revealAttachmentInTree: [assetId: string | undefined, relativePath: string];
  requestEmbedAttachment: [capture: AttachmentInsertionCapture];
  requestEmbedImage: [capture: ImageInsertionCapture];
  showAttachmentInFolder: [assetId: string | undefined, relativePath: string];
  vaultImageDrop: [capture: ImageInsertionCapture, relativePath: string];
  vaultAttachmentDrop: [capture: AttachmentInsertionCapture, relativePath: string];
  'update:modelValue': [value: string];
}>();

const editorHost = ref<HTMLElement>();
const editorView = shallowRef<EditorView>();
const editorRenderReady = ref( false );
const externalFileDragActive = ref( false );
const suggestionIndex = ref( 0 );
const suggestionQuery = ref<string | null>( null );
const externalUpdate = Annotation.define<boolean>();
const historyCompartment = new Compartment();
const lineNumbersCompartment = new Compartment();
const INDENT = '  ';
const VIEWPORT_ANCHOR_MARGIN = 8;
const VIRTUALIZED_VIEWPORT_THRESHOLD = 200;
const MAX_EXTERNAL_DROP_FILES = 100;
let externalFileDragDepth = 0;
let outputLineEnding = preferredLineEnding( props.modelValue );
let frontmatterHistoryChanged = false;
let frontmatterLineOffset = 0;
let frontmatterPrefix = '';
let positionCaptureEnabled = false;
let removePositionCapture: ( () => boolean ) | undefined;
let closeEditorHistory: ReturnType<typeof openNoteEditorHistory>[ 'close' ] | undefined;
let viewportRestoreFrame: number | undefined;
let imageInsertionSequence = 0;
let imageResolverDisposed = false;
let imageResolverGeneration = 0;

interface PendingImageInsertion extends ImageInsertionCapture {
  from: number;
  to: number;
}

const pendingImageInsertions = new Map<string, PendingImageInsertion>();
const imageSourcePromises = new Map<string, Promise<string>>();
const imageObjectUrls = new Set<string>();

const positionCaptureKey = {};
const renderReadyKey = {};
const viewportRestoreKey = {};

interface ListLine {
  indent: string;
  ordered: boolean;
  number: number;
  delimiter: '.' | ')';
  bullet: '-' | '+' | '*';
  spacing: string;
  contentOffset: number;
  task: boolean;
}

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

const handleEnter: Command = ( view ) => enterAtSelections( view, false );
const handleShiftEnter: Command = ( view ) => enterAtSelections( view, true );

function enterAtSelections( view: EditorView, soft: boolean ): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  const edits = view.state.selection.ranges.map( ( range ) => {
    const tableEdit = soft
      ? insertLiveMarkdownTableLineBreak( value, tables, range.anchor, range.head )
      : insertLiveMarkdownTableRow( value, tables, range.head );
    return tableEdit
      ? markdownRangeEdit( value, tableEdit )
      : soft ? softLineBreakEdit( view, range )
        : smartEnterEdit( view, range ) ?? replacementRangeEdit( range, '\n' );
  });

  return applyRangeEdits( view, edits, 'input' );
}

function applyTableNavigation(
  view: EditorView,
  navigation: LiveMarkdownTableNavigation
): boolean {
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  const edits = view.state.selection.ranges.map( ( range ) =>
    navigateLiveMarkdownTable( value, tables, range.head, navigation )
  );
  if ( !edits.some( ( edit ) => edit ) ) {
    return false;
  }

  return applyRangeEdits( view, edits.map( ( edit, index ) => {
    const range = view.state.selection.ranges[ index ]!;
    if ( edit && ( !view.state.readOnly || edit.value === value ) ) {
      return markdownRangeEdit( value, edit );
    }
    const down = navigation === 'down-row';
    return { changes: [], range: view.moveVertically( range, down ) };
  }), 'select.table' );
}

const handleTab: Command = ( view ) => indentSelections( view, false );
const handleShiftTab: Command = ( view ) => indentSelections( view, true );

function indentSelections( view: EditorView, outdent: boolean ): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) => {
    const tableEdit = navigateLiveMarkdownTable(
      value, tables, range.head, outdent ? 'previous-cell' : 'next-cell'
    );
    if ( tableEdit ) {
      return markdownRangeEdit( value, tableEdit );
    }
    return selectedLineEdits( view, range, outdent ) ??
      ( outdent ? { changes: [], range } : replacementRangeEdit( range, INDENT ) );
  }), 'input.indent' );
}

const handleTableArrowDown: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }

  return applyTableNavigation( view, 'down-row' );
};

const handleTableArrowUp: Command = ( view ) => {
  if ( view.composing ) {
    return false;
  }

  return applyTableNavigation( view, 'up-row' );
};

function deleteAtTableSelections(
  view: EditorView,
  forward: boolean,
  unit: 'character' | 'word' | 'line'
): boolean {
  if ( view.composing || view.state.readOnly ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  if ( !view.state.selection.ranges.some( ( range ) =>
    liveMarkdownTableCellTextBounds( tables, range.head )
  ) ) {
    return false;
  }
  const deletedRows = !forward && unit === 'character'
    ? view.state.selection.ranges.flatMap( ( range ) => {
      const edit = range.empty ? deleteEmptyLiveMarkdownTableRow( value, tables, range.head ) : undefined;
      return edit?.change ? [ edit ] : [];
    }) : [];
  const survivingRowTarget = ( position: number ): number => {
    for ( let index = deletedRows.length - 1; index >= 0; index -= 1 ) {
      const edit = deletedRows[ index ]!;
      if ( edit.change!.from <= position && position < edit.change!.to ) {
        position = edit.selectionStart;
      }
    }
    return position;
  };
  const skipAtomic = ( position: number, after: boolean ): number => {
    for ( const ranges of view.state.facet( EditorView.atomicRanges ) ) {
      ranges( view ).between( position, position, ( from, to ) => {
        if ( from < position && position < to ) {
          position = after ? to : from;
        }
      });
    }
    return position;
  };
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) => {
    if ( range.empty && !forward && unit === 'character' ) {
      const rowEdit = deleteEmptyLiveMarkdownTableRow( value, tables, range.head );
      if ( rowEdit ) {
        const edit = markdownRangeEdit( value, rowEdit );
        edit.range = EditorSelection.cursor( survivingRowTarget( rowEdit.selectionStart ), -1 );
        return edit;
      }
      const target = survivingRowTarget( range.head );
      if ( target !== range.head ) {
        return { changes: [], range: EditorSelection.cursor( target, -1 ) };
      }
    }
    let from = range.from;
    let to = range.to;
    if ( range.empty ) {
      const line = view.state.doc.lineAt( range.head );
      let target = range.head;
      if ( unit === 'line' ) {
        target = view.moveToLineBoundary( range, forward ).head;
      } else if ( unit === 'word' ) {
        const categorize = view.state.charCategorizer( range.head );
        let category: ReturnType<typeof categorize> | undefined;
        while ( target !== ( forward ? line.to : line.from ) ) {
          const next = line.from + findClusterBreak( line.text, target - line.from, forward );
          const text = value.slice( Math.min( target, next ), Math.max( target, next ) );
          const nextCategory = categorize( text );
          if ( category !== undefined && category !== nextCategory ) {
            break;
          }
          if ( text !== ' ' || target !== range.head ) {
            category = nextCategory;
          }
          target = next;
        }
      } else {
        target = line.from + findClusterBreak( line.text, range.head - line.from, forward, forward );
        const before = line.text.slice( 0, range.head - line.from );
        if ( !forward && before.length && before.length < 200 && !/[^ \t]/.test( before ) ) {
          const unit = getIndentUnit( view.state );
          const drop = countColumn( before, view.state.tabSize ) % unit || unit;
          target = range.head;
          for ( let index = 0; index < drop && before[ before.length - 1 - index ] === ' '; index += 1 ) {
            target -= 1;
          }
          if ( before.endsWith( '\t' ) ) {
            target -= 1;
          }
        } else if ( !forward && /[\ufe00-\ufe0f]/.test( value.slice( target, range.head ) ) ) {
          target = line.from + findClusterBreak( line.text, target - line.from, false, false );
        }
      }
      if ( target === range.head ) {
        target = Math.max( 0, Math.min( value.length, target + ( forward ? 1 : -1 ) ) );
      }
      const bounds = liveMarkdownTableCellTextBounds( tables, range.head );
      target = bounds
        ? forward
          ? Math.max( range.head, Math.min( target, bounds.to ) )
          : Math.min( range.head, Math.max( target, bounds.from ) )
        : skipAtomic( target, forward );
      from = Math.min( range.head, target );
      to = Math.max( range.head, target );
    } else {
      from = skipAtomic( from, false );
      to = skipAtomic( to, true );
    }
    return {
      changes: from === to ? [] : [{ from, to, insert: '' }],
      range: from === to ? range : EditorSelection.cursor( from, forward ? 1 : -1 )
    };
  }), forward ? 'delete.forward' : 'delete.backward' );
}

const deleteTableBackward: Command = ( view ) => deleteAtTableSelections( view, false, 'character' );
const deleteTableForward: Command = ( view ) => deleteAtTableSelections( view, true, 'character' );
const deleteTableWordBackward: Command = ( view ) => deleteAtTableSelections( view, false, 'word' );
const deleteTableWordForward: Command = ( view ) => deleteAtTableSelections( view, true, 'word' );
const deleteToTableCellTextStart: Command = ( view ) => deleteAtTableSelections( view, false, 'line' );
const deleteToTableCellTextEnd: Command = ( view ) => deleteAtTableSelections( view, true, 'line' );

function moveHorizontal(
  view: EditorView,
  direction: 'left' | 'right',
  extend = false
): boolean {
  if ( view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  let handled = false;
  const ranges = view.state.selection.ranges.map( ( range ) => {
    const forward = ( direction === 'right' ) === ( view.textDirectionAt( range.head ) === Direction.LTR );
    const boundary = forward ? 'end' : 'start';
    if ( extend && isLiveMarkdownTableCellBoundary( tables, range.head, boundary ) ) {
      handled = true;
      return range;
    }
    const tableTarget = range.empty && !extend
      ? moveAcrossLiveMarkdownTableCellBoundary( value, tables, range.head, direction )
      : undefined;
    const line = view.state.doc.lineAt( range.head );
    const offset = renderedListTextOffset( line.text );
    const revealList = !extend && range.empty && !forward && offset !== undefined &&
      range.head === line.from + offset && offset > 0;
    let target: SelectionRange;
    if ( tableTarget ) {
      handled = true;
      target = EditorSelection.cursor( tableTarget.position, tableTarget.assoc );
    } else if ( revealList ) {
      handled = true;
      target = EditorSelection.cursor( range.head - 1 );
    } else {
      target = range.empty || extend
        ? view.moveByChar( range, forward )
        : EditorSelection.cursor( forward ? range.to : range.from );
    }
    return extend
      ? EditorSelection.range( range.anchor, target.head, target.goalColumn, target.bidiLevel ?? undefined, target.assoc )
      : target;
  });
  return handled && applyRangeEdits( view, ranges.map( ( range ) => ({ changes: [], range }) ), 'select' );
}

const moveAcrossTableCellLeft: Command = ( view ) => moveHorizontal( view, 'left' );
const moveAcrossTableCellRight: Command = ( view ) => moveHorizontal( view, 'right' );
const protectTableCellSelectionLeft: Command = ( view ) => moveHorizontal( view, 'left', true );
const protectTableCellSelectionRight: Command = ( view ) => moveHorizontal( view, 'right', true );

function setTextBoundary(
  view: EditorView,
  boundary: 'end' | 'start' | 'left' | 'right',
  extend: boolean
): boolean {
  if ( view.composing ) {
    return false;
  }
  const tables = liveMarkdownDocumentModel( view.state ).tables;
  let handled = false;
  const ranges = view.state.selection.ranges.map( ( range ) => {
    const forward = boundary === 'end' || ( boundary === 'left' || boundary === 'right' ) &&
      ( ( boundary === 'right' ) === ( view.textDirectionAt( range.head ) === Direction.LTR ) );
    const block = view.lineBlockAt( range.head );
    let target = view.moveToLineBoundary( range, forward );
    if ( target.head === range.head && target.head !== ( forward ? block.to : block.from ) ) {
      target = view.moveToLineBoundary( range, forward, false );
    }
    let position = target.head;
    let assoc = target.assoc;
    const bounds = liveMarkdownTableCellTextBounds( tables, range.head );
    const anchorBounds = extend ? liveMarkdownTableCellTextBounds( tables, range.anchor ) : bounds;
    const line = view.state.doc.lineAt( range.head );
    const offset = renderedListTextOffset( line.text );
    if ( bounds && anchorBounds && bounds.from === anchorBounds.from && bounds.to === anchorBounds.to ) {
      handled = true;
      position = Math.max( bounds.from, Math.min( position, bounds.to ) );
      if ( bounds.from === bounds.to ) {
        assoc = forward ? -1 : 1;
      } else if ( position === bounds.from ) {
        assoc = 1;
      } else if ( position === bounds.to ) {
        assoc = -1;
      }
    } else if ( !forward && offset !== undefined && position <= line.from + offset ) {
      handled = true;
      const textStart = line.from + offset;
      position = range.head < textStart || range.head === textStart && ( extend || range.empty )
        ? line.from : textStart;
    } else if ( !forward && position === block.from && block.length ) {
      const whitespace = view.state.sliceDoc( block.from, block.to ).match( /^\s*/ )![ 0 ].length;
      if ( whitespace && range.head !== block.from + whitespace ) {
        position += whitespace;
      }
    }
    return extend
      ? EditorSelection.range( range.anchor, position, target.goalColumn, target.bidiLevel ?? undefined, assoc )
      : EditorSelection.cursor( position, assoc );
  });
  return handled && applyRangeEdits( view, ranges.map( ( range ) => ({ changes: [], range }) ), 'select' );
}

const moveToTableCellTextStart: Command = ( view ) => setTextBoundary( view, 'start', false );
const selectToTableCellTextStart: Command = ( view ) => setTextBoundary( view, 'start', true );
const moveToTableCellTextEnd: Command = ( view ) => setTextBoundary( view, 'end', false );
const selectToTableCellTextEnd: Command = ( view ) => setTextBoundary( view, 'end', true );
const moveToTableCellTextLeft: Command = ( view ) => setTextBoundary( view, 'left', false );
const selectToTableCellTextLeft: Command = ( view ) => setTextBoundary( view, 'left', true );
const moveToTableCellTextRight: Command = ( view ) => setTextBoundary( view, 'right', false );
const selectToTableCellTextRight: Command = ( view ) => setTextBoundary( view, 'right', true );

const toggleBold: Command = ( view ) => toggleSelectionFormatting( view, '**', [ '__' ]);
const toggleItalic: Command = ( view ) => toggleSelectionFormatting( view, '*', [ '_' ]);
const toggleStrikethrough: Command = ( view ) => toggleSelectionFormatting( view, '~~' );
const insertLiteralHyphen: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }

  view.dispatch(
    view.state.replaceSelection( '-' ),
    {
      scrollIntoView: true,
      userEvent: 'input.type'
    }
  );

  return true;
};
const wrapSelectionAsInlineCode: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    range.empty ? replacementRangeEdit( range, '`' ) :
      markdownRangeEdit( value, wrapInlineCode( value, range.from, range.to ), range )
  ), 'input.format' );
};
const wrapSelectionAsMarkdownLink: Command = ( view ) => {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }
  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    markdownRangeEdit( value, wrapMarkdownLink( value, range.from, range.to ), range )
  ), 'input.format' );
};

async function resolveLiveMarkdownImageSource(
  image: ParsedMarkdownImage
): Promise<string | undefined> {
  const source = sanitizeImageUrl( image.destination );
  if ( !source ) {
    return undefined;
  }
  if (
    !isTauri()
    || !props.vaultPath
    || /^[a-z][a-z0-9+.-]*:/i.test( source )
    || source.startsWith( '//' )
  ) {
    return source;
  }

  const generation = imageResolverGeneration;
  const cacheKey = `${ generation }\u0000${ props.vaultPath }\u0000${ props.noteRelativePath }\u0000${
    image.assetId ?? ''
  }\u0000${ image.destination }`;
  const cached = imageSourcePromises.get( cacheKey );
  if ( cached ) {
    return cached;
  }

  const pending = ( async () => {
    const bytes = await readWorkspaceImage(
      props.vaultPath!,
      props.noteRelativePath,
      decodeMarkdownImageDestination( image.destination ),
      image.assetId
    );
    if ( imageResolverDisposed || generation !== imageResolverGeneration ) {
      throw new Error( 'The image changed before it finished loading.' );
    }
    const mediaType = props.embeddedImages.find( ( asset ) => asset.id === image.assetId )?.mediaType
      ?? imageMediaTypeForPath( image.destination );
    const url = URL.createObjectURL( new Blob([ bytes.slice().buffer ], { type: mediaType }) );
    imageObjectUrls.add( url );

    return url;
  })();
  imageSourcePromises.set( cacheKey, pending );
  pending.catch( () => {
    if ( imageSourcePromises.get( cacheKey ) === pending ) {
      imageSourcePromises.delete( cacheKey );
    }
  });

  return pending;
}

function clearLiveMarkdownImageSources(): void {
  imageResolverGeneration += 1;
  imageSourcePromises.clear();
  for ( const url of imageObjectUrls ) {
    URL.revokeObjectURL( url );
  }
  imageObjectUrls.clear();
}

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
    historyCompartment.of( history() ),
    drawSelection(),
    dropCursor(),
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
        for ( const insertion of pendingImageInsertions.values() ) {
          const empty = insertion.from === insertion.to;
          insertion.from = update.changes.mapPos( insertion.from, -1 );
          insertion.to = update.changes.mapPos( insertion.to, empty ? -1 : 1 );
        }
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
  const historySession = openNoteEditorHistory(
    props.vaultId,
    props.noteId
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
  let state = savedState
    ? EditorState.fromJSON(
      savedState,
      { extensions },
      { history: historyField }
    )
    : EditorState.create({
      doc: editableDocument.body,
      selection: initialPosition
        ? EditorSelection.range( initialPosition.selection.anchor, initialPosition.selection.head )
        : undefined,
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
  removePositionCapture = registerNoteEditorPositionCapture(
    props.vaultId,
    props.noteId,
    () => positionCaptureEnabled ? captureEditorPosition( view ) : undefined
  );
  view.scrollDOM.addEventListener( 'scroll', handleEditorScroll, { passive: true });
  updateSuggestions( view );
  view.focus();

  if ( initialPosition ) {
    scheduleViewportRestore( view, initialPosition );
  } else {
    positionCaptureEnabled = true;
    schedulePositionCapture( view );
    scheduleEditorRenderReady( view );
  }
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
  imageResolverDisposed = true;
  pendingImageInsertions.clear();
  clearLiveMarkdownImageSources();
  const view = editorView.value;
  const captureWasActive = removePositionCapture?.() ?? false;
  removePositionCapture = undefined;
  if ( viewportRestoreFrame !== undefined ) {
    window.cancelAnimationFrame( viewportRestoreFrame );
    viewportRestoreFrame = undefined;
  }
  if ( view ) {
    view.scrollDOM.removeEventListener( 'scroll', handleEditorScroll );
    if ( positionCaptureEnabled && captureWasActive ) {
      emitEditorPosition( captureEditorPosition( view ) );
    }
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

const attachmentPathKeys = computed( () => new Set(
  props.attachmentFiles.map( ( attachment ) => attachment.relativePath.toLocaleLowerCase() )
) );

function isKnownExtensionlessAttachment( destination: string ): boolean {
  const relativePath = resolveMarkdownImagePath( props.noteRelativePath, destination );

  return Boolean(
    relativePath
    && attachmentPathKeys.value.has( relativePath.toLocaleLowerCase() )
  );
}

function resolveLiveMarkdownAttachmentMetadata(
  attachment: ParsedMarkdownAttachment
): MarkdownAttachmentMetadata | undefined {
  const tracked = attachment.assetId
    ? props.embeddedAttachments.find( ( asset ) => asset.id === attachment.assetId )
    : undefined;
  const relativePath = tracked?.relativePath
    ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
  if ( !relativePath ) {
    return undefined;
  }
  const portablePath = relativePath.toLocaleLowerCase();
  const matches = attachment.assetId
    ? props.attachmentFiles.filter( ( candidate ) =>
      candidate.assetId === attachment.assetId
    )
    : props.attachmentFiles.filter( ( candidate ) =>
      candidate.relativePath.toLocaleLowerCase() === portablePath
    );
  const file = matches.length === 1 ? matches[ 0 ] : undefined;
  const renameTarget = file && props.vaultPath && isTauri()
    ? {
      ...( file.assetId ? { assetId: file.assetId } : {}),
      relativePath: file.relativePath
    }
    : undefined;

  return {
    byteLength: tracked?.byteLength ?? file?.byteLength,
    mediaType: tracked?.mediaType ?? file?.mediaType,
    openingDisabled: tracked?.openingDisabled ?? file?.openingDisabled,
    ...( renameTarget ? { renameTarget } : {}),
    relativePath: tracked?.relativePath ?? file?.relativePath ?? relativePath
  };
}

function activateLiveMarkdownAttachment(
  attachment: ParsedMarkdownAttachment,
  metadata: MarkdownAttachmentMetadata | null | undefined
): void {
  const relativePath = metadata?.relativePath
    ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
  if ( relativePath ) {
    emit(
      'activateAttachment',
      attachment.assetId,
      relativePath,
      metadata?.mediaType,
      metadata?.openingDisabled
    );
  }
}

function revealLiveMarkdownAttachmentInTree(
  attachment: ParsedMarkdownAttachment,
  metadata: MarkdownAttachmentMetadata | null | undefined
): void {
  emitLiveMarkdownAttachmentLocation( 'revealAttachmentInTree', attachment, metadata );
}

function showLiveMarkdownAttachmentInFolder(
  attachment: ParsedMarkdownAttachment,
  metadata: MarkdownAttachmentMetadata | null | undefined
): void {
  emitLiveMarkdownAttachmentLocation( 'showAttachmentInFolder', attachment, metadata );
}

function emitLiveMarkdownAttachmentLocation(
  event: 'revealAttachmentInTree' | 'showAttachmentInFolder',
  attachment: ParsedMarkdownAttachment,
  metadata: MarkdownAttachmentMetadata | null | undefined
): void {
  const relativePath = metadata?.relativePath
    ?? resolveMarkdownImagePath( props.noteRelativePath, attachment.destination );
  if ( !relativePath ) {
    return;
  }
  const assetId = metadata?.renameTarget?.assetId;
  if ( event === 'revealAttachmentInTree' ) {
    emit( 'revealAttachmentInTree', assetId, relativePath );
  } else {
    emit( 'showAttachmentInFolder', assetId, relativePath );
  }
}

function focusDocumentOffset( offset: number ): boolean {
  const view = editorView.value;
  if ( !view || !Number.isFinite( offset ) ) {
    return false;
  }

  if ( viewportRestoreFrame !== undefined ) {
    window.cancelAnimationFrame( viewportRestoreFrame );
    viewportRestoreFrame = undefined;
  }

  const bodyStart = markdownBodyStart(
    frontmatterPrefix,
    view.state.doc.toString()
  );
  const position = Math.min(
    view.state.doc.length,
    Math.max( 0, Math.trunc( offset ) - bodyStart )
  );

  positionCaptureEnabled = true;
  view.dispatch({
    selection: EditorSelection.cursor( position ),
    effects: EditorView.scrollIntoView( position, {
      y: 'start',
      yMargin: VIEWPORT_ANCHOR_MARGIN
    }),
    userEvent: 'select'
  });
  view.focus();
  schedulePositionCapture( view );

  return true;
}

function captureImageInsertion(
  view = editorView.value
): ImageInsertionCapture | undefined {
  if ( props.readOnly || !view ) {
    return undefined;
  }
  if ( view.state.selection.ranges.length !== 1 ) {
    notify( 'Use a single selection to insert an image or file.', 'neutral' );
    return undefined;
  }

  const selection = view.state.selection.main;
  const inTable = liveMarkdownDocumentModel( view.state ).tables.some( ( table ) =>
    selection.from >= table.from && selection.to <= table.to
  );
  imageInsertionSequence += 1;
  const token = `${ Date.now().toString( 36 ) }-${ imageInsertionSequence.toString( 36 ) }`;
  const insertion: PendingImageInsertion = {
    from: selection.from,
    inTable,
    noteId: props.noteId,
    selectedText: view.state.sliceDoc( selection.from, selection.to ),
    to: selection.to,
    token
  };
  pendingImageInsertions.set( token, insertion );

  return {
    inTable: insertion.inTable,
    noteId: insertion.noteId,
    selectedText: insertion.selectedText,
    token: insertion.token
  };
}

function cancelImageInsertion( capture: ImageInsertionCapture ): void {
  pendingImageInsertions.delete( capture.token );
}

function insertEmbeddedImage(
  capture: ImageInsertionCapture,
  markdownImage: string
): boolean {
  const view = editorView.value;
  const insertion = pendingImageInsertions.get( capture.token );
  pendingImageInsertions.delete( capture.token );
  if (
    props.readOnly
    || !view
    || !insertion
    || insertion.noteId !== props.noteId
    || insertion.from < 0
    || insertion.to < insertion.from
    || insertion.to > view.state.doc.length
    || view.state.sliceDoc( insertion.from, insertion.to ) !== insertion.selectedText
  ) {
    return false;
  }

  if ( view.state.selection.ranges.length !== 1 ) {
    notify( 'Use a single selection to insert an image or file.', 'neutral' );
    return false;
  }

  const cursor = insertion.from + markdownImage.length;
  view.dispatch({
    changes: {
      from: insertion.from,
      to: insertion.to,
      insert: markdownImage
    },
    selection: EditorSelection.cursor( cursor ),
    scrollIntoView: true,
    userEvent: 'input'
  });
  view.focus();

  return true;
}

function requestImageEmbed( view: EditorView ): boolean {
  const capture = captureImageInsertion( view );
  if ( !capture ) {
    return view.state.selection.ranges.length > 1;
  }
  emit( 'requestEmbedImage', capture );

  return true;
}

function captureAttachmentInsertion(
  view = editorView.value
): AttachmentInsertionCapture | undefined {
  return captureImageInsertion( view );
}

function cancelAttachmentInsertion( capture: AttachmentInsertionCapture ): void {
  cancelImageInsertion( capture );
}

function insertEmbeddedAttachment(
  capture: AttachmentInsertionCapture,
  markdownAttachment: string
): boolean {
  return insertEmbeddedImage( capture, markdownAttachment );
}

function requestAttachmentEmbed( view: EditorView ): boolean {
  const capture = captureAttachmentInsertion( view );
  if ( !capture ) {
    return view.state.selection.ranges.length > 1;
  }
  emit( 'requestEmbedAttachment', capture );

  return true;
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

function toggleSelectionFormatting(
  view: EditorView,
  marker: string,
  alternatives: string[] = []
): boolean {
  if ( view.state.readOnly || view.composing ) {
    return false;
  }

  const value = view.state.doc.toString();
  return applyRangeEdits( view, view.state.selection.ranges.map( ( range ) =>
    markdownRangeEdit( value, toggleInlineFormatting(
      value, range.from, range.to, marker, alternatives
    ), range )
  ), 'input.format' );
}

function replacementRangeEdit( range: SelectionRange, insert: string ): MultiCursorEdit {
  return {
    changes: [{ from: range.from, to: range.to, insert }],
    range: EditorSelection.cursor( range.from + insert.length )
  };
}

function softLineBreakEdit( view: EditorView, range: SelectionRange ): MultiCursorEdit {
  const { state } = view;
  let { from, to } = range;
  const line = state.doc.lineAt( from );
  let brackets: { from: number; to: number } | undefined;
  if ( range.empty ) {
    if ( /\(\)|\[\]|\{\}/.test( state.sliceDoc( Math.max( 0, from - 1 ), from + 1 ) ) ) {
      brackets = { from, to };
    } else {
      const context = syntaxTree( state ).resolveInner( from );
      const before = context.childBefore( from );
      const after = context.childAfter( from );
      if ( before && after && before.to <= from && after.from >= from &&
        before.type.prop( NodeProp.closedBy )?.includes( after.name ) &&
        state.doc.lineAt( before.to ).from === state.doc.lineAt( after.from ).from &&
        !/\S/.test( state.sliceDoc( before.to, after.from ) ) ) {
        brackets = { from: before.to, to: after.from };
      }
    }
  }
  const context = new IndentContext( state, { simulateBreak: from, simulateDoubleBreak: Boolean( brackets ) });
  const indentation = getIndentation( context, from ) ??
    countColumn( line.text.match( /^\s*/ )![ 0 ], state.tabSize );
  while ( to < line.to && /\s/.test( line.text[ to - line.from ]! ) ) {
    to += 1;
  }
  if ( brackets ) {
    ({ from, to } = brackets );
  } else if ( from > line.from && !/\S/.test( state.sliceDoc( line.from, from ) ) ) {
    from = line.from;
  }
  const indent = indentString( state, indentation );
  const closing = brackets ? `\n${ indentString( state, context.lineIndent( line.from, -1 ) ) }` : '';
  return {
    changes: [{ from, to, insert: `\n${ indent }${ closing }` }],
    range: EditorSelection.cursor( from + 1 + indent.length )
  };
}

function markdownRangeEdit(
  value: string,
  edit: MarkdownSelectionEdit & { change?: LiveMarkdownTextEdit },
  original?: SelectionRange
): MultiCursorEdit {
  return {
    changes: value === edit.value ? [] : [ edit.change ?? minimalDocumentChange( value, edit.value ) ],
    range: original && original.anchor > original.head
      ? EditorSelection.range( edit.selectionEnd, edit.selectionStart )
      : EditorSelection.range( edit.selectionStart, edit.selectionEnd )
  };
}

function applyRangeEdits(
  view: EditorView,
  edits: readonly MultiCursorEdit[],
  userEvent: string
): boolean {
  const result = multiCursorChanges( view.state, edits );
  if ( !result ) {
    notify( 'These selections affect the same text. Adjust the selections and try again.', 'neutral' );
    return true;
  }
  if ( view.state.readOnly && !result.changes.empty ) {
    return false;
  }
  const docChanged = !result.changes.empty;
  view.dispatch( result, {
    annotations: docChanged ? isolateHistory.of( 'full' ) : undefined,
    scrollIntoView: true,
    userEvent: docChanged && userEvent.startsWith( 'select.' ) ? 'input.table' : userEvent
  });
  if ( docChanged ) {
    // Record the final ranges after transaction filters (including renumbering)
    // so redo restores the intentional caret positions in every edited range.
    view.dispatch({ selection: view.state.selection, userEvent: 'select' });
  }
  return true;
}

function minimalDocumentChange(
  current: string,
  next: string
): { from: number; to: number; insert: string } {
  let from = 0;
  const sharedLength = Math.min( current.length, next.length );
  while ( from < sharedLength && current[ from ] === next[ from ]) {
    from += 1;
  }

  let currentTo = current.length;
  let nextTo = next.length;
  while (
    currentTo > from &&
    nextTo > from &&
    current[ currentTo - 1 ] === next[ nextTo - 1 ]
  ) {
    currentTo -= 1;
    nextTo -= 1;
  }

  return {
    from,
    to: currentTo,
    insert: next.slice( from, nextTo )
  };
}

function preferredLineEnding( value: string ): '\n' | '\r' | '\r\n' {
  const lineEnding = value.match( /\r\n|\r|\n/ )?.[ 0 ];

  return lineEnding === '\r\n' || lineEnding === '\r'
    ? lineEnding
    : '\n';
}

function normalizeDocumentText( value: string ): string {
  return value.replace( /\r\n|\r/g, '\n' );
}

function projectEditableDocument(
  normalizedMarkdown: string,
  showFrontmatter: boolean
) {
  if ( !showFrontmatter ) {
    return splitLeadingFrontmatter( normalizedMarkdown );
  }

  return {
    body: normalizedMarkdown,
    bodyStart: 0,
    lineNumberOffset: 0,
    prefix: ''
  };
}

function restorableEditorHistory(
  snapshot: NoteEditorHistorySnapshot | undefined,
  normalizedMarkdown: string,
  editableDocument: ReturnType<typeof projectEditableDocument>
): NoteEditorHistorySnapshot | undefined {
  if ( !snapshot ) {
    return undefined;
  }
  if ( snapshot.doc === editableDocument.body ) {
    return snapshot;
  }
  const savedMarkdown = joinLeadingFrontmatter( snapshot.prefix, snapshot.doc );
  if ( savedMarkdown !== normalizedMarkdown ) {
    return undefined;
  }
  const savedBodyStart = markdownBodyStart( snapshot.prefix, snapshot.doc );
  const hidingEditedFrontmatter = savedBodyStart === 0
    && editableDocument.bodyStart > 0
    && snapshot.frontmatterHistoryChanged;

  return hidingEditedFrontmatter ? undefined : snapshot;
}

function frontmatterVisibilitySelection(
  selection: EditorSelection,
  currentBodyStart: number,
  editableDocument: ReturnType<typeof projectEditableDocument>
): EditorSelection {
  const clamp = ( position: number ): number => Math.min(
    editableDocument.body.length,
    Math.max( 0, position + currentBodyStart - editableDocument.bodyStart )
  );

  return EditorSelection.create( selection.ranges.map( ( range ) =>
    EditorSelection.range( clamp( range.anchor ), clamp( range.head ) )
  ), selection.mainIndex );
}

function changeTouchesLeadingFrontmatter( update: ViewUpdate ): boolean {
  const previousEnd = leadingFrontmatterEnd( update.startState.doc.toString() ) ?? 0;
  const nextEnd = leadingFrontmatterEnd( update.state.doc.toString() ) ?? 0;
  let touchesFrontmatter = previousEnd !== nextEnd;
  update.changes.iterChangedRanges( ( fromA, _toA, fromB ) => {
    if ( fromA < previousEnd || fromB < nextEnd ) {
      touchesFrontmatter = true;
    }
  });

  return touchesFrontmatter;
}

function normalizeInitialPosition(
  position: NoteEditorPosition | undefined,
  documentLength: number,
  bodyStart: number
): NoteEditorPosition | undefined {
  if ( !position ) {
    return undefined;
  }

  const clamp = ( value: number ): number => Math.min(
    documentLength,
    Math.max( 0, Math.trunc( value ) - bodyStart )
  );

  return {
    selection: {
      anchor: clamp( position.selection.anchor ),
      head: clamp( position.selection.head )
    },
    viewport: {
      anchor: clamp( position.viewport.anchor ),
      offset: position.viewport.offset,
      left: Math.max( 0, position.viewport.left )
    }
  };
}

function handleEditorScroll(): void {
  const view = editorView.value;
  if ( view ) {
    schedulePositionCapture( view );
  }
}

function schedulePositionCapture( view: EditorView ): void {
  if ( !positionCaptureEnabled ) {
    return;
  }

  view.requestMeasure({
    key: positionCaptureKey,
    read: captureEditorPosition,
    write: emitEditorPosition
  });
}

function scheduleEditorRenderReady( view: EditorView ): void {
  if ( editorRenderReady.value || !positionCaptureEnabled ) {
    return;
  }

  // Keep the mounted editor measurable while CodeMirror finishes parsing the
  // restored viewport, then expose the completed rendering in a single frame.
  view.requestMeasure({
    key: renderReadyKey,
    read: ( measuredView ) => syntaxTreeAvailable(
      measuredView.state,
      measuredView.viewport.to
    ),
    write: ( ready, measuredView ) => {
      if ( ready && editorView.value === measuredView ) {
        const scrollLeft = measuredView.scrollDOM.scrollLeft;
        const scrollTop = measuredView.scrollDOM.scrollTop;
        editorRenderReady.value = true;
        measuredView.scrollDOM.scrollLeft = scrollLeft;
        measuredView.scrollDOM.scrollTop = scrollTop;
      }
    }
  });
}

function captureEditorPosition( view: EditorView ): NoteEditorPosition {
  const selection = view.state.selection.main;
  const bodyStart = markdownBodyStart(
    frontmatterPrefix,
    view.state.doc.toString()
  );
  const scrollTop = view.scrollDOM.scrollTop;
  const candidate = view.lineBlockAtHeight( scrollTop + VIEWPORT_ANCHOR_MARGIN );
  const firstVisibleBlock = view.viewportLineBlocks[ 0 ];
  const viewportBlock = candidate.from >= view.viewport.from
    || !firstVisibleBlock
    || firstVisibleBlock.top - scrollTop > VIRTUALIZED_VIEWPORT_THRESHOLD
    ? candidate
    : firstVisibleBlock;

  return {
    selection: {
      anchor: selection.anchor + bodyStart,
      head: selection.head + bodyStart
    },
    viewport: {
      anchor: viewportBlock.from + bodyStart,
      offset: viewportBlock.top - scrollTop,
      left: view.scrollDOM.scrollLeft
    }
  };
}

function editorLineNumbers( offset: number ): Extension {
  return lineNumbers({
    formatNumber: ( lineNumber ) => String( lineNumber + offset )
  });
}

function emitEditorPosition( position: NoteEditorPosition ): void {
  emit( 'editorPosition', props.vaultId, props.noteId, position );
}

function scheduleViewportRestore(
  view: EditorView,
  position: NoteEditorPosition
): void {
  viewportRestoreFrame = window.requestAnimationFrame( () => {
    viewportRestoreFrame = undefined;
    if ( editorView.value !== view ) {
      return;
    }

    view.requestMeasure({
      key: viewportRestoreKey,
      read: ( measuredView ) => {
        const viewportBlock = measuredView.lineBlockAt( position.viewport.anchor );
        const maximumTop = Math.max(
          0,
          measuredView.scrollDOM.scrollHeight - measuredView.scrollDOM.clientHeight
        );
        const maximumLeft = Math.max(
          0,
          measuredView.scrollDOM.scrollWidth - measuredView.scrollDOM.clientWidth
        );

        return {
          left: Math.min( maximumLeft, position.viewport.left ),
          top: Math.min(
            maximumTop,
            Math.max( 0, viewportBlock.top - position.viewport.offset )
          )
        };
      },
      write: ({ left, top }, measuredView ) => {
        measuredView.scrollDOM.scrollLeft = left;
        measuredView.scrollDOM.scrollTop = top;
        positionCaptureEnabled = true;
        schedulePositionCapture( measuredView );
        scheduleEditorRenderReady( measuredView );
      }
    });
  });
}

function restoreLineEndings(
  value: string,
  lineEnding: '\n' | '\r' | '\r\n'
): string {
  return lineEnding === '\n' ? value : value.replace( /\n/g, lineEnding );
}

function smartEnterEdit( view: EditorView, selection: SelectionRange ): MultiCursorEdit | undefined {
  if ( !selection.empty ) {
    return undefined;
  }

  const value = view.state.doc.toString();
  const position = selection.head;
  const line = view.state.doc.lineAt( position );
  const source = line.text;
  const openingFence = source.match( /^([ \t]*)(`{3,}|~{3,})([^\n]*)$/ );
  if (
    openingFence &&
    position === line.to &&
    !activeFenceBefore( value, line.from )
  ) {
    const indent = openingFence[ 1 ]!;
    const marker = openingFence[ 2 ]!;
    const followingLineStart = line.to < value.length ? line.to + 1 : value.length;
    const followingLine = view.state.doc.lineAt( followingLineStart ).text;
    const existingClosing = followingLine.match( /^([ \t]*)(`+|~+)\s*$/ );
    const closesFence = Boolean( existingClosing
      && existingClosing[ 2 ]![ 0 ] === marker[ 0 ]
      && existingClosing[ 2 ]!.length >= marker.length );
    const insertion = closesFence
      ? `\n${ indent }`
      : `\n${ indent }\n${ indent }${ marker }`;
    return {
      changes: [{ from: position, to: position, insert: insertion }],
      range: EditorSelection.cursor( position + 1 + indent.length )
    };
  }

  if ( activeFenceBefore( value, line.from ) ) {
    return undefined;
  }

  const item = matchEditableListLine( source );
  if ( !item ) {
    return undefined;
  }

  const contentStart = line.from + item.contentOffset;
  const insertionPosition = Math.max( position, contentStart );
  const bodyBefore = value.slice( contentStart, insertionPosition );
  const bodyAfter = value.slice( insertionPosition, line.to );
  const fullBody = `${ bodyBefore }${ bodyAfter }`;

  if ( !fullBody.trim() && insertionPosition === line.to ) {
    const replacement = item.indent;
    return {
      changes: [{ from: line.from, to: line.to, insert: replacement }],
      range: EditorSelection.cursor( line.from + replacement.length )
    };
  }

  const marker = item.ordered
    ? `${ item.number >= 999_999_999 ? 1 : item.number + 1 }${ item.delimiter }`
    : item.bullet;
  const taskPrefix = item.task ? '[ ] ' : '';
  const continuation = `${ item.indent }${ marker }${ item.spacing }${ taskPrefix }`;
  const separatorLength = bodyAfter.match( /^[ \t]+/ )?.[ 0 ].length ?? 0;
  const cursor = insertionPosition + 1 + continuation.length;
  return {
    changes: [{ from: insertionPosition, to: insertionPosition + separatorLength, insert: `\n${ continuation }` }],
    range: EditorSelection.cursor( cursor )
  };
}

function matchEditableListLine( line: string ): ListLine | undefined {
  const match = line.match( /^([ \t]*)(?:(\d{1,9})([.)])|([-+*]))([ \t]+)(.*)$/ );
  if ( !match ) {
    return undefined;
  }
  const ordered = Boolean( match[ 2 ]);
  const markerLength = ordered ? match[ 2 ]!.length + 1 : 1;
  const spacing = match[ 5 ]!;
  const listPrefixLength = match[ 1 ]!.length + markerLength + spacing.length;
  const taskPrefix = match[ 6 ]!.match( /^\[[ xX]\][ \t]+/ )?.[ 0 ];

  return {
    indent: match[ 1 ]!,
    ordered,
    number: ordered ? Number.parseInt( match[ 2 ]!, 10 ) : 1,
    delimiter: ordered ? match[ 3 ]! as '.' | ')' : '.',
    bullet: ordered ? '-' : match[ 4 ]! as '-' | '+' | '*',
    spacing,
    contentOffset: listPrefixLength + ( taskPrefix?.length ?? 0 ),
    task: Boolean( taskPrefix )
  };
}

function renderedListTextOffset( line: string ): number | undefined {
  const item = matchEditableListLine( line );
  if ( !item ) {
    return undefined;
  }

  return item.contentOffset;
}

function activeFenceBefore(
  value: string,
  position: number
): { marker: string; length: number } | undefined {
  const lines = value.slice( 0, position ).split( '\n' );
  lines.pop();
  let active: { marker: string; length: number } | undefined;

  for ( const line of lines ) {
    if ( !active ) {
      const opening = line.match( /^[ \t]*(`{3,}|~{3,})[^\n]*$/ );
      if ( opening ) {
        active = { marker: opening[ 1 ]![ 0 ]!, length: opening[ 1 ]!.length };
      }

      continue;
    }

    const closing = line.match( /^[ \t]*(`+|~+)\s*$/ );
    if (
      closing &&
      closing[ 1 ]![ 0 ] === active.marker &&
      closing[ 1 ]!.length >= active.length
    ) {
      active = undefined;
    }
  }

  return active;
}

function selectedLineEdits(
  view: EditorView,
  selection: SelectionRange,
  outdent: boolean
): MultiCursorEdit | undefined {
  const value = view.state.doc.toString();
  const selectionStart = selection.from;
  const selectionEnd = selection.to;
  const hasSelection = !selection.empty;
  const firstLine = view.state.doc.lineAt( selectionStart );

  if ( !hasSelection && !matchEditableListLine( firstLine.text ) ) {
    if ( !outdent || !/^[ \t]/.test( firstLine.text ) ) {
      return undefined;
    }
  }

  const selectionEndsAtLineStart = hasSelection
    && selectionEnd > 0
    && value[ selectionEnd - 1 ] === '\n';
  const effectiveEnd = selectionEndsAtLineStart ? selectionEnd - 1 : selectionEnd;
  const lastLine = view.state.doc.lineAt( effectiveEnd );
  const block = value.slice( firstLine.from, lastLine.to );
  const lines = block.split( '\n' );
  const edits: { from: number; to: number; insert: string }[] = [];
  let sourceOffset = firstLine.from;

  for ( const line of lines ) {
    if ( !outdent ) {
      edits.push({ from: sourceOffset, to: sourceOffset, insert: INDENT });
      sourceOffset += line.length + 1;

      continue;
    }

    const removable = line.startsWith( '\t' ) ? 1 : line.match( /^ {1,2}/ )?.[ 0 ].length ?? 0;
    if ( removable ) {
      edits.push({ from: sourceOffset, to: sourceOffset + removable, insert: '' });
    }
    sourceOffset += line.length + 1;
  }

  return { changes: edits, range: selection.map( view.state.changes( edits ), 1 ) };
}

function blockPendingEditorInteraction( event: Event ): void {
  if ( !editorRenderReady.value || ( props.readOnly && event.type !== 'keydown' ) ) {
    event.preventDefault();
    event.stopImmediatePropagation();
  }
}

function handleSourceEditorPaste( event: ClipboardEvent ): void {
  if ( props.readOnly || !editorRenderReady.value ) {
    blockPendingEditorInteraction( event );

    return;
  }

  const clipboard = event.clipboardData;
  if ( !clipboard ) {
    return;
  }
  const imageItem = Array.from( clipboard.items ).find( ( item ) =>
    item.kind === 'file' && item.type.toLocaleLowerCase().startsWith( 'image/' )
  );
  const imageSignaled = Boolean( imageItem )
    || Array.from( clipboard.types ).some( ( type ) =>
      type === 'Files' || type.toLocaleLowerCase().startsWith( 'image/' )
    );
  if ( !imageSignaled ) {
    return;
  }

  event.preventDefault();
  event.stopImmediatePropagation();
  const capture = captureImageInsertion();
  if ( !capture ) {
    return;
  }
  emit( 'pasteImage', capture, imageItem?.getAsFile() ?? undefined );
}

function isExternalFileDrag( event: DragEvent ): boolean {
  const types = Array.from( event.dataTransfer?.types ?? []);
  return types.includes( 'Files' )
    && !types.includes( NOTE_IMAGE_DRAG_MIME )
    && !types.includes( VAULT_IMAGE_DRAG_MIME )
    && !types.includes( VAULT_ATTACHMENT_DRAG_MIME );
}

function clearExternalFileDrag(): void {
  externalFileDragDepth = 0;
  externalFileDragActive.value = false;
}

function handleSourceEditorDragEnter( event: DragEvent ): void {
  if ( props.readOnly || !editorRenderReady.value || !isExternalFileDrag( event ) ) {
    return;
  }
  externalFileDragDepth += 1;
  externalFileDragActive.value = true;
}

function handleSourceEditorDragLeave(): void {
  if ( !externalFileDragActive.value ) {
    return;
  }
  externalFileDragDepth = Math.max( 0, externalFileDragDepth - 1 );
  if ( !externalFileDragDepth ) {
    externalFileDragActive.value = false;
  }
}

function handleSourceEditorDragOver( event: DragEvent ): void {
  const types = Array.from( event.dataTransfer?.types ?? []);
  const movingWithinNote = types.includes( NOTE_IMAGE_DRAG_MIME );
  const vaultImage = types.includes( VAULT_IMAGE_DRAG_MIME );
  const vaultAttachment = types.includes( VAULT_ATTACHMENT_DRAG_MIME );
  const externalFiles = isExternalFileDrag( event );
  if (
    !movingWithinNote
    && !vaultImage
    && !vaultAttachment
    && !externalFiles
  ) {
    return;
  }
  event.preventDefault();
  event.stopImmediatePropagation();
  if ( externalFiles && editorRenderReady.value && !props.readOnly ) {
    externalFileDragActive.value = true;
  }
  if ( event.dataTransfer ) {
    event.dataTransfer.dropEffect = props.readOnly ? 'none' : movingWithinNote ? 'move' : 'copy';
  }
}

function handleSourceEditorDrop( event: DragEvent ): void {
  clearExternalFileDrag();
  if ( props.readOnly ) {
    event.preventDefault();
    event.stopImmediatePropagation();

    return;
  }
  const transfer = event.dataTransfer;
  const internalImage = parseInternalImageDrag( transfer?.getData( NOTE_IMAGE_DRAG_MIME ) );
  const relativePath = transfer?.getData( VAULT_IMAGE_DRAG_MIME ).trim() ?? '';
  const attachmentRelativePath = transfer
    ?.getData( VAULT_ATTACHMENT_DRAG_MIME )
    .trim() ?? '';
  const types = Array.from( transfer?.types ?? []);
  const externalFiles = !internalImage
    && !relativePath
    && !attachmentRelativePath
    && types.includes( 'Files' );
  const view = editorView.value;
  if (
    !internalImage
    && !relativePath
    && !attachmentRelativePath
    && !externalFiles
  ) {
    return;
  }
  event.preventDefault();
  event.stopImmediatePropagation();
  if ( !view || !editorRenderReady.value ) {
    return;
  }
  const position = view.posAtCoords({ x: event.clientX, y: event.clientY }, false )
    ?? view.state.selection.main.head;
  if ( internalImage ) {
    moveImageReferenceWithinNote( view, internalImage.from, internalImage.to, position );

    return;
  }
  view.dispatch({
    selection: EditorSelection.cursor( position ),
    scrollIntoView: true,
    userEvent: 'select.pointer'
  });
  if ( externalFiles && transfer ) {
    const dropped = collectExternalDroppedFiles( transfer );
    const capture = captureAttachmentInsertion( view );
    if ( capture ) {
      emit( 'externalFileDrop', capture, dropped.files, dropped.rejectedCount );
    }

    return;
  }
  if ( attachmentRelativePath ) {
    const capture = captureAttachmentInsertion( view );
    if ( capture ) {
      emit( 'vaultAttachmentDrop', capture, attachmentRelativePath );
    }
  } else {
    const capture = captureImageInsertion( view );
    if ( !capture ) {
      return;
    }
    emit( 'vaultImageDrop', capture, relativePath );
  }
}

interface ExternalFileSystemEntry {
  isDirectory: boolean;
  isFile: boolean;
}

function isAmbiguousExternalDroppedFile( file: File ): boolean {
  // Without entry metadata, WebKit can represent an unavailable folder this way.
  return file.size === 0 && !file.type.trim();
}

function collectExternalDroppedFiles(
  transfer: DataTransfer
): { files: File[]; rejectedCount: number } {
  const items = Array.from( transfer.items ).filter( ( item ) => item.kind === 'file' );
  const files: File[] = [];
  let rejectedCount = 0;
  if ( items.length ) {
    for ( const item of items ) {
      const entry = ( item as DataTransferItem & {
        webkitGetAsEntry?: () => ExternalFileSystemEntry | null;
      }).webkitGetAsEntry?.();
      const file = entry?.isDirectory || ( entry && !entry.isFile )
        ? null
        : item.getAsFile();
      if (
        !file
        || ( !entry && isAmbiguousExternalDroppedFile( file ) )
        || files.length >= MAX_EXTERNAL_DROP_FILES
      ) {
        rejectedCount += 1;
      } else {
        files.push( file );
      }
    }

    return { files, rejectedCount };
  }

  for ( const file of Array.from( transfer.files ) ) {
    if (
      isAmbiguousExternalDroppedFile( file )
      || files.length >= MAX_EXTERNAL_DROP_FILES
    ) {
      rejectedCount += 1;
    } else {
      files.push( file );
    }
  }

  return { files, rejectedCount };
}

function parseInternalImageDrag(
  value: string | undefined
): { from: number; to: number } | undefined {
  if ( !value ) {
    return undefined;
  }
  try {
    const parsed = JSON.parse( value ) as { from?: unknown; to?: unknown };
    if (
      Number.isSafeInteger( parsed.from )
      && Number.isSafeInteger( parsed.to )
      && ( parsed.from as number ) >= 0
      && ( parsed.to as number ) > ( parsed.from as number )
    ) {
      return { from: parsed.from as number, to: parsed.to as number };
    }
  } catch {
    // Ignore malformed drag data from outside this editor.
  }

  return undefined;
}

function moveImageReferenceWithinNote(
  view: EditorView,
  from: number,
  to: number,
  position: number
): void {
  if (
    from < 0
    || to > view.state.doc.length
    || from >= to
    || ( position >= from && position <= to )
  ) {
    view.focus();

    return;
  }
  const source = view.state.sliceDoc( from, to );
  const image = parseMarkdownImageAt( source, 0 );
  if ( !image || image.end + 1 !== source.length ) {
    view.focus();

    return;
  }
  const insertionStart = position < from ? position : position - ( to - from );
  const changes = position < from
    ? [{ from: position, insert: source }, { from, to, insert: '' }]
    : [{ from, to, insert: '' }, { from: position, insert: source }];
  view.dispatch({
    changes,
    selection: EditorSelection.cursor( insertionStart + source.length ),
    scrollIntoView: true,
    userEvent: 'input.move'
  });
  view.focus();
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
    @dragover.capture="handleSourceEditorDragOver"
    @dragend.capture="clearExternalFileDrag"
    @drop.capture="handleSourceEditorDrop"
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
