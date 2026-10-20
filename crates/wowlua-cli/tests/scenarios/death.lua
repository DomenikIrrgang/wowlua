-- /stdtest engine scenario death, played with the engine's controls: the
-- level 2 warrior pulls too many wolves and kobolds, cannot hit them
-- (facing the wrong way), dies, releases its spirit, runs back and takes
-- its corpse (scenarios/death.lua). Each step at its recorded time (a
-- click: a moment before the server's answer), the ping as recorded
-- (~0.3 s). The fight's steps were read off the recording; every hit on
-- the player is given to its target of the moment (several creatures hit
-- it); each hit's combat text after its recorded delay (`textAfter`). The
-- worn items' durability as the quest scenario left it.
local e = __engine
-- The damage meter's sessions numbered on from the client's before the
-- recording (its first: 11).
e.setDamageMeterSession(11)
-- As in the client's frames, some events wait for the frame's end, each
-- once, in their names' order (`setFrameBatching`).
e.setFrameBatching(true)
local player = e.player
e.setLatency(0.3)
-- The warrior's names as the recording has them, scrubbed.
player:setName("Player", "Lastname")
-- The warrior's Defensive State: its cooldown told when the warrior dodges,
-- parries or blocks (recorded).
e.defineSpell(5302, { name = "Defensive State", procOnAvoid = true })
-- Heroic Strike: usable with its rage (recorded: short of it at the
-- fight's start, usable from the hit at 6.927 to the death; the rage's
-- values secret, so guessed: 5 before, 15 from then on).
e.defineSpell(78, { name = "Heroic Strike", powerCost = 15, nextSwing = true })
player:setDurability(7, 24, 25)
player:setDurability(8, 16, 16)
player:setDurability(16, 19, 20)
player:setDurability(17, 20, 20)
-- The creatures' health (secret in the recording; each alive to the
-- end, unrecorded); every one neutral to the player (recorded: its
-- reaction, 4), the player hostile to the kobolds (recorded: the player's
-- reaction to each, 2).
local u_69_cd4e8eb2c4 = e.spawnUnit({ name = "Timber Wolf", guid = "Creature-0-6782-0-7-69-CD4E8EB2C4", npcID = 69, level = 2, reaction = "neutral", sex = 1, class = "WARRIOR", classID = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 40, maxHealth = 40 })
local u_299_cd4e8eb2c4 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-CD4E8EB2C4", npcID = 299, level = 1, reaction = "neutral", sex = 1, class = "WARRIOR", classID = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 40, maxHealth = 40 })
local u_6_9adbb50a8f = e.spawnUnit({ name = "Kobold Vermin", guid = "Creature-0-6782-0-7-6-9ADBB50A8F", npcID = 6, level = 2, reaction = "neutral", playerReaction = "hostile", sex = 1, class = "WARRIOR", classID = 1, powerType = 1, creatureType = "Humanoid", creatureTypeID = 7, health = 40, maxHealth = 40 })
local u_6_f4dca45d75 = e.spawnUnit({ name = "Kobold Vermin", guid = "Creature-0-6782-0-7-6-F4DCA45D75", npcID = 6, level = 2, reaction = "neutral", playerReaction = "hostile", sex = 1, class = "WARRIOR", classID = 1, powerType = 1, creatureType = "Humanoid", creatureTypeID = 7, health = 40, maxHealth = 40 })
local u_6_65112a2818 = e.spawnUnit({ name = "Kobold Vermin", guid = "Creature-0-6782-0-7-6-65112A2818", npcID = 6, level = 1, reaction = "neutral", playerReaction = "hostile", sex = 1, class = "WARRIOR", classID = 1, powerType = 1, creatureType = "Humanoid", creatureTypeID = 7, health = 40, maxHealth = 40 })
-- A creature another player killed nearby: its death.
local function bystander(guid, npcID, name)
    e.spawnUnit({ name = name, guid = guid, npcID = npcID, level = 1, reaction = "hostile" }):kill()
end
local now = 0
-- Time passes in frames, as in the client (60 a second: Blizzard's
-- animations and OnUpdate code run), the rest of a step as one jump.
local function at(t)
    if t < now then error(("at(%s): before %s"):format(t, now), 2) end
    local frames = math.floor((t - now) * 60)
    if frames > 0 then
        e.advanceFrames(frames, 1000 / 60)
    end
    e.advanceTime(math.max(0, t - now - frames / 60))
    now = t
end
local function press(frame, button)
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
-- The target change's echo with the auto attack's start (recorded at
-- 6.927 and 10.321: UNIT_TARGET("player") right after
-- PLAYER_ENTER_COMBAT, not a round trip after the target change).
local function echo()
    e.fireUnitEvent("UNIT_TARGET", "player")
end
-- The player's flags told for the hidden aura (recorded at 20.571 and
-- 24.356, by the target's target too; the flag unknown).
local function flagsTold()
    e.fireUnitEvent("UNIT_FLAGS", "player")
    e.fireUnitEvent("UNIT_FLAGS", "targettarget")
end
local function chatHas(frame, text)
    for i = 1, frame:GetNumMessages() do
        if (frame:GetMessageInfo(i)) == text then return true end
    end
    return false
end
at(2.231) player:target(u_69_cd4e8eb2c4)
at(2.478) player:startAttack()
-- The first blows: the player's swing, the wolf turning on it, then the
-- fight's start (recorded in that order).
-- (The swing's damage unrecorded: its text came once the target was
-- cleared; its health told.)
at(3.019) player:swing(u_69_cd4e8eb2c4, 5, { combat = false })
at(3.019) u_69_cd4e8eb2c4:target(player)
at(3.019) player:enterCombat()
at(3.019) player:setPower(5)
at(3.019) u_69_cd4e8eb2c4:swing(player, 2, { victimPower = 5, textAfter = 0.398 })
at(3.019) u_69_cd4e8eb2c4:setThreat(5)
at(3.364) player:target(nil) -- cleared (recorded: no target till 3.969)
at(3.656) player:stopAttack()
at(3.969) player:target(u_299_cd4e8eb2c4)
at(4.215) player:startAttack()
at(4.934) player:swing(u_299_cd4e8eb2c4, 6, { power = 5, textAfter = 0.397 })
at(4.934) u_299_cd4e8eb2c4:target(player)
at(4.934) u_299_cd4e8eb2c4:setThreat(5)
-- The dodge in the blow's frame (recorded: its cooldown before the
-- frame's health).
at(4.934) u_299_cd4e8eb2c4:swing(player, 0, { kind = "DODGE", textAfter = 0.333, statesAfter = 0.102, stateFor = 5.387 })
-- Both skills up in one packet before the blow's power (recorded).
at(5.036) player:raiseSkill("Defense", 7, "Swords", 8)
at(5.036) u_299_cd4e8eb2c4:swing(player, 1, { victimPower = 5, textAfter = 0.332 })
at(6.270) u_299_cd4e8eb2c4:setThreat(6)
at(6.589) player:target(u_6_9adbb50a8f, { echo = false })
at(6.589) bystander("Creature-0-6782-0-7-69-49CA867F07", 69, "Timber Wolf")
at(6.810) e.gameError("BADATTACKFACING")
-- The attack turned to the new target: off, then on (recorded).
at(6.927) player:stopAttack()
at(6.927) player:startAttack()
at(6.927) echo()
at(6.927) u_6_9adbb50a8f:swing(player, 2, { victimPower = 15, textAfter = 0.397 })
at(7.873) player:swing(u_6_9adbb50a8f, 6, { power = 15, textAfter = 0.396 })
at(7.873) u_6_9adbb50a8f:target(player)
at(7.873) u_6_9adbb50a8f:swing(player, 3, { victimPower = 15, textAfter = 0.365 })
at(7.873) u_6_9adbb50a8f:setThreat(5)
at(7.988) player:setPower(15)
at(9.055) u_6_9adbb50a8f:setThreat(6)
at(9.854) e.gameError("BADATTACKFACING")
at(9.930) u_6_9adbb50a8f:swing(player, 2, { victimPower = 15, textAfter = 0.375 })
at(10.045) player:target(u_6_f4dca45d75, { echo = false })
at(10.321) player:stopAttack()
-- The attack's start and the echo after the swing, before its rage
-- (recorded).
at(10.321) e.beforeEvent("UNIT_POWER_UPDATE", function()
    player:startAttack()
    echo()
end)
at(10.321) player:swing(u_6_f4dca45d75, 7, { power = 15, textAfter = 0.387 })
at(10.321) u_6_f4dca45d75:setThreat(6)
at(10.414) u_6_f4dca45d75:target(player)
at(10.414) u_6_f4dca45d75:swing(player, 2, { victimPower = 15, textAfter = 0.274 })
at(10.981) u_6_f4dca45d75:swing(player, 1, { victimPower = 15, textAfter = 0.391 })
at(11.454) u_6_f4dca45d75:setThreat(7)
at(11.509) player:target(u_6_65112a2818)
at(11.781) player:stopAttack()
at(11.781) player:startAttack()
at(11.884) u_6_65112a2818:swing(player, 3, { textAfter = 0.297, statesAfter = 0.104 })
at(11.988) player:setPower(15)
-- The swing's rage told twice, then the kobold's turn, then its blow
-- (recorded in that order).
at(12.318) player:swing(u_6_65112a2818, 5, { power = 15, textAfter = 0.397 })
at(12.318) player:setPower(15)
at(12.318) u_6_65112a2818:target(player)
at(12.318) e.tellTurns()
at(12.318) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.3 })
at(12.318) u_6_65112a2818:setThreat(4)
at(12.419) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.296 })
at(12.419) player:raiseSkill("Defense", 8)
at(12.969) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.397 })
at(13.141) u_6_65112a2818:swing(player, 0, { kind = "MISS", textAfter = 0.33 })
at(13.560) u_6_65112a2818:setThreat(5)
-- Two power packets in this moment (recorded).
at(13.909) player:setPower(15)
at(13.909) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.362 })
at(14.162) e.gameError("BADATTACKFACING")
at(14.324) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.299 })
-- A second blow in the same packet (recorded: its text 0.361 s later).
at(14.324) u_6_65112a2818:swing(player, 2, { textAfter = 0.361 })
at(14.439) player:setPower(15)
at(14.968) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.397 })
-- Two power packets in this moment (recorded).
at(15.127) player:setPower(15)
at(15.127) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.398 })
at(15.841) bystander("Creature-0-6782-0-7-299-069F61ED3A", 299, "Young Wolf")
at(15.882) u_6_65112a2818:swing(player, 3, { textAfter = 0.298 })
at(15.985) player:setPower(15)
at(15.985) player:setPower(15)
at(16.248) u_6_65112a2818:swing(player, 1, { textAfter = 0.298 })
at(16.366) player:setPower(15)
at(16.366) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.299 })
at(16.990) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.396 })
at(17.093) u_6_65112a2818:swing(player, 3, { victimPower = 15, textAfter = 0.396, statesAfter = 0.103 })
at(17.937) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.298 })
at(18.163) e.gameError("BADATTACKFACING")
at(18.287) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.299 })
at(18.287) player:raiseSkill("Defense", 9)
at(18.462) u_6_65112a2818:swing(player, 3, { victimPower = 15, textAfter = 0.367 })
at(18.988) u_6_65112a2818:swing(player, 2, { textAfter = 0.33 })
at(19.055) player:setPower(15)
-- The blow's rage before the bystander's death (recorded).
at(19.179) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.396 })
at(19.179) bystander("Creature-0-6782-0-7-257-9FB7AE1D64", 257, "Kobold Worker")
at(20.037) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.334 })
at(20.271) bystander("Creature-0-6782-0-7-257-93E6A8BB49", 257, "Kobold Worker")
at(20.271) u_6_65112a2818:swing(player, 2, { textAfter = 0.333 })
at(20.371) player:setPower(15)
-- An aura on the player the recording keeps secret (in combat): which one
-- is unknown; its coming and going as recorded.
at(20.371) local hidden = player:applyAura({ spellId = 0, name = "Unknown", harmful = true })
at(20.371) u_6_65112a2818:swing(player, 2, { textAfter = 0.333 })
at(20.571) flagsTold()
at(20.571) player:setPower(15)
at(21.104) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.333 })
at(21.137) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.334 })
at(21.904) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.336 })
at(22.164) e.gameError("BADATTACKFACING")
at(22.333) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.297 })
at(22.473) u_6_65112a2818:swing(player, 0, { kind = "MISS", textAfter = 0.33 })
at(22.990) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.396 })
at(23.187) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.334 })
at(23.908) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.297 })
at(24.257) u_6_65112a2818:swing(player, 1, { textAfter = 0.297 })
at(24.356) player:setPower(15)
at(24.356) flagsTold()
at(24.356) player:setPower(15)
at(24.356) player:removeAura(hidden)
at(24.356) u_6_65112a2818:swing(player, 1, { textAfter = 0.297 })
at(24.492) bystander("Creature-0-6782-0-7-257-C8B267FD82", 257, "Kobold Worker")
at(25.000) u_6_65112a2818:swing(player, 1, { victimPower = 15, textAfter = 0.331 })
at(25.103) u_6_65112a2818:swing(player, 2, { textAfter = 0.332 })
at(25.279) player:setPower(15)
at(25.974) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.362 })
at(26.166) e.gameError("BADATTACKFACING")
at(26.327) u_6_65112a2818:swing(player, 2, { victimPower = 15, textAfter = 0.295 })
-- Dead: a tenth of each item's durability lost, the ghost's hidden aura
-- to come (57).
-- The target's clearing echoed 0.492 s on (recorded), with the player's
-- threat situation; the creatures' lists left at 27.377 (their
-- nameplates', the situation again).
at(26.440) e.setLatency(0.492)
at(26.440) player:die({ ghostAura = 57 })
at(26.440) e.setLatency(0.3)
at(26.6)
check("dead: the death popup", StaticPopup1 and StaticPopup1:IsShown() and StaticPopup1.which == "DEATH")
check("chat: you died", chatHas(ChatFrame1, "|cff71d5ff|Hdeath:1|h[You died.]|h"))
-- The killing blow's text, after the death (recorded: 2 damage).
at(26.736) e.fireUnitEvent("UNIT_COMBAT", "player", "WOUND", "", 2, 1)
at(26.932) e.fireUnitEvent("UNIT_THREAT_SITUATION_UPDATE", "player")
at(27.377) for _, u in ipairs({ u_69_cd4e8eb2c4, u_299_cd4e8eb2c4, u_6_9adbb50a8f, u_6_f4dca45d75, u_6_65112a2818 }) do u:setThreat(nil) end
at(29.150) press(StaticPopup1Button1)
at(29.6)
check("a ghost", UnitIsGhost("player"))
-- Running back: the area's updates, then a building's other part
-- (recorded; the places unrecorded).
at(39.381) e.areaChanged()
at(39.714) e.newWmoChunk()
at(43.106) player:wear()
at(44.277) bystander("Creature-0-6782-0-7-257-1DCD5746D5", 257, "Kobold Worker")
at(49.158) bystander("Creature-0-6782-0-7-257-DE4AC94919", 257, "Kobold Worker")
at(49.366) e.nearCorpse(true)
at(49.6)
check("the corpse offered", StaticPopup1:IsShown() and StaticPopup1.which == "RECOVER_CORPSE")
-- The corpse's answer took longer than the usual ping (recorded: 0.411 s).
at(50.355) e.setLatency(0.411)
at(50.355) press(StaticPopup1Button1)
at(50.355) e.setLatency(0.3)
-- A bystander's death in the resurrection's moment, before its usability
-- (recorded).
at(50.355) e.beforeEvent("ACTION_USABLE_CHANGED", function()
    bystander("Creature-0-6782-0-7-299-289E593BC2", 299, "Young Wolf")
end)
at(51.0)
check("alive again", not UnitIsDeadOrGhost("player"))
-- Health comes back (values unrecorded).
at(52.807) player:heal(1, { statesAfter = 0.389 })
at(54.807) player:heal(1)
at(56.910) player:heal(1)
at(57.5)
-- The last frame's batched events.
e.setFrameBatching(false)
