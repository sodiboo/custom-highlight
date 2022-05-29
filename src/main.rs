use std::collections::HashMap;

use lazy_static::lazy_static;
use serenity::{
    async_trait,
    model::channel::{Channel, Message},
    prelude::*,
};
use tree_sitter::{Language, Parser};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

macro_rules! map {
    (@key $name:literal) => { $name };
    (@key $name:ident) => { stringify!($name) };
    (@m $callback:ident ($($args:tt)*) $($k:tt => $v:expr,)*) => { $callback!($($args)* $((map!(@key $k), $v),)*) };
    (@arr $($t:tt)*) => { [$($t)*] };
    ($($t:tt)*) => { map!(@m map (@arr) $($t)*) };

}
macro_rules! unzip {
    ($(($a:expr, $b:expr),)*) => {
        (&[$($a),*], &[$($b),*])
    };
    ($($t:tt)*) => {
        map!(@m unzip () $($t)*)
    };
}

macro_rules! lang {
    ($pkg:ident; $($t:tt)*) => {{
        let language = $pkg::language();
        let mut highlight = HighlightConfiguration::new(
            language,
            $pkg::HIGHLIGHTS_QUERY,
            "",
            "",
        ).unwrap();
        let (recognized_names, formats): (&[&str], &[&str]) = unzip![$($t)*];
        highlight.configure(recognized_names);
        LanguageConfig {
            highlight,
            formats,
            language,
        }
    }};
}

struct LanguageConfig {
    highlight: HighlightConfiguration,
    formats: &'static [&'static str],
    language: Language,
}

macro_rules! colors {
    ($($name:ident = $value:literal)*) => {
        $(const $name: &str = concat!("\u{001b}[", $value, "m");)*
    }
}

colors! {
    GRAY = 30
    RED = 31
    GREEN = 32
    YELLOW = 33
    BLUE = 34
    PINK = 35
    CYAN = 36
    WHITE = 37
}

lazy_static! {
    static ref LANGUAGES: HashMap<&'static str, LanguageConfig> = HashMap::from(map![
        ursl => lang![tree_sitter_ursl;
            comment => GRAY,
            number => CYAN,
            port => GREEN,
            label => YELLOW,
            "label.data" => YELLOW,
            function => YELLOW,
            macro => PINK,
            address => CYAN,
            register => CYAN,
            string => CYAN,
            "string.special" => CYAN,
            instruction => BLUE,
            property => RED,
            keyword => PINK,
            "punctuation.delimiter" => GRAY,
            "punctuation.bracket" => GRAY,
        ],
        "urcl" => lang![tree_sitter_urcl;
            header => PINK,
            constant => YELLOW,
            number => CYAN,
            relative => CYAN,
            port => GREEN,
            macro => PINK,
            comment => GRAY,
            label => YELLOW,
            register => CYAN,
            instruction => BLUE,
            string => CYAN,
            "string.special" => CYAN,
            operator => GRAY,
            "punctuation.bracket" => GRAY,
            identifier => WHITE,
            "identifier.placeholder" => WHITE,
        ],
    ]);
}

#[tokio::main]
async fn main() {
    let token = include_str!("../token");
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .await
        .expect("Error creating client");
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {why:?}");
    }
}

struct Handler;

async fn send(ctx: &Context, channel: &Channel, content: &str) -> serenity::Result<Message> {
    match channel {
        Channel::Guild(c) => c.send_message(&ctx, |msg| msg.content(content)).await,
        Channel::Private(c) => c.send_message(&ctx, |msg| msg.content(content)).await,
        &_ => panic!("bad channel"),
    }
}

async fn chunk_ansi(ctx: Context, channel: Channel, content: &str) -> serenity::Result<()> {
    let mut chunk = String::new();
    for line in content.split("\n") {
        if "```ansi\n".len() + chunk.len() + line.len() + "\n```".len() > 2000 {
            if "```ansi\n".len() + line.len() + "\n```".len() > 2000 {
                send(&ctx, &channel, "Line is too long").await?;
                return Ok(());
            }
            chunk.insert_str(0, "```ansi\n");
            chunk.push_str("```");
            send(&ctx, &channel, &chunk).await?;
            chunk = String::new();
        }
        chunk.push_str(line);
        chunk.push('\n');
    }
    if !chunk.is_empty() {
        chunk.insert_str(0, "```ansi\n");
        chunk.push_str("```");
        send(&ctx, &channel, &chunk).await?;
    }
    Ok(())
}

// empty, but don't remove, in case there is ever a namespace collision with another bot doing the same thing as this bot
// the contents of this array will NOT be allowed to highlight without the +highlight prefix
const NO_HIGHLIGHT_BY_DEFAULT: &[&str] = &[];

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if let Some((op, lang, code)) = codeblock(&msg.content) {
            if let Some(config) = LANGUAGES.get(lang) {
                let channel = msg.channel(&ctx).await.unwrap();
                let op = match op {
                    "" if !NO_HIGHLIGHT_BY_DEFAULT.contains(&lang) => "+highlight",
                    _ => op,
                };
                match op {
                    "+highlight" => {
                        if let Some(formatted) = syntax_highlight(config, code) {
                            chunk_ansi(ctx, channel, &formatted).await.unwrap()
                        }
                    }
                    "+parse" => {
                        if let Some(mut sexp) = sexp(config, code) {
                            sexp.insert_str(0, "```");
                            sexp.push_str("```");
                            send(&ctx, &channel, &sexp).await.unwrap();
                        }
                    }
                    _ => (),
                }
            }
        }
    }
}

fn codeblock(content: &str) -> Option<(&str, &str, &str)> {
    let content = content.trim_end();
    if !content.ends_with("\n```") {
        return None
    }
    let content = &content[..(content.len() - 4)];
    let (before, content) = content.split_once("```")?;
    // multiple codeblocks, nontrivial, so abort
    if content.contains("```") {
        return None
    }
    let (lang, code) = content.split_once("\n")?;
    if code.trim().is_empty() {
        return None
    }
    Some((before.trim(), lang, code))
}

fn syntax_highlight(config: &LanguageConfig, code: &str) -> Option<String> {
    let mut output = String::new();
    let mut highlighter = Highlighter::new();
    for event in highlighter
        .highlight(&config.highlight, code.as_bytes(), None, |_| None)
        .ok()?
    {
        output += match event.ok()? {
            HighlightEvent::HighlightStart(Highlight(u)) => config.formats[u],
            HighlightEvent::Source { start, end } => &code[start..end],
            HighlightEvent::HighlightEnd => "\u{001b}[0m",
        }
    }
    Some(output)
}

fn sexp(config: &LanguageConfig, code: &str) -> Option<String> {
    let mut parser = Parser::new();
    parser.set_language(config.language).ok()?;
    let tree = parser.parse(code, None)?;
    Some(tree.root_node().to_sexp())
}
