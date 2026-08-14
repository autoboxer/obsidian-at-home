import { EditorView, WidgetType } from "@codemirror/view";
import type { LiveMarkdownBlock } from "./liveMarkdown";

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
    private readonly openWiki: (target: string) => void,
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
      if (target) {
        this.openWiki(target);
      }
    });

    return link;
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
  const position = event.clientX < bounds.left + bounds.width / 2 ? from : to;
  view.dispatch({
    selection: { anchor: position },
    scrollIntoView: true,
    userEvent: "select.pointer",
  });
  view.focus();
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
