import { Module } from "cli-program-lib/decorators/module.decorator"
import { JsonParser } from "cli-program-lib/properties/json.parser"
import { InitCommand } from "./commands/init/init.command";
import { WowluaConfig } from "./config/wowlua.config";
import { BuildCommand } from "./commands/build.command";
import { InstallCommand } from "./commands/install.command";
import { Performance } from "./util/performance";
import { BuildSystem } from "./build-system/build-system";

@Module({
    commands: [
        InitCommand,
        BuildCommand,
        InstallCommand
    ],
    providers: [
        WowluaConfig,
        JsonParser,
        Performance,
        BuildSystem
    ],
})
export class WowLuaModule {}