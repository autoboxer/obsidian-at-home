import type { CssSnippet, Folder, Note, NoteTemplate, VaultData } from "../types";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export function createSeedVault(now = Date.now()): VaultData {
  const folders: Folder[] = [
    { id: "folder-inbox", name: "Inbox", parentId: null, createdAt: now - 30 * DAY },
    { id: "folder-projects", name: "Projects", parentId: null, createdAt: now - 28 * DAY },
    { id: "folder-research", name: "Research", parentId: null, createdAt: now - 21 * DAY },
    { id: "folder-journal", name: "Journal", parentId: null, createdAt: now - 14 * DAY },
    { id: "folder-garden", name: "Garden Studio", parentId: "folder-projects", createdAt: now - 10 * DAY },
  ];

  const notes: Note[] = [
    {
      id: "note-welcome",
      title: "Welcome home",
      folderId: "folder-inbox",
      tags: ["welcome", "guide"],
      pinned: true,
      createdAt: now - 12 * DAY,
      updatedAt: now - 8 * MINUTE,
      content: `# Welcome to Obsidian At Home

Keep local Markdown notes, organize them in folders, and connect them with links. Your notes remain on this device and export as plain Markdown.

## Create and link notes

Write in the source editor, then connect notes with double brackets. Try opening [[Slow technology]] or the [[Garden studio brief]].

> Keep useful context close to the note that needs it.

## Useful shortcuts

- Press **⌘ K** to search everything
- Press **⌘ N** to create a note
- Type \`[[Note title]]\` to make a link
- Open **Templates** for common note structures
- Open **CSS snippets** to customize the interface

The right panel shows outgoing links and backlinks, so you can review related notes without leaving the editor.`,
    },
    {
      id: "note-slow-tech",
      title: "Slow technology",
      folderId: "folder-research",
      tags: ["design", "attention"],
      pinned: true,
      createdAt: now - 9 * DAY,
      updatedAt: now - 48 * MINUTE,
      content: `# Slow technology

Software can be useful without asking for attention throughout the day.

## Principles

1. **Local storage.** Notes stay on the user's device.
2. **Clear structure.** Folders organize notes; links connect notes across folders.
3. **Quiet feedback.** Save indicators should be visible without becoming distracting.
4. **Direct editing.** A source-first editor keeps Markdown visible and editable.

These principles guide [[Welcome home]] and the interface notes in [[Writing environment]].

### Question

How can a tool become more useful without demanding more attention?`,
    },
    {
      id: "note-writing-environment",
      title: "Writing environment",
      folderId: "folder-research",
      tags: ["interface", "writing"],
      pinned: false,
      createdAt: now - 7 * DAY,
      updatedAt: now - 3 * HOUR,
      content: `# Writing environment

The editor should keep controls accessible while leaving most of the screen for writing.

| Layer | Purpose |
| --- | --- |
| Explorer | Browse folders and notes |
| Note list | Review notes in the current group |
| Editor | Write and edit the current note |
| Context | Review links and backlinks |

- [x] Source-first Markdown editing
- [x] Linked-note context
- [x] Fast full-text search
- [ ] A distraction-free focus mode

Related: [[Slow technology]]`,
    },
    {
      id: "note-garden-brief",
      title: "Garden studio brief",
      folderId: "folder-garden",
      tags: ["project", "architecture"],
      pinned: false,
      createdAt: now - 6 * DAY,
      updatedAt: now - 2 * HOUR,
      content: `# Garden studio brief

Create a small writing room with garden views and comfortable year-round use.

## Constraints

- 3 × 4 meter footprint
- North light above the desk
- Built-in shelving for field notes
- A deep sill for coffee and seedlings

## References

Use [[Quiet materials]] for material choices and [[Writing environment]] as a reference for the desk layout.

\`\`\`text
north light / timber / shelves / garden view
\`\`\``,
    },
    {
      id: "note-quiet-materials",
      title: "Quiet materials",
      folderId: "folder-garden",
      tags: ["materials", "research"],
      pinned: false,
      createdAt: now - 5 * DAY,
      updatedAt: now - 8 * HOUR,
      content: `# Quiet materials

Choose durable materials that develop a visible patina and suit a small room.

- Limewashed plywood
- Cork pinboard
- Brushed aluminum details
- Wool felt acoustic panels

These options are being considered for [[Garden studio brief]].`,
    },
    {
      id: "note-today",
      title: new Intl.DateTimeFormat("en", { month: "long", day: "numeric", year: "numeric" }).format(now),
      folderId: "folder-journal",
      tags: ["daily"],
      pinned: false,
      createdAt: now - 5 * HOUR,
      updatedAt: now - 18 * MINUTE,
      content: `# Today

## Notes

- Use the first hour for the hardest task.
- Return to [[Garden studio brief]] after lunch.

## Small win

The new notebook layout is comfortable to use.`,
    },
    {
      id: "note-reading-list",
      title: "Reading list",
      folderId: "folder-inbox",
      tags: ["books"],
      pinned: false,
      createdAt: now - 4 * DAY,
      updatedAt: now - DAY,
      content: `# Reading list

- [ ] *The Poetics of Space* — Gaston Bachelard
- [ ] *How to Do Nothing* — Jenny Odell
- [x] *The Nature of Order* — Christopher Alexander

These books support the notes on [[Slow technology]] and [[Quiet materials]].`,
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
      name: "Wide writing page",
      description: "Increase the page width for long-form notes.",
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
    activeNoteId: "note-welcome",
    selectedFolderId: "all",
    editorMode: "source",
  };
}
