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
//
// Debugging (a cross-origin isolated page: `wowlua.debug.available`):
//   wowlua.debug.onEvent((e) => …);                  // { event: "stopped", reason, … }
//   await wowlua.call("run", { debug: { breakpoints: { "src/main.wowlua": [{ line: 6 }] } } });
//   const { frames } = await wowlua.debug.request("stackTrace");
//   await wowlua.debug.request("continue");

export function connect(worker, { wasm } = {}) {
    let next = 1;
    const waiting = new Map();
    const listeners = new Set();
    const outputs = new Set();
    const progress = new Set();
    const debugEvents = new Set();
    const debugWaiting = new Map();
    let nextDebug = 1;
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
        if (m.debug !== undefined) {
            const d = m.debug;
            const pending = d.id !== undefined && debugWaiting.get(d.id);
            if (pending) {
                debugWaiting.delete(d.id);
                if (d.error !== undefined) pending.reject(new Error(d.error));
                else pending.resolve(d.response);
            } else if (d.event !== undefined) {
                for (const listen of debugEvents) listen(d);
            }
            return;
        }
        const pending = waiting.get(m.id);
        if (!pending) return;
        waiting.delete(m.id);
        if (m.error !== undefined) pending.reject(new Error(m.error));
        else pending.resolve(m.result);
    });
    if (wasm !== undefined) worker.postMessage({ wasm: String(wasm) });
    // The debugger's mailbox: written here, waited on in the worker.
    const isolated = typeof SharedArrayBuffer !== "undefined" && globalThis.crossOriginIsolated === true;
    const mailbox = isolated ? new SharedArrayBuffer(8 + (1 << 20)) : null;
    const control = mailbox && new Int32Array(mailbox, 0, 2);
    const bytes = mailbox && new Uint8Array(mailbox, 8);
    const outbox = [];
    const encoder = new TextEncoder();
    let pumping = false;
    const pump = () => {
        if (pumping) return;
        while (outbox.length > 0) {
            if (Atomics.load(control, 0) !== 0) {
                // the worker has not taken the last one yet
                pumping = true;
                const again = () => { pumping = false; pump(); };
                const waiting = Atomics.waitAsync?.(control, 0, 1);
                if (waiting?.async) waiting.value.then(again);
                else setTimeout(again, waiting ? 0 : 2);
                return;
            }
            const data = encoder.encode(outbox.shift());
            bytes.set(data.subarray(0, bytes.length));
            Atomics.store(control, 1, Math.min(data.length, bytes.length));
            Atomics.store(control, 0, 1);
            Atomics.notify(control, 0);
        }
    };
    if (mailbox) worker.postMessage({ debugMailbox: mailbox });
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
        debug: {
            /** Whether the page can debug (it is cross-origin isolated). */
            available: isolated,
            /**
             * A request of the debugger's protocol (`stackTrace`, `scopes`,
             * `variables`, `evaluate`, `continue`, `next`, `stepIn`,
             * `stepOut`, `pause`, `setBreakpoints`, `frameTree`, …): its
             * response.
             */
            request(command, args = {}) {
                if (!mailbox) return Promise.reject(new Error("debugging needs a cross-origin isolated page"));
                const id = nextDebug++;
                return new Promise((resolve, reject) => {
                    debugWaiting.set(id, { resolve, reject });
                    outbox.push(JSON.stringify({ id, command, ...args }));
                    pump();
                });
            },
            /**
             * Its events: `stopped` (`reason`: breakpoint, step, pause,
             * exception, …), `breakpoint` (one was placed when its file
             * loaded), `terminated`. The answer to `continue` or a step
             * means the game runs on.
             */
            onEvent(listen) {
                debugEvents.add(listen);
                return () => debugEvents.delete(listen);
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
