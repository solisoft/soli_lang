//! Font roles an application declares (`spec/02-wire-format.md` §5.1).
//!
//! A style says `{"font": "Playfair Display"}`; the wire carries one byte.
//! This is the table in between: a name, the role it was given, and the
//! faces that draw it, each already in the asset store and named by its
//! hash.
//!
//! Per application, like the capabilities beside it: two co-hosted apps that
//! both declare a display face must not find each other's, and a role number
//! means nothing outside the app that handed it out.
//!
//! Nothing here fetches anything. A face gets into the store the way every
//! other asset does — a file under `public/` or `app/assets/`, or bytes
//! through `eui_asset` — and an application that wants a face from a font
//! service downloads it *itself*, once, and serves it from its own origin.
//! That is the whole reason this table holds hashes and not URLs: the client
//! opens no connection the session did not (08 §8).

use super::assets::Hash;
use crate::serve::tenant::TenantValue;

/// Roles `0` and `1` are the client's own sans and mono; an application may
/// replace either by declaring a face under that name.
const SANS: u8 = 0;
const MONO: u8 = 1;
/// The first role an application's own family is given.
const FIRST_ROLE: u8 = 2;

/// One declared family: the role it draws in, and the faces that draw it.
#[derive(Clone, Debug)]
pub struct Font {
    /// The name a style uses, as the application wrote it.
    pub name: String,
    /// The `font_family` byte.
    pub role: u8,
    /// BLAKE3 of each face. `font_weight` selects among them.
    pub faces: Vec<Hash>,
}

/// What this application declared, in the order it declared it.
static FONTS: TenantValue<Vec<Font>> = TenantValue::new(Vec::new);

/// `eui_font("Playfair Display", [...])`: bind a name to its faces and
/// answer the role they draw in.
///
/// Declaring the same name twice replaces its faces and keeps its role —
/// an application that re-reads its fonts on a code reload must not burn a
/// role each time. `"sans"` and `"mono"` are the client's own roles and
/// replace the embedded face rather than taking a new one.
pub fn declare(name: &str, faces: Vec<Hash>) -> Result<u8, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("eui_font: a font needs a name".into());
    }
    if faces.is_empty() {
        return Err(format!("eui_font: '{name}' names no face"));
    }
    let max = eui_proto::limits::MAX_FACES_PER_ROLE as usize;
    if faces.len() > max {
        return Err(format!(
            "eui_font: '{name}' names {} faces; a role carries at most {max} — one per weight",
            faces.len()
        ));
    }
    FONTS.write(|fonts| {
        if let Some(held) = fonts.iter_mut().find(|f| f.name == name) {
            held.faces = faces;
            return Ok(held.role);
        }
        let role = match name {
            "sans" => SANS,
            "mono" => MONO,
            _ => {
                let next = fonts
                    .iter()
                    .filter(|f| f.role >= FIRST_ROLE)
                    .map(|f| f.role)
                    .max()
                    .map_or(FIRST_ROLE, |r| r.saturating_add(1));
                if next > eui_proto::limits::MAX_FONT_ROLE {
                    return Err(format!(
                        "eui_font: '{name}' is one font too many; an application declares at most {} beside sans and mono",
                        eui_proto::limits::MAX_FONT_ROLE - FIRST_ROLE + 1
                    ));
                }
                next
            }
        };
        fonts.push(Font {
            name: name.to_owned(),
            role,
            faces,
        });
        Ok(role)
    })
}

/// The role a style's `"font"` name draws in, if the application declared it.
pub fn role_of(name: &str) -> Option<u8> {
    FONTS.read(|fonts| fonts.iter().find(|f| f.name == name).map(|f| f.role))
}

/// The faces bound to a role, for the `DefFont` that carries it.
pub fn faces_of(role: u8) -> Option<Vec<Hash>> {
    FONTS.read(|fonts| {
        fonts
            .iter()
            .find(|f| f.role == role)
            .map(|f| f.faces.clone())
    })
}

/// Whether this application declared any font at all.
///
/// The manifest asks for a client that speaks `DefFont` only when the answer
/// is yes: an application with no font of its own keeps every client it had.
pub fn any() -> bool {
    FONTS.read(|fonts| !fonts.is_empty())
}

/// Every declared name, for an error that has to list them.
pub fn names() -> Vec<String> {
    FONTS.read(|fonts| fonts.iter().map(|f| f.name.clone()).collect())
}
