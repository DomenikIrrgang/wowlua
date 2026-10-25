// The page's side of @wowlua/web's worker (worker.js): the playground's
// calls as promises, the language server as a message transport an
// editor's LSP client can use, and the game drawn on a canvas of the page
// with its mouse and keyboard.
//
//   import { connect } from "@wowlua/web/client";
//   const wowlua = connect(new Worker(new URL("@wowlua/web/worker", import.meta.url), { type: "module" }));
//   await wowlua.call("writeFile", "src/main.wowlua", text);
//   const diagnostics = await wowlua.call("check");
//   wowlua.lsp.onMessage((m) => …);  wowlua.lsp.send({ jsonrpc: "2.0", … });
//   await wowlua.call("loadBlizzardUI", { flavor: "camelot" });
//   await wowlua.attachCanvas(canvas);          // the game is drawn there
//   await wowlua.call("run", { blizzardUI: true });
//   wowlua.onOutput((o) => console.log(o.text));

export function connect(worker, { wasm } = {}) {
    let next = 1;
    const waiting = new Map();
    const listeners = new Set();
    const outputs = new Set();
    const progress = new Set();
    worker.addEventListener("message", (event) => {
        const m = event.data;
        if (m.lsp !== undefined) {
            for (const listen of listeners) listen(m.lsp);
            return;
        }
        if (m.output !== undefined) {
            for (const listen of outputs) listen(m.output);
            return;
        }
        if (m.progress !== undefined) {
            for (const listen of progress) listen(...m.progress);
            return;
        }
        const pending = waiting.get(m.id);
        if (!pending) return;
        waiting.delete(m.id);
        if (m.error !== undefined) pending.reject(new Error(m.error));
        else pending.resolve(m.result);
    });
    if (wasm !== undefined) worker.postMessage({ wasm: String(wasm) });
    const ask = (message, transfer = []) => {
        const id = next++;
        return new Promise((resolve, reject) => {
            waiting.set(id, { resolve, reject });
            worker.postMessage({ id, ...message }, transfer);
        });
    };
    // (the screen's size, asked once per run)
    let size = null;
    const call = (method, ...args) => {
        if (method === "run") size = null;
        return ask({ call: method, args });
    };
    const screen = async () => (size ??= await call("screenSize"));
    return {
        /** Calls a Playground method in the worker. */
        call,
        lsp: {
            send(message) {
                worker.postMessage({ lsp: message });
            },
            /** Returns a function that stops listening. */
            onMessage(listen) {
                listeners.add(listen);
                return () => listeners.delete(listen);
            },
        },
        /** What the running addon prints and raises (`{ kind, text, stack }`). */
        onOutput(listen) {
            outputs.add(listen);
            return () => outputs.delete(listen);
        },
        /** `(done, total)` while Blizzard's UI loads. */
        onProgress(listen) {
            progress.add(listen);
            return () => progress.delete(listen);
        },
        /**
         * Draws the game on `canvas` (its drawing size: set `width` and
         * `height` first) and passes its mouse and keyboard to the game.
         * Resolves to the backend that draws (`BrowserWebGpu`, `Gl`).
         */
        async attachCanvas(canvas) {
            const offscreen = canvas.transferControlToOffscreen();
            const backend = await ask({ canvas: offscreen }, [offscreen]);
            bindInput(canvas, call, screen);
            return backend;
        },
        /** The canvas's new drawing size. */
        resize(width, height) {
            worker.postMessage({ resize: [width, height] });
        },
        terminate() {
            worker.terminate();
        },
    };
}

/** The game's names of keys the browser names otherwise. */
const KEYS = {
    Enter: "ENTER", Escape: "ESCAPE", Tab: "TAB", Backspace: "BACKSPACE", " ": "SPACE",
    ArrowUp: "UP", ArrowDown: "DOWN", ArrowLeft: "LEFT", ArrowRight: "RIGHT",
    Delete: "DELETE", Home: "HOME", End: "END", PageUp: "PAGEUP", PageDown: "PAGEDOWN",
};

/** The canvas's mouse and keyboard as the game's input. */
function bindInput(canvas, call, screen) {
    const at = async (event) => {
        const [w, h] = await screen();
        const r = canvas.getBoundingClientRect();
        return { x: ((event.clientX - r.left) / r.width) * w, y: ((event.clientY - r.top) / r.height) * h };
    };
    const send = (input) => call("input", input).catch(() => {});
    let hovering = false;
    canvas.addEventListener("mousemove", async (event) => {
        if (hovering) return;
        hovering = true;
        requestAnimationFrame(() => (hovering = false));
        send({ action: "hover", ...(await at(event)) });
    });
    canvas.addEventListener("mouseup", async (event) => {
        const button = ["LeftButton", "MiddleButton", "RightButton"][event.button] ?? "LeftButton";
        send({ action: "click", button, ...(await at(event)) });
    });
    canvas.addEventListener("contextmenu", (event) => event.preventDefault());
    if (!canvas.hasAttribute("tabindex")) canvas.tabIndex = 0;
    canvas.addEventListener("keydown", (event) => {
        event.preventDefault();
        if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
            // into the edit box with the focus, else a key press
            call("input", { action: "type", text: event.key }).catch(() =>
                send({ action: "key", key: event.key.toUpperCase() }),
            );
        } else {
            send({ action: "key", key: KEYS[event.key] ?? event.key.toUpperCase() });
        }
    });
}
