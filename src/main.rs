mod render;
use std::collections::HashMap;

use const_format::concatcp;
use hex_literal::hex;
use image::{codecs::png::PngEncoder, ImageEncoder, Rgba};
use lazy_static::lazy_static;
use render::render;
use serenity::{
    async_trait,
    builder::CreateMessage,
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
            concatcp!("(ERROR) @error\n", $pkg::HIGHLIGHTS_QUERY),
            "",
            "",
        ).unwrap();
        let (recognized_names, formats): (&[&str], &[Color]) = unzip![error => ERROR, $($t)*];
        highlight.configure(recognized_names);
        LanguageConfig {
            highlight,
            formats,
            language,
        }
    }};
}

pub trait ErrAs<E> {
    type Err;
    fn err_as(self, err: E) -> Self::Err;
}

impl<T, E, U> ErrAs<U> for Result<T, E> {
    type Err = Result<T, U>;
    fn err_as(self, err: U) -> Result<T, U> {
        match self {
            Ok(ok) => Ok(ok),
            Err(_) => Err(err),
        }
    }
}

pub const TS_ERROR: &str = "internal error from tree-sitter (not a syntax error)";

pub struct LanguageConfig {
    highlight: HighlightConfiguration,
    formats: &'static [Color],
    language: Language,
}

#[derive(Clone, Copy, Debug)]
struct Color {
    ansi: &'static str,
    rgb: Rgba<u8>,
}

macro_rules! colors {
    ($($name:ident = $value:literal, $hex:literal)*) => {
        $(const $name: Color = Color { ansi: concat!("\u{001b}[", $value, "m"), rgb: Rgba(hex!($hex)) };)*
    }
}

// Note that there are not ANSI names, they are names that fit the specific colors discord uses for the relevant ansi code.
colors! { // bg:
    // last 2 digits is alpha
    ERROR = "31;4", "ff0000ff"
    RESET = 0, "b9bbbeff"
    GRAY = 30, "4f545cff"
    RED = 31, "dc322fff"
    GREEN = 32, "859900ff"
    YELLOW = 33, "b58900ff"
    BLUE = 34, "268bd2ff"
    PINK = 35, "d33682ff"
    CYAN = 36, "2aa198ff"
    WHITE = 37, "ffffffff"
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
        urcl => lang![tree_sitter_urcl;

            comment => GRAY,
            header => PINK,
            constant => YELLOW,
            number => CYAN,
            relative => CYAN,
            port => GREEN,
            macro => PINK,
            label => YELLOW,
            register => CYAN,
            "register.special" => CYAN,
            instruction => BLUE,
            string => CYAN,
            "string.special" => CYAN,
            operator => GRAY,
            "punctuation.bracket" => GRAY,
            identifier => WHITE,
            "identifier.placeholder" => WHITE,
        ],
        phinix => lang![tree_sitter_phinix;
            comment => GRAY,
            segment => RED,
            param => GREEN,
            label => YELLOW,
            number => CYAN,
            keyword => PINK,
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

async fn send<'a>(
    ctx: &Context,
    channel: &Channel,
    f: impl for<'b> FnOnce(&'b mut CreateMessage<'a>) -> &'b mut CreateMessage<'a>,
) -> serenity::Result<Message> {
    match channel {
        Channel::Guild(c) => c.send_message(&ctx, f).await,
        Channel::Private(c) => c.send_message(&ctx, f).await,
        &_ => panic!("bad channel"),
    }
}

async fn send_str(ctx: &Context, channel: &Channel, content: &str) -> serenity::Result<Message> {
    send(ctx, channel, |msg| msg.content(content)).await
}

async fn chunk_ansi(ctx: &Context, channel: &Channel, content: &str) -> serenity::Result<()> {
    let mut chunk = String::new();
    for line in content.split("\n") {
        if "```ansi\n".len() + chunk.len() + line.len() + "\n```".len() > 2000 {
            if "```ansi\n".len() + line.len() + "\n```".len() > 2000 {
                send_str(ctx, channel, "Line is too long").await?;
                return Ok(());
            }
            chunk.insert_str(0, "```ansi\n");
            chunk.push_str("```");
            send_str(ctx, channel, &chunk).await?;
            chunk = String::new();
        }
        chunk.push_str(line);
        chunk.push('\n');
    }
    if !chunk.is_empty() {
        chunk.insert_str(0, "```ansi\n");
        chunk.push_str("```");
        send_str(ctx, channel, &chunk).await?;
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
                if let Err(error) = command(
                    &ctx,
                    &channel,
                    &msg,
                    match op {
                        "" if !NO_HIGHLIGHT_BY_DEFAULT.contains(&lang) => "+highlight",
                        _ => op,
                    },
                    config,
                    code,
                )
                .await
                {
                    msg.reply(&ctx, error).await.unwrap();
                }
            }
        }
    }
}

async fn command(
    ctx: &Context,
    channel: &Channel,
    msg: &Message,
    op: &str,
    config: &LanguageConfig,
    code: &str,
) -> Result<(), &'static str> {
    match op {
        "+highlight" => {
            println!("{} ran +highlight", msg.author);
            let formatted = syntax_highlight(config, code)?;
            chunk_ansi(ctx, channel, &formatted).await.unwrap()
        }
        "+parse" => {
            println!("{} ran +parse", msg.author);
            let sexp = sexp(config, code)?;
            chunk_ansi(ctx, channel, &sexp).await.unwrap();
        }
        "+render" => {
            println!("{} ran +render", msg.author);
            // normalize newlines to \n
            let code = code
                .lines()
                .fold(String::new(), |out, line| out + "\n" + line);
            // ensure no leading or trailing newlines
            let code = code.trim_matches('\n');

            let image = render(config, code)?;
            let mut buffer = Vec::new();
            let png = PngEncoder::new(&mut buffer);
            png.write_image(
                &image,
                image.width(),
                image.height(),
                image::ColorType::Rgba8,
            )
            .err_as("Error when encoding the image")?;
            let bytes = &buffer[..];
            println!("encoded png ({} B)", bytes.len());
            send(ctx, channel, |msg| msg.add_file((bytes, "code.png")))
                .await
                .unwrap();
            println!("sent img");
        }
        _ => (),
    }
    Ok(())
}

fn codeblock(content: &str) -> Option<(&str, &str, &str)> {
    let content = content.trim_end();
    if !content.ends_with("\n```") {
        return None;
    }
    let content = &content[..(content.len() - 4)];
    let (before, content) = content.split_once("```")?;
    // multiple codeblocks, nontrivial, so abort
    if content.contains("```") {
        return None;
    }
    let (lang, code) = content.split_once("\n")?;
    if code.trim().is_empty() {
        return None;
    }
    Some((before.trim(), lang, code))
}

fn syntax_highlight(config: &LanguageConfig, code: &str) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut highlighter = Highlighter::new();
    for event in highlighter
        .highlight(&config.highlight, code.as_bytes(), None, |_| None)
        .err_as(TS_ERROR)?
    {
        output += match event.err_as(TS_ERROR)? {
            HighlightEvent::HighlightStart(Highlight(u)) => config.formats[u].ansi,
            HighlightEvent::Source { start, end } => &code[start..end],
            HighlightEvent::HighlightEnd => RESET.ansi,
        }
    }
    Ok(output)
}

fn sexp(config: &LanguageConfig, code: &str) -> Result<String, &'static str> {
    let mut parser = Parser::new();
    parser.set_language(config.language).err_as(TS_ERROR)?;
    let tree = parser.parse(code, None).ok_or(TS_ERROR)?;
    Ok(tree.root_node().to_sexp())
}
