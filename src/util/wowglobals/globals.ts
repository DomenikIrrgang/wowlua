import { GameVersion } from "../game-version"

export let GLOBALS_FILE: Map<GameVersion, string> = new Map()
GLOBALS_FILE.set(GameVersion.CLASSIC, "data/classicglobals.json")
GLOBALS_FILE.set(GameVersion.WOTLK, "data/classicglobals.json")
GLOBALS_FILE.set(GameVersion.RETAIL, "data/classicglobals.json")