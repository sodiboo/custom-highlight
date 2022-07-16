mod render;
use std::{collections::HashMap, fmt::Debug, iter, sync::Arc};

use const_format::concatcp;
use hex_literal::hex;
use image::{codecs::png, ColorType, ImageEncoder, Rgb};
use lazy_static::lazy_static;
use non_empty_vec::ne_vec;
use owoify_rs::{Owoifiable, OwoifyLevel};
use render::render_command;
use serenity::{
    async_trait,
    builder::{
        CreateActionRow, CreateInteractionResponse, CreateInteractionResponseFollowup,
        CreateMessage,
    },
    model::{
        channel::{Channel, Message, ReactionType},
        gateway::Ready,
        id::{MessageId, UserId},
        interactions::{
            application_command::{ApplicationCommand, ApplicationCommandType},
            message_component::{ButtonStyle, ComponentType, MessageComponentInteraction},
            Interaction, InteractionResponseType,
        },
        Permissions,
    },
    prelude::*,
};
use tree_sitter::{Language, Parser, TreeCursor};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};
use unicode_normalization::UnicodeNormalization;

macro_rules! owo {
    ($($t:tt)*) => {
        format!($($t)*)
            .owoify(OwoifyLevel::Uvu)
            .owoify(OwoifyLevel::Uvu)
            .owoify(OwoifyLevel::Uvu)
            .owoify(OwoifyLevel::Uvu)
    }
}

macro_rules! map {
    (@key $name:literal) => { $name };
    (@key $name:ident) => { stringify!($name) };
    (@m $callback:ident ($($args:tt)*) $($k:tt => $v:expr),* $(,)?) => { $callback!($($args)* $((map!(@key $k), $v),)*) };
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
            highlight: HighlightType::TreeSitter(highlight),
            formats,
            language: Some(language),
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

enum HighlightType {
    TreeSitter(HighlightConfiguration),
    Plaintext,
}

pub struct LanguageConfig {
    highlight: HighlightType,
    formats: &'static [Color],
    language: Option<Language>,
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
//
// Some of these are using bold and other styles to achieve a finer granularity of styles
// The renderer does not support these styles, so i'm using colors from dark_vs to make them
// look distinct when rendereing
colors! {
    ERROR = "31;4", "ff0000"
    RESET = 0, "b9bbbe"
    GRAY = 30, "4f545c"
    RED = 31, "dc322f"
    LIGHT_GREEN = 32, "b5cea8" // dark_vs constant.numeric
    DARK_GREEN = "32;1", "6a9955" // dark_vs comment
    YELLOW = 33, "b58900"
    BLUE = 34, "268bd2"
    DARK_BLUE = "34;1", "569cd6" // dark_vs constant.language
    PINK = 35, "d33682"
    CYAN = 36, "2aa198"
    WHITE = 37, "ffffff"
}

lazy_static! {
    static ref LANGUAGES: HashMap<&'static str, LanguageConfig> = HashMap::from(map![
        "" => {
            LanguageConfig {
                highlight: HighlightType::Plaintext,
                formats: &[],
                language: None,
            }
        },
        ursl => lang![tree_sitter_ursl;
            comment => GRAY,
            number => LIGHT_GREEN,
            port => DARK_GREEN,
            label => YELLOW,
            "label.data" => YELLOW,
            function => YELLOW,
            macro => PINK,
            address => DARK_BLUE,
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
            number => LIGHT_GREEN,
            relative => LIGHT_GREEN,
            port => DARK_GREEN,
            macro => PINK,
            label => YELLOW,
            register => CYAN,
            "register.special" => CYAN,
            address => DARK_BLUE,
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
            param => DARK_GREEN,
            label => YELLOW,
            number => LIGHT_GREEN,
            keyword => PINK,
        ],
        hexagn => lang![tree_sitter_hexagn;
            comment => GRAY,
            number => LIGHT_GREEN,
            func_name => YELLOW,
            keyword => PINK,
            type => DARK_GREEN,
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

async fn get_ref(ctx: &Context, channel: &Channel, message_id: MessageId) -> Message {
    match channel {
        Channel::Guild(channel) => channel.message(ctx, message_id).await.unwrap(),
        Channel::Private(channel) => channel.message(ctx, message_id).await.unwrap(),
        _ => panic!("bad channel"),
    }
}

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

#[derive(Clone, Copy, Debug)]
pub enum ReplyMethod<'a> {
    PublicReference(&'a Message),
    EphemeralFollowup(&'a Interaction),
}

async fn send_chunked_message_with_commands(
    ctx: &Context,
    channel: &Channel,
    chunks: Vec<String>,
    reply_to: ReplyMethod<'_>,
    except: Option<Command>,
    ephemeralish: bool,
) -> serenity::Result<()> {
    let first = 0;
    let last = chunks.len() - 1;
    for i in 0..chunks.len() {
        let chunk = &chunks[i];
        match reply_to {
            ReplyMethod::PublicReference(reply_to) => send(&ctx, channel, |msg| {
                if i == first {
                    msg.reference_message(reply_to)
                        .allowed_mentions(|f| f.replied_user(false));
                }
                if i == last {
                    if let Some(except) = except {
                        msg.components(|c| {
                            c.create_action_row(|row| {
                                add_command_buttons_except(row, reply_to.id, except, ephemeralish)
                            })
                        });
                    }
                }
                msg.content(&chunk)
            })
            .await
            .unwrap(),
            ReplyMethod::EphemeralFollowup(reply_to) => {
                create_followup_message(ctx, reply_to, |msg| msg.ephemeral(true).content(&chunk))
                    .await
                    .unwrap()
            }
        };
    }
    Ok(())
}

fn chunk_ansi(content: &str) -> Result<Vec<String>, &'static str> {
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for line in content.split("\n") {
        if "```ansi\n".len() + chunk.len() + line.len() + "\n```".len() > 2000 {
            if "```ansi\n".len() + line.len() + "\n```".len() > 2000 {
                return Err("Line is too long");
            }
            chunk.insert_str(0, "```ansi\n");
            chunk.push_str("```");
            chunks.push(chunk);
            chunk = String::new();
        }
        chunk.push_str(line);
        chunk.push('\n');
    }
    if !chunk.is_empty() {
        chunk.insert_str(0, "```ansi\n");
        chunk.push_str("```");
        chunks.push(chunk);
    }
    Ok(chunks)
}

// the contents of this array will NOT be responded to automatically
// "" is the plaintext highlighting, so you can test rendering without a lang
// do not respond to plain codeblocks lmao
const NO_AUTO_RESPOND: &[&str] = &[""];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Highlight,
    Render,
    PrettyParse,
    PlainParse,
}

const COMMANDS: &[Command] = &[
    Command::Highlight,
    Command::Render,
    Command::PrettyParse,
    Command::PlainParse,
];

const COMMAND_NAME_HIGHLIGHT: &str = "Highlight Codeblock";
const COMMAND_NAME_PLAIN_PARSE: &str = "Parse Syntax";
const COMMAND_NAME_PRETTY_PARSE: &str = "Pretty Parse Syntax";
const COMMAND_NAME_RENDER: &str = "Render Codeblock";

impl Command {
    fn add_button(
        self,
        row: &mut CreateActionRow,
        id: MessageId,
        ephemeralish: bool,
    ) -> &mut CreateActionRow {
        let suffix = if ephemeralish { "-ephemeralish" } else { "" };
        match self {
            Command::Highlight => row.create_button(|button| {
                button
                    .custom_id(format!("highlight-{id}{suffix}"))
                    .emoji('🖍')
                    .label("Highlight")
                    .style(ButtonStyle::Primary)
            }),
            Command::Render => row.create_button(|button| {
                button
                    .custom_id(format!("render-{id}{suffix}"))
                    .emoji('🖼')
                    .label("Render")
                    .style(ButtonStyle::Success)
            }),
            Command::PrettyParse => row.create_button(|button| {
                button
                    .custom_id(format!("pretty-parse-{id}{suffix}"))
                    .emoji('🔣')
                    .label("Pretty Parse")
                    .style(ButtonStyle::Secondary)
            }),
            Command::PlainParse => row.create_button(|button| {
                button
                    .custom_id(format!("plain-parse-{id}{suffix}"))
                    .emoji('📱')
                    .label("Parse")
                    .style(ButtonStyle::Secondary)
            }),
        }
    }
}

fn delete_button(row: &mut CreateActionRow, ephemeralish: bool) -> &mut CreateActionRow {
    let suffix = if ephemeralish { "-ephemeralish" } else { "" };
    row.create_button(|button| {
        button.custom_id(format!("delete{suffix}"));
        if ephemeralish {
            button.emoji('🗑').label("Delete").style(ButtonStyle::Danger)
        } else {
            button
                .emoji(ReactionType::Custom {
                    animated: false,
                    id: 991327676302364742.into(),
                    name: Some("hide".into()),
                })
                .label("Hide Buttons")
                .style(ButtonStyle::Danger)
        }
    })
}

fn add_command_buttons(
    row: &mut CreateActionRow,
    id: MessageId,
    ephemeralish: bool,
) -> &mut CreateActionRow {
    for &command in COMMANDS {
        command.add_button(row, id, ephemeralish);
    }
    row
}

fn add_command_buttons_except(
    row: &mut CreateActionRow,
    id: MessageId,
    except: Command,
    ephemeralish: bool,
) -> &mut CreateActionRow {
    for &command in COMMANDS {
        if except != command {
            command.add_button(row, id, ephemeralish);
        }
    }
    delete_button(row, ephemeralish)
}

async fn create_interaction_response<'a, F>(
    ctx: &Context,
    interaction: &Interaction,
    f: F,
) -> serenity::Result<()>
where
    for<'b> F:
        FnOnce(&'b mut CreateInteractionResponse<'a>) -> &'b mut CreateInteractionResponse<'a>,
{
    match interaction {
        Interaction::MessageComponent(interaction) => {
            interaction.create_interaction_response(ctx, f).await
        }
        Interaction::ApplicationCommand(interaction) => {
            interaction.create_interaction_response(ctx, f).await
        }
        _ => panic!("bad interaction type"),
    }
}

async fn create_followup_message<'a, F>(
    ctx: &Context,
    interaction: &Interaction,
    f: F,
) -> serenity::Result<Message>
where
    for<'b> F: FnOnce(
        &'b mut CreateInteractionResponseFollowup<'a>,
    ) -> &'b mut CreateInteractionResponseFollowup<'a>,
{
    match interaction {
        Interaction::MessageComponent(interaction) => {
            interaction.create_followup_message(ctx, f).await
        }
        Interaction::ApplicationCommand(interaction) => {
            interaction.create_followup_message(ctx, f).await
        }
        _ => panic!("bad interaction type"),
    }
}

async fn defer(ctx: &Context, interaction: &Interaction, ephemeral: bool) -> serenity::Result<()> {
    if ephemeral {
        create_interaction_response(ctx, interaction, |response| {
            response
                .kind(match interaction {
                    Interaction::MessageComponent(_) => {
                        InteractionResponseType::DeferredUpdateMessage
                    }
                    Interaction::ApplicationCommand(_) => {
                        InteractionResponseType::DeferredChannelMessageWithSource
                    }
                    _ => panic!("bad interaction type"),
                })
                .interaction_response_data(|data| data.ephemeral(true))
        })
        .await
    } else {
        match interaction {
            Interaction::MessageComponent(interaction) => interaction.defer(ctx).await,
            Interaction::ApplicationCommand(interaction) => interaction.defer(ctx).await,
            _ => panic!("bad interaction type"),
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, _ready: Ready) {
        ApplicationCommand::set_global_application_commands(&ctx, |commands| {
            commands
                .create_application_command(|cmd| {
                    cmd.kind(ApplicationCommandType::Message)
                        .name(COMMAND_NAME_HIGHLIGHT)
                })
                .create_application_command(|cmd| {
                    cmd.kind(ApplicationCommandType::Message)
                        .name(COMMAND_NAME_PLAIN_PARSE)
                })
                .create_application_command(|cmd| {
                    cmd.kind(ApplicationCommandType::Message)
                        .name(COMMAND_NAME_PRETTY_PARSE)
                })
                .create_application_command(|cmd| {
                    cmd.kind(ApplicationCommandType::Message)
                        .name(COMMAND_NAME_RENDER)
                })
        })
        .await
        .unwrap();
    }

    async fn message(&self, ctx: Context, message: Message) {
        if message.is_own(&ctx) {
            return;
        }
        // normalize to NFKC because rusttype doesn't support ligatures
        let content = message.content.nfkc().collect::<String>();

        // normalize newlines to \n
        let content = content
            .lines()
            .fold(String::from("\n"), |out, line| out + line + "\n");
        // trim trailing newline
        let content = &content[..(content.len() - 1)];
        // hmm something feels wrong about this pyramid of doom. when eta let else stable
        if let Some((before, lang, code, after)) = codeblock(content) {
            if let Some(config) = LANGUAGES.get(lang) {
                let channel = message.channel(&ctx).await.unwrap();
                if let Some(command) = parse_command(before) {
                    if after.trim().is_empty() {
                        if let Err(error) = run_command(
                            &ctx,
                            &channel,
                            command,
                            config,
                            code,
                            ReplyMethod::PublicReference(&message),
                            message.author.id,
                            false,
                        )
                        .await
                        {
                            message.reply(&ctx, error).await.unwrap();
                        }
                    }
                } else if !NO_AUTO_RESPOND.contains(&lang) && !message.author.bot {
                    send(&ctx, &channel, |msg| {
                        // empty messages are not allowed, so i guess just send a zwsp lol
                        msg.reference_message(&message)
                            .allowed_mentions(|mentions| mentions.replied_user(false))
                            .content(format!("This message contains a `{lang}` codeblock which i know how to work with! Press `Delete` to remove this."))
                            .components(|c| {
                                c.create_action_row(|row| {
                                    add_command_buttons(row, message.id, true);
                                    delete_button(row, true)
                                })
                            })
                    })
                    .await
                    .unwrap();
                }
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, original_interaction: Interaction) {
        match original_interaction {
            Interaction::MessageComponent(ref interaction) => {
                if interaction.data.component_type == ComponentType::Button {
                    let ref message = interaction.message;
                    let channel = message.channel(&ctx).await.unwrap();
                    let interact_id = &interaction.data.custom_id[..];
                    let (interact_id, ephemeralish) = if interact_id.ends_with("-ephemeralish") {
                        (
                            &interact_id[..(interact_id.len() - "-ephemeralish".len())],
                            true,
                        )
                    } else {
                        (interact_id, false)
                    };
                    let (interact_id, reference_id) = interact_id
                        // if the "custom_id" looks like this: "highlight-991266068330975302"
                        // then the component contains the ID. Try parsing it.
                        .rsplit_once("-")
                        .and_then(|(interact_id, reference_id)| {
                            // If that ID is invalid, ignore it, and pretend it was never there.
                            // Always Some() on this path
                            reference_id
                                .parse::<u64>()
                                .ok()
                                .map(|reference_id| (interact_id, Some(reference_id.into())))
                        })
                        // Either it was absent (i.e. "highlight"), or invalid (i.e. "pretty-parse"), so assume it was absent.
                        // Use message reference field as the referenced message instead.
                        .unwrap_or((
                            interact_id,
                            message
                                .message_reference
                                .as_ref()
                                .map(|reference| reference.message_id.unwrap()),
                        ));
                    async fn delete(ctx: &Context, message: &Message, ephemeralish: bool) {
                        if ephemeralish {
                            message.delete(&ctx).await.unwrap();
                        } else {
                            message
                                .clone()
                                .edit(ctx, |msg| msg.set_components(Default::default()))
                                .await
                                .unwrap()
                        }
                    }

                    let referenced = match reference_id {
                        Some(reference_id) => get_ref(&ctx, &channel, reference_id).await,
                        None => {
                            // where is the replied message?? just delete it already we don't care
                            interaction.defer(&ctx).await.unwrap();
                            return delete(&ctx, message, ephemeralish).await;
                        }
                    };

                    fn can_delete(
                        ctx: &Context,
                        interaction: &MessageComponentInteraction,
                        channel: &Channel,
                        referenced: &Message,
                    ) -> bool {
                        let channel = match channel {
                            Channel::Guild(c) => c,
                            _ => {
                                return true;
                            }
                        };
                        if interaction.user == referenced.author {
                            // delete if user is author, since they might want the bot to fuck off
                            true
                        } else if channel
                            .permissions_for_user(&ctx, interaction.user.id)
                            .map(|p| p.contains(Permissions::MANAGE_MESSAGES))
                            .unwrap_or(false)
                        {
                            // can delete messages anyways, let them do it through interaction
                            true
                        } else {
                            false
                        }
                    }

                    let command = match interact_id {
                        "highlight" => Command::Highlight,
                        "render" => Command::Render,
                        "pretty-parse" => Command::PrettyParse,
                        "plain-parse" => Command::PlainParse,
                        "delete" => {
                            if can_delete(&ctx, &interaction, &channel, &referenced) {
                                interaction.defer(&ctx).await.unwrap();
                                delete(&ctx, message, ephemeralish).await;
                            } else {
                                interaction
                                .create_interaction_response(&ctx, |response| {
                                    response.interaction_response_data(|msg| {
                                        msg.ephemeral(true).content(
                                            owo!("You didn't send the original message, so you can't delete this.")
                                        )
                                    })
                                })
                                .await
                                .unwrap();
                            }
                            return;
                        }
                        kind => {
                            return interaction
                                .create_interaction_response(&ctx, |response| {
                                    response.interaction_response_data(|msg| {
                                        msg.ephemeral(true)
                                            .content(owo!("Unknown command `{kind}`"))
                                    })
                                })
                                .await
                                .unwrap()
                        }
                    };
                    println!("{} clicked to execute {command:?}", interaction.user.tag());
                    match run_command_from_interaction(
                        &ctx,
                        command,
                        &original_interaction,
                        &channel,
                        &referenced,
                        true,
                        false,
                    )
                    .await
                    {
                        // command was not acknowledged in this case, so must defer it
                        InteractionCommandResult::NoCodeblock
                        // the message was edited to be the wrong lang, so delete silently here too
                        | InteractionCommandResult::BadLang(_) => {
                            interaction.defer(&ctx).await.unwrap();
                            delete(&ctx, message, ephemeralish).await;
                        }
                        InteractionCommandResult::FinishedSuccessfully => {
                            delete(&ctx, message, ephemeralish).await
                        }
                        InteractionCommandResult::InformedError => (), // do nothing, we already informed the user
                    }
                }
            }
            Interaction::ApplicationCommand(ref interaction)
                if interaction.data.kind == ApplicationCommandType::Message =>
            {
                let command = match interaction.data.name.as_str() {
                    COMMAND_NAME_HIGHLIGHT => Command::Highlight,
                    COMMAND_NAME_RENDER => Command::Render,
                    COMMAND_NAME_PRETTY_PARSE => Command::PrettyParse,
                    COMMAND_NAME_PLAIN_PARSE => Command::PlainParse,
                    name => {
                        interaction
                            .create_interaction_response(&ctx, |response| {
                                response.interaction_response_data(|msg| {
                                    msg.ephemeral(true)
                                        .content(owo!("Unknown command `{name}`"))
                                })
                            })
                            .await
                            .unwrap();
                        return;
                    }
                };
                println!("{} clicked to execute {command:?}", interaction.user.tag());
                let channel = interaction.channel_id.to_channel(&ctx).await.unwrap();
                let target = interaction.data.target_id.unwrap().to_message_id();
                let message = if let Some(message) = interaction.data.resolved.messages.get(&target)
                {
                    message.clone()
                } else {
                    get_ref(
                        &ctx,
                        &channel,
                        interaction.data.target_id.unwrap().to_message_id(),
                    )
                    .await
                };
                match run_command_from_interaction(
                    &ctx,
                    command,
                    &original_interaction,
                    &channel,
                    &message,
                    false,
                    true,
                )
                .await
                {
                    InteractionCommandResult::NoCodeblock => {
                        interaction
                            .create_interaction_response(&ctx, |response| {
                                response.interaction_response_data(|msg| {
                                    msg.ephemeral(true).content(owo!(
                                        "That's not a codeblock. Am i a joke to you?"
                                    ))
                                })
                            })
                            .await
                            .unwrap();
                    }
                    InteractionCommandResult::BadLang(lang) => {
                        interaction
                            .create_interaction_response(&ctx, |response| {
                            response.interaction_response_data(|msg| {
                                msg.ephemeral(true)
                                    .content(
                                        owo!("I know that's a codeblock and all, but like, i don't understand {lang}, sorry!")
                                    )
                                })
                            })
                            .await
                            .unwrap();
                    }
                    // both other cases already responded to the user, so do nothing here
                    InteractionCommandResult::FinishedSuccessfully
                    | InteractionCommandResult::InformedError => (),
                }
            }
            _ => (),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractionCommandResult<'a> {
    FinishedSuccessfully,
    InformedError,
    NoCodeblock,
    BadLang(&'a str),
}

async fn run_command_from_interaction<'a>(
    ctx: &Context,
    command: Command,
    interaction: &Interaction,
    channel: &Channel,
    referenced: &'a Message,
    add_components: bool,
    send_as_followup: bool,
) -> InteractionCommandResult<'a> {
    if let Some((_, lang, code, _)) = codeblock(&referenced.content) {
        if let Some(lang) = LANGUAGES.get(lang) {
            if command == Command::Render && !send_as_followup {
                create_interaction_response(&ctx, &interaction, |response| {
                    response.interaction_response_data(|msg| {
                    msg.ephemeral(true);
                    let bounds = |max_len| {
                        code.lines().map(str::len).max().unwrap_or(0) > max_len
                            || code.lines().count() > max_len
                    };
                    if bounds(700) {
                        msg.content("Rendering... (this could take a while, especially if you're trying to break it intentionally)")
                    } else if bounds(100) {
                        msg.content("Rendering... (this could take a while, especially if the code is really big)")
                    } else {
                        msg.content("Rendering...")
                    }})
                }).await.unwrap();
            } else {
                defer(&ctx, &interaction, send_as_followup).await.unwrap();
            }
            if let Err(why) = run_command(
                &ctx,
                &channel,
                command,
                lang,
                code,
                if send_as_followup {
                    ReplyMethod::EphemeralFollowup(interaction)
                } else {
                    ReplyMethod::PublicReference(referenced)
                },
                match &interaction {
                    Interaction::MessageComponent(interaction) => interaction.user.id,
                    Interaction::ApplicationCommand(interaction) => interaction.user.id,
                    _ => unreachable!(),
                },
                add_components,
            )
            .await
            {
                create_followup_message(
                    &ctx,
                    &interaction,
                    |msg: &mut CreateInteractionResponseFollowup| msg.ephemeral(true).content(why),
                )
                .await
                .unwrap();
                InteractionCommandResult::InformedError
            } else {
                InteractionCommandResult::FinishedSuccessfully
            }
        } else {
            InteractionCommandResult::BadLang(lang)
        }
    } else {
        InteractionCommandResult::NoCodeblock
    }
}

fn parse_command(before: &str) -> Option<Command> {
    match before {
        "+highlight" => Some(Command::Highlight),
        "+render" => Some(Command::Render),
        "+parse" => Some(Command::PrettyParse),
        "+pparse" => Some(Command::PlainParse),
        _ => None,
    }
}

async fn run_command(
    ctx: &Context,
    channel: &Channel,
    command: Command,
    config: &'static LanguageConfig,
    code: &str,
    reply_to: ReplyMethod<'_>,
    lock_render_for: UserId,
    add_components: bool,
) -> Result<(), &'static str> {
    let except = if add_components { Some(command) } else { None };
    Ok(match command {
        Command::Highlight => {
            let formatted = syntax_highlight(config, code)?;
            send_chunked_message_with_commands(
                ctx,
                channel,
                chunk_ansi(&formatted)?,
                reply_to,
                except,
                false,
            )
            .await
            .unwrap()
        }
        Command::PrettyParse => {
            let formatted = pretty_parse(config, code, true)?;
            send_chunked_message_with_commands(
                ctx,
                channel,
                chunk_ansi(&formatted)?,
                reply_to,
                except,
                false,
            )
            .await
            .unwrap()
        }
        Command::PlainParse => {
            let formatted = pretty_parse(config, code, false)?;
            send_chunked_message_with_commands(
                ctx,
                channel,
                chunk_ansi(&formatted)?,
                reply_to,
                except,
                false,
            )
            .await
            .unwrap()
        }
        Command::Render => {
            lazy_static! {
                static ref DENY_RENDER: Mutex<HashMap<UserId, Arc<Mutex<()>>>> =
                    Mutex::new(HashMap::new());
            }
            let user_mutex = {
                let mut map = DENY_RENDER.lock().await;
                map.entry(lock_render_for)
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone()
            };
            // this is dropped after render_command() finishes
            let _lock = user_mutex
                .try_lock()
                .err_as("You've already queued up a rendering task")?;
            render_command(ctx, channel, config, code, reply_to, add_components).await?;
        }
    })
}

fn codeblock(content: &str) -> Option<(&str, &str, &str, &str)> {
    let (before, content) = content.split_once("```")?;
    let (content, after) = content.split_once("```")?;
    // multiple codeblocks, nontrivial, so abort
    if after.contains("```") {
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
        Some((before.trim(), lang, code, after))
    }
}

fn syntax_highlight(config: &LanguageConfig, code: &str) -> Result<String, &'static str> {
    match config.highlight {
        HighlightType::TreeSitter(ref highlight) => {
            let mut output = String::new();
            let mut highlighter = Highlighter::new();
            let mut colors = ne_vec![RESET];
            for event in highlighter
                .highlight(highlight, code.as_bytes(), None, |_| None)
                .err_as(TS_ERROR)?
            {
                output += match event.err_as(TS_ERROR)? {
                    HighlightEvent::HighlightStart(Highlight(u)) => {
                        colors.push(config.formats[u]);
                        colors.last().ansi
                    }
                    HighlightEvent::Source { start, end } => &code[start..end],
                    HighlightEvent::HighlightEnd => {
                        colors.pop();
                        colors.last().ansi
                    }
                }
            }
            Ok(output)
        }
        HighlightType::Plaintext => Ok(code.to_string()),
    }
}

fn pretty_parse(
    config: &LanguageConfig,
    code: &str,
    colored: bool,
) -> Result<String, &'static str> {
    let mut parser = Parser::new();
    parser
        .set_language(
            config
                .language
                .ok_or("This language doesn't have parsing support")?,
        )
        .err_as(TS_ERROR)?;
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
            string.push_str(LIGHT_GREEN.ansi);
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
