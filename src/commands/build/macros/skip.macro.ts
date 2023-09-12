import { GameVersion } from "../../../util/game-version";
import { Macro } from "../macro";
import { SourceFile } from "../source-file";

export class SkipMacro implements Macro {

    public parse(sourceFiles: SourceFile[], sourceFile: SourceFile, args: string[], lineNumberStart: number, lineNumberEnd: number): string[] {
        sourceFiles.splice(sourceFiles.indexOf(sourceFile), 1)
        return []
    }

}