// Theme: Deck's palette becomes CSS variables; the CodeMirror theme and the
// highlight style only reference those variables, so a theme switch is just
// a variable update (plus the dark/light flag for CodeMirror's base themes).

import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { jsonLanguage } from "@codemirror/lang-json";
import { yamlLanguage } from "@codemirror/lang-yaml";
import { rgba, mix, luminance, saturation, prefs } from "./util.js";

const KEYS = ["bg", "panel", "raise", "line", "text", "text2", "dim", "accent", "red", "green", "yellow", "blue", "purple", "cyan", "selection"];

export const DEFAULT_THEME = {
  dark: true,
  bg: "#0a0a0a",
  panel: "#050505",
  raise: "#141414",
  line: "#1c1c1c",
  text: "#f5f5f5",
  text2: "#a3a3a3",
  dim: "#6b6b6b",
  accent: "#ffffff",
  red: "#ff7b7b",
  green: "#6ddf9a",
  yellow: "#ffc069",
  blue: "#8b9dff",
  purple: "#c4a6ff",
  cyan: "#8fd3f4",
  selection: "#3a3a3a",
};

export const DEFAULT_LIGHT = {
  dark: false,
  bg: "#ffffff",
  panel: "#f5f5f6",
  raise: "#fbfbfb",
  line: "#e4e4e7",
  text: "#1d1d1f",
  text2: "#55555a",
  dim: "#86868b",
  accent: "#000000",
  red: "#c4282e",
  green: "#1a7f37",
  yellow: "#9a6700",
  blue: "#0550ae",
  purple: "#8250df",
  cyan: "#1b7c83",
  selection: "#cfcfd4",
};

// Best guess before Deck answers: the last theme it sent, else the system
// appearance. Avoids a dark flash in light mode (and vice versa).
export function initialTheme() {
  const last = prefs.get("theme", null);
  if (last && typeof last === "object" && last.bg) return last;
  const light = typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: light)").matches;
  return light ? DEFAULT_LIGHT : DEFAULT_THEME;
}

export let currentTheme = DEFAULT_THEME;

// Black or white, whichever contrasts more with `bg`.
function readableOn(bg) {
  const l = luminance(bg);
  return (l + 0.05) / 0.05 >= 1.05 / (l + 0.05) ? "#000000" : "#ffffff";
}

const colorful = (hex) => saturation(hex) > 0.25 && luminance(hex) > 0.04 && luminance(hex) < 0.85;

export function applyTheme(theme, remember = false) {
  const base = theme && theme.dark === false ? DEFAULT_LIGHT : DEFAULT_THEME;
  const th = { ...base, ...(theme || {}) };
  // Older hosts send no purple/cyan: derive them from the colours they do send.
  if (!theme?.purple) th.purple = mix(th.blue, th.red, 0.42);
  if (!theme?.cyan) th.cyan = mix(th.blue, th.green, 0.5);
  currentTheme = th;
  if (remember) prefs.set("theme", th);
  const dark = !!th.dark;
  const root = document.documentElement;
  const set = (k, v) => root.style.setProperty("--" + k, v);
  for (const k of KEYS) set(k, th[k]);

  // Deck's accent is often pure white or black, so syntax colours never use
  // it: they come from the ANSI palette (purple and cyan included).
  const tinted = colorful(th.accent);
  set("syn-kw", th.purple);
  set("syn-str", th.green);
  set("syn-num", th.yellow);
  set("syn-type", th.cyan);
  set("syn-fn", th.blue);
  set("syn-prop", th.text);
  set("syn-var", th.text);
  set("syn-com", mix(th.dim, th.bg, dark ? 0.05 : 0));
  set("syn-op", th.text2);
  set("syn-tag", th.red);
  set("syn-attr", th.yellow);
  set("syn-esc", th.cyan);
  set("syn-re", th.red);
  set("syn-head", th.blue);
  set("syn-link", th.cyan);
  set("syn-meta", mix(th.yellow, th.text2, 0.35));
  set("caret", tinted ? th.accent : th.text);
  // Graph lanes and avatars.
  const lanes = [th.blue, th.purple, th.green, th.yellow, th.cyan, th.red, mix(th.blue, th.cyan, 0.5), mix(th.red, th.yellow, 0.5)];
  lanes.forEach((c, i) => {
    set("lane" + i, c);
    set("lane" + i + "-soft", rgba(c, dark ? 0.2 : 0.16));
  });
  set("on-accent", readableOn(th.accent));
  set("on-red", readableOn(th.red));
  set("seg-active", dark ? rgba(th.text, 0.11) : th.bg);
  set("accent-soft", rgba(tinted ? th.accent : th.text, dark ? 0.12 : 0.1));

  set("accent-solid", tinted ? th.accent : th.text);
  set("hover", rgba(th.text, dark ? 0.05 : 0.045));
  set("hover-strong", rgba(th.text, dark ? 0.08 : 0.07));
  set("active-row", rgba(th.text, dark ? 0.085 : 0.075));
  set("active-row-focus", tinted ? rgba(th.accent, dark ? 0.2 : 0.16) : rgba(th.text, dark ? 0.12 : 0.1));
  set("ring", rgba(tinted ? th.accent : th.text, dark ? 0.45 : 0.4));
  set("guide", rgba(th.text, dark ? 0.06 : 0.08));
  set("gutter", mix(th.bg, th.dim, dark ? 0.8 : 0.75));
  set("field", dark ? rgba(th.text, 0.035) : th.bg);
  set("field-edge", rgba(th.text, dark ? 0.1 : 0.14));
  set("field-edge-focus", rgba(tinted ? th.accent : th.text, dark ? 0.4 : 0.45));
  set("popover", dark ? mix(th.raise, th.text, 0.02) : th.bg);
  set("popover-edge", rgba(th.text, dark ? 0.1 : 0.1));
  set(
    "shadow",
    dark
      ? "0 12px 32px rgba(0,0,0,.55), 0 2px 6px rgba(0,0,0,.35)"
      : "0 12px 32px rgba(0,0,0,.12), 0 2px 6px rgba(0,0,0,.06)",
  );
  set("scrim", dark ? "rgba(0,0,0,.45)" : "rgba(0,0,0,.18)");
  set("thumb", rgba(th.text, dark ? 0.13 : 0.17));
  set("thumb-hover", rgba(th.text, dark ? 0.24 : 0.3));

  set("active-line", rgba(th.text, dark ? 0.03 : 0.035));
  set("gutter-active", th.text2);
  set("selection-blur", mix(th.selection, th.bg, 0.35));
  set("sel-match", rgba(th.text, dark ? 0.08 : 0.07));
  set("bracket", rgba(th.text, dark ? 0.16 : 0.12));
  set("search-match", rgba(th.yellow, dark ? 0.2 : 0.28));
  set("search-edge", rgba(th.yellow, dark ? 0.45 : 0.6));
  set("search-cur", rgba(th.yellow, dark ? 0.45 : 0.5));

  set("green-line", rgba(th.green, dark ? 0.075 : 0.09));
  set("green-text", rgba(th.green, dark ? 0.16 : 0.17));
  set("red-line", rgba(th.red, dark ? 0.075 : 0.07));
  set("red-text", rgba(th.red, dark ? 0.2 : 0.17));
  set("yellow-soft", rgba(th.yellow, dark ? 0.14 : 0.14));
  set("red-soft", rgba(th.red, dark ? 0.14 : 0.1));
  set("green-soft", rgba(th.green, dark ? 0.14 : 0.12));

  root.dataset.theme = dark ? "dark" : "light";
  root.style.colorScheme = dark ? "dark" : "light";
}

export function applyFont(font, uiSize) {
  const root = document.documentElement;
  if (font) {
    const family = String(font.family || "").replace(/["\\;{}]/g, "").trim();
    root.style.setProperty(
      "--mono",
      `${family ? `"${family}", ` : ""}"JetBrains Mono", "SF Mono", ui-monospace, Menlo, Monaco, monospace`,
    );
    if (font.size) root.style.setProperty("--mono-size", `${Math.max(8, Math.min(40, +font.size))}px`);
  }
  if (uiSize) root.style.setProperty("--ui-size", `${Math.max(10, Math.min(20, +uiSize))}px`);
}

export const editorTheme = EditorView.theme({
  "&": { color: "var(--text)", backgroundColor: "var(--bg)", height: "100%", fontSize: "var(--mono-size)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.5", overflow: "auto" },
  ".cm-content": { caretColor: "var(--caret)", padding: "8px 0" },
  ".cm-line": { padding: "0 24px 0 4px" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--caret)", borderLeftWidth: "2px" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-content ::selection": {
    backgroundColor: "var(--selection) !important",
  },
  ".cm-selectionBackground": { backgroundColor: "var(--selection-blur)" },
  ".cm-activeLine": { backgroundColor: "var(--active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--gutter-active)" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--gutter)", border: "none", paddingLeft: "6px" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 10px", minWidth: "32px", fontVariantNumeric: "tabular-nums" },
  ".cm-foldGutter .cm-gutterElement": { padding: "0 4px 0 2px", color: "var(--dim)", opacity: "0", transition: "opacity .12s" },
  ".cm-gutters:hover .cm-foldGutter .cm-gutterElement": { opacity: "1" },
  ".cm-foldPlaceholder": {
    backgroundColor: "var(--hover-strong)",
    border: "none",
    color: "var(--text2)",
    borderRadius: "4px",
    padding: "0 5px",
    margin: "0 3px",
  },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {
    backgroundColor: "var(--bracket)",
    color: "inherit",
    outline: "none",
    borderRadius: "2px",
  },
  ".cm-nonmatchingBracket, &.cm-focused .cm-nonmatchingBracket": { color: "var(--red)", backgroundColor: "transparent" },
  ".cm-selectionMatch": { backgroundColor: "var(--sel-match)", borderRadius: "2px" },
  ".cm-searchMatch": { backgroundColor: "var(--search-match)", outline: "1px solid var(--search-edge)", borderRadius: "2px" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--search-cur)" },
  ".cm-specialChar": { color: "var(--red)" },
  ".cm-tooltip": {
    backgroundColor: "var(--popover)",
    color: "var(--text)",
    border: "1px solid var(--popover-edge)",
    borderRadius: "6px",
    boxShadow: "var(--shadow)",
  },
  ".cm-panels": { backgroundColor: "var(--panel)", color: "var(--text)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--line)" },
  ".cm-panels.cm-panels-bottom": { borderTop: "1px solid var(--line)" },

  // Diffs (@codemirror/merge). Background washes for whole lines, stronger
  // fills for the exact characters that changed.
  "&.cm-merge-b .cm-changedLine, .cm-inlineChangedLine": { backgroundColor: "var(--green-line)" },
  "&.cm-merge-a .cm-changedLine, .cm-deletedChunk": { backgroundColor: "var(--red-line)" },
  "&.cm-merge-b .cm-changedText": { background: "var(--green-text)", borderRadius: "2px" },
  "&.cm-merge-a .cm-changedText, .cm-deletedChunk .cm-deletedText": { background: "var(--red-text)", borderRadius: "2px" },
  "&.cm-merge-b .cm-deletedText": { background: "var(--red-text)" },
  ".cm-deletedChunk": { paddingLeft: "4px", position: "relative" },
  ".cm-deletedChunk .cm-deletedLine": { padding: "0 24px 0 0", color: "var(--text2)" },
  ".cm-changeGutter": { width: "3px", padding: "0", marginRight: "2px" },
  "&.cm-merge-b .cm-changedLineGutter": { background: "var(--green)" },
  "&.cm-merge-a .cm-changedLineGutter, .cm-deletedLineGutter": { background: "var(--red)" },
  ".cm-inlineChangedLineGutter": { background: "var(--yellow)" },
  ".cm-collapsedLines": {
    color: "var(--dim)",
    background: "var(--panel)",
    borderTop: "1px solid var(--line)",
    borderBottom: "1px solid var(--line)",
    fontFamily: "var(--ui-font)",
    fontSize: "11px",
    padding: "3px 12px",
  },
  ".cm-collapsedLines:hover": { color: "var(--text)" },
  ".cm-mergeSpacer": { background: "repeating-linear-gradient(-45deg, transparent 0 5px, var(--guide) 5px 6px)" },
});

const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.controlKeyword, t.moduleKeyword, t.operatorKeyword, t.definitionKeyword, t.modifier, t.self], color: "var(--syn-kw)" },
  { tag: t.special(t.variableName), color: "var(--syn-kw)", fontStyle: "italic" },
  { tag: [t.string, t.special(t.string), t.character, t.attributeValue, t.inserted, t.docString], color: "var(--syn-str)" },
  { tag: [t.escape], color: "var(--syn-esc)" },
  { tag: [t.regexp], color: "var(--syn-re)" },
  { tag: [t.number, t.integer, t.float, t.bool, t.null, t.atom, t.unit, t.constant(t.variableName), t.constant(t.name), t.standard(t.variableName)], color: "var(--syn-num)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--syn-com)", fontStyle: "italic" },
  { tag: [t.typeName, t.className, t.namespace, t.standard(t.typeName), t.definition(t.typeName), t.typeOperator], color: "var(--syn-type)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName)), t.macroName], color: "var(--syn-fn)" },
  // Properties and variables stay in the text colour (JSON/YAML keys below).
  { tag: [t.tagName], color: "var(--syn-tag)" },
  { tag: [t.attributeName], color: "var(--syn-attr)" },
  { tag: [t.labelName], color: "var(--syn-num)" },
  { tag: [t.annotation, t.meta, t.processingInstruction, t.documentMeta], color: "var(--syn-meta)" },
  { tag: [t.operator, t.punctuation, t.separator, t.bracket, t.angleBracket, t.squareBracket, t.paren, t.brace, t.derefOperator], color: "var(--syn-op)" },
  { tag: t.heading, color: "var(--syn-head)", fontWeight: "600" },
  { tag: [t.list, t.quote], color: "var(--syn-op)" },
  { tag: t.quote, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "600" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: [t.link, t.url], color: "var(--syn-link)", textDecoration: "underline", textUnderlineOffset: "2px" },
  { tag: t.monospace, color: "var(--syn-str)" },
  { tag: [t.deleted], color: "var(--red)" },
  { tag: [t.changed], color: "var(--yellow)" },
  { tag: t.invalid, color: "var(--red)", textDecoration: "underline wavy", textDecorationColor: "var(--red)" },
  { tag: t.contentSeparator, color: "var(--syn-com)" },
]);

// JSON and YAML keys read best in a colour of their own.
const keySpec = [{ tag: [t.propertyName, t.definition(t.propertyName)], color: "var(--syn-fn)" }];
const jsonKeys = HighlightStyle.define(keySpec, { scope: jsonLanguage });
const yamlKeys = HighlightStyle.define(keySpec, { scope: yamlLanguage });

export const highlight = [syntaxHighlighting(jsonKeys), syntaxHighlighting(yamlKeys), syntaxHighlighting(highlightStyle)];
