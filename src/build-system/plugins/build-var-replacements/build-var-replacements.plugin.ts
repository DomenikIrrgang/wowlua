import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { BuildPlugin } from "../build-plugin";
import { BuildContext } from "../../build-context";
import { SourceFile } from "../../source-file";
import { GameVersion } from "../../../util/game-version";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { WowluaConfig } from "../../../config/wowlua.config";


@Injectable()
export class BuildVarReplacementPlugin extends BuildPlugin {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    public parse(buildContext: BuildContext, sourceFile: SourceFile): void {
        for (let lineNumber = 0; lineNumber < sourceFile.parsedCode.length; lineNumber++) {
            let line = sourceFile.parsedCode[lineNumber].replace("\n", "").replace("\r", "")
            let startPosition = line.indexOf("@")
            if (startPosition !== -1) {
                let endPosition = line.indexOf("@", startPosition + 1)
                if (endPosition !== -1) {
                    let variable = line.substring(startPosition + 1, endPosition)
                    let replacement = this.getReplacement(buildContext, variable)
                    if (replacement === undefined) {
                        throw new Error("Could not find replacement for variable: " + variable + " in file: " + sourceFile.fileName + " at line: " + lineNumber)
                    }
                    if (typeof replacement === "string") {
                        replacement = "\"" + replacement + "\""
                    }
                    sourceFile.parsedCode[lineNumber] = line.substring(0, startPosition) + replacement + line.substring(endPosition + 1)
                    console.log(sourceFile.parsedCode[lineNumber])
                }
            }
        }
    }

    public build(buildContext: BuildContext, gameVersion: GameVersion): void {}
    public ast(sourceFile: SourceFile, node): void {}

    private getReplacement(buildContext: BuildContext, variable: string): string | number | boolean {
        if (buildContext.variables[variable] !== undefined) {
            return buildContext.variables[variable]
        }
    }
}