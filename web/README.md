# @wowlua/web

[wowlua](https://github.com/DomenikIrrgang/wowlua) in a web page: the
compiler, checker, formatter and language server for World of Warcraft
addons, and the emulated client that runs and draws them, as WebAssembly.
It is what the playground on wowlua.dev embeds; it brings no editor and no
page design of its own.

- A package in memory: write, check, build, format, share in a link.
- The language server of `wowlua lsp`, for any LSP client in the browser.
- The addon running in the emulated client on a canvas, after Blizzard's
  default UI, with mouse, keyboard and its chat output.
- The debugger: breakpoints, stepping, the stack and variables.
- The user's own WoW installation for the game's art and data, read in
  the browser and never uploaded.

```js
import { connect } from "@wowlua/web/client";

const wowlua = connect(
    new Worker(new URL("@wowlua/web/worker", import.meta.url), { type: "module" }),
);
await wowlua.call("writeFile", "src/main.wowlua", 'print("Hello")');
const diagnostics = await wowlua.call("check");
```

Install it from a release (`wowlua-web-<version>.tgz` on GitHub). The
guide is
[docs/reference/embedding.md](https://github.com/DomenikIrrgang/wowlua/blob/main/docs/reference/embedding.md);
the repository's `web/demo/` is a bare page that uses all of it.

Blizzard's UI code is fetched by the page from a public mirror and the
game's assets come from the user's installation: the package distributes
neither.
