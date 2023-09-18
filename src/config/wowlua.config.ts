import { Injectable } from "cli-program-lib/decorators/injectable.decorator"
import { Json } from "cli-program-lib/decorators/json.decorator"
import { GameVersion } from "../util/game-version"
import { Dependency } from "../dependency-manager/dependency"
import { getPath } from "../util/trim-path"

@Injectable({
    unique: true,
})
export class WowluaConfig {

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.name",
        default: "WowluaProject"
    })
    public name: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.version",
        default: "0.0.1"
    })
    public version: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.author",
        default: "Wowlua"
    })
    public author: string

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.description",
        default: "Addon created using wowlua."
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
        return getPath(this.srcPath)
    }

    public getLibPath(): string {
        return getPath(this.libPath)
    }

    public save(): void {
        Bun.write("wowlua.json", JSON.stringify(this, null, 4))
    }

}
