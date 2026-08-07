<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed } from "vue";
import { renderMarkdown, resolveWikiLink, toggleMarkdownTask } from "../lib";
import { notify, vaultState } from "../stores/vault";

const props = defineProps<{
  content: string;
}>();

const emit = defineEmits<{
  openWiki: [target: string];
  "update:content": [content: string];
}>();

const html = computed(() => renderMarkdown(props.content, {
  resolveWikiLink: (target) => resolveWikiLink(target, vaultState.notes),
  externalLinksInNewTab: true,
}));

async function handleClick(event: MouseEvent): Promise<void> {
  const target = event.target as HTMLElement;
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
