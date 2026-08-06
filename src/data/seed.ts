import type { CssSnippet, Folder, Note, NoteTemplate, VaultData } from "../types";

const DAY = 86_400_000;

export function createSeedVault(now = Date.now()): VaultData {
  const folders: Folder[] = [];

  const notes: Note[] = [
    {
      id: "note-getting-started",
      title: "Getting started",
      folderId: null,
      tags: ["getting-started"],
      pinned: false,
      createdAt: now - DAY,
      updatedAt: now,
      content: `# Getting started

Obsidian At Home is a local-only app for writing, organizing, and linking Markdown notes. Changes save automatically. Use **Settings → Export portable vault** to create a backup or move notes to Obsidian.

## Write with Markdown

Choose **Source** to edit Markdown, **Split** to edit beside a preview, or **Read** to see the formatted note. The app supports the Markdown features below; it is not a complete CommonMark implementation.

### 1. Headings and inline text

Start headings with \`#\` through \`######\`. Use \`**bold**\`, \`*italic*\`, \`~~strikethrough~~\`, and inline code wrapped in backticks.

### 2. Linked notes and web links

Type \`[[\` to search for another note and insert a wiki link. The right panel lists outgoing links and backlinks. Standard links use \`[label](https://example.com)\`.

### 3. Lists and tasks

- Bulleted item
1. Numbered item
- [ ] Open task
- [x] Completed task

### 4. Tables and app features

| Feature | What it does |
| --- | --- |
| Folders and tags | Group and label notes |
| Links and backlinks | Connect notes and show where links come from |
| Search | Find text in titles, content, folders, and tags |
| Templates | Create notes from reusable Markdown structures |
| CSS snippets | Customize the interface |
| Obsidian transfer | Import or export Markdown vaults in the desktop app |

### 5. Quotes and code blocks

> Start a line with \`>\` to create a blockquote.

\`\`\`text
Fenced code blocks preserve spacing and line breaks.
\`\`\`

For a broader syntax reference, see the [CommonMark Markdown Reference](https://commonmark.org/help/). Raw HTML, images, footnotes, nested lists, and reference-style links are not supported.

## Useful shortcuts

- **⌘/Ctrl K** — search notes
- **⌘/Ctrl N** — create a note
- **⌘/Ctrl Shift T** — open templates
- **⌘/Ctrl + backslash** — toggle the vault panel

## Customize with CSS snippets

Open **CSS snippets**, create a snippet, then save and enable it. Disable a snippet to remove its styles. These app targets cover the most common changes:

| Target | Styles |
| --- | --- |
| \`:root\` | Theme variables used across the app |
| \`.markdown-preview\` | The rendered note page |
| \`.markdown-preview h1\` and \`h2\` | Preview headings |
| \`.markdown-preview .wiki-link\` | Linked notes in the preview |
| \`.source-textarea\` | The Markdown source editor |
| \`.note-title-input\` | The note title |
| \`.tag-chip\` | Tags below the title |
| \`.editor-page\` and \`.preview-page\` | Source and preview panes |

\`\`\`css
.markdown-preview {
  --page-width: 900px;
  font-size: 17px;
}

.markdown-preview .wiki-link {
  color: #c9b4f3;
}

.source-textarea {
  line-height: 1.85;
}
\`\`\`

For CSS properties and examples, see the [MDN CSS reference](https://developer.mozilla.org/en-US/docs/Web/CSS).`,
    },
  ];

  const templates: NoteTemplate[] = [
    {
      id: "template-blank",
      name: "Blank note",
      description: "A title and open space for free-form notes.",
      titlePattern: "Untitled note",
      content: "# {{title}}\n\n",
      glyph: "file-plus",
      createdAt: now - 30 * DAY,
      builtIn: true,
    },
    {
      id: "template-daily",
      name: "Daily note",
      description: "A simple structure for priorities and daily notes.",
      titlePattern: "{{date}}",
      content: "# {{date}}\n\n## Focus\n\n- \n\n## Notes\n\n\n## Small win\n\n",
      glyph: "calendar",
      createdAt: now - 30 * DAY,
      builtIn: true,
    },
    {
      id: "template-meeting",
      name: "Meeting notes",
      description: "Context, decisions, and follow-ups in one place.",
      titlePattern: "Meeting — {{date}}",
      content: "# {{title}}\n\n**When:** {{date}} at {{time}}\n\n## Context\n\n## Notes\n\n## Decisions\n\n- \n\n## Follow-ups\n\n- [ ] \n",
      glyph: "users",
      createdAt: now - 30 * DAY,
      builtIn: true,
    },
    {
      id: "template-project",
      name: "Project brief",
      description: "Define a project's goal, constraints, references, and next action.",
      titlePattern: "Project brief",
      content: "# {{title}}\n\n## Purpose\n\n## Desired outcome\n\n## Constraints\n\n- \n\n## References\n\n- [[Related note]]\n\n## Next action\n\n- [ ] \n",
      glyph: "briefcase",
      createdAt: now - 30 * DAY,
      builtIn: true,
    },
  ];

  const snippets: CssSnippet[] = [
    {
      id: "snippet-editor-serif",
      name: "Comfortable reading",
      description: "Use a readable line height and spacing for rendered notes.",
      enabled: true,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.markdown-preview {\n  --note-font: "Inter Variable", Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;\n}\n\n.markdown-preview p {\n  line-height: 1.74;\n  letter-spacing: -0.006em;\n}`,
    },
    {
      id: "snippet-violet-headings",
      name: "Violet headings",
      description: "Give note headings a subtle lavender tint.",
      enabled: false,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.markdown-preview h1,\n.markdown-preview h2 {\n  color: #c9c1ff;\n}`,
    },
    {
      id: "snippet-wide-page",
      name: "Wide reading page",
      description: "Increase the rendered note width for long-form notes.",
      enabled: false,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.editor-page,\n.preview-page {\n  --page-width: 980px;\n}`,
    },
  ];

  return {
    name: "Home Vault",
    notes,
    folders,
    templates,
    snippets,
    activeNoteId: "note-getting-started",
    selectedFolderId: "all",
    editorMode: "source",
  };
}

export function createEmptyVault(now = Date.now()): VaultData {
  const starter = createSeedVault(now);
  return {
    ...starter,
    notes: [],
    folders: [],
    activeNoteId: null,
    selectedFolderId: "all",
  };
}
