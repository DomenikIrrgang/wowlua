#!/usr/bin/env node
// Checks the wowlua examples in docs/ by compiling them.
//   node scripts/check-doc-examples.mjs [docs/guide/testing.md ...]
//   WOWLUA=target/release/wowlua node scripts/check-doc-examples.mjs
//
// Each ```wowlua block is checked with `wowlua check` in a fresh camelot
// package. The words after the language name say how:
//
//   ```wowlua title=src/bags.wowlua   a file of the page's package: every titled
//                                     block of a page goes into one package, so
//                                     its files can import each other; a later
//                                     block with the same title replaces the file
//   ```json title=wowlua.json         the page package's manifest (any titled json
//                                     block is a file of it: locales/enUS.json)
//   ```wowlua fails=E2001             must report E2001 (and may report others)
//   ```wowlua fragment                not checked: part of a file, or pseudo-code
//
// An untitled block is checked on its own as src/main.wowlua, or as
// tests/main.test.wowlua when it imports wowlua:test or wowlua:mock.
import { execFileSync, spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const wowlua = process.env.WOWLUA ?? "wowlua";

function markdownFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return markdownFiles(full);
    return name.endsWith(".md") && name !== "emulator-coverage.md" ? [full] : [];
  });
}

function blocks(md) {
  const out = [];
  const lines = md.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const open = /^(\s*)(`{3,})(\w+)?(.*)$/.exec(lines[i]);
    if (!open) continue;
    const [, indent, fence, lang = "", rest] = open;
    const body = [];
    let j = i + 1;
    for (; j < lines.length && !lines[j].startsWith(indent + fence); j++) body.push(lines[j].slice(indent.length));
    const attrs = Object.fromEntries(
      rest.trim().split(/\s+/).filter(Boolean).map((w) => (w.includes("=") ? w.split(/=(.*)/s).slice(0, 2) : [w, true])),
    );
    out.push({ lang, attrs, text: body.join("\n") + "\n", line: i + 1 });
    i = j;
  }
  return out;
}

const template = mkdtempSync(path.join(tmpdir(), "wowlua-doc-examples-"));
execFileSync(wowlua, ["new", "Example", "--flavors", "camelot"], { cwd: template, stdio: "ignore" });
const pkg = path.join(template, "Example");
rmSync(path.join(pkg, ".git"), { recursive: true, force: true });
rmSync(path.join(pkg, "src"), { recursive: true });
rmSync(path.join(pkg, "tests"), { recursive: true });

function check(files, expect) {
  const dir = mkdtempSync(path.join(tmpdir(), "wowlua-doc-example-"));
  try {
    cpSync(pkg, dir, { recursive: true });
    for (const [file, text] of Object.entries(files)) {
      mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
      writeFileSync(path.join(dir, file), text);
    }
    const r = spawnSync(wowlua, ["check", "--offline", "--color", "never", "--message-format", "short"], { cwd: dir, encoding: "utf8" });
    const output = (r.stdout + r.stderr).trim();
    const codes = new Set([...output.matchAll(/error\[([EWP]\d{4})\]/g)].map((m) => m[1]));
    if (expect) return codes.has(expect) ? null : `expected ${expect}, got: ${output || "no errors"}`;
    return r.status === 0 ? null : output;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const targets = process.argv.slice(2).length ? process.argv.slice(2).map((f) => path.resolve(f)) : markdownFiles(path.join(root, "docs"));
let failed = 0;
let checked = 0;
for (const file of targets) {
  const rel = path.relative(root, file);
  const page = {};
  const pageLines = [];
  for (const b of blocks(readFileSync(file, "utf8"))) {
    if (b.attrs.fragment) continue;
    if (typeof b.attrs.title === "string" && ["wowlua", "lua", "json"].includes(b.lang)) {
      page[b.attrs.title] = b.text;
      pageLines.push(b.line);
      continue;
    }
    if (b.lang !== "wowlua") continue;
    checked++;
    const file = /from\s+"wowlua:(test|mock)"/.test(b.text) ? "tests/main.test.wowlua" : "src/main.wowlua";
    const problem = check({ [file]: b.text }, typeof b.attrs.fails === "string" ? b.attrs.fails : null);
    if (problem) {
      failed++;
      console.log(`✗ ${rel}:${b.line}\n${problem.replace(/^/gm, "    ")}\n`);
    }
  }
  if (Object.keys(page).length) {
    checked++;
    const problem = check(page, null);
    if (problem) {
      failed++;
      console.log(`✗ ${rel}: the page's titled files (blocks at lines ${pageLines.join(", ")})\n${problem.replace(/^/gm, "    ")}\n`);
    }
  }
}
rmSync(template, { recursive: true, force: true });
console.log(`${checked - failed} of ${checked} examples compile`);
process.exit(failed ? 1 : 0);
