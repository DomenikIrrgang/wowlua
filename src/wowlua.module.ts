import { Module } from "cli-program-lib/decorators/module.decorator"
import { JsonParser } from "cli-program-lib/properties/json.parser"
import { InitCommand } from "./commands/init.command";
import { WowluaConfig } from "./config/wowlua.config";
import { BuildCommand } from "./commands/build/build.command";
import { InstallCommand } from "./commands/install/install.command";
import { Performance } from "./util/performance";

@Module({
    commands: [
        InitCommand,
        BuildCommand,
        InstallCommand
    ],
    providers: [
        WowluaConfig,
        JsonParser,
        Performance
    ],
})
export class WowLuaModule {}