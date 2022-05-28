use std::collections::HashMap;

use serenity::{
    async_trait,
    model::channel::{Channel, Message},
    prelude::*,
};
use tree_sitter_highlight::{
    Error as HighlightError, Highlight, HighlightConfiguration, HighlightEvent, Highlighter,
};

use lazy_static::lazy_static;

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
        let mut config = HighlightConfiguration::new(
            $pkg::language(),
            $pkg::HIGHLIGHTS_QUERY,
            "",
            "",
        ).unwrap();
        let (recognized_names, formats) = unzip![$($t)*];
        config.configure(recognized_names);
        HighlightLanguage {
            config,
            formats,
        }
    }};
}

struct HighlightLanguage {
    config: HighlightConfiguration,
    formats: &'static [&'static str],
}

lazy_static! {
    static ref LANGUAGES: HashMap<&'static str, HighlightLanguage> = HashMap::from(map![
        ursl => lang![tree_sitter_ursl;
            comment => "\u{001b}[30m",
            number => "\u{001b}[36m",
            port => "\u{001b}[32m",
            label => "\u{001b}[33m",
            "label.data" => "\u{001b}[33m",
            function => "\u{001b}[33m",
            macro => "\u{001b}[35m",
            address => "\u{001b}[36m",
            register => "\u{001b}[36m",
            string => "\u{001b}[36m",
            "string.special" => "\u{001b}[36m",
            instruction => "\u{001b}[34m",
            property => "\u{001b}[31m",
            keyword => "\u{001b}[35m",
            "punctuation.delimiter" => "\u{001b}[30m",
            "punctuation.bracket" => "\u{001b}[30m",
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

async fn send(ctx: &Context, channel: &Channel, content: String) -> serenity::Result<Message> {
    match channel {
        Channel::Guild(c) => c.send_message(&ctx, |msg| msg.content(content)).await,
        Channel::Private(c) => c.send_message(&ctx, |msg| msg.content(content)).await,
        &_ => panic!("bad channel"),
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        let content = &msg.content[..];
        if content.starts_with("```") && content.ends_with("\n```") {
            let content = &content[3..(content.len() - 4)];
            if let Some((lang, code)) = content.split_once("\n") {
                if let Some(lang) = LANGUAGES.get(lang) {
                    if let Ok(formatted) = format(lang, code) {
                        let channel = msg.channel(&ctx).await.unwrap();
                        let mut chunk = String::new();
                        for line in formatted.split("\n") {
                            if "```ansi\n".len() + chunk.len() + line.len() + "\n```".len() > 2000 {
                                chunk.insert_str(0, "```ansi\n");
                                chunk.push_str("```");
                                send(&ctx, &channel, chunk).await.unwrap();
                                chunk = String::new();
                            }
                            chunk.push_str(line);
                            chunk.push('\n');
                        }
                        if !chunk.is_empty() {
                            chunk.insert_str(0, "```ansi\n");
                            chunk.push_str("```");
                            send(&ctx, &channel, chunk).await.unwrap();
                        }
                    }
                }
            }
        }
    }
}

fn format(lang: &HighlightLanguage, code: &str) -> Result<String, HighlightError> {
    let mut output = String::new();
    let mut highlighter = Highlighter::new();
    for event in highlighter.highlight(&lang.config, code.as_bytes(), None, |_| None)? {
        output += match event {
            Ok(e) => match e {
                HighlightEvent::HighlightStart(Highlight(u)) => lang.formats[u],
                HighlightEvent::Source { start, end } => &code[start..end],
                HighlightEvent::HighlightEnd => "\u{001b}[0m",
            },
            Err(e) => match e {
                HighlightError::Unknown => "{{HighlightError::Unknown}}",
                HighlightError::Cancelled => "{{HighlightError::Cancelled}}",
                HighlightError::InvalidLanguage => "{{HighlightError::InvalidLanguage}}",
            },
        }
    }
    return Ok(output);
}
