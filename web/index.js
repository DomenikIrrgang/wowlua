// @wowlua/web: wowlua in a web page (spec M14 S2, decision T395).
//
// The compiler, checker, formatter and language server, built as
// WebAssembly (crates/wowlua-web), over a package held in memory. One
// playground per page (or Web Worker): its files are the module's.
//
//   import { load, Playground } from "@wowlua/web";
//   await load();                          // fetches wowlua_web_bg.wasm
//   const p = new Playground();            // the template package
//   p.writeFile("src/main.wowlua", text);
//   p.check();                             // [{ severity, code, message, file, range }]
//
// Paths are relative to the package's folder; the language server's URIs
// are `file:///playground/<path>`. A panic in wowlua ends the module
// (WebAssembly has no unwinding): calls then throw, and a page makes a new
// one with `load()` again.

import init, { Playground as Raw } from "./pkg/wowlua_web.js";

let loaded = null;

/**
 * Loads the WebAssembly module once: from `wasm` (a URL, a Response, bytes
 * or a WebAssembly.Module), by default the file next to this one.
 */
export function load(wasm) {
    if (!loaded) {
        loaded = init(wasm === undefined ? undefined : { module_or_path: wasm });
    }
    return loaded;
}

/** wowlua's version, once loaded. */
export function version() {
    return Raw.version();
}

const now = () => (typeof performance !== "undefined" ? performance.now() : Date.now());

/** A package in memory, and its language server. */
export class Playground {
    constructor() {
        this.raw = new Raw();
        this.tick();
    }

    /** Passes the clocks in (WebAssembly has none it can read). */
    tick() {
        this.raw.setTime(Date.now(), now());
    }

    /** `{ "<path>": "<text>" }` of the package (not what builds wrote). */
    files() {
        return JSON.parse(this.raw.files());
    }

    /** Replaces every file with `files` (`{ "<path>": "<text>" }`). */
    reset(files) {
        this.raw.reset(JSON.stringify(files));
    }

    writeFile(path, text) {
        this.raw.writeFile(path, text);
    }

    /**
     * A file's text, or `undefined`: a path of the package, or an absolute
     * one (go to definition can lead to wowlua's built-in declarations,
     * under `/.wowlua-<version>/`).
     */
    readFile(path) {
        return this.raw.readFile(path) ?? undefined;
    }

    deleteFile(path) {
        this.raw.deleteFile(path);
    }

    /** The diagnostics of `wowlua check`. */
    check() {
        this.tick();
        return JSON.parse(this.raw.check());
    }

    /**
     * `wowlua build --profile <profile>` without writing: `{ ok,
     * diagnostics, files: [{ path, text } | { path, base64 }] }`.
     */
    build(profile = "dev") {
        this.tick();
        return JSON.parse(this.raw.build(profile));
    }

    /** Formats a file in place: `{ text }`, or `{ diagnostics }`. */
    format(path) {
        return JSON.parse(this.raw.format(path));
    }

    /** The package as a link's fragment: `https://…/playground#${share()}`. */
    share() {
        return this.raw.share();
    }

    /** Opens a shared package (a link or its fragment); false if it is none. */
    openShared(link) {
        return this.raw.openShared(link);
    }

    /**
     * The language server, as an editor's client talks to it: `send` takes
     * a JSON-RPC message (an object), and `onMessage` gets the server's.
     * Checks run on this thread when due (a Web Worker keeps the page
     * responsive: see `worker.js`).
     */
    languageServer(onMessage) {
        let timer = null;
        const deliver = (json) => {
            for (const m of JSON.parse(json)) onMessage(m);
        };
        const schedule = () => {
            if (timer !== null) clearTimeout(timer);
            timer = null;
            const wait = this.raw.lspWait();
            if (wait < 0) return;
            timer = setTimeout(() => {
                timer = null;
                this.tick();
                deliver(this.raw.lspRunDue());
                schedule();
            }, Math.ceil(wait));
        };
        return {
            send: (message) => {
                this.tick();
                deliver(this.raw.lspMessage(JSON.stringify(message)));
                schedule();
            },
            close: () => {
                if (timer !== null) clearTimeout(timer);
                timer = null;
            },
        };
    }
}
