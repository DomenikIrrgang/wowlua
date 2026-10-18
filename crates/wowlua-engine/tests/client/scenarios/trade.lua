-- WoW Forever 1.60.1 (70425): /stdtest engine scenario trade: its events in
-- order, and after the events Blizzard's UI answers, `= call -> answer`
-- (cargo xtask ingest); the world before and after: trade-before.json,
-- trade-after.json.
return {
    ["00001"] = "1.138 LOADING_SCREEN_ENABLED()",
    ["00002"] = "1.138 SECURE_TRANSFER_CANCEL()",
    ["00003"] = "1.138 TAXIMAP_CLOSED()",
    ["00004"] = "1.138 = UnitOnTaxi(\"player\") -> 1: boolean false",
    ["00005"] = "1.138 = C_Map.GetBestMapForUnit(\"player\") -> 1: number 1413",
    ["00006"] = "1.138 SECURE_TRANSFER_CANCEL()",
    ["00007"] = "1.138 TRADE_SKILL_CLOSE()",
    ["00008"] = "1.138 SOCKET_INFO_CLOSE()",
    ["00009"] = "1.138 ARTIFACT_CLOSE()",
    ["00010"] = "1.138 NAVIGATION_FRAME_DESTROYED()",
    ["00011"] = "1.138 PLAYER_SOFT_FRIEND_CHANGED()",
    ["00012"] = "1.138 INSTANCE_LOCK_STOP()",
    ["00013"] = "1.138 PLAYER_LEAVING_WORLD()",
    ["00014"] = "1.138 ADDONS_UNLOADING(boolean false)",
    ["00015"] = "1.138 PLAYER_LOGOUT()",
    ["00016"] = "1.138 NAME_PLATE_UNIT_REMOVED(string nameplate4)",
    ["00017"] = "1.138 NAME_PLATE_UNIT_REMOVED(string nameplate7)",
    ["00018"] = "1.138 NAME_PLATE_UNIT_REMOVED(string nameplate3)",
    ["00019"] = "1.138 NAME_PLATE_UNIT_REMOVED(string nameplate2)",
    ["00020"] = "1.138 NAME_PLATE_UNIT_REMOVED(string nameplate1)",
    ["00021"] = "1.138 GAME_PAD_CONFIGS_CHANGED()",
    ["latency"] = "3.488336801528931, 0.2454992234706879, 270, 244",
}
