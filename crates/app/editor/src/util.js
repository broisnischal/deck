// Small DOM and path helpers shared by every module.

const PROPS = new Set(["value", "checked", "disabled", "tabIndex", "spellcheck", "placeholder", "title", "rows", "innerHTML"]);

export function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const k in props) {
      const v = props[k];
      if (v == null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "text") el.textContent = v;
      else if (k === "style") typeof v === "string" ? (el.style.cssText = v) : Object.assign(el.style, v);
      else if (k === "dataset") Object.assign(el.dataset, v);
      else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2).toLowerCase(), v);
      else if (PROPS.has(k)) el[k] = v;
      else el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  append(el, kids);
  return el;
}

export function append(el, kids) {
  for (const k of kids) {
    if (k == null || k === false) continue;
    if (Array.isArray(k)) append(el, k);
    else el.append(k instanceof Node ? k : String(k));
  }
  return el;
}

export function clear(el) {
  while (el.firstChild) el.firstChild.remove();
  return el;
}

export const basename = (p) => p.slice(p.lastIndexOf("/") + 1);
export const dirname = (p) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "");
export const join = (dir, name) => (dir ? dir + "/" + name : name);
export function ancestors(path) {
  const out = [];
  let d = dirname(path);
  while (d) {
    out.unshift(d);
    d = dirname(d);
  }
  return out;
}

export function debounce(fn, ms) {
  let t = 0;
  const d = (...a) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
  d.cancel = () => clearTimeout(t);
  return d;
}

export function formatSize(n) {
  if (n < 1024) return `${n} bytes`;
  const u = ["KB", "MB", "GB"];
  let i = -1;
  do {
    n /= 1024;
    i++;
  } while (n >= 1024 && i < u.length - 1);
  return `${n.toFixed(n < 10 ? 1 : 0)} ${u[i]}`;
}

export const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

// Keep a tiny bit of UI state across reloads. Storage can be unavailable in
// some WebView configurations, so every access is guarded.
export const prefs = {
  get(key, fallback) {
    try {
      const v = localStorage.getItem("deck-editor:" + key);
      return v == null ? fallback : JSON.parse(v);
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem("deck-editor:" + key, JSON.stringify(value));
    } catch {}
  },
};

// Hex color helpers used to derive translucent tints from the theme.
export function parseHex(hex) {
  let s = String(hex || "").trim().replace(/^#/, "");
  if (s.length === 3) s = s.replace(/./g, "$&$&");
  const n = parseInt(s.slice(0, 6), 16);
  if (Number.isNaN(n)) return [128, 128, 128];
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
export const rgba = (hex, a) => `rgba(${parseHex(hex).join(",")},${a})`;
export function mix(a, b, t) {
  const x = parseHex(a), y = parseHex(b);
  return "#" + x.map((v, i) => Math.round(v + (y[i] - v) * t).toString(16).padStart(2, "0")).join("");
}
export function luminance(hex) {
  const [r, g, b] = parseHex(hex).map((v) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
export function saturation(hex) {
  const [r, g, b] = parseHex(hex).map((v) => v / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  if (max === min) return 0;
  const l = (max + min) / 2;
  return (max - min) / (l > 0.5 ? 2 - max - min : max + min);
}

export function hue(hex) {
  const [r, g, b] = parseHex(hex).map((v) => v / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  if (!d) return 0;
  let hh = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (hh * 60 + 360) % 360;
}

// "now", "5m", "3h", "2d", "3w", "4mo", "2y".
export function relTime(unix, now = Date.now() / 1000) {
  const s = Math.max(0, now - unix);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d`;
  if (s < 86400 * 30) return `${Math.floor(s / (86400 * 7))}w`;
  if (s < 86400 * 365) return `${Math.max(1, Math.floor(s / (86400 * 30.4)))}mo`;
  return `${Math.floor(s / (86400 * 365))}y`;
}

// "3 hours ago", for sentences.
export function relTimeLong(unix, now = Date.now() / 1000) {
  const s = Math.max(0, now - unix);
  const unit = (n, w) => `${n} ${w}${n === 1 ? "" : "s"} ago`;
  if (s < 60) return "just now";
  if (s < 3600) return unit(Math.floor(s / 60), "minute");
  if (s < 86400) return unit(Math.floor(s / 3600), "hour");
  if (s < 86400 * 30) return unit(Math.floor(s / 86400), "day");
  if (s < 86400 * 365) return unit(Math.max(1, Math.floor(s / (86400 * 30.4))), "month");
  return unit(Math.floor(s / (86400 * 365)), "year");
}

export function fullDate(unix) {
  try {
    return new Date(unix * 1000).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  } catch {
    return new Date(unix * 1000).toString();
  }
}

export function initials(name) {
  const words = String(name || "?").replace(/\[bot\]$/, "").split(/[\s._-]+/).filter(Boolean);
  if (!words.length) return "?";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

export function hashStr(s) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

// Text for a start-truncated (direction: rtl) span: the marks keep the
// neutral characters of a path in left-to-right order.
export const ltr = (s) => "‎" + s + "‎";

export const plural = (n, word, many = word + "s") => `${n} ${n === 1 ? word : many}`;

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {}
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.cssText = "position:fixed;left:-9999px;top:0;opacity:0";
  document.body.append(ta);
  ta.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {}
  ta.remove();
  return ok;
}

const BAD_REF = /(^[-/.])|(\.\.)|(@\{)|([\s~^:?*[\\\x00-\x1f\x7f])|(\/\/)|([/.]$)|(\.lock$)|(^@$)/;
export const validateBranch = (name) => (BAD_REF.test(name) ? "That isn't a valid branch name." : null);

// "/Users/me/code/x" -> "~/code/x".
export const tildify = (p) => String(p || "").replace(/^\/Users\/[^/]+(?=\/|$)/, "~");

export function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}
