-- /stdtest engine scenario combat, played with the engine's controls: the
-- recording's level 1 warrior targets a Young Wolf, fights it with auto
-- attack and Heroic Strike, kills it and loots it (scenarios/combat.lua).
-- Each step at its recorded time, the ping as recorded (~0.3 s). Rage
-- values are the engine's guesses, consistent with the usability the
-- client told (it keeps rage secret in combat).
local e = __engine
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
-- What it drops (the recording's loot slots; Chipped Claw is not in the
-- warrior's bags yet).
e.defineItem(7074, {
    name = "Chipped Claw", quality = 0, icon = 133723, stackCount = 10, classID = 15, subclassID = 0,
    link = "|cnIQ0:|Hitem:7074::::::::1:1491:::::::::|h[Chipped Claw]|h|r",
})
-- Heroic Strike: on the next swing, its rage cost unverified (15; to
-- record: C_Spell.GetSpellPowerCost(78)).
e.defineSpell(78, { name = "Heroic Strike", powerCost = 15, nextSwing = true })
local now = 0
local function at(t)
    e.advanceTime(t - now)
    now = t
end
at(2.561) player:target(wolf)
at(2.864) player:startAttack()
at(4.330) player:swing(wolf, 6, { power = 4 })
wolf:setThreat(5)
at(4.434) wolf:target(player)
wolf:swing(player, 2, { victimPower = 7 })
at(5.332) player:useSpell(1259718)
at(5.909) wolf:setThreat(6)
at(6.283) player:swing(wolf, 5, { power = 10 })
at(6.420) wolf:swing(player, 1, { victimPower = 11 })
at(7.135) wolf:setThreat(10)
at(8.200) player:swing(wolf, 6, { power = 16 })
at(8.305) wolf:setThreat(15)
at(8.409) wolf:swing(player, 1, { victimPower = 17 })
at(10.065) player:useSpell(78)
at(10.103) player:swing(wolf, 0, { kind = "PARRY" })
at(10.437) wolf:swing(player, 1, { victimPower = 18 })
at(11.976) player:swing(wolf, 16, { spent = 3, power = 8 })
at(12.410) wolf:swing(player, 1, { victimPower = 9 })
at(13.237) wolf:setThreat(41)
at(13.892) player:swing(wolf, 7, { power = 13 })
at(14.151) player:useSpell(78)
at(14.343) player:useSpell(78)
at(14.348) wolf:swing(player, 1, { victimPower = 14 })
wolf:setThreat(47)
at(14.454) player:setPower(15)
at(14.849) player:useSpell(78)
at(15.857) player:swing(wolf, 17, { spent = 0, power = 2, kill = { xp = 50, loot = { { id = 7073 }, { id = 7074 } } } })
at(16.116) player:stopAttack()
at(16.915) player:leaveCombat()
-- The corpse clicked: targeted, looted with autoloot.
at(21.077) player:target(wolf)
player:loot(wolf, { auto = true })
at(22.921)
