import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import go from "highlight.js/lib/languages/go";
import graphql from "highlight.js/lib/languages/graphql";
import http from "highlight.js/lib/languages/http";
import ini from "highlight.js/lib/languages/ini";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import makefile from "highlight.js/lib/languages/makefile";
import markdown from "highlight.js/lib/languages/markdown";
import nginx from "highlight.js/lib/languages/nginx";
import protobuf from "highlight.js/lib/languages/protobuf";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import wasm from "highlight.js/lib/languages/wasm";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const MAX_HIGHLIGHT_CHARS = 250_000;

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("css", css);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("dockerfile", dockerfile);
hljs.registerLanguage("go", go);
hljs.registerLanguage("graphql", graphql);
hljs.registerLanguage("http", http);
hljs.registerLanguage("ini", ini);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("makefile", makefile);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("nginx", nginx);
hljs.registerLanguage("protobuf", protobuf);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("wasm", wasm);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("yaml", yaml);

export interface CodeLanguageOption {
  value: string;
  label: string;
  aliases: readonly string[];
  highlightAs?: string;
}

export interface CodeHighlightRange {
  className: string;
  from: number;
  to: number;
}

interface OpenHighlightRange {
  className: string;
  from: number;
}

function codeLanguage(
  value: string,
  label: string,
  aliases: readonly string[] = [],
  highlightAs = value || undefined,
): CodeLanguageOption {
  return {
    value,
    label,
    aliases,
    ...(highlightAs ? { highlightAs } : {}),
  };
}

export const CODE_LANGUAGE_OPTIONS: readonly CodeLanguageOption[] = [
  codeLanguage("", "Plain text", ["text", "txt", "none"]),
  codeLanguage("bash", "Bash / Shell", ["shell", "sh", "zsh"]),
  codeLanguage("css", "CSS"),
  codeLanguage("diff", "Diff / Patch", ["patch"]),
  codeLanguage("dockerfile", "Dockerfile", ["docker"]),
  codeLanguage("go", "Go", ["golang"]),
  codeLanguage("graphql", "GraphQL", ["gql"]),
  codeLanguage(
    "html",
    "HTML / XML",
    ["xml", "svg", "xhtml", "plist"],
    "xml",
  ),
  codeLanguage("http", "HTTP", ["https"]),
  codeLanguage("javascript", "JavaScript", ["js", "jsx", "mjs", "cjs"]),
  codeLanguage("json", "JSON", ["jsonc"]),
  codeLanguage("makefile", "Makefile", ["make", "mk", "mak"]),
  codeLanguage("markdown", "Markdown", ["md", "mkdown", "mkd"]),
  codeLanguage("nginx", "Nginx", ["nginxconf"]),
  codeLanguage("protobuf", "Protocol Buffers", ["proto"]),
  codeLanguage("python", "Python", ["py", "gyp", "ipython"]),
  codeLanguage("rust", "Rust", ["rs"]),
  codeLanguage("sql", "SQL"),
  codeLanguage("toml", "TOML / INI", ["ini"], "ini"),
  codeLanguage(
    "typescript",
    "TypeScript",
    ["ts", "tsx", "mts", "cts"],
  ),
  codeLanguage("vue", "Vue", [], "xml"),
  codeLanguage("wasm", "WebAssembly", ["wat", "webassembly"]),
  codeLanguage("yaml", "YAML", ["yml"]),
];

const LANGUAGE_ALIASES = new Map<string, string>();

for (const option of CODE_LANGUAGE_OPTIONS) {
  if (!option.highlightAs) {
    continue;
  }

  LANGUAGE_ALIASES.set(option.value, option.highlightAs);
  for (const alias of option.aliases) {
    LANGUAGE_ALIASES.set(alias, option.highlightAs);
  }
}

export function findCodeLanguageOption(
  language: string,
): CodeLanguageOption | undefined {
  const normalized = language.trim().toLocaleLowerCase();

  return CODE_LANGUAGE_OPTIONS.find((option) =>
    option.value === normalized || option.aliases.includes(normalized)
  );
}

/** Highlight only explicitly supported top-level fence languages */
export function highlightCode(
  code: string,
  language: string,
): string | undefined {
  if (code.length > MAX_HIGHLIGHT_CHARS) {
    return undefined;
  }
  const normalized = LANGUAGE_ALIASES.get(language.toLowerCase());
  if (!normalized) {
    return undefined;
  }
  try {
    return hljs.highlight(code, { language: normalized, ignoreIllegals: true }).value;
  } catch {
    return undefined;
  }
}

export function highlightCodeRanges(
  code: string,
  language: string,
): CodeHighlightRange[] {
  if (code.length > MAX_HIGHLIGHT_CHARS) {
    return [];
  }
  const normalized = LANGUAGE_ALIASES.get(language.toLowerCase());
  if (!normalized) {
    return [];
  }

  try {
    const result = hljs.highlight(code, {
      language: normalized,
      ignoreIllegals: true,
    });
    return parseHighlightRanges(result.value, code);
  } catch {
    return [];
  }
}

function parseHighlightRanges(
  highlighted: string,
  source: string,
): CodeHighlightRange[] {
  const ranges: CodeHighlightRange[] = [];
  const openRanges: OpenHighlightRange[] = [];
  const markup = /<span class="([^"]+)">|<\/span>|([^<]+)/g;
  let plainText = "";
  let match: RegExpExecArray | null;

  while ((match = markup.exec(highlighted))) {
    if (match[1]) {
      openRanges.push({ className: match[1], from: plainText.length });
    } else if (match[0] === "</span>") {
      const range = openRanges.pop();
      if (range && range.from < plainText.length) {
        ranges.push({ ...range, to: plainText.length });
      }
    } else {
      plainText += decodeHighlightText(match[2] ?? "");
    }
  }

  return plainText === source && openRanges.length === 0 ? ranges : [];
}

function decodeHighlightText(value: string): string {
  const entities: Record<string, string> = {
    "&amp;": "&",
    "&gt;": ">",
    "&lt;": "<",
    "&quot;": '"',
    "&#x27;": "'",
  };

  return value.replace(
    /&(?:amp|gt|lt|quot|#x27);/g,
    (entity) => entities[entity] ?? entity,
  );
}
