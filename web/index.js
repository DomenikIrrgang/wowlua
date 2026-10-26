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

import init, { Playground as Raw, createScreen as rawScreen } from "./pkg/wowlua_web.js";

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

/**
 * A canvas the running addon is drawn on (an HTMLCanvasElement, or an
 * OffscreenCanvas in a worker): WebGPU where the browser has it, else
 * WebGL 2. `screen.backend` says which.
 */
export function createScreen(canvas) {
    return rawScreen(canvas);
}

/** Fetches `urls` with at most `limit` at once; `each(url, response)`. */
async function fetchAll(items, limit, each) {
    let next = 0;
    const worker = async () => {
        while (next < items.length) {
            const item = items[next++];
            await each(item);
        }
    };
    await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
}

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
     * Fetches a flavor's Blizzard UI code into the playground: what loading
     * it reads first (about 2,400 files, 18 MB for camelot; the browser
     * caches them), then, in the background, the code of the addons it
     * loads on demand (`later`, a promise). From jsDelivr's copy of
     * Gethe/wow-ui-source at the commit wowlua's declarations name, unless
     * `base` names another place with the same paths.
     */
    async loadBlizzardUI({ flavor = "camelot", base, concurrency = 48, onProgress, fetch: get = fetch } = {}) {
        const json = Raw.blizzardUI(flavor);
        if (json === undefined) throw new Error(`no Blizzard UI for ${flavor} in the playground`);
        const ui = JSON.parse(json);
        const from = base ?? ui.base;
        const total = ui.startup.length + ui.later.length;
        let done = 0;
        const one = async (path) => {
            const at = `${ui.root}/${path}`;
            if (!this.raw.hasHostFile(at)) {
                const response = await get(from + path);
                // (a file a TOC names that is not there: the game skips it too)
                if (response.ok) this.raw.writeHostFile(at, new Uint8Array(await response.arrayBuffer()));
            }
            done++;
            onProgress?.(done, total);
        };
        if (ui.strings && !this.raw.hasHostFile(ui.strings.path)) {
            const response = await get(ui.strings.url);
            if (!response.ok) throw new Error(`the global strings: ${response.status}`);
            this.raw.writeHostFile(ui.strings.path, new Uint8Array(await response.arrayBuffer()));
        }
        await fetchAll(ui.startup, concurrency, one);
        const later = fetchAll(ui.later, Math.max(1, concurrency >> 2), one);
        return { flavor, commit: ui.commit, later };
    }

    /**
     * Builds the package and runs it: `{ flavor = "camelot", blizzardUI =
     * false, profile, debug }` (`profile`: a world profile's JSON text;
     * `debug`: the debugger's breakpoints, for a worker with a mailbox, see
     * client.js). `{ ok, diagnostics, error }`.
     */
    run({ flavor = "camelot", blizzardUI = false, profile, debug } = {}) {
        this.tick();
        return JSON.parse(this.raw.run(JSON.stringify({ flavor, blizzardUI, profile, debug })));
    }

    stop() {
        this.raw.stop();
    }

    get running() {
        return this.raw.running;
    }

    /** Moves the game to now: what it printed and raised (`{ kind, text, stack }`). */
    advance() {
        this.tick();
        return JSON.parse(this.raw.advance(now()));
    }

    /**
     * Input as the user gives it: `{ action: "click", x, y, button }`,
     * `{ action: "hover", x, y }`, `{ action: "type", text }`,
     * `{ action: "key", key }`, `{ action: "slash", text }`; screen pixels
     * from the top left. What it printed and raised.
     */
    input(input) {
        return JSON.parse(this.raw.input(JSON.stringify(input)));
    }

    /** The running addon's screen size in pixels: `[width, height]`. */
    screenSize() {
        return Array.from(this.raw.screenSize());
    }

    /**
     * Runs the game on `screen` each animation frame until `stop()` is
     * called on what this returns; `onOutput` gets what the addon prints
     * and raises.
     */
    play(screen, onOutput = () => {}) {
        const frame = typeof requestAnimationFrame === "function"
            ? requestAnimationFrame
            : (f) => setTimeout(f, 1000 / 60);
        let on = true;
        const step = () => {
            if (!on) return;
            if (this.raw.running) {
                for (const o of this.advance()) onOutput(o);
                screen.draw(this.raw);
            }
            frame(step);
        };
        frame(step);
        return { stop: () => { on = false; } };
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
