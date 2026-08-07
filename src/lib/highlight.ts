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

const LANGUAGE_ALIASES: Record<string, string> = {
  bash: "bash",
  shell: "bash",
  sh: "bash",
  zsh: "bash",
  css: "css",
  diff: "diff",
  patch: "diff",
  dockerfile: "dockerfile",
  docker: "dockerfile",
  go: "go",
  golang: "go",
  graphql: "graphql",
  gql: "graphql",
  http: "http",
  https: "http",
  ini: "ini",
  toml: "ini",
  javascript: "javascript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  jsonc: "json",
  makefile: "makefile",
  make: "makefile",
  mk: "makefile",
  mak: "makefile",
  markdown: "markdown",
  md: "markdown",
  mkdown: "markdown",
  mkd: "markdown",
  nginx: "nginx",
  nginxconf: "nginx",
  protobuf: "protobuf",
  proto: "protobuf",
  python: "python",
  py: "python",
  gyp: "python",
  ipython: "python",
  rust: "rust",
  rs: "rust",
  sql: "sql",
  typescript: "typescript",
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  cts: "typescript",
  wasm: "wasm",
  wat: "wasm",
  webassembly: "wasm",
  vue: "xml",
  html: "xml",
  xml: "xml",
  svg: "xml",
  xhtml: "xml",
  plist: "xml",
  yaml: "yaml",
  yml: "yaml",
};

/** Highlight only explicitly supported top-level fence languages. */
export function highlightCode(code: string, language: string): string | undefined {
  if (code.length > MAX_HIGHLIGHT_CHARS) return undefined;
  const normalized = LANGUAGE_ALIASES[language.toLowerCase()];
  if (!normalized) return undefined;
  try {
    return hljs.highlight(code, { language: normalized, ignoreIllegals: true }).value;
  } catch {
    return undefined;
  }
}
