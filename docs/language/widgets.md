# Frames as classes

A class can extend a frame type such as `Frame`, `Button`, `StatusBar` or
`EditBox`. Its instances are real frames made with `CreateFrame`, with your
fields and methods next to the game's own. This page shows how to build
widgets, windows and XML templates this way.

These classes compile to the mixin pattern Blizzard's own UI code uses: the
frame keeps the metatable the game gave it, and your methods are copied onto
it. The rules are in
[Classes §3.2](../../spec/language/03-classes.md#32-widget-classes) of the
specification.

## A button that counts its clicks

In the first class of the chain that extends a frame type, `super(...)` creates
the frame. It takes the arguments `CreateFrame` takes after the type: a global
name or `nil`, the parent, a template and an ID. This button comes from the
widget demo in [`examples/widget-demo`](../../examples/widget-demo):

```wowlua title=src/ui/counter.wowlua
export class CounterButton extends Button
    count: number = 0
    label: string
    onCount: (number) -> ()

    constructor(parent: Frame, label: string, onCount: (number) -> ())
        super(nil, parent, "UIPanelButtonTemplate")
        self.label = label
        self.onCount = onCount
        self:SetSize(140, 24)
        self:refresh()
    end

    @script("OnClick")
    function increment()
        self.count += 1
        self:refresh()
        self.onCount(self.count)
    end

    function refresh()
        self:SetText(`{self.label}: {self.count}`)
    end
end
```

`CounterButton.new(...)` creates one. Your methods and the game's are called
the same way:

```wowlua title=src/demo.wowlua
import CounterButton from "./ui/counter"

local clicks = CounterButton.new(UIParent, "Clicks", function(count)
    print(`clicked {count} times`)
end)
clicks:SetPoint("CENTER")   -- a method of the game's Button
clicks:increment()          -- a method of CounterButton
```

- `self` is the frame. All of the game's
  [widget methods](../reference/api.md) are on it, typed, next to your own
  members.
- A handler you set with `self:SetScript(...)` gets your class as `self`, not
  the plain `Button`, so your fields are typed inside it.
- Before `super(...)` there is no frame, so `self` can't be used yet. The
  compiler reports it:

```wowlua fails=E3001
class Badge extends Frame
    constructor(parent: Frame)
        self:SetSize(20, 20)
        super(nil, parent)
    end
end
```

```text
error[E3001]: `self` is used before `super(...)`
  ┌─ src/ui/badge.wowlua:3:9
  │
3 │         self:SetSize(20, 20)
  │         ^^^^ used here
```

A frame class without a constructor of its own takes the `CreateFrame`
arguments directly: `Badge.new("MyBadge", UIParent)`.

## Handle scripts with @script

`@script("OnClick")` above makes `increment` the button's `OnClick` handler.
`self` is the frame, and the script's arguments are the method's parameters.
A handler set this way replaces any other handler of that script, as
`SetScript` does. Write `@script("OnShow", { hook = true })` to add it with
`HookScript` instead and keep a handler the template already has.

```wowlua title=src/ui/input.wowlua
-- A single-line input that reports what was entered.
export class InputBox extends EditBox
    onSubmit: (string) -> ()

    constructor(parent: Frame, onSubmit: (string) -> ())
        super(nil, parent, "InputBoxTemplate")
        self.onSubmit = onSubmit
        self:SetSize(180, 20)
        self:SetAutoFocus(false)
    end

    @script("OnEnterPressed")
    function submit()
        local text = self:GetText()
        if text ~= "" then
            self.onSubmit(text)
        end
        self:SetText("")
        self:ClearFocus()
    end

    @script("OnEscapePressed")
    function cancel()
        self:ClearFocus()
    end
end
```

The handlers are set after the constructor has finished. Setting up the frame
in the constructor (its text, its value, showing it) never runs them early. If
the constructor sets a handler for the same script itself, the `@script`
method replaces it.

The compiler checks the script name against the frame type: a script the
frame type doesn't have, such as a misspelled `OnClik` or `OnClick` on an
`EditBox`, is error E2001. Where the [WoW API reference](../reference/api.md)
gives the script's arguments, the method's parameters are checked against
them too, as for a function passed to `SetScript`.

## Variations on a widget

Frame classes can extend each other. Only the first one in the chain creates
the frame; the classes below it add fields and methods. An `@script`
registration is inherited, and it calls the most-derived version of its
method, so overriding the method is enough:

```wowlua title=src/ui/limited.wowlua
import CounterButton from "./counter"

-- A counter button that starts over after `limit` clicks.
export class LimitedCounterButton extends CounterButton
    limit: number = 10

    override function increment()
        if self.count >= self.limit then
            self.count = 0
        end
        super:increment()
    end
end
```

`super:increment()` calls `CounterButton`'s version. In the first frame class
of a chain, `super:m(...)` calls the game's own method of that name.

## A whole window

A window is a frame class too, built from the others. It keeps its parts in
typed fields, so a call such as `self.progress:setProgress(count)` is checked
like any other. First a progress bar:

```wowlua title=src/ui/progress.wowlua
export class ProgressBar extends StatusBar
    constructor(parent: Frame, max: number)
        super(nil, parent)
        self:SetSize(260, 14)
        self:SetStatusBarTexture("Interface\\TargetingFrame\\UI-StatusBar")
        self:SetMinMaxValues(0, max)
        self:SetValue(0)
    end

    function setProgress(value: number)
        self:SetValue(value)
    end
end
```

Then the window that holds it, the counter and the input box:

```wowlua title=src/ui/window.wowlua
import LimitedCounterButton from "./limited"
import InputBox from "./input"
import ProgressBar from "./progress"

export class DemoWindow extends Frame
    progress: ProgressBar
    status: FontString

    constructor()
        super("WidgetDemoWindow", UIParent)
        self:SetSize(300, 380)
        self:SetPoint("CENTER")
        self:SetMovable(true)
        self:EnableMouse(true)
        self:RegisterForDrag("LeftButton")
        self:SetScript("OnDragStart", self.StartMoving)
        self:SetScript("OnDragStop", self.StopMovingOrSizing)
        tinsert(UISpecialFrames, "WidgetDemoWindow")   -- Escape closes it

        self.progress = ProgressBar.new(self, 10)
        self.progress:SetPoint("TOPLEFT", 16, -48)

        local counter = LimitedCounterButton.new(self, "Clicks", function(count)
            self.progress:setProgress(count)
            self:setStatus(`clicked {count} times`)
        end)
        counter:SetPoint("TOPLEFT", 16, -70)

        local input = InputBox.new(self, function(text)
            self:setStatus(`you typed {text}`)
        end)
        input:SetPoint("TOPLEFT", 22, -104)

        self.status = self:CreateFontString(nil, "OVERLAY", "GameFontNormalSmall")
        self.status:SetPoint("BOTTOM", 0, 12)
        self:Hide()
    end

    function setStatus(text: string)
        self.status:SetText(text)
    end

    function toggle()
        self:SetShown(not self:IsShown())
    end
end
```

A slash command opens it. The window is created the first time and only
shown or hidden after that:

```wowlua title=src/main.wowlua
import slash from "wowlua:slash"
import DemoWindow from "./ui/window"

local window: DemoWindow? = nil

@slash("/wdemo")
local function toggleWindow()
    window ??= DemoWindow.new()
    window:toggle()
end
```

> [!TIP]
> You can test the window without the game. In a test, `mock.slash("/wdemo")`
> types the command, `mock.click(frame)` clicks a button, and
> `expect(frame).toMatchScreenshot()` compares how it looks with a saved
> screenshot, all in wowlua's emulated client. See
> [Testing your addon](../guide/testing.md).

## Classes behind XML templates

To lay a frame out in XML, give the class `@global("Name")` and use that name
as the template's `mixin`, and have the template call `OnLoad` with
`<OnLoad method="OnLoad"/>`. For every frame made from the template, that
call first runs your class's field initializers and sets its `@script`
handlers, then runs your own `OnLoad` method, if the class has one. Children
with a `parentKey` are fields of the frame, so declare them in the class to
give them a type:

```wowlua title=src/ui/row.wowlua
@global("WidgetDemoRowMixin")
export class Row extends Frame
    index: number = 0
    Label: FontString   -- the template's child with parentKey="Label"

    function OnLoad()
        self:SetHeight(20)
        self.Label:SetText("(empty)")
    end

    function setEntry(index: number, text: string)
        self.index = index
        self.Label:SetText(`{index}. {text}`)
    end
end
```

The template, in `src/ui/templates.xml`:

```xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
  <Frame name="WidgetDemoRowTemplate" virtual="true" mixin="WidgetDemoRowMixin">
    <Size x="260" y="20"/>
    <Layers>
      <Layer level="ARTWORK">
        <FontString parentKey="Label" inherits="GameFontHighlight" justifyH="LEFT">
          <Anchors>
            <Anchor point="LEFT" x="4"/>
          </Anchors>
        </FontString>
      </Layer>
    </Layers>
    <Scripts>
      <OnLoad method="OnLoad"/>
    </Scripts>
  </Frame>
</Ui>
```

Create rows from the template with `CreateFrame` and cast the result to the
class. `import _ from "./templates.xml"` binds no names; it makes sure the XML
file loads before the module that uses its template:

```wowlua fragment
import Row from "./row"
import _ from "./templates.xml"

local function addRow(list: Frame, index: number, text: string): Row
    local row = CreateFrame("Frame", nil, list, "WidgetDemoRowTemplate") :: Row
    row:SetPoint("TOPLEFT", 0, -(index - 1) * 20)
    row:setEntry(index, text)
    return row
end
```

## Frames you didn't create

Some frames already exist before your code sees them: the buttons a secure
group header makes, a frame built from someone else's template, or a font
string from `CreateFontString`. `adopt` applies your class to such a frame
instead of creating a new one. Your constructor runs on it, with the
constructor's arguments after the frame, and your `@script` handlers are set,
as with `new`. The frame must be of the type the class extends.

```wowlua title=src/ui/label.wowlua
export class Label extends FontString
    function showGold(copper: number)
        self:SetText(GetMoneyString(copper))
    end
end

export function goldLabel(parent: Frame): Label
    local label = Label.adopt(parent:CreateFontString(nil, "OVERLAY", "GameFontNormal"))
    label:showGold(GetMoney())
    return label
end
```

A class that extends a region type (`FontString`, `Texture`, `Line` or
`MaskTexture`) has no `new` at all, because `CreateFrame` can't make regions.
It is always built with `adopt`.

## What is different from plain classes

- The game owns the frame's metatable, so a frame class can't have `get`/`set`
  accessors (E3043) or metamethods such as `__tostring` (E3007).
- Frames take any field, as Lua tables do. A field your class doesn't declare
  reads as `any`. Turn on warning W2063 in `wowlua.json`
  (`"check": { "lints": { "W2063": "warn" } }`) to have such reads reported,
  which catches typos like `button.cuont`.
- Every instance has a `__class` field, so `reflect.isInstance(frame,
  CounterButton)` and `reflect.classOf(frame)` from `wowlua:reflect` work on
  frames too, for example when you walk a frame's children.
- A method replaced on the class later (a plugin patching `refresh`) reaches
  the frames created after that. Frames created before keep the method they
  were given.

Plain classes, inheritance and methods are on [Classes](classes.md).
