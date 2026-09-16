import { syntaxTree } from '@codemirror/language';
import { Decoration } from '@codemirror/view';
import { parseMarkdownHeadingTarget } from './headingLinks';
import { parsePairedInlineMarkup } from './inlineMarkup';
import {
  MarkdownAttachmentWidget,
  MarkdownImageWidget,
  WikiLinkWidget
} from './liveMarkdownCodeMirrorWidgets';
import {
  addConstruct,
  addMarkDecoration,
  rangeIsContainedByAny,
  rangeOverlapsAny,
  rangesCross
} from './liveMarkdownDecorationModel';
import { liveMarkdownDocumentModel } from './liveMarkdownDocumentModel';
import { sanitizeImageUrl, sanitizeLinkUrl } from './markdown';
import { parseMarkdownAttachmentAt } from './markdownAttachments';
import { parseMarkdownImageAt } from './markdownImages';
import { parseWikiLinks } from './wikiLinks';
import type { EditorState, Text } from '@codemirror/state';
import type { SyntaxNodeRef } from '@lezer/common';
import type { InlineMarkupKind } from './inlineMarkup';
import type { LiveMarkdownRange } from './liveMarkdown';
import type { MarkdownImageSourceResolver } from './liveMarkdownCodeMirrorWidgets';
import type { LiveMarkdownModel, LiveMarkdownOptions } from './liveMarkdownDecorationModel';

export interface InlineMarkupSpan extends LiveMarkdownRange {
  contentFrom: number;
  contentTo: number;
  kind: InlineMarkupKind | 'code';
  syntax: LiveMarkdownRange[];
}

const defaultMarkdownImageSource: MarkdownImageSourceResolver = ( image ) =>
  sanitizeImageUrl( image.destination );
const inlineMarkupSpanCache = new WeakMap<Text, InlineMarkupSpan[]>();

export function inlineMarkupSpans(
  state: EditorState,
  from: number,
  to: number
): InlineMarkupSpan[] {
  const cached = inlineMarkupSpanCache.get( state.doc );
  if ( cached ) {
    return cached.filter( ( span ) => span.from <= to && span.to >= from );
  }

  const value = state.doc.toString();
  const blockExclusions = liveMarkdownDocumentModel( state ).blocks.flatMap(
    ( block ) => {
      if (
        block.type === 'code' ||
        block.type === 'frontmatter' ||
        block.type === 'horizontal-rule'
      ) {
        return [{ from: block.from, to: block.end }];
      }

      return block.syntax;
    }
  );
  const wikiExclusions = parseWikiLinks( value ).map( ( link ) => ({
    from: link.index,
    to: link.index + link.raw.length
  }) );
  const syntaxSpans = syntaxTreeInlineMarkupSpans( state ).filter( ( span ) =>
    !rangeIsContainedByAny( span, blockExclusions ) &&
    !rangeOverlapsAny( span, wikiExclusions )
  );
  const spans: InlineMarkupSpan[] = [];

  spans.push(
    ...syntaxSpans,
    ...pairedInlineMarkupSpans(
      state,
      value,
      [ ...blockExclusions, ...wikiExclusions ],
      syntaxSpans
    )
  );
  inlineMarkupSpanCache.set( state.doc, spans );

  return spans.filter( ( span ) => span.from <= to && span.to >= from );
}

function syntaxTreeInlineMarkupSpans(
  state: EditorState
): InlineMarkupSpan[] {
  const spans: InlineMarkupSpan[] = [];

  syntaxTree( state ).iterate({
    enter( reference ) {
      const span = inlineMarkupSpan( reference, state );
      if ( span ) {
        spans.push( span );
      }

      return undefined;
    }
  });

  return spans;
}

function inlineMarkupSpan(
  node: SyntaxNodeRef,
  state: EditorState
): InlineMarkupSpan | undefined {
  const markup = node.name === 'InlineCode'
    ? { kind: 'code' as const, markerName: 'CodeMark' }
    : node.name === 'StrongEmphasis'
      ? { kind: 'strong' as const, markerName: 'EmphasisMark' }
      : node.name === 'Emphasis'
        ? { kind: 'emphasis' as const, markerName: 'EmphasisMark' }
        : node.name === 'Strikethrough'
          ? { kind: 'strikethrough' as const, markerName: 'StrikethroughMark' }
          : undefined;
  if ( !markup ) {
    return undefined;
  }

  const markers = node.node.getChildren( markup.markerName );
  const opening = markers[ 0 ];
  const closing = markers.at( -1 );
  if ( !opening || !closing || opening === closing || opening.to > closing.from ) {
    return undefined;
  }

  const syntax: LiveMarkdownRange[] = [
    { from: opening.from, to: opening.to },
    { from: closing.from, to: closing.to }
  ];
  let contentFrom = opening.to;
  let contentTo = closing.from;
  if ( node.name === 'InlineCode' ) {
    const rawContent = state.sliceDoc( contentFrom, contentTo );
    if ( /\r|\n/.test( rawContent ) ) {
      return undefined;
    }
    if ( /^\s.*\s$/.test( rawContent ) && rawContent.trim() ) {
      syntax.push(
        { from: contentFrom, to: contentFrom + 1 },
        { from: contentTo - 1, to: contentTo }
      );
      contentFrom += 1;
      contentTo -= 1;
    }
  }

  return {
    from: node.from,
    to: node.to,
    contentFrom,
    contentTo,
    kind: markup.kind,
    syntax
  };
}

function pairedInlineMarkupSpans(
  state: EditorState,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  syntaxSpans: readonly InlineMarkupSpan[]
): InlineMarkupSpan[] {
  const protectedRanges = [
    ...excludedRanges,
    ...inlineMarkupSyntaxExclusions( state )
  ];

  return parsePairedInlineMarkup( value, protectedRanges )
    .map( ( span ): InlineMarkupSpan => ({
      contentFrom: span.contentFrom,
      contentTo: span.contentTo,
      from: span.from,
      kind: span.kind,
      syntax: [
        { from: span.from, to: span.contentFrom },
        { from: span.contentTo, to: span.to }
      ],
      to: span.to
    }) )
    .filter( ( span ) => !syntaxSpans.some( ( syntaxSpan ) =>
      inlineMarkupSpansMatch( span, syntaxSpan ) || rangesCross( span, syntaxSpan )
    ) );
}

function inlineMarkupSyntaxExclusions(
  state: EditorState
): LiveMarkdownRange[] {
  const exclusions: LiveMarkdownRange[] = [];

  syntaxTree( state ).iterate({
    enter( reference ) {
      if (
        reference.name === 'Autolink' ||
        reference.name === 'Escape' ||
        reference.name === 'HTMLTag' ||
        reference.name === 'Image' ||
        reference.name === 'InlineCode'
      ) {
        exclusions.push({ from: reference.from, to: reference.to });

        return false;
      }
      if ( reference.name !== 'Link' ) {
        return undefined;
      }

      const labelClosing = reference.node.getChildren( 'LinkMark' )[ 1 ];
      exclusions.push( labelClosing
        ? { from: labelClosing.from, to: reference.to }
        : { from: reference.from, to: reference.to });

      return undefined;
    }
  });

  return exclusions;
}

function inlineMarkupSpansMatch(
  left: InlineMarkupSpan,
  right: InlineMarkupSpan
): boolean {
  return left.from === right.from &&
    left.to === right.to &&
    left.kind === right.kind;
}

export function supportedMarkdownLinkRanges(
  state: EditorState,
  excludedRanges: readonly LiveMarkdownRange[]
): LiveMarkdownRange[] {
  const ranges: LiveMarkdownRange[] = [];

  syntaxTree( state ).iterate({
    enter( reference ) {
      const range = { from: reference.from, to: reference.to };
      if ( rangeIsContainedByAny( range, excludedRanges ) ) {
        return false;
      }
      if (
        reference.name !== 'Autolink' &&
        reference.name !== 'Image' &&
        reference.name !== 'Link'
      ) {
        return undefined;
      }

      const markers = reference.node.getChildren( 'LinkMark' );
      const requiredMarkers = reference.name === 'Autolink' ? 2 : 4;
      if (
        markers.length >= requiredMarkers &&
        reference.node.getChild( 'URL' )
      ) {
        ranges.push( range );
      }

      return undefined;
    }
  });

  return ranges;
}

export function addWikiLinkDecorations(
  model: LiveMarkdownModel,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  markdownLinkRanges: readonly LiveMarkdownRange[],
  options: LiveMarkdownOptions,
  wikiResolutionVersion: number
): LiveMarkdownRange[] {
  const ranges: LiveMarkdownRange[] = [];

  for ( const link of parseWikiLinks( value ) ) {
    const range = { from: link.index, to: link.index + link.raw.length };
    if (
      rangeOverlapsAny( range, excludedRanges ) ||
      rangeOverlapsAny( range, markdownLinkRanges )
    ) {
      continue;
    }

    ranges.push( range );
    addConstruct( model, range.from, range.to, [{
      ...range,
      widget: new WikiLinkWidget(
        link.display || link.heading || link.target,
        link.target,
        link.heading,
        link.embedded,
        options.wikiLinkIsResolved( link.target ),
        options.openWiki,
        range.from,
        range.to,
        wikiResolutionVersion
      )
    }]);
  }

  return ranges;
}

export function addInlineDecorations(
  model: LiveMarkdownModel,
  state: EditorState,
  value: string,
  excludedRanges: readonly LiveMarkdownRange[],
  wikiRanges: readonly LiveMarkdownRange[],
  inlineMarkupExcludedRanges: readonly LiveMarkdownRange[],
  options: LiveMarkdownOptions,
  resolutionVersion: number
): void {
  const syntaxSpans: InlineMarkupSpan[] = [];

  syntaxTree( state ).iterate({
    enter( reference ) {
      const node = reference;
      const range = { from: node.from, to: node.to };
      if ( rangeIsContainedByAny( range, excludedRanges ) ) {
        return false;
      }

      if ( node.name === 'Escape' ) {
        if ( !rangeOverlapsAny( range, wikiRanges ) && node.to - node.from >= 2 ) {
          addConstruct( model, node.from, node.to, [{
            from: node.from,
            to: node.from + 1
          }]);
        }

        return false;
      }
      if ( rangeOverlapsAny( range, wikiRanges ) ) {
        return undefined;
      }
      if ( node.name === 'Image' ) {
        addMarkdownImageDecoration(
          model,
          node,
          value,
          options,
          resolutionVersion
        );

        return false;
      }
      if (
        node.name === 'Link'
        && addMarkdownAttachmentDecoration(
          model,
          node,
          value,
          options,
          resolutionVersion
        )
      ) {
        return false;
      }
      const span = inlineMarkupSpan( node, state );
      if ( span ) {
        syntaxSpans.push( span );
        addInlineMarkupDecoration( model, span );
      } else if (
        node.name === 'Autolink' ||
        node.name === 'Link'
      ) {
        addMarkdownLinkDecoration( model, node, value );
      }

      return undefined;
    }
  });

  const pairedSpans = pairedInlineMarkupSpans(
    state,
    value,
    inlineMarkupExcludedRanges,
    syntaxSpans
  );
  inlineMarkupSpanCache.set( state.doc, [ ...syntaxSpans, ...pairedSpans ]);

  for ( const span of pairedSpans ) {
    addInlineMarkupDecoration( model, span );
  }
}

function addMarkdownAttachmentDecoration(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  value: string,
  options: LiveMarkdownOptions,
  resolutionVersion: number
): boolean {
  const attachment = parseMarkdownAttachmentAt( value, node.from, {
    acceptExtensionless: options.acceptExtensionlessAttachment
  });
  if ( !attachment || attachment.end + 1 !== node.to ) {
    return false;
  }

  const range = { from: node.from, to: node.to };
  const metadata = options.resolveAttachmentMetadata?.( attachment );
  addConstruct( model, range.from, range.to, [{
    ...range,
    widget: new MarkdownAttachmentWidget(
      attachment,
      metadata,
      range.from,
      range.to,
      resolutionVersion,
      options.activateAttachment,
      options.renameAttachment,
      options.revealAttachmentInTree,
      options.showAttachmentInFolder
    )
  }], [], {
    atomicRanges: [ range ],
    boundaryReveal: 'construct',
    revealWhenSelectedWithin: true
  });

  return true;
}

function addMarkdownImageDecoration(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  value: string,
  options: LiveMarkdownOptions,
  resolutionVersion: number
): void {
  const image = parseMarkdownImageAt( value, node.from );
  if ( !image || image.end + 1 !== node.to ) {
    return;
  }

  const resolveSource = options.resolveImageSource ?? defaultMarkdownImageSource;
  const range = { from: node.from, to: node.to };
  addConstruct( model, range.from, range.to, [{
    ...range,
    widget: new MarkdownImageWidget(
      image,
      resolveSource,
      range.from,
      range.to,
      resolutionVersion
    )
  }], [], {
    atomicRanges: [ range ],
    boundaryReveal: 'construct',
    revealWhenSelectedWithin: true
  });
}

function addInlineMarkupDecoration(
  model: LiveMarkdownModel,
  span: InlineMarkupSpan
): void {
  addConstruct( model, span.from, span.to, span.syntax, [], {
    boundaryReveal: 'construct',
    revealWhenSelectedWithin: true
  });
  addMarkDecoration(
    model,
    { from: span.contentFrom, to: span.contentTo },
    `live-inline-segment is-${ span.kind }`
  );
}

function addMarkdownLinkDecoration(
  model: LiveMarkdownModel,
  node: SyntaxNodeRef,
  value: string
): void {
  const markers = node.node.getChildren( 'LinkMark' );
  const urlNode = node.node.getChild( 'URL' );
  const autolink = node.name === 'Autolink';
  if ( ( autolink ? markers.length < 2 : markers.length < 4 ) || !urlNode ) {
    return;
  }

  const labelOpening = markers[ 0 ]!;
  const labelClosing = autolink ? markers.at( -1 )! : markers[ 1 ]!;
  if ( labelOpening.to > labelClosing.from ) {
    return;
  }

  const rawHref = value.slice( urlNode.from, urlNode.to );
  const href = sanitizeLinkUrl(
    autolink && /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test( rawHref )
      ? `mailto:${ rawHref }`
      : rawHref
  );
  const titleNode = node.node.getChild( 'LinkTitle' );
  const title = titleNode
    ? unwrapLinkTitle( value.slice( titleNode.from, titleNode.to ) )
    : undefined;
  const attributes: Record<string, string> = {};
  if ( href ) {
    attributes.href = href;
    attributes[ 'data-live-href' ] = href;
    attributes.rel = 'noopener noreferrer';
    if ( /^(?:https?:)?\/\//i.test( href ) ) {
      attributes.target = '_blank';
    }
    if ( title ) {
      attributes.title = title;
    }
  }

  addConstruct( model, node.from, node.to, [
    { from: labelOpening.from, to: labelOpening.to },
    {
      from: labelClosing.from,
      to: autolink ? labelClosing.to : node.to
    }
  ], [], {
    boundaryReveal: 'construct',
    revealWhenSelectedWithin: true
  });
  if ( labelOpening.to < labelClosing.from ) {
    const className = href
      ? [
        'live-inline-segment',
        'is-link',
        ...( parseMarkdownHeadingTarget( href ) ? [ 'is-heading-link' ] : [])
      ].join( ' ' )
      : 'live-inline-segment is-unsafe-link';
    model.decorations.push({
      from: labelOpening.to,
      to: labelClosing.from,
      decoration: Decoration.mark({
        class: className,
        ...( href
          ? {
            attributes,
            tagName: 'a'
          }
          : {})
      })
    });
  }
}

function unwrapLinkTitle( value: string ): string {
  const opening = value[ 0 ];
  const closing = value.at( -1 );
  if (
    ( opening === '"' && closing === '"' ) ||
    ( opening === "'" && closing === "'" ) ||
    ( opening === '(' && closing === ')' )
  ) {
    return value.slice( 1, -1 );
  }

  return value;
}
