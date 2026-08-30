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
- **Import from Obsidian** copies selected notes and their image and attachment files into the current vault; it does not change the source folder.
- **Export portable vault** creates a separate copy, including image and attachment files, that Obsidian can open.

## Organize notes

The Notes view always shows the tree from the root of the vault. Notes can live at the root or inside folders nested to any depth. Expand or collapse folders to show or hide their contents. Drag notes and folders onto a folder to move them there, or onto the root of the tree to move them back to the vault root.

## Write with Markdown

Write directly in the live Markdown editor. Formatting stays visible while you read, and Markdown syntax appears when the cursor reaches an editable boundary. The app supports the Markdown features below; it is not a complete CommonMark implementation.

### 1. Headings and inline text

Start headings with \`#\` through \`######\`. Use \`**bold**\`, \`*italic*\`, \`~~strikethrough~~\`, and inline code wrapped in backticks.

### 2. Linked notes and web links

Type \`[[\` to search for another note and insert a wiki link. The right panel lists outgoing links and backlinks. Standard links use \`[label](https://example.com)\`.

Add a heading after \`#\` to jump directly to a section. For example, [jump to Useful shortcuts](#useful-shortcuts) in this note. The supported forms are:

- \`[[#Useful shortcuts]]\` or \`[Useful shortcuts](#useful-shortcuts)\` for the current note
- \`[[Project plan#Next steps]]\` or \`[Next steps](Project%20plan.md#next-steps)\` for another note

A target can use the visible heading text or its lowercase, hyphenated slug. Matching ignores case and accents. When duplicate headings share a target, the first one in the note is used. A missing note or heading shows a warning without creating a note.

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

Inside a table, press Tab or Shift+Tab to move between cells. Enter inserts a row below the current row and moves to its first cell; Backspace in the first cell of an empty body row removes that row; Shift+Enter adds a line break inside a cell. Use Up Arrow or Down Arrow to move within a column. Down Arrow from the final row exits below the table.

### 5. Images

In the desktop app, use the image button in the note toolbar, press **⌘/Ctrl Shift I**, or paste an image from the clipboard. The image file is copied into the vault and inserted using standard Markdown such as \`![Diagram](Assets/Diagram.png)\`. Choose whether new images go at the vault root, beside the current note, or in a specific vault folder under **Settings → Current vault**.

Add an Obsidian-compatible suffix to the alt text to control its displayed size:

- \`![Diagram|300](Assets/Diagram.png)\` — 300 pixels wide
- \`![Diagram|300x200](Assets/Diagram.png)\` — 300 pixels wide and 200 pixels high
- \`![Diagram|x200](Assets/Diagram.png)\` — 200 pixels high

Images work on their own and inside lists, tasks, blockquotes, and tables. The app escapes the sizing separator when it inserts an image into a table. App-created references also contain an \`#oah-image=...\` fragment; leave it in place so the app can recover a uniquely identifiable image if its file is renamed or moved. Image files remain in the left file tree after their final reference is removed; drag one from the tree into a note to use it again, or drag a rendered image within its note to move the reference. Images can be renamed or dragged between folders.

### 6. Files

In the desktop app, use the file button or press **⌘/Ctrl Shift A** to copy a non-image file into the vault. You can also drag up to 100 files from Finder, File Explorer, or a Linux file manager to an exact position in the note; images use the image pipeline, other regular files use the attachment pipeline, and folders are skipped. The app inserts an ordinary Markdown link such as \`[Report.pdf](Attachments/Report.pdf)\`, with an \`#oah-asset=...\` fragment that lets it recover a uniquely identifiable file after a rename or move. Choose root, beside-note, or specific-folder storage under **Settings → Current vault**.

File cards work on their own and inside lists, tasks, blockquotes, and tables. Use **Rename** on a rendered card or in the file tree to rename the stored file and update its references throughout the vault. Editing a Markdown link label changes only its displayed text. Click an ordinary document to open it in the system's default app. Archives use **Save archive as…** to a location outside the vault and are never extracted in place. Executables and installers cannot be opened from the app. Attachment files remain in the left file tree after their final reference is removed, where they can be dragged into notes, renamed, or moved between folders.

### 7. Quotes and code blocks

> Start a line with \`>\` to create a blockquote.

\`\`\`javascript
const message = "Fenced code blocks preserve spacing and line breaks.";
console.log(message);
\`\`\`

Type three backticks and press Enter to insert a complete fenced block. Add a supported language after the opening backticks for syntax highlighting:

- Programming: \`bash\`, \`javascript\` or \`js\`, \`typescript\` or \`ts\`, \`python\`, \`rust\`, \`go\`, \`sql\`, and \`wasm\` or \`wat\`
- Web and data: \`vue\`, \`html\`, \`xml\`, \`svg\`, \`xhtml\`, \`plist\`, \`css\`, \`json\`, \`yaml\`, \`toml\`, \`graphql\`, \`http\`, and \`markdown\`
- Tooling: \`dockerfile\`, \`diff\`, \`makefile\`, \`nginx\`, and \`protobuf\`

For a broader syntax reference, see the [CommonMark Markdown Reference](https://commonmark.org/help/). Raw HTML, footnotes, and reference-style links are not supported.

## Useful shortcuts

- **⌘/Ctrl O** — search notes
- **⌘/Ctrl N** — create a note
- **⌘/Ctrl Shift T** — open templates
- **⌘/Ctrl Backslash** — toggle the vault panel
- **⌘/Ctrl B** — toggle bold text
- **⌘/Ctrl I** — toggle italic text
- **⌘/Ctrl K** — create a Markdown link
- **⌘/Ctrl Shift I** — choose and embed an image
- **⌘/Ctrl Shift A** — choose and embed a non-image file
- **⌘/Ctrl V** — embed an image when the clipboard contains one
- **⌘/Ctrl Shift X** — toggle strikethrough text
- **⌘/Ctrl F** — find text in the current note; use Tab/Shift+Tab, Enter/Shift+Enter, or F3/Shift+F3 to move through matches
- **Select text, then press Backtick** — wrap it as inline code

Open **Keyboard shortcuts** from the bottom of the activity rail for the complete reference.

## Customize with CSS snippets

Open **CSS snippets**, create a snippet, then save and enable it. Use **Selector reference** in that panel to find stable targets for app views, panels, controls, and live Markdown.

\`\`\`css
.source-editor {
  --source-editor-line-height: calc(var(--note-font-size) * 1.85);
}

.live-inline-segment.is-wiki-link {
  color: #c9b4f3;
}

.live-markdown-block.heading-level-1 {
  color: #ddd5ff;
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
      name: "Comfortable writing",
      description: "Use relaxed line height and spacing in the live editor.",
      enabled: true,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.source-editor {\n  --source-editor-line-height: calc(var(--note-font-size) * 1.82);\n}\n\n.source-textarea {\n  letter-spacing: -0.006em;\n}`,
    },
    {
      id: "snippet-violet-headings",
      name: "Violet headings",
      description: "Give note headings a subtle lavender tint.",
      enabled: false,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.live-markdown-block.is-heading {\n  color: #c9c1ff;\n}`,
    },
    {
      id: "snippet-wide-page",
      name: "Wide editor",
      description: "Reduce side padding for notes that benefit from more room.",
      enabled: false,
      builtIn: true,
      createdAt: now - 30 * DAY,
      css: `.source-textarea {\n  padding-right: clamp(20px, 2.4vw, 38px);\n  padding-left: clamp(20px, 2.4vw, 38px);\n}`,
    },
  ];

  return {
    name: "Home Vault",
    notes,
    folders,
    templates,
    snippets,
    activeNoteId: "note-getting-started",
    recentNoteIds: ["note-getting-started"],
    selectedFolderId: "all",
    embeddedImages: [],
    imageFiles: [],
    imageEmbedSettings: {
      location: "vault-root",
      folderPath: "",
    },
    embeddedAttachments: [],
    attachmentFiles: [],
    attachmentEmbedSettings: {
      location: "vault-root",
      folderPath: "",
    },
  };
}

export function createEmptyVault(now = Date.now()): VaultData {
  const starter = createSeedVault(now);

  return {
    ...starter,
    notes: [],
    folders: [],
    activeNoteId: null,
    recentNoteIds: [],
    selectedFolderId: "all",
  };
}
