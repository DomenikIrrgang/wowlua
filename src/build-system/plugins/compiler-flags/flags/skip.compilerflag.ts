import { BuildContext } from "../../../build-context";
import { SourceFile } from "../../../source-file";
import { CompilerFlag } from "./compilerflag";

export class SkipCompilerFlag implements CompilerFlag {

    public parse(buildContext: BuildContext, sourceFile: SourceFile, args: string[], lineNumberStart: number, lineNumberEnd: number): string {
        if (args.length > 0) {
            throw new Error("Invalid number of arguments for Skip: " + args.length + " (expected 0)")
        }
        if (buildContext.sourceFiles.indexOf(sourceFile) !== -1) {
            buildContext.sourceFiles.splice(buildContext.sourceFiles.indexOf(sourceFile), 1)
        }
        if (buildContext.libFiles.indexOf(sourceFile) !== -1) {
            buildContext.libFiles.splice(buildContext.libFiles.indexOf(sourceFile), 1)
        }
        return ""
    }

}