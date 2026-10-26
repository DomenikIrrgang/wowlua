# x5xxx · codegen

## E5001

The code goes past one of Lua 5.1's fixed limits: at most 200 locals in one
function (the file's main chunk counts as one), 60 upvalues per function, 250
registers, 200 nested blocks and subexpressions, and a bounded number of
constants and jump distance. WoW would refuse to load such a file, so this
stays an error even in a plain Lua file. wowlua compiles every output file
with Lua 5.1 to find these, and counts the locals the compiler adds (imports,
hoisted declarations) as well.

Split the function, or keep related values in a table instead of separate
locals. The upvalue limit often comes from a function that uses many
top-level locals of its file: below, `show` captures 61 of them.

```wowlua fails
local v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16, v17, v18, v19, v20,
      v21, v22, v23, v24, v25, v26, v27, v28, v29, v30, v31, v32, v33, v34, v35, v36, v37, v38, v39, v40,
      v41, v42, v43, v44, v45, v46, v47, v48, v49, v50, v51, v52, v53, v54, v55, v56, v57, v58, v59, v60,
      v61
local function show()
    print(v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14, v15, v16, v17, v18, v19, v20,
        v21, v22, v23, v24, v25, v26, v27, v28, v29, v30, v31, v32, v33, v34, v35, v36, v37, v38, v39, v40,
        v41, v42, v43, v44, v45, v46, v47, v48, v49, v50, v51, v52, v53, v54, v55, v56, v57, v58, v59, v60,
        v61)
end
show()
```

```wowlua compiles
local values = {}
for i = 1, 61 do
    values[i] = i
end
local function show()
    print(unpack(values))
end
show()
```

## E5002

The finished AddOn would not load as it is: an XML file references a file
that doesn't exist (`<Script file>`, `<Include file>`), an XML file is not
well-formed, a TOC entry names a file that is not part of the output, a Lua
or XML file is shipped but nothing loads it, or a `build.shared-files` entry
is not a source file. wowlua checks the output before it writes it, because in
game a missing file only shows up as an error when the AddOn loads.

Fix the path (it is relative to the XML file's folder), add the missing file,
or remove the reference.

```xml fails title=src/ui.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <Script file="missing.lua"/>
</Ui>
```

```xml compiles title=src/ui.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <Script file="main.lua"/>
</Ui>
```
```lua compiles title=src/main.lua
print("loaded")
```

Related: W5003, W5004

## W5003

With `compat.packager-keywords` (a project converted from the BigWigs
packager), an XML reference inside a `<!--@debug@-->` block that doesn't
resolve to a source file is left out of debug builds instead of failing the
build with E5002. Such blocks often load developer files that another addon
of the project provides. Release builds comment the block out, as the
packager does.

The warning reminds you that the file is not loaded. Add the file if the
build should load it, or ignore the warning if another addon provides it.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "compat": { "packager-keywords": true }
}
```
```xml fails title=src/ui.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <!--@debug@-->
    <Script file="DevTools.lua"/>
    <!--@end-debug@-->
</Ui>
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "compat": { "packager-keywords": true }
}
```
```xml compiles title=src/ui.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <!--@debug@-->
    <Script file="DevTools.lua"/>
    <!--@end-debug@-->
</Ui>
```
```lua compiles title=src/DevTools.lua
print("dev tools loaded")
```

Related: E5002

## W5004

An XML file of the build loads a file that exists in your sources but is not
part of this build's output: `build.flavor-files` restricts it to flavors
this build doesn't target, or `build.exclude` or the profile's `exclude`
leaves it out (a developer tool left out of `release`). In game the
reference fails to load in those builds.

Restrict the XML file to the same flavors, or let the generated TOC load the
file (it lists each file only for the flavors it belongs to) and drop the
reference. For an excluded file, load it from code that the same builds
leave out, or stop excluding it. A reference to a file that doesn't exist at
all is E5002.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "flavor-files": { "src/Vanilla.lua": ["vanilla"] } }
}
```
```xml fails title=src/ui.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <Script file="Vanilla.lua"/>
</Ui>
```
```lua fails title=src/Vanilla.lua
print("vanilla only")
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "flavor-files": { "src/Vanilla.lua": ["vanilla"] } }
}
```
```lua compiles title=src/Vanilla.lua
print("vanilla only")
```

Related: E5002
