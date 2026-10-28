# Embedding wowlua in a web page

`@wowlua/web` (the `web/` folder) is wowlua's compiler, checker, formatter
and language server as WebAssembly, over a package held in memory, with the
emulated client that runs and draws the addon and the debugger. It is what
the playground on wowlua.dev embeds; it brings no editor and no page design
of its own.

## Install

Each wowlua release has the package attached:
`wowlua-web-<version>.tgz` (with its SHA-256). Install it from there:

```sh
npm install https://github.com/DomenikIrrgang/wowlua/releases/download/v0.13.6/wowlua-web-0.13.6.tgz
```

The module is about 74 MB, 13 MB with gzip and less with Brotli (the
compiler, the language server, the emulated client and its renderer, and
the game API's declarations and recordings): serve it compressed, with
`Content-Type: application/wasm`, and let the browser cache it.

The worker, the module and the scripts load each other by relative URLs,
so the simplest way to serve them with any framework is as static files:
copy the package's files (`index.js`, `client.js`, `worker.js`,
`installation.js`, `pkg/`) to a folder the site serves, say
`public/wowlua/` in Next.js (a `postinstall` script does it), and start
the worker from there:

```js
import { connect } from "@wowlua/web/client";
const wowlua = connect(new Worker("/wowlua/worker.js", { type: "module" }));
```

The worker loads `pkg/wowlua_web_bg.wasm` next to it; `connect(worker, {
wasm: url })` loads the module from another URL.

## Build

To build it from the repository you need Rust with the
`wasm32-unknown-unknown` target and
[`wasm-bindgen`](https://github.com/wasm-bindgen/wasm-bindgen) 0.2.129 (the
version in `Cargo.lock`):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cd web
npm run build   # pkg/wowlua_web.js and pkg/wowlua_web_bg.wasm
npm test        # the package as a page uses it, in Node
npm pack        # wowlua-web-<version>.tgz, as the release has it
```

## The playground

One playground per page or Web Worker; its files are the module's. Paths
are relative to the package's folder.

```js
import { load, Playground } from "@wowlua/web";

await load();                    // or load(url) when the .wasm is elsewhere
const p = new Playground();      // wowlua.json and src/main.wowlua
p.writeFile("src/main.wowlua", text);
p.check();                       // [{ severity, code, message, file, range, text }]
p.build("dev");                  // { ok, diagnostics, files: [{ path, text }] }
p.format("src/main.wowlua");     // { text } (written) or { diagnostics }
p.files();                       // { "wowlua.json": "…", "src/main.wowlua": "…" }
```

The files you write are the package, as files on disk are for the
command line: `check`, `build`, `run` and `format` read them, and report
what `wowlua check` and `wowlua build` report for the same files (the
manifest's `check.lints` apply, so W2063 stays off until you turn it on).
Write the editor's text with `writeFile` before you check, build or run
it.

A diagnostic's `text` is the diagnostic as `wowlua check` prints it,
without colors: the code, the message, the source lines and the help.
Ranges are the Language Server Protocol's: lines and UTF-16 columns from
0, which CodeMirror and Monaco both take. A built file that is not text,
such as an image, comes as URL-safe `base64` instead of `text`.

## Share links

`p.share()` gives the package's files as a link's fragment (deflated JSON,
URL-safe base64): put it after the `#` so it never reaches the server.
`p.openShared(location.href)` opens one and returns how it runs (below; `{}`
when the link does not say). For anything else it returns `null` and changes
nothing.

## The language server

Run the playground in a Web Worker so a check never blocks typing:

```js
import { connect } from "@wowlua/web/client";

const wowlua = connect(
    new Worker(new URL("@wowlua/web/worker", import.meta.url), { type: "module" }),
);
await wowlua.call("writeFile", "src/main.wowlua", text);

// the editor's LSP client
wowlua.lsp.onMessage((message) => client.receive(message));
client.onSend((message) => wowlua.lsp.send(message));
```

The server is the one `wowlua lsp` runs in VS Code: diagnostics as you
type, hover, completion, signature help, go to definition, references,
rename, formatting and the outline. Documents are
`file:///playground/<path>`; open them with `textDocument/didOpen`
(`languageId` `wowlua`) and send changes as usual. Go to definition can
lead into wowlua's built-in declarations (the game's API, the standard
library), whose URIs are `file:///.wowlua-<version>/…`: read them with
`call("readFile", path)` and show them read-only.

The server works as in an editor on disk: it reads the documents the
editor has open from the editor (`didOpen`, `didChange`) and the other
files as you wrote them. When you write, delete or replace files
(`writeFile`, `deleteFile`, `reset`, `openShared`, `format`), it checks
the package again, as after files changed on disk; a document the editor
has open keeps the editor's text there until the editor changes it.

Any LSP client works over this transport, for example
`@codemirror/lsp-client` or `monaco-languageclient`.

## Running the addon

The page can run the package in the emulated client and draw it on a
canvas, with its mouse and keyboard. Through the worker:

```js
await wowlua.attachCanvas(canvas);   // set canvas.width and height first
wowlua.onOutput((o) => chat.append(o.text));      // print() and errors
wowlua.onProgress((done, total) => …);            // while the UI loads
await wowlua.call("loadBlizzardUI", { flavor: "camelot" });
const started = await wowlua.call("run", { blizzardUI: true });
// started: { ok, diagnostics, error }
await wowlua.call("stop");
const png = await wowlua.call("snapshot", 1);     // the screen as a PNG Blob
```

- The game draws with WebGPU where the browser has a GPU adapter for it,
  else WebGL 2; `attachCanvas` resolves to which (`BrowserWebGpu`, `Gl`).
  It draws at the canvas's size, scaled from the client's 1920×1080
  screen; keep the canvas 16:9.
- Clicks, mouse moves and keys on the canvas reach the game: a click on a
  button runs its `OnClick`, typing goes into the focused edit box.
- Blizzard's default UI is its code from Gethe/wow-ui-source at the
  commit wowlua's declarations name, fetched by the page from jsDelivr
  (about 4,500 files, 36 MB for camelot: what loading it reads, then the
  addons it loads on demand, such as the settings and the spellbook). The
  browser caches the files: a first load takes about a minute, later ones
  a few seconds. `background: true` starts runs after the first 2,450 or so
  files and fetches the rest meanwhile; an addon Blizzard loads on demand
  before its files came is then missing. Pass `base` to fetch from
  another place with the same paths. Without it (`blizzardUI: false`) the
  addon runs alone, at once.
- Without a canvas the game runs all the same, not drawn: what it prints
  and raises comes to `onOutput`. `call("advance")` moves it to now at
  once and returns that output too, as `input` and `world` return theirs.
- The game's textures, fonts and models are the user's installation's
  (below); without it images are placeholders (magenta) and text uses
  stand-in fonts, except the addon's own images (its PNG, TGA and BLP
  files).

Without the worker, `Playground` has the same calls (`loadBlizzardUI`,
`run`, `advance`, `input`, `stop`) and `createScreen(canvas)` with
`playground.play(screen, onOutput)` (`play(null, onOutput)` runs it
without drawing).

## World profiles

A package's world profiles (`tests/profiles/<name>.json`, the world a
test or a run starts in: the character, money, gear, bags, units around,
…) work in the page as in tests:

```js
await wowlua.call("profiles");                         // ["rich", "stormwind"]
await wowlua.call("run", { profile: "stormwind" });    // by name
await wowlua.call("run", { profileText: json });       // or given as JSON
await wowlua.call("world", "mock.player:setMoney(500)"); // change it while it runs
const schema = await wowlua.call("profileSchema");     // for the editor's JSON support
```

A check reports a profile's mistakes like any other (E4400 to E4402, on
the profile's file). `world` runs Lua with `mock` the controls tests use
(`mock.player:enterCombat()`, `mock.player:target(mock.spawnUnit({ name =
"Hogger", reaction = "hostile" }))`); while debugging, the debugger's
`world` request does the same between frames. Share links can carry how
the package runs: `call("share", { flavor, profile, blizzardUI })`, and
`openShared` gives those settings back.

## The user's installation

With the user's World of Warcraft folder the game draws with its own
textures, fonts and models, and answers from its tables (spells, items,
currencies, …; current data before wowlua's recordings, as `wowlua test
--game-data` does). The folder is read in the worker, a file at a time as
wowlua needs it, and never leaves the user's machine.

```js
// a button's click (a user gesture): Chrome and Edge
const folder = await showDirectoryPicker({ id: "wow", mode: "read" });
await wowlua.useInstallation(folder, { flavor: "camelot" });
// elsewhere: <input type="file" webkitdirectory> and its files
await wowlua.useInstallation(input.files, { flavor: "camelot" });
// → { flavor, product: "wow_classic_beta", files: 501 }
```

The user picks the folder that holds `Data` and the clients' folders
(`_classic_beta_`, …); wowlua indexes `.build.info`, `Data/config`,
`Data/data` and each client's `.flavor.info` and `Cache` (the hotfixes),
about 500 files, and opens the flavor's client in well under a second.
Runs of that flavor started afterwards use it. A browser keeps a picked
folder's permission for the session; a page may keep the
`FileSystemDirectoryHandle` in IndexedDB and ask again
(`requestPermission`) on a later visit. A folder `<input>` makes the
browser list every file of the folder first and may ask the user to
confirm an "upload" (nothing is sent). Without the installation
everything works with placeholders.

## Debugging

The debugger of `wowlua debug`
([spec chapter 18](../../spec/toolchain/debugger.md)) runs in the page too:
breakpoints (lines, functions, events, scripts, exceptions), stepping,
the stack, variables and evaluating code where it stopped, the frame
tree. When it stops, the worker waits for the page's next request on a
`SharedArrayBuffer`, so the page must be **cross-origin isolated**: serve
it with

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

(in Next.js, `headers()` in `next.config.js` for `/playground/:path*`).
Other origins' images and scripts the page embeds then need CORS or
`Cross-Origin-Resource-Policy`; `credentialless` instead of `require-corp`
avoids that where the browser supports it. Blizzard's UI is fetched with
CORS and works either way. `wowlua.debug.available` says whether the page
is isolated; without it everything else works.

```js
wowlua.debug.onEvent(async (e) => {
    if (e.event === "stopped") {                  // reason: breakpoint, step, pause, …
        const { frames } = await wowlua.debug.request("stackTrace");
        const { scopes } = await wowlua.debug.request("scopes", { frame: frames[0].id });
        const { variables } = await wowlua.debug.request("variables", { reference: scopes[0].reference });
    }
});
await wowlua.call("run", {
    debug: {
        breakpoints: { "src/main.wowlua": [{ line: 12 }, { line: 20, condition: "count > 3" }] },
        functionBreakpoints: [{ name: "event:PLAYER_LOGIN" }],
        exceptions: ["uncaught"],
    },
});
await wowlua.debug.request("setBreakpoints", { source: "src/main.wowlua", breakpoints: [{ line: 14 }] });
await wowlua.debug.request("next");               // continue, stepIn, stepOut, pause
const { variable } = await wowlua.debug.request("evaluate", { expression: "count", frame: 1 });
```

Requests and answers are wowlua's debugger protocol
([`crates/wowlua-debug/src/protocol.rs`](../../crates/wowlua-debug/src/protocol.rs)):
a request is `{ command, … }`, an answer `{ kind, … }`. The answer to
`continue` or a step means the game runs on until the next `stopped`;
`terminated` ends the session (a `stop`, or the run ended). While it is stopped the game does not move or
draw, and calls to the worker wait until it runs on. Sources are the
package's paths; frames in Blizzard's code name its files
(`Interface/AddOns/…`), which the page can show with `readFile`.

## A test page

`web/demo/index.html` uses only the package, through its worker: a text
area for `src/main.wowlua`, the language server's diagnostics as you type,
hover on click, build, format, share, the game on a canvas (with or
without Blizzard's UI) with its chat output, and the debugger. Serve
`web/` cross-origin isolated and open it:

```sh
cd web && npm run build && node demo/serve.mjs
# http://127.0.0.1:8000/demo/
```

## When wowlua fails

A panic in wowlua ends the WebAssembly module (it has no unwinding), and
every later call throws; its message is on the console ("wowlua panicked:
…"). Start a new worker; the files of the old one are
gone, so keep the editor's texts and write them again.

## Not yet

Dependencies other than the standard library and Blizzard's UI for flavors
other than camelot are not in the playground yet (see the
[roadmap](../../spec/roadmap.md)).
