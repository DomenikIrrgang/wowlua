-- /stdtest engine scenario auradeath, played with the engine's controls:
-- the level 21 paladin seals itself (Seal of Fury), consecrates the ground
-- under a Savannah Highmane, fights it (the seal's holy damage with each
-- swing), stuns it (Hammer of Justice, pressed again on its cooldown) and
-- kills it with Judgement while it is stunned (scenarios/auradeath.lua):
-- its death takes its auras. Each step at its recorded time (a key: when
-- the client acted), each cast's answer its recorded round trip later.
local e = __engine
-- The damage meter's sessions numbered on from the client's before the
-- recording (its first: 2).
e.setDamageMeterSession(2)
-- As in the client's frames, some events wait for the frame's end, each
-- once, in their names' order (`setFrameBatching`).
e.setFrameBatching(true)
local player = e.player
e.setLatency(0.37)
player:setName("Player", "Lastname")
local cat = e.spawnUnit({
    name = "Savannah Highmane", guid = "Creature-0-4621-1-21-3243-03C97F1DA4", npcID = 3243, level = 12,
    reaction = "hostile", sex = 2, powerType = 1, health = 300, maxHealth = 300,
    creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Cat", creatureFamilyID = 2,
})
cat:showNameplate()
-- What each spell does as it lands, and the cooldowns its packet tells
-- (recorded). The seal replaces the one on (the snapshot's first, 37), in
-- one update.
local seal = 37
e.defineSpell(1311656, {
    name = "Seal of Fury", targeted = false,
    -- Its slot told changed twice as it is sent (recorded).
    slotChanges = { 0, 0 },
    onCast = function()
        seal = player:applyAura({
            spellId = 1311656, name = "Seal of Fury", icon = 135957, duration = 30,
            auraInstanceID = seal == 37 and 41 or nil, replaces = seal,
        })
    end,
})
-- Each one's cooldown category (recorded with its cooldown events).
-- Consecration's slots told changed as it is sent and 7.937 s later
-- (recorded).
e.defineSpell(26573, { name = "Consecration", targeted = false, category = 932, slotChanges = { 0, 7.937 } })
e.defineSpell(853, {
    name = "Hammer of Justice", cooldown = 60, category = 32,
    cooldowns = { { 853, 32, 133 }, { 1324513 }, { 1260198, 0, 0 }, { 1311654, 0, 0 } },
    -- The stun (the cat's flags at once), its mana, a swing landing with it
    -- (the seal's holy damage, a drain). An aura on the player with it,
    -- secret in the recording: which one is unknown (instance 45, gone at
    -- 24.275; its update told before the cat's).
    onCast = function()
        player:applyAura({ spellId = 0, name = "Unknown", auraInstanceID = 45 })
        cat:applyAura({ spellId = 853, name = "Hammer of Justice", duration = 6, harmful = true, source = player, stuns = true })
        player:setPower(695)
        player:swing(cat, 40, { speed = 2.6, textAfter = 0.43 })
        player:hit(cat, 39, { school = 32 })
        player:hit(cat, 12, { school = 2 })
    end,
})
e.defineSpell(1866, { category = 2404 })
-- Judgement: its mana (too little left for Redemption), the kill (its
-- holy damage the blow), the seal consumed.
e.defineSpell(20271, {
    name = "Judgement", cooldown = 8,
    cooldowns = { { 20232, 0, 0 }, { 20271, 0, 0 }, { 1311655, 0, 0 } },
    onCast = function()
        player:setPower(250)
        cat:kill({ by = player, lootable = true, amount = 54, school = 2 })
        if seal then player:removeAura(seal) seal = nil end
    end,
})
-- Redemption (slot 54): out of combat only, its mana cost (unrecorded:
-- what the fight's spending left below it, the regeneration crossed).
e.defineSpell(7328, { name = "Redemption", powerCost = 300, outOfCombat = true })
-- Judgement off the global cooldown (recorded: its slot not on it).
e.setSpellTraits({ offGlobalCooldown = { 20271 } })
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
cat:setDistance(5)
-- What each tick and blow triggers: its spells' cooldowns (recorded).
local TICK = { { 1280353 } }
local BURN = { { 1280345, 0, 0 }, { 1280353 } }
local SEAL = { 1311654, 0, 0 }
local steps = {}
-- The seal: its aura when the server answers.
steps[#steps + 1] = { 1.927, function() e.setLatency(0.286) player:useSpell(1311656) end }
steps[#steps + 1] = { 4.789, function() player:softTarget(cat, "enemy") end }
-- Consecration under the cat: the cat targeted as it lands, the fight
-- begins, the ground's aura on it ticking every half second.
local consecration
steps[#steps + 1] = { 4.957, function() e.setLatency(0.372) player:useSpell(26573) end }
steps[#steps + 1] = { 5.329, function()
    player:target(cat)
    -- (Its end told at 13.957, recorded: its duration left out.)
    consecration = cat:applyAura({ spellId = 26573, name = "Consecration", harmful = true, source = player })
    cat:target(player)
end }
-- The cat's claws (each at its health's update, its combat text the
-- recorded delay later); the consecrated ground's holy damage; its ticks.
local claws = { { 5.329, 9, 0.43 }, { 6.835, 8, 0.761 }, { 8.386, 8, 0.433 }, { 9.846, 9, 0.426 }, { 12.846, 7, 0.758 }, { 14.311, 8, 0.43 } }
local burns = { { 6.352, 7 }, { 7.348, 8 }, { 8.386, 7 }, { 9.328, 7 }, { 12.346, 8 }, { 13.266, 11, "CRITICAL" } }
local ticks = { 5.833, 6.835, 7.857, 8.899, 9.846, 10.851, 11.842, 12.846 }
for _, c in ipairs(claws) do steps[#steps + 1] = { c[1], function() cat:swing(player, c[2], { textAfter = c[3] }) end } end
for _, b in ipairs(burns) do
    steps[#steps + 1] = { b[1], function()
        player:hit(cat, b[2], { school = 2, indicator = b[3], aura = consecration, cooldowns = BURN })
    end }
end
for _, t in ipairs(ticks) do steps[#steps + 1] = { t, function() cat:updateAura(consecration, { cooldowns = TICK }) end } end
-- The paladin's swings: the seal's holy damage at once, the blow's text
-- later; the first with a burn.
-- Right-clicked: targeted again, the soft target let go (recorded:
-- no soft enemy events after it).
steps[#steps + 1] = { 10.030, function() player:target(cat) player:softTarget(nil, "enemy") end }
-- The first: the seal's damage, the mana, the swing (its text 0.534 s
-- later) and auto attack on, the mana again, the burn, a proc failing.
steps[#steps + 1] = { 10.354, function()
    player:hit(cat, 7, { school = 2 })
    player:setPower(695)
    player:swing(cat, 43, { speed = 2.6, textAfter = 0.534 })
    player:startAttack()
    player:setPower(700)
    player:hit(cat, 12, { school = 2, aura = consecration, cooldowns = { BURN[1], BURN[2], SEAL } })
    player:spellFailedQuiet(1310927)
end }
-- Struck and burning: the claw, the burn's damage, the mana, the
-- paladin's Redoubt (its aura: not gone by the recording's end, its
-- duration unrecorded), then the ground's update with the burn's and
-- Redoubt's cooldowns (recorded in that order).
local redoubt
steps[#steps + 1] = { 11.334, function()
    cat:swing(player, 8, { textAfter = 0.433 })
    player:hit(cat, 7, { school = 2 })
    player:setPower(690)
    redoubt = player:applyAura({ spellId = 20128, name = "Redoubt" })
    cat:updateAura(consecration, { cooldowns = { BURN[1], BURN[2], { 20128 } } })
end }
steps[#steps + 1] = { 12.951, function()
    player:swing(cat, 47, { speed = 2.6, textAfter = 0.431 })
    player:hit(cat, 12, { school = 2, cooldowns = { SEAL } })
end }
-- The seal again; the ground's aura gone.
steps[#steps + 1] = { 13.122, function() e.setLatency(0.382) player:useSpell(1311656) end }
steps[#steps + 1] = { 13.957, function() cat:removeAura(consecration) end }
steps[#steps + 1] = { 14.311, function()
    if redoubt then player:updateAura(redoubt, { cooldowns = { { 20128 } } }) end
end }
-- Hammer of Justice: the stun, a swing, then pressed again on its
-- cooldown.
steps[#steps + 1] = { 15.185, function() e.setLatency(0.377) player:useSpell(853) end }
steps[#steps + 1] = { 15.696, function() player:useSpell(853) end }
steps[#steps + 1] = { 15.722, function() player:hit(player, 39, { kind = "HEAL" }) end }
steps[#steps + 1] = { 15.863, function() player:useSpell(853) end }
-- Judgement kills it, stunned.
steps[#steps + 1] = { 16.419, function() e.setLatency(0.378) player:useSpell(20271) end }
-- Pressed again during the last moments (a key with both spells): held
-- by the client, failing when the cat dies.
steps[#steps + 1] = { 16.524, function() player:useSpell(1866) end }
steps[#steps + 1] = { 16.641, function() player:useSpell(1866) end }
steps[#steps + 1] = { 16.649, function() player:useSpell(20271) end }
steps[#steps + 1] = { 16.782, function() player:useSpell(20271) end }
-- The target's death told by the server a while after (recorded: 0.58 s).
steps[#steps + 1] = { 16.79, function() e.setLatency(0.58) end }
-- Still held after the kill: both spells refused at once, out of range
-- (recorded; the dead cat soft-interacted), each time the key repeats.
for _, t in ipairs({ 16.877, 16.983, 17.081 }) do
    steps[#steps + 1] = { t, function()
        player:castFailed(20271, { error = "SPELL_OUT_OF_RANGE", client = true })
        player:castFailed(1866, { error = "SPELL_OUT_OF_RANGE", client = true })
    end }
end
-- The server's answers to the presses held while Judgement flew: the
-- second 1866 dropped, a Judgement not ready (its cooldown), a 1866 on a
-- dead target; the last Judgement not ready, auto attack off and on
-- (recorded).
steps[#steps + 1] = { 17.005, function()
    player:spellFailedQuiet(1866, { client = true })
    player:castFailed(20271, { error = "SPELL_COOLDOWN" })
    player:castFailed(1866, { error = "TARGETS_DEAD" })
end }
-- The mana's changes (costs, regeneration: its values secret) and the
-- cat's threat list as the client told them (recorded times).
local mana = 700
for _, t in ipairs({ 0.352, 2.322, 5.329, 10.489, 10.851, 11.842, 12.346, 12.456, 12.846, 13.102, 13.382, 13.504 }) do
    mana = mana == 700 and 690 or 700
    local m = mana
    steps[#steps + 1] = { t, function() player:setPower(m) end }
end
-- A regeneration tick gives Redemption's back (22.434), then the
-- updates.
steps[#steps + 1] = { 22.434, function() player:setPower(305, { frequent = true }) end }
steps[#steps + 1] = { 22.667, function() player:setPower(310) end }
steps[#steps + 1] = { 24.708, function() player:setPower(320) end }
steps[#steps + 1] = { 24.275, function() player:removeAura(45) end }
-- A proc the server cast failing quietly (1310927; what it is unknown),
-- after the seal's blows and the drain's heal.
for _, t in ipairs({ 13.102, 15.722 }) do
    steps[#steps + 1] = { t, function() player:spellFailedQuiet(1310927) end }
end
-- The fight over: the last Judgement's refusal (auto attack off and on
-- with it), then out of combat.
steps[#steps + 1] = { 17.165, function()
    player:castFailed(20271, { error = "SPELL_COOLDOWN" })
    player:stopAttack()
    player:startAttack()
end }
-- Health back out of combat (values unrecorded).
for _, t in ipairs({ 18.599, 20.634 }) do
    steps[#steps + 1] = { t, function() player:heal(1) end }
end
steps[#steps + 1] = { 17.377, function() player:leaveCombat() end }
-- (The threat as recorded: the cat's list, from the fight's start.)
for _, h in ipairs({ { 5.329, 0 }, { 6.483, 7 }, { 7.684, 15 }, { 8.899, 22 }, { 10.111, 29 }, { 11.334, 92 }, { 12.456, 107 }, { 13.750, 177 }, { 14.965, 177 }, { 16.092, 268 } }) do
    steps[#steps + 1] = { h[1], function() cat:setThreat(h[2]) end }
end
-- In time, steps of the same time in the order written.
for i, step in ipairs(steps) do step[3] = i end
table.sort(steps, function(a, b) return a[1] < b[1] or (a[1] == b[1] and a[3] < b[3]) end)
for _, s in ipairs(steps) do
    at(s[1])
    s[2]()
end
at(30)
-- The last frame's batched events.
e.setFrameBatching(false)
