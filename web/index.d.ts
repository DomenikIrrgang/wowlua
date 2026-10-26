// Types of @wowlua/web (index.js) and @wowlua/web/client (client.js).

/** A position as the Language Server Protocol has it: from 0, UTF-16 columns. */
export interface Position {
    line: number;
    character: number;
}

export interface Range {
    start: Position;
    end: Position;
}

export interface Diagnostic {
    severity: "error" | "warning" | "note";
    /** wowlua's code, such as "E2001"; null for none. */
    code: string | null;
    message: string;
    /** The file, relative to the package; null for the package as a whole. */
    file: string | null;
    range: Range;
}

export type BuiltFile = { path: string; text: string } | { path: string; base64: string };

export interface Built {
    ok: boolean;
    diagnostics: Diagnostic[];
    /** The AddOn folder's files; `base64` (URL-safe) for one that is no text. */
    files: BuiltFile[];
}

export type Formatted = { text: string } | { diagnostics: Diagnostic[] };

/** What the running addon printed (`print`) or raised. */
export interface Output {
    kind: "print" | "error";
    text: string;
    stack?: string;
}

export interface RunOptions {
    /** The flavor to run (default "camelot", the one the playground has Blizzard's UI for). */
    flavor?: string;
    /** Load Blizzard's UI first (`loadBlizzardUI` fetched it). */
    blizzardUI?: boolean;
    /** A world profile's JSON text (`tests/profiles/<name>.json`'s). */
    profile?: string;
    /** Debug the run (through the worker, a cross-origin isolated page). */
    debug?: DebugSetup | true;
}

export interface DebugSetup {
    /** Line breakpoints by the package's source path (`src/main.wowlua`). */
    breakpoints?: Record<string, { line: number; condition?: string; hitCondition?: string; logMessage?: string }[]>;
    /** `Name`, `Table:method`, `event:PLAYER_LOGIN`, `script:OnClick`, `timer`, … */
    functionBreakpoints?: { name: string; condition?: string }[];
    /** "uncaught", "all", "taint", "blocked". */
    exceptions?: string[];
    stopOnEntry?: boolean;
}

export interface Started {
    ok: boolean;
    diagnostics: Diagnostic[];
    error?: string;
}

/** Input as the user gives it; x and y are screen pixels from the top left. */
export type Input =
    | { action: "click"; x?: number; y?: number; frame?: string; button?: "LeftButton" | "RightButton" | "MiddleButton" }
    | { action: "hover"; x?: number; y?: number; frame?: string }
    | { action: "type"; text: string }
    | { action: "key"; key: string }
    | { action: "slash"; text: string };

/** A canvas the running addon is drawn on. */
export interface Screen {
    /** "BrowserWebGpu" or "Gl". */
    readonly backend: string;
    resize(width: number, height: number): void;
}

/** Draws on a canvas (an HTMLCanvasElement or, in a worker, an OffscreenCanvas). */
export function createScreen(canvas: HTMLCanvasElement | OffscreenCanvas): Promise<Screen>;

/** A JSON-RPC message of the Language Server Protocol. */
export type LspMessage = Record<string, unknown>;

export interface LanguageServer {
    send(message: LspMessage): void;
    close(): void;
}

/** Loads the WebAssembly module once (by default the file next to index.js). */
export function load(wasm?: string | URL | Response | BufferSource | WebAssembly.Module): Promise<unknown>;

/** wowlua's version, once loaded. */
export function version(): string;

/** A package in memory (one per page or worker), and its language server. */
export class Playground {
    constructor();
    /** Passes the clocks in; the other methods call it where they need it. */
    tick(): void;
    files(): Record<string, string>;
    reset(files: Record<string, string>): void;
    writeFile(path: string, text: string): void;
    readFile(path: string): string | undefined;
    deleteFile(path: string): void;
    check(): Diagnostic[];
    build(profile?: string): Built;
    format(path: string): Formatted;
    share(): string;
    openShared(link: string): boolean;
    languageServer(onMessage: (message: LspMessage) => void): LanguageServer;
    /**
     * Fetches a flavor's Blizzard UI code (from jsDelivr by default, or
     * `base`): the startup files before it resolves, the rest in `later`.
     */
    loadBlizzardUI(options?: {
        flavor?: string;
        base?: string;
        concurrency?: number;
        onProgress?: (done: number, total: number) => void;
        fetch?: typeof fetch;
    }): Promise<{ flavor: string; commit: string; later: Promise<void> }>;
    run(options?: RunOptions): Started;
    stop(): void;
    readonly running: boolean;
    /** Moves the game to now; what it printed and raised. */
    advance(): Output[];
    input(input: Input): Output[];
    /** The running addon's screen in pixels: [width, height]. */
    screenSize(): [number, number];
    /** Advances and draws each animation frame until `stop()`. */
    play(screen: Screen, onOutput?: (output: Output) => void): { stop(): void };
}

export type PlaygroundCall =
    | "files" | "reset" | "writeFile" | "readFile" | "deleteFile"
    | "check" | "build" | "format" | "share" | "openShared" | "version"
    | "loadBlizzardUI" | "run" | "stop" | "input" | "screenSize"
    /** The game's screen as a PNG Blob (`call("snapshot", scale)`). */
    | "snapshot";

/** The page's side of worker.js (from "@wowlua/web/client"). */
export interface Connection {
    call(method: PlaygroundCall, ...args: unknown[]): Promise<any>;
    lsp: {
        send(message: LspMessage): void;
        /** Returns a function that stops listening. */
        onMessage(listen: (message: LspMessage) => void): () => void;
    };
    /** The debugger (a cross-origin isolated page only). */
    debug: {
        readonly available: boolean;
        /**
         * A request of wowlua's debugger protocol (`stackTrace`, `scopes`,
         * `variables`, `evaluate`, `setVariable`, `setBreakpoints`,
         * `setFunctionBreakpoints`, `setExceptionBreakpoints`, `continue`,
         * `next`, `stepIn`, `stepOut`, `pause`, `frameTree`, `findFrames`,
         * `fireEvent`, `profileStart`, `profileStop`): its response.
         */
        request(command: string, args?: Record<string, unknown>): Promise<any>;
        /** `{ event: "stopped" | "breakpoint" | "terminated", … }`. */
        onEvent(listen: (event: Record<string, any>) => void): () => void;
    };
    /** What the running addon prints and raises. */
    onOutput(listen: (output: Output) => void): () => void;
    /** Progress while Blizzard's UI loads. */
    onProgress(listen: (done: number, total: number) => void): () => void;
    /**
     * Draws the game on `canvas` (set its width and height first) and
     * passes its mouse and keyboard to the game; the backend that draws.
     */
    attachCanvas(canvas: HTMLCanvasElement): Promise<string>;
    /** The canvas's new drawing size. */
    resize(width: number, height: number): void;
    terminate(): void;
}

export function connect(worker: Worker, options?: { wasm?: string | URL }): Connection;
