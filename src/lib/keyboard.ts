const APPLE_USER_AGENT = /Macintosh|Mac OS|iPhone|iPad|iPod/;

export function shortcutCommandKey(
  userAgent = globalThis.navigator?.userAgent ?? "",
): "⌘" | "Ctrl" {
  return APPLE_USER_AGENT.test(userAgent) ? "⌘" : "Ctrl";
}

export function formatCommandShortcut(
  key: string,
  userAgent = globalThis.navigator?.userAgent ?? "",
): string {
  const commandKey = shortcutCommandKey(userAgent);

  return `${commandKey}${commandKey === "⌘" ? "" : "+"}${key}`;
}
