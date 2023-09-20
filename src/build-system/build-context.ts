import { SourceFile } from "./source-file";

export interface BuildContext {
    sourceFiles: SourceFile[],
    libFiles: SourceFile[],
    outputFiles?: SourceFile[],
    variables: { [key: string]: string | number | boolean },
}