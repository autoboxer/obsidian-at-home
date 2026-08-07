import { getCurrentWindow } from "@tauri-apps/api/window";
import { computed, reactive, ref } from "vue";
import { listSystemFonts } from "../services/native";

export type ThemeId =
  | "aubergine"
  | "dracula"
  | "one-dark"
  | "tokyo-night"
  | "catppuccin-mocha"
  | "nord"
  | "gruvbox-dark"
  | "solarized-dark"
  | "github-light"
  | "one-light"
  | "catppuccin-latte"
  | "gruvbox-light"
  | "solarized-light"
  | "rose-pine-dawn";

type BundledFontId =
  | "inter"
  | "system-ui"
  | "ibm-plex-sans"
  | "source-serif-4"
  | "jetbrains-mono";

export type FontId = BundledFontId | `system:${string}`;

type ColorScheme = "dark" | "light";

interface ThemePreview {
  background: string;
  surface: string;
  text: string;
  accent: string;
}

export interface ThemeOption {
  id: ThemeId;
  label: string;
  description: string;
  mode: ColorScheme;
  preview: ThemePreview;
}

export interface FontOption {
  id: FontId;
  label: string;
  description: string;
  family: string;
  monospaced?: boolean;
}

interface AppearanceState {
  themeId: ThemeId;
  fontId: FontId;
  noteFontSize: number;
}

interface StoredAppearance {
  version: number;
  themeId: ThemeId;
  fontId: FontId;
  noteFontSize: number;
}

export const themes: readonly ThemeOption[] = [
  {
    id: "aubergine",
    label: "Aubergine",
    description: "Soft violet on deep charcoal",
    mode: "dark",
    preview: { background: "#0d0c12", surface: "#17141d", text: "#f0edf5", accent: "#a88ae8" },
  },
  {
    id: "dracula",
    label: "Dracula",
    description: "Vivid purple with bright contrast",
    mode: "dark",
    preview: { background: "#282a36", surface: "#44475a", text: "#f8f8f2", accent: "#bd93f9" },
  },
  {
    id: "one-dark",
    label: "One Dark",
    description: "Atom's balanced developer palette",
    mode: "dark",
    preview: { background: "#282c34", surface: "#3e4451", text: "#abb2bf", accent: "#c678dd" },
  },
  {
    id: "tokyo-night",
    label: "Tokyo Night",
    description: "Cool blue with neon violet",
    mode: "dark",
    preview: { background: "#1a1b26", surface: "#24283b", text: "#c0caf5", accent: "#bb9af7" },
  },
  {
    id: "catppuccin-mocha",
    label: "Catppuccin Mocha",
    description: "Warm pastel accents on navy",
    mode: "dark",
    preview: { background: "#1e1e2e", surface: "#313244", text: "#cdd6f4", accent: "#cba6f7" },
  },
  {
    id: "nord",
    label: "Nord",
    description: "Calm arctic blues and frost",
    mode: "dark",
    preview: { background: "#2e3440", surface: "#3b4252", text: "#eceff4", accent: "#b48ead" },
  },
  {
    id: "gruvbox-dark",
    label: "Gruvbox Dark",
    description: "Retro warmth with earthy contrast",
    mode: "dark",
    preview: { background: "#282828", surface: "#3c3836", text: "#ebdbb2", accent: "#d3869b" },
  },
  {
    id: "solarized-dark",
    label: "Solarized Dark",
    description: "Low-glare teal and balanced accents",
    mode: "dark",
    preview: { background: "#002b36", surface: "#073642", text: "#839496", accent: "#6c71c4" },
  },
  {
    id: "github-light",
    label: "GitHub Light",
    description: "Crisp, familiar code-hosting clarity",
    mode: "light",
    preview: { background: "#ffffff", surface: "#f6f8fa", text: "#1f2328", accent: "#8250df" },
  },
  {
    id: "one-light",
    label: "One Light",
    description: "Clean Atom-inspired neutrals",
    mode: "light",
    preview: { background: "#fafafa", surface: "#f0f0f0", text: "#383a42", accent: "#a626a4" },
  },
  {
    id: "catppuccin-latte",
    label: "Catppuccin Latte",
    description: "Soft lavender on a cool canvas",
    mode: "light",
    preview: { background: "#eff1f5", surface: "#e6e9ef", text: "#4c4f69", accent: "#8839ef" },
  },
  {
    id: "gruvbox-light",
    label: "Gruvbox Light",
    description: "Warm paper with retro accents",
    mode: "light",
    preview: { background: "#fbf1c7", surface: "#ebdbb2", text: "#3c3836", accent: "#8f3f71" },
  },
  {
    id: "solarized-light",
    label: "Solarized Light",
    description: "Cream canvas with calibrated color",
    mode: "light",
    preview: { background: "#fdf6e3", surface: "#eee8d5", text: "#586e75", accent: "#6c71c4" },
  },
  {
    id: "rose-pine-dawn",
    label: "Rosé Pine Dawn",
    description: "Gentle rose tones on warm paper",
    mode: "light",
    preview: { background: "#faf4ed", surface: "#f2e9e1", text: "#575279", accent: "#907aa9" },
  },
];

export const fontOptions: readonly FontOption[] = [
  {
    id: "inter",
    label: "Inter",
    description: "Clean and neutral",
    family: '"Inter Variable", Inter, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  },
  {
    id: "system-ui",
    label: "System UI",
    description: "Matches your device",
    family: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
  },
  {
    id: "ibm-plex-sans",
    label: "IBM Plex Sans",
    description: "Technical and readable",
    family: '"IBM Plex Sans Variable", "IBM Plex Sans", "Segoe UI", system-ui, sans-serif',
  },
  {
    id: "source-serif-4",
    label: "Source Serif 4",
    description: "Comfortable long-form serif",
    family: '"Source Serif 4 Variable", "Source Serif 4", "Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif',
  },
  {
    id: "jetbrains-mono",
    label: "JetBrains Mono",
    description: "Developer-friendly monospace",
    family: '"JetBrains Mono Variable", "JetBrains Mono", "Cascadia Code", "SFMono-Regular", Consolas, "Liberation Mono", monospace',
    monospaced: true,
  },
];

export const installedFontOptions = ref<FontOption[]>([]);
export const installedFontsLoading = ref(false);
export const installedFontsUnavailable = ref(false);

export const MIN_NOTE_FONT_SIZE = 13;
export const MAX_NOTE_FONT_SIZE = 22;

const APPEARANCE_STORAGE_KEY = "obsidian-at-home.appearance.v1";
const APPEARANCE_STORAGE_VERSION = 1;
const APPEARANCE_STYLE_ID = "obsidian-at-home-appearance";
const DEFAULT_THEME_ID: ThemeId = "aubergine";
const DEFAULT_FONT_ID: FontId = "inter";
const DEFAULT_NOTE_FONT_SIZE = 16;
const themeIds = new Set<ThemeId>(themes.map((theme) => theme.id));
const bundledFontIds = new Set<FontId>(fontOptions.map((font) => font.id));
const bundledFontFamilies = new Set(
  ["Inter", "Inter Variable", "System UI", "IBM Plex Sans", "Source Serif 4", "JetBrains Mono"]
    .map(normalizeFontFamily),
);

export const appearanceState = reactive<AppearanceState>({
  themeId: DEFAULT_THEME_ID,
  fontId: DEFAULT_FONT_ID,
  noteFontSize: DEFAULT_NOTE_FONT_SIZE,
});

export const selectedFontOption = computed(() => resolveFontOption(appearanceState.fontId));

let initialized = false;
let installedFontsLoaded = false;
let installedFontsRequest: Promise<void> | null = null;
let nativeThemeRequest = 0;
let synchronizedNativeTheme: ColorScheme | null = null;

export function initializeAppearance(): void {
  if (!initialized) {
    Object.assign(appearanceState, readStoredAppearance());
    initialized = true;
  }

  applyAppearance();
}

export function setAppearanceTheme(themeId: ThemeId): void {
  if (!themeIds.has(themeId)) {
    return;
  }

  appearanceState.themeId = themeId;
  commitAppearance();
}

export function setAppearanceFont(fontId: string): void {
  if (!isFontId(fontId)) {
    return;
  }

  appearanceState.fontId = fontId;
  commitAppearance();
}

export function setAppearanceFontSize(fontSize: number): void {
  if (!Number.isFinite(fontSize)) {
    return;
  }

  appearanceState.noteFontSize = clampFontSize(fontSize);
  commitAppearance();
}

export function resetAppearancePreferences(): void {
  appearanceState.themeId = DEFAULT_THEME_ID;
  appearanceState.fontId = DEFAULT_FONT_ID;
  appearanceState.noteFontSize = DEFAULT_NOTE_FONT_SIZE;
  removeStoredAppearance();
  applyAppearance();
}

export function loadInstalledFonts(): Promise<void> {
  if (installedFontsLoaded) {
    return Promise.resolve();
  }
  if (installedFontsRequest) {
    return installedFontsRequest;
  }
  if (typeof window === "undefined" || !window.__TAURI__?.core?.invoke) {
    installedFontsUnavailable.value = true;

    return Promise.resolve();
  }

  installedFontsLoading.value = true;
  installedFontsUnavailable.value = false;
  const request = listSystemFonts()
    .then((systemFonts: Array<{ family: string; monospaced: boolean }>) => {
      const seen = new Set<string>();
      const options: FontOption[] = [];

      for (const systemFont of systemFonts) {
        const family = sanitizeFontFamily(systemFont.family);
        if (!family) {
          continue;
        }

        const normalized = normalizeFontFamily(family);
        if (seen.has(normalized) || bundledFontFamilies.has(normalized)) {
          continue;
        }

        seen.add(normalized);
        options.push({
          id: systemFontId(family),
          label: family,
          description: systemFont.monospaced ? "Installed monospace" : "Installed on this device",
          family: systemFontStack(family, systemFont.monospaced),
          monospaced: systemFont.monospaced,
        });
      }

      installedFontOptions.value = options.sort((left, right) => left.label.localeCompare(right.label));
      installedFontsLoaded = true;
      applyAppearance();
    })
    .catch(() => {
      installedFontOptions.value = [];
      installedFontsUnavailable.value = true;
    })
    .finally(() => {
      installedFontsLoading.value = false;
      installedFontsRequest = null;
    });

  installedFontsRequest = request;

  return request;
}

export function applyAppearance(): void {
  if (typeof document === "undefined") {
    return;
  }

  const theme = themes.find((option) => option.id === appearanceState.themeId) ?? themes[0];
  const font = resolveFontOption(appearanceState.fontId);
  const root = document.documentElement;

  root.dataset.theme = theme.id;
  root.dataset.colorScheme = theme.mode;
  root.style.colorScheme = theme.mode;

  updateTypographyStyle(font.family, clampFontSize(appearanceState.noteFontSize));
  updateThemeColor(theme.preview.background);
  synchronizeNativeTheme(theme.mode);
}

function commitAppearance(): void {
  persistAppearance();
  applyAppearance();
}

function readStoredAppearance(): AppearanceState {
  const defaults: AppearanceState = {
    themeId: DEFAULT_THEME_ID,
    fontId: DEFAULT_FONT_ID,
    noteFontSize: DEFAULT_NOTE_FONT_SIZE,
  };
  const raw = readStorage();

  if (!raw) {
    return defaults;
  }

  try {
    const stored = JSON.parse(raw) as Partial<StoredAppearance> | null;

    if (!stored || typeof stored !== "object" || stored.version !== APPEARANCE_STORAGE_VERSION) {
      return defaults;
    }

    return {
      themeId: isThemeId(stored.themeId) ? stored.themeId : defaults.themeId,
      fontId: isFontId(stored.fontId) ? stored.fontId : defaults.fontId,
      noteFontSize: typeof stored.noteFontSize === "number" && Number.isFinite(stored.noteFontSize)
        ? clampFontSize(stored.noteFontSize)
        : defaults.noteFontSize,
    };
  } catch {
    return defaults;
  }
}

function persistAppearance(): void {
  const stored: StoredAppearance = {
    version: APPEARANCE_STORAGE_VERSION,
    themeId: appearanceState.themeId,
    fontId: appearanceState.fontId,
    noteFontSize: clampFontSize(appearanceState.noteFontSize),
  };

  try {
    window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(stored));
  } catch {
    // Appearance preferences are non-critical when browser storage is unavailable.
  }
}

function readStorage(): string | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    return window.localStorage.getItem(APPEARANCE_STORAGE_KEY);
  } catch {
    return null;
  }
}

function removeStoredAppearance(): void {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.removeItem(APPEARANCE_STORAGE_KEY);
  } catch {
    // Appearance preferences are non-critical when browser storage is unavailable.
  }
}

function isThemeId(value: unknown): value is ThemeId {
  return typeof value === "string" && themeIds.has(value as ThemeId);
}

function isFontId(value: unknown): value is FontId {
  if (typeof value !== "string") {
    return false;
  }

  return bundledFontIds.has(value as FontId) || systemFontFamily(value) !== null;
}

function clampFontSize(fontSize: number): number {
  return Math.min(MAX_NOTE_FONT_SIZE, Math.max(MIN_NOTE_FONT_SIZE, Math.round(fontSize)));
}

function resolveFontOption(fontId: FontId): FontOption {
  const bundled = fontOptions.find((option) => option.id === fontId);
  if (bundled) {
    return bundled;
  }

  const installed = installedFontOptions.value.find((option) => option.id === fontId);
  if (installed) {
    return installed;
  }

  const family = systemFontFamily(fontId);
  if (family) {
    return {
      id: fontId,
      label: family,
      description: "Installed on this device",
      family: systemFontStack(family, false),
    };
  }

  return fontOptions[0];
}

function systemFontId(family: string): `system:${string}` {
  return `system:${encodeURIComponent(family)}`;
}

function systemFontFamily(value: string): string | null {
  if (!value.startsWith("system:")) {
    return null;
  }

  const encodedFamily = value.slice("system:".length);
  if (!encodedFamily) {
    return null;
  }

  try {
    const family = sanitizeFontFamily(decodeURIComponent(encodedFamily));

    return family && systemFontId(family) === value ? family : null;
  } catch {
    return null;
  }
}

function sanitizeFontFamily(family: unknown): string | null {
  if (typeof family !== "string") {
    return null;
  }

  const trimmed = family.trim();
  if (!trimmed || trimmed.length > 160 || /[\u0000-\u001f\u007f]/u.test(trimmed)) {
    return null;
  }

  return trimmed;
}

function normalizeFontFamily(family: string): string {
  return family.trim().toLocaleLowerCase();
}

function systemFontStack(family: string, monospaced: boolean): string {
  const quotedFamily = `"${family.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;

  return monospaced
    ? `${quotedFamily}, "Cascadia Code", "SFMono-Regular", Consolas, "Liberation Mono", monospace`
    : `${quotedFamily}, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
}

function updateTypographyStyle(fontFamily: string, fontSize: number): void {
  let style = document.querySelector<HTMLStyleElement>(`#${APPEARANCE_STYLE_ID}`);

  if (!style) {
    style = document.createElement("style");
    style.id = APPEARANCE_STYLE_ID;
    document.head.append(style);
  }

  style.textContent = `:root {\n  --note-font-family: ${fontFamily};\n  --note-font-size: ${fontSize}px;\n}`;
}

function updateThemeColor(color: string): void {
  let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');

  if (!meta) {
    meta = document.createElement("meta");
    meta.name = "theme-color";
    document.head.append(meta);
  }

  meta.content = color;
}

function synchronizeNativeTheme(scheme: ColorScheme): void {
  if (typeof window === "undefined" || !window.__TAURI__?.core?.invoke) {
    return;
  }

  if (synchronizedNativeTheme === scheme) {
    return;
  }

  synchronizedNativeTheme = scheme;
  const request = ++nativeThemeRequest;

  void getCurrentWindow().setTheme(scheme).catch(() => {
    if (request === nativeThemeRequest) {
      synchronizedNativeTheme = null;
    }

    // Browser previews and older webviews continue using the CSS color scheme.
  });
}
