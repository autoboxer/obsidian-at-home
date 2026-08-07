import type { CssSnippet, Folder, Note, NoteTemplate, VaultData } from "../types";

const DAY = 86_400_000;

export function createSeedVault(now = Date.now()): VaultData {
  const folders: Folder[] = [];

  const notes: Note[] = [
    {
      id: "note-getting-started",
      title: "Getting started",
      relativePath: "Getting started.md",
      folderId: null,
      tags: ["getting-started"],
      pinned: false,
      createdAt: now - DAY,
      updatedAt: now,
      content: `# Getting started

In the desktop app, your notes are ordinary Markdown files in the vault folder you selected. Changes save automatically, and the app does not require an account or sync notes to a server. The browser preview keeps a separate preview copy in that browser.

## Work with vaults

Use the vault name at the top of the left panel to switch between recent vaults, create a vault, or open an existing folder. In **Settings → Current vault**, you can see the full path and choose **Show in folder**.

- **Create vault** makes a new folder for Markdown notes.
- **Open folder** edits the Markdown files in that folder directly.
- **Import from Obsidian** copies selected notes into the current vault; it does not change the source folder.
- **Export portable vault** creates a separate copy that Obsidian can open.

## Organize notes

The Notes view always shows the tree from the root of the vault. Notes can live at the root or inside folders nested to any depth. Expand or collapse folders to show or hide their contents. Drag notes and folders onto a folder to move them there, or onto the root of the tree to move them back to the vault root.

## Write with Markdown

Choose **Source** to edit Markdown, **Split** to edit beside a preview, or **Read** to see the formatted note. The app supports the Markdown features below; it is not a complete CommonMark implementation.

### 1. Headings and inline text

Start headings with \`#\` through \`######\`. Use \`**bold**\`, \`*italic*\`, \`~~strikethrough~~\`, and inline code wrapped in backticks.

### 2. Linked notes and web links

Type \`[[\` to search for another note and insert a wiki link. The right panel lists outgoing links and backlinks. Standard links use \`[label](https://example.com)\`.

### 3. Lists and tasks

- Bulleted item
  - Nested bullets change shape at each level
    - Keep pressing Tab to nest further
1. Numbered item
  1. Nested ordered items use letters
    1. The next level uses lowercase Roman numerals
- [ ] Open task
- [x] Completed task

Press Enter at the end of a list item to continue the list. Press Tab or Shift+Tab to move the current item in or out one level.

### 4. Tables and app features

| Feature | What it does |
| --- | --- |
| Vaults | Create, open, and switch between Markdown folders |
| Folders and tags | Nest folders, drag notes or folders between locations, and label notes |
| Links and backlinks | Connect notes and show where links come from |
| Search | Find text in titles, content, folders, and tags |
| Templates | Create notes from reusable Markdown structures |
| CSS snippets | Customize the interface |
| Obsidian transfer | Copy notes into or out of the current vault |

### 5. Quotes and code blocks

> Start a line with \`>\` to create a blockquote.

\`\`\`javascript
const message = "Fenced code blocks preserve spacing and line breaks.";
console.log(message);
\`\`\`

Type three backticks and press Enter to insert a complete fenced block. Add a supported language after the opening backticks for syntax highlighting:

- Programming: \`bash\`, \`javascript\` or \`js\`, \`typescript\` or \`ts\`, \`python\`, \`rust\`, \`go\`, \`sql\`, and \`wasm\` or \`wat\`
- Web and data: \`vue\`, \`html\`, \`xml\`, \`svg\`, \`xhtml\`, \`plist\`, \`css\`, \`json\`, \`yaml\`, \`toml\`, \`graphql\`, \`http\`, and \`markdown\`
- Tooling: \`dockerfile\`, \`diff\`, \`makefile\`, \`nginx\`, and \`protobuf\`

For a broader syntax reference, see the [CommonMark Markdown Reference](https://commonmark.org/help/). Raw HTML, images, footnotes, and reference-style links are not supported.

## Useful shortcuts

- **⌘/Ctrl K** — search notes
- **⌘/Ctrl N** — create a note
- **⌘/Ctrl Shift T** — open templates
- **⌘/Ctrl + backslash** — toggle the vault panel

## Customize with CSS snippets

Open **CSS snippets**, create a snippet, then save and enable it. Use **Selector reference** in that panel to find stable targets for app views, panels, controls, editor modes, and rendered Markdown.

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
      css: `.markdown-preview {\n  --page-width: 980px;\n}`,
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
