import { GameVersion } from "../../util/game-version";
import { Chunk } from "luaparse";

export interface SourceFile {
    path: string,
    fileName: string,
    originalCode: string[],
    usedGlobals: string[],
    declaredGlobals: string[],
    importedGlobals: string[],
    gameVersionSpecific: boolean,
    gameVersions?: GameVersion[],
    parsedCode?: string[],
    definedGlobals?: string[],
    compilerFlags?: { lineNumberStart: number, lineNumberEnd: number, flag: string, args: string[] }[]
    ast?: any
}