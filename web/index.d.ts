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
}

export type PlaygroundCall =
    | "files" | "reset" | "writeFile" | "readFile" | "deleteFile"
    | "check" | "build" | "format" | "share" | "openShared" | "version";

/** The page's side of worker.js (from "@wowlua/web/client"). */
export interface Connection {
    call(method: PlaygroundCall, ...args: unknown[]): Promise<any>;
    lsp: {
        send(message: LspMessage): void;
        /** Returns a function that stops listening. */
        onMessage(listen: (message: LspMessage) => void): () => void;
    };
    terminate(): void;
}

export function connect(worker: Worker, options?: { wasm?: string | URL }): Connection;
