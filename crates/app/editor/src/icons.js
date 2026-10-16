// Monochrome 16px line icons. They inherit currentColor.

const PAGE = '<path d="M4 1.75h5.25l3.25 3.25v9.25H4z"/><path d="M9.25 1.75V5h3.25"/>';

const P = {
  file: PAGE,
  code: PAGE + '<path d="M7.1 8.1 5.75 9.6l1.35 1.5M9.4 8.1l1.35 1.5-1.35 1.5"/>',
  doc: PAGE + '<path d="M6 8h4.5M6 10.25h4.5M6 12.5h2.75"/>',
  braces:
    PAGE +
    '<path d="M7.1 7.75c-.75 0-1.05.35-1.05.95v.55c0 .4-.25.65-.65.65.4 0 .65.25.65.65v.55c0 .6.3.95 1.05.95M9.4 7.75c.75 0 1.05.35 1.05.95v.55c0 .4.25.65.65.65-.4 0-.65.25-.65.65v.55c0 .6-.3.95-1.05.95"/>',
  image: '<rect x="2.25" y="2.75" width="11.5" height="10.5" rx="1.5"/><circle cx="6" cy="6.5" r="1.1"/><path d="m2.75 12 3.5-3.5 2.5 2.25 1.75-1.5 2.75 2.5"/>',
  terminal: '<rect x="2" y="2.75" width="12" height="10.5" rx="1.75"/><path d="m4.75 6.25 2 1.75-2 1.75M8.5 10.25h2.75"/>',
  lock: '<rect x="3.75" y="7.25" width="8.5" height="6.5" rx="1.25"/><path d="M5.75 7.25V5.5a2.25 2.25 0 0 1 4.5 0v1.75"/>',
  git: '<circle cx="5" cy="4" r="1.4"/><circle cx="5" cy="12" r="1.4"/><circle cx="11" cy="6.5" r="1.4"/><path d="M5 5.4v5.2M11 7.9c0 2-6 1.5-6 2.7"/>',
  folder: '<path d="M1.75 4.5c0-.7.55-1.25 1.25-1.25h3l1.5 1.5h5.5c.7 0 1.25.55 1.25 1.25v6c0 .7-.55 1.25-1.25 1.25H3c-.7 0-1.25-.55-1.25-1.25z"/>',
  folderOpen:
    '<path d="M1.75 11.75V4.5c0-.7.55-1.25 1.25-1.25h3l1.5 1.5h4.75c.7 0 1.25.55 1.25 1.25v.75"/><path d="M1.75 11.75 3.4 7.6c.2-.5.6-.85 1.15-.85h9.2c.5 0 .85.5.65.95L12.8 12c-.2.45-.6.75-1.1.75H2.75c-.55 0-1-.45-1-1z"/>',
  chevron: '<path d="m6 4 4 4-4 4"/>',
  chevronDown: '<path d="m4 6 4 4 4-4"/>',
  plus: '<path d="M8 3.5v9M3.5 8h9"/>',
  minus: '<path d="M3.5 8h9"/>',
  discard: '<path d="M4.5 6.25h5.25a3 3 0 0 1 0 6H7.5"/><path d="M6.75 3.75 4.25 6.25l2.5 2.5"/>',
  openFile: PAGE + '<path d="M6.25 10.75 9 8M6.75 8H9v2.25"/>',
  branch: '<circle cx="4.75" cy="3.75" r="1.5"/><circle cx="4.75" cy="12.25" r="1.5"/><circle cx="11.25" cy="5.25" r="1.5"/><path d="M4.75 5.25v5.5M11.25 6.75c0 2.75-6.5 2.25-6.5 4"/>',
  check: '<path d="m3.5 8.5 3 3 6-7"/>',
  close: '<path d="m4.5 4.5 7 7m0-7-7 7"/>',
  more: '<circle cx="3.75" cy="8" r=".6" fill="currentColor"/><circle cx="8" cy="8" r=".6" fill="currentColor"/><circle cx="12.25" cy="8" r=".6" fill="currentColor"/>',
  refresh: '<path d="M13 8a5 5 0 1 1-1.46-3.54"/><path d="M13 2.75V5.5h-2.75"/>',
  search: '<circle cx="7" cy="7" r="4.25"/><path d="m10.25 10.25 3.25 3.25"/>',
  collapse: '<path d="m5 3 3 3 3-3M5 13l3-3 3 3"/>',
  split: '<rect x="2" y="2.75" width="12" height="10.5" rx="1.5"/><path d="M8 2.75v10.5"/>',
  inline: '<rect x="2" y="2.75" width="12" height="10.5" rx="1.5"/><path d="M4.75 6h6.5M4.75 8h6.5M4.75 10h4"/>',
  diff: PAGE + '<path d="M8.25 6.75v3.5M6.5 8.5H10M6.5 12h3.5"/>',
  up: '<path d="M8 13V3.5M4.25 7.25 8 3.5l3.75 3.75"/>',
  down: '<path d="M8 3v9.5M4.25 8.75 8 12.5l3.75-3.75"/>',
  arrowUp: '<path d="m4.5 9.5 3.5-3.5 3.5 3.5"/>',
  arrowDown: '<path d="m4.5 6.5 3.5 3.5 3.5-3.5"/>',
  cloudUp: '<path d="M4.75 12.25H4.5a3 3 0 0 1-.4-5.97 4 4 0 0 1 7.8.1A2.75 2.75 0 0 1 11.5 12.25h-.25"/><path d="M8 13.5V8.25M6 10.25l2-2 2 2"/>',
  stash: '<rect x="2.25" y="3" width="11.5" height="3.25" rx=".75"/><path d="M3.25 6.25v6c0 .4.35.75.75.75h8c.4 0 .75-.35.75-.75v-6M6.5 9h3"/>',
  unstash: '<rect x="2.25" y="3" width="11.5" height="3.25" rx=".75"/><path d="M3.25 6.25v6c0 .4.35.75.75.75h8c.4 0 .75-.35.75-.75v-6M8 11.5V8.25M6.5 9.5 8 8l1.5 1.5"/>',
  newBranch: '<circle cx="4.75" cy="3.75" r="1.5"/><circle cx="4.75" cy="12.25" r="1.5"/><path d="M4.75 5.25v5.5M11.5 3.5v5M9 6h5"/>',
  fold: '<path d="M2.75 8h10.5M5.5 4.5 8 2l2.5 2.5M5.5 11.5 8 14l2.5-2.5"/>',
  warning: '<path d="M8 2.5 14 13H2z"/><path d="M8 6.5v3M8 11.25v.01"/>',
  binary: PAGE + '<path d="M6.25 8.25v3M9.75 8.25v3"/><rect x="7.4" y="8.25" width="1.2" height="3" rx=".6"/>',
  commit: '<circle cx="8" cy="8" r="2.6"/><path d="M1.75 8h3.65M10.6 8h3.65"/>',
  history: '<path d="M2.9 8.6A5.25 5.25 0 1 0 4.4 4.2"/><path d="M4.6 1.9v2.5H2.1"/><path d="M8 5.25V8.1l1.9 1.3"/>',
  tag: '<path d="M2.5 3.6v3.55c0 .35.14.68.38.92l5.5 5.5c.5.5 1.32.5 1.82 0l3.12-3.12c.5-.5.5-1.32 0-1.82l-5.5-5.5a1.3 1.3 0 0 0-.92-.38H3.75c-.69 0-1.25.56-1.25 1.25z"/><circle cx="5.6" cy="5.85" r=".95"/>',
  cloud: '<path d="M4.6 12.75h6.65a2.75 2.75 0 0 0 .5-5.45 4 4 0 0 0-7.73-.95A3.2 3.2 0 0 0 4.6 12.75z"/>',
  list: '<path d="M5.75 4.25h7.5M5.75 8h7.5M5.75 11.75h7.5"/><path d="M2.75 4.25h.01M2.75 8h.01M2.75 11.75h.01" stroke-width="1.6"/>',
  tree: '<path d="M2.75 3.25h6.5"/><path d="M4.5 3.25v8.5c0 .41.34.75.75.75h1M4.5 7.75h1.75"/><path d="M8.5 7.75h4.75M8.5 12.5h4.75"/>',
  trash: '<path d="M2.75 4.5h10.5M6.25 4.5V3.1c0-.33.27-.6.6-.6h2.3c.33 0 .6.27.6.6v1.4M4.1 4.5l.62 8.35c.05.66.6 1.15 1.25 1.15h4.06c.65 0 1.2-.5 1.25-1.15l.62-8.35M6.6 7v4.25M9.4 7v4.25"/>',
  merge: '<circle cx="4.5" cy="3.75" r="1.5"/><circle cx="4.5" cy="12.25" r="1.5"/><circle cx="11.5" cy="9" r="1.5"/><path d="M4.5 5.25v5.5M4.5 5.25c0 2.4 2.1 3.75 5.5 3.75"/>',
  worktree: '<path d="M1.75 5.75V3.6c0-.47.38-.85.85-.85h2.5l1.15 1.15h3.9"/><path d="M3.75 6.6c0-.47.38-.85.85-.85h2.5l1.15 1.15h4.3c.47 0 .85.38.85.85v4.9c0 .47-.38.85-.85.85H4.6a.85.85 0 0 1-.85-.85z"/>',
  copy: '<rect x="5.5" y="5.5" width="7.75" height="7.75" rx="1.5"/><path d="M10.5 5.5V4.25c0-.83-.67-1.5-1.5-1.5H4.25c-.83 0-1.5.67-1.5 1.5V9c0 .83.67 1.5 1.5 1.5H5.5"/>',
  external: '<path d="M9.25 2.75h4v4M13.25 2.75 7.5 8.5"/><path d="M11.75 9.5v2.5c0 .69-.56 1.25-1.25 1.25H4c-.69 0-1.25-.56-1.25-1.25V5.5c0-.69.56-1.25 1.25-1.25h2.5"/>',
  pencil: '<path d="M10.4 3.1a1.5 1.5 0 0 1 2.12 0l.38.38a1.5 1.5 0 0 1 0 2.12L6.25 12.25 3 13l.75-3.25z"/><path d="m9.25 4.25 2.5 2.5"/>',
  sync: '<path d="M13 6.5A5.1 5.1 0 0 0 3.6 5"/><path d="M3.25 2.75V5.2h2.45"/><path d="M3 9.5a5.1 5.1 0 0 0 9.4 1.5"/><path d="M12.75 13.25V10.8H10.3"/>',
  lockSmall: '<rect x="4.5" y="7.25" width="7" height="5.5" rx="1.1"/><path d="M6.1 7.25V5.9a1.9 1.9 0 0 1 3.8 0v1.35"/>',
  dot: '<circle cx="8" cy="8" r="2.25" fill="currentColor" stroke="none"/>',
  filter: '<path d="M2.75 4h10.5M4.75 8h6.5M6.75 12h2.5"/>',
  conflict: '<path d="M8 2.5 14 13H2z"/><path d="M8 6.5v3M8 11.25v.01"/>',
  plusCircle: '<circle cx="8" cy="8" r="5.5"/><path d="M8 5.5v5M5.5 8h5"/>',
  switchTo: '<path d="M2.75 8h7.5M7.25 4.75 10.5 8l-3.25 3.25"/><path d="M13.25 3.25v9.5"/>',
  sidebar: '<rect x="2" y="2.75" width="12" height="10.5" rx="1.5"/><path d="M6 2.75v10.5"/>',
};

export const iconSvg = (name) =>
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P[name] || P.file}</svg>`;

export function icon(name, cls = "") {
  const s = document.createElement("span");
  s.className = "ic" + (cls ? " " + cls : "");
  s.innerHTML = iconSvg(name);
  return s;
}

const CODE = new Set(
  "rs js jsx ts tsx mjs cjs py go c h cpp hpp cc cxx hh m mm java kt swift rb lua php sql html htm xml css scss less vue svelte zig ex exs erl hs ml scala dart cs fs r jl".split(" "),
);
const CONFIG = new Set("json jsonc json5 toml yaml yml plist ini conf cfg env properties editorconfig".split(" "));
const DOCS = new Set("md markdown mdx txt rst adoc org".split(" "));
const IMAGES = new Set("png jpg jpeg gif webp ico icns bmp tif tiff avif heic svg".split(" "));
const FONTS = new Set("ttf otf woff woff2".split(" "));
const SHELL = new Set("sh bash zsh fish ps1".split(" "));

export function fileIcon(name) {
  const lower = name.toLowerCase();
  const dot = lower.lastIndexOf(".");
  const ext = dot > 0 ? lower.slice(dot + 1) : "";
  if (IMAGES.has(ext)) return "image";
  if (FONTS.has(ext)) return "binary";
  if (ext === "lock" || lower === "package-lock.json" || lower === "pnpm-lock.yaml" || lower === "yarn.lock") return "lock";
  if (lower.startsWith(".git")) return "git";
  if (SHELL.has(ext) || /^\.(zshrc|bashrc|bash_profile|zprofile|profile|zshenv)$/.test(lower)) return "terminal";
  if (lower === ".env" || lower.startsWith(".env.") || CONFIG.has(ext)) return "braces";
  if (DOCS.has(ext) || /^(license|licence|changelog|authors|notice)$/.test(lower)) return "doc";
  if (CODE.has(ext) || lower === "dockerfile" || lower === "makefile" || lower === "gemfile") return "code";
  return "file";
}
