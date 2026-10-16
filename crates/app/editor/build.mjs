// Bundles src/ into dist/{index.html, app.js, app.css}. `--watch` rebuilds on change.
import * as esbuild from "esbuild";
import { copyFile, mkdir, readdir, rm, stat } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, "dist");
const watch = process.argv.includes("--watch");

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });
await copyFile(join(root, "src/index.html"), join(dist, "index.html"));

const options = {
  absWorkingDir: root,
  entryPoints: { app: "src/main.js" },
  outdir: "dist",
  bundle: true,
  minify: true,
  splitting: false,
  format: "iife",
  target: ["safari15", "chrome100"],
  legalComments: "none",
  charset: "utf8",
  logLevel: "info",
};

if (watch) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
} else {
  await esbuild.build(options);
  let total = 0;
  for (const f of await readdir(dist)) {
    const { size } = await stat(join(dist, f));
    total += size;
    console.log(`${f.padEnd(12)} ${(size / 1024).toFixed(1).padStart(8)} KB`);
  }
  console.log(`${"total".padEnd(12)} ${(total / 1024).toFixed(1).padStart(8)} KB`);
}
