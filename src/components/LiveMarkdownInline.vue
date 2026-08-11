<script setup lang="ts">
import type { LiveMarkdownInlineSegment } from "../lib/liveMarkdownInline";

defineProps<{
  segments: readonly LiveMarkdownInlineSegment[];
}>();

function segmentClasses(
  segment: LiveMarkdownInlineSegment,
): Array<string | Record<string, boolean>> {
  return [
    `is-${segment.kind}`,
    ...segment.marks.map((mark) => `is-${mark}`),
    {
      "is-resolved": segment.resolved === true,
      "is-unresolved": segment.resolved === false,
    },
  ];
}

function segmentIsInteractive(segment: LiveMarkdownInlineSegment): boolean {
  return segment.kind === "wiki-link" || (
    segment.kind === "link" && Boolean(segment.href)
  );
}

function segmentHref(segment: LiveMarkdownInlineSegment): string {
  return segment.kind === "wiki-link" ? "#" : segment.href ?? "";
}

function segmentTarget(
  segment: LiveMarkdownInlineSegment,
): "_blank" | undefined {
  return segment.kind === "link" && /^(?:https?:)?\/\//i.test(segment.href ?? "")
    ? "_blank"
    : undefined;
}
</script>

<template>
  <span class="live-markdown-inline">
    <template
      v-for="(segment, index) in segments"
      :key="`${index}:${segment.kind}:${segment.text}`"
    >
      <a
        v-if="segmentIsInteractive(segment)"
        class="live-inline-segment"
        :class="segmentClasses(segment)"
        :href="segmentHref(segment)"
        :title="segment.title"
        :target="segmentTarget(segment)"
        rel="noopener noreferrer"
        :data-wiki-target="segment.wikiTarget"
        :data-wiki-heading="segment.wikiHeading"
        @mousedown.prevent
      >{{ segment.text }}</a>
      <span
        v-else
        class="live-inline-segment"
        :class="segmentClasses(segment)"
        aria-hidden="true"
      >{{ segment.text }}</span>
    </template>
  </span>
</template>
