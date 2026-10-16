// Ref decorations ("HEAD -> main", "origin/main", "tag: v1.0") as pills, and
// author avatars. Shared by the history list and the commit view.

import { S } from "./store.js";
import { escapeHtml, hashStr } from "./util.js";
import { iconSvg } from "./icons.js";
import { COLORS } from "./graph.js";

const remotes = () => S.remotes || new Set(["origin", "upstream"]);

export function parseRefs(refs) {
  let head = null;
  const local = [], remote = [], tags = [];
  const known = remotes();
  for (const raw of refs || []) {
    const r = String(raw).trim();
    if (!r) continue;
    if (r === "HEAD") head = "";
    else if (r.startsWith("HEAD -> ")) {
      head = r.slice(8);
      local.push(head);
    } else if (r.startsWith("tag: ")) tags.push(r.slice(5));
    else if (/(^|\/)HEAD$/.test(r)) continue;
    else if (r.includes("/") && known.has(r.split("/")[0])) remote.push(r);
    else local.push(r);
  }
  // "main" and "origin/main" on the same commit become one pill.
  const pills = [];
  const used = new Set();
  for (const name of local) {
    const twin = remote.find((x) => x.slice(x.indexOf("/") + 1) === name);
    if (twin) used.add(twin);
    pills.push({ kind: "local", name, head: name === head, synced: !!twin, title: twin ? `${name}, in sync with ${twin}` : name });
  }
  if (head === "") pills.unshift({ kind: "head", name: "HEAD", head: true, title: "Detached HEAD" });
  for (const r of remote) if (!used.has(r)) pills.push({ kind: "remote", name: r, title: r });
  for (const t of tags) pills.push({ kind: "tag", name: t, title: `Tag ${t}` });
  pills.sort((a, b) => (b.head ? 1 : 0) - (a.head ? 1 : 0));
  return pills;
}

const CLOUD = iconSvg("cloud");
const TAG = iconSvg("tag");

export function pillsHtml(refs, max = 2) {
  const pills = parseRefs(refs);
  if (!pills.length) return "";
  const shown = max ? pills.slice(0, max) : pills;
  let s = "";
  for (const p of shown) {
    s += `<span class="ref ref-${p.kind}${p.head ? " is-head" : ""}" title="${escapeHtml(p.title)}">`;
    if (p.kind === "tag") s += `<span class="ic">${TAG}</span>`;
    s += `<span class="ref-name">${escapeHtml(p.name)}</span>`;
    if (p.synced) s += `<span class="ic ref-cloud">${CLOUD}</span>`;
    s += "</span>";
  }
  if (pills.length > shown.length) s += `<span class="ref ref-more" title="${escapeHtml(pills.slice(shown.length).map((p) => p.name).join("\n"))}">+${pills.length - shown.length}</span>`;
  return s;
}

export const avatarClass = (email, name) => "av av" + (hashStr(String(email || name || "").toLowerCase()) % COLORS);
