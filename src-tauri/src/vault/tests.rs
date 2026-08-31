use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "obsidian-at-home-{label}-{}-{nonce}-{count}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parses_basic_frontmatter_without_touching_content() {
    let content = "---\r\ntitle: \"A linked idea\"\r\ntags: [garden, 'in progress', '#garden']\r\nextra: keep me\r\n---\r\n# Body\r\n";
    let parsed = parse_basic_frontmatter(content);

    assert_eq!(parsed.title.as_deref(), Some("A linked idea"));
    assert_eq!(parsed.tags, vec!["garden", "in progress"]);
    assert!(content.contains("extra: keep me"));
}

#[test]
fn parses_block_tags_and_yaml_comments() {
    let parsed = parse_basic_frontmatter(
            "---\ntitle: The title # a comment\ntags:\n  - one\n  - \"two words\"\n  - #three\n---\nbody",
        );

    assert_eq!(parsed.title.as_deref(), Some("The title"));
    assert_eq!(parsed.tags, vec!["one", "two words", "three"]);
}

#[test]
fn rendering_adds_frontmatter_but_preserves_existing_frontmatter() {
    let rendered = render_note_markdown(
        "A \"quoted\" title",
        &["garden".into(), "#ideas".into()],
        "A [[linked note]].",
    );
    assert!(rendered.starts_with("---\ntitle: \"A \\\"quoted\\\" title\"\n"));
    assert!(rendered.contains("  - \"garden\"\n  - \"ideas\"\n"));
    assert!(rendered.ends_with("A [[linked note]]."));

    let existing = "---\ncustom: true\n---\nBody";
    assert_eq!(render_note_markdown("Changed", &[], existing), existing);
}

#[test]
fn rejects_traversal_and_reserved_export_paths() {
    assert!(checked_relative_folder("../outside").is_err());
    assert!(checked_relative_folder("Notes/.obsidian").is_err());
    assert!(checked_relative_folder("C:/Users/person").is_err());
    assert!(checked_relative_folder(".obsidian-at-home/recently-deleted").is_err());
    assert!(checked_relative_folder("Projects/Ideas").is_ok());
    assert!(checked_relative_attachment_path("Board.canvas").is_err());
    assert!(checked_relative_attachment_path("Projects/Board.CANVAS").is_err());
    assert!(validate_vault_name("../vault").is_err());
    assert!(validate_vault_name("Obsidian At Home export").is_ok());
}

#[test]
fn creates_new_export_directories_and_never_reuses_one() {
    let parent = TempDirectory::new("unique-export");
    let first = create_unique_export_dir(parent.path(), "My Vault").unwrap();
    fs::write(first.join("sentinel.txt"), "keep").unwrap();
    let second = create_unique_export_dir(parent.path(), "My Vault").unwrap();

    assert_eq!(first.file_name().unwrap(), "My Vault");
    assert_eq!(second.file_name().unwrap(), "My Vault (1)");
    assert_eq!(
        fs::read_to_string(first.join("sentinel.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn imports_nested_notes_and_snippet_state() {
    let vault = TempDirectory::new("import");
    fs::create_dir_all(vault.path().join("Projects/Alpha")).unwrap();
    fs::create_dir_all(vault.path().join("Assets")).unwrap();
    fs::create_dir_all(vault.path().join(".trash")).unwrap();
    fs::create_dir_all(vault.path().join(".git")).unwrap();
    fs::create_dir_all(vault.path().join(".obsidian-at-home/recently-deleted")).unwrap();
    fs::create_dir_all(vault.path().join(".obsidian/snippets")).unwrap();
    fs::write(
        vault.path().join("Projects/Alpha/Plan.md"),
        "---\ntitle: Alpha plan\ntags: [work]\n---\n![Diagram](../../Assets/diagram.png)",
    )
    .unwrap();
    fs::write(
        vault.path().join("Assets/diagram.png"),
        b"\x89PNG\r\n\x1a\nportable-image",
    )
    .unwrap();
    fs::write(vault.path().join("Assets/report.pdf"), b"portable-report").unwrap();
    fs::write(
        vault.path().join("Projects/Alpha/Board.canvas"),
        r#"{"nodes":[],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        vault.path().join("Assets/Sketch.CANVAS"),
        r#"{"nodes":[],"edges":[]}"#,
    )
    .unwrap();
    fs::write(vault.path().join(".trash/Deleted.md"), "deleted").unwrap();
    fs::write(
        vault.path().join(".git/private.png"),
        b"\x89PNG\r\n\x1a\nprivate-image",
    )
    .unwrap();
    fs::write(
        vault
            .path()
            .join(".obsidian-at-home/recently-deleted/Private.md"),
        "private recovery data",
    )
    .unwrap();
    fs::write(
        vault.path().join(".obsidian/snippets/pretty.css"),
        ".note { color: plum; }",
    )
    .unwrap();
    fs::write(
        vault.path().join(".obsidian/appearance.json"),
        r#"{"enabledCssSnippets":["pretty"]}"#,
    )
    .unwrap();

    let result = import_obsidian_vault(vault.path().to_string_lossy().into_owned()).unwrap();

    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].folder_path, "Projects/Alpha");
    assert_eq!(result.notes[0].relative_path, "Projects/Alpha/Plan.md");
    assert!(result.notes[0]
        .content
        .ends_with("![Diagram](../../Assets/diagram.png)"));
    assert_eq!(
        result.images,
        vec![ImportedImage {
            relative_path: "Assets/diagram.png".into(),
        }]
    );
    assert_eq!(
        result.attachments,
        vec![ImportedAttachment {
            relative_path: "Assets/report.pdf".into(),
        }]
    );
    assert_eq!(result.snippets.len(), 1);
    assert!(result.snippets[0].enabled);
}

#[test]
fn bounded_note_reads_enforce_the_actual_byte_limit() {
    let vault = TempDirectory::new("bounded-note-read");
    let path = vault.path().join("Growing.md");
    fs::write(&path, "1234567890").unwrap();

    assert_eq!(
        import::read_utf8_file_bounded(&path, 10)
            .unwrap()
            .as_deref(),
        Some("1234567890"),
    );

    fs::write(&path, "12345678901").unwrap();
    assert_eq!(import::read_utf8_file_bounded(&path, 10).unwrap(), None);

    fs::write(&path, [0xff]).unwrap();
    assert_eq!(
        import::read_utf8_file_bounded(&path, 10)
            .expect_err("invalid UTF-8 should be rejected")
            .kind(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn exports_obsidian_compatible_structure_and_avoids_note_collisions() {
    let parent = TempDirectory::new("export");
    let source = TempDirectory::new("export-source");
    fs::create_dir_all(source.path().join("Assets")).unwrap();
    fs::create_dir_all(source.path().join("Projects/Nested")).unwrap();
    fs::write(
        source.path().join("Assets/diagram.png"),
        b"\x89PNG\r\n\x1a\nportable-image",
    )
    .unwrap();
    fs::write(source.path().join("Assets/report.pdf"), b"portable-report").unwrap();
    fs::write(
        source.path().join("Projects/Board.canvas"),
        r#"{"nodes":[],"edges":[]}"#,
    )
    .unwrap();
    fs::write(
        source.path().join("Projects/Nested/Sketch.CANVAS"),
        r#"{"nodes":[],"edges":[]}"#,
    )
    .unwrap();
    let result = export_obsidian_vault(
        parent.path().to_string_lossy().into_owned(),
        source.path().to_string_lossy().into_owned(),
        "Ideas".into(),
        vec![
            ExportNote {
                title: "First note".into(),
                content: "Link to [[Second note]].".into(),
                folder_path: "Projects/Alpha".into(),
                tags: vec!["work".into()],
            },
            ExportNote {
                title: "First note".into(),
                content: "A distinct note.".into(),
                folder_path: "Projects/Alpha".into(),
                tags: vec![],
            },
            ExportNote {
                title: "Template guide".into(),
                content: "Templates can coexist with notes.".into(),
                folder_path: "Templates".into(),
                tags: vec![],
            },
        ],
        vec![ExportTemplate {
            name: "Daily".into(),
            content: "# {{date}}".into(),
        }],
        vec![VaultSnippet {
            name: "focus.css".into(),
            css: ".workspace { color: plum; }".into(),
            enabled: true,
        }],
    )
    .unwrap();

    let root = PathBuf::from(&result.path);
    assert_eq!(result.note_count, 3);
    assert_eq!(result.image_count, 1);
    assert_eq!(result.attachment_count, 1);
    assert_eq!(result.template_count, 1);
    assert_eq!(result.snippet_count, 1);
    assert!(root.join("Projects/Alpha/First note.md").is_file());
    assert!(root.join("Projects/Alpha/First note 1.md").is_file());
    assert!(root.join("Templates/Template guide.md").is_file());
    assert_eq!(
        fs::read_to_string(root.join("Templates/Daily.md")).unwrap(),
        "# {{date}}"
    );
    assert!(root.join(".obsidian/snippets/focus.css").is_file());
    assert_eq!(
        fs::read(root.join("Assets/diagram.png")).unwrap(),
        b"\x89PNG\r\n\x1a\nportable-image"
    );
    assert_eq!(
        fs::read(root.join("Assets/report.pdf")).unwrap(),
        b"portable-report"
    );
    assert!(!root.join("Projects/Board.canvas").exists());
    assert!(!root.join("Projects/Nested/Sketch.CANVAS").exists());
    let appearance = fs::read_to_string(root.join(".obsidian/appearance.json")).unwrap();
    assert!(appearance.contains("focus"));
}

#[cfg(unix)]
#[test]
fn import_does_not_follow_symbolic_links() {
    use std::os::unix::fs::symlink;

    let vault = TempDirectory::new("symlink-vault");
    let outside = TempDirectory::new("symlink-outside");
    fs::write(outside.path().join("Private.md"), "do not import").unwrap();
    fs::write(
        outside.path().join("Private.png"),
        b"\x89PNG\r\n\x1a\nprivate-image",
    )
    .unwrap();
    fs::write(outside.path().join("Private.pdf"), b"private-attachment").unwrap();
    symlink(outside.path(), vault.path().join("linked")).unwrap();

    let result = import_obsidian_vault(vault.path().to_string_lossy().into_owned()).unwrap();
    assert!(result.notes.is_empty());
    assert!(result.images.is_empty());
    assert!(result.attachments.is_empty());
}
