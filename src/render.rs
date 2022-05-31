use std::{cmp, iter};

use super::*;
use image::Rgba;
use imageproc::{
    definitions::Image,
    drawing::{draw_filled_circle_mut, draw_filled_rect_mut, draw_text_mut, text_size, Canvas},
    rect::Rect,
};
use rusttype::{Font, Scale};

lazy_static! {
    static ref FONT: Font<'static> = Font::try_from_bytes(include_bytes!("../font.ttf")).unwrap();
}

const BG: Rgba<u8> = Rgba(hex!("2f3136ff"));
const BORDER: Rgba<u8> = Rgba(hex!("202225ff"));
const GLOBAL_SCALE: u32 = 2;

const RADIUS: u32 = 4 * GLOBAL_SCALE;
const BORDER_WIDTH: u32 = 1 * GLOBAL_SCALE;
const LINE_SPACING: u32 = 4 * GLOBAL_SCALE;

const BASE_UNIFORM_SCALE: f32 = 14.0; // discord value
const SCALE: Scale = Scale { // Scale::uniform isn't const, so therefore i have to WET (Write Everything Twice!)
    x: BASE_UNIFORM_SCALE * GLOBAL_SCALE as f32,
    y: BASE_UNIFORM_SCALE * GLOBAL_SCALE as f32,
};

#[derive(Debug)]
enum LineHighlightEvent<'a> {
    Color(Color),
    Segment(&'a str),
    Newline,
}

fn draw_rounded_box<C>(canvas: &mut C, rect: Rect, radius: u32, color: <C as Canvas>::Pixel, draw_safe_area: bool)
where
    C: Canvas,
{
    assert!(rect.width() >= 2 * radius);
    assert!(rect.height() >= 2 * radius);
    let offset = radius as i32;
    let diameter = 2 * radius;
    let safe = Rect::at(rect.left() + offset, rect.top() + offset)
        .of_size(rect.width() - diameter, rect.height() - diameter);
    let left = Rect::at(rect.left(), safe.top()).of_size(radius, safe.height());
    let top = Rect::at(safe.left(), rect.top()).of_size(safe.width(), radius);
    let right = Rect::at(safe.right() + 1, safe.top()).of_size(radius, safe.height());
    let bottom = Rect::at(safe.left(), safe.bottom() + 1).of_size(safe.width(), radius);
    if draw_safe_area {
        draw_filled_rect_mut(canvas, safe, color);
    }
    draw_filled_rect_mut(canvas, left, color);
    draw_filled_rect_mut(canvas, top, color);
    draw_filled_rect_mut(canvas, right, color);
    draw_filled_rect_mut(canvas, bottom, color);

    draw_filled_circle_mut(canvas, (safe.left(), safe.top()), radius as i32, color);
    draw_filled_circle_mut(canvas, (safe.left(), safe.bottom()), radius as i32, color);
    draw_filled_circle_mut(canvas, (safe.right(), safe.top()), radius as i32, color);
    draw_filled_circle_mut(canvas, (safe.right(), safe.bottom()), radius as i32, color);
}

fn text_width(text: &str) -> i32 {
    // this is a horrible hack because text_size() displays the minimum bounds, i.e. what is actually *drawn*, and not the *full size*. meaning spaces don't count towards it.
    // here i am relying on || being used as a non-ligature output of constant width, so that spaces count towards the text size.
    // if this invariant is broken, then the output may overlap, or have unnecessary spacing.
    let (base_width, _) = text_size(SCALE, &FONT, "||");
    let (width, _) = text_size(SCALE, &FONT, &format!("|{text}|"));
    width - base_width
}

// Right-to-left text is completely unsupported because none of my spoken languages are right-to-left so it does not affect me personally, and is therefore seen as an inconvenience rather than a requirement.
pub fn render(config: &LanguageConfig, code: &str) -> Result<Image<Rgba<u8>>, &'static str> {
    let mut highlighter = Highlighter::new();
    let mut events = Vec::new();
    for event in highlighter
        .highlight(&config.highlight, code.as_bytes(), None, |_| None)
        .err_as(TS_ERROR)?
    {
        match event.err_as(TS_ERROR)? {
            HighlightEvent::HighlightStart(Highlight(i)) => {
                events.push(LineHighlightEvent::Color(config.formats[i]))
            }
            HighlightEvent::Source { start, end } => {
                let text = &code[start..end];
                let (first, lines) = text
                    .split_once("\n")
                    .map(|(first, lines)| (first, Some(lines)))
                    .unwrap_or((text, None));
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
            HighlightEvent::HighlightEnd => events.push(LineHighlightEvent::Color(RESET)),
        }
    }
    
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

    // zero-width non-joiner for computing the width without potential ligatures. i only expect this to be used with monospace fonts but this i can do safely regardless
    const ZWNJ: &str = "\u{200C}";

    let dimensions = lines
        .iter()
        .map(|segs| {
            segs.iter()
                .fold(String::new(), |line, &(_, seg)| line + ZWNJ + seg)
        })
        .map(|line| text_size(SCALE, &FONT, &line))
        .collect::<Vec<_>>();
    let (width, height) =
        dimensions
            .iter()
            .fold((0, 0), |(total_width, total_height), &(width, height)| {
                (
                    cmp::max(total_width, width as u32),
                    total_height + height as u32 + LINE_SPACING,
                )
            });

    const BORDER_RADIUS: u32 = RADIUS + BORDER_WIDTH;
    print!("dimensions are {width}x{height}");
    if width * height > 1000 * 1000 {
        println!(" (too big)");
        return Err("Image is too big, fuck off")
    }
    println!();
    let mut image = Image::new(width + BORDER_RADIUS * 2, height + BORDER_RADIUS * 2);
    let full = Rect::at(0, 0).of_size(image.width(), image.height());
    let inner = Rect::at(BORDER_WIDTH as i32, BORDER_WIDTH as i32)
        .of_size(width + RADIUS * 2, height + RADIUS * 2);
    draw_rounded_box(&mut image, full, BORDER_RADIUS, BORDER, false);
    draw_rounded_box(&mut image, inner, RADIUS, BG, true);

    let mut y = BORDER_RADIUS as i32;
    for (segments, (_, height)) in iter::zip(lines, dimensions) {
        let mut x = BORDER_RADIUS as i32;
        for (color, text) in segments {
            draw_text_mut(&mut image, color.rgb, x, y, SCALE, &FONT, text);
            x += text_width(text);
        }
        y += height + LINE_SPACING as i32;
    }
    Ok(image)
}
