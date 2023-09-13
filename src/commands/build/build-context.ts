import { SourceFile } from "./source-file";

export interface BuildContext {
    sourceFiles: SourceFile[],
    libFiles: SourceFile[],
    outputFiles?: SourceFile[],
}