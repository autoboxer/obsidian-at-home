import {
  ChangeSet,
  EditorSelection,
  EditorState,
  MapMode,
  StateEffect,
  StateField
} from '@codemirror/state';
import { Decoration, EditorView } from '@codemirror/view';
import { isMultiCursorGesture } from './codeMirrorMultiCursor';
import {
  markdownHeadingSlug,
  markdownHeadingText
} from './headingLinks';
import { liveMarkdownDocumentModel } from './liveMarkdownDocumentModel';
import { LiveMarkdownWidget } from './liveMarkdownCodeMirrorWidgets';
import type {
  ChangeDesc,
  ChangeSpec,
  Extension,
  SelectionRange,
  Text,
  Transaction,
  TransactionSpec
} from '@codemirror/state';
import type { DecorationSet, MouseSelectionStyle } from '@codemirror/view';

interface HeadingSection {
  bodyFrom: number;
  bodyTo: number;
  contentTo: number;
  from: number;
  hasContent: boolean;
  headingTo: number;
  key: string;
  label: string;
  level: number;
}

interface HeadingFoldState {
  atomicRanges: DecorationSet;
  collapsed: readonly CollapsedHeading[];
  decorations: DecorationSet;
}

interface HeadingSectionLookup {
  byFrom: ReadonlyMap<number, HeadingSection>;
  byHeadingTo: ReadonlyMap<number, HeadingSection>;
  sections: readonly HeadingSection[];
}

interface CollapsedHeading {
  bodyTo: number;
  from: number;
  headingTo: number;
  key: string;
}

interface SetHeadingCollapsed {
  collapsed: boolean;
  from: number;
}

const collapsedHeadingsByDocument = new Map<string, {
  document: Text;
  headings: readonly CollapsedHeading[];
}>();
const setHeadingCollapsedEffect = StateEffect.define<SetHeadingCollapsed>({
  map( value, changes ) {
    const from = changes.mapPos( value.from, 1, MapMode.TrackAfter );

    return from === null ? undefined : { ...value, from };
  }
});

export function liveMarkdownHeadingFoldingExtension(
  documentId: string
): Extension {
  const headingFoldState = StateField.define<HeadingFoldState>({
    create( state ) {
      const sections = headingSections( state );
      const persisted = collapsedHeadingsByDocument.get( documentId );
      const persistedKeys = new Set( persisted?.headings.map( ( heading ) => heading.key ) );
      const collapsed = normalizedCollapsedHeadings(
        sections,
        state.selection.ranges,
        persisted?.document.eq( state.doc )
          ? persisted.headings
          : sections
            .filter( ( section ) => persistedKeys.has( section.key ) )
            .map( ( section ) => collapsedHeading( section, foldBodyEnd( state, section ) ) )
      );
      persistCollapsedHeadings( documentId, state.doc, collapsed );

      return buildHeadingFoldState( sections, collapsed );
    },
    update( value, transaction ) {
      const foldEffects = transaction.effects.filter( ( effect ) =>
        effect.is( setHeadingCollapsedEffect )
      );
      const selectionMayRevealFold = Boolean(
        transaction.selection && value.collapsed.length
      );
      if (
        !transaction.docChanged
        && !foldEffects.length
        && !selectionMayRevealFold
      ) {
        return value;
      }

      const sections = headingSections( transaction.state );
      const sectionLookup = headingSectionLookup( sections );
      let requestedCollapsed = transaction.docChanged
        ? mappedCollapsedHeadings(
          sectionLookup,
          value.collapsed,
          transaction.changes,
          transaction.newDoc
        )
        : [ ...value.collapsed ];
      if ( foldEffects.length ) {
        for ( const effect of foldEffects ) {
          const section = headingSectionAtPosition(
            sectionLookup,
            effect.value.from
          );
          if ( !section ) {
            continue;
          }

          requestedCollapsed = requestedCollapsed.filter( ( heading ) =>
            heading.from !== section.from
          );
          if ( effect.value.collapsed ) {
            requestedCollapsed.push( collapsedHeading( section, foldBodyEnd( transaction.state, section ) ) );
          }
        }
      }

      const collapsed = normalizedCollapsedHeadings(
        sections,
        transaction.state.selection.ranges,
        requestedCollapsed
      );
      const collapsedChanged = !sameCollapsedHeadings(
        collapsed,
        value.collapsed
      );
      if ( collapsedChanged || transaction.docChanged ) {
        persistCollapsedHeadings( documentId, transaction.newDoc, collapsed );
      }
      if ( !transaction.docChanged && !collapsedChanged ) {
        return value;
      }

      return buildHeadingFoldState( sections, collapsed );
    },
    provide( field ) {
      return [
        EditorView.decorations.from( field, ( value ) => value.decorations ),
        EditorView.atomicRanges.of( ( view ) =>
          view.state.field( field ).atomicRanges
        )
      ];
    }
  });

  return [
    headingFoldState,
    EditorView.mouseSelectionStyle.of( ( view, event ) =>
      collapsedHeadingMouseSelection( view, event, headingFoldState )
    ),
    EditorState.transactionFilter.of( ( transaction ) =>
      separateFoldBoundaryInput( transaction, transaction.startState.field( headingFoldState ).collapsed )
    )
  ];
}

function collapsedHeadingMouseSelection(
  view: EditorView,
  event: MouseEvent,
  field: StateField<HeadingFoldState>
): MouseSelectionStyle | null {
  if (
    event.button !== 0 || event.detail < 3
    || event.ctrlKey || event.metaKey || ( event.altKey && event.shiftKey )
  ) {
    return null;
  }

  const lineAtEvent = ( pointer: MouseEvent ) => view.state.doc.lineAt(
    view.posAtCoords({ x: pointer.clientX, y: pointer.clientY }, false )
  );
  const initialLine = lineAtEvent( event );
  const heading = view.state.field( field ).collapsed.find( ( fold ) =>
    fold.from === initialLine.from
  );
  if ( !heading ) {
    return null;
  }

  // Use the stored end so text typed below the fold stays outside the selection.
  let start = EditorSelection.range( heading.from, heading.bodyTo );
  let startSelection = view.state.selection;

  return {
    get( currentEvent, extend, multiple ) {
      const line = lineAtEvent( currentEvent );
      const fold = view.state.field( field ).collapsed.find( ( item ) =>
        item.from === line.from
      );
      const end = fold?.bodyTo ?? Math.min( line.to + 1, view.state.doc.length );
      const current = EditorSelection.range( line.from, end );
      const range = extend ? current : current.from < start.from
        ? EditorSelection.range( start.to, current.from )
        : EditorSelection.range( start.from, Math.max( start.to, current.to ) );

      if ( extend ) {
        return startSelection.replaceRange(
          startSelection.main.extend( range.from, range.to, range.assoc )
        );
      }
      if ( multiple ) {
        return startSelection.addRange( range );
      }

      return EditorSelection.create([ range ]);
    },
    update( update ) {
      if ( update.docChanged ) {
        start = start.map( update.changes );
        startSelection = startSelection.map( update.changes );
      }

      return false;
    }
  };
}

function separateFoldBoundaryInput(
  transaction: Transaction,
  collapsed: readonly CollapsedHeading[]
): Transaction | readonly TransactionSpec[] {
  if ( !transaction.docChanged || !transaction.isUserEvent( 'input' ) || !collapsed.length ) {
    return transaction;
  }

  const changes: ChangeSpec[] = [];
  const movedCarets = new Map<number, number>();
  const sections = headingSectionLookup( headingSections( transaction.startState ) );
  transaction.changes.iterChanges( ( from, to, fromB, toB, inserted ) => {
    if ( from !== to || !inserted.length ) {
      return;
    }
    const heading = collapsed.find( ( fold ) =>
      sections.byFrom.get( fold.from )?.contentTo === from
    );
    if ( heading && inserted.toString() === '\n' ) {
      // Enter on a closed heading creates a line after the hidden body.
      const end = transaction.changes.mapPos( heading.bodyTo, -1 );
      changes.push({ from: fromB, to: toB }, { from: end, insert: '\n' });
      movedCarets.set( toB, end );
    } else if (
      collapsed.some( ( fold ) => fold.bodyTo === from )
      && transaction.startState.doc.lineAt( from ).from !== from
      && inserted.sliceString( 0, 1 ) !== '\n'
    ) {
      // A block fold can display an empty row at EOF without a source newline.
      changes.push({ from: fromB, insert: '\n' });
    }
  });
  if ( !changes.length ) {
    return transaction;
  }
  const adjustment = ChangeSet.of( changes, transaction.newDoc.length );

  return [ transaction, {
    changes: adjustment,
    selection: EditorSelection.create( transaction.newSelection.ranges.map( ( range ) => {
      const moved = range.empty ? movedCarets.get( range.head ) : undefined;

      if ( moved === undefined ) {
        return range.map( adjustment );
      }

      // Stay on the new blank line, before any following content. At an
      // unterminated body end, the newline must come before the caret instead.
      const atLineStart = transaction.newDoc.lineAt( moved ).from === moved;

      return EditorSelection.cursor( adjustment.mapPos( moved, atLineStart ? -1 : 1 ) );
    }), transaction.newSelection.mainIndex ),
    sequential: true
  }];
}

function buildHeadingFoldState(
  sections: readonly HeadingSection[],
  collapsed: readonly CollapsedHeading[]
): HeadingFoldState {
  const decorations = [];
  const atomicRanges = [];
  const collapsedPositions = new Map( collapsed.map( ( heading ) => [ heading.from, heading ]) );
  let hiddenUntil = 0;

  for ( const section of sections ) {
    if ( section.from < hiddenUntil ) {
      continue;
    }
    if ( !section.hasContent ) {
      continue;
    }

    decorations.push(
      Decoration.widget({
        side: -100,
        widget: new HeadingFoldWidget(
          section,
          collapsedPositions.has( section.from )
        )
      }).range( section.from )
    );

    if ( !collapsedPositions.has( section.from ) ) {
      continue;
    }

    const bodyTo = collapsedPositions.get( section.from )!.bodyTo;
    const folded = Decoration.replace({
      block: true,
      inclusiveStart: true,
      inclusiveEnd: false
    }).range( section.bodyFrom, bodyTo );
    decorations.push( folded );
    atomicRanges.push(
      Decoration.mark({}).range( section.contentTo, bodyTo )
    );
    hiddenUntil = bodyTo;
  }

  return {
    atomicRanges: Decoration.set( atomicRanges, true ),
    collapsed,
    decorations: Decoration.set( decorations, true )
  };
}

function normalizedCollapsedHeadings(
  sections: readonly HeadingSection[],
  selections: readonly SelectionRange[],
  requested: readonly CollapsedHeading[]
): readonly CollapsedHeading[] {
  const collapsibleSections = new Map(
    sections
      .filter( ( section ) => section.hasContent )
      .map( ( section ) => [ section.from, section ])
  );
  const collapsed: CollapsedHeading[] = [];
  const seenPositions = new Set<number>();

  for ( const heading of requested ) {
    const currentSection = collapsibleSections.get( heading.from );
    const section = currentSection && {
      ...currentSection,
      bodyTo: Math.min( heading.bodyTo, currentSection.bodyTo )
    };
    if (
      section
      && section.bodyTo > section.bodyFrom
      && !seenPositions.has( section.from )
      && !selections.some( ( range ) =>
        selectionRevealsSectionBody( range, section )
      )
    ) {
      collapsed.push( collapsedHeading( section ) );
      seenPositions.add( section.from );
    }
  }

  collapsed.sort( ( first, second ) => first.from - second.from );

  return sameCollapsedHeadings( collapsed, requested ) ? requested : collapsed;
}

function mappedCollapsedHeadings(
  sectionLookup: HeadingSectionLookup,
  collapsed: readonly CollapsedHeading[],
  changes: ChangeDesc,
  document: Text
): CollapsedHeading[] {
  return collapsed.flatMap( ( heading ) => {
    const mappedFrom = changes.mapPos(
      heading.from,
      1,
      MapMode.TrackAfter
    );
    const mappedTo = changes.mapPos(
      heading.headingTo,
      -1,
      MapMode.TrackBefore
    );
    const fromSection = mappedFrom === null
      ? undefined
      : headingSectionAtPosition( sectionLookup, mappedFrom );
    const toSection = mappedTo === null
      ? undefined
      : headingSectionAtPosition( sectionLookup, mappedTo );

    if ( fromSection && toSection && fromSection.from !== toSection.from ) {
      return [];
    }

    const section = fromSection ?? toSection;

    let bodyTo = changes.mapPos( heading.bodyTo, -1 );
    const boundaryLine = document.lineAt( bodyTo );
    if ( bodyTo < document.length && boundaryLine.from < bodyTo && boundaryLine.to === bodyTo ) {
      bodyTo += 1;
    }

    return section ? [ collapsedHeading( section, bodyTo ) ] : [];
  });
}

function headingSectionAtPosition(
  lookup: HeadingSectionLookup,
  position: number
): HeadingSection | undefined {
  return lookup.byFrom.get( position )
    ?? lookup.byHeadingTo.get( position )
    ?? lookup.sections.find( ( section ) =>
      section.from < position && position <= section.headingTo
    );
}

function headingSectionLookup(
  sections: readonly HeadingSection[]
): HeadingSectionLookup {
  return {
    byFrom: new Map( sections.map( ( section ) => [ section.from, section ]) ),
    byHeadingTo: new Map(
      sections.map( ( section ) => [ section.headingTo, section ])
    ),
    sections
  };
}

function collapsedHeading( section: HeadingSection, bodyTo = section.bodyTo ): CollapsedHeading {
  return {
    bodyTo,
    from: section.from,
    headingTo: section.headingTo,
    key: section.key
  };
}

function foldBodyEnd( state: EditorState, section: HeadingSection ): number {
  let end = section.bodyTo;
  while ( end > section.bodyFrom ) {
    const line = state.doc.lineAt( end - 1 );
    if ( line.text.trim() ) {
      return Math.min( section.bodyTo, line.to + 1 );
    }
    end = line.from;
  }

  return end;
}

function headingSections( state: EditorState ): HeadingSection[] {
  const sections: HeadingSection[] = [];
  const openSections: HeadingSection[] = [];
  const occurrences = new Map<string, number>();

  const closeSectionsThroughLevel = ( level: number, bodyTo: number ): void => {
    while ( ( openSections.at( -1 )?.level ?? 0 ) >= level ) {
      openSections.pop()!.bodyTo = bodyTo;
    }
  };

  for ( const block of liveMarkdownDocumentModel( state ).blocks ) {
    if ( block.type !== 'heading' || !block.headingLevel ) {
      if ( block.type !== 'blank' ) {
        const currentSection = openSections.at( -1 );
        if ( currentSection ) {
          currentSection.hasContent = true;
        }
      }

      continue;
    }

    closeSectionsThroughLevel( block.headingLevel, block.from );
    const parentSection = openSections.at( -1 );
    if ( parentSection ) {
      parentSection.hasContent = true;
    }

    const source = state.sliceDoc( block.content.from, block.content.to );
    const slug = markdownHeadingSlug( source );
    const occurrenceKey = `${ block.headingLevel }:${ slug }`;
    const occurrence = ( occurrences.get( occurrenceKey ) ?? 0 ) + 1;
    occurrences.set( occurrenceKey, occurrence );

    const section: HeadingSection = {
      bodyFrom: block.end,
      bodyTo: block.end,
      contentTo: block.content.to,
      from: block.from,
      hasContent: false,
      headingTo: block.to,
      key: `h${ block.headingLevel }:${ slug }:${ occurrence }`,
      label: markdownHeadingText( source ) || 'heading',
      level: block.headingLevel
    };
    sections.push( section );
    openSections.push( section );
  }

  closeSectionsThroughLevel( 1, state.doc.length );

  return sections;
}

function persistCollapsedHeadings(
  documentId: string,
  document: Text,
  collapsed: readonly CollapsedHeading[]
): void {
  if ( collapsed.length ) {
    collapsedHeadingsByDocument.set(
      documentId,
      { document, headings: collapsed }
    );
  } else {
    collapsedHeadingsByDocument.delete( documentId );
  }
}

function sameCollapsedHeadings(
  first: readonly CollapsedHeading[],
  second: readonly CollapsedHeading[]
): boolean {
  return first.length === second.length && first.every( ( heading, index ) =>
    heading.from === second[ index ]?.from
    && heading.bodyTo === second[ index ]?.bodyTo
    && heading.headingTo === second[ index ]?.headingTo
    && heading.key === second[ index ]?.key
  );
}

function selectionRevealsSectionBody(
  selection: SelectionRange,
  section: HeadingSection
): boolean {
  return selection.empty
    ? selection.head >= section.bodyFrom && selection.head < section.bodyTo
    : selection.from < section.bodyTo && selection.to > section.bodyFrom
      // Selecting a whole section need not reveal it; targeting its body does.
      && !( selection.from <= section.from && selection.to >= section.bodyTo );
}

class HeadingFoldWidget extends LiveMarkdownWidget {
  constructor(
    private readonly section: HeadingSection,
    private readonly collapsed: boolean
  ) {
    super();
  }

  eq( other: HeadingFoldWidget ): boolean {
    return this.section.key === other.section.key
      && this.section.from === other.section.from
      && this.section.bodyFrom === other.section.bodyFrom
      && this.section.bodyTo === other.section.bodyTo
      && this.section.contentTo === other.section.contentTo
      && this.section.label === other.section.label
      && this.collapsed === other.collapsed;
  }

  toDOM( view: EditorView ): HTMLElement {
    const document = view.dom.ownerDocument;
    const button = document.createElement( 'button' );
    const chevron = document.createElement( 'span' );
    const action = this.collapsed ? 'Expand' : 'Collapse';

    button.type = 'button';
    button.className = 'live-heading-toggle';
    button.setAttribute( 'aria-expanded', String( !this.collapsed ) );
    button.setAttribute( 'aria-label', `${ action } ${ this.section.label } section` );
    button.title = `${ action } section`;
    chevron.className = 'live-heading-chevron';
    chevron.setAttribute( 'aria-hidden', 'true' );
    button.append( chevron );

    button.addEventListener( 'mousedown', ( event ) => {
      if ( isMultiCursorGesture( event ) ) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
    });
    button.addEventListener( 'click', ( event ) => {
      event.preventDefault();
      event.stopPropagation();
      if ( isMultiCursorGesture( event ) ) {
        return;
      }
      const willCollapse = !this.collapsed;

      view.dispatch({
        effects: setHeadingCollapsedEffect.of({
          collapsed: willCollapse,
          from: this.section.from
        }),
        selection: EditorSelection.cursor( this.section.contentTo ),
        userEvent: 'select.heading-fold'
      });
      view.focus();
    });

    return button;
  }
}
