--[==[ A long comment with [[ inner ]] brackets
and several lines ]==]
local s1 = [[
first line skipped]]
local s2 = [==[ ]] ]=] ]==]
local t = {
    [ [[key]] ] = 1, -- long string key
    ['quote"d'] = "it's", "plain";
    nested = { { {} } },
    f = function(...) return ... end,
}
local x = 1
    + 2 -- operator at line start
    * 3
local y = t
    and t.nested
    or nil
local z = (f or print)(
    "a",
    "b"
)
;(function() end)()
local n = - -1 + 2 ^ -3 ^ 2 % 4 .. 5 .. 6
local c = a == b ~= (c < d) == (e >= f)
local esc = "tab\tnl\n\65\066\0067 \\ \' \"" .. 'single \'q\' "d"'
local hex, dec, exp = 0xFF, .5, 1e-3
print(#t, not t, - x)
do local _ = {...} end
while false do break end
for i = 10, 1, -1 do end
if x then elseif y then else end
return t
