<p align="center">
  <img src="src/assets/app-icon.png" width="104" height="104" alt="Obsidian At Home app icon" />
</p>

<h1 align="center">Obsidian At Home</h1>

<p align="center">
  A local-only desktop app for writing, organizing, and linking Markdown notes.
</p>

<p align="center">
  <strong>Vue 3</strong> · <strong>Tauri 2</strong> · <strong>Rust</strong>
</p>

![Obsidian At Home note editor with the Inter interface](docs/screenshots/notes-inter.png)

Obsidian At Home is a local-only Markdown notes app built with Vue and Tauri. You can organize notes into nested folders, connect them with `[[wiki links]]`, inspect backlinks, and search the whole notebook. The app does not require an account or send notes to a server.

The scope is intentionally limited: there is **no heavy runtime, cloud sync, graph view, workflow engine, or plugin marketplace**. Obsidian import and export copy Markdown between the apps; they do not mount or continuously mirror an Obsidian vault.

> [!NOTE]
> This is an independent project. It is not affiliated with or endorsed by Obsidian or Dynalist Inc., and it is not intended to be a drop-in implementation of every Obsidian feature.

## Highlights

- A source-first Markdown editor with line numbers, source/split/reading modes, formatting shortcuts, spellcheck, and `[[` link suggestions.
- Nested folders, unfiled and favorites views, pinning, tags, note counts, and quick note creation.
- Obsidian-style wiki links, clickable rendered links, unresolved-link note creation, outgoing links, and backlink excerpts.
- Ranked local search across titles, Markdown content, tags, and folder names, plus a keyboard-driven quick switcher with `⌘K` / `Ctrl+K`.
- Reusable Markdown templates with `{{date}}`, `{{time}}`, and `{{title}}` tokens.
- A CSS snippet editor with local enable/disable controls and editable custom styles.
- Obsidian import and export tools in the desktop app.

## Architecture

| Layer | Technology | Responsibility |
| --- | --- | --- |
| Interface | Vue 3 + TypeScript | Editor, explorer, search, links, templates, snippets, and settings |
| Design system | Custom components + CSS | Interface styling and reusable components; no third-party UI component framework |
| Frontend tooling | Vite | Browser development and optimized frontend builds |
| Desktop shell | Tauri 2 | Uses the operating system WebView rather than bundled Chromium |
| Native core | Rust | Folder selection and bounded, non-overwriting Obsidian vault import/export |

The app automatically saves notebook data to the current WebView's `localStorage`. It does **not** continuously write notes to a folder of Markdown files. The browser preview and installed desktop app have separate storage, and removing the app's data can remove the notebook. Use **Settings → Export portable vault** to create a readable backup.

The app has no account or background sync service. Syncing, collaboration, graph visualization, automation/workflows, attachments, and Obsidian community plugins are outside its current scope.

## Obsidian import and export

Import and export require the Tauri desktop build. The browser preview keeps its own local notebook but cannot open arbitrary folders.

### Import from Obsidian

From **Settings → Import from Obsidian**, choose an Obsidian vault folder and review the import summary before applying it to the notebook. You can:

- **Merge with notebook** — append all imported notes. Notes are not deduplicated.
- **Replace notes & folders** — clear the current notes and folder tree, then import. Existing app templates and CSS snippets remain; imported snippets with a case-insensitive name match are skipped.

The importer:

- Recursively reads UTF-8 `.md` and `.markdown` files while preserving their Markdown source unchanged.
- Recreates nested note folders and keeps wiki-link text intact.
- Uses a basic top-level YAML `title` value as the display title when present, otherwise the filename; scalar, inline-list, and indented-list `tags` are exposed as app tags.
- Reads `.obsidian/snippets/*.css` and the enabled snippet names in `.obsidian/appearance.json`.
- Never modifies the selected source vault.

It skips `.obsidian` and `.trash` while scanning notes, does not follow symbolic links, and does not import attachments, Canvas files, themes, plugins, hotkeys, workspace state, or other Obsidian settings. A configured Obsidian templates folder is imported only as ordinary Markdown notes; it is not converted into the app's template library. Other frontmatter remains in the Markdown text but is not modeled by the interface, and original filesystem timestamps are not retained.

Imports are limited to 100,000 notes, 10 MiB per note, 512 MiB of note text in total, 5 MiB per CSS snippet, and 128 directory levels. Unreadable, non-UTF-8, oversized, or unsafe entries are skipped, with warnings shown in the import review.

### Export to Obsidian

From **Settings → Export portable vault**, choose a parent directory. The exporter always creates a new folder using the notebook name. If that name already exists, it uses `Name (1)`, `Name (2)`, and so on. Existing files and folders are never reused or overwritten.

The exported vault contains:

- Notes as `.md` files in their current nested folder structure. Filenames are generated from sanitized note titles, and filename collisions receive a numeric suffix.
- Every app template in `Templates/`, plus `.obsidian/templates.json` pointing Obsidian at that directory.
- CSS snippets in `.obsidian/snippets/`, plus `.obsidian/appearance.json` with the enabled snippet list.

If a note has no frontmatter, export prepends `title` and current `tags` YAML. Existing frontmatter and all Markdown content are preserved verbatim rather than rewritten. Consequently, changing a title or tag in the app does not update an existing YAML block, although the current title still determines the exported filename.

This is a portability export, not a lossless round trip: original filenames are not retained, attachments are not copied, advanced Obsidian metadata/settings are not recreated, and later changes do not flow between the two apps automatically. After export, use **Open folder as vault** in Obsidian to open the new directory.

## Prerequisites

- [Node.js](https://nodejs.org/) 20.19 or newer and npm (required by Vite 7).
- A stable [Rust toolchain](https://rustup.rs/).
- The [platform prerequisites for Tauri](https://v2.tauri.app/start/prerequisites/).
- For macOS builds: macOS 11 or newer and Xcode Command Line Tools (`xcode-select --install`).

## Develop locally

Install the locked dependencies:

```sh
npm ci
```

Run the interface in a browser:

```sh
npm run dev
```

Vite serves the app at `http://localhost:1420`. Native folder import/export is disabled in this mode.

Run it as a desktop app with live frontend updates:

```sh
npm run desktop
```

## Build and install on macOS

Build macOS artifacts on the Mac that will run them:

```sh
git clone <repository-url>
cd obsidian-at-home
npm ci
npm run desktop:mac
```

`desktop:mac` regenerates the platform icons and creates both application and disk-image bundles. The build is native to the active Rust target, so an Apple Silicon Mac normally produces an Apple Silicon build and an Intel Mac produces an Intel build.

Artifacts are written under:

```text
src-tauri/target/release/bundle/macos/Obsidian At Home.app
src-tauri/target/release/bundle/dmg/*.dmg
```

Open the DMG, drag **Obsidian At Home** into **Applications**, and launch it from Finder, Launchpad, Spotlight, or the Dock. To put a launch icon on the desktop, select `/Applications/Obsidian At Home.app` in Finder, choose **File → Make Alias** (`⌘L`), and drag the alias to the desktop.

This repository does not configure Apple Developer signing or notarization. If Gatekeeper blocks an unsigned local build, Control-click the app and choose **Open**, then confirm **Open**. Alternatively, use **System Settings → Privacy & Security → Open Anyway** after the first launch attempt. Only bypass the warning for a build you created or otherwise trust.

## Project commands

| Command | Purpose |
| --- | --- |
| `npm run dev` | Start the Vite browser preview on port 1420 |
| `npm run build` | Type-check the Vue app and build the frontend into `dist/` |
| `npm run preview` | Serve the built frontend locally |
| `npm run desktop` | Start the Tauri desktop app in development mode |
| `npm run desktop:build` | Create a release build with the platform's configured Tauri bundle targets |
| `npm run desktop:mac` | Regenerate icons and build macOS `.app` and `.dmg` bundles |
| `npm run icons` | Regenerate Tauri platform icons from `src-tauri/icons/icon-source.png` |
| `npm run check` | Run the production frontend build and `cargo check` |
| `npm run tauri -- <args>` | Pass arguments directly to the Tauri CLI |

## License

Released under the [MIT License](LICENSE).
