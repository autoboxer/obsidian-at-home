<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { leadingFrontmatterEnd } from "../lib/frontmatter";
import {
  findMarkdownHeading,
  parseMarkdownHeadingTarget,
} from "../lib/headingLinks";
import { formatCommandShortcut } from "../lib/keyboard";
import {
  imageAltFromPath,
  isSupportedImageFileName,
  pastedImageFileName,
} from "../lib/imageEmbeds";
import {
  formatMarkdownImage,
  relativeImageDestination,
} from "../lib/markdownImages";
import {
  attachmentLabelFromPath,
  formatMarkdownAttachment,
  relativeAttachmentDestination,
} from "../lib/markdownAttachments";
import { resolveWikiLink } from "../lib/wikiLinks";
import {
  discardWorkspaceExternalAsset,
  embedWorkspaceAttachmentFile,
  embedWorkspaceExternalAttachment,
  embedWorkspaceExternalImage,
  embedWorkspaceImageBytes,
  embedWorkspaceImageFile,
  embedWorkspaceVaultAttachment,
  embedWorkspaceVaultImage,
  isTauri,
  pickAttachmentFile,
  pickImageFile,
  readClipboardImagePng,
} from "../services/native";
import {
  editorPositionVaultId,
  getNoteEditorPosition,
  setNoteEditorPosition,
} from "../stores/editorPositions";
import {
  activeNote,
  activateVaultAttachment,
  applyEmbeddedAttachmentResult,
  applyEmbeddedImageResult,
  applyExternalAssetDiscardResult,
  backNavigationNote,
  canNavigateBack,
  canNavigateForward,
  createFolder,
  createLinkedNote,
  createNote,
  deleteNote,
  folderPath,
  forwardNavigationNote,
  flushVault,
  navigateBack,
  navigateForward,
  moveNoteToFolder,
  notify,
  renameVaultAttachment,
  revealVaultItemInTree,
  selectNote,
  showVaultItemInFolder,
  togglePinned,
  uiState,
  updateNote,
  vaultAttachmentInsertRequest,
  vaultImageInsertRequest,
  vaultSession,
  vaultState,
} from "../stores/vault";
import type {
  AssetInsertionCapture,
  AttachmentInsertionCapture,
  ImageInsertionCapture,
  Note,
  NoteEditorPosition,
  WorkspaceEmbedAttachmentResult,
  WorkspaceEmbedImageResult,
  WorkspaceExternalAssetDiscardResult,
} from "../types";
import AppIcon from "./AppIcon.vue";
import SourceEditor from "./SourceEditor.vue";

const createNoteShortcut = formatCommandShortcut("N");
const embedAttachmentShortcut = formatCommandShortcut("Shift+A");
const embedImageShortcut = formatCommandShortcut("Shift+I");
const nativeAvailable = isTauri();
const tagInputOpen = ref(false);
const tagInput = ref("");
const tagField = ref<HTMLInputElement>();
const tagSuggestionIndex = ref(-1);
const noteMenuOpen = ref(false);
const quickFolderOpen = ref(false);
const quickFolderName = ref("");
const quickFolderField = ref<HTMLInputElement>();
const quickFolderButton = ref<HTMLButtonElement>();
const attachmentEmbedBusy = ref(false);
const imageEmbedBusy = ref(false);
let externalFileDropAbort: AbortController | undefined;
const sourceEditor = ref<{
  cancelAttachmentInsertion: (capture: AttachmentInsertionCapture) => void;
  cancelImageInsertion: (capture: ImageInsertionCapture) => void;
  captureAttachmentInsertion: () => AttachmentInsertionCapture | undefined;
  captureImageInsertion: () => ImageInsertionCapture | undefined;
  focusDocumentOffset: (offset: number) => boolean;
  insertEmbeddedAttachment: (
    capture: AttachmentInsertionCapture,
    markdownAttachment: string,
  ) => boolean;
  insertEmbeddedImage: (capture: ImageInsertionCapture, markdownImage: string) => boolean;
}>();

const noteTitles = computed(() => vaultState.notes.map((note) => note.title));
const positionVaultId = computed(() => editorPositionVaultId(vaultSession.backend, vaultSession.path));
const editorKey = computed(() => JSON.stringify([
  positionVaultId.value,
  activeNote.value?.id ?? null,
]));
const sortedFolders = computed(() => [...vaultState.folders].sort((a, b) => folderPath(a.id).localeCompare(folderPath(b.id))));
const tagSuggestions = computed(() => {
  const query = normalizeTag(tagInput.value).toLocaleLowerCase();
  const appliedTags = new Set(activeNote.value?.tags.map((tag) => tag.toLocaleLowerCase()) ?? []);
  const uniqueTags = new Map<string, string>();

  for (const note of vaultState.notes) {
    for (const tag of note.tags) {
      if (!uniqueTags.has(tag.toLocaleLowerCase())) {
        uniqueTags.set(tag.toLocaleLowerCase(), tag);
      }
    }
  }

  return [...uniqueTags.values()]
    .filter((tag) => !appliedTags.has(tag.toLocaleLowerCase()))
    .filter((tag) => !query || tag.toLocaleLowerCase().includes(query))
    .sort((a, b) => {
      const aStartsWithQuery = query && a.toLocaleLowerCase().startsWith(query) ? 0 : 1;
      const bStartsWithQuery = query && b.toLocaleLowerCase().startsWith(query) ? 0 : 1;

      return aStartsWithQuery - bStartsWithQuery || a.localeCompare(b);
    })
    .slice(0, 7);
});
const wordCount = computed(() => {
  const content = activeNote.value?.content ?? "";

  return content.trim() ? content.trim().split(/\s+/).length : 0;
});
const characterCount = computed(() => activeNote.value?.content.length ?? 0);
const hasFrontmatter = computed(() => (
  activeNote.value
    ? leadingFrontmatterEnd(activeNote.value.content) !== undefined
    : false
));
const backNavigationLabel = computed(() => backNavigationNote.value
  ? `Back to “${backNavigationNote.value.title.trim() || "Untitled note"}”`
  : "No previous note");
const forwardNavigationLabel = computed(() => forwardNavigationNote.value
  ? `Forward to “${forwardNavigationNote.value.title.trim() || "Untitled note"}”`
  : "No next note");

function setTitle(event: Event): void {
  if (!activeNote.value) {
    return;
  }
  updateNote(activeNote.value.id, { title: (event.target as HTMLInputElement).value });
}

function setContent(content: string): void {
  if (activeNote.value) {
    updateNote(activeNote.value.id, { content });
  }
}

function savedEditorPosition(noteId: string, content: string): NoteEditorPosition | undefined {
  return getNoteEditorPosition(positionVaultId.value, noteId, content);
}

function rememberEditorPosition(
  vaultId: string,
  noteId: string,
  position: NoteEditorPosition,
): void {
  if (
    vaultId === positionVaultId.value
    && !vaultState.notes.some((note) => note.id === noteId)
  ) {
    return;
  }

  setNoteEditorPosition(vaultId, noteId, position);
}

async function openRenderedLink(href: string): Promise<void> {
  const headingTarget = parseMarkdownHeadingTarget(href);
  if (headingTarget) {
    await openHeadingLink(headingTarget.noteTarget, headingTarget.heading);

    return;
  }

  try {
    await openUrl(href);
  } catch {
    notify("Could not open that link", "warning");
  }
}

async function openWikiLink(target: string, heading?: string): Promise<void> {
  if (!heading) {
    createLinkedNote(target);

    return;
  }

  await openHeadingLink(target, heading);
}

async function openHeadingLink(target: string, heading: string): Promise<void> {
  const note = resolveWikiLink(target, vaultState.notes, activeNote.value);
  if (!note) {
    notify(`Could not find note “${linkedNoteLabel(target)}”`, "warning");

    return;
  }

  const match = findMarkdownHeading(note.content, heading);
  if (!match) {
    notify(`Could not find heading “${heading}” in “${note.title}”`, "warning");

    return;
  }

  if (note.id !== activeNote.value?.id) {
    selectNote(note.id);
    await nextTick();
  }

  if (!sourceEditor.value?.focusDocumentOffset(match.contentFrom)) {
    notify("Could not move to that heading", "warning");
  }
}

function linkedNoteLabel(target: string): string {
  return target.trim().replace(/\.md$/i, "") || "current note";
}

interface AssetEmbedContext {
  note: Note;
  noteRelativePath: string;
  vaultPath: string;
}

interface StoreAndInsertOptions {
  announce?: boolean;
  cleanupFailedInsertion?: (
    context: AssetEmbedContext,
    assetId: string,
    relativePath: string,
    expectedRevision: number,
  ) => Promise<WorkspaceExternalAssetDiscardResult>;
  markdownPrefix?: string;
}

async function cleanupFailedExternalInsertion(
  context: AssetEmbedContext,
  assetId: string,
  relativePath: string,
  expectedRevision: number,
): Promise<WorkspaceExternalAssetDiscardResult> {
  return discardWorkspaceExternalAsset(
    context.vaultPath,
    assetId,
    relativePath,
    expectedRevision,
  );
}

async function prepareExternalFileFinish(
  context: AssetEmbedContext,
  signal: AbortSignal,
): Promise<number> {
  if (signal.aborted) {
    throw new DOMException("The dropped-file transfer was cancelled.", "AbortError");
  }
  if (!(await flushVault())) {
    throw new Error(vaultSession.error || "Save the note before finishing the file drop.");
  }
  if (
    signal.aborted
    || vaultSession.path !== context.vaultPath
    || activeNote.value?.id !== context.note.id
    || context.note.relativePath !== context.noteRelativePath
  ) {
    throw new Error("The note or vault changed before the file drop could finish.");
  }

  return vaultSession.revision;
}

function embedError(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }

  return fallback;
}

function assetEmbedContext(capture: AssetInsertionCapture): AssetEmbedContext | undefined {
  const note = vaultState.notes.find((candidate) => candidate.id === capture.noteId);
  const vaultPath = vaultSession.backend === "native" ? vaultSession.path : null;
  if (!nativeAvailable || !note || !vaultPath || activeNote.value?.id !== note.id) {
    return undefined;
  }

  return {
    note,
    noteRelativePath: note.relativePath,
    vaultPath,
  };
}

async function storeAndInsertImage(
  capture: ImageInsertionCapture,
  embed: (
    context: AssetEmbedContext,
    expectedRevision: number,
  ) => Promise<WorkspaceEmbedImageResult>,
  options: StoreAndInsertOptions = {},
): Promise<boolean> {
  const context = assetEmbedContext(capture);
  if (!context) {
    throw new Error("Images can be embedded into an open note in a desktop vault.");
  }
  if (!(await flushVault())) {
    throw new Error(vaultSession.error || "Save the current note before embedding an image.");
  }
  if (
    vaultSession.path !== context.vaultPath
    || activeNote.value?.id !== context.note.id
  ) {
    throw new Error("The note or vault changed before the image could be embedded.");
  }
  context.noteRelativePath = context.note.relativePath;
  if (!context.noteRelativePath) {
    throw new Error("The note does not have a saved file path for the embedded image.");
  }

  const result = await embed(context, vaultSession.revision);
  if (
    vaultSession.path !== context.vaultPath
    || activeNote.value?.id !== context.note.id
  ) {
    await retainOrDiscardFailedImage(context, result, options);
    throw new Error("The note or vault changed before the image could be inserted.");
  }
  const selectedAlt = capture.selectedText.trim();
  const alt = selectedAlt && !/[\r\n]/.test(selectedAlt) && selectedAlt.length <= 240
    ? selectedAlt
    : imageAltFromPath(result.image.relativePath);
  const markdownImage = `${options.markdownPrefix ?? ""}${formatMarkdownImage({
    alt,
    assetId: result.image.id,
    destination: relativeImageDestination(
      context.noteRelativePath,
      result.image.relativePath,
    ),
    inTable: capture.inTable,
  })}`;
  const inserted = sourceEditor.value?.insertEmbeddedImage(capture, markdownImage) ?? false;
  if (!inserted) {
    const discarded = await retainOrDiscardFailedImage(context, result, options);
    notify(
      discarded
        ? "The image reference could not be inserted, so its unused stored copy was removed."
        : "The image was saved, but its Markdown reference could not be inserted.",
      "warning",
    );

    return false;
  }
  applyEmbeddedImageResult(result);

  if (result.warnings.length) {
    notify(result.warnings[0]!, "warning");
  } else if (options.announce !== false) {
    notify(`Embedded ${imageAltFromPath(result.image.relativePath)}`, "success");
  }

  return true;
}

async function retainOrDiscardFailedImage(
  context: AssetEmbedContext,
  result: WorkspaceEmbedImageResult,
  options: StoreAndInsertOptions,
): Promise<boolean> {
  if (!options.cleanupFailedInsertion) {
    if (vaultSession.path === context.vaultPath) {
      applyEmbeddedImageResult(result);
    }

    return false;
  }
  try {
    const cleanup = await options.cleanupFailedInsertion(
      context,
      result.image.id,
      result.image.relativePath,
      result.revision,
    );
    if (vaultSession.path === context.vaultPath) {
      if (cleanup.discarded) {
        applyExternalAssetDiscardResult(cleanup);
      } else {
        applyEmbeddedImageResult({
          ...result,
          revision: cleanup.revision,
          savedAt: cleanup.savedAt,
          warnings: cleanup.warnings,
        });
      }
    }

    return cleanup.discarded;
  } catch {
    if (vaultSession.path === context.vaultPath) {
      applyEmbeddedImageResult(result);
    }

    return false;
  }
}

async function embedImageFromFile(capture: ImageInsertionCapture): Promise<void> {
  if (imageEmbedBusy.value || attachmentEmbedBusy.value) {
    sourceEditor.value?.cancelImageInsertion(capture);
    notify("Wait for the current image to finish embedding.", "warning");

    return;
  }

  imageEmbedBusy.value = true;
  try {
    const context = assetEmbedContext(capture);
    if (!context) {
      throw new Error("Images can be embedded into an open note in a desktop vault.");
    }
    if (!(await flushVault())) {
      throw new Error(vaultSession.error || "Save the current note before embedding an image.");
    }
    const sourcePath = await pickImageFile();
    if (!sourcePath) {
      return;
    }
    if (vaultSession.path !== context.vaultPath || activeNote.value?.id !== context.note.id) {
      throw new Error("The note or vault changed before the image could be embedded.");
    }
    await storeAndInsertImage(capture, (current, expectedRevision) =>
      embedWorkspaceImageFile(
        current.vaultPath,
        sourcePath,
        current.noteRelativePath,
        { ...vaultState.imageEmbedSettings },
        expectedRevision,
      )
    );
  } catch (error) {
    notify(embedError(error, "The image could not be embedded."), "warning");
  } finally {
    sourceEditor.value?.cancelImageInsertion(capture);
    imageEmbedBusy.value = false;
  }
}

async function embedImageFromClipboard(
  capture: ImageInsertionCapture,
  file?: File,
): Promise<void> {
  if (imageEmbedBusy.value || attachmentEmbedBusy.value) {
    sourceEditor.value?.cancelImageInsertion(capture);
    notify("Wait for the current image to finish embedding.", "warning");

    return;
  }

  imageEmbedBusy.value = true;
  try {
    const bytes = file
      ? new Uint8Array(await file.arrayBuffer())
      : await readClipboardImagePng();
    const fileName = file?.name?.trim() || pastedImageFileName();
    await storeAndInsertImage(capture, (context, expectedRevision) =>
      embedWorkspaceImageBytes(
        context.vaultPath,
        fileName,
        bytes,
        context.noteRelativePath,
        { ...vaultState.imageEmbedSettings },
        expectedRevision,
      )
    );
  } catch (error) {
    notify(embedError(error, "The image could not be embedded."), "warning");
  } finally {
    sourceEditor.value?.cancelImageInsertion(capture);
    imageEmbedBusy.value = false;
  }
}

async function embedImageFromVault(
  capture: ImageInsertionCapture,
  relativePath: string,
): Promise<void> {
  if (imageEmbedBusy.value || attachmentEmbedBusy.value) {
    sourceEditor.value?.cancelImageInsertion(capture);
    notify("Wait for the current image to finish embedding.", "warning");

    return;
  }

  imageEmbedBusy.value = true;
  try {
    await storeAndInsertImage(capture, (context, expectedRevision) =>
      embedWorkspaceVaultImage(
        context.vaultPath,
        relativePath,
        context.noteRelativePath,
        { ...vaultState.imageEmbedSettings },
        expectedRevision,
      )
    );
  } catch (error) {
    notify(embedError(error, "The image could not be embedded."), "warning");
  } finally {
    sourceEditor.value?.cancelImageInsertion(capture);
    imageEmbedBusy.value = false;
  }
}

function requestImageFromToolbar(): void {
  const capture = sourceEditor.value?.captureImageInsertion();
  if (capture) {
    void embedImageFromFile(capture);
  }
}

async function storeAndInsertAttachment(
  capture: AttachmentInsertionCapture,
  embed: (
    context: AssetEmbedContext,
    expectedRevision: number,
  ) => Promise<WorkspaceEmbedAttachmentResult>,
  options: StoreAndInsertOptions = {},
): Promise<boolean> {
  const context = assetEmbedContext(capture);
  if (!context) {
    throw new Error("Files can be embedded into an open note in a desktop vault.");
  }
  if (!(await flushVault())) {
    throw new Error(vaultSession.error || "Save the current note before embedding a file.");
  }
  if (
    vaultSession.path !== context.vaultPath
    || activeNote.value?.id !== context.note.id
  ) {
    throw new Error("The note or vault changed before the file could be embedded.");
  }
  context.noteRelativePath = context.note.relativePath;
  if (!context.noteRelativePath) {
    throw new Error("The note does not have a saved file path for the embedded file.");
  }

  const result = await embed(context, vaultSession.revision);
  if (
    vaultSession.path !== context.vaultPath
    || activeNote.value?.id !== context.note.id
  ) {
    await retainOrDiscardFailedAttachment(context, result, options);
    throw new Error("The note or vault changed before the file could be inserted.");
  }
  const selectedLabel = capture.selectedText.trim();
  const label = selectedLabel && !/[\r\n]/.test(selectedLabel) && selectedLabel.length <= 240
    ? selectedLabel
    : attachmentLabelFromPath(result.attachment.relativePath);
  const markdownAttachment = `${options.markdownPrefix ?? ""}${formatMarkdownAttachment({
    label,
    assetId: result.attachment.id,
    destination: relativeAttachmentDestination(
      context.noteRelativePath,
      result.attachment.relativePath,
    ),
    inTable: capture.inTable,
  })}`;
  const inserted = sourceEditor.value?.insertEmbeddedAttachment(
    capture,
    markdownAttachment,
  ) ?? false;
  if (!inserted) {
    const discarded = await retainOrDiscardFailedAttachment(context, result, options);
    notify(
      discarded
        ? "The file reference could not be inserted, so its unused stored copy was removed."
        : "The file was saved, but its Markdown reference could not be inserted.",
      "warning",
    );

    return false;
  }
  applyEmbeddedAttachmentResult(result);

  if (result.warnings.length) {
    notify(result.warnings[0]!, "warning");
  } else if (options.announce !== false) {
    notify(`Embedded ${attachmentLabelFromPath(result.attachment.relativePath)}`, "success");
  }

  return true;
}

async function retainOrDiscardFailedAttachment(
  context: AssetEmbedContext,
  result: WorkspaceEmbedAttachmentResult,
  options: StoreAndInsertOptions,
): Promise<boolean> {
  if (!options.cleanupFailedInsertion) {
    if (vaultSession.path === context.vaultPath) {
      applyEmbeddedAttachmentResult(result);
    }

    return false;
  }
  try {
    const cleanup = await options.cleanupFailedInsertion(
      context,
      result.attachment.id,
      result.attachment.relativePath,
      result.revision,
    );
    if (vaultSession.path === context.vaultPath) {
      if (cleanup.discarded) {
        applyExternalAssetDiscardResult(cleanup);
      } else {
        applyEmbeddedAttachmentResult({
          ...result,
          revision: cleanup.revision,
          savedAt: cleanup.savedAt,
          warnings: cleanup.warnings,
        });
      }
    }

    return cleanup.discarded;
  } catch {
    if (vaultSession.path === context.vaultPath) {
      applyEmbeddedAttachmentResult(result);
    }

    return false;
  }
}

async function embedAttachmentFromFile(
  capture: AttachmentInsertionCapture,
): Promise<void> {
  if (attachmentEmbedBusy.value || imageEmbedBusy.value) {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    notify("Wait for the current file to finish embedding.", "warning");

    return;
  }

  attachmentEmbedBusy.value = true;
  try {
    const context = assetEmbedContext(capture);
    if (!context) {
      throw new Error("Files can be embedded into an open note in a desktop vault.");
    }
    const sourcePath = await pickAttachmentFile();
    if (!sourcePath) {
      return;
    }
    if (vaultSession.path !== context.vaultPath || activeNote.value?.id !== context.note.id) {
      throw new Error("The note or vault changed before the file could be embedded.");
    }
    await storeAndInsertAttachment(capture, (current, expectedRevision) =>
      embedWorkspaceAttachmentFile(
        current.vaultPath,
        sourcePath,
        current.noteRelativePath,
        { ...vaultState.attachmentEmbedSettings },
        expectedRevision,
      )
    );
  } catch (error) {
    notify(embedError(error, "The file could not be embedded."), "warning");
  } finally {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    attachmentEmbedBusy.value = false;
  }
}

async function embedAttachmentFromVault(
  capture: AttachmentInsertionCapture,
  relativePath: string,
): Promise<void> {
  if (attachmentEmbedBusy.value || imageEmbedBusy.value) {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    notify("Wait for the current file to finish embedding.", "warning");

    return;
  }

  attachmentEmbedBusy.value = true;
  try {
    await storeAndInsertAttachment(capture, (context, expectedRevision) =>
      embedWorkspaceVaultAttachment(
        context.vaultPath,
        relativePath,
        context.noteRelativePath,
        { ...vaultState.attachmentEmbedSettings },
        expectedRevision,
      )
    );
  } catch (error) {
    notify(embedError(error, "The file could not be embedded."), "warning");
  } finally {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    attachmentEmbedBusy.value = false;
  }
}

async function embedExternalFiles(
  capture: AttachmentInsertionCapture,
  files: File[],
  rejectedCount: number,
): Promise<void> {
  if (attachmentEmbedBusy.value || imageEmbedBusy.value) {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    notify("Wait for the current file to finish embedding.", "warning");

    return;
  }
  if (!files.length) {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    notify(
      rejectedCount
        ? "Folders and unavailable items cannot be embedded."
        : "No regular files were available in that drop.",
      "warning",
    );

    return;
  }
  const initialContext = assetEmbedContext(capture);
  if (!initialContext) {
    sourceEditor.value?.cancelAttachmentInsertion(capture);
    notify("Drop files into an open note in a desktop vault.", "warning");

    return;
  }

  const controller = new AbortController();
  externalFileDropAbort?.abort();
  externalFileDropAbort = controller;
  attachmentEmbedBusy.value = true;
  imageEmbedBusy.value = true;
  const imageSettings = { ...vaultState.imageEmbedSettings };
  const attachmentSettings = { ...vaultState.attachmentEmbedSettings };
  let currentCapture: AssetInsertionCapture | undefined = capture;
  let embeddedCount = 0;
  let failedCount = rejectedCount;
  const errors: string[] = rejectedCount
    ? [`${rejectedCount} folder${rejectedCount === 1 ? " or item was" : "s or items were"} skipped.`]
    : [];

  try {
    for (let index = 0; index < files.length && currentCapture; index += 1) {
      const file = files[index]!;
      const options: StoreAndInsertOptions = {
        announce: false,
        cleanupFailedInsertion: cleanupFailedExternalInsertion,
        ...(embeddedCount ? { markdownPrefix: " " } : {}),
      };
      try {
        const inserted = isSupportedImageFileName(file.name)
          ? await storeAndInsertImage(
              currentCapture,
              (context, expectedRevision) => embedWorkspaceExternalImage(
                context.vaultPath,
                file,
                context.noteRelativePath,
                imageSettings,
                expectedRevision,
                controller.signal,
                () => prepareExternalFileFinish(context, controller.signal),
              ),
              options,
            )
          : await storeAndInsertAttachment(
              currentCapture,
              (context, expectedRevision) => embedWorkspaceExternalAttachment(
                context.vaultPath,
                file,
                context.noteRelativePath,
                attachmentSettings,
                expectedRevision,
                controller.signal,
                () => prepareExternalFileFinish(context, controller.signal),
              ),
              options,
            );
        if (!inserted) {
          failedCount += 1;
          errors.push("A stored file could not be inserted into the note.");
          currentCapture = undefined;
          break;
        }
        embeddedCount += 1;
        currentCapture = index + 1 < files.length
          ? sourceEditor.value?.captureAttachmentInsertion()
          : undefined;
        if (index + 1 < files.length && !currentCapture) {
          failedCount += files.length - index - 1;
          errors.push("The remaining files could not retain their drop position.");
        }
      } catch (error) {
        failedCount += 1;
        errors.push(`${file.name.trim() || "Dropped file"}: ${embedError(
          error,
          "The file could not be embedded.",
        )}`);
        if (
          controller.signal.aborted
          || !currentCapture
          || !assetEmbedContext(currentCapture)
        ) {
          break;
        }
      }
    }
  } finally {
    if (currentCapture) {
      sourceEditor.value?.cancelAttachmentInsertion(currentCapture);
    }
    if (externalFileDropAbort === controller) {
      externalFileDropAbort = undefined;
    }
    attachmentEmbedBusy.value = false;
    imageEmbedBusy.value = false;
  }

  if (controller.signal.aborted) {
    notify("The file drop was cancelled because the note or vault changed.", "warning");
  } else if (errors.length) {
    notify(
      embeddedCount
        ? `Embedded ${embeddedCount} file${embeddedCount === 1 ? "" : "s"}; ${failedCount} could not be added.`
        : errors[0]!,
      "warning",
    );
  } else {
    notify(
      files.length === 1
        ? `Embedded ${files[0]!.name}`
        : `Embedded ${files.length} files`,
      "success",
    );
  }
}

function requestAttachmentFromToolbar(): void {
  const capture = sourceEditor.value?.captureAttachmentInsertion();
  if (capture) {
    void embedAttachmentFromFile(capture);
  }
}

async function insertRequestedVaultImage(): Promise<void> {
  const relativePath = vaultImageInsertRequest.relativePath;
  await nextTick();
  const capture = sourceEditor.value?.captureImageInsertion();
  if (!capture || !relativePath) {
    notify("Place the cursor in an open note before inserting an image", "warning");

    return;
  }
  await embedImageFromVault(capture, relativePath);
}

async function insertRequestedVaultAttachment(): Promise<void> {
  const relativePath = vaultAttachmentInsertRequest.relativePath;
  await nextTick();
  const capture = sourceEditor.value?.captureAttachmentInsertion();
  if (!capture || !relativePath) {
    notify("Place the cursor in an open note before inserting a file", "warning");

    return;
  }
  await embedAttachmentFromVault(capture, relativePath);
}

function activateEmbeddedAttachment(
  assetId: string | undefined,
  relativePath: string,
  mediaType: string | undefined,
  openingDisabled: boolean | undefined,
): void {
  void activateVaultAttachment({
    ...(assetId ? { assetId } : {}),
    relativePath,
    mediaType: mediaType ?? "application/octet-stream",
    openingDisabled: openingDisabled ?? false,
  });
}

function revealEmbeddedAttachmentInTree(
  assetId: string | undefined,
  relativePath: string,
): void {
  void revealVaultItemInTree({
    ...(assetId ? { assetId } : {}),
    kind: "attachment",
    relativePath,
  });
}

function showEmbeddedAttachmentInFolder(
  assetId: string | undefined,
  relativePath: string,
): void {
  void showVaultItemInFolder({
    ...(assetId ? { assetId } : {}),
    kind: "attachment",
    relativePath,
  });
}

async function setFolder(event: Event): Promise<void> {
  if (!activeNote.value) {
    return;
  }
  const value = (event.target as HTMLSelectElement).value;
  await moveNoteToFolder(activeNote.value.id, value || null);
}

function openTagInput(): void {
  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = true;
  nextTick(() => {
    tagField.value?.focus();
    tagField.value?.select();
  });
}

function normalizeTag(value: string): string {
  return value.trim().replace(/^#/, "").replace(/\s+/g, "-");
}

function addTag(suggestedTag?: string): void {
  if (!activeNote.value) {
    return;
  }

  const tag = normalizeTag(suggestedTag ?? tagInput.value);
  const alreadyApplied = activeNote.value.tags.some(
    (candidate) => candidate.toLocaleLowerCase() === tag.toLocaleLowerCase(),
  );
  if (tag && !alreadyApplied) {
    updateNote(activeNote.value.id, { tags: [...activeNote.value.tags, tag] });
  }

  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function submitTag(): void {
  const suggestion = tagSuggestionIndex.value >= 0
    ? tagSuggestions.value[tagSuggestionIndex.value]
    : undefined;
  addTag(suggestion);
}

function cancelTagInput(): void {
  tagInput.value = "";
  tagSuggestionIndex.value = -1;
  tagInputOpen.value = false;
}

function handleTagKeydown(event: KeyboardEvent): void {
  if (event.key === "ArrowDown" && tagSuggestions.value.length) {
    event.preventDefault();
    tagSuggestionIndex.value = (tagSuggestionIndex.value + 1) % tagSuggestions.value.length;
  } else if (event.key === "ArrowUp" && tagSuggestions.value.length) {
    event.preventDefault();
    tagSuggestionIndex.value = tagSuggestionIndex.value <= 0
      ? tagSuggestions.value.length - 1
      : tagSuggestionIndex.value - 1;
  }
}

function removeTag(tag: string): void {
  if (activeNote.value) {
    updateNote(activeNote.value.id, { tags: activeNote.value.tags.filter((candidate) => candidate !== tag) });
  }
}

function requestDelete(): void {
  if (!activeNote.value) {
    return;
  }
  noteMenuOpen.value = false;
  if (window.confirm(`Delete “${activeNote.value.title || "Untitled note"}”? It will remain in Recently Deleted for seven days.`)) {
    void deleteNote(activeNote.value.id);
  }
}

function toggleFrontmatter(): void {
  uiState.frontmatterVisible = !uiState.frontmatterVisible;
}

function openQuickFolder(): void {
  quickFolderOpen.value = true;
  nextTick(() => quickFolderField.value?.focus());
}

function closeQuickFolder(restoreFocus = false): void {
  quickFolderOpen.value = false;
  quickFolderName.value = "";
  if (restoreFocus) {
    nextTick(() => quickFolderButton.value?.focus());
  }
}

function submitQuickFolder(): void {
  const name = quickFolderName.value.trim();
  if (name) {
    createFolder(name);
  }
  closeQuickFolder(true);
}

function handleQuickFolderFocusOut(event: FocusEvent): void {
  const form = event.currentTarget as HTMLElement;
  const next = event.relatedTarget;
  if (!(next instanceof Node) || !form.contains(next)) {
    closeQuickFolder();
  }
}

watch(
  () => uiState.explorerOpen,
  (open) => {
    if (open) {
      closeQuickFolder();
    }
  },
);

watch(
  () => activeNote.value?.id,
  () => {
    noteMenuOpen.value = false;
  },
);

watch(
  [() => vaultSession.path, () => activeNote.value?.id],
  () => externalFileDropAbort?.abort(),
);

watch(
  () => vaultImageInsertRequest.id,
  () => void insertRequestedVaultImage(),
);

watch(
  () => vaultAttachmentInsertRequest.id,
  () => void insertRequestedVaultAttachment(),
);

watch(tagInput, () => {
  tagSuggestionIndex.value = -1;
});

onBeforeUnmount(() => externalFileDropAbort?.abort());
</script>

<template>
  <main class="editor-workspace" data-ui-region="editor" data-editor-view="live">
    <header class="editor-toolbar">
      <div class="editor-crumbs">
        <button
          class="icon-button explorer-toggle"
          type="button"
          :class="{ active: uiState.explorerOpen }"
          :title="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-label="uiState.explorerOpen ? 'Hide vault panel' : 'Show vault panel'"
          :aria-pressed="uiState.explorerOpen"
          @click="uiState.explorerOpen = !uiState.explorerOpen"
        >
          <AppIcon name="sidebar" :size="17" />
        </button>
        <div
          v-if="activeNote"
          class="note-navigation"
          data-ui-region="note-history"
          role="group"
          aria-label="Note history"
        >
          <button
            class="icon-button subtle"
            type="button"
            data-note-action="navigate-back"
            :disabled="!canNavigateBack"
            :title="backNavigationLabel"
            :aria-label="backNavigationLabel"
            @click="navigateBack"
          >
            <AppIcon name="history-back" :size="15" />
          </button>
          <button
            class="icon-button subtle"
            type="button"
            data-note-action="navigate-forward"
            :disabled="!canNavigateForward"
            :title="forwardNavigationLabel"
            :aria-label="forwardNavigationLabel"
            @click="navigateForward"
          >
            <AppIcon name="history-forward" :size="15" />
          </button>
        </div>
        <Transition name="chip-swap">
          <div v-if="!uiState.explorerOpen" class="vault-hidden-actions">
            <button
              type="button"
              class="icon-button subtle"
              aria-label="Create note"
              :title="`Create note · ${createNoteShortcut}`"
              @click="createNote()"
            >
              <AppIcon name="file-plus" :size="15" />
            </button>
            <div class="menu-anchor">
              <button
                ref="quickFolderButton"
                type="button"
                class="icon-button subtle"
                aria-label="Create folder"
                title="Create folder"
                :aria-expanded="quickFolderOpen"
                @mousedown.prevent
                @click="quickFolderOpen ? closeQuickFolder(true) : openQuickFolder()"
              >
                <AppIcon name="folder-plus" :size="15" />
              </button>
              <Transition name="popover-fade">
                <form
                  v-if="quickFolderOpen"
                  class="popover-menu quick-folder-popover"
                  @submit.prevent="submitQuickFolder"
                  @focusout="handleQuickFolderFocusOut"
                  @keydown.esc.prevent="closeQuickFolder(true)"
                >
                  <strong>New folder</strong>
                  <div class="quick-folder-entry">
                    <input
                      ref="quickFolderField"
                      v-model="quickFolderName"
                      type="text"
                      maxlength="120"
                      autocomplete="off"
                      aria-label="Folder name"
                      placeholder="Folder name"
                    />
                    <button type="submit" :disabled="!quickFolderName.trim()" aria-label="Create folder">
                      <AppIcon name="arrow" :size="14" />
                    </button>
                  </div>
                </form>
              </Transition>
            </div>
          </div>
        </Transition>
        <template v-if="activeNote">
          <span class="crumb-vault">{{ vaultState.name }}</span>
          <AppIcon name="chevron" :size="12" />
          <span v-if="activeNote.folderId" class="crumb-folder">{{ folderPath(activeNote.folderId) }}</span>
          <AppIcon v-if="activeNote.folderId" name="chevron" :size="12" />
          <span class="crumb-note">{{ activeNote.title || "Untitled note" }}</span>
        </template>
      </div>

      <div v-if="activeNote" class="editor-toolbar-actions">
        <button
          class="icon-button"
          type="button"
          data-note-action="embed-attachment"
          :disabled="!nativeAvailable || attachmentEmbedBusy || imageEmbedBusy || vaultSession.busy"
          aria-label="Embed file"
          :title="`Embed file · ${embedAttachmentShortcut}`"
          @click="requestAttachmentFromToolbar"
        >
          <AppIcon name="paperclip" :size="16" />
        </button>
        <button
          class="icon-button"
          type="button"
          data-note-action="embed-image"
          :disabled="!nativeAvailable || imageEmbedBusy || attachmentEmbedBusy || vaultSession.busy"
          aria-label="Embed image"
          :title="`Embed image · ${embedImageShortcut}`"
          @click="requestImageFromToolbar"
        >
          <AppIcon name="image" :size="16" />
        </button>
        <button
          class="icon-button"
          type="button"
          :class="{ active: activeNote.pinned }"
          :aria-label="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          :title="activeNote.pinned ? 'Remove from favorites' : 'Favorite'"
          @click="togglePinned(activeNote.id)"
        >
          <AppIcon name="star" :size="16" />
        </button>
        <button
          v-if="hasFrontmatter || uiState.frontmatterVisible"
          class="icon-button frontmatter-toggle"
          type="button"
          data-note-action="toggle-frontmatter"
          :class="{ active: uiState.frontmatterVisible }"
          :aria-label="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :title="uiState.frontmatterVisible ? 'Hide frontmatter' : 'Show frontmatter'"
          :aria-pressed="uiState.frontmatterVisible"
          @click="toggleFrontmatter"
        >
          <AppIcon name="code" :size="16" />
        </button>
        <button
          class="icon-button context-toggle"
          type="button"
          :class="{ active: uiState.contextOpen }"
          :aria-label="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          :title="uiState.contextOpen ? 'Hide note context' : 'Show note context'"
          @click="uiState.contextOpen = !uiState.contextOpen"
        >
          <AppIcon name="panel-right" :size="17" />
        </button>
        <div class="menu-anchor">
          <button class="icon-button" type="button" title="More actions" @click="noteMenuOpen = !noteMenuOpen">
            <AppIcon name="more" :size="18" />
          </button>
          <Transition name="popover-fade">
            <div v-if="noteMenuOpen" class="popover-menu compact-menu">
              <button type="button" class="danger" @click="requestDelete">
                <AppIcon name="trash" :size="15" /> Delete note
              </button>
            </div>
          </Transition>
        </div>
      </div>
    </header>

    <template v-if="activeNote">
      <section class="editor-document">
        <div class="document-heading">
          <input
            class="note-title-input"
            data-ui-region="note-title"
            :value="activeNote.title"
            aria-label="Note title"
            placeholder="Untitled note"
            @input="setTitle"
          />
          <div class="note-properties">
            <label class="property-control folder-property">
              <AppIcon name="folder" :size="14" />
              <select :value="activeNote.folderId ?? ''" aria-label="Move note to folder" @change="setFolder">
                <option value="">Vault root</option>
                <option v-for="folder in sortedFolders" :key="folder.id" :value="folder.id">
                  {{ folderPath(folder.id) }}
                </option>
              </select>
            </label>

            <span v-for="tag in activeNote.tags" :key="tag" class="tag-chip">
              <span>#</span>{{ tag }}
              <button type="button" :aria-label="`Remove ${tag} tag`" @click="removeTag(tag)">
                <AppIcon name="x" :size="10" />
              </button>
            </span>
            <Transition name="chip-swap" mode="out-in">
              <form v-if="tagInputOpen" key="tag-input" class="inline-tag-form" @submit.prevent="submitTag">
                <span>#</span>
                <input
                  ref="tagField"
                  v-model="tagInput"
                  placeholder="tag"
                  autocomplete="off"
                  autocapitalize="none"
                  autocorrect="off"
                  spellcheck="false"
                  role="combobox"
                  aria-autocomplete="list"
                  aria-controls="tag-suggestions"
                  :aria-expanded="tagSuggestions.length > 0"
                  :aria-activedescendant="tagSuggestionIndex >= 0 ? `tag-suggestion-${tagSuggestionIndex}` : undefined"
                  @blur="addTag()"
                  @keydown="handleTagKeydown"
                  @keydown.esc.prevent="cancelTagInput"
                />
                <div v-if="tagSuggestions.length" id="tag-suggestions" class="tag-suggestions" role="listbox">
                  <button
                    v-for="(tag, index) in tagSuggestions"
                    :id="`tag-suggestion-${index}`"
                    :key="tag"
                    type="button"
                    role="option"
                    :aria-selected="index === tagSuggestionIndex"
                    :class="{ active: index === tagSuggestionIndex }"
                    @mouseenter="tagSuggestionIndex = index"
                    @mousedown.prevent="addTag(tag)"
                  >
                    <span>#</span>{{ tag }}
                  </button>
                </div>
              </form>
              <button v-else key="tag-button" type="button" class="add-tag-button" @click="openTagInput">
                <AppIcon name="plus" :size="12" /> Add tag
              </button>
            </Transition>
          </div>
        </div>

        <div class="editor-canvas" data-editor-pane="live">
          <SourceEditor
            :key="editorKey"
            ref="sourceEditor"
            :initial-position="savedEditorPosition(activeNote.id, activeNote.content)"
            :attachment-files="vaultState.attachmentFiles"
            :attachment-refresh-token="uiState.attachmentRefreshToken"
            :embedded-attachments="vaultState.embeddedAttachments"
            :embedded-images="vaultState.embeddedImages"
            :image-refresh-token="uiState.imageRefreshToken"
            :model-value="activeNote.content"
            :note-id="activeNote.id"
            :note-relative-path="activeNote.relativePath"
            :note-titles="noteTitles"
            :rename-attachment="renameVaultAttachment"
            :show-frontmatter="uiState.frontmatterVisible"
            :vault-id="positionVaultId"
            :vault-path="vaultSession.path"
            @editor-position="rememberEditorPosition"
            @activate-attachment="activateEmbeddedAttachment"
            @external-file-drop="embedExternalFiles"
            @open-link="openRenderedLink"
            @open-wiki="openWikiLink"
            @paste-image="embedImageFromClipboard"
            @reveal-attachment-in-tree="revealEmbeddedAttachmentInTree"
            @request-embed-attachment="embedAttachmentFromFile"
            @vault-attachment-drop="embedAttachmentFromVault"
            @vault-image-drop="embedImageFromVault"
            @request-embed-image="embedImageFromFile"
            @show-attachment-in-folder="showEmbeddedAttachmentInFolder"
            @update:model-value="setContent"
          />
        </div>
      </section>

      <footer class="editor-statusbar">
        <div>
          <span class="status-dot" :class="uiState.saveStatus" />
          <span v-if="uiState.saveStatus === 'saving'">Saving…</span>
          <span v-else-if="uiState.saveStatus === 'error'">Couldn’t save</span>
          <span v-else>Saved</span>
        </div>
        <div class="status-stats">
          <span>Ln {{ activeNote.content.slice(0, activeNote.content.length).split('\n').length }}</span>
          <span>{{ wordCount }} words</span>
          <span>{{ characterCount.toLocaleString() }} characters</span>
          <span>Markdown</span>
        </div>
      </footer>
    </template>

    <div v-else class="empty-editor">
      <div class="empty-glyph"><AppIcon name="file-plus" :size="28" /></div>
      <h2>No note selected</h2>
      <p>Select a note or create a new one.</p>
    </div>
  </main>
</template>
