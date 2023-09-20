import { Chunk } from "luaparse";
import { GameVersion } from "../util/game-version";
import { Variable } from "./variable";

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
    ast?: any,
    variables: { [name: string]: Variable },
}