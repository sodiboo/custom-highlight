# Custom Discord Highlighter

This is a bot i originally made to highlight URSL, a little language i made. It also supports URCL, and i'm happy to add support for more languages, such as your own ISA. Just contact me if you'd like to add your language. The main server this bot is intended to run in is the [URCL discord server](https://discord.gg/Nv8jzWg5j8), but you're free to fork this, clone this, do whatever, and run it on your own servers. You can also [invite the bot](https://discord.com/api/oauth2/authorize?client_id=980132414305214505&permissions=2048&scope=bot%20applications.commands), but please self-host it if possible so that i won't run into the guild limit for privileged intents. Or do invite it, maybe i can get the bot verified eventually.

It reacts to any message that looks like so:

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

The above may not look great in wherever you're viewing this, but in discord that renders pretty nicely:

![The above code, rendered in discord](example+highlight.png)

Now of course, that may not look too great on mobile, because (at least on iOS) discord mobile does not support syntax highlighting whatsoever, including ANSI. For that, there's the ``+render`` command:

![The above code, highlighted and rendered by my bot, to look like a discord codeblock](example+render.jpg)

This bot is easily extensible to any tree-sitter grammar. It responds to any message that is a codeblock (in a language it knows) and optionally a command it recognizes, defaulting to ``+highlight`` for languages that are determined to be "highlight by default".

- ``+highlight`` will print the ANSI highlighting of the codeblock, and chunk it into multiple messages if it's too long to fit in a single message (since the ANSI escape codes can easily increase the length fast, with about 8 extra chars per token in the tree)
- ``+render`` will render the highlighted text to an image, intended for mobile use where ANSI highlighting is not supported
- ``+parse`` will just parse the codeblock's contents and dump the tree in a readable format, highlighted nicely and everything
- ``+pparse`` (plain parse) is the same as ``+parse``, but does not color the output. It is primarily for use on mobile.

The color scheme of this bot's highlighting is generally based loosely on vscode's default theme of Dark+, with some compromises being made. Most notably, all literals are ``CYAN`` to match discord's default language settings.

The functionality of the bot as described above is also implemented through interactions, you can right click any message with a codeblock to get an ephemeral response (that means it doesn't spam the channel with a bunch of messages) and if you send a codeblock without a command, you get buttons to choose what to do with it.

If you wanna run this bot locally, create ``token`` file with the token in the root of this repository, add a font named ``font.ttf`` (i use [Fira Code](https://github.com/tonsky/FiraCode)) and then just ``cargo run``.

---

Avatar by [tezar tantular](https://thenounproject.com/icon/coding-2996800/0). I haven't modified the icon outside of the preview options The Noun Project provides. 