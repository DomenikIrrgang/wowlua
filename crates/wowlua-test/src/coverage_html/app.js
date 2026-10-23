// wowlua's coverage report: the index (totals, where to test next, every
// file) or one file (its totals, the lines that never ran, its functions,
// its source), drawn from the data in #data. The address says which:
// "#/src/a.wowlua" a file, "#/src/a.wowlua:L12" a line in it.
"use strict";

const KINDS = ["lines", "branches", "functions", "conditions"];

// wowlua's mark and name, on top of every view.
const BRAND = '<div class="brand"><a href="#/" title="All files"><svg viewBox="0 0 48 48" aria-hidden="true">' +
  '<path d="M9.5 5H4V43H9.5M38.5 5H44V43H38.5" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="square"/>' +
  '<g class="wl"><path transform="translate(12.14 31.63)" d="M0.36 -11.16H3.33L3.73 -6.48L3.87 -2.72H4.23L4.9 -11.16H8.08L8.69 -2.72H9.05L9.18 -6.48L9.56 -11.16H12.24L11 0.0H7.11L6.52 -8.41H6.16L5.51 0.0H1.6ZM20.12 -15.26V-2.84H23.36V0.0H13.64V-2.84H16.88V-12.42H13.64V-15.26Z"/></g>' +
  '</svg></a><span class="word">wowlua</span><span class="what">coverage</span></div>';

function pct(c, n) { return n === 0 ? 100 : (c * 100) / n; }
function cls(p) { return p >= 90 ? "good" : p >= 60 ? "mid" : "bad"; }
function esc(s) {
  return String(s).replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);
}
function plural(n, one, many) { return n + " " + (n === 1 ? one : many); }

// A file's counts, unpacked from the data's flat arrays.
function unpack(f) {
  if (f.hits) return f;
  f.hits = new Map();
  for (let i = 0; i < f.l.length; i += 2) f.hits.set(f.l[i], f.l[i + 1]);
  f.branches = new Map();
  for (let i = 0; i < f.b.length; i += 3) {
    const line = f.b[i];
    if (!f.branches.has(line)) f.branches.set(line, []);
    f.branches.get(line).push([f.b[i + 1], f.b[i + 2]]);
  }
  f.functions = [];
  for (let i = 0; i < f.f.length; i += 3) f.functions.push({ line: f.f[i], name: f.f[i + 1], calls: f.f[i + 2] });
  f.missed = [];
  for (const [line, h] of f.hits) if (h === 0) f.missed.push(line);
  f.missed.sort((a, b) => a - b);
  return f;
}

// Lines that never ran, as ranges: [[3, 5], [9, 9]].
function ranges(lines) {
  const out = [];
  for (const l of lines) {
    const last = out[out.length - 1];
    if (last && l === last[1] + 1) last[1] = l;
    else out.push([l, l]);
  }
  return out;
}

function totals(t) {
  return KINDS.map((k, i) => [k, t[2 * i], t[2 * i + 1]]);
}

function chips(t) {
  return '<div class="chips">' + totals(t).map(([k, c, n]) => {
    const p = pct(c, n);
    return `<span class="chip">${k} <b class="${cls(p)}">${p.toFixed(1)}%</b> <small>${c}/${n}</small></span>`;
  }).join("") + "</div>";
}

function cell(c, n) {
  const p = pct(c, n);
  return `<td class="${cls(p)}">${p.toFixed(1)}% <small>${c}/${n}</small></td>`;
}

function fileLink(path, line) {
  return "#/" + encodeURI(path) + (line ? ":L" + line : "");
}

// ---------------------------------------------------------------- index --

const state = { sort: "path", desc: false, filter: "", partial: false };
try {
  const saved = JSON.parse(localStorage.getItem("wowlua-coverage") || "{}");
  Object.assign(state, saved);
} catch (e) { /* no storage: the defaults */ }
function save() {
  try { localStorage.setItem("wowlua-coverage", JSON.stringify(state)); } catch (e) { /* no storage */ }
}

// What a column sorts by.
function sortValue(f, key) {
  if (key === "path") return f.p;
  if (key === "missed") return unpack(f).missed.length;
  const i = KINDS.indexOf(key);
  return pct(f.t[2 * i], f.t[2 * i + 1]);
}

function visibleFiles(data) {
  const q = state.filter.toLowerCase();
  let files = data.files.filter((f) => (!q || f.p.toLowerCase().includes(q)) &&
    (!state.partial || totals(f.t).some(([, c, n]) => c < n)));
  files = files.slice().sort((a, b) => {
    const x = sortValue(a, state.sort), y = sortValue(b, state.sort);
    const d = x < y ? -1 : x > y ? 1 : a.p < b.p ? -1 : a.p > b.p ? 1 : 0;
    return state.desc ? -d : d;
  });
  return files;
}

function folderOf(path) { const i = path.lastIndexOf("/"); return i < 0 ? "." : path.slice(0, i); }

function renderIndex(data) {
  const worst = data.files.map(unpack).filter((f) => f.missed.length > 0 || f.functions.some((x) => x.calls === 0))
    .sort((a, b) => b.missed.length - a.missed.length || (a.p < b.p ? -1 : 1)).slice(0, 10);
  let out = BRAND + `<header><div class="row"><h1>${esc(data.title)}</h1>${chips(data.totals)}</div></header><main>`;
  if (worst.length) {
    out += '<section><h2>Where to test next</h2><table><tr><th>File</th><th>Lines not run</th><th>Functions never called</th><th>Lines</th></tr>';
    for (const f of worst) {
      const never = f.functions.filter((x) => x.calls === 0).length;
      out += `<tr><td class="path"><a href="${fileLink(f.p)}">${esc(f.p)}</a></td><td>${f.missed.length}</td><td>${never}</td>${cell(f.t[0], f.t[1])}</tr>`;
    }
    out += "</table></section>";
  }
  out += '<section><h2>Files</h2><div class="controls">' +
    `<input type="search" id="filter" placeholder="Filter files" value="${esc(state.filter)}">` +
    `<label><input type="checkbox" id="partial"${state.partial ? " checked" : ""}> only files not fully covered</label>` +
    '<span class="muted" id="count"></span></div><div id="table"></div></section></main>';
  return out;
}

function renderTable(data) {
  const files = visibleFiles(data);
  const head = [["path", "File"], ["lines", "Lines"], ["branches", "Branches"], ["functions", "Functions"],
    ["conditions", "Conditions"], ["missed", "Not run"]];
  let out = "<table><tr>" + head.map(([k, label]) => {
    const arrow = state.sort === k ? ` <span class="arrow">${state.desc ? "▼" : "▲"}</span>` : "";
    return `<th data-sort="${k}">${label}${arrow}</th>`;
  }).join("") + "</tr>";
  // folders while the list is in path order
  const grouped = state.sort === "path" && !state.desc;
  let folder = null;
  const folderTotals = new Map();
  if (grouped) {
    for (const f of files) {
      const d = folderOf(f.p);
      const t = folderTotals.get(d) || [0, 0, 0, 0, 0, 0, 0, 0, 0];
      for (let i = 0; i < 8; i++) t[i] += f.t[i];
      t[8] += unpack(f).missed.length;
      folderTotals.set(d, t);
    }
  }
  for (const f of files) {
    if (grouped && folderOf(f.p) !== folder) {
      folder = folderOf(f.p);
      const t = folderTotals.get(folder);
      out += `<tr class="folder"><td><strong>${esc(folder)}/</strong></td>` +
        totals(t).map(([, c, n]) => cell(c, n)).join("") + `<td>${t[8]}</td></tr>`;
    }
    const missed = unpack(f).missed.length;
    out += `<tr><td class="path">${grouped ? "&nbsp;&nbsp;" : ""}<a href="${fileLink(f.p)}">${esc(f.p)}</a></td>` +
      totals(f.t).map(([, c, n]) => cell(c, n)).join("") + `<td>${missed || ""}</td></tr>`;
  }
  return { html: out + "</table>", count: files.length };
}

// ----------------------------------------------------------------- file --

// A branch on a line: how often it went each way, in words.
function branchChip(taken, notTaken) {
  if (taken > 0 && notTaken > 0) {
    return `<span class="br both" title="went both ways: ${taken}× one way, ${notTaken}× the other">both ways ${taken} · ${notTaken}</span>`;
  }
  if (taken > 0 || notTaken > 0) {
    return `<span class="br one" title="went only one way (${taken + notTaken}×): the other way never ran">one way only ${taken} · ${notTaken}</span>`;
  }
  return '<span class="br none" title="never reached">not reached</span>';
}

function renderFile(data, f) {
  unpack(f);
  const at = data.files.indexOf(f);
  const prev = data.files[at - 1], next = data.files[at + 1];
  let out = BRAND + '<header><div class="row"><a href="#/">← all files</a>' +
    `<h1>${esc(f.p)}</h1>${chips(f.t)}</div></header><main>`;
  const gaps = ranges(f.missed);
  out += "<section>";
  if (gaps.length) {
    out += `<h3>Not run: ${plural(f.missed.length, "line", "lines")}</h3><p class="gaps">` +
      gaps.map(([a, b]) => `<a href="${fileLink(f.p, a)}">${a === b ? a : a + "–" + b}</a>`).join("") +
      '</p><p><button id="prevGap">↑ previous not run</button> <button id="nextGap">↓ next not run</button> ' +
      '<small>(<kbd>k</kbd> / <kbd>j</kbd>)</small></p>';
  } else {
    out += "<h3>Every line ran</h3>";
  }
  out += "<p>" + (prev ? `<a href="${fileLink(prev.p)}">← ${esc(prev.p)}</a>` : "") +
    (prev && next ? " · " : "") + (next ? `<a href="${fileLink(next.p)}">${esc(next.p)} →</a>` : "") + "</p></section>";
  if (f.functions.length) {
    const never = f.functions.filter((x) => x.calls === 0);
    // never called first, then in file order
    const list = f.functions.slice().sort((a, b) => (b.calls === 0) - (a.calls === 0) || a.line - b.line);
    // folded away while every one was called
    out += `<section><details${never.length ? " open" : ""}><summary><h3>Functions: ${never.length ? never.length + " of " + f.functions.length + " never called" : "all " + f.functions.length + " called"}</h3></summary>` +
      "<table><tr><th>Function</th><th>Line</th><th>Calls</th></tr>" +
      list.map((x) => `<tr><td><a href="${fileLink(f.p, x.line)}">${esc(x.name)}</a></td><td>${x.line}</td>` +
        `<td class="${x.calls === 0 ? "bad" : ""}">${x.calls === 0 ? "never" : x.calls}</td></tr>`).join("") +
      "</table></details></section>";
  }
  const fnAt = new Map();
  for (const x of f.functions) {
    if (!fnAt.has(x.line)) fnAt.set(x.line, []);
    fnAt.get(x.line).push(x);
  }
  out += '<section><h3>Source</h3><div class="src" id="src">';
  const text = f.s.split("\n");
  if (text.length && text[text.length - 1] === "") text.pop();
  text.forEach((line, i) => {
    const ln = i + 1;
    const h = f.hits.get(ln);
    const brs = f.branches.get(ln) || [];
    const oneWay = brs.some(([a, b]) => a === 0 || b === 0);
    const c = h === undefined ? "" : h === 0 ? "miss" : oneWay ? "part" : "hit";
    const marks = brs.map(([a, b]) => branchChip(a, b)).join("") +
      (fnAt.get(ln) || []).map((x) => `<span class="fn${x.calls === 0 ? " never" : ""}">${x.calls === 0 ? "never called" : "called " + x.calls + "×"}</span>`).join("");
    out += `<div id="L${ln}" class="${c}"><span class="n"><a href="${fileLink(f.p, ln)}">${ln}</a></span>` +
      `<span class="h">${h === undefined ? "" : h}</span><span class="c">${esc(line)}${marks}</span></div>`;
  });
  return out + "</div></section></main>";
}

// The first line not run after (or before) the one at the top of the view.
function gapFrom(f, down) {
  const starts = ranges(f.missed).map(([a]) => a);
  const top = window.scrollY + 120;
  const lineAt = (ln) => { const el = document.getElementById("L" + ln); return el ? el.getBoundingClientRect().top + window.scrollY : 0; };
  if (down) return starts.find((ln) => lineAt(ln) > top + 4);
  return starts.slice().reverse().find((ln) => lineAt(ln) < top - 4);
}

// --------------------------------------------------------------- routing --

function route(data) {
  const hash = decodeURI(location.hash.replace(/^#\/?/, ""));
  const m = hash.match(/^(.*?)(?::L(\d+))?$/);
  const f = m && m[1] ? data.files.find((x) => x.p === m[1]) : null;
  const root = document.getElementById("app");
  if (!f) {
    root.innerHTML = renderIndex(data);
    document.title = data.title + " coverage";
    const draw = () => {
      const t = renderTable(data);
      document.getElementById("table").innerHTML = t.html;
      document.getElementById("count").textContent = t.count + " of " + data.files.length + " files";
      for (const th of document.querySelectorAll("th[data-sort]")) {
        th.onclick = () => {
          const k = th.dataset.sort;
          state.desc = state.sort === k ? !state.desc : k !== "path";
          state.sort = k;
          save();
          draw();
        };
      }
    };
    document.getElementById("filter").oninput = (e) => { state.filter = e.target.value; save(); draw(); };
    document.getElementById("partial").onchange = (e) => { state.partial = e.target.checked; save(); draw(); };
    draw();
    window.onkeydown = null;
    return;
  }
  root.innerHTML = renderFile(data, f);
  document.title = f.p + " · " + data.title + " coverage";
  const jump = (down) => {
    const ln = gapFrom(f, down);
    if (ln) location.hash = fileLink(f.p, ln);
  };
  const prev = document.getElementById("prevGap"), next = document.getElementById("nextGap");
  if (prev) prev.onclick = () => jump(false);
  if (next) next.onclick = () => jump(true);
  window.onkeydown = (e) => {
    if (e.target.tagName === "INPUT") return;
    if (e.key === "j") jump(true);
    if (e.key === "k") jump(false);
  };
  const line = m[2] && document.getElementById("L" + m[2]);
  if (line) {
    line.classList.add("focus");
    line.scrollIntoView({ block: "center" });
  } else {
    window.scrollTo(0, 0);
  }
}

if (typeof document !== "undefined" && document.getElementById("data")) {
  const data = JSON.parse(document.getElementById("data").textContent);
  window.addEventListener("hashchange", () => route(data));
  route(data);
}
if (typeof module !== "undefined") module.exports = { renderIndex, renderTable, renderFile, ranges, unpack, branchChip };
