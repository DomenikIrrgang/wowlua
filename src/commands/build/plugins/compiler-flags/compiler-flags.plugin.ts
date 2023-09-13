import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { GameVersion } from "../../../../util/game-version";
import { BuildContext } from "../../build-context";
import { SourceFile } from "../../source-file";
import { BuildPlugin } from "../build-plugin";
import { CompilerFlag } from "./flags/compilerflag";
import { FileGameVersionCompilerFlag } from "./flags/file-game-version.compilerflag";
import { SkipCompilerFlag } from "./flags/skip.compilerflag";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { Logger } from "cli-program-lib/logging/logger";

@Injectable()
export class CompilerFlagsPlugin implements BuildPlugin {

    @Inject(Logger)
    private logger: Logger

    private COMPILER_FLAG_PREFIX = "--$"

    private flags = {
        FileGameVersion: new FileGameVersionCompilerFlag(),
        Skip: new SkipCompilerFlag(),
    }

    public parse(buildContext: BuildContext, sourceFile: SourceFile): void {
        for (let lineNumber = 0; lineNumber < sourceFile.originalCode.length; lineNumber++) {
            let line = sourceFile.originalCode[lineNumber].replace("\n", "").replace("\r", "")
            if (line.startsWith(this.COMPILER_FLAG_PREFIX)) {
                let flag = line.split(" ")[0].replace(this.COMPILER_FLAG_PREFIX, "")
                let args = line.split(" ").slice(1)
                if (this.flags[flag] !== undefined) {
                    let parsedCode = (this.flags[flag] as CompilerFlag).parse(buildContext, sourceFile, args, lineNumber, lineNumber)
                    sourceFile.parsedCode = sourceFile.parsedCode.concat(parsedCode)
                }
            } else {
                sourceFile.parsedCode.push(line)
            }
        }
    }

    public build(buildContext: BuildContext, gameVersion: GameVersion): void {}

}