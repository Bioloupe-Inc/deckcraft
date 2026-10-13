//! A deck's own fonts: the ones embedded in it, registered so the deck draws in them, and the
//! families it asks for that no loaded font provides.

use deckcraft_model::text::TextBody;
use deckcraft_model::{Presentation, Shape, ShapeKind};
use serde::Serialize;

use crate::fonts::{FontDb, embedded_font_file, substitutes};

/// Registers the fonts embedded in `pres` with the global font database; returns how many faces
/// were added. MicroType Express compressed fonts are skipped and show up in [`missing`].
pub fn register_embedded(pres: &Presentation) -> usize {
    let db = FontDb::global();
    let mut added = 0;
    for font in &pres.embedded_fonts {
        for media in [font.regular, font.bold, font.italic, font.bold_italic].into_iter().flatten() {
            if let Some(file) = pres.media(media).and_then(|m| embedded_font_file(&m.data)) {
                added += db.add_font(file);
            }
        }
    }
    added
}

/// Families with an open twin of the same metrics, so text lays out exactly as in the original.
const SAME_METRICS: [(&str, &str); 6] = [
    ("calibri", "Carlito"),
    ("cambria", "Caladea"),
    ("arial", "Liberation Sans"),
    ("helvetica", "Liberation Sans"),
    ("times new roman", "Liberation Serif"),
    ("courier new", "Liberation Mono"),
];

/// A family the deck asks for that isn't loaded, and the one its text draws in instead.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingFont {
    pub family: String,
    pub drawn_with: String,
    /// `drawn_with` has the family's metrics: lines break where they did.
    pub same_metrics: bool,
}

fn bodies<'a>(shapes: &'a [Shape], out: &mut Vec<&'a TextBody>) {
    deckcraft_model::walk(shapes, &mut |s, _| {
        out.extend(s.text.as_ref());
        if let ShapeKind::Table(t) = &s.kind {
            out.extend(t.rows.iter().flat_map(|r| r.cells.iter().map(|c| &c.text)));
        }
    });
}

/// The Latin families the deck's text and theme fonts ask for that no loaded font provides, in
/// order of first use.
pub fn missing(pres: &Presentation) -> Vec<MissingFont> {
    let mut families: Vec<String> = Vec::new();
    let mut add = |family: &str| {
        if !family.is_empty() && !families.iter().any(|f| f.eq_ignore_ascii_case(family)) {
            families.push(family.to_string());
        }
    };
    for master in &pres.masters {
        add(&master.theme.fonts.major.latin);
        add(&master.theme.fonts.minor.latin);
        let mut list = Vec::new();
        bodies(&master.shapes, &mut list);
        for layout in &master.layouts {
            bodies(&layout.shapes, &mut list);
        }
        for slide in pres.slides.iter().filter(|s| pres.master_for(s).is_some_and(|(m, _)| std::ptr::eq(m, master.as_ref()))) {
            bodies(&slide.shapes, &mut list);
        }
        for run in list.iter().flat_map(|b| &b.paragraphs).flat_map(|p| &p.runs) {
            if let Some(font) = run.props.font.as_deref() {
                add(&master.theme.font(font));
            }
        }
    }
    let db = FontDb::global();
    families
        .into_iter()
        .filter(|f| !db.has_family(f))
        .map(|family| {
            let drawn_with = db.face(&family, "Regular").family.clone();
            let lower = family.to_ascii_lowercase();
            let same_metrics = SAME_METRICS.iter().any(|(f, twin)| *f == lower && drawn_with.eq_ignore_ascii_case(twin));
            debug_assert!(!same_metrics || substitutes(&family).first().is_some_and(|s| s.eq_ignore_ascii_case(&drawn_with)));
            MissingFont { family, drawn_with, same_metrics }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_theme_and_run_families_that_are_not_loaded() {
        let mut s = crate::Session::with_new();
        s.execute("shape.insert", &serde_json::json!({"preset": "rect", "rect": [10, 10, 200, 50], "text": "Hi"})).unwrap();
        s.execute("text.edit", &serde_json::json!({})).unwrap();
        s.execute("text.selectAll", &serde_json::json!({})).unwrap();
        s.execute("format.font", &serde_json::json!({"family": "Nowhere Sans"})).unwrap();
        let report = missing(&s.doc().unwrap().doc);
        let nowhere = report.iter().find(|m| m.family == "Nowhere Sans").unwrap();
        assert!(!nowhere.same_metrics);
        assert!(!nowhere.drawn_with.is_empty());
    }
}
