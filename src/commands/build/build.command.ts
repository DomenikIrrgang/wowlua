import { Command } from "cli-program-lib/decorators/command.decorator";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { Logger } from "cli-program-lib/logging/logger";
import { WowluaConfig } from "../../config/wowlua.config";
import * as chokidar from "chokidar"
import * as fs from "fs"
import { GameVersion } from "../../util/game-version";
import { INTERFACE_VERSION } from "../../util/interface-version";
import { TOC_NAME } from "../../util/toc-names";
import { SourceFile } from "./source-file";
import { writeFileSyncRecursive } from "../../util/file-write-sync";
import { FileGameVersionMacro } from "./macros/file-game-version.macro";
import { GameVersionMacro } from "./macros/game-version.macro";
import { Macro } from "./macro";
import * as luaparse from "luaparse"
import { JsonParser } from "cli-program-lib/properties/json.parser"
import { GLOBALS_FILE } from "../../util/wowglobals/globals";
import * as dependencyGraph from "dependency-graph"
import { SkipMacro } from "./macros/skip.macro";
import { GLOBAL_LUA_FUNCTIONS } from "../../util/global-lua-function";
import { readDirSyncRecursive } from "../../util/ready-directory-recursive";


@Command({
    name: "build",
    description: "Build the project."
})
export class BuildCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    @Inject(JsonParser)
    private jsonParser: JsonParser

    private globals = {
        [GameVersion.CLASSIC]: {},
        [GameVersion.RETAIL]: {},
        [GameVersion.WOTLK]: {}
    }

    private macros = {
        FileGameVersion: new FileGameVersionMacro(),
        GameVersion: new GameVersionMacro(),
        Skip: new SkipMacro(),
    }

    public async execute(args: String[]): Promise<void> {
        const watch: boolean = args.includes("--watch")
        this.globals[GameVersion.CLASSIC] = this.jsonParser.parseFile(__dirname + "/../../../" + GLOBALS_FILE.get(GameVersion.CLASSIC))
        this.globals[GameVersion.RETAIL] = this.jsonParser.parseFile(__dirname + "/../../../" + GLOBALS_FILE.get(GameVersion.RETAIL))
        this.globals[GameVersion.WOTLK] = this.jsonParser.parseFile(__dirname + "/../../../" + GLOBALS_FILE.get(GameVersion.WOTLK))
        this.build()
        if (watch) {
            this.logger.info("Watching for file changes in path: " + this.config.getSrcPath())
            let rebuild = () => {
                console.clear()
                this.logger.info("File change detected re-building project...")
                this.build()
                this.logger.info("Watching for file changes in path: " + this.config.getSrcPath())
            }
            chokidar.watch(this.config.getSrcPath(), { ignoreInitial: true }).on("change", rebuild).on("add", rebuild).on("unlink", rebuild).on("unlinkDir", rebuild)
        }
    }


    public build(): void {
        this.logger.info("Building project...")
        let startTime = performance.now()
        let sourceFiles = this.scanSourceFiles()
        let libFiles = this.scanLibFIles()
        try {
            this.logger.debug("Parsing source files...")
            this.parseSourceFiles(sourceFiles)
            this.parseSourceFiles(libFiles)
            this.parseDecorators(sourceFiles)
            this.parseDecorators(libFiles)
            this.generateAsts(sourceFiles)
            this.generateAsts(libFiles)
            this.deleteBuildDirectory()
            this.createBuildDirectory()
            this.config.gameVersions.forEach((gameVersion: GameVersion) => {
                this.logger.debug("Building for game version: " + gameVersion)
                this.generateTocFile(gameVersion)
                let dependencies = this.generateDependencies(sourceFiles, libFiles)
                this.detectUnknownGlobals(dependencies)
                this.generateImportFile(gameVersion, dependencies)
                this.generateSourceFiles(gameVersion, dependencies)
            })
            let endTime = performance.now()
            this.logger.info("Finished building project. (" + (endTime - startTime).toFixed(2) + "ms)")
        } catch (error) {
            this.logger.error("Failed to build project: " + error.message)
        }
    }

    public scanLibFIles(): SourceFile[] {
        this.logger.debug("Scanning lib files...")
        let sourceFiles: SourceFile[] = []
        if (fs.existsSync(this.config.getLibPath()) === true) {
            let files = readDirSyncRecursive(this.config.getLibPath())
            for (let file of files) {
                const fileName = file
                if (fileName.endsWith(".lua")) {
                    let fileContent = fs.readFileSync(file, "utf-8")
                    fileContent = fileContent.replace(/\r\n/g, "\n")
                    let fileLines = fileContent.split("\n")
                    sourceFiles.push({
                        path: this.config.getLibPath(),
                        fileName: fileName.replace(this.config.getLibPath() + "/", ""),
                        originalCode: fileLines,
                        gameVersionSpecific: false,
                        parsedCode: [],
                        usedGlobals: [],
                        declaredGlobals: [],
                        importedGlobals: []
                    })
                }
            }
        }
        return sourceFiles
    }

    public detectUnknownGlobals(sourceFiles: SourceFile[]): void {
        this.logger.debug("Detecting missing globals...")
        for (let sourceFile of sourceFiles) {
            for (let importedGlobal of sourceFile.importedGlobals) {
                let found = false
                for (let otherSourceFile of sourceFiles) {
                    if (otherSourceFile.declaredGlobals.includes(importedGlobal)) {
                        found = true
                        break
                    }
                }
                if (found === false) {
                    this.logger.warning("Using potentially undefined global: " + importedGlobal + " in file: " + sourceFile.path + "/" + sourceFile.fileName + "")
                }
            }
        }
    }

    public generateAsts(sourceFiles: SourceFile[]): boolean {
        this.logger.debug("Generating ASTs...")
        for (let sourceFile of sourceFiles) {
            try {
                sourceFile.ast = luaparse.parse(sourceFile.parsedCode.join("\n"), {
                    locations: true,
                    scope: true,
                    ranges: true
                })
                this.calculateGlobals(sourceFile)
            } catch (error) {
                if (error.name === "SyntaxError") {
                    this.logger.error("Syntax error in file: " + sourceFile.path + " at line: " + error.line + " column: " + error.column + " message: " + error.message)
                } else {
                    this.logger.error("Error in file: " + sourceFile.path + " message: " + error.message)
                }
                return false
            }
        }
        return true
    }

    public calculateGlobals(sourceFile: SourceFile): void {
        this.logger.debug("Calculating used globals in file", sourceFile.fileName)
        sourceFile.usedGlobals = sourceFile.ast.globals.map((global) => global.name)
        sourceFile.declaredGlobals = sourceFile.ast.body.filter((node) => {
            return (node.type === "AssignmentStatement" && node.variables[0].isLocal === false) || (node.type === "FunctionDeclaration" && node.identifier.isLocal === false && node.identifier.type === "Identifier")
        }).map((node) => {
            if (node.type === "AssignmentStatement") {
                return node.variables[0].name
            } else if (node.type === "FunctionDeclaration") {
                return node.identifier.name
            }
        })
        sourceFile.importedGlobals = sourceFile.usedGlobals
            .filter((global) => this.globals[GameVersion.WOTLK][global] === undefined)
            .filter((global) => GLOBAL_LUA_FUNCTIONS.includes(global) === false)
            .filter((global) => sourceFile.declaredGlobals.includes(global) === false)
    }

    public parseSourceFiles(sourceFiles: SourceFile[]): void {
        this.logger.debug("Parsing source files...")
        for (let i = 0; i < sourceFiles.length; i++) {
            this.parseSourceFile(sourceFiles, sourceFiles[i])
        }
    }

    public parseDecorators(sourceFiles: SourceFile[]): void {
        this.logger.debug("Parsing decorators...")
        for (let i = 0; i < sourceFiles.length; i++) {
            let sourceFile = sourceFiles[i]
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
    }

    public parseSourceFile(sourceFiles: SourceFile[], sourceFile: SourceFile): void {
        sourceFile.parsedCode = []
        for (let lineNumber = 0; lineNumber < sourceFile.originalCode.length; lineNumber++) {
            let line = sourceFile.originalCode[lineNumber].replace("\n", "").replace("\r", "")
            if (line.startsWith("--@")) {
                let flag = line.split(" ")[0].replace("--@", "")
                let args = line.split(" ").slice(1)
                if (this.macros[flag] !== undefined) {
                    let parsedCode = (this.macros[flag] as Macro).parse(sourceFiles, sourceFile, args, lineNumber, lineNumber)
                    sourceFile.parsedCode = sourceFile.parsedCode.concat(parsedCode)
                }
            } else if (line.startsWith("--{@")) {
                let flag = line.split(" ")[0].replace("--{@", "")
                let args = line.split(" ").slice(1)
                let lineNumberEnd = lineNumber
                let flagCounter = 0;
                for (let i = lineNumber + 1; i < sourceFile.originalCode.length; i++) {
                    let line = sourceFile.originalCode[i].replace("\n", "").replace("\r", "")
                    if (line.startsWith("--{@")) {
                        let nestedFlag = line.split(" ")[0].replace("--{@", "")
                        if (nestedFlag === flag) {
                            flagCounter++
                        }
                    }
                    if (line.startsWith("--}@" + flag)) {
                        if (flagCounter === 0) {
                            lineNumberEnd = i
                            break
                        } else {
                            flagCounter--
                        }
                    }
                }
                if (this.macros[flag] !== undefined) {
                    let parsedCode = (this.macros[flag] as Macro).parse(sourceFiles, sourceFile, args, lineNumber, lineNumberEnd)
                    sourceFile.parsedCode = sourceFile.parsedCode.concat(parsedCode)
                }
                lineNumber = lineNumberEnd
            } else {
                sourceFile.parsedCode.push(line)
            }
        }
    }

    public generateSourceFiles(gameVersion: GameVersion, sourceFiles: SourceFile[]): void {
        this.logger.debug("Generating source files...")
        for (let sourceFile of sourceFiles) {
            writeFileSyncRecursive(this.config.getBuildPath() + "/src/" + sourceFile.fileName, sourceFile.parsedCode.join("\n"))
        }
    }

    public deleteBuildDirectory(): void {
        if (fs.existsSync(this.config.getBuildPath())) {
            this.logger.debug("Clearing build directory...")
            fs.rmSync(this.config.getBuildPath(), { recursive: true })
        }
    }

    public generateImportFile(gameVersion: GameVersion, sourceFiles: SourceFile[]): void {
        this.logger.debug("Generating import file...")
        let importContent = `<Ui xmlns="http://www.blizzard.com/wow/ui/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.blizzard.com/wow/ui/ ..\\FrameXML\\UI.xsd">\n`
        for (let sourceFile of sourceFiles) {
            if (sourceFile.gameVersionSpecific === false || (sourceFile.gameVersionSpecific && sourceFile.gameVersions.includes(gameVersion))) {
                importContent += `    <Script file="src/${sourceFile.fileName}" />\n`
            }
        }
        importContent += "</Ui>"
        Bun.write(this.config.getBuildPath() + "/source" + TOC_NAME.get(gameVersion) + ".xml", importContent)
    }

    public generateDependencies(sourceFiles: SourceFile[], libFiles: SourceFile[]): SourceFile[] {
        this.logger.debug("Generating dependency graph...")
        let graph = new dependencyGraph.DepGraph()
        for (let sourceFile of sourceFiles) {
            graph.addNode(sourceFile.path + "/" + sourceFile.fileName, sourceFile)
        }
        for (let sourceFile of sourceFiles) {
            for (let importedGlobal of sourceFile.importedGlobals) {
                let importedSourceFile = sourceFiles.find(sourceFile => sourceFile.declaredGlobals.includes(importedGlobal)) || libFiles.find(sourceFile => sourceFile.declaredGlobals.includes(importedGlobal))
                if (importedSourceFile !== undefined) {
                    // Add source file from lib folder to graph
                    if (!graph.hasNode(importedSourceFile.path + "/" + importedSourceFile.fileName)) {
                        graph.addNode(importedSourceFile.path + "/" + importedSourceFile.fileName, importedSourceFile)
                    }
                    graph.addDependency(sourceFile.path + "/" + sourceFile.fileName, importedSourceFile.path + "/" + importedSourceFile.fileName)
                }
            }
        }
        return graph.overallOrder().map((node) => (graph.getNodeData(node) as SourceFile))
    }

    public createBuildDirectory(): void {
        this.logger.debug("Creating build directory...")
        fs.mkdirSync(this.config.getBuildPath())
    }

    public generateTocFiles(): void {
        this.logger.debug("Generating toc files...")
        this.config.gameVersions.forEach((gameVersion: GameVersion) => {
            this.generateTocFile(gameVersion)
        })
    }

    public generateTocFile(gameVersion: GameVersion): void {
        this.logger.debug("Generating toc file for game version: " + gameVersion)
        let tocContent = `## Interface: ${INTERFACE_VERSION.get(gameVersion)}\n## Title: ${this.config.name}\n## Notes: ${this.config.description}\n## Author: ${this.config.author}\n## Version: ${this.config.version}\n${(this.config.savedVariables !== "") ? "## SavedVariables: " + this.config.savedVariables + "\n" : "\n"}            `
        tocContent += "\nsource" + TOC_NAME.get(gameVersion) + ".xml"
        Bun.write(this.config.getBuildPath() + "/" + this.config.name + "_" + TOC_NAME.get(gameVersion) + ".toc", tocContent)
    }

    public scanSourceFiles(): SourceFile[] {
        this.logger.debug("Scanning source files...")
        let sourceFiles: SourceFile[] = []
        let files = readDirSyncRecursive(this.config.getSrcPath())
        for (let file of files) {
            const fileName = file
            if (fileName.endsWith(".lua")) {
                let fileContent = fs.readFileSync(file, "utf-8")
                fileContent = fileContent.replace(/\r\n/g, "\n")
                let fileLines = fileContent.split("\n")
                sourceFiles.push({
                    path: this.config.getSrcPath(),
                    fileName: fileName.replace(this.config.getSrcPath() + "/", ""),
                    originalCode: fileLines,
                    gameVersionSpecific: false,
                    parsedCode: [],
                    usedGlobals: [],
                    declaredGlobals: [],
                    importedGlobals: []
                })
            }
        }
        return sourceFiles
    }

}