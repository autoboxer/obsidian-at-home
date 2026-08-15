import { EditorSelection } from "@codemirror/state";
import { EditorView, WidgetType } from "@codemirror/view";
import {
  CODE_LANGUAGE_OPTIONS,
  findCodeLanguageOption,
} from "./highlight";
import type { CodeLanguageOption } from "./highlight";
import type { LiveMarkdownCodeFence } from "./liveMarkdownCode";

export class CodeFenceHeaderWidget extends WidgetType {
  constructor(
    private readonly fence: LiveMarkdownCodeFence,
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: CodeFenceHeaderWidget): boolean {
    return this.fence.language === other.fence.language &&
      this.fence.info.from === other.fence.info.from &&
      this.fence.info.to === other.fence.info.to &&
      this.fence.languageRange?.from === other.fence.languageRange?.from &&
      this.fence.languageRange?.to === other.fence.languageRange?.to &&
      this.from === other.from &&
      this.to === other.to;
  }

  toDOM(view: EditorView): HTMLElement {
    const document = view.dom.ownerDocument;
    const root = document.createElement("span");
    const button = document.createElement("button");
    const label = document.createElement("span");
    const picker = document.createElement("span");
    const searchLabel = document.createElement("label");
    const search = document.createElement("input");
    const options = document.createElement("span");
    const listboxId = `code-languages-${this.from}-${this.to}`;
    let activeIndex = 0;
    let filteredOptions = [...CODE_LANGUAGE_OPTIONS];

    root.className = "live-code-header-widget";
    root.addEventListener("mousedown", (event) => {
      if (event.target === root) {
        revealSource(view, event, this.from, this.to);
      }
    });

    button.type = "button";
    button.className = "live-code-language-button";
    button.setAttribute("aria-haspopup", "listbox");
    button.setAttribute("aria-expanded", "false");
    label.textContent = codeLanguageLabel(this.fence.language);
    button.append(label, createChevronIcon(document));

    picker.className = "live-code-language-picker";
    picker.hidden = true;
    picker.addEventListener("mousedown", (event) => event.stopPropagation());

    searchLabel.className = "live-code-language-search";
    searchLabel.append(createSearchIcon(document), search);
    search.type = "search";
    search.setAttribute("role", "combobox");
    search.placeholder = "Filter languages…";
    search.setAttribute("aria-label", "Filter code languages");
    search.setAttribute("aria-autocomplete", "list");
    search.setAttribute("aria-controls", listboxId);
    search.setAttribute("aria-expanded", "true");

    options.id = listboxId;
    options.className = "live-code-language-options";
    options.setAttribute("role", "listbox");
    options.setAttribute("aria-label", "Code language");

    const closePicker = (restoreFocus = false): void => {
      picker.hidden = true;
      button.setAttribute("aria-expanded", "false");
      search.value = "";
      if (restoreFocus) {
        button.focus();
      }
    };

    const setActiveOption = (index: number): void => {
      activeIndex = Math.max(0, Math.min(index, filteredOptions.length - 1));
      renderLanguageOptions();
      const active = options.children.item(activeIndex);
      if (active instanceof HTMLElement) {
        active.scrollIntoView({ block: "nearest", inline: "nearest" });
        search.setAttribute("aria-activedescendant", active.id);
      } else {
        search.removeAttribute("aria-activedescendant");
      }
    };

    const chooseLanguage = (option: CodeLanguageOption): void => {
      const range = option.value
        ? this.fence.languageRange ?? this.fence.info
        : this.fence.info;
      const insert = option.value
        ? this.fence.languageRange ? option.value : ` ${option.value}`
        : "";
      closePicker();
      view.dispatch({
        changes: { from: range.from, to: range.to, insert },
        userEvent: "input.code-language",
      });
      view.focus();
    };

    const renderLanguageOptions = (): void => {
      if (!filteredOptions.length) {
        const empty = document.createElement("span");
        empty.className = "live-code-language-empty";
        empty.setAttribute("role", "status");
        empty.textContent = "No matching languages";
        options.replaceChildren(empty);

        return;
      }

      options.replaceChildren(...filteredOptions.map((option, index) => {
        const optionButton = document.createElement("button");
        const optionLabel = document.createElement("span");
        optionButton.id = `${listboxId}-option-${index}`;
        optionButton.type = "button";
        optionButton.setAttribute("role", "option");
        optionButton.setAttribute("aria-selected", String(index === activeIndex));
        optionButton.tabIndex = -1;
        optionButton.classList.toggle("active", index === activeIndex);
        optionLabel.textContent = option.label;
        optionButton.append(optionLabel);
        if (findCodeLanguageOption(this.fence.language)?.value === option.value) {
          optionButton.append(createCheckIcon(document));
        }
        optionButton.addEventListener("mouseenter", () => {
          activeIndex = index;
          for (const [childIndex, child] of [...options.children].entries()) {
            child.classList.toggle("active", childIndex === activeIndex);
            child.setAttribute("aria-selected", String(childIndex === activeIndex));
          }
          search.setAttribute("aria-activedescendant", optionButton.id);
        });
        optionButton.addEventListener("click", () => chooseLanguage(option));

        return optionButton;
      }));
    };

    const resetOptions = (): void => {
      const query = search.value.trim().toLocaleLowerCase();
      filteredOptions = CODE_LANGUAGE_OPTIONS.filter((option) =>
        !query || [option.label, option.value, ...option.aliases]
          .join(" ")
          .toLocaleLowerCase()
          .includes(query)
      );
      const selectedValue = findCodeLanguageOption(this.fence.language)?.value;
      const selectedIndex = query
        ? -1
        : filteredOptions.findIndex((option) => option.value === selectedValue);
      setActiveOption(Math.max(0, selectedIndex));
    };

    button.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const opening = picker.hidden;
      if (!opening) {
        closePicker();

        return;
      }

      picker.hidden = false;
      button.setAttribute("aria-expanded", "true");
      resetOptions();
      search.focus();
    });
    search.addEventListener("input", resetOptions);
    search.addEventListener("keydown", (event) => {
      if (event.isComposing) {
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        closePicker(true);

        return;
      }
      if (!filteredOptions.length) {
        return;
      }
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setActiveOption((activeIndex + 1) % filteredOptions.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setActiveOption(
          (activeIndex - 1 + filteredOptions.length) % filteredOptions.length,
        );
      } else if (event.key === "Home") {
        event.preventDefault();
        setActiveOption(0);
      } else if (event.key === "End") {
        event.preventDefault();
        setActiveOption(filteredOptions.length - 1);
      } else if (event.key === "Enter") {
        event.preventDefault();
        chooseLanguage(filteredOptions[activeIndex]!);
      }
    });
    root.addEventListener("focusout", (event) => {
      if (event.relatedTarget instanceof Node && root.contains(event.relatedTarget)) {
        return;
      }
      closePicker();
    });

    picker.append(searchLabel, options);
    root.append(button, picker);
    renderLanguageOptions();

    return root;
  }
}

export class CodeFenceFooterWidget extends WidgetType {
  constructor(
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: CodeFenceFooterWidget): boolean {
    return this.from === other.from && this.to === other.to;
  }

  toDOM(view: EditorView): HTMLElement {
    const footer = view.dom.ownerDocument.createElement("span");
    footer.className = "live-code-footer-widget";
    footer.setAttribute("aria-hidden", "true");
    footer.addEventListener("mousedown", (event) =>
      revealSource(view, event, this.from, this.to)
    );

    return footer;
  }
}

export class TableDelimiterWidget extends WidgetType {
  constructor(
    private readonly from: number,
    private readonly to: number,
  ) {
    super();
  }

  eq(other: TableDelimiterWidget): boolean {
    return this.from === other.from && this.to === other.to;
  }

  toDOM(view: EditorView): HTMLElement {
    const divider = view.dom.ownerDocument.createElement("span");
    divider.className = "live-table-divider";
    divider.setAttribute("aria-hidden", "true");
    divider.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      view.dispatch({
        selection: EditorSelection.cursor(
          Math.min(this.to - 1, this.from + Math.floor((this.to - this.from) / 2)),
        ),
        scrollIntoView: true,
        userEvent: "select.pointer",
      });
      view.focus();
    });

    return divider;
  }
}

export class EmptyTableCellWidget extends WidgetType {
  constructor(
    private readonly position: number,
    private readonly columnIndex: number,
    private readonly className: string,
  ) {
    super();
  }

  eq(other: EmptyTableCellWidget): boolean {
    return this.position === other.position &&
      this.columnIndex === other.columnIndex &&
      this.className === other.className;
  }

  toDOM(view: EditorView): HTMLElement {
    const cell = view.dom.ownerDocument.createElement("span");
    cell.className = [
      this.className,
      "is-empty",
    ].join(" ");
    cell.dataset.columnIndex = String(this.columnIndex);
    cell.textContent = "\u00a0";
    cell.setAttribute("aria-hidden", "true");
    cell.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      view.dispatch({
        selection: EditorSelection.cursor(this.position),
        scrollIntoView: true,
        userEvent: "select.pointer",
      });
      view.focus();
    });

    return cell;
  }

  coordsAt(dom: HTMLElement): DOMRect | null {
    const text = dom.firstChild;
    if (!text) {
      return null;
    }

    const range = dom.ownerDocument.createRange();
    range.setStart(text, 0);
    range.collapse(true);

    return range.getBoundingClientRect();
  }
}

function codeLanguageLabel(language: string): string {
  return findCodeLanguageOption(language)?.label || language || "Plain text";
}

function revealSource(
  view: EditorView,
  event: MouseEvent,
  from: number,
  to: number,
): void {
  event.preventDefault();
  event.stopPropagation();
  const element = event.currentTarget;
  const bounds = element instanceof Element
    ? element.getBoundingClientRect()
    : undefined;
  const position = bounds && event.clientX >= bounds.left + bounds.width / 2
    ? to
    : from;
  view.dispatch({
    selection: EditorSelection.cursor(position),
    scrollIntoView: true,
    userEvent: "select.pointer",
  });
  view.focus();
}

function createChevronIcon(document: Document): SVGSVGElement {
  return createIcon(document, "M6 9l6 6 6-6", 11);
}

function createSearchIcon(document: Document): SVGSVGElement {
  return createIcon(document, "m21 21-4.35-4.35M11 19a8 8 0 1 1 0-16 8 8 0 0 1 0 16Z", 13);
}

function createCheckIcon(document: Document): SVGSVGElement {
  return createIcon(document, "m5 12 4 4L19 6", 12);
}

function createIcon(
  document: Document,
  pathData: string,
  size: number,
): SVGSVGElement {
  const namespace = "http://www.w3.org/2000/svg";
  const icon = document.createElementNS(namespace, "svg");
  const path = document.createElementNS(namespace, "path");
  icon.classList.add("app-icon");
  icon.setAttribute("width", String(size));
  icon.setAttribute("height", String(size));
  icon.setAttribute("viewBox", "0 0 24 24");
  icon.setAttribute("fill", "none");
  icon.setAttribute("stroke", "currentColor");
  icon.setAttribute("stroke-width", "2");
  icon.setAttribute("stroke-linecap", "round");
  icon.setAttribute("stroke-linejoin", "round");
  icon.setAttribute("aria-hidden", "true");
  path.setAttribute("d", pathData);
  icon.append(path);

  return icon;
}
