# Custom Discord Highlighter

This is a bot i made to highlight URSL, a little language i made. It reacts to any message that looks like so:

````
```ursl
bits 8
func $main {
    // code
}
```
````

And responds with an ANSI-formatted syntax highlighting of that code:

````
```ansi
[35mbits[0m [36m8[0m
[35mfunc[0m [33m$main[0m [30m{[0m
  [30m// code[0m
[30m}[0m
```
````

It also supports URCL, and as the bot's name suggests, i'm happy to add support for more languages, such as your own ISA.

The above may not look great in wherever you're viewing this, but in discord that renders pretty nicely:

![The above code, rendered in discord](example.png)

This bot is easily extensible to any tree-sitter grammar, but currently it only renders ``ursl`` codeblocks, ignoring all else.

If you wanna run this bot locally, create ``token`` file with the token in the root of this repository, and then just ``cargo run``