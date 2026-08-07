<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed } from "vue";
import { renderMarkdown, resolveWikiLink, toggleMarkdownTask } from "../lib";
import { notify, vaultState } from "../stores/vault";

const props = defineProps<{
  content: string;
  noteId: string;
  collapsibleHeadings?: boolean;
}>();

const emit = defineEmits<{
  openWiki: [target: string];
  "update:content": [content: string];
}>();

const collapsedHeadingsByNote = new Map<string, Set<string>>();

const html = computed(() => renderMarkdown(props.content, {
  resolveWikiLink: (target) => resolveWikiLink(target, vaultState.notes),
  externalLinksInNewTab: true,
  collapsibleHeadings: props.collapsibleHeadings,
  collapsedHeadingKeys: collapsedHeadingsByNote.get(props.noteId),
}));

async function handleClick(event: MouseEvent): Promise<void> {
  const target = event.target as HTMLElement;
  const headingToggle = target.closest<HTMLButtonElement>(".markdown-heading-toggle");
  if (headingToggle) {
    toggleHeadingSection(headingToggle);

    return;
  }

  const wikiLink = target.closest<HTMLAnchorElement>(".wiki-link");
  if (wikiLink) {
    event.preventDefault();
    emit("openWiki", wikiLink.dataset.wikiTarget ?? "");

    return;
  }

  const externalLink = target.closest<HTMLAnchorElement>(
    'a[href^="http://"], a[href^="https://"], a[href^="mailto:"]',
  );
  if (!externalLink || !window.__TAURI__) {
    return;
  }

  event.preventDefault();
  try {
    await openUrl(externalLink.href);
  } catch {
    notify("Could not open that link", "warning");
  }
}

function toggleHeadingSection(toggle: HTMLButtonElement): void {
  const section = toggle.closest<HTMLElement>(".markdown-heading-section");
  const body = section?.querySelector<HTMLElement>(":scope > .markdown-heading-body");
  const headingKey = section?.dataset.headingKey;
  if (!section || !body || !headingKey) {
    return;
  }

  const isExpanded = toggle.getAttribute("aria-expanded") === "true";
  const willExpand = !isExpanded;

  toggle.setAttribute("aria-expanded", String(willExpand));
  toggle.setAttribute("aria-label", `${willExpand ? "Collapse" : "Expand"} section`);
  body.hidden = !willExpand;
  section.classList.toggle("is-collapsed", !willExpand);

  let collapsedHeadings = collapsedHeadingsByNote.get(props.noteId);
  if (!collapsedHeadings) {
    collapsedHeadings = new Set();
    collapsedHeadingsByNote.set(props.noteId, collapsedHeadings);
  }

  if (willExpand) {
    collapsedHeadings.delete(headingKey);
  } else {
    collapsedHeadings.add(headingKey);
  }
}

function handleChange(event: Event): void {
  const checkbox = (event.target as HTMLElement).closest<HTMLInputElement>(
    'input[type="checkbox"][data-task-index]',
  );
  if (!checkbox) {
    return;
  }

  const taskIndex = Number.parseInt(checkbox.dataset.taskIndex ?? "", 10);
  const updatedContent = toggleMarkdownTask(props.content, taskIndex, checkbox.checked);
  if (updatedContent !== props.content) {
    emit("update:content", updatedContent);
  }
}
</script>

<template>
  <article class="markdown-preview" @click="handleClick" @change="handleChange" v-html="html" />
</template>
