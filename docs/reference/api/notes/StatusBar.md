## In the emulator

A status bar in `wowlua test` keeps its range and value the way the client
does. This was recorded from the client:

- Values are kept as 32-bit floats, so `SetValue(7.3)` reads back as
  `7.3000001907349`.
- The range is 0 to 0 until `SetMinMaxValues` sets it, and a value is clamped
  into the range. A `SetValue` before the first `SetMinMaxValues` is lost: the
  value stays 0. The first range then clamps the value without running
  `OnValueChanged`, so a new bar given the range 3 to 10 reads 3. Set the
  range first, as Blizzard's unit frames do in `UnitFrameHealthBar_Update`.
- `OnValueChanged(self, value)` runs at once, even while the bar is hidden,
  and only when the value actually changes. A status bar passes only the
  value. The `userInput` argument is a slider's and is `nil` here, although
  the handler's type says `boolean`.
- `SetMinMaxValues` runs `OnMinMaxChanged(self, min, max)` when the range
  changes, then clamps the value into the new range, which runs
  `OnValueChanged` if the value moves. A maximum below the minimum is an
  error: `max must be >= min`.
- The color is kept with 8 bits per channel, so `SetStatusBarColor(0.2, 0.8,
  0.3)` reads back as about `0.2, 0.8, 0.302`.

`SetStatusBarTexture` with a file path creates the bar's texture, which
`GetStatusBarTexture` returns. Before that, `GetStatusBarTexture` returns
`nil`. The bar's color is that texture's vertex color. A screenshot draws the
texture over the part of the bar the value fills, following `SetOrientation`
and `SetReverseFill`.

The emulator doesn't animate. With an interpolation argument, the bar still
jumps straight to the new value, `GetInterpolatedValue` returns that value
and `IsInterpolating` returns false.

`SetValue` and `SetMinMaxValues` accept secret values. After a secret one,
`GetValue` or `GetMinMaxValues` returns a secret, although its declared type
is `number`. It returns plain numbers again after the next plain `SetValue`
or `SetMinMaxValues`. In a camelot test, `UnitHealth("player")` returns a
secret, so a health bar's `GetValue` returns one too.

## Example

A health bar for the player. The bar takes `UnitHealth` as it is, secret or
not.

```wowlua title=src/healthbar.wowlua
import unitEvent from "wowlua:events"

-- The player's health, in a bar below the character.
export local healthBar = CreateFrame("StatusBar", nil, UIParent)
healthBar:SetSize(200, 16)
healthBar:SetPoint("CENTER", 0, -120)
healthBar:SetStatusBarTexture("Interface\\TargetingFrame\\UI-StatusBar")
healthBar:SetStatusBarColor(0.1, 0.8, 0.2)

local function update()
    -- The range first: a value outside the range is clamped into it.
    healthBar:SetMinMaxValues(0, UnitHealthMax("player"))
    healthBar:SetValue(UnitHealth("player"))
end

@unitEvent("UNIT_HEALTH", "player")
@unitEvent("UNIT_MAXHEALTH", "player")
local function healthChanged(_unit: UnitToken)
    update()
end

update()
```

## Testing

`mock.player:damage` lowers the player's health and fires `UNIT_HEALTH`.
Because the bar's value is a secret, check that it is one with `toBeSecret`,
and check what the bar shows with `toMatchScreenshot`.
`mock.setPlayer(data, { events = true })` changes the player's maximum health
and fires `UNIT_MAXHEALTH`. Without `events = true`, it fires nothing.

```wowlua title=tests/healthbar.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import healthBar from "/healthbar"

@test("the bar follows the player's health")
local function followsHealth()
    mock.setPlayer({ health = 60, maxHealth = 60 })
    mock.player:damage(15)
    expect(healthBar:GetValue()).toBeSecret()
    expect(healthBar).toMatchScreenshot("hurt")
end

@test("the range follows the maximum health")
local function followsMaximum()
    mock.setPlayer({ health = 100, maxHealth = 120 }, { events = true })
    local min, max = healthBar:GetMinMaxValues()
    expect(min).toBe(0)
    expect(max).toBe(120)
end
```

The first run writes the screenshot to
`tests/__screenshots__/healthbar.test.wowlua/`, and later runs compare against
it. Here it shows the bar three quarters full.
