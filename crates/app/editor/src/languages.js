// Language support picked by file name / extension.

import { StreamLanguage } from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import { rust } from "@codemirror/lang-rust";
import { python } from "@codemirror/lang-python";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { go } from "@codemirror/lang-go";
import { cpp } from "@codemirror/lang-cpp";
import { java } from "@codemirror/lang-java";
import { php } from "@codemirror/lang-php";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { lua } from "@codemirror/legacy-modes/mode/lua";

const streams = new Map();
const stream = (parser) => () => {
  if (!streams.has(parser)) streams.set(parser, StreamLanguage.define(parser));
  return streams.get(parser);
};

const L = {
  js: ["JavaScript", () => javascript({ jsx: true })],
  ts: ["TypeScript", () => javascript({ typescript: true })],
  tsx: ["TSX", () => javascript({ jsx: true, typescript: true })],
  rust: ["Rust", () => rust()],
  python: ["Python", () => python()],
  html: ["HTML", () => html()],
  css: ["CSS", () => css()],
  scss: ["SCSS", () => css()],
  less: ["Less", () => css()],
  json: ["JSON", () => json()],
  markdown: ["Markdown", () => markdown({ codeLanguages: fencedLanguage })],
  go: ["Go", () => go()],
  c: ["C", () => cpp()],
  cpp: ["C++", () => cpp()],
  objc: ["Objective-C", () => cpp()],
  java: ["Java", () => java()],
  php: ["PHP", () => php()],
  sql: ["SQL", () => sql()],
  xml: ["XML", () => xml()],
  svg: ["SVG", () => xml()],
  plist: ["Property List", () => xml()],
  yaml: ["YAML", () => yaml()],
  shell: ["Shell", stream(shell)],
  toml: ["TOML", stream(toml)],
  docker: ["Dockerfile", stream(dockerFile)],
  swift: ["Swift", stream(swift)],
  ruby: ["Ruby", stream(ruby)],
  lua: ["Lua", stream(lua)],
};

const EXT = {
  js: "js", jsx: "js", mjs: "js", cjs: "js",
  ts: "ts", mts: "ts", cts: "ts", tsx: "tsx",
  rs: "rust",
  py: "python", pyi: "python", pyw: "python",
  html: "html", htm: "html", xhtml: "html",
  css: "css", scss: "scss", less: "less",
  json: "json", jsonc: "json", json5: "json", webmanifest: "json",
  md: "markdown", markdown: "markdown", mdx: "markdown",
  go: "go",
  c: "c", h: "c", cpp: "cpp", hpp: "cpp", cc: "cpp", cxx: "cpp", hh: "cpp", hxx: "cpp", m: "objc", mm: "objc",
  java: "java",
  php: "php",
  sql: "sql",
  xml: "xml", svg: "svg", plist: "plist", xib: "xml", storyboard: "xml",
  yaml: "yaml", yml: "yaml",
  sh: "shell", bash: "shell", zsh: "shell", env: "shell",
  toml: "toml",
  dockerfile: "docker",
  swift: "swift",
  rb: "ruby", rake: "ruby", gemspec: "ruby",
  lua: "lua",
};

const NAMES = {
  ".zshrc": "shell", ".zshenv": "shell", ".zprofile": "shell", ".zlogin": "shell",
  ".bashrc": "shell", ".bash_profile": "shell", ".profile": "shell", ".env": "shell",
  dockerfile: "docker", containerfile: "docker",
  gemfile: "ruby", rakefile: "ruby", podfile: "ruby", brewfile: "ruby",
  "cargo.lock": "toml", "poetry.lock": "toml", "uv.lock": "toml", "pipfile": "toml",
  "package.json": "json", "tsconfig.json": "json",
};

function keyFor(name) {
  const lower = name.toLowerCase();
  if (NAMES[lower]) return NAMES[lower];
  if (lower.startsWith(".env.")) return "shell";
  if (lower.startsWith("dockerfile.")) return "docker";
  const dot = lower.lastIndexOf(".");
  return dot >= 0 ? EXT[lower.slice(dot + 1)] : undefined;
}

export const WRAP = new Set(["markdown"]);

// {name, key, support()} for a file path; support() builds a fresh extension.
export function languageFor(path) {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const key = keyFor(name);
  if (!key) return { name: "Plain Text", key: null, support: () => [] };
  const [label, make] = L[key];
  return { name: label, key, support: make };
}

// Fenced code blocks in Markdown: map the info string through the same table.
const FENCE = { javascript: "js", typescript: "ts", rust: "rust", rs: "rust", python: "python", py: "python", shell: "shell", sh: "shell", bash: "shell", zsh: "shell", console: "shell", golang: "go", "c++": "cpp", yml: "yaml" };
function fencedLanguage(info) {
  const word = String(info).trim().toLowerCase().split(/\s/)[0];
  const key = FENCE[word] || EXT[word] || (L[word] ? word : null);
  if (!key || key === "markdown") return null;
  const ext = L[key][1]();
  return ext.language || ext;
}
