import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { BuildContext } from "../build-context";
import { SourceFile } from "../source-file";
import { BuildPlugin } from "./build-plugin";
import { GameVersion } from "../../util/game-version";

@Injectable()
export class DecoratorBuildPlugin implements BuildPlugin {

    public parse(buildContext: BuildContext, sourceFile: SourceFile): void {
        for (let lineNumber = 0; lineNumber < sourceFile.parsedCode.length; lineNumber++) {
            let line = sourceFile.parsedCode[lineNumber].replace("\n", "").replace("\r", "")
            if (line.startsWith("@")) {
                let decorator = line.replace("@", "").split("(")[0]
                let args = line.replace("@", "").split("(")[1]
                for (let j = lineNumber + 1; j < sourceFile.parsedCode.length; j++) {
                    if (sourceFile.parsedCode[j].startsWith("\n") || sourceFile.parsedCode[j].startsWith("\r")) {
                        throw new Error("Unexpected newline after decorator: " + decorator)
                    }
                    let line = sourceFile.parsedCode[j].replace("\n", "").replace("\r", "")
                    if (line.startsWith("function")) {
                        let className = line.split(" ")[1].split(":")[0]
                        let functionName = line.split(" ")[1].split(":")[1].split("(")[0]
                        sourceFile.parsedCode[lineNumber] = decorator + "(" + className + ", \"" + functionName + "\", " + args
                    }
                }
            }
        }
    }

    public build(buildContext: BuildContext, gameVersion: GameVersion): void {}
}