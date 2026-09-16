import { Decoration } from '@codemirror/view';
import type { WidgetType } from '@codemirror/view';
import type { LiveMarkdownRange } from './liveMarkdown';
import type {
  MarkdownAttachmentAction,
  MarkdownAttachmentMetadataResolver,
  MarkdownAttachmentRenameAction,
  MarkdownImageSourceResolver
} from './liveMarkdownCodeMirrorWidgets';

export interface LiveMarkdownOptions {
  readonly acceptExtensionlessAttachment?: ( destination: string ) => boolean;
  readonly activateAttachment?: MarkdownAttachmentAction;
  readonly documentId: string;
  readonly openLink: ( href: string ) => void;
  readonly openWiki: ( target: string, heading?: string ) => void;
  readonly renameAttachment?: MarkdownAttachmentRenameAction;
  readonly revealAttachmentInTree?: MarkdownAttachmentAction;
  readonly resolveAttachmentMetadata?: MarkdownAttachmentMetadataResolver;
  readonly resolveImageSource?: MarkdownImageSourceResolver;
  readonly showAttachmentInFolder?: MarkdownAttachmentAction;
  readonly wikiLinkIsResolved: ( target: string ) => boolean;
}

export interface HiddenSyntax extends LiveMarkdownRange {
  widget?: WidgetType;
}

export interface LiveConstruct {
  atomicRanges: LiveMarkdownRange[];
  boundaryReveal: 'construct' | 'none' | 'syntax';
  from: number;
  renderedDecorations: StoredDecoration[];
  revealWhenSelectedWithin: boolean;
  revealWithinSyntax: boolean;
  to: number;
  syntax: HiddenSyntax[];
}

interface LiveConstructOptions {
  atomicRanges?: readonly LiveMarkdownRange[];
  boundaryReveal?: LiveConstruct[ 'boundaryReveal' ];
  revealWhenSelectedWithin?: boolean;
  revealWithinSyntax?: boolean;
}

export interface StoredDecoration extends LiveMarkdownRange {
  decoration: Decoration;
}

export interface LiveMarkdownModel {
  constructs: LiveConstruct[];
  decorations: StoredDecoration[];
}

export function addConstruct(
  model: LiveMarkdownModel,
  from: number,
  to: number,
  syntax: readonly HiddenSyntax[],
  renderedDecorations: readonly StoredDecoration[] = [],
  options: LiveConstructOptions = {}
): void {
  const nonemptySyntax = syntax.filter( ( range ) => range.from < range.to );
  if ( !nonemptySyntax.length ) {
    return;
  }

  model.constructs.push({
    atomicRanges: [ ...( options.atomicRanges ?? []) ],
    boundaryReveal: options.boundaryReveal ?? 'syntax',
    from,
    renderedDecorations: [ ...renderedDecorations ],
    revealWhenSelectedWithin: options.revealWhenSelectedWithin ?? false,
    revealWithinSyntax: options.revealWithinSyntax ?? true,
    to,
    syntax: nonemptySyntax
  });
}

export function addLineDecoration(
  model: LiveMarkdownModel,
  from: number,
  className: string
): void {
  model.decorations.push( lineDecoration( from, className ) );
}

export function lineDecoration(
  from: number,
  className: string
): StoredDecoration {
  return {
    from,
    to: from,
    decoration: Decoration.line({ class: className })
  };
}

export function addMarkDecoration(
  model: LiveMarkdownModel,
  range: LiveMarkdownRange,
  className: string
): void {
  if ( range.from >= range.to ) {
    return;
  }

  model.decorations.push({
    ...range,
    decoration: Decoration.mark({ class: className })
  });
}

export function rangeOverlapsAny(
  range: LiveMarkdownRange,
  candidates: readonly LiveMarkdownRange[]
): boolean {
  return candidates.some( ( candidate ) => rangesOverlap( range, candidate ) );
}

export function rangeIsContainedByAny(
  range: LiveMarkdownRange,
  candidates: readonly LiveMarkdownRange[]
): boolean {
  return candidates.some( ( candidate ) =>
    range.from >= candidate.from && range.to <= candidate.to
  );
}

export function rangesOverlap(
  first: LiveMarkdownRange,
  second: LiveMarkdownRange
): boolean {
  return first.from < second.to && first.to > second.from;
}

export function rangesCross(
  first: LiveMarkdownRange,
  second: LiveMarkdownRange
): boolean {
  return (
    first.from < second.from &&
    second.from < first.to &&
    first.to < second.to
  ) || (
    second.from < first.from &&
    first.from < second.to &&
    second.to < first.to
  );
}
