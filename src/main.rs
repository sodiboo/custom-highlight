mod render;
use std::{collections::HashMap, fmt::Debug, iter, sync::Arc};

use const_format::concatcp;
use hex_literal::hex;
use image::{codecs::png, ColorType, ImageEncoder, Rgb};
use lazy_static::lazy_static;
use render::render_command;
use serenity::{
    async_trait,
    builder::CreateMessage,
    model::{
        channel::{Channel, Message},
        id::UserId,
    },
    prelude::*,
};
use tree_sitter::{Language, Parser, TreeCursor};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};
use unicode_normalization::UnicodeNormalization;

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

impl<T, E: Debug, U> ErrAs<U> for Result<T, E> {
    type Err = Result<T, U>;
    fn err_as(self, err: U) -> Result<T, U> {
        match self {
            Ok(ok) => Ok(ok),
            Err(actual_err) => {
                println!("Error: {actual_err:?}");
                Err(err)
            }
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
    rgb: Rgb<u8>,
}

macro_rules! colors {
    ($($name:ident = $value:literal, $hex:literal)*) => {
        $(const $name: Color = Color { ansi: concat!("\u{001b}[", $value, "m"), rgb: Rgb(hex!($hex)) };)*
    }
}

// Note that there are not ANSI names, they are names that fit the specific colors
// discord uses for the relevant ansi code (and also the hex codes discord uses for them)
//
// ERROR is just #FF0000 because that's distinct from RED's color
// the same way with ANSI it uses underlines to be distinct from RED
colors! {
    ERROR = "31;4", "ff0000"
    RESET = 0, "b9bbbe"
    GRAY = 30, "4f545c"
    RED = 31, "dc322f"
    GREEN = 32, "859900"
    YELLOW = 33, "b58900"
    BLUE = 34, "268bd2"
    PINK = 35, "d33682"
    CYAN = 36, "2aa198"
    WHITE = 37, "ffffff"
}

lazy_static! {
    static ref LANGUAGES: HashMap<&'static str, LanguageConfig> = HashMap::from(map![
        "" => lang![tree_sitter_plaintext;],
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
            address => CYAN,
            instruction => BLUE,
            string => CYAN,
            "string.special" => BLUE,
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

// the contents of this array will NOT be allowed to highlight without the +highlight prefix
const NO_HIGHLIGHT_BY_DEFAULT: &[&str] = &[""];

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // normalize to NFKC because rusttype doesn't support ligatures
        let content = msg.content.nfkc().collect::<String>();

        // normalize newlines to \n
        let content = content
            .lines()
            .fold(String::new(), |out, line| out + "\n" + line);
        // ensure no leading or trailing newlines
        let content = content.trim_matches('\n');
        if let Some((op, lang, code)) = codeblock(content) {
            if let Some(config) = LANGUAGES.get(lang) {
                let channel = msg.channel(&ctx).await.unwrap();
                if let Err(error) = command(
                    &ctx,
                    &channel,
                    match op {
                        "" if !NO_HIGHLIGHT_BY_DEFAULT.contains(&lang) => "+highlight",
                        _ => op,
                    },
                    config,
                    code,
                    &msg,
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
    op: &str,
    config: &'static LanguageConfig,
    code: &str,
    msg: &Message,
) -> Result<(), &'static str> {
    match op {
        "+highlight" => {
            let formatted = syntax_highlight(config, code)?;
            chunk_ansi(ctx, channel, &formatted).await.unwrap()
        }
        "+parse" => {
            let formatted = pretty_parse(config, code, true)?;
            chunk_ansi(ctx, channel, &formatted).await.unwrap();
        }
        "+pparse" => {
            let formatted = pretty_parse(config, code, false)?;
            chunk_ansi(ctx, channel, &formatted).await.unwrap();
        }
        "+render" => {
            lazy_static! {
                static ref DENY_RENDER: Mutex<HashMap<UserId, Arc<Mutex<()>>>> =
                    Mutex::new(HashMap::new());
            }
            let user_mutex = {
                let mut map = DENY_RENDER.lock().await;
                map.entry(msg.author.id)
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone()
            };
            let _lock = user_mutex
                .try_lock()
                .err_as("You've already queued up a rendering task")?;
            render_command(ctx, channel, config, code).await?;
        }
        _ => (),
    }
    Ok(())
}

fn codeblock(content: &str) -> Option<(&str, &str, &str)> {
    let content = content.trim_end();
    if !content.ends_with("```") {
        return None;
    }
    let content = &content[..(content.len() - 3)];
    let (before, content) = content.split_once("```")?;
    // multiple codeblocks, nontrivial, so abort
    if content.contains("```") {
        return None;
    }
    let (lang, code) = content.split_once("\n").unwrap_or((content, ""));
    let code = code.trim_matches('\n');
    let (lang, code) = if code.is_empty() {
        ("", lang)
    } else if !lang.chars().all(char::is_alphanumeric) {
        ("", content)
    } else {
        (lang, code)
    };
    if code.is_empty() {
        None
    } else {
        Some((before.trim(), lang, code))
    }
}

fn syntax_highlight(config: &LanguageConfig, code: &str) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut highlighter = Highlighter::new();
    let mut colors = Vec::new();
    colors.push(RESET);
    fn last(colors: &mut Vec<Color>) -> &str {
        println!("{colors:?}");
        colors.last().unwrap().ansi
    }
    for event in highlighter
        .highlight(&config.highlight, code.as_bytes(), None, |_| None)
        .err_as(TS_ERROR)?
    {
        output += match event.err_as(TS_ERROR)? {
            HighlightEvent::HighlightStart(Highlight(u)) => {
                colors.push(config.formats[u]);
                last(&mut colors)
            }
            HighlightEvent::Source { start, end } => &code[start..end],
            HighlightEvent::HighlightEnd => {
                colors.pop();
                last(&mut colors)
            }
        }
    }
    Ok(output)
}

fn pretty_parse(
    config: &LanguageConfig,
    code: &str,
    colored: bool,
) -> Result<String, &'static str> {
    let mut parser = Parser::new();
    parser.set_language(config.language).err_as(TS_ERROR)?;
    let tree = parser.parse(code, None).ok_or(TS_ERROR)?;
    let mut cursor = tree.walk();
    Ok(pretty_parse_node(
        &mut cursor,
        0,
        String::new(),
        code,
        colored,
    ))
}

fn pretty_parse_node(
    cursor: &mut TreeCursor,
    indent: usize,
    mut string: String,
    code: &str,
    colored: bool,
) -> String {
    const INDENT: &str = "    ";
    string.extend(iter::repeat(INDENT).take(indent));
    if let Some(field_name) = cursor.field_name() {
        if colored {
            string.push_str(YELLOW.ansi);
        }
        string.push_str(field_name);
        string.push_str(": ");
        if colored {
            string.push_str(RESET.ansi);
        }
    }
    if colored {
        if cursor.node().is_error() {
            string.push_str(RED.ansi);
        } else if cursor.node().is_extra() {
            string.push_str(GRAY.ansi);
        } else {
            string.push_str(GREEN.ansi);
        }
    }
    string.push_str(cursor.node().kind());
    if colored {
        string.push_str(RESET.ansi);
    }

    let printed = cursor.goto_first_child() && {
        let mut printed = false;
        loop {
            if cursor.field_name().is_some()
                || cursor.node().is_named()
                || cursor.node().child_count() > 0
            {
                printed = true;
                string.push('\n');
                string = pretty_parse_node(cursor, indent + 1, string, code, colored);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
        printed
    };
    if !printed {
        if colored {
            string.push_str(PINK.ansi);
        }
        string.push_str(" [");
        let tree_sitter::Point { row, column } = cursor.node().start_position();
        string.push_str(&(row + 1).to_string());
        string.push_str(", ");
        string.push_str(&(column + 1).to_string());
        string.push_str("] ");
        if cursor.node().is_named() {
            if colored {
                if cursor.node().is_extra() {
                    string.push_str(GRAY.ansi);
                } else {
                    string.push_str(BLUE.ansi);
                }
            }
            string.push_str(&code[cursor.node().byte_range()]);
            if colored {
                string.push_str(RESET.ansi);
            }
        }
    }
    string
}
