//! Pre-embedding font subsetting.
//!
//! printpdf 0.9 embeds fonts in full (its own subsetting is hard-disabled
//! upstream). We shrink each face to the glyphs the document actually uses
//! *before* handing the bytes to printpdf.
//!
//! We use a **retain-GID** subset (`subsetter` 0.1, `Profile::pdf`): unused
//! glyph outlines are stripped but glyph IDs and the `cmap` are left unchanged.
//! That makes subsetting completely transparent to printpdf — it re-derives the
//! content-stream glyph ids, the `/W` widths, and `/ToUnicode` from whatever
//! bytes it is given, so a subset with identical GIDs yields a semantically
//! identical (just smaller) PDF, and the Factur-X `/CIDToGIDMap /Identity` stays
//! correct because CID = GID is unchanged.

/// Subset `original` to `gids` for PDF embedding, retaining glyph IDs and the
/// `cmap` table.
///
/// `index` selects a face within a `.ttc`/`.otc` collection (0 otherwise).
/// Subsetting is a non-fatal size optimization: on any failure this returns the
/// original bytes unchanged so rendering still succeeds.
pub(crate) fn subset_face(original: &[u8], index: u32, gids: &[u16]) -> Vec<u8> {
    match subsetter::subset(original, index, subsetter::Profile::pdf(gids)) {
        Ok(bytes) => bytes,
        Err(_) => original.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttf_parser::Face;

    static TITILLIUM: &[u8] = include_bytes!("../../fonts/TitilliumWeb-Regular.ttf");

    /// GIDs for the glyphs covering the given chars, plus .notdef.
    fn gids_for(font: &[u8], chars: &str) -> Vec<u16> {
        let face = Face::parse(font, 0).unwrap();
        let mut gids: std::collections::BTreeSet<u16> = std::collections::BTreeSet::from([0]);
        for ch in chars.chars() {
            if let Some(g) = face.glyph_index(ch) {
                gids.insert(g.0);
            }
        }
        gids.into_iter().collect()
    }

    #[test]
    fn subset_is_smaller_and_reparses() {
        let gids = gids_for(TITILLIUM, "Invoice");
        let sub = subset_face(TITILLIUM, 0, &gids);

        // Substantially smaller than the ~62 KB original.
        assert!(
            sub.len() < TITILLIUM.len() / 2,
            "subset {} not < half of original {}",
            sub.len(),
            TITILLIUM.len()
        );

        // Still a valid face, and glyph ids are retained (cmap intact).
        let orig = Face::parse(TITILLIUM, 0).unwrap();
        let face = Face::parse(&sub, 0).expect("subset font re-parses");
        for ch in "Invoice".chars() {
            assert_eq!(
                face.glyph_index(ch),
                orig.glyph_index(ch),
                "glyph id for {ch:?} changed after subsetting"
            );
            assert!(face.glyph_index(ch).is_some());
        }
    }

    #[test]
    fn subset_failure_falls_back_to_original() {
        // Not a font: subsetting must not panic and must return the input bytes.
        let garbage = b"this is definitely not an sfnt".to_vec();
        let out = subset_face(&garbage, 0, &[0]);
        assert_eq!(out, garbage);
    }
}
