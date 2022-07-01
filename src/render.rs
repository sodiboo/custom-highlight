use std::{cmp, iter};

use super::*;
use image::{codecs::png::PngDecoder, GenericImage, GenericImageView, Rgba, RgbaImage, SubImage};
use image::{ImageDecoder, Pixel};
use rusttype::{Font, Scale};

lazy_static! {
    static ref FONT: Font<'static> = Font::try_from_bytes(include_bytes!("../font.ttf")).unwrap();
}

const TEXT_SIZE: u32 = 36;
const SCALE: Scale = Scale {
    // Scale::uniform isn't const, so therefore i have to WET (Write Everything Twice!)
    x: TEXT_SIZE as f32,
    y: TEXT_SIZE as f32,
};

#[derive(Debug)]
enum LineHighlightEvent<'a> {
    Color(Color),
    Segment(&'a str),
    Newline,
}

pub async fn render_command(
    ctx: &Context,
    channel: &Channel,
    config: &'static LanguageConfig,
    code: &str,
    reply_to: ReplyMethod<'_>,
    add_components: bool,
) -> Result<(), &'static str> {
    println!("begin render ({} bytes)", code.len());
    let code = code.to_owned();
    let buffer = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, &'static str> {
        let image = render(config, &code)?;
        println!("Begin encode: {}x{}", image.width(), image.height());
        // I've tested all other encodings that ``image`` comes with
        // and the only other one that even worked was JPEG
        // which is too moldy for text, and therefore unacceptable.
        // PNG is the only acceptable encoding.
        //
        // I've hand-picked these settings through trial and error:
        //
        // CompressionType = Run length encoding
        //
        // Because most of the image is gonna be the same gray BG color
        // especially when the image is big enough that
        // the choice of these settings actually matter
        //
        // FilterType = Up (scanline above)
        //
        // Because text generally contains a lot of vertical lines
        // and this measurably decreased size by about 20% with no noticeable delay
        // for the example.ursl in URSL repository
        let mut buffer = Vec::new();
        let png = png::PngEncoder::new_with_quality(
            &mut buffer,
            png::CompressionType::Rle,
            png::FilterType::Up,
        );
        png.write_image(&image, image.width(), image.height(), ColorType::Rgba8)
            .err_as("The image failed to encode")?;
        Ok(buffer)
    })
    .await
    .err_as("The rendering task failed to join")??;
    let bytes = &buffer[..];
    println!("encoded png ({} bytes)", bytes.len());
    // discord has an upload limit of 8MB. Is that actually MiB? I don't know, and i'd rather be on the safe side of that margin
    if bytes.len() > 8_000_000 {
        return Err("The resulting image is WAYY TOO BIG, get lost");
    }
    match reply_to {
        ReplyMethod::EphemeralFollowup(interaction) => {
            create_followup_message(ctx, interaction, |msg| {
                println!("ephemeral msg");
                msg.ephemeral(true).add_file((bytes, "code.png"))
            })
            .await
            .unwrap()
        }
        ReplyMethod::PublicReference(referenced) => send(ctx, channel, |msg| {
            if add_components {
                msg.components(|c| {
                    c.create_action_row(|row| {
                        add_command_buttons_except(row, referenced.id, Command::Render, false)
                    })
                });
            }
            msg.reference_message(referenced)
                .allowed_mentions(|mentions| mentions.replied_user(false))
                .add_file((bytes, "code.png"))
        })
        .await
        .unwrap(),
    };
    Ok(())
}

// Right-to-left text is completely unsupported because none of my spoken languages are right-to-left so it does not affect me personally, and is therefore seen as an inconvenience rather than a requirement.
pub fn render(config: &LanguageConfig, code: &str) -> Result<RgbaImage, &'static str> {
    let events = match config.highlight {
        HighlightType::TreeSitter(ref highlight) => {
            let mut highlighter = Highlighter::new();
            let mut events = Vec::new();
            let mut colors = ne_vec![RESET];
            for event in highlighter
                .highlight(highlight, code.as_bytes(), None, |_| None)
                .err_as(TS_ERROR)?
            {
                match event.err_as(TS_ERROR)? {
                    HighlightEvent::HighlightStart(Highlight(i)) => {
                        colors.push(config.formats[i]);
                        events.push(LineHighlightEvent::Color(*colors.last()))
                    }
                    HighlightEvent::Source { start, end } => {
                        let text = &code[start..end];
                        let (first, lines) = text
                            .split_once("\n")
                            .map_or((text, None), |(first, lines)| (first, Some(lines)));
                        events.push(LineHighlightEvent::Segment(first));
                        if let Some(lines) = lines {
                            events.extend(lines.split("\n").flat_map(|line| {
                                [
                                    LineHighlightEvent::Newline,
                                    LineHighlightEvent::Segment(line),
                                ]
                            }));
                        }
                    }
                    HighlightEvent::HighlightEnd => {
                        colors.pop();
                        events.push(LineHighlightEvent::Color(*colors.last()))
                    }
                }
            }
            events
        }
        HighlightType::Plaintext => {
            let (first, lines) = code
                .split_once("\n")
                .map_or((code, None), |(first, lines)| (first, Some(lines)));
            let mut events = Vec::new();
            events.push(LineHighlightEvent::Segment(first));
            if let Some(lines) = lines {
                events.extend(lines.split("\n").flat_map(|line| {
                    [
                        LineHighlightEvent::Newline,
                        LineHighlightEvent::Segment(line),
                    ]
                }));
            }
            events
        }
    };

    let lines = {
        let mut next_color = RESET;
        let mut lines = Vec::new();
        let mut current_line = Vec::new();

        for event in events {
            match event {
                LineHighlightEvent::Color(color) => next_color = color,
                LineHighlightEvent::Segment(seg) => {
                    current_line.push((next_color, seg));
                }
                LineHighlightEvent::Newline => {
                    lines.push(current_line);
                    current_line = Vec::new();
                }
            }
        }
        lines.push(current_line);
        lines
    };

    let line_strings = lines
        .iter()
        .map(|segs| {
            segs.iter()
                .fold(String::new(), |line, &(_, seg)| line + seg)
        })
        .collect::<Vec<_>>();

    let width = line_strings.iter().fold(0, |width, line| {
        let mut caret = 0f32;
        let mut last_glyph = None;

        for ch in line.chars() {
            let glyph = FONT.glyph(ch).scaled(SCALE);
            if let Some(last) = last_glyph {
                caret += FONT.pair_kerning(SCALE, last, glyph.id());
            }
            caret += glyph.h_metrics().advance_width;
            last_glyph = Some(glyph.id());
        }
        cmp::max(width, caret.ceil() as u32)
    });
    let height = SCALE.y as u32 * lines.len() as u32;
    println!("dimensions are {width}x{height}");

    let mut image = RgbaImage::default();
    let safe_area = &mut border::make_image(&mut image, width, height);

    let mut y = 0f32;
    let ascent = FONT.v_metrics(SCALE).ascent;
    for (line, segments) in iter::zip(line_strings, lines) {
        let colors = segments
            .into_iter()
            .flat_map(|(color, text)| iter::repeat(color).take(text.len()));
        for (color, glyph) in iter::zip(
            colors,
            FONT.layout(
                &line,
                SCALE,
                rusttype::Point {
                    x: 0f32,
                    y: y + ascent,
                },
            ),
        ) {
            if let Some(bounds) = glyph.pixel_bounding_box() {
                glyph.draw(|dx, dy, v| {
                    let a = (v * u8::MAX as f32).trunc() as u8;
                    let Rgb([r, g, b]) = color.rgb;
                    let color = Rgba([r, g, b, a]);

                    let x = bounds.min.x as u32 + dx;
                    let y = bounds.min.y as u32 + dy;
                    let mut pixel = safe_area.get_pixel(x, y);
                    pixel.blend(&color);
                    safe_area.put_pixel(x, y, pixel);
                });
            }
        }
        y += SCALE.y;
    }
    Ok(image)
}

mod border {
    use super::*;

    const R: u32 = 10;
    lazy_static! {
        static ref BORDER: RgbaImage = {
            let bytes = include_bytes!("../border.png").as_ref();
            let png = PngDecoder::new(bytes).unwrap();
            let width = {
                let (x, y) = png.dimensions();
                assert_eq!(x, y);
                x
            };
            assert_eq!(R * 2 + 1, width);
            assert_eq!(png.color_type(), ColorType::Rgba8);
            let mut image = RgbaImage::new(width, width);
            png.read_image(&mut image).unwrap();
            image
        };
        static ref TOP_LEFT: SubImage<&'static RgbaImage> = BORDER.view(0, 0, R, R);
        static ref TOP_RIGHT: SubImage<&'static RgbaImage> = BORDER.view(R + 1, 0, R, R);
        static ref BOTTOM_LEFT: SubImage<&'static RgbaImage> = BORDER.view(0, R + 1, R, R);
        static ref BOTTOM_RIGHT: SubImage<&'static RgbaImage> = BORDER.view(R + 1, R + 1, R, R);
        static ref TOP: SubImage<&'static RgbaImage> = BORDER.view(R, 0, 1, R);
        static ref LEFT: SubImage<&'static RgbaImage> = BORDER.view(0, R, R, 1);
        static ref BOTTOM: SubImage<&'static RgbaImage> = BORDER.view(R, R + 1, 1, R);
        static ref RIGHT: SubImage<&'static RgbaImage> = BORDER.view(R + 1, R, R, 1);
        static ref CENTER: Rgba<u8> = *BORDER.get_pixel(R, R);
    }

    pub fn make_image<'a>(
        image: &'a mut RgbaImage,
        width: u32,
        height: u32,
    ) -> SubImage<&'a mut RgbaImage> {
        let real_width = width + R * 2;
        let real_height = height + R * 2;
        *image = RgbaImage::from_pixel(real_width, real_height, *CENTER);
        // tokio::task::yield_now().await;
        put(&mut image.sub_image(0, 0, R, R), *TOP_LEFT);
        put(&mut image.sub_image(R + width, 0, R, R), *TOP_RIGHT);
        put(&mut image.sub_image(0, R + height, R, R), *BOTTOM_LEFT);
        put(
            &mut image.sub_image(R + width, R + height, R, R),
            *BOTTOM_RIGHT,
        );
        for x in 0..width {
            put(&mut image.sub_image(R + x, 0, 1, R), *TOP);
            put(&mut image.sub_image(R + x, R + height, 1, R), *BOTTOM);
        }
        for y in 0..height {
            put(&mut image.sub_image(0, R + y, R, 1), *LEFT);
            put(&mut image.sub_image(R + width, R + y, R, 1), *RIGHT);
        }
        image.sub_image(R, R, width, height)
    }

    fn put(destination: &mut SubImage<&mut RgbaImage>, source: SubImage<&RgbaImage>) {
        assert_eq!(destination.dimensions(), source.dimensions());
        for y in 0..source.height() {
            for x in 0..source.width() {
                destination.put_pixel(x, y, source.get_pixel(x, y));
            }
        }
    }
}
