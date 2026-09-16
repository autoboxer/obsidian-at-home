import { ref, type Ref } from 'vue';
import { syntaxTreeAvailable } from '@codemirror/language';
import { EditorSelection } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { markdownBodyStart } from '../lib/frontmatter';
import { registerNoteEditorPositionCapture } from '../stores/editorPositions';
import type { NoteEditorPosition } from '../types';

const VIEWPORT_ANCHOR_MARGIN = 8;
const VIRTUALIZED_VIEWPORT_THRESHOLD = 200;

export function useCodeMirrorViewport(
  props: { vaultId: string; noteId: string },
  emit: ( event: 'editorPosition', vaultId: string, noteId: string, position: NoteEditorPosition ) => void,
  editorView: Ref<EditorView | undefined>,
  getFrontmatterPrefix: () => string
) {
  const editorRenderReady = ref( false );
  let positionCaptureEnabled = false;
  let removePositionCapture: ( () => boolean ) | undefined;
  let viewportRestoreFrame: number | undefined;
  const positionCaptureKey = {};
  const renderReadyKey = {};
  const viewportRestoreKey = {};

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
      getFrontmatterPrefix(),
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
      getFrontmatterPrefix(),
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

  function registerEditorPosition( view: EditorView ): void {
    removePositionCapture = registerNoteEditorPositionCapture(
      props.vaultId,
      props.noteId,
      () => positionCaptureEnabled ? captureEditorPosition( view ) : undefined
    );
    view.scrollDOM.addEventListener( 'scroll', handleEditorScroll, { passive: true });
  }

  function restoreEditorPosition(
    view: EditorView,
    initialPosition: NoteEditorPosition | undefined
  ): void {
    if ( initialPosition ) {
      scheduleViewportRestore( view, initialPosition );
    } else {
      positionCaptureEnabled = true;
      schedulePositionCapture( view );
      scheduleEditorRenderReady( view );
    }
  }

  function disposeEditorPosition(): void {
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
    }
  }

  return {
    disposeEditorPosition,
    editorRenderReady,
    focusDocumentOffset,
    registerEditorPosition,
    restoreEditorPosition,
    scheduleEditorRenderReady,
    schedulePositionCapture
  };
}
