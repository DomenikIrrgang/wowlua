# Appendix A. Grammar

EBNF for the complete wowlua syntax. `{ x }` means zero or more repetitions,
`[ x ]` means optional, `|` separates alternatives, and quoted strings are
terminals. Rules marked **(Lua)** are unchanged from the Lua 5.1 reference
manual. Contextual keywords are written in quotes like reserved words. Their
recognition conditions are in [Syntax §1.2.5](../language/01-syntax.md#125-reserved-and-contextual-keywords).

Where the grammar writes `"?" ":"`, the lexer produces the single token `?:`
(maximal munch), and the parser accepts it.

## A.1 Chunks and blocks

```ebnf
chunk        ::= [ BOM ] [ shebang ] { comment } { directive } block
directive    ::= "--!" Name                    (* only before the first token *)
block        ::= { stat [ ";" ] } [ laststat [ ";" ] ]       (* Lua *)
laststat     ::= "return" [ explist ] | "break" | "continue"
```

`continue` is a `laststat`, like `break`: it must be the last statement of
its block (E0114).

## A.2 Statements

```ebnf
stat ::= varlist "=" explist                                  (* Lua *)
       | var compoundop exp                                   (* single target, no optional chain *)
       | functioncall                                         (* Lua *)
       | "do" block "end"                                     (* Lua *)
       | "while" exp "do" block "end"                         (* Lua *)
       | "repeat" block "until" exp                           (* Lua *)
       | "if" exp "then" block { "elseif" exp "then" block } [ "else" block ] "end"
       | "for" typedname "=" exp "," exp [ "," exp ] "do" block "end"
       | "for" typednamelist "in" explist "do" block "end"
       | { decorator } "function" funcname funcbody
       | { decorator } "local" "function" Name funcbody
       | { decorator } "local" localtargets [ "=" explist ]
       | { decorator } constdecl
       | { decorator } classdecl                              (* top level only *)
       | { decorator } interfacedecl
       | { decorator } enumdecl                               (* top level only *)
       | { decorator } typealias
       | { decorator } declarestat                            (* top level only *)
       | { decorator } importstat                             (* top level only *)
       | { decorator } exportstat                             (* top level only *)

compoundop   ::= "+=" | "-=" | "*=" | "/=" | "//=" | "%=" | "^=" | "..="
               | "&=" | "|=" | "<<=" | ">>=" | "??="

constdecl    ::= "const" Name [ ":" type ] "=" exp

localtargets ::= typednamelist
               | "{" destructfield { "," destructfield } [ "," ] "}" [ ":" type ]
destructfield::= Name [ "=" exp ]

typedname    ::= Name [ ":" type ]
typednamelist::= typedname { "," typedname }

funcname     ::= Name { "." Name } [ ":" Name ]               (* Lua *)
funcbody     ::= [ typeparams ] "(" [ parlist ] ")" [ ":" rettype ] block "end"
parlist      ::= param { "," param } [ "," vararg ] | vararg
param        ::= Name [ ":" type ] [ "=" exp ]
vararg       ::= "..." [ ":" ( type | Name "..." ) ]          (* Name "..." : pack *)

exportstat   ::= "export" ( "local" typednamelist "=" explist  (* initializer required *)
                          | "function" Name funcbody            (* plain name only *)
                          | constdecl
                          | classdecl | interfacedecl | enumdecl | typealias )

importstat   ::= "import" importclause "from" String
               | "import" "type" Name [ "as" Name ] { "," importspec } "from" String
importclause ::= "*" "as" Name
               | "_"                                          (* side-effect import, alone *)
               | importspec { "," importspec }
importspec   ::= Name [ "as" Name ]
```

`import type` followed by a name other than `from` is a type-only import.
`import type from "x"` imports a value named `type`.

## A.3 Declarations

```ebnf
classdecl    ::= [ "abstract" ] "class" Name [ typeparams ]
                 [ "extends" typeref ] [ "implements" typeref { "," typeref } ]
                 { classmember } "end"
classmember  ::= { decorator } { modifier } Name [ "?" ] ":" type [ "=" exp ]          (* field *)
               | { decorator } { modifier } "function" Name funcbody                   (* method *)
               | { decorator } "abstract" { modifier } "function" Name sigbody         (* no body *)
               | { decorator } { modifier } "overload" "function" Name sigbody         (* overload signature *)
               | { decorator } { modifier } "get" Name "(" ")" [ ":" type ] block "end"
               | { decorator } { modifier } "set" Name "(" Name [ ":" type ] ")" block "end"
               | { decorator } "constructor" "(" [ parlist ] ")" block "end"
               | { decorator } "overload" "constructor" "(" [ parlist ] ")"                 (* overload signature *)
modifier     ::= "public" | "private" | "protected" | "static" | "readonly" | "override"
sigbody      ::= [ typeparams ] "(" [ parlist ] ")" [ ":" rettype ]

interfacedecl::= "interface" Name [ typeparams ] [ "extends" typeref { "," typeref } ]
                 { ifacemember } "end"
ifacemember  ::= [ "readonly" ] Name [ "?" ] ":" type
               | "function" Name sigbody

enumdecl     ::= "enum" Name "{" [ enumitem { ( "," | ";" ) enumitem } [ "," | ";" ] ] "}"
enumitem     ::= Name [ "=" exp ]

typealias    ::= "type" Name [ typeparams ] "=" type

declarestat  ::= "declare" "function" funcname declsig
               | "declare" "global" Name ":" type
               | "declare" "namespace" Name { nsmember } "end"
               | "declare" "class" Name [ typeparams ] [ "extends" typeref ]
                 [ "implements" typeref { "," typeref } ] { declmember } "end"
               | "declare" "interface" Name [ typeparams ] [ "extends" typeref { "," typeref } ]
                 { ifacemember } "end"                                     (* merges *)
               | "declare" "event" Name "(" [ declparlist ] ")"
               | "declare" "message" Name "(" [ declparlist ] ")"
               | "declare" "module" String { nsmember } "end"
declsig      ::= [ typeparams ] "(" [ declparlist ] ")" [ ":" rettype ]
declparlist  ::= declparam { "," declparam } [ "," vararg ] | vararg
declparam    ::= { decorator } Name [ ":" type ]               (* no defaults; @deferred allowed *)
nsmember     ::= { decorator } "function" Name declsig
               | { decorator } [ "const" ] Name ":" type
               | { decorator } "namespace" Name { nsmember } "end"
               | { decorator } "class" Name [ typeparams ] [ "extends" typeref ] { declmember } "end"
               | { decorator } "interface" Name [ typeparams ] { ifacemember } "end"
               | { decorator } "type" Name [ typeparams ] "=" type
               | { decorator } enumdecl
declmember   ::= { decorator } [ "static" ] "function" Name declsig
               | { decorator } [ "static" ] [ "readonly" ] Name [ "?" ] ":" type

decorator    ::= "@" Name { "." Name } [ args ]               (* args start on the same line *)
```

## A.4 Expressions

```ebnf
exp          ::= "nil" | "false" | "true" | Number | String | "..."
               | function | prefixexp | tableconstructor
               | interpstring
               | exp binop exp | unop exp
               | exp "::" type                                (* full type, see below *)

prefixexp    ::= var | functioncall | "(" exp ")" | optchain   (* Lua, plus optchain *)
var          ::= Name | prefixexp "[" exp "]" | prefixexp "." Name   (* Lua; assignable *)
optchain     ::= prefixexp "?." Name
               | prefixexp "?." "[" exp "]"
               | prefixexp "?:" Name args
               | prefixexp "?." "(" [ explist ] ")"
functioncall ::= prefixexp args | prefixexp ":" Name args     (* Lua *)
args         ::= "(" [ explist ] ")" | tableconstructor | String   (* Lua *)
function     ::= "function" funcbody

interpstring ::= "`" { interpchar | "{" exp "}" } "`"

binop        ::= "+" | "-" | "*" | "/" | "//" | "^" | "%" | ".."
               | "<" | "<=" | ">" | ">=" | "==" | "~="
               | "and" | "or" | "??"
               | "&" | "|" | "~" | "<<" | ">>"
unop         ::= "-" | "not" | "#" | "~"
```

- Precedence and associativity are in
  [Syntax §1.4.6](../language/01-syntax.md#146-operator-precedence). Mixing `??`
  with `and`/`or` without parentheses is a syntax error (E0110).
- `::` applies to the operand on its left at unary level. The type on its
  right is a **full** `type` and extends as far as the type grammar allows,
  including `|` and `&`. A binary operator applied to a cast result needs
  parentheses ([Syntax §1.4.3](../language/01-syntax.md#143-type-assertions)).
- Optional chains are never assignment targets (`var` excludes `optchain`,
  E0113).
- An interpolated string is not a valid `args` form: `f`…`` is not a call.

## A.5 Types

```ebnf
type         ::= uniontype
uniontype    ::= [ "|" ] isecttype { "|" isecttype }
isecttype    ::= postfixtype { "&" postfixtype }
postfixtype  ::= primarytype { "?" | "[" "]" }
primarytype  ::= "nil" | "true" | "false" | "function" | String | Number
               | typeref
               | "typeof" "(" exp ")"
               | tabletype
               | functype
               | "(" type ")"
typeref      ::= Name { "." Name } [ typeargs ]
typeargs     ::= "<" typearg { "," typearg } ">"              (* ">>", ">=", ">>=" are split *)
typearg      ::= type | typepack
typeparams   ::= "<" typeparam { "," typeparam } ">"
typeparam    ::= Name [ "extends" type ]
               | Name "..."                                   (* pack parameter *)

tabletype    ::= "{" [ tablefield { ( "," | ";" ) tablefield } [ "," | ";" ] ] "}"
tablefield   ::= Name [ "?" ] ":" type
               | "[" type "]" ":" type                        (* "[" String "]" names a field *)
               | type                                         (* only as the sole field: array *)

functype     ::= [ typeparams ] "(" [ paramtypes ] ")" "->" rettype
paramtypes   ::= paramtype { "," paramtype } [ "," varargtype ] | varargtype
paramtype    ::= [ Name ":" ] type
varargtype   ::= "..." [ ":" ] type
               | "..." ":" Name "..."                         (* pack *)

typepack     ::= Name "..."                                   (* a pack parameter *)
               | "(" [ type { "," type } [ "," varargtype ] ] ")"   (* pack literal *)

rettype      ::= type
               | "(" ")"
               | "(" type "," type { "," type } [ "," varargtype ] ")"
               | "(" type { "," type } "," varargtype ")"
               | "(" varargtype ")"
               | Name "..."                                   (* pack *)
               | Name "is" type                               (* type predicate; "is" on the same line *)
```

Notes:

- `"[" String "]" ":" type`, for example `{ ["end"]: number }`, declares a
  named field whose key is a reserved word or not an identifier.
- `(T)` in a type-argument position is a parenthesized type when the
  corresponding parameter is an ordinary type parameter, and a pack literal when
  it is a pack parameter. The checker resolves this from the declaration.
- The return type of a function type extends as far as possible:
  `() -> number | nil` returns `number | nil`.
- The type predicate form requires `is` on the same line as the parameter name,
  so a function body starting with a call to a function named `is` stays
  unambiguous.

## A.6 Lexical additions

```ebnf
Number       ::= LuaNumber                                    (* Lua 5.1 numerals *)
               | decnum | hexnum | binnum                     (* with digit separators *)
decnum       ::= digits [ "." [ digits ] ] [ exponent ] | "." digits [ exponent ]
exponent     ::= ( "e" | "E" ) [ "+" | "-" ] digits
digits       ::= digit { [ "_" ] digit }
hexnum       ::= "0" ( "x" | "X" ) hexdigit { [ "_" ] hexdigit }
binnum       ::= "0" ( "b" | "B" ) bindigit { [ "_" ] bindigit }
interpchar   ::= any byte except "`", "{", "}", "\"
               | "\" ( luaescape | "x" hexdigit hexdigit | "u{" hexdigit { hexdigit } "}"
                     | "z" | "{" | "}" | "`" )
```

A `_` may only appear between two digits of the same run (E0103). A lone `}`
in an interpolated string is an error (E0116).

New operator tokens (`@ ? ?? ?. ?: :: -> += -= *= /= //= %= ^= ..= &= |= <<=
>>= ??= // & | ~ << >>` and the backtick) are listed in
[Syntax §1.2.1](../language/01-syntax.md#121-new-tokens).
