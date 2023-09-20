import { Injectable } from "cli-program-lib/decorators/injectable.decorator"
import { Json } from "cli-program-lib/decorators/json.decorator"
import { GameVersion } from "../util/game-version"
import { Dependency } from "../dependency-manager/dependency"
import { getPath } from "../util/trim-path"
import { PluginOptions } from "../build-system/plugins/plugin-options"

@Injectable({
    unique: true,
    global: true
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
        jsonPath: "$.buildVars",
        default: ""
    })
    public buildVars: { [variableName: string]: string | number | boolean }

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
        default: []
    })
    public savedVariables: string[]

    @Json({
        filePath: "wowlua.json",
        jsonPath: "$.pluginOptions",
        default: {}
    })
    public pluginOptions: { [pluginName: string]: PluginOptions }

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

    public getPluginOptions(pluginName: string): PluginOptions {
        let result: PluginOptions = { enabled: true}
        if (this.pluginOptions[pluginName] !== undefined) {
            result = this.pluginOptions[pluginName]
            if (result.enabled === undefined) {
                result.enabled = true
            }
        }
        return result
    }

}
