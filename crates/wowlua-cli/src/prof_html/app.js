// wowlua's performance report (`wowlua prof report`, spec Tooling §16.5):
// the recordings the dev window exported (format 1, Debugging §7.7.9),
// drawn from the data in #data. The address names the recording and the
// view: "#/2/flame" is the third recording's flame graph.
"use strict";

const DATA = JSON.parse(document.getElementById("data").textContent);

const BRAND = '<div class="brand"><svg viewBox="0 0 48 48" aria-hidden="true">' +
  '<path d="M9.5 5H4V43H9.5M38.5 5H44V43H38.5" fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="square"/>' +
  '<g class="wl"><path transform="translate(12.14 31.63)" d="M0.36 -11.16H3.33L3.73 -6.48L3.87 -2.72H4.23L4.9 -11.16H8.08L8.69 -2.72H9.05L9.18 -6.48L9.56 -11.16H12.24L11 0.0H7.11L6.52 -8.41H6.16L5.51 0.0H1.6ZM20.12 -15.26V-2.84H23.36V0.0H13.64V-2.84H16.88V-12.42H13.64V-15.26Z"/></g>' +
  '</svg><span class="word">wowlua</span><span class="what">performance</span>';

const VIEWS = [
  ["overview", "overview"], ["functions", "functions"], ["events", "by event"], ["tree", "tree"],
  ["flame", "flame"], ["timeline", "timeline"], ["slow", "slow"], ["garbage", "garbage"],
];

// ------------------------------------------------------------- helpers --

function esc(s) {
  return String(s == null ? "" : s).replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);
}
function num(v) { return typeof v === "number" && isFinite(v) ? v : 0; }
function n(v) { return Math.round(num(v)).toLocaleString("en-US"); }
function ms(v) { v = num(v); return (v >= 10 ? v.toFixed(1) : v.toFixed(2)) + " ms"; }
function kb(v) { v = num(v); return v >= 1024 ? (v / 1024).toFixed(1) + " MB" : v.toFixed(0) + " KB"; }
function pct(a, b) { return b > 0 ? (a * 100) / b : 0; }
function duration(s) {
  s = Math.round(num(s));
  if (s < 60) return s + " s";
  const m = Math.floor(s / 60), r = s % 60;
  if (m < 60) return m + "m " + String(r).padStart(2, "0") + "s";
  return Math.floor(m / 60) + "h " + String(m % 60).padStart(2, "0") + "m";
}
function when(unix) {
  if (!unix) return "";
  const d = new Date(unix * 1000);
  const p = (x) => String(x).padStart(2, "0");
  return d.getFullYear() + "-" + p(d.getMonth() + 1) + "-" + p(d.getDate()) + " " + p(d.getHours()) + ":" + p(d.getMinutes());
}
function plural(c, one, many) { return n(c) + " " + (c === 1 ? one : many); }
// A Lua array: the client writes it as an array; a table of names as an object.
function list(x) { return Array.isArray(x) ? x.filter((v) => v != null) : x && typeof x === "object" ? Object.values(x) : []; }
function byName(x) {
  if (Array.isArray(x)) return x.map((v, i) => [String(i + 1), v]).filter(([, v]) => v != null);
  return x && typeof x === "object" ? Object.entries(x) : [];
}
// Per-second figures: holes count as 0.
function series(x, len) {
  const out = [];
  if (Array.isArray(x)) for (let i = 0; i < x.length; i++) out.push(num(x[i]));
  else if (x && typeof x === "object") for (const [k, v] of Object.entries(x)) out[Number(k) - 1] = num(v);
  for (let i = 0; i < (len || out.length); i++) if (out[i] == null) out[i] = 0;
  return out;
}
function share(part, whole) {
  const p = Math.max(0, Math.min(100, pct(part, whole)));
  return `<span class="share" title="${p.toFixed(1)}%"><i style="width:${p.toFixed(1)}%"></i></span>`;
}

// ------------------------------------------------------------ the data --

function prepare(r) {
  r.fns = byName(r.functions).map(([name, s]) => Object.assign({ name }, s));
  r.rootList = byName(r.roots).map(([name, s]) => Object.assign({ name }, s));
  r.pathList = list(r.paths).map((p) => Object.assign({}, p, { path: list(p.path) })).filter((p) => p.path.length > 0);
  r.slowList = list(r.slow);
  r.hintList = list(r.hints);
  r.memList = list(r.memory);
  const s = r.seconds || {};
  const len = Math.max(series(s.frames).length, series(s.time).length);
  r.sec = {};
  for (const k of ["frames", "time", "max", "over", "garbage", "garbageAll"]) r.sec[k] = series(s[k], len);
  r.monitor = {};
  for (const k of ["events", "scripts", "timers", "commands"]) {
    r.monitor[k] = byName(r[k]).map(([name, v]) => Object.assign({ name }, v));
  }
  return r;
}
const RECS = list(DATA.recordings).map(prepare);

// The call tree from the call paths: a node per path, with its root's own
// figures at the top.
function tree(r) {
  const top = { name: "all", kids: new Map(), calls: 0, total: 0, self: 0, max: 0, key: "" };
  for (const p of r.pathList) {
    let node = top;
    p.path.forEach((name, i) => {
      let kid = node.kids.get(name);
      if (!kid) {
        kid = { name, kids: new Map(), calls: 0, total: 0, self: 0, max: 0, key: node.key + "\u0001" + name, depth: i, parent: node };
        node.kids.set(name, kid);
      }
      node = kid;
    });
    node.calls += num(p.calls); node.total += num(p.total); node.self += num(p.self);
    node.max = Math.max(node.max, num(p.max));
  }
  const roots = new Map(r.rootList.map((x) => [x.name, x]));
  for (const kid of top.kids.values()) {
    const own = roots.get(kid.name);
    const below = [...kid.kids.values()].reduce((a, k) => a + k.total, 0);
    if (own) {
      kid.calls = num(own.calls); kid.total = Math.max(num(own.total), below); kid.max = num(own.max);
      kid.kind = own.kind;
    } else {
      kid.total = Math.max(kid.total, below);
    }
    kid.self = Math.max(kid.total - below, 0);
    top.total += kid.total;
  }
  return top;
}
function sortedKids(node) { return [...node.kids.values()].sort((a, b) => b.total - a.total); }

// --------------------------------------------------------------- state --

const state = { fnSort: "self", fnDesc: true, fnFilter: "", flame: "" };
function route() {
  const m = /^#\/(\d+)(?:\/([a-z]+))?/.exec(location.hash);
  let rec = m ? Number(m[1]) : 0;
  if (!(rec >= 0 && rec < RECS.length)) rec = 0;
  let view = m && m[2] ? m[2] : "overview";
  if (!VIEWS.some(([k]) => k === view)) view = "overview";
  return { rec, view };
}

// ------------------------------------------------------------- the page --

function header(r, at) {
  const what = r.kind === "pull" ? (r.detail || "boss pull") : r.kind;
  const recs = RECS.length > 1 ? '<div class="recs">' + RECS.map((x, i) =>
    `<a href="#/${i}/${at.view}" class="${i === at.rec ? "on" : ""}">${esc(x.name)}<small>${esc(x.kind === "pull" ? (x.detail || "pull") : x.kind)} · ${duration(x.duration)}</small></a>`
  ).join("") + "</div>" : "";
  const counts = { slow: r.slowList.length, overview: r.hintList.length };
  const nav = "<nav>" + VIEWS.map(([k, label]) =>
    `<a href="#/${at.rec}/${k}" class="${k === at.view ? "on" : ""}">${label}${counts[k] ? `<span class="count">${counts[k]}</span>` : ""}</a>`
  ).join("") + "</nav>";
  return `<header><div class="row"><h1>${esc(DATA.addon)} · ${esc(r.name)}</h1>` +
    `<span class="muted">${esc(what)} · ${duration(r.duration)}${r.started ? " · " + when(r.started) : ""}</span></div>` +
    `<div class="row meta"><small>version ${esc(r.version || "?")} · profile ${esc(r.profile || "?")} · build ${esc(r.build || "?")}` +
    `${r.compiler ? " · wowlua " + esc(r.compiler) : ""}</small></div>${recs}${nav}</header>`;
}

function tile(value, label, sub, cls) {
  return `<div class="tile"><div class="v ${cls || ""}">${value}</div><div class="k">${esc(label)}</div>${sub ? `<div class="s">${sub}</div>` : ""}</div>`;
}

function hints(r) {
  if (r.hintList.length === 0) return '<p class="note">No hints for this recording.</p>';
  return r.hintList.map((h) => `<div class="hint"><span class="code">hint[${esc(h.code)}]</span><span class="subject">${esc(h.subject)}</span>` +
    `<p>${esc(h.evidence)}</p><p class="fix">${esc(h.fix)}</p><p><small>wowlua explain ${esc(h.code)}</small></p></div>`).join("");
}

function overview(r) {
  const frames = num(r.frames), dur = Math.max(num(r.duration), 1);
  const worst = Math.max(0, ...r.sec.max);
  const tiles = [
    tile(n(frames), "frames", `${(frames / dur).toFixed(0)} per second`),
    tile(ms(frames ? num(r.time) / frames : 0), "addon time per frame", `${ms(r.time)} in all`),
    tile(n(r.over), "frames over the budget", `${pct(num(r.over), frames).toFixed(1)}% · budget ${ms(r.frameBudget)}`, num(r.over) > 0 ? "bad" : "good"),
    tile(ms(worst), "slowest frame", ""),
    tile(String(r.slowList.length), "slow calls", `over ${ms(r.slowCall)}`, r.slowList.length ? "mid" : ""),
    tile(kb(num(r.garbageAddon) / dur) + "/s", "garbage", `${kb(r.garbageAddon)} in all`),
    tile(kb(num(r.garbageAll) / dur) + "/s", "garbage, all addons", `the addon: ${pct(num(r.garbageAddon), num(r.garbageAll)).toFixed(1)}%`),
    tile(r.callPaths ? "on" : "off", "call paths", r.garbage ? "garbage per function on" : "garbage per function off"),
  ].join("");
  const top = r.fns.slice().sort((a, b) => num(b.self) - num(a.self)).slice(0, 10);
  const topTable = top.length ? `<div class="table-wrap"><table><thead><tr><th>Function</th><th class="l">Location</th><th>Calls</th><th>Self</th><th>Total</th><th>Max</th></tr></thead><tbody>` +
    top.map((f) => `<tr><td class="name">${esc(f.name)}</td><td class="l loc">${esc(f.location)}</td><td>${n(f.calls)}</td><td>${ms(f.self)}${share(num(f.self), num(r.time))}</td><td>${ms(f.total)}</td><td>${ms(f.max)}</td></tr>`).join("") +
    "</tbody></table></div>" : '<p class="note">No profiled function ran in this recording.</p>';
  return `<section><div class="tiles">${tiles}</div></section>` +
    `<section><h2>Hints</h2>${hints(r)}</section>` +
    `<section><h2>Frame time</h2>${timelineChart(r)}</section>` +
    `<section><h2>Most own time</h2>${topTable}</section>`;
}

// The functions table: sortable, filterable.
const FN_COLS = [
  ["name", "Function", "l"], ["location", "Location", "l"], ["calls", "Calls"], ["total", "Total"], ["self", "Self"],
  ["max", "Max"], ["avg", "Avg"], ["over", "Over budget"], ["errors", "Errors"],
];
const GC_COLS = [["garbage", "Garbage"], ["selfGarbage", "Own garbage"]];
function functions(r) {
  const cols = FN_COLS.concat(r.garbage ? GC_COLS : []);
  const q = state.fnFilter.toLowerCase();
  const val = (f, k) => k === "avg" ? num(f.total) / Math.max(num(f.calls), 1) : k === "name" || k === "location" ? String(f[k] || "") : num(f[k]);
  const rows = r.fns.filter((f) => !q || f.name.toLowerCase().includes(q) || String(f.location || "").toLowerCase().includes(q))
    .sort((a, b) => {
      const x = val(a, state.fnSort), y = val(b, state.fnSort);
      const d = x < y ? -1 : x > y ? 1 : a.name < b.name ? -1 : 1;
      return state.fnDesc ? -d : d;
    });
  const head = cols.map(([k, label, cls]) => `<th data-sort="${k}" class="${cls || ""}">${label}${state.fnSort === k ? `<span class="arrow"> ${state.fnDesc ? "▾" : "▴"}</span>` : ""}</th>`).join("");
  const body = rows.map((f) => "<tr>" + cols.map(([k]) => {
    if (k === "name") return `<td class="name" title="${esc(f.name)}">${esc(f.name)}</td>`;
    if (k === "location") return `<td class="l loc">${esc(f.location)}</td>`;
    if (k === "calls" || k === "over" || k === "errors") return `<td class="${k !== "calls" && num(f[k]) > 0 ? "bad" : ""}">${n(f[k])}</td>`;
    if (k === "garbage" || k === "selfGarbage") return `<td>${kb(f[k])}</td>`;
    if (k === "self") return `<td>${ms(f.self)}${share(num(f.self), num(r.time))}</td>`;
    return `<td>${ms(val(f, k))}</td>`;
  }).join("") + "</tr>").join("");
  return `<section><div class="controls"><input type="search" id="fn-filter" placeholder="Filter by name or file" value="${esc(state.fnFilter)}">` +
    `<small>${plural(rows.length, "function", "functions")} · self time's share of the addon's ${ms(r.time)}</small></div>` +
    (r.fns.length ? `<div class="table-wrap"><table id="fn-table"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>` :
      '<p class="note">No profiled function ran in this recording.</p>') + "</section>";
}

function events(r) {
  const roots = r.rootList.slice().sort((a, b) => num(b.total) - num(a.total));
  const all = roots.reduce((a, x) => a + num(x.total), 0);
  let out = "<section><h2>By event</h2>";
  out += roots.length ? `<div class="table-wrap"><table><thead><tr><th>Root</th><th class="l">Kind</th><th>Calls</th><th>Total</th><th>Max</th><th>Avg</th><th class="l">Most own time in</th><th>Its self</th></tr></thead><tbody>` +
    roots.map((x) => `<tr><td class="name">${esc(x.name)}</td><td class="l muted">${esc(x.kind)}</td><td>${n(x.calls)}</td><td>${ms(x.total)}${share(num(x.total), all)}</td>` +
      `<td class="${num(x.max) > num(r.frameBudget) ? "bad" : ""}">${ms(x.max)}</td><td>${ms(num(x.total) / Math.max(num(x.calls), 1))}</td><td class="l">${esc(x.top || "")}</td><td>${x.top ? ms(x.topSelf) : ""}</td></tr>`).join("") +
    "</tbody></table></div>" : '<p class="note">Nothing ran in this recording.</p>';
  out += "</section>";
  const titles = { events: "Events", scripts: "Scripts", timers: "Timers", commands: "Slash commands" };
  for (const k of ["events", "scripts", "timers", "commands"]) {
    const rows = r.monitor[k].slice().sort((a, b) => num(b.total) - num(a.total));
    if (!rows.length) continue;
    const extra = k === "events" ? "<th>Bursty frames</th><th>Most in a frame</th>" : "";
    out += `<section><h3>${titles[k]}</h3><div class="table-wrap"><table><thead><tr><th>${k === "scripts" ? "Frame and script" : "Name"}</th><th>Count</th><th>Total</th><th>Max</th><th>Over budget</th><th>Frames</th>${extra}<th class="l">Set at</th></tr></thead><tbody>` +
      rows.map((x) => `<tr><td class="name" title="${esc(x.name)}">${esc(x.name)}${x.showsNothing ? ` <small class="mid">(${esc(x.showsNothing)})</small>` : ""}</td><td>${n(x.count)}</td><td>${ms(x.total)}</td><td>${ms(x.max)}</td>` +
        `<td class="${num(x.over) > 0 ? "bad" : ""}">${n(x.over)}</td><td>${n(x.frames)}</td>` +
        (k === "events" ? `<td>${n(x.bursts)}</td><td>${n(x.burstMax)}</td>` : "") + `<td class="l loc">${esc(x.detail)}</td></tr>`).join("") +
      "</tbody></table></div></section>";
  }
  return out;
}

function needsPaths(r) {
  return '<p class="note">Call paths were off for this recording. Switch on <b>Call paths</b> in the dev window\'s performance tab ' +
    "(or set <code>profile-call-paths</code> in the profile) before you record, then export again.</p>";
}

function treeView(r) {
  if (!r.pathList.length) return "<section>" + needsPaths(r) + "</section>";
  const top = tree(r);
  const row = (node, depth) => {
    const cells = `<span class="nm" style="padding-left:${depth * 16}px" title="${esc(node.name)}">${esc(node.name)}${node.kind ? `<span class="kind">${esc(node.kind)}</span>` : ""}</span>` +
      `<span>${n(node.calls)}</span><span>${ms(node.total)}</span><span>${ms(node.self)}</span><span>${ms(node.max)}</span><span>${pct(node.total, top.total).toFixed(1)}%</span>`;
    if (!node.kids.size) return `<div class="leaf">${cells}</div>`;
    return `<details${depth < 2 ? " open" : ""}><summary>${cells}</summary>${sortedKids(node).map((k) => row(k, depth + 1)).join("")}</details>`;
  };
  return `<section><p class="muted">From the event, script, timer or slash command that started the work down to each profiled function; ${ms(top.total)} in all.</p>` +
    `<div class="tree"><div class="head"><span>Call</span><span>Calls</span><span>Total</span><span>Self</span><span>Max</span><span>Share</span></div>` +
    sortedKids(top).map((k) => row(k, 0)).join("") + "</div></section>";
}

// The flame graph: width by total time; click a bar to zoom in, a bar
// above it to zoom out.
const HUES = [32, 18, 42, 8, 50, 26];
function hue(name) {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return HUES[h % HUES.length];
}
function flameView(r) {
  if (!r.pathList.length) return "<section>" + needsPaths(r) + "</section>";
  const top = tree(r);
  const find = (node, key) => {
    if (node.key === key) return node;
    for (const k of node.kids.values()) {
      if (key.startsWith(k.key)) { const f = find(k, key); if (f) return f; }
    }
    return null;
  };
  const focus = (state.flame && find(top, state.flame)) || top;
  const bars = [];
  const up = [];
  for (let p = focus; p; p = p.parent) up.unshift(p);
  up.forEach((p, i) => {
    const label = p === top ? `all · ${ms(top.total)}` : `${p.name} · ${ms(p.total)}`;
    bars.push(`<div class="${p === focus ? "" : "up"}" data-key="${esc(p.key)}" style="left:0;width:100%;top:${i * 20}px;${p === focus ? `background:hsl(${hue(p.name)} 80% 62%)` : ""}" title="${esc(p.name)}: ${ms(p.total)}">${esc(label)}</div>`);
  });
  let depth = up.length;
  let maxDepth = depth;
  const lay = (node, left, width, d) => {
    let x = left;
    for (const k of sortedKids(node)) {
      const w = node.total > 0 ? (width * k.total) / node.total : 0;
      if (w >= 0.05) {
        bars.push(`<div data-key="${esc(k.key)}" style="left:${x}%;width:${w}%;top:${d * 20}px;background:hsl(${hue(k.name)} 80% ${62 - Math.min(d, 8) * 2}%)" ` +
          `title="${esc(k.name)}&#10;total ${ms(k.total)} (${pct(k.total, top.total).toFixed(1)}%), self ${ms(k.self)}, ${n(k.calls)} calls">${esc(k.name)}</div>`);
        maxDepth = Math.max(maxDepth, d + 1);
        lay(k, x, w, d + 1);
      }
      x += w;
    }
  };
  lay(focus, 0, 100, depth);
  return `<section><p class="muted">Width by total time. Click a bar to zoom in, a bar above it to zoom out.</p>` +
    `<div class="flame" id="flame" style="height:${maxDepth * 20 + 1}px">${bars.join("")}</div></section>`;
}

// A bar chart over the recording's seconds: `bars` [{ values, cls, label }]
// stacked side by side per second, `line` a dashed threshold.
function barChart(values, opts) {
  const len = values.length;
  const span = opts.span || len;
  if (!len) return '<p class="note">No frames were measured in this recording.</p>';
  const top = Math.max(opts.min || 0, ...values, ...(opts.line ? [opts.line * 1.5] : []), 1e-9);
  const h = 100;
  const rects = values.map((v, i) => {
    const bh = (v / top) * h;
    const cls = opts.cls(i, v);
    return `<rect x="${i + 0.1}" y="${h - bh}" width="0.8" height="${bh}" class="${cls}"><title>${esc(opts.title(i, v))}</title></rect>`;
  }).join("");
  const extra = (opts.overlay || []).map((o) => {
    const pts = o.values.map((v, i) => `${i + 0.5},${h - (Math.min(v, top) / top) * h}`).join(" ");
    return `<polyline points="${pts}" fill="none" class="${o.cls}" stroke-width="1.5" vector-effect="non-scaling-stroke"/>`;
  }).join("");
  const line = opts.line ? `<line x1="0" x2="${len}" y1="${h - (opts.line / top) * h}" y2="${h - (opts.line / top) * h}" class="stroke-budget" stroke-width="1" vector-effect="non-scaling-stroke"/>` : "";
  const legend = (opts.legend || []).map(([cls, label]) => `<span><i class="${cls}"></i>${esc(label)}</span>`).join("");
  return `<div class="chart">${legend ? `<div class="legend">${legend}</div>` : ""}<div class="plot"><div class="y"><span>${opts.fmt(top)}</span><span>${opts.fmt(top / 2)}</span><span>0</span></div>` +
    `<div class="area"><svg viewBox="0 0 ${len} ${h}" preserveAspectRatio="none" role="img" aria-label="${esc(opts.label)}">${rects}${extra}${line}</svg>` +
    `<div class="axis"><span>0 s</span><span>${duration(span / 2)}</span><span>${duration(span)}</span></div></div></div></div>`;
}

function timelineChart(r) {
  const s = r.sec, budget = num(r.frameBudget);
  const avg = s.time.map((t, i) => s.frames[i] ? t / s.frames[i] : 0);
  return barChart(s.max, {
    label: "slowest frame per second",
    line: budget,
    fmt: ms,
    cls: (i, v) => v > budget ? "fill-bad" : "fill-accent",
    title: (i, v) => `${duration(i)}: ${n(s.frames[i])} frames, slowest ${ms(v)}, average ${ms(avg[i])}, ${n(s.over[i])} over the budget`,
    overlay: [{ values: avg, cls: "stroke-text" }],
    legend: [["bg-accent", "slowest frame per second"], ["bg-bad", "over the budget"], ["bg-text", "average frame"], ["bg-bad", `budget ${ms(budget)} (dashed)`]],
  });
}

function timelineView(r) {
  const s = r.sec;
  const rows = s.max.map((v, i) => ({ i, max: v, frames: s.frames[i], time: s.time[i], over: s.over[i] }))
    .filter((x) => x.frames > 0).sort((a, b) => b.max - a.max).slice(0, 15);
  const overChart = barChart(s.over, {
    label: "frames over the budget per second", fmt: (v) => v.toFixed(0), min: 1,
    cls: () => "fill-bad", title: (i, v) => `${duration(i)}: ${n(v)} of ${n(s.frames[i])} frames over the budget`,
  });
  return `<section><h2>Frame time</h2><p class="muted">The addon's time in each rendered frame, bucketed per second, against the frame budget of ${ms(r.frameBudget)}.</p>${timelineChart(r)}</section>` +
    `<section><h2>Frames over the budget</h2>${overChart}</section>` +
    (rows.length ? `<section><h2>Slowest seconds</h2><div class="table-wrap"><table><thead><tr><th>At</th><th>Frames</th><th>Slowest frame</th><th>Average</th><th>Over budget</th><th>Addon time</th></tr></thead><tbody>` +
      rows.map((x) => `<tr><td>${duration(x.i)}</td><td>${n(x.frames)}</td><td class="${x.max > num(r.frameBudget) ? "bad" : ""}">${ms(x.max)}</td><td>${ms(x.time / x.frames)}</td><td>${n(x.over)}</td><td>${ms(x.time)}</td></tr>`).join("") +
      "</tbody></table></div></section>" : "");
}

function stackHtml(text) {
  return esc(text).split("\n").map((l) => /^\s*→ /.test(l) ? `<span class="mapped">${l}</span>` : l).join("\n");
}
function slowView(r) {
  if (!r.slowList.length) return `<section><p class="note">No call took longer than ${ms(r.slowCall)} in this recording.</p></section>`;
  return `<section><p class="muted">Calls longer than ${ms(r.slowCall)}, newest first; only the deepest slow call of a stack is listed, its callers are in its stack. Click one for its stack.</p>` +
    `<div class="slow"><div class="head"><span>At</span><span class="r">Time</span><span>Function</span><span>Location</span><span>Root</span></div>` +
    r.slowList.map((s) => `<details><summary><span>${duration(s.at)}</span><span class="r ${num(s.ms) > num(r.frameBudget) ? "bad" : ""}">${ms(s.ms)}</span>` +
      `<span title="${esc(s.name)}">${esc(s.name)}</span><span class="muted" title="${esc(s.location)}">${esc(s.location)}</span><span class="muted">${esc(s.root)}</span></summary>` +
      `<pre class="stack">${s.stack ? stackHtml(s.stack) : "(no stack)"}</pre></details>`).join("") + "</div></section>";
}

function garbageView(r) {
  const s = r.sec, dur = Math.max(num(r.duration), 1);
  const tiles = [
    tile(kb(num(r.garbageAddon) / dur) + "/s", "the addon's garbage", `${kb(r.garbageAddon)} in all`),
    tile(kb(num(r.garbageAll) / dur) + "/s", "all addons' garbage", `${kb(r.garbageAll)} in all`),
    tile(pct(num(r.garbageAddon), num(r.garbageAll)).toFixed(1) + "%", "the addon's share", ""),
  ];
  if (r.garbage) tiles.push(tile(kb(r.garbageFns), "made by profiled functions themselves", ""));
  const chart = barChart(s.garbageAll, {
    label: "garbage per second", fmt: kb,
    cls: () => "fill-faint",
    title: (i, v) => `${duration(i)}: the addon ${kb(s.garbage[i])}, all addons ${kb(v)}`,
    overlay: [{ values: s.garbage, cls: "stroke-note" }],
    legend: [["bg-faint", "all addons per second"], ["bg-note", "the addon per second"]],
  });
  let memory = "";
  if (r.memList.length > 1) {
    const step = Math.max(...r.memList.map((m) => num(m.at)), 1);
    const len = Math.ceil(step / 5) + 1;
    const vals = new Array(len).fill(0);
    for (const m of r.memList) vals[Math.round(num(m.at) / 5)] = num(m.kb);
    for (let i = 1; i < len; i++) if (!vals[i]) vals[i] = vals[i - 1];
    memory = `<section><h2>Memory</h2><p class="muted">The addon's memory, sampled every 5 seconds outside combat.</p>` + barChart(vals, {
      label: "memory", fmt: kb, cls: () => "fill-note",
      title: (i, v) => `${duration(i * 5)}: ${kb(v)}`, span: step,
    }) + "</section>";
  }
  let fns;
  if (!r.garbage) {
    fns = '<p class="note">Garbage per function was off for this recording. Switch on <b>Garbage per function</b> in the dev window\'s performance tab ' +
      "(or set <code>profile-garbage</code> in the profile) before you record, then export again.</p>";
  } else {
    const rows = r.fns.filter((f) => num(f.garbage) > 0).sort((a, b) => num(b.selfGarbage) - num(a.selfGarbage));
    const all = Math.max(num(r.garbageFns), 1e-9);
    fns = rows.length ? `<div class="table-wrap"><table><thead><tr><th>Function</th><th class="l">Location</th><th>Calls</th><th>Own garbage</th><th>With callees</th><th>Per call</th><th>Per second</th></tr></thead><tbody>` +
      rows.map((f) => `<tr><td class="name">${esc(f.name)}</td><td class="l loc">${esc(f.location)}</td><td>${n(f.calls)}</td><td>${kb(f.selfGarbage)}${share(num(f.selfGarbage), all)}</td>` +
        `<td>${kb(f.garbage)}</td><td>${kb(num(f.selfGarbage) / Math.max(num(f.calls), 1))}</td><td>${kb(num(f.selfGarbage) / dur)}</td></tr>`).join("") +
      "</tbody></table></div>" : '<p class="note">No profiled function made garbage in this recording.</p>';
  }
  return `<section><div class="tiles">${tiles.join("")}</div></section><section><h2>Garbage per second</h2>${chart}</section>${memory}` +
    `<section><h2>Garbage per function</h2>${fns}</section>`;
}

// ---------------------------------------------------------------- draw --

function draw() {
  const app = document.getElementById("app");
  const from = `<span class="from">${esc(DATA.client)} · ${esc(DATA.account)}<br>${esc(DATA.date)}</span></div>`;
  if (!RECS.length) {
    app.innerHTML = BRAND + from + `<header><h1>${esc(DATA.addon)}</h1></header><main><p class="note">No recordings.</p></main>`;
    return;
  }
  const at = route();
  const r = RECS[at.rec];
  const views = { overview, functions, events, tree: treeView, flame: flameView, timeline: timelineView, slow: slowView, garbage: garbageView };
  document.title = `${DATA.addon} · ${r.name} · performance`;
  app.innerHTML = BRAND + from + header(r, at) + "<main>" + views[at.view](r) + "</main>";
  const filter = document.getElementById("fn-filter");
  if (filter) {
    filter.addEventListener("input", () => {
      state.fnFilter = filter.value;
      const pos = filter.selectionStart;
      draw();
      const again = document.getElementById("fn-filter");
      again.focus();
      again.setSelectionRange(pos, pos);
    });
  }
  for (const th of app.querySelectorAll("th[data-sort]")) {
    th.addEventListener("click", () => {
      const k = th.dataset.sort;
      if (state.fnSort === k) state.fnDesc = !state.fnDesc;
      else { state.fnSort = k; state.fnDesc = k !== "name" && k !== "location"; }
      draw();
    });
  }
  const flame = document.getElementById("flame");
  if (flame) {
    flame.addEventListener("click", (e) => {
      const bar = e.target.closest("[data-key]");
      if (!bar) return;
      state.flame = bar.dataset.key;
      draw();
    });
  }
}

window.addEventListener("hashchange", () => { state.flame = ""; draw(); window.scrollTo(0, 0); });
draw();
