//! Rendering fidelity against a few curated golden pictures: rotated and flipped shapes, a turned
//! group, and a deck drawn in its embedded font. Text uses fonts every build has (Ubuntu, and Hack
//! as the embedded one), so the pictures don't depend on the machine's fonts. A picture passes
//! when few pixels differ by more than anti-aliasing does; `UPDATE_GOLDENS=1` writes them anew.

use std::path::PathBuf;

use deckcraft_engine::Session;
use deckcraft_engine::cmd::file::{open_presentation, save_bytes};
use deckcraft_engine::model::{EmbeddedFont, Presentation};
use deckcraft_engine::render::{RenderOpts, render_slide};
use serde_json::{Value, json};

/// A channel off by more than this is a real difference, not anti-aliasing.
const CHANNEL: u8 = 48;
/// The share of pixels that may differ that much.
const SHARE: f64 = 0.004;

fn golden(name: &str, pres: &Presentation) {
    let img = render_slide(pres, 0, &RenderOpts { scale: 0.5, ..Default::default() });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens").join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, img.to_png()).unwrap();
        return;
    }
    let want = image::load_from_memory(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap().to_rgba8();
    assert_eq!((want.width(), want.height()), (img.width, img.height), "{name}: size");
    let got = img.to_straight();
    let differ = want.as_raw().chunks(4).zip(got.chunks(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > CHANNEL)).count();
    let share = differ as f64 / f64::from(img.width * img.height);
    assert!(share <= SHARE, "{name}: {differ} pixels ({:.2}%) differ from the golden picture", share * 100.0);
}

fn deck(steps: &[(&str, Value)]) -> Session {
    let mut s = Session::with_new();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    s.execute("slide.delete", &json!({"index": 0})).unwrap();
    for (id, params) in steps {
        s.execute(id, params).unwrap();
    }
    s
}

fn text_in(font: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("text.edit", json!({})),
        ("text.selectAll", json!({})),
        ("format.font", json!({"family": font})),
        ("format.size", json!({"size": 28})),
        ("text.exit", json!({})),
    ]
}

#[test]
fn rotated_and_flipped_shapes() {
    let mut steps = vec![("shape.insert", json!({"preset": "rect", "rect": [80, 80, 300, 120], "text": "Turned 30°"}))];
    steps.extend(text_in("Ubuntu"));
    steps.push(("shape.rotate", json!({"deg": 30})));
    steps.push(("shape.insert", json!({"preset": "rightArrow", "rect": [520, 260, 300, 140], "text": "Flipped"})));
    steps.extend(text_in("Ubuntu"));
    steps.push(("shape.rotate", json!({"deg": 120})));
    steps.push(("shape.flip", json!({"axis": "horizontal"})));
    let s = deck(&steps);
    golden("rotated-shapes", &s.doc().unwrap().doc);
}

#[test]
fn turned_group() {
    let s = deck(&[
        ("shape.insert", json!({"preset": "ellipse", "rect": [300, 160, 160, 160]})),
        ("shape.insert", json!({"preset": "triangle", "rect": [480, 160, 160, 160]})),
        ("edit.selectAll", json!({})),
        ("arrange.group", json!({})),
        ("shape.rotate", json!({"deg": 45})),
        ("shape.flip", json!({"axis": "vertical"})),
    ]);
    golden("turned-group", &s.doc().unwrap().doc);
}

/// Hack wrapped as PowerPoint embeds fonts: uncompressed Embedded OpenType, XOR-obscured.
fn eot(font: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; 82];
    let size = (v.len() + font.len()) as u32;
    v[0..4].copy_from_slice(&size.to_le_bytes());
    v[4..8].copy_from_slice(&(font.len() as u32).to_le_bytes());
    v[8..12].copy_from_slice(&0x0002_0001u32.to_le_bytes());
    v[12..16].copy_from_slice(&0x1000_0000u32.to_le_bytes());
    v[34..36].copy_from_slice(&0x504Cu16.to_le_bytes());
    v.extend(font.iter().map(|b| b ^ 0x50));
    v
}

#[test]
fn embedded_font() {
    let mut steps = vec![("shape.insert", json!({"preset": "rect", "rect": [80, 180, 800, 140], "text": "Embedded Hack 0Oo1lI"}))];
    steps.extend(text_in("Hack"));
    let s = deck(&steps);
    let mut pres = s.doc().unwrap().doc.as_ref().clone();
    let media = pres.add_media("font1.fntdata", "application/x-fontdata", eot(epaint_default_fonts::HACK_REGULAR));
    pres.embedded_fonts.push(EmbeddedFont { typeface: "Hack".into(), regular: Some(media), ..Default::default() });
    // Opening the file registers its fonts, as a viewer opening the deck does.
    let opened = open_presentation("embedded.pptx", &save_bytes(&pres, "pptx").unwrap()).unwrap();
    assert!(deckcraft_engine::deck_fonts::missing(&opened).iter().all(|m| m.family != "Hack"));
    golden("embedded-font", &opened);
}
