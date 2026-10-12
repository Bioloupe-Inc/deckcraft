//! Open-and-save fidelity: what DeckCraft reads from a PowerPoint package comes back on save,
//! including parts it doesn't edit. Every fixture is a deck built here, then patched.

use std::io::{Cursor, Read, Write};

use deckcraft_geom::Xfrm;
use deckcraft_model::style::PictureFill;
use deckcraft_model::{Presentation, Shape, ShapeId, ShapeKind, defaults};

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#3366cc"/></svg>"##;
const SVG_EXT: &str = "{96DAC541-7B7A-43D3-8B79-37D633B846F1}";

/// A package as a list of (part name, bytes), patched in place.
struct Pkg(Vec<(String, Vec<u8>)>);

impl Pkg {
    fn read(bytes: &[u8]) -> Self {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        Pkg((0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).expect("entry");
                let mut d = vec![];
                f.read_to_end(&mut d).expect("read");
                (f.name().to_string(), d)
            })
            .collect())
    }
    fn bytes(&self) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in &self.0 {
            z.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).expect("start");
            z.write_all(d).expect("write");
        }
        z.finish().expect("finish").into_inner()
    }
    fn get(&self, name: &str) -> Option<&[u8]> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }
    fn text(&self, name: &str) -> String {
        String::from_utf8(self.get(name).unwrap_or_else(|| panic!("no part {name}")).to_vec()).expect("utf8")
    }
    fn set(&mut self, name: &str, data: &[u8]) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = data.to_vec(),
            None => self.0.push((name.to_string(), data.to_vec())),
        }
    }
    fn remove(&mut self, name: &str) {
        self.0.retain(|(n, _)| n != name);
    }
    /// Replace the first `from` in a part (which must contain it).
    fn patch(&mut self, name: &str, from: &str, to: &str) {
        let t = self.text(name);
        assert!(t.contains(from), "{name} has no {from:?}:\n{t}");
        self.set(name, t.replacen(from, to, 1).as_bytes());
    }
    /// Add a relationship to a part's `.rels`.
    fn add_rel(&mut self, part: &str, id: &str, ty: &str, target: &str) {
        let rels = rels_path(part);
        if self.get(&rels).is_none() {
            self.set(&rels, br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#);
        }
        self.patch(&rels, "</Relationships>", &format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/></Relationships>"#));
    }
    /// Add a part with a content type override.
    fn add_part(&mut self, name: &str, content_type: &str, data: &[u8]) {
        self.set(name, data);
        self.patch("[Content_Types].xml", "</Types>", &format!(r#"<Override PartName="/{name}" ContentType="{content_type}"/></Types>"#));
    }
    /// Target part of relationship `id` of `part`, resolved to a part name.
    fn rel_target(&self, part: &str, id: &str) -> Option<String> {
        let rels = self.get(&rels_path(part)).map(|d| String::from_utf8_lossy(d).to_string())?;
        let at = rels.find(&format!(r#"Id="{id}""#))?;
        let start = rels[..at].rfind('<')?;
        let end = start + rels[start..].find('>')?;
        let target = attr(&rels[start..end], "Target")?;
        Some(resolve(part, &target))
    }
    /// Content type the package declares for a part.
    fn content_type(&self, name: &str) -> Option<String> {
        let ct = self.text("[Content_Types].xml");
        let over = format!(r#"PartName="/{name}""#);
        if let Some(at) = ct.find(&over) {
            let start = ct[..at].rfind('<')?;
            let end = start + ct[start..].find('>')?;
            return attr(&ct[start..end], "ContentType");
        }
        let ext = name.rsplit('.').next()?;
        let def = format!(r#"Extension="{ext}""#);
        let at = ct.find(&def)?;
        let start = ct[..at].rfind('<')?;
        let end = start + ct[start..].find('>')?;
        attr(&ct[start..end], "ContentType")
    }
}

fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

fn resolve(from: &str, target: &str) -> String {
    let mut segs: Vec<&str> = if target.starts_with('/') { vec![] } else { from.split('/').collect() };
    if !target.starts_with('/') {
        segs.pop();
    }
    for s in target.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// Value of attribute `name` in an XML start tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    Some(tag[at..at + tag[at..].find('"')?].to_string())
}

/// The first `<a:blip …/>` or `<a:blip …>…</a:blip>` element of a part.
fn first_blip(xml: &str) -> String {
    let start = xml.find("<a:blip ").or_else(|| xml.find("<a:blip>")).expect("a blip");
    let rest = &xml[start..];
    let tag_end = rest.find('>').expect("blip start tag") + 1;
    let end = if rest[..tag_end].ends_with("/>") { tag_end } else { rest.find("</a:blip>").expect("blip end") + "</a:blip>".len() };
    rest[..end].to_string()
}

fn png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 3, image::Rgb([200, 40, 40]));
    let mut out = Cursor::new(vec![]);
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// A one-slide deck with a single PNG picture, as a package.
fn picture_deck() -> Pkg {
    let mut p = defaults::new_presentation(None);
    let media = p.add_media("image1.png", "image/png", png());
    let id = ShapeId(p.alloc_id());
    let pic = Shape {
        id,
        name: "Icon".into(),
        xfrm: Some(Xfrm::new(100.0, 100.0, 50.0, 50.0)),
        kind: ShapeKind::Picture { fill: PictureFill { media, ..Default::default() } },
        ..Default::default()
    };
    std::sync::Arc::make_mut(&mut p.slides[0]).shapes.push(pic);
    Pkg::read(&deckcraft_pptx::export(&p).expect("export"))
}

fn picture(p: &Presentation) -> &PictureFill {
    p.slides[0]
        .shapes
        .iter()
        .find_map(|s| match &s.kind {
            ShapeKind::Picture { fill } if s.name == "Icon" => Some(fill),
            _ => None,
        })
        .expect("the picture survives")
}

fn svg_blip(rid: &str) -> String {
    format!(
        r#"<a:extLst><a:ext uri="{SVG_EXT}"><asvg:svgBlip xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main" r:embed="{rid}"/></a:ext></a:extLst>"#
    )
}

/// The SVG part a saved picture's `svgBlip` points at.
fn saved_svg(pkg: &Pkg) -> Vec<u8> {
    let slide = "ppt/slides/slide1.xml";
    let blip = first_blip(&pkg.text(slide));
    let at = blip.find("svgBlip").unwrap_or_else(|| panic!("no svgBlip in {blip}"));
    let rid = attr(&blip[at..], "r:embed").expect("svgBlip r:embed");
    let part = pkg.rel_target(slide, &rid).expect("svgBlip relationship");
    assert!(pkg.content_type(&part).is_some_and(|ct| ct.starts_with("image/svg")), "{part}");
    pkg.get(&part).expect("svg part").to_vec()
}

#[test]
fn svg_only_picture_is_kept() {
    // Some writers declare SVG as `image/svg`.
    for content_type in ["image/svg+xml", "image/svg"] {
        svg_only_picture(content_type);
    }
}

fn svg_only_picture(content_type: &str) {
    // PowerPoint's icons: a blip with only the SVG extension, no raster `r:embed`.
    let mut pkg = picture_deck();
    let slide = "ppt/slides/slide1.xml";
    let blip = first_blip(&pkg.text(slide));
    let rid = attr(&blip, "r:embed").expect("raster rel");
    let png_part = pkg.rel_target(slide, &rid).expect("png part");
    pkg.patch(slide, &blip, &format!("<a:blip>{}</a:blip>", svg_blip("rId90")));
    pkg.add_rel(slide, "rId90", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image", "../media/icon1.svg");
    pkg.add_part("ppt/media/icon1.svg", content_type, SVG);
    pkg.remove(&png_part);

    let p = deckcraft_pptx::import(&pkg.bytes()).expect("import");
    let fill = picture(&p);
    let m = p.media(fill.media).expect("media");
    assert_eq!((m.content_type.as_str(), m.data.as_slice()), (content_type, SVG));

    let saved = Pkg::read(&deckcraft_pptx::export(&p).expect("export"));
    assert_eq!(saved_svg(&saved), SVG);

    let again = deckcraft_pptx::import(&saved.bytes()).expect("reopen");
    assert_eq!(again.media(picture(&again).media).map(|m| m.data.as_slice()), Some(SVG));
}

#[test]
fn svg_with_png_fallback_keeps_both() {
    for content_type in ["image/svg+xml", "image/svg"] {
        svg_with_png_fallback(content_type);
    }
}

fn svg_with_png_fallback(content_type: &str) {
    let mut pkg = picture_deck();
    let slide = "ppt/slides/slide1.xml";
    let blip = first_blip(&pkg.text(slide));
    let rid = attr(&blip, "r:embed").expect("raster rel");
    let png_bytes = pkg.get(&pkg.rel_target(slide, &rid).expect("png part")).expect("png").to_vec();
    pkg.patch(slide, &blip, &format!(r#"<a:blip r:embed="{rid}">{}</a:blip>"#, svg_blip("rId91")));
    pkg.add_rel(slide, "rId91", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image", "../media/icon2.svg");
    pkg.add_part("ppt/media/icon2.svg", content_type, SVG);

    let p = deckcraft_pptx::import(&pkg.bytes()).expect("import");
    let fill = picture(&p);
    assert_eq!(p.media(fill.media).map(|m| m.content_type.as_str()), Some("image/png"));
    assert_eq!(fill.svg.and_then(|s| p.media(s)).map(|m| m.data.as_slice()), Some(SVG));

    let saved = Pkg::read(&deckcraft_pptx::export(&p).expect("export"));
    assert_eq!(saved_svg(&saved), SVG);
    let blip = first_blip(&saved.text(slide));
    let raster = saved.rel_target(slide, &attr(&blip, "r:embed").expect("raster r:embed")).expect("raster part");
    assert_eq!(saved.get(&raster), Some(png_bytes.as_slice()));

    let again = deckcraft_pptx::import(&saved.bytes()).expect("reopen");
    assert!(picture(&again).svg.is_some());
}
