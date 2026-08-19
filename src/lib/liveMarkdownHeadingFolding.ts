import {
  EditorSelection,
  MapMode,
  StateEffect,
  StateField,
} from "@codemirror/state";
import {
  Decoration,
  EditorView,
  WidgetType,
} from "@codemirror/view";
import {
  markdownHeadingSlug,
  markdownHeadingText,
} from "./headingLinks";
import { liveMarkdownDocumentModel } from "./liveMarkdownDocumentModel";
import type {
  ChangeDesc,
  EditorState,
  Extension,
  SelectionRange,
} from "@codemirror/state";
import type { DecorationSet } from "@codemirror/view";

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
  parentKey?: string;
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
  from: number;
  headingTo: number;
  key: string;
}

interface SetHeadingCollapsed {
  collapsed: boolean;
  from: number;
}

const collapsedHeadingsByDocument = new Map<string, Set<string>>();
const setHeadingCollapsedEffect = StateEffect.define<SetHeadingCollapsed>({
  map(value, changes) {
    const from = changes.mapPos(value.from, 1, MapMode.TrackAfter);

    return from === null ? undefined : { ...value, from };
  },
});

export function liveMarkdownHeadingFoldingExtension(
  documentId: string,
): Extension {
  const headingFoldState = StateField.define<HeadingFoldState>({
    create(state) {
      const sections = headingSections(state);
      const persistedKeys = collapsedHeadingsByDocument.get(documentId);
      const collapsed = normalizedCollapsedHeadings(
        sections,
        state.selection.ranges,
        persistedKeys
          ? sections
            .filter((section) => persistedKeys.has(section.key))
            .map(collapsedHeading)
          : [],
      );
      persistCollapsedHeadings(documentId, collapsed);

      return buildHeadingFoldState(sections, collapsed);
    },
    update(value, transaction) {
      const foldEffects = transaction.effects.filter((effect) =>
        effect.is(setHeadingCollapsedEffect)
      );
      const selectionMayRevealFold = Boolean(
        transaction.selection && value.collapsed.length,
      );
      if (
        !transaction.docChanged
        && !foldEffects.length
        && !selectionMayRevealFold
      ) {
        return value;
      }

      const sections = headingSections(transaction.state);
      const sectionLookup = headingSectionLookup(sections);
      let requestedCollapsed = transaction.docChanged
        ? mappedCollapsedHeadings(
            sectionLookup,
            value.collapsed,
            transaction.changes,
          )
        : [...value.collapsed];
      if (foldEffects.length) {
        for (const effect of foldEffects) {
          const section = headingSectionAtPosition(
            sectionLookup,
            effect.value.from,
          );
          if (!section) {
            continue;
          }

          requestedCollapsed = requestedCollapsed.filter((heading) =>
            heading.from !== section.from
          );
          if (effect.value.collapsed) {
            requestedCollapsed.push(collapsedHeading(section));
          }
        }
      }

      const collapsed = normalizedCollapsedHeadings(
        sections,
        transaction.state.selection.ranges,
        requestedCollapsed,
      );
      const collapsedChanged = !sameCollapsedHeadings(
        collapsed,
        value.collapsed,
      );
      if (collapsedChanged) {
        persistCollapsedHeadings(documentId, collapsed);
      }
      if (!transaction.docChanged && !collapsedChanged) {
        return value;
      }

      return buildHeadingFoldState(sections, collapsed);
    },
    provide(field) {
      return [
        EditorView.decorations.from(field, (value) => value.decorations),
        EditorView.atomicRanges.of((view) =>
          view.state.field(field).atomicRanges
        ),
      ];
    },
  });

  return headingFoldState;
}

function buildHeadingFoldState(
  sections: readonly HeadingSection[],
  collapsed: readonly CollapsedHeading[],
): HeadingFoldState {
  const decorations = [];
  const atomicRanges = [];
  const collapsedPositions = new Set(collapsed.map((heading) => heading.from));
  const hiddenSections = new Set<string>();

  for (const section of sections) {
    const hiddenByParent = section.parentKey
      ? hiddenSections.has(section.parentKey)
      : false;
    if (hiddenByParent) {
      hiddenSections.add(section.key);

      continue;
    }
    if (!section.hasContent) {
      continue;
    }

    decorations.push(
      Decoration.widget({
        side: -100,
        widget: new HeadingFoldWidget(
          section,
          collapsedPositions.has(section.from),
        ),
      }).range(section.from),
    );

    if (!collapsedPositions.has(section.from)) {
      continue;
    }

    const folded = Decoration.replace({
      block: true,
      inclusive: false,
    }).range(section.bodyFrom, section.bodyTo);
    decorations.push(folded);
    atomicRanges.push(
      Decoration.mark({}).range(section.contentTo, section.bodyTo),
    );
    hiddenSections.add(section.key);
  }

  return {
    atomicRanges: Decoration.set(atomicRanges, true),
    collapsed,
    decorations: Decoration.set(decorations, true),
  };
}

function normalizedCollapsedHeadings(
  sections: readonly HeadingSection[],
  selections: readonly SelectionRange[],
  requested: readonly CollapsedHeading[],
): readonly CollapsedHeading[] {
  const collapsibleSections = new Map(
    sections
      .filter((section) => section.hasContent)
      .map((section) => [section.from, section]),
  );
  const collapsed: CollapsedHeading[] = [];
  const seenPositions = new Set<number>();

  for (const heading of requested) {
    const section = collapsibleSections.get(heading.from);
    if (
      section
      && !seenPositions.has(section.from)
      && !selections.some((range) =>
        selectionHeadIsInSectionBody(range, section)
      )
    ) {
      collapsed.push(collapsedHeading(section));
      seenPositions.add(section.from);
    }
  }

  collapsed.sort((first, second) => first.from - second.from);

  return sameCollapsedHeadings(collapsed, requested) ? requested : collapsed;
}

function mappedCollapsedHeadings(
  sectionLookup: HeadingSectionLookup,
  collapsed: readonly CollapsedHeading[],
  changes: ChangeDesc,
): CollapsedHeading[] {
  return collapsed.flatMap((heading) => {
    const mappedFrom = changes.mapPos(
      heading.from,
      1,
      MapMode.TrackAfter,
    );
    const mappedTo = changes.mapPos(
      heading.headingTo,
      -1,
      MapMode.TrackBefore,
    );
    const fromSection = mappedFrom === null
      ? undefined
      : headingSectionAtPosition(sectionLookup, mappedFrom);
    const toSection = mappedTo === null
      ? undefined
      : headingSectionAtPosition(sectionLookup, mappedTo);

    if (fromSection && toSection && fromSection.from !== toSection.from) {
      return [];
    }

    const section = fromSection ?? toSection;

    return section ? [collapsedHeading(section)] : [];
  });
}

function headingSectionAtPosition(
  lookup: HeadingSectionLookup,
  position: number,
): HeadingSection | undefined {
  return lookup.byFrom.get(position)
    ?? lookup.byHeadingTo.get(position)
    ?? lookup.sections.find((section) =>
      section.from < position && position <= section.headingTo
    );
}

function headingSectionLookup(
  sections: readonly HeadingSection[],
): HeadingSectionLookup {
  return {
    byFrom: new Map(sections.map((section) => [section.from, section])),
    byHeadingTo: new Map(
      sections.map((section) => [section.headingTo, section]),
    ),
    sections,
  };
}

function collapsedHeading(section: HeadingSection): CollapsedHeading {
  return {
    from: section.from,
    headingTo: section.headingTo,
    key: section.key,
  };
}

function headingSections(state: EditorState): HeadingSection[] {
  const sections: HeadingSection[] = [];
  const openSections: HeadingSection[] = [];
  const occurrences = new Map<string, number>();

  const closeSectionsThroughLevel = (level: number, bodyTo: number): void => {
    while ((openSections.at(-1)?.level ?? 0) >= level) {
      openSections.pop()!.bodyTo = bodyTo;
    }
  };

  for (const block of liveMarkdownDocumentModel(state).blocks) {
    if (block.type !== "heading" || !block.headingLevel) {
      if (block.type !== "blank") {
        const currentSection = openSections.at(-1);
        if (currentSection) {
          currentSection.hasContent = true;
        }
      }

      continue;
    }

    closeSectionsThroughLevel(block.headingLevel, block.from);
    const parentSection = openSections.at(-1);
    if (parentSection) {
      parentSection.hasContent = true;
    }

    const source = state.sliceDoc(block.content.from, block.content.to);
    const slug = markdownHeadingSlug(source);
    const occurrenceKey = `${block.headingLevel}:${slug}`;
    const occurrence = (occurrences.get(occurrenceKey) ?? 0) + 1;
    occurrences.set(occurrenceKey, occurrence);

    const section: HeadingSection = {
      bodyFrom: block.end,
      bodyTo: block.end,
      contentTo: block.content.to,
      from: block.from,
      hasContent: false,
      headingTo: block.to,
      key: `h${block.headingLevel}:${slug}:${occurrence}`,
      label: markdownHeadingText(source) || "heading",
      level: block.headingLevel,
      ...(parentSection ? { parentKey: parentSection.key } : {}),
    };
    sections.push(section);
    openSections.push(section);
  }

  closeSectionsThroughLevel(1, state.doc.length);

  return sections;
}

function persistCollapsedHeadings(
  documentId: string,
  collapsed: readonly CollapsedHeading[],
): void {
  if (collapsed.length) {
    collapsedHeadingsByDocument.set(
      documentId,
      new Set(collapsed.map((heading) => heading.key)),
    );
  } else {
    collapsedHeadingsByDocument.delete(documentId);
  }
}

function sameCollapsedHeadings(
  first: readonly CollapsedHeading[],
  second: readonly CollapsedHeading[],
): boolean {
  return first.length === second.length && first.every((heading, index) =>
    heading.from === second[index]?.from
    && heading.headingTo === second[index]?.headingTo
    && heading.key === second[index]?.key
  );
}

function selectionHeadIsInSectionBody(
  selection: SelectionRange,
  section: HeadingSection,
): boolean {
  return selection.head >= section.bodyFrom && selection.head < section.bodyTo;
}

function selectionTouchesSectionBody(
  selection: SelectionRange,
  section: HeadingSection,
): boolean {
  return selection.empty
    ? selection.head >= section.bodyFrom && selection.head < section.bodyTo
    : selection.from < section.bodyTo && selection.to > section.bodyFrom;
}

class HeadingFoldWidget extends WidgetType {
  constructor(
    private readonly section: HeadingSection,
    private readonly collapsed: boolean,
  ) {
    super();
  }

  eq(other: HeadingFoldWidget): boolean {
    return this.section.key === other.section.key
      && this.section.from === other.section.from
      && this.section.bodyFrom === other.section.bodyFrom
      && this.section.bodyTo === other.section.bodyTo
      && this.section.contentTo === other.section.contentTo
      && this.section.label === other.section.label
      && this.collapsed === other.collapsed;
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const button = document.createElement("button");
    const chevron = document.createElement("span");
    const action = this.collapsed ? "Expand" : "Collapse";

    button.type = "button";
    button.className = "live-heading-toggle";
    button.setAttribute("aria-expanded", String(!this.collapsed));
    button.setAttribute("aria-label", `${action} ${this.section.label} section`);
    button.title = `${action} section`;
    chevron.className = "live-heading-chevron";
    chevron.setAttribute("aria-hidden", "true");
    button.append(chevron);

    button.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const willCollapse = !this.collapsed;
      const selectionTouchesBody = willCollapse && view.state.selection.ranges.some(
        (range) => selectionTouchesSectionBody(range, this.section),
      );

      view.dispatch({
        effects: setHeadingCollapsedEffect.of({
          collapsed: willCollapse,
          from: this.section.from,
        }),
        ...(selectionTouchesBody
          ? { selection: EditorSelection.cursor(this.section.contentTo) }
          : {}),
        scrollIntoView: true,
        userEvent: "select.heading-fold",
      });
      view.focus();
    });

    return button;
  }
}
