# Embedding wowlua in a web page

`@wowlua/web` (the `web/` folder) is wowlua's compiler, checker, formatter
and language server as WebAssembly, over a package held in memory. It is
what the playground on wowlua.dev embeds; it brings no editor and no page
design of its own.

## Build

You need Rust with the `wasm32-unknown-unknown` target and
[`wasm-bindgen`](https://github.com/wasm-bindgen/wasm-bindgen) 0.2.129 (the
version in `Cargo.lock`):

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cd web
npm run build   # pkg/wowlua_web.js and pkg/wowlua_web_bg.wasm
npm test        # the package as a page uses it, in Node
```

The module is about 41 MB, 7.5 MB with gzip: serve it compressed, with
`Content-Type: application/wasm`, and let the browser cache it.

## The playground

One playground per page or Web Worker; its files are the module's. Paths
are relative to the package's folder.

```js
import { load, Playground } from "@wowlua/web";

await load();                    // or load(url) when the .wasm is elsewhere
const p = new Playground();      // wowlua.json and src/main.wowlua
p.writeFile("src/main.wowlua", text);
p.check();                       // [{ severity, code, message, file, range }]
p.build("dev");                  // { ok, diagnostics, files: [{ path, text }] }
p.format("src/main.wowlua");     // { text } (written) or { diagnostics }
p.files();                       // { "wowlua.json": "…", "src/main.wowlua": "…" }
```

Ranges are the Language Server Protocol's: lines and UTF-16 columns from
0, which CodeMirror and Monaco both take.

## Share links

`p.share()` gives the package's files as a link's fragment (deflated JSON,
URL-safe base64): put it after the `#` so it never reaches the server.
`p.openShared(location.href)` opens one; it returns `false` and changes
nothing for anything else.

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

Any LSP client works over this transport, for example
`@codemirror/lsp-client` or `monaco-languageclient`.

## When wowlua fails

A panic in wowlua ends the WebAssembly module (it has no unwinding), and
every later call throws. Start a new worker; the files of the old one are
gone, so keep the editor's texts and write them again.

## Not yet

Running the addon, drawing it and the debugger come with M14's next
stages (`spec/roadmap.md`); so do dependencies other than the standard
library and game data from the user's installation.
