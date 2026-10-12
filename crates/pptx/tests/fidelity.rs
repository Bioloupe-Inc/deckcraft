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

/// A deck whose picture (100, 100, 50 × 50 pt) is an SVG-only blip, as PowerPoint stores icons.
fn svg_only_deck(content_type: &str) -> Pkg {
    let mut pkg = picture_deck();
    let slide = "ppt/slides/slide1.xml";
    let blip = first_blip(&pkg.text(slide));
    let rid = attr(&blip, "r:embed").expect("raster rel");
    let png_part = pkg.rel_target(slide, &rid).expect("png part");
    pkg.patch(slide, &blip, &format!("<a:blip>{}</a:blip>", svg_blip("rId90")));
    pkg.add_rel(slide, "rId90", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image", "../media/icon1.svg");
    pkg.add_part("ppt/media/icon1.svg", content_type, SVG);
    pkg.remove(&png_part);
    pkg
}

#[test]
fn svg_only_picture_renders() {
    let p = deckcraft_pptx::import(&svg_only_deck("image/svg+xml").bytes()).expect("import");
    let img = deckcraft_engine::render::render_slide(&p, 0, &deckcraft_engine::render::RenderOpts { scale: 2.0, ..Default::default() });
    // The SVG is one #3366cc square filling the picture's box.
    for (x, y) in [(210, 210), (250, 250), (290, 290)] {
        let [r, g, b, a] = img.pixel(x, y);
        assert!(r.abs_diff(0x33) < 8 && g.abs_diff(0x66) < 8 && b.abs_diff(0xcc) < 8 && a > 250, "({x}, {y}) is {:?}", [r, g, b, a]);
    }
}

fn svg_only_picture(content_type: &str) {
    // PowerPoint's icons: a blip with only the SVG extension, no raster `r:embed`.
    let pkg = svg_only_deck(content_type);

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

const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT_XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const CT_CHART_STYLE: &str = "application/vnd.ms-office.chartstyle+xml";
const CT_CHART_COLORS: &str = "application/vnd.ms-office.chartcolorstyle+xml";
const CT_FONT: &str = "application/x-fontdata";

/// Bytes standing in for a workbook: a zip with one entry.
fn workbook() -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    z.start_file("xl/workbook.xml", zip::write::SimpleFileOptions::default()).expect("start");
    z.write_all(b"<workbook/>").expect("write");
    z.finish().expect("finish").into_inner()
}

/// The relationship of `part` with type `ty` → (id, target part).
fn rel_of_type(pkg: &Pkg, part: &str, ty: &str) -> Option<(String, String)> {
    let rels = String::from_utf8_lossy(pkg.get(&rels_path(part))?).to_string();
    rels.split("<Relationship ").skip(1).find_map(|r| {
        (attr(&format!(" {r}"), "Type")? == ty).then_some(())?;
        let id = attr(&format!(" {r}"), "Id")?;
        let target = attr(&format!(" {r}"), "Target")?;
        Some((id, resolve(part, &target)))
    })
}

/// A part's bytes and content type through the relationship of `part` with type `ty`.
fn related(pkg: &Pkg, part: &str, ty: &str) -> (Vec<u8>, String) {
    let (_, target) = rel_of_type(pkg, part, ty).unwrap_or_else(|| panic!("{part} lost its {ty} relationship"));
    let data = pkg.get(&target).unwrap_or_else(|| panic!("{target} is missing")).to_vec();
    (data, pkg.content_type(&target).unwrap_or_default())
}

fn round_trip(pkg: &Pkg) -> Pkg {
    let p = deckcraft_pptx::import(&pkg.bytes()).expect("import");
    Pkg::read(&deckcraft_pptx::export(&p).expect("export"))
}

#[test]
fn chart_workbook_and_styles_are_kept() {
    let mut p = defaults::new_presentation(None);
    let id = ShapeId(p.alloc_id());
    let chart = deckcraft_model::Chart {
        categories: vec!["A".into(), "B".into()],
        series: vec![deckcraft_model::chart::Series { name: "S1".into(), values: vec![Some(1.0), Some(2.0)], ..Default::default() }],
        ..Default::default()
    };
    let shape = Shape {
        id,
        name: "Chart".into(),
        xfrm: Some(Xfrm::new(50.0, 50.0, 400.0, 300.0)),
        kind: ShapeKind::Chart(Box::new(chart)),
        ..Default::default()
    };
    std::sync::Arc::make_mut(&mut p.slides[0]).shapes.push(shape);
    let mut pkg = Pkg::read(&deckcraft_pptx::export(&p).expect("export"));
    let (_, chart) = rel_of_type(&pkg, "ppt/slides/slide1.xml", &format!("{RT}/chart")).expect("chart part");
    pkg.add_rel(&chart, "rId7", &format!("{RT}/package"), "../embeddings/Microsoft_Excel_Worksheet1.xlsx");
    pkg.add_rel(&chart, "rId8", "http://schemas.microsoft.com/office/2011/relationships/chartStyle", "style1.xml");
    pkg.add_rel(&chart, "rId9", "http://schemas.microsoft.com/office/2011/relationships/chartColorStyle", "colors1.xml");
    pkg.patch(&chart, "</c:chartSpace>", r#"<c:externalData r:id="rId7"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#);
    pkg.add_part("ppt/embeddings/Microsoft_Excel_Worksheet1.xlsx", CT_XLSX, &workbook());
    let style = br#"<cs:chartStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" id="201"/>"#;
    let colors = br#"<cs:colorStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" meth="cycle" id="10"/>"#;
    pkg.add_part("ppt/charts/style1.xml", CT_CHART_STYLE, style);
    pkg.add_part("ppt/charts/colors1.xml", CT_CHART_COLORS, colors);

    let saved = round_trip(&pkg);
    let (_, chart) = rel_of_type(&saved, "ppt/slides/slide1.xml", &format!("{RT}/chart")).expect("chart part");
    assert_eq!(related(&saved, &chart, &format!("{RT}/package")), (workbook(), CT_XLSX.to_string()));
    assert_eq!(related(&saved, &chart, "http://schemas.microsoft.com/office/2011/relationships/chartStyle"), (style.to_vec(), CT_CHART_STYLE.into()));
    assert_eq!(
        related(&saved, &chart, "http://schemas.microsoft.com/office/2011/relationships/chartColorStyle"),
        (colors.to_vec(), CT_CHART_COLORS.into())
    );
    // The chart still points at its workbook by a relationship id that exists.
    let xml = saved.text(&chart);
    let at = xml.find("<c:externalData ").expect("externalData kept");
    let rid = attr(&xml[at..], "r:id").expect("externalData r:id");
    assert_eq!(saved.rel_target(&chart, &rid).and_then(|t| saved.get(&t).map(<[u8]>::to_vec)), Some(workbook()));

    // An edited chart no longer matches its workbook, so only the styles stay.
    let mut p = deckcraft_pptx::import(&pkg.bytes()).expect("import");
    for sh in &mut std::sync::Arc::make_mut(&mut p.slides[0]).shapes {
        if let ShapeKind::Chart(c) = &mut sh.kind {
            c.series[0].values[0] = Some(5.0);
        }
    }
    let edited = Pkg::read(&deckcraft_pptx::export(&p).expect("export"));
    let (_, chart) = rel_of_type(&edited, "ppt/slides/slide1.xml", &format!("{RT}/chart")).expect("chart part");
    assert!(rel_of_type(&edited, &chart, &format!("{RT}/package")).is_none());
    assert!(!edited.text(&chart).contains("externalData"));
    assert_eq!(related(&edited, &chart, "http://schemas.microsoft.com/office/2011/relationships/chartStyle").0, style.to_vec());
}

#[test]
fn embedded_fonts_are_kept() {
    let mut pkg = Pkg::read(&deckcraft_pptx::export(&defaults::new_presentation(None)).expect("export"));
    let pres = "ppt/presentation.xml";
    let notes_sz = {
        let t = pkg.text(pres);
        let at = t.find("<p:notesSz ").expect("notesSz");
        t[at..at + t[at..].find("/>").expect("notesSz end") + 2].to_string()
    };
    pkg.patch(
        pres,
        &notes_sz,
        &format!(
            r#"{notes_sz}<p:embeddedFontLst><p:embeddedFont><p:font typeface="Test Sans" panose="020B0604020202020204" pitchFamily="34" charset="0"/><p:regular r:id="rId90"/><p:bold r:id="rId91"/></p:embeddedFont></p:embeddedFontLst>"#
        ),
    );
    pkg.add_rel(pres, "rId90", &format!("{RT}/font"), "fonts/font1.fntdata");
    pkg.add_rel(pres, "rId91", &format!("{RT}/font"), "fonts/font2.fntdata");
    pkg.add_part("ppt/fonts/font1.fntdata", CT_FONT, b"regular font bytes");
    pkg.add_part("ppt/fonts/font2.fntdata", CT_FONT, b"bold font bytes");

    let saved = round_trip(&pkg);
    let xml = saved.text(pres);
    assert!(xml.contains(r#"embedTrueTypeFonts="1""#), "{xml}");
    let at = xml.find("<p:embeddedFont>").expect("embeddedFontLst kept");
    let font = &xml[at..at + xml[at..].find("</p:embeddedFont>").expect("embeddedFont end")];
    assert!(font.contains(r#"typeface="Test Sans""#) && font.contains(r#"panose="020B0604020202020204""#), "{font}");
    for (style, bytes) in [("regular", &b"regular font bytes"[..]), ("bold", &b"bold font bytes"[..])] {
        let s = font.find(&format!("<p:{style} ")).unwrap_or_else(|| panic!("no {style} in {font}"));
        let part = saved.rel_target(pres, &attr(&font[s..], "r:id").expect("r:id")).expect("font relationship");
        assert_eq!(saved.get(&part), Some(bytes), "{part}");
        assert_eq!(saved.content_type(&part).as_deref(), Some(CT_FONT));
    }
}

#[test]
fn unknown_parts_are_carried_through() {
    let mut pkg = Pkg::read(&deckcraft_pptx::export(&defaults::new_presentation(None)).expect("export"));
    let custom_ct = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
    let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"/>"#;
    pkg.add_rel("", "rId9", &format!("{RT}/custom-properties"), "docProps/custom.xml");
    pkg.add_part("docProps/custom.xml", custom_ct, custom);
    let printer_ct = "application/vnd.openxmlformats-officedocument.presentationml.printerSettings";
    pkg.add_rel("ppt/presentation.xml", "rId98", &format!("{RT}/printerSettings"), "printerSettings/printerSettings1.bin");
    pkg.add_part("ppt/printerSettings/printerSettings1.bin", printer_ct, b"\x00\x01printer");
    // A part with relationships of its own.
    let item_ct = "application/xml";
    let props_ct = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
    pkg.add_rel("ppt/presentation.xml", "rId99", &format!("{RT}/customXml"), "../customXml/item1.xml");
    pkg.add_part("customXml/item1.xml", item_ct, b"<b:Sources xmlns:b=\"urn:x\"/>");
    pkg.add_rel("customXml/item1.xml", "rId1", &format!("{RT}/customXmlProps"), "itemProps1.xml");
    pkg.add_part("customXml/itemProps1.xml", props_ct, b"<ds:datastoreItem xmlns:ds=\"urn:y\"/>");

    let saved = round_trip(&pkg);
    assert_eq!(related(&saved, "", &format!("{RT}/custom-properties")), (custom.to_vec(), custom_ct.into()));
    assert_eq!(related(&saved, "ppt/presentation.xml", &format!("{RT}/printerSettings")), (b"\x00\x01printer".to_vec(), printer_ct.into()));
    let (_, item) = rel_of_type(&saved, "ppt/presentation.xml", &format!("{RT}/customXml")).expect("customXml kept");
    assert_eq!(related(&saved, &item, &format!("{RT}/customXmlProps")), (b"<ds:datastoreItem xmlns:ds=\"urn:y\"/>".to_vec(), props_ct.into()));
}

fn session_of(bytes: &[u8]) -> deckcraft_engine::Session {
    let mut s = deckcraft_engine::Session::new();
    let data = deckcraft_engine::cmd::base64_encode(bytes);
    s.execute("file.openBytes", &serde_json::json!({"name": "deck.pptx", "data": data})).expect("open");
    s
}

fn save(s: &mut deckcraft_engine::Session) -> Vec<u8> {
    let v = s.execute("file.saveBytes", &serde_json::json!({"format": "pptx"})).expect("save");
    deckcraft_engine::cmd::base64_decode(v["data"].as_str().expect("data")).expect("base64")
}

/// (title, file slide id as `document.inspect` reports it) per slide.
fn inspected_ids(s: &mut deckcraft_engine::Session) -> Vec<(String, u64)> {
    let v = s.execute("document.inspect", &serde_json::json!({})).expect("inspect");
    v["slides"]
        .as_array()
        .expect("slides")
        .iter()
        .map(|x| (x["title"].as_str().unwrap_or("").to_string(), x["fileId"].as_u64().unwrap_or_else(|| panic!("no fileId in {x}"))))
        .collect()
}

/// The `p:sldId` ids of `presentation.xml`, in slide order.
fn sld_ids(pres: &str) -> Vec<u64> {
    let list = &pres[pres.find("<p:sldIdLst>").expect("sldIdLst")..pres.find("</p:sldIdLst>").expect("sldIdLst end")];
    list.split("<p:sldId ").skip(1).map(|t| attr(&format!(" {t}"), "id").and_then(|v| v.parse().ok()).expect("sldId id")).collect()
}

/// (title, `p:sldId` id) per slide of a saved deck, in slide order.
fn saved_ids(bytes: &[u8]) -> Vec<(String, u64)> {
    let ids = sld_ids(&Pkg::read(bytes).text("ppt/presentation.xml"));
    let titles = inspected_ids(&mut session_of(bytes)).into_iter().map(|(t, _)| t);
    titles.zip(ids).collect()
}

fn sorted(mut v: Vec<(String, u64)>) -> Vec<(String, u64)> {
    v.sort();
    v
}

#[test]
fn slide_ids_survive_reorder_add_and_save() {
    let mut s = deckcraft_engine::Session::with_new();
    for t in ["A", "B", "C"] {
        s.execute("slide.new", &serde_json::json!({"title": t})).expect("slide.new");
    }
    s.execute("slide.delete", &serde_json::json!({"index": 0})).expect("delete");
    // The file gives the slides ids that aren't 256, 257, 258.
    let mut pkg = Pkg::read(&save(&mut s));
    let ids = sld_ids(&pkg.text("ppt/presentation.xml"));
    for (from, to) in ids.iter().zip([300, 412, 299]) {
        pkg.patch("ppt/presentation.xml", &format!(r#"<p:sldId id="{from}" "#), &format!(r#"<p:sldId id="{to}" "#));
    }
    let mut s = session_of(&pkg.bytes());
    let original = vec![("A".to_string(), 300), ("B".to_string(), 412), ("C".to_string(), 299)];
    assert_eq!(inspected_ids(&mut s), original);

    s.execute("slide.move", &serde_json::json!({"from": 2, "to": 0})).expect("move");
    s.execute("slide.new", &serde_json::json!({"title": "D"})).expect("slide.new");
    // A copy of A, moved in front of it, doesn't take A's id.
    s.execute("slide.duplicate", &serde_json::json!({"index": 2})).expect("duplicate");
    s.execute("slide.move", &serde_json::json!({"from": 3, "to": 0})).expect("move");
    let before = inspected_ids(&mut s);
    assert_eq!(before.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["A", "C", "D", "A", "B"]);
    // The original slides keep their ids; new ones take the next unused id.
    let mut expect = original.clone();
    expect.extend([("D".to_string(), 413), ("A".to_string(), 414)]);
    assert_eq!(sorted(before.clone()), sorted(expect));
    let saved = save(&mut s);
    assert_eq!(saved_ids(&saved), before);
    assert_eq!(inspected_ids(&mut session_of(&saved)), before);
}
