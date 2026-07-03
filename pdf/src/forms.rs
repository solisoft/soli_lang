//! Fill an existing PDF's **AcroForm** fields from data, and optionally
//! **flatten** them (bake the values into non-editable appearances).
//!
//! This opens the "take a government/enterprise form PDF and fill it
//! programmatically" workflow — the one thing the render engine (which *writes*
//! PDFs) can't do, because it needs to *read* an existing document. Everything
//! here is a lopdf pass over the loaded form, like the other post-passes.
//!
//! Scope: text fields (`/Tx`), checkboxes/radios (`/Btn`) and choice fields
//! (`/Ch`). Filling sets `/V` and turns on `/NeedAppearances` so viewers render
//! the values; flattening generates an appearance stream for each text field,
//! marks fields read-only, and turns `/NeedAppearances` off.

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::error::{PdfError, Result};

/// Fill the form fields named in `values`. `flatten` bakes the values into
/// static appearances and makes the fields read-only.
pub fn fill_form(pdf: &[u8], values: &[(String, String)], flatten: bool) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("forms: could not parse the PDF: {e}")))?;
    let map: HashMap<&str, &str> = values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let acroform_id = acroform_id(&mut doc).ok_or_else(|| {
        PdfError::Backend("forms: the PDF has no AcroForm (no fillable fields)".into())
    })?;

    let field_ids = collect_field_ids(&doc, acroform_id);
    let mut helv: Option<ObjectId> = None;

    for fid in field_ids {
        let Some((name, ft)) = field_info(&doc, fid) else {
            continue;
        };
        let Some(&val) = map.get(name.as_str()) else {
            continue;
        };
        match ft.as_str() {
            "Tx" | "Ch" => fill_text(&mut doc, fid, val, flatten, &mut helv)?,
            "Btn" => fill_button(&mut doc, fid, val),
            _ => {}
        }
    }

    // Without flattening, ask viewers to build appearances from `/V`. With it,
    // we've baked appearances ourselves, so turn the hint off.
    if let Ok(af) = doc
        .get_object_mut(acroform_id)
        .and_then(|o| o.as_dict_mut())
    {
        af.set("NeedAppearances", Object::Boolean(!flatten));
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("forms: could not save: {e}")))?;
    Ok(out)
}

/// The AcroForm dictionary's object id, normalising an inline dict to an
/// indirect object so we can mutate it.
fn acroform_id(doc: &mut Document) -> Option<ObjectId> {
    let val = doc.catalog().ok()?.get(b"AcroForm").ok()?.clone();
    match val {
        Object::Reference(id) => Some(id),
        Object::Dictionary(d) => {
            let id = doc.add_object(Object::Dictionary(d));
            if let Ok(cat) = doc.catalog_mut() {
                cat.set("AcroForm", Object::Reference(id));
            }
            Some(id)
        }
        _ => None,
    }
}

/// Every named field reachable from the AcroForm, descending `/Kids`.
fn collect_field_ids(doc: &Document, acroform_id: ObjectId) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let fields = doc
        .get_object(acroform_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Fields").ok())
        .and_then(|o| o.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let mut stack: Vec<ObjectId> = fields
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .collect();
    while let Some(id) = stack.pop() {
        let Ok(d) = doc.get_object(id).and_then(|o| o.as_dict()) else {
            continue;
        };
        if d.get(b"T").is_ok() {
            out.push(id);
        }
        if let Ok(kids) = d.get(b"Kids").and_then(|o| o.as_array()) {
            for k in kids {
                if let Ok(kid) = k.as_reference() {
                    stack.push(kid);
                }
            }
        }
    }
    out
}

/// A field's partial name (`/T`) and type (`/FT`, inherited from `/Parent`).
fn field_info(doc: &Document, id: ObjectId) -> Option<(String, String)> {
    let d = doc.get_object(id).ok()?.as_dict().ok()?;
    let name = d
        .get(b"T")
        .ok()?
        .as_str()
        .ok()
        .map(|s| String::from_utf8_lossy(s).into_owned())?;
    let ft = field_type(doc, d).unwrap_or_default();
    Some((name, ft))
}

fn field_type(doc: &Document, d: &Dictionary) -> Option<String> {
    if let Ok(ft) = d.get(b"FT").and_then(|o| o.as_name()) {
        return Some(String::from_utf8_lossy(ft).into_owned());
    }
    // Inherited from the parent field.
    let parent = d.get(b"Parent").ok()?.as_reference().ok()?;
    let pd = doc.get_object(parent).ok()?.as_dict().ok()?;
    field_type(doc, pd)
}

fn fill_text(
    doc: &mut Document,
    fid: ObjectId,
    val: &str,
    flatten: bool,
    helv: &mut Option<ObjectId>,
) -> Result<()> {
    if let Ok(d) = doc.get_object_mut(fid).and_then(|o| o.as_dict_mut()) {
        d.set(
            "V",
            Object::String(val.as_bytes().to_vec(), StringFormat::Literal),
        );
    }
    if flatten {
        set_readonly(doc, fid);
        let font = ensure_helvetica(doc, helv);
        // The field itself may be the widget, or carry `/Kids` widgets.
        for wid in widget_ids(doc, fid) {
            generate_text_appearance(doc, fid, wid, val, font);
        }
    }
    Ok(())
}

/// The widget annotation object ids for a field: its `/Kids`, or the field
/// itself when field-and-widget are merged (it has a `/Rect`).
fn widget_ids(doc: &Document, fid: ObjectId) -> Vec<ObjectId> {
    let Ok(d) = doc.get_object(fid).and_then(|o| o.as_dict()) else {
        return Vec::new();
    };
    if let Ok(kids) = d.get(b"Kids").and_then(|o| o.as_array()) {
        return kids.iter().filter_map(|k| k.as_reference().ok()).collect();
    }
    if d.get(b"Rect").is_ok() {
        return vec![fid];
    }
    Vec::new()
}

/// Add (once) a standard Helvetica font object and return its id.
fn ensure_helvetica(doc: &mut Document, helv: &mut Option<ObjectId>) -> ObjectId {
    if let Some(id) = helv {
        return *id;
    }
    let mut f = Dictionary::new();
    f.set("Type", Object::Name(b"Font".to_vec()));
    f.set("Subtype", Object::Name(b"Type1".to_vec()));
    f.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    let id = doc.add_object(Object::Dictionary(f));
    *helv = Some(id);
    id
}

/// Build a `/AP /N` appearance stream showing `val` inside the widget rect.
fn generate_text_appearance(
    doc: &mut Document,
    fid: ObjectId,
    wid: ObjectId,
    val: &str,
    font: ObjectId,
) {
    let rect = doc
        .get_object(wid)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Rect").ok())
        .and_then(|o| o.as_array().ok())
        .map(|a| {
            a.iter()
                .filter_map(|n| {
                    n.as_float()
                        .ok()
                        .or_else(|| n.as_i64().ok().map(|i| i as f32))
                })
                .collect::<Vec<f32>>()
        })
        .unwrap_or_default();
    if rect.len() != 4 {
        return;
    }
    let w = (rect[2] - rect[0]).abs();
    let h = (rect[3] - rect[1]).abs();
    let size = appearance_font_size(doc, fid).min(h - 2.0).max(6.0);
    let baseline = (h - size) / 2.0 + size * 0.2;

    let content = format!(
        "/Tx BMC\nq\nBT\n/Helv {size} Tf\n0 g\n2 {baseline:.2} Td\n({text}) Tj\nET\nQ\nEMC",
        text = escape_pdf_string(val),
    );

    let mut fonts = Dictionary::new();
    fonts.set("Helv", Object::Reference(font));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(w),
            Object::Real(h),
        ]),
    );
    dict.set("Resources", Object::Dictionary(resources));
    let xobj = doc.add_object(Object::Stream(Stream::new(dict, content.into_bytes())));

    if let Ok(wd) = doc.get_object_mut(wid).and_then(|o| o.as_dict_mut()) {
        let mut ap = Dictionary::new();
        ap.set("N", Object::Reference(xobj));
        wd.set("AP", Object::Dictionary(ap));
    }
}

/// The font size from a field's (or the AcroForm's) `/DA`, defaulting to 10 and
/// treating the auto-size `0` as 10.
fn appearance_font_size(doc: &Document, fid: ObjectId) -> f32 {
    let da = doc
        .get_object(fid)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"DA").ok())
        .and_then(|o| o.as_str().ok())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default();
    // "/Helv 12 Tf 0 g" — the number before "Tf".
    if let Some(idx) = da.find("Tf") {
        if let Some(num) = da[..idx].split_whitespace().last() {
            if let Ok(sz) = num.parse::<f32>() {
                if sz > 0.0 {
                    return sz;
                }
            }
        }
    }
    10.0
}

fn escape_pdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Set the read-only flag (bit 1) on a field's `/Ff`.
fn set_readonly(doc: &mut Document, fid: ObjectId) {
    if let Ok(d) = doc.get_object_mut(fid).and_then(|o| o.as_dict_mut()) {
        let ff = d.get(b"Ff").and_then(|o| o.as_i64()).unwrap_or(0);
        d.set("Ff", Object::Integer(ff | 1));
    }
}

/// Check / uncheck a `/Btn` field. The "on" state is the widget's non-`Off`
/// appearance key; a truthy value selects it, anything else selects `/Off`.
fn fill_button(doc: &mut Document, fid: ObjectId, val: &str) {
    let truthy = matches!(
        val.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1" | "x" | "checked"
    );
    let on = on_state(doc, fid).unwrap_or_else(|| "Yes".to_string());
    let state = if truthy { on } else { "Off".to_string() };

    if let Ok(d) = doc.get_object_mut(fid).and_then(|o| o.as_dict_mut()) {
        d.set("V", Object::Name(state.as_bytes().to_vec()));
        if d.get(b"Rect").is_ok() {
            d.set("AS", Object::Name(state.as_bytes().to_vec()));
        }
    }
    for wid in widget_ids(doc, fid) {
        if wid == fid {
            continue;
        }
        if let Ok(wd) = doc.get_object_mut(wid).and_then(|o| o.as_dict_mut()) {
            wd.set("AS", Object::Name(state.as_bytes().to_vec()));
        }
    }
}

/// The "on" state name of a checkbox/radio: the first non-`Off` key of a
/// widget's `/AP /N` dictionary.
fn on_state(doc: &Document, fid: ObjectId) -> Option<String> {
    for wid in widget_ids(doc, fid) {
        let ap = doc
            .get_object(wid)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"AP").ok())
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"N").ok())
            .and_then(|o| o.as_dict().ok());
        if let Some(n) = ap {
            for (k, _) in n.iter() {
                if k != b"Off" {
                    return Some(String::from_utf8_lossy(k).into_owned());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// Build a minimal one-page PDF with an AcroForm carrying a text field
    /// (`full_name`) and a checkbox (`agree`, on-state `Yes`).
    fn form_pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let text_field = doc.add_object(dictionary! {
            "FT" => Object::Name(b"Tx".to_vec()),
            "T" => Object::string_literal("full_name"),
            "Rect" => vec![50.into(), 700.into(), 300.into(), 720.into()],
            "Subtype" => Object::Name(b"Widget".to_vec()),
            "Type" => Object::Name(b"Annot".to_vec()),
            "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        });
        let mut on_ap = Dictionary::new();
        on_ap.set("Yes", Object::Reference(text_field)); // any ref; only keys matter
        on_ap.set("Off", Object::Reference(text_field));
        let checkbox = doc.add_object(dictionary! {
            "FT" => Object::Name(b"Btn".to_vec()),
            "T" => Object::string_literal("agree"),
            "Rect" => vec![50.into(), 660.into(), 66.into(), 676.into()],
            "Subtype" => Object::Name(b"Widget".to_vec()),
            "Type" => Object::Name(b"Annot".to_vec()),
            "AS" => Object::Name(b"Off".to_vec()),
            "AP" => Object::Dictionary(dictionary! { "N" => Object::Dictionary(on_ap) }),
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Annots" => vec![Object::Reference(text_field), Object::Reference(checkbox)],
        });
        doc.set_object(
            pages_id,
            dictionary! {
                "Type" => Object::Name(b"Pages".to_vec()),
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            },
        );
        let acroform = doc.add_object(dictionary! {
            "Fields" => vec![Object::Reference(text_field), Object::Reference(checkbox)],
            "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        });
        let catalog = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Catalog".to_vec()),
            "Pages" => Object::Reference(pages_id),
            "AcroForm" => Object::Reference(acroform),
        });
        doc.trailer.set("Root", Object::Reference(catalog));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn field_named<'a>(doc: &'a Document, name: &str) -> &'a Dictionary {
        for (_, obj) in doc.objects.iter() {
            if let Ok(d) = obj.as_dict() {
                if d.get(b"T").ok().and_then(|o| o.as_str().ok()) == Some(name.as_bytes()) {
                    return d;
                }
            }
        }
        panic!("field {name} not found");
    }

    #[test]
    fn fills_text_and_checkbox_values() {
        let pdf = form_pdf();
        let filled = fill_form(
            &pdf,
            &[
                ("full_name".into(), "Ada Lovelace".into()),
                ("agree".into(), "yes".into()),
            ],
            false,
        )
        .expect("fill");
        let doc = Document::load_mem(&filled).unwrap();

        let name = field_named(&doc, "full_name");
        assert_eq!(
            name.get(b"V").unwrap().as_str().unwrap(),
            b"Ada Lovelace",
            "text /V set"
        );
        let agree = field_named(&doc, "agree");
        assert_eq!(
            agree.get(b"V").unwrap().as_name().unwrap(),
            b"Yes",
            "checkbox on"
        );
        assert_eq!(agree.get(b"AS").unwrap().as_name().unwrap(), b"Yes");
    }

    #[test]
    fn need_appearances_on_when_not_flattened() {
        let pdf = form_pdf();
        let filled = fill_form(&pdf, &[("full_name".into(), "x".into())], false).expect("fill");
        let doc = Document::load_mem(&filled).unwrap();
        let af = doc
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        let af = doc.get_object(af).unwrap().as_dict().unwrap();
        assert!(af.get(b"NeedAppearances").unwrap().as_bool().unwrap());
    }

    #[test]
    fn flatten_bakes_appearance_and_locks_the_field() {
        let pdf = form_pdf();
        let filled = fill_form(&pdf, &[("full_name".into(), "Ada".into())], true).expect("flatten");
        let doc = Document::load_mem(&filled).unwrap();

        let name = field_named(&doc, "full_name");
        assert!(
            name.get(b"AP").is_ok(),
            "an appearance stream was generated"
        );
        assert_eq!(
            name.get(b"Ff").unwrap().as_i64().unwrap() & 1,
            1,
            "read-only"
        );

        let af = doc
            .catalog()
            .unwrap()
            .get(b"AcroForm")
            .unwrap()
            .as_reference()
            .unwrap();
        let af = doc.get_object(af).unwrap().as_dict().unwrap();
        assert!(
            !af.get(b"NeedAppearances").unwrap().as_bool().unwrap(),
            "NeedAppearances off after flatten"
        );
    }

    #[test]
    fn errors_without_an_acroform() {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => Object::Reference(pages),
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        doc.set_object(
            pages,
            dictionary! {
                "Type" => Object::Name(b"Pages".to_vec()),
                "Kids" => vec![Object::Reference(page)], "Count" => 1,
            },
        );
        let cat = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Catalog".to_vec()), "Pages" => Object::Reference(pages),
        });
        doc.trailer.set("Root", Object::Reference(cat));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let err = fill_form(&bytes, &[("x".into(), "y".into())], false).unwrap_err();
        assert!(err.to_string().contains("no AcroForm"));
    }
}
