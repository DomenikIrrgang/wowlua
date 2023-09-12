import { SourceFile } from "./source-file";

export interface Macro {
    parse(
        sourceFiles: SourceFile[],
        sourceFile: SourceFile,
        args: string[],
        lineNumberStart: number,
        lineNumberEnd: number,
    ): string[]
}

let test = new Map()