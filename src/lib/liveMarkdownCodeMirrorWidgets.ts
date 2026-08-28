import { EditorView, WidgetType } from "@codemirror/view";
import { NOTE_IMAGE_DRAG_MIME } from "./imageEmbeds";
import {
  markdownAttachmentIsArchive,
  markdownAttachmentIsExecutable,
  markdownAttachmentPresentation,
  type MarkdownAttachmentMetadata,
  type ParsedMarkdownAttachment,
} from "./markdownAttachments";
import type { Rect } from "@codemirror/view";
import type { LiveMarkdownBlock } from "./liveMarkdown";
import type { ParsedMarkdownImage } from "./markdownImages";

const UNORDERED_LIST_MARKERS = ["•", "◦", "▪"] as const;
const ROMAN_NUMERALS: ReadonlyArray<readonly [number, string]> = [
  [1_000, "m"],
  [900, "cm"],
  [500, "d"],
  [400, "cd"],
  [100, "c"],
  [90, "xc"],
  [50, "l"],
  [40, "xl"],
  [10, "x"],
  [9, "ix"],
  [5, "v"],
  [4, "iv"],
  [1, "i"],
];

export class ListMarkerWidget extends WidgetType {
  constructor(
    private readonly source: string,
    private readonly marker: string,
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: ListMarkerWidget): boolean {
    return this.source === other.source &&
      this.marker === other.marker &&
      this.from === other.from &&
      this.to === other.to;
  }

  coordsAt(dom: HTMLElement, pos: number, side: number): Rect | null {
    return listControlCoordinates(dom, pos, side);
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const control = document.createElement("span");
    const prefix = document.createElement("span");
    const marker = document.createElement("span");
    control.className = "live-list-control";
    control.setAttribute("aria-hidden", "true");
    prefix.className = "live-list-prefix";
    prefix.textContent = this.source;
    marker.className = "live-list-marker";
    marker.textContent = this.marker;
    control.append(prefix, marker);
    control.addEventListener("mousedown", (event) =>
      revealWidgetSource(view, marker, event, this.from, this.to)
    );

    return control;
  }
}

export class TaskWidget extends WidgetType {
  constructor(
    private readonly source: string,
    private readonly checked: boolean,
    private readonly checkFrom: number,
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: TaskWidget): boolean {
    return this.source === other.source &&
      this.checked === other.checked &&
      this.checkFrom === other.checkFrom &&
      this.from === other.from &&
      this.to === other.to;
  }

  coordsAt(dom: HTMLElement, pos: number, side: number): Rect | null {
    return listControlCoordinates(dom, pos, side);
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const control = document.createElement("span");
    const marker = document.createElement("span");
    const checkbox = document.createElement("button");
    control.className = "live-task-control";
    marker.className = "live-task-marker";
    marker.setAttribute("aria-hidden", "true");
    marker.textContent = this.source;
    checkbox.className = "live-task-checkbox";
    checkbox.type = "button";
    checkbox.tabIndex = -1;
    checkbox.setAttribute(
      "aria-label",
      this.checked ? "Mark task incomplete" : "Mark task complete",
    );
    checkbox.setAttribute("aria-pressed", String(this.checked));
    if (this.checked) {
      checkbox.append(createCheckIcon(document));
    }
    control.addEventListener("mousedown", (event) =>
      revealWidgetSource(view, control, event, this.from, this.to)
    );
    checkbox.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    checkbox.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      view.dispatch({
        changes: {
          from: this.checkFrom,
          to: this.checkFrom + 1,
          insert: this.checked ? " " : "x",
        },
        userEvent: "input",
      });
      view.focus();
    });
    control.append(marker, checkbox);

    return control;
  }
}

export class QuoteMarkerWidget extends WidgetType {
  constructor(
    private readonly source: string,
    private readonly depth: number,
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: QuoteMarkerWidget): boolean {
    return this.source === other.source &&
      this.depth === other.depth &&
      this.from === other.from &&
      this.to === other.to;
  }

  toDOM(view: EditorView): HTMLElement {
    const element = view.dom.ownerDocument.createElement("span");
    element.className = "live-quote-control";
    element.dataset.depth = String(Math.min(this.depth, 3));
    element.setAttribute("aria-hidden", "true");
    element.textContent = this.source;
    element.addEventListener("mousedown", (event) =>
      revealWidgetSource(view, element, event, this.from, this.to)
    );

    return element;
  }
}

export class HorizontalRuleWidget extends WidgetType {
  constructor(
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: HorizontalRuleWidget): boolean {
    return this.from === other.from && this.to === other.to;
  }

  toDOM(view: EditorView): HTMLElement {
    const element = view.dom.ownerDocument.createElement("span");
    element.className = "live-horizontal-rule";
    element.setAttribute("aria-hidden", "true");
    element.addEventListener("mousedown", (event) =>
      revealWidgetSource(view, element, event, this.from, this.to)
    );

    return element;
  }
}

export class WikiLinkWidget extends WidgetType {
  constructor(
    private readonly display: string,
    private readonly target: string,
    private readonly heading: string | undefined,
    private readonly embedded: boolean,
    private readonly resolved: boolean,
    private readonly openWiki: (target: string, heading?: string) => void,
    private readonly from: number,
    private readonly to: number,
    private readonly resolutionVersion: number,
  ) {
    super();
  }

  eq(other: WikiLinkWidget): boolean {
    return this.display === other.display &&
      this.target === other.target &&
      this.heading === other.heading &&
      this.embedded === other.embedded &&
      this.resolved === other.resolved &&
      this.openWiki === other.openWiki &&
      this.from === other.from &&
      this.to === other.to &&
      this.resolutionVersion === other.resolutionVersion;
  }

  toDOM(view: EditorView): HTMLElement {
    const link = view.dom.ownerDocument.createElement("a");
    link.className = [
      "live-inline-segment",
      "is-wiki-link",
      ...(this.heading ? ["is-heading-link"] : []),
      this.resolved ? "is-resolved" : "is-unresolved",
    ].join(" ");
    link.href = "#";
    link.rel = "noopener noreferrer";
    link.textContent = this.display;
    link.dataset.wikiTarget = this.target;
    if (this.heading) {
      link.dataset.wikiHeading = this.heading;
    }
    if (this.embedded) {
      link.dataset.embedded = "true";
    }
    link.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    link.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const target = this.target.trim();
      if (target || this.heading) {
        this.openWiki(target, this.heading);
      }
    });

    return link;
  }
}

export type MarkdownImageSourceResolver = (
  image: ParsedMarkdownImage,
) => Promise<string | null | undefined> | string | null | undefined;

export type MarkdownAttachmentMetadataResolver = (
  attachment: ParsedMarkdownAttachment,
) => MarkdownAttachmentMetadata | null | undefined;

export type MarkdownAttachmentAction = (
  attachment: ParsedMarkdownAttachment,
  metadata: MarkdownAttachmentMetadata | null | undefined,
) => void;

export class MarkdownAttachmentWidget extends WidgetType {
  constructor(
    private readonly attachment: ParsedMarkdownAttachment,
    private readonly metadata: MarkdownAttachmentMetadata | null | undefined,
    private readonly from: number,
    private readonly to: number,
    private readonly resolutionVersion: number,
    private readonly activateAttachment?: MarkdownAttachmentAction,
  ) {
    super();
  }

  eq(other: MarkdownAttachmentWidget): boolean {
    return this.attachment.raw === other.attachment.raw
      && this.metadata?.byteLength === other.metadata?.byteLength
      && this.metadata?.mediaType === other.metadata?.mediaType
      && this.metadata?.openingDisabled === other.metadata?.openingDisabled
      && this.metadata?.relativePath === other.metadata?.relativePath
      && this.from === other.from
      && this.to === other.to
      && this.resolutionVersion === other.resolutionVersion
      && this.activateAttachment === other.activateAttachment;
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const presentation = markdownAttachmentPresentation(
      this.attachment,
      this.metadata ?? undefined,
    );
    const card = document.createElement("span");
    const icon = document.createElement("span");
    const copy = document.createElement("span");
    const name = document.createElement("span");
    const details = document.createElement("span");
    const action = document.createElement("button");
    const archive = markdownAttachmentIsArchive(
      this.metadata?.relativePath ?? this.attachment.destination,
      this.metadata?.mediaType,
    );
    const executable = markdownAttachmentIsExecutable(
      this.metadata?.relativePath ?? this.attachment.destination,
      this.metadata?.openingDisabled,
    );

    card.className = "live-attachment-card";
    card.dataset.attachmentAssetId = this.attachment.assetId ?? "";
    card.dataset.attachmentDestination = this.attachment.destination;
    card.setAttribute("contenteditable", "false");
    card.setAttribute("role", "group");
    card.setAttribute(
      "aria-label",
      `${presentation.name}, ${presentation.typeLabel}, ${presentation.sizeLabel}`,
    );
    icon.className = "attachment-card__icon";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = presentation.iconLabel;
    copy.className = "attachment-card__copy";
    name.className = "attachment-card__name";
    name.textContent = presentation.name;
    details.className = "attachment-card__details";
    details.textContent = `${presentation.typeLabel} · ${presentation.sizeLabel}`;
    copy.append(name, details);
    action.className = "attachment-card__action";
    action.type = "button";
    action.disabled = executable;
    action.textContent = executable
      ? "Unavailable"
      : archive ? "Save a copy…" : "Open";
    action.title = executable
      ? "Opening executable or installer attachments is not supported"
      : archive ? "Save a copy outside the vault" : "Open with the default application";
    action.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    action.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (!executable) {
        this.activateAttachment?.(this.attachment, this.metadata);
      }
    });
    card.append(icon, copy, action);
    card.addEventListener("mousedown", (event) => {
      if (event.button === 0) {
        event.stopPropagation();
        view.focus();
      }
    });
    card.addEventListener("click", (event) =>
      revealWidgetSource(view, card, event, this.from, this.to)
    );

    return card;
  }
}

export class MarkdownImageWidget extends WidgetType {
  constructor(
    private readonly image: ParsedMarkdownImage,
    private readonly resolveSource: MarkdownImageSourceResolver | undefined,
    private readonly from: number,
    private readonly to: number,
    private readonly resolutionVersion: number,
  ) {
    super();
  }

  eq(other: MarkdownImageWidget): boolean {
    return this.image.raw === other.image.raw &&
      this.resolveSource === other.resolveSource &&
      this.from === other.from &&
      this.to === other.to &&
      this.resolutionVersion === other.resolutionVersion;
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const frame = document.createElement("span");
    const image = document.createElement("img");
    frame.className = "live-embedded-image is-loading";
    frame.dataset.imageAssetId = this.image.assetId ?? "";
    frame.dataset.imageDestination = this.image.destination;
    frame.setAttribute("contenteditable", "false");
    frame.draggable = true;
    image.alt = this.image.alt;
    image.className = "live-embedded-image__content";
    image.decoding = "async";
    image.draggable = false;
    image.loading = "lazy";
    if (this.image.title) {
      image.title = this.image.title;
    }
    if (this.image.width) {
      image.style.width = `${this.image.width}px`;
    }
    if (this.image.height) {
      image.style.height = `${this.image.height}px`;
    }
    image.addEventListener("load", () => {
      frame.classList.remove("is-loading", "is-error");
    }, { once: true });
    image.addEventListener("error", () => {
      frame.classList.remove("is-loading");
      frame.classList.add("is-error");
      frame.setAttribute(
        "aria-label",
        this.image.alt ? `Could not load image: ${this.image.alt}` : "Could not load image",
      );
    }, { once: true });
    frame.addEventListener("mousedown", (event) => {
      if (event.button === 0) {
        // Keep CodeMirror from replacing the widget before the browser can
        // decide whether this pointer gesture is a click or a native drag.
        event.stopPropagation();
        view.focus();
      }
    });
    frame.addEventListener("click", (event) =>
      revealWidgetSource(view, frame, event, this.from, this.to)
    );
    frame.addEventListener("dragstart", (event) => {
      if (!event.dataTransfer) {
        return;
      }
      event.dataTransfer.clearData();
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData(NOTE_IMAGE_DRAG_MIME, JSON.stringify({
        from: this.from,
        to: this.to,
      }));
      event.dataTransfer.setData("text/plain", this.image.raw);
      frame.classList.add("is-dragging");
    });
    frame.addEventListener("dragend", () => frame.classList.remove("is-dragging"));
    frame.append(image);

    void Promise.resolve()
      .then(() => this.resolveSource?.(this.image) ?? this.image.destination)
      .then((source) => {
        if (source) {
          image.src = source;
        } else {
          image.dispatchEvent(new Event("error"));
        }
      })
      .catch(() => image.dispatchEvent(new Event("error")));

    return frame;
  }
}

export function renderedListMarker(block: LiveMarkdownBlock): string {
  if (!block.list) {
    return "";
  }
  if (!block.list.ordered) {
    return UNORDERED_LIST_MARKERS[block.list.depth % 3]!;
  }

  const number = block.list.number ?? 1;
  if (block.list.depth % 3 === 1) {
    return `${alphabeticListMarker(number)}.`;
  }
  if (block.list.depth % 3 === 2) {
    return `${romanListMarker(number)}.`;
  }

  return `${number}.`;
}

function revealWidgetSource(
  view: EditorView,
  element: HTMLElement,
  event: MouseEvent,
  from: number,
  to: number,
): void {
  event.preventDefault();
  event.stopPropagation();
  const bounds = element.getBoundingClientRect();
  const approachFromLeft = event.clientX < bounds.left + bounds.width / 2;
  const position = approachFromLeft ? from : to;
  view.focus();
  view.dispatch({
    selection: { anchor: position },
    scrollIntoView: true,
    userEvent: "select.pointer",
  });
}

function listControlCoordinates(
  dom: HTMLElement,
  pos: number,
  side: number,
): Rect | null {
  if (pos > 0) {
    return null;
  }

  const line = dom.closest<HTMLElement>(".cm-line");
  if (!line) {
    return null;
  }

  const lineBounds = line.getBoundingClientRect();
  const strongSide = Math.abs(side) > 1;
  const horizontalBounds = strongSide
    ? dom.closest(".cm-content")?.getBoundingClientRect() ?? lineBounds
    : lineBounds;
  const lineHeight = Number.parseFloat(
    dom.ownerDocument.defaultView?.getComputedStyle(line).lineHeight ?? "",
  );
  const lineBottom = Number.isFinite(lineHeight)
    ? Math.min(lineBounds.bottom, lineBounds.top + lineHeight)
    : lineBounds.bottom;
  // Drawn selections otherwise fill the leading gap above this widget line,
  // leaving a thin strip beside the selected list item.
  const lineTop = strongSide
    ? previousLineTextBottom(line, lineBounds.top)
    : lineBounds.top;

  return {
    left: horizontalBounds.left,
    right: horizontalBounds.left,
    top: Math.min(lineTop, lineBottom),
    bottom: lineBottom,
  };
}

function previousLineTextBottom(
  line: HTMLElement,
  fallback: number,
): number {
  const previousLine = line.previousElementSibling;
  if (!(previousLine instanceof HTMLElement)) {
    return fallback;
  }

  const range = line.ownerDocument.createRange();
  range.selectNodeContents(previousLine);
  const bottoms = [...range.getClientRects()]
    .filter((bounds) => bounds.width > 0 && bounds.height > 0)
    .map((bounds) => bounds.bottom)
    .filter((bottom) => bottom <= fallback + 0.5);

  return bottoms.length ? Math.max(...bottoms) : fallback;
}

function createCheckIcon(document: Document): SVGSVGElement {
  const namespace = "http://www.w3.org/2000/svg";
  const icon = document.createElementNS(namespace, "svg");
  const path = document.createElementNS(namespace, "path");
  icon.classList.add("app-icon");
  icon.setAttribute("width", "9");
  icon.setAttribute("height", "9");
  icon.setAttribute("viewBox", "0 0 24 24");
  icon.setAttribute("fill", "none");
  icon.setAttribute("stroke", "currentColor");
  icon.setAttribute("stroke-width", "2.4");
  icon.setAttribute("stroke-linecap", "round");
  icon.setAttribute("stroke-linejoin", "round");
  icon.setAttribute("aria-hidden", "true");
  path.setAttribute("d", "m5 12 4 4L19 6");
  icon.append(path);

  return icon;
}

function alphabeticListMarker(number: number): string {
  if (number < 1) {
    return String(number);
  }

  let value = number;
  let marker = "";
  while (value > 0) {
    value -= 1;
    marker = String.fromCharCode(97 + (value % 26)) + marker;
    value = Math.floor(value / 26);
  }

  return marker;
}

function romanListMarker(number: number): string {
  if (number < 1 || number > 3_999) {
    return String(number);
  }

  let remaining = number;
  let marker = "";
  for (const [value, numeral] of ROMAN_NUMERALS) {
    while (remaining >= value) {
      marker += numeral;
      remaining -= value;
    }
  }

  return marker;
}
