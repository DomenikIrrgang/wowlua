import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { WowluaConfig } from "../config/wowlua.config";
import { Logger } from "cli-program-lib/logging/logger";
import { JsonParser } from "cli-program-lib/properties/json.parser";
import { Performance } from "../util/performance";
import { BuildContext } from "./build-context";
import { GameVersion } from "../util/game-version";
import { BuildPlugin } from "./plugins/build-plugin";
import { CompilerFlagsPlugin } from "./plugins/compiler-flags/compiler-flags.plugin";
import { DecoratorBuildPlugin } from "./plugins/decorator.plugin";
import { UnknownGlobalsPlugin } from "./plugins/unknown-globals.plugin";
import { injector } from "cli-program-lib/dependency-injection/injector";
import { parse } from "luaparse";
import { VariableAssignment, getVariableAssignments } from "../util/luaparse-util";
import { GLOBAL_LUA_FUNCTIONS } from "../util/global-lua-function";
import { SourceFile } from "./source-file";
import { writeFileSyncRecursive } from "../util/file-write-sync";
import * as fs from "fs"
import * as DependencyGraph from "dependency-graph"
import { INTERFACE_VERSION } from "../util/interface-version";
import { TOC_NAME } from "../util/toc-names";
import { readDirSyncRecursive } from "../util/ready-directory-recursive";
import { GLOBALS_FILE } from "../util/wowglobals/globals";

@Injectable()
export class BuildSystem {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    @Inject(Performance)
    private performance: Performance

    @Inject(JsonParser)
    private jsonParser: JsonParser

    private globals = {
        [GameVersion.CLASSIC]: {},
        [GameVersion.RETAIL]: {},
        [GameVersion.WOTLK]: {}
    }

    private buildPlugins: BuildPlugin[] = [
        injector.getInstance(CompilerFlagsPlugin),
        injector.getInstance(DecoratorBuildPlugin),
        injector.getInstance(UnknownGlobalsPlugin)
    ]

    public init(): void {
        this.globals[GameVersion.CLASSIC] = this.jsonParser.parseFile(__dirname + "/../../" + GLOBALS_FILE.get(GameVersion.CLASSIC))
        this.globals[GameVersion.RETAIL] = this.jsonParser.parseFile(__dirname + "/../../" + GLOBALS_FILE.get(GameVersion.RETAIL))
        this.globals[GameVersion.WOTLK] = this.jsonParser.parseFile(__dirname + "/../../" + GLOBALS_FILE.get(GameVersion.WOTLK))
    }

    public build(): void {
        this.logger.info("Building project...")
        this.performance.measureStart("Build")
        let buildContext: BuildContext = this.generateBuildContext()
        try {
            if (buildContext.sourceFiles.length === 0) {
                throw new Error("No source files found in path: " + this.config.getSrcPath())
            }

            // Parse all source files for metadata and transform into valid lua code
            this.performance.measureStart("ParseBuildContext")
            this.parseBuildContext(buildContext)
            this.performance.measureEnd("ParseBuildContext")

            // Generate AST to enable code analysis
            // Note: luaparse expects valid lua and will fail otherwise
            this.performance.measureStart("GenerateAst")
            this.generateAsts(buildContext)
            this.performance.measureEnd("GenerateAst")

            // Clear and create the build directory
            this.deleteBuildDirectory()
            this.createBuildDirectory()

            // Build the project for all targeted game versions
            this.config.gameVersions.forEach((gameVersion: GameVersion) => {
                this.performance.measureStart("BuildGameVersion" + gameVersion)
                this.logger.debug("Building for game version: " + gameVersion)
                buildContext.outputFiles = this.generateDependencies(buildContext)
                this.performance.measureStart("BuildPlugins" + gameVersion)
                this.runBuildPlugins(buildContext, gameVersion)
                this.performance.measureEnd("BuildPlugins" + gameVersion)
                this.generateTocFile(gameVersion)
                this.generateImportFile(gameVersion, buildContext.outputFiles)
                this.generateSourceFiles(gameVersion, buildContext.outputFiles)
                this.performance.measureEnd("BuildGameVersion" + gameVersion)
            })
            this.performance.measureEnd("Build")
            this.logger.info(`Finished building project. (${this.performance.getMeasure("Build")}ms)`)
            this.performance.logMeasures()
        } catch (error) {
            this.logger.error("Failed to build project: " + error.message)
        }
    }

    public generateBuildContext(): BuildContext {
        let context: BuildContext = {
            sourceFiles: this.loadSourceFiles(this.config.getSrcPath()),
            libFiles: this.loadSourceFiles(this.config.getLibPath()),
        }
        return context
    }

    public parseBuildContext(buildContext: BuildContext): void {
        this.logger.debug("Parsing source files...")
        for (let buildPlugin of this.buildPlugins) {
            for (let sourceFile of buildContext.sourceFiles) {
                buildPlugin.parse(buildContext, sourceFile)
            }
            
            for (let sourceFile of buildContext.libFiles) {
                buildPlugin.parse(buildContext, sourceFile)
            }
        }
    }

    public generateAsts(buildContext: BuildContext): boolean {
        this.logger.debug("Generating ASTs...")
        let sourceFiles = buildContext.sourceFiles.concat(buildContext.libFiles)
        for (let sourceFile of sourceFiles) {
            try {
                sourceFile.ast = parse(sourceFile.parsedCode.join("\n"), {
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

    public runBuildPlugins(buildContext: BuildContext, gameVersion: GameVersion) {
        for (let plugin of this.buildPlugins) {
            plugin.build(buildContext, gameVersion)
        }
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

        let indexedGlobals = getVariableAssignments(sourceFile.ast)
        let resolvedGlobals = indexedGlobals
            .filter((assignment: VariableAssignment) => assignment.target === "_G" && (assignment.indexLookupType === "StringLiteral" || assignment.indexLookupType === "Identifier" || assignment.indexLookupType === "NumericLiteral"))
            .map((assignment: VariableAssignment) => {
                if (assignment.indexLookupType === "StringLiteral") {
                    return assignment.index
                } else if (assignment.indexLookupType === "Identifier") {
                    return indexedGlobals.find((findValue: VariableAssignment) => findValue.target === assignment.index)?.value
                } else if (assignment.indexLookupType === "NumericLiteral") {
                    return assignment.index
                }
            })
        sourceFile.declaredGlobals = sourceFile.declaredGlobals.concat(resolvedGlobals)
        
        sourceFile.importedGlobals = sourceFile.usedGlobals
            .filter((global) => this.globals[GameVersion.WOTLK][global] === undefined)
            .filter((global) => GLOBAL_LUA_FUNCTIONS.includes(global) === false)
            .filter((global) => sourceFile.declaredGlobals.includes(global) === false)
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

    public generateDependencies(buildContext: BuildContext): SourceFile[] {
        this.logger.debug("Generating dependency graph...")
        let graph = new DependencyGraph.DepGraph()
        for (let sourceFile of buildContext.sourceFiles) {
            graph.addNode(sourceFile.path + "/" + sourceFile.fileName, sourceFile)
        }
        for (let sourceFile of buildContext.sourceFiles) {
            for (let importedGlobal of sourceFile.importedGlobals) {
                let importedSourceFile = buildContext.sourceFiles.find(sourceFile => sourceFile.declaredGlobals.includes(importedGlobal)) || buildContext.libFiles.find(sourceFile => sourceFile.declaredGlobals.includes(importedGlobal))
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

    public generateTocFile(gameVersion: GameVersion): void {
        this.logger.debug("Generating toc file for game version: " + gameVersion)
        let tocContent = `## Interface: ${INTERFACE_VERSION.get(gameVersion)}\n## Title: ${this.config.name}\n## Notes: ${this.config.description}\n## Author: ${this.config.author}\n## Version: ${this.config.version}\n${(this.config.savedVariables !== "") ? "## SavedVariables: " + this.config.savedVariables + "\n" : "\n"}            `
        tocContent += "\nsource" + TOC_NAME.get(gameVersion) + ".xml"
        Bun.write(this.config.getBuildPath() + "/" + this.config.name + "_" + TOC_NAME.get(gameVersion) + ".toc", tocContent)
    }

    public loadSourceFiles(path: string): SourceFile[] {
        this.logger.debug(`Scanning for source files in ${path} ...`)
        let sourceFiles: SourceFile[] = []
        if (fs.existsSync(path) === true) {
            let files = readDirSyncRecursive(path)
            for (let file of files) {
                const fileName = file
                if (fileName.endsWith(".lua")) {
                    let fileContent = fs.readFileSync(file, "utf-8")
                    fileContent = fileContent.replace(/\r\n/g, "\n")
                    let fileLines = fileContent.split("\n")
                    sourceFiles.push({
                        path: path,
                        fileName: fileName.replace(path + "/", ""),
                        originalCode: fileLines,
                        parsedCode: JSON.parse(JSON.stringify(fileLines)),
                        compilerFlags: [],
                        gameVersions: [],
                        definedGlobals: [],
                        gameVersionSpecific: false,
                        usedGlobals: [],
                        declaredGlobals: [],
                        importedGlobals: []
                    })
                }
            }
        }
        return sourceFiles
    }

}