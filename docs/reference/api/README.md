# WoW API reference data

The data behind the website's WoW API reference.

## camelot.json.gz

`camelot.json.gz` describes the WoW API of the camelot flavor (WoW Forever) in
one gzipped JSON file. It holds:

- every function, widget method, event and table (structures, enumerations,
  constants, callback types) that Blizzard's API documentation describes, with
  their arguments, results, payloads and flags. The client's functions that
  the documentation lacks are listed too.
- what each one does, and where that description comes from: Blizzard's
  documentation, warcraft.wiki.gg, or wowlua's own description where neither
  has one.
- how far `wowlua test` emulates each one: implemented, answered from
  recordings of the real client, guessed from the documentation, or not
  implemented. For events, whether the emulated client fires the event itself
  or only a test's `mock.fireEvent` does. These are the same numbers as the
  [emulator coverage report](../../emulator-coverage.md).

The file is generated, so don't edit it. To regenerate it, run:

```sh
cargo xtask api-reference
```

`cargo xtask check` fails when the file is out of date. The output is the same
byte for byte on every run.

## Licence of the descriptions

Descriptions marked `wiki` are adapted from
[warcraft.wiki.gg](https://warcraft.wiki.gg)'s API pages, by its contributors.
They are licensed under the
[Creative Commons Attribution-ShareAlike 4.0 International License](https://creativecommons.org/licenses/by-sa/4.0/),
and this file shares them under the same licence (see
[`LICENSE-warcraft-wiki.txt`](../../../crates/wowlua-api/data/LICENSE-warcraft-wiki.txt)).
A page that shows one must credit warcraft.wiki.gg and link the entry's wiki
page (its `doc.wiki`).

## Notes

`notes/<Name>.md` files in this directory hold hand-written notes. The website
shows a note on the page of the entry it is named after, for example
`notes/C_Container.GetContainerItemInfo.md` or `notes/BAG_UPDATE.md`. A widget
method's note puts a dot in place of the colon, because Windows file names
can't contain one: `notes/Frame.Show.md` for `Frame:Show`. A note is a Markdown
page with these optional sections:

```markdown
## In the emulator

How `wowlua test` answers it, and what it does not do yet.

## Example

A short example of its use.

## Testing

How to test code that uses it with `wowlua test`.
```
