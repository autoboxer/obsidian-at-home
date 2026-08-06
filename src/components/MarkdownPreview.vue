<script setup lang="ts">
import { computed } from "vue";
import { renderMarkdown, resolveWikiLink } from "../lib";
import { vaultState } from "../stores/vault";

const props = defineProps<{
  content: string;
}>();

const emit = defineEmits<{
  openWiki: [target: string];
}>();

const html = computed(() => renderMarkdown(props.content, {
  resolveWikiLink: (target) => resolveWikiLink(target, vaultState.notes),
  externalLinksInNewTab: true,
}));

function handleClick(event: MouseEvent): void {
  const target = event.target as HTMLElement;
  const link = target.closest<HTMLAnchorElement>(".wiki-link");
  if (!link) return;
  event.preventDefault();
  emit("openWiki", link.dataset.wikiTarget ?? "");
}
</script>

<template>
  <article class="markdown-preview" @click="handleClick" v-html="html" />
</template>
