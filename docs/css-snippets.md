# CSS snippets

CSS snippets customize the current vault's interface. Open **CSS snippets**, create or edit a snippet, save it, and enable it. Enabled snippets apply after the app's built-in styles.

The selectors documented here are stable. Other classes may change as the interface evolves. The same reference is available from **CSS snippets → Selector reference** in the app.

## App views

The active view is set on the app frame:

| Selector | View |
| --- | --- |
| `[data-app-view="notes"]` | Notes |
| `[data-app-view="search"]` | Search |
| `[data-app-view="templates"]` | Templates |
| `[data-app-view="snippets"]` | CSS snippets |
| `[data-app-view="settings"]` | Settings |

Use a view selector when a style should apply only while that screen is open.

## Interface regions

| Selector | Region |
| --- | --- |
| `[data-ui-region="titlebar"]` | Desktop titlebar |
| `[data-ui-region="activity-rail"]` | Left navigation |
| `[data-ui-region="vault-panel"]` | Vault and files panel |
| `[data-ui-region="editor"]` | Note editor |
| `[data-ui-region="note-title"]` | Note title field |
| `[data-ui-region="context-panel"]` | Links and note details |
| `[data-ui-region="search"]` | Search page |
| `[data-ui-region="templates"]` | Templates page |
| `[data-ui-region="snippets"]` | CSS snippets page |
| `[data-ui-region="snippet-library"]` | Snippet list |
| `[data-ui-region="snippet-editor"]` | Snippet editor |
| `[data-ui-region="settings"]` | Settings page |
| `[data-ui-region="quick-switcher"]` | Quick switcher |
| `[data-ui-region="vault-chooser"]` | Vault chooser |
| `[data-ui-region="template-dialog"]` | Template editor |
| `[data-ui-region="selector-reference"]` | In-app selector reference |
| `[data-ui-region="notification"]` | Notifications |

The context panel also exposes `[data-context-view="links"]` and `[data-context-view="info"]`.

## Editor views and panes

| Selector | Target |
| --- | --- |
| `[data-editor-view="source"]` | Source view |
| `[data-editor-view="split"]` | Split view |
| `[data-editor-view="reading"]` | Reading view |
| `[data-editor-pane="source"]` | Source pane |
| `[data-editor-pane="preview"]` | Rendered pane |
| `.source-textarea` | Markdown source text |
| `.markdown-preview` | Rendered note |
| `[data-ui-region="note-title"]` | Note title |
| `.tag-chip` | Note tags |

Source and preview panes stay mounted while hidden. Combine view and pane selectors to limit a style to one mode:

```css
/* Reading view only */
[data-editor-view="reading"] .markdown-preview {
  --page-width: 900px;
  font-size: 18px;
}

/* Preview half of Split view only */
[data-editor-view="split"] [data-editor-pane="preview"] {
  background: #111;
}

/* Source text in both Source and Split views */
.source-textarea {
  line-height: 1.85;
}
```

## Common interface elements

| Selector | Target |
| --- | --- |
| `.rail-button` | Navigation buttons |
| `.vault-tree-folder-row` | Folder rows |
| `.vault-tree-note` | Note rows |
| `.connection-card` | Outgoing-link cards |
| `.backlink-card` | Backlink cards |
| `.search-result-card` | Search results |
| `.template-card` | Template cards |
| `.snippet-list-item` | CSS snippet rows |
| `.settings-section` | Settings sections |
| `.popover-menu` | Context menus |
| `.command-palette` | Quick switcher dialog |
| `.vault-chooser-dialog` | Vault chooser dialog |
| `.editor-modal` | Editor dialogs |
| `.primary-action-button` | Primary buttons |
| `.app-toast` | Notifications |

```css
[data-ui-region="vault-panel"] .vault-tree-note {
  border-radius: 6px;
}

[data-app-view="search"] .search-result-card {
  border-color: rgba(255, 255, 255, 0.14);
}
```

## Rendered Markdown

Rendered notes use normal HTML elements inside `.markdown-preview`. Scope styles to that class so they do not affect the rest of the app.

| Selector | Target |
| --- | --- |
| `.markdown-preview h1` through `h6` | Headings |
| `.markdown-preview p` | Paragraphs |
| `.markdown-preview a` | Web links |
| `.markdown-preview .wiki-link` | Wiki links |
| `.wiki-link.is-unresolved` | Unresolved wiki links |
| `.markdown-preview blockquote` | Blockquotes |
| `.markdown-preview pre code` | Code blocks |
| `.markdown-preview table` | Tables |
| `.markdown-preview .task-list-item` | Tasks |
| `.markdown-preview .language-js` | A specific fenced language |

## Theme variables

Override variables on `:root` to change the whole app, or on a region to limit their effect.

| Variable | Controls |
| --- | --- |
| `--bg`, `--bg-raised`, `--bg-elevated` | Backgrounds |
| `--panel`, `--panel-soft`, `--panel-deep` | Panels |
| `--text`, `--text-soft`, `--text-muted` | Text |
| `--border`, `--border-strong` | Borders |
| `--violet`, `--violet-bright` | Accent colors |
| `--font-sans`, `--mono` | Interface and code fonts |
| `--note-font-family`, `--note-font-size` | Source and rendered note typography |
| `--note-font` | Rendered note font override |
| `--explorer-width`, `--inspector-width` | Side panel widths |

```css
:root {
  --violet: #8fbfe8;
  --explorer-width: 320px;
}
```

## Obsidian snippets

Importing an Obsidian vault copies its CSS snippets, but Obsidian-specific selectors are not translated. Update those selectors using this reference before enabling the snippet.
