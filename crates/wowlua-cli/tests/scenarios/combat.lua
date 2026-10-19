-- /stdtest engine scenario combat, played with the engine's controls: the
-- recording's level 1 warrior targets a Young Wolf, fights it with auto
-- attack and Heroic Strike, kills it and loots it (scenarios/combat.lua).
-- Each step at its recorded time, the ping as recorded (~0.3 s). Rage
-- values are the engine's guesses, consistent with the usability the
-- client told (it keeps rage secret in combat). Each hit's combat text
-- after its recorded delay (`textAfter`), the loot's slots at their
-- recorded times, its bags' and new item's data too (`slotsAfter`,
-- `bagsAfter`, `dataAfter`).
local e = __engine
-- The damage meter's sessions numbered on from the client's before the
-- recording (its first: 3).
e.setDamageMeterSession(3)
-- As in the client's frames, some events wait for the frame's end, each
-- once, in their names' order (`setFrameBatching`).
e.setFrameBatching(true)
local player = e.player
e.setLatency(0.3)
-- The warrior's names as the recording has them, scrubbed.
player:setName("Player", "Lastname")
local wolf = e.spawnUnit({
    name = "Young Wolf", guid = "Creature-0-6782-0-7-299-DEE9062BA3", npcID = 299, level = 1,
    reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1,
    creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1,
    -- Its health: the damage it took (6, 5, 6, 16, 7, 17).
    health = 57, maxHealth = 57,
})
-- What it drops (the recording's loot slots; Chipped Claws stack to 5:
-- the quest scenario's bags).
e.defineItem(7074, {
    name = "Chipped Claw", quality = 0, icon = 133723, stackCount = 5, classID = 15, subclassID = 0,
    link = "|cnIQ0:|Hitem:7074::::::::1:1491:::::::::|h[Chipped Claw]|h|r",
})
-- Heroic Strike: on the next swing, its rage cost unverified (15; to
-- record: C_Spell.GetSpellPowerCost(78)).
e.defineSpell(78, { name = "Heroic Strike", powerCost = 15, nextSwing = true })
-- The instant spell on slot 10: no target (its SENT names none).
e.defineSpell(1259718, { targeted = false })
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
at(2.561) player:target(wolf)
local main = TargetFrame.TargetFrameContent.TargetFrameContentMain
check("the target frame: Young Wolf, level 1", TargetFrame:IsShown() and main.Name:GetText() == "Young Wolf" and main.LevelText:GetText() == "1")
at(2.864) player:startAttack()
at(4.330) player:swing(wolf, 6, { power = 4, textAfter = 0.529 })
local attackIcon = PlayerFrame.PlayerFrameContent.PlayerFrameContentContextual.AttackIcon
check("in combat: the player frame's combat icon", attackIcon:IsVisible())
wolf:setThreat(5)
at(4.434) wolf:target(player)
wolf:swing(player, 2, { victimPower = 7 })
at(5.332) player:useSpell(1259718)
at(5.909) wolf:setThreat(6)
at(6.283) player:swing(wolf, 5, { power = 10, textAfter = 0.528 })
at(6.420) wolf:swing(player, 1, { victimPower = 11, textAfter = 0.396 })
at(7.135) wolf:setThreat(10)
at(8.200) player:swing(wolf, 6, { power = 16, textAfter = 0.401 })
at(8.305) wolf:setThreat(15)
at(8.409) wolf:swing(player, 1, { victimPower = 17, textAfter = 0.335 })
at(8.788) player:raiseSkill("Defense", 4)
-- Heroic Strike pressed; the server told of it the recorded delay later.
at(10.065) e.setLatency(0.268) player:useSpell(78) e.setLatency(0.3)
at(10.103) player:swing(wolf, 0, { kind = "PARRY", textAfter = 0.531 })
at(10.437) wolf:swing(player, 1, { victimPower = 18, textAfter = 0.399 })
at(11.976) player:swing(wolf, 16, { spent = 3, power = 8 })
at(12.410) wolf:swing(player, 1, { victimPower = 9, textAfter = 0.334 })
at(13.237) wolf:setThreat(41)
at(13.892) player:swing(wolf, 7, { power = 13, textAfter = 0.396 })
at(14.151) player:useSpell(78)
at(14.343) player:useSpell(78)
at(14.348) wolf:swing(player, 1, { textAfter = 0.396 })
wolf:setThreat(47)
at(14.454) player:setPower(15)
at(14.507) e.setLatency(0.342) player:useSpell(78) e.setLatency(0.3)
at(15.857) player:swing(wolf, 17, { spent = 0, power = 2, kill = { xp = 50, loot = { { id = 7073 }, { id = 7074 } } } })
at(16.116) player:stopAttack()
-- Off the dead wolf's threat list first (recorded).
at(16.782) wolf:setThreat(nil)
at(16.915) player:leaveCombat()
-- Out of combat: rage fades, health comes back.
at(18.782) player:setPower(0)
player:heal(1)
at(20.808) player:heal(1)
-- The corpse clicked: targeted, looted with autoloot.
at(21.077) player:target(wolf)
player:loot(wolf, { auto = true, slotsAfter = { 0.516, 0.767 }, bagsAfter = { 0.252, 0 }, dataAfter = { 0.52 } })
at(21.700) check("the loot window open", LootFrame:IsShown())
at(22.921)
-- What Blizzard's UI shows at the end.
local function chatHas(frame, text)
    for i = 1, frame:GetNumMessages() do
        if (frame:GetMessageInfo(i)) == text then return true end
    end
    return false
end
check("the target cleared: its frame hidden", not TargetFrame:IsShown())
check("out of combat: no combat icon", not attackIcon:IsVisible())
check("the loot window closed", not LootFrame:IsShown())
check("chat: the skill up", chatHas(ChatFrame1, "Your skill in Defense has increased to 4."))
check("chat: the loot", chatHas(ChatFrame1, "You receive loot: |cnIQ0:|Hitem:7073::::::::1:1491:::::::::|h[Broken Fang]|h|r")
    and chatHas(ChatFrame1, "You receive loot: |cnIQ0:|Hitem:7074::::::::1:1491:::::::::|h[Chipped Claw]|h|r"))
check("the combat log tab: the experience", chatHas(ChatFrame2, "Young Wolf dies, you gain 50 experience."))
local xpBar
local function find(f, depth)
    if depth > 5 or not f or f:IsForbidden() or xpBar then return end
    if f:IsObjectType("StatusBar") and f:IsVisible() then xpBar = f return end
    for _, c in ipairs({ f:GetChildren() }) do find(c, depth + 1) end
end
find(MainStatusTrackingBarContainer, 0)
local _, xpMax = xpBar:GetMinMaxValues()
check("the experience bar: 151 of 400", xpBar:GetValue() == 151 and xpMax == 400)
-- The last frame's batched events.
e.setFrameBatching(false)
