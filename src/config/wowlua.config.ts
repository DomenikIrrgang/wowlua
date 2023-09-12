import { Injectable } from "cli-program-lib/decorators/injectable.decorator"
import { Json } from "cli-program-lib/decorators/json.decorator"
import { GameVersion } from "../util/game-version"

export enum DependencyType {
    SVN = "svn",
    GIT = "git",
    LOCAL = "local"
}

export interface Dependency {
    name: string,
    source: string,
    type: DependencyType,
    version?: string,
    path?: string
}

@Injectable({
    unique: true
})
export class WowluaConfig {

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.name",
        default: "<default project name>"
    })
    public name: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.version",
        default: "<default version>"
    })
    public version: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.author",
        default: "<default author>"
    })
    public author: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.description",
        default: "<default description>"
    })
    public description: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.license",
        default: "MIT"
    })
    public license: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.repository",
        default: ""
    })
    public repository: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.keywords",
        default: []
    })
    public keywords: string[]

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.gameVersions",
        default: [ GameVersion.WOTLK ]
    })
    public gameVersions: GameVersion[]

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.dependencies",
        default: []
    })
    public dependencies: Dependency[]

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.devDependencies",
        default: []
    })
    public devDependencies: string[]

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.srcPath",
        default: "src"
    })
    public srcPath: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.buildPath",
        default: "build"
    })
    public buildPath: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.libPath",
        default: "libs"
    })
    public libPath: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.savedVariables",
        default: ""
    })
    public savedVariables: string

    public getBuildPath(): string {
        if (this.buildPath.startsWith("/")) {
            return this.buildPath
        }
        if (this.buildPath[1] === ":") {
            return this.buildPath
        }
        if (this.buildPath[0] === "~") {
            return process.env.HOME + "/" + this.buildPath.substring(1)
        }
        return process.cwd() + "/" + this.buildPath
    }

    public getSrcPath(): string {
        return process.cwd() + "/" + this.srcPath
    }

    public getLibPath(): string {
        return process.cwd() + "/" + this.libPath
    }

}
