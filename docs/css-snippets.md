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
| `[data-ui-region="document-search"]` | Find-in-note bar |
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
| `[data-ui-region="keyboard-shortcuts"]` | Keyboard shortcut reference |
| `[data-ui-region="selector-reference"]` | In-app selector reference |
| `[data-ui-region="notification"]` | Notifications |

The context panel also exposes `[data-context-view="links"]` and `[data-context-view="info"]`.

## Editor and context

| Selector | Target |
| --- | --- |
| `[data-editor-view="live"]` | Unified editor view |
| `[data-editor-pane="live"]` | Unified note pane |
| `[data-context-view="links"]` | Links context tab |
| `[data-context-view="info"]` | Info context tab |
| `.source-editor` | Live Markdown editor |
| `.live-markdown-layer` | Formatted Markdown layer |
| `.source-textarea` | Source input and caret |
| `.document-search-bar` | Find-in-note controls |
| `[data-ui-region="note-title"]` | Note title |
| `.tag-chip` | Note tags |

The formatted layer and source input make up one interactive editor. Combine the live view selector with an editor element to keep customizations scoped to notes:

```css
[data-editor-view="live"] .source-editor {
  --source-editor-line-height: calc(var(--note-font-size) * 1.85);
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
| `.shortcut-reference-modal` | Keyboard shortcut dialog |
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

## Live Markdown

The live editor renders formatted blocks and inline regions in `.live-markdown-layer` while `.source-textarea` provides the editable Markdown source. Scope styles to the selectors below so they do not affect the rest of the app.

| Selector | Target |
| --- | --- |
| `.live-markdown-block.heading-level-1` | Level-one headings |
| `.live-inline-segment.is-wiki-link` | Wiki links |
| `.live-inline-segment.is-wiki-link.is-unresolved` | Unresolved wiki links |
| `.live-markdown-block.is-blockquote` | Blockquotes |
| `.live-code-block` | Code blocks |
| `.live-table-block` | Tables |
| `.live-markdown-block.is-task` | Tasks |
| `.live-task-checkbox` | Task checkboxes |
| `.live-code-language-button` | Code language control |
| `.live-code-body .hljs-keyword` | Highlighted keywords |

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
