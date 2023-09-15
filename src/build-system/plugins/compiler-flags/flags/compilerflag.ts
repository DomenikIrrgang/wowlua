import { BuildContext } from "../../../build-context";
import { SourceFile } from "../../../source-file";

export interface CompilerFlag {
    parse(
        buildContext: BuildContext,
        sourceFile: SourceFile,
        args: string[],
        lineNumberStart: number,
        lineNumberEnd: number,
    ): string
}