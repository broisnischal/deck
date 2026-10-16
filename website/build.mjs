// Builds the website into public/: every page in src/pages is wrapped in
// src/layout.html (one header, one footer), and src/assets is copied
// across. No dependencies; `node build.mjs`.
//
// A page starts with its settings as a JSON comment:
//   <!-- {"title": "Pricing", "description": "…", "path": "/pricing/"} -->

import { cpSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "src");
const out = join(here, "public");
const SITE = "https://deck.stroke.click";

const layout = readFileSync(join(src, "layout.html"), "utf8");

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(join(src, "assets"), out, { recursive: true });

const pages = [];
for (const file of readdirSync(join(src, "pages")).sort()) {
  if (!file.endsWith(".html")) continue;
  const text = readFileSync(join(src, "pages", file), "utf8");
  const head = text.match(/^<!--\s*(\{[\s\S]*?\})\s*-->\n?/);
  if (!head) throw new Error(`${file}: no settings comment at the top`);
  const meta = JSON.parse(head[1]);
  const body = text.slice(head[0].length);
  const path = meta.path;
  // The nav marks where you are.
  let html = layout
    .replaceAll("{{title}}", meta.title === "Deck" ? "Deck: a better way to work with coding agents" : `${meta.title} · Deck`)
    .replaceAll("{{description}}", meta.description)
    .replaceAll("{{url}}", SITE + (path === "/404" ? "/" : path))
    .replaceAll("{{bodyclass}}", meta.class ?? "")
    .replace("{{body}}", body);
  html = html.replace(new RegExp(`href="${path.replace(/[/]/g, "\\/")}"(?= data-nav)`, "g"), `href="${path}" aria-current="page"`);
  const dest = path === "/" ? join(out, "index.html") : path === "/404" ? join(out, "404.html") : join(out, path, "index.html");
  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, html);
  if (path !== "/404") pages.push(path);
}

writeFileSync(
  join(out, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${pages
    .filter((p) => p !== "/account/")
    .map((p) => `  <url><loc>${SITE}${p}</loc></url>`)
    .join("\n")}\n</urlset>\n`,
);
writeFileSync(join(out, "robots.txt"), `User-agent: *\nDisallow: /account/\nSitemap: ${SITE}/sitemap.xml\n`);

console.log(`built ${pages.length + 1} pages into public/`);
