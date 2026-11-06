//! Where the framework's script tags go in an HTML page, and which of them a
//! page still needs.
//!
//! `finish_html` gives every page `nav.js` (or `prefetch.js`, with
//! `SOLI_NAV=off`), and `native.js`, `camera.js`, `sensors.js` and `pow.js` to
//! the pages whose own markup asks for them. Each of those decisions is a
//! substring test (the feature's gate, such as `data-soli-camera`, and the
//! marker a tag already in the page carries), and every tag goes before the
//! last `</body>`. Taken one script at a time, that was six forward scans of
//! the page and one backward search per tag added. On a fragment without
//! `</body>` (an htmx swap, a `layout: false` render) that search lowercased
//! every byte of the page, once for `</body>` and once more for `</html>`.
//!
//! [`add_framework_scripts`] answers every test from one forward pass
//! ([`Probe`]) and places every tag from one backward pass
//! ([`body_close_position`]). The bytes are the ones the injectors produced
//! one at a time; the tests below hold it to that.

use std::borrow::Cow;
use std::sync::OnceLock;

use memchr::memmem;

use crate::serve::{camera, native, nav, pow, prefetch, sensors};

/// The script every page gets, chosen once for the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Navigation {
    /// Instant navigation, `nav.js`: the default.
    Nav,
    /// Hover preloading, `prefetch.js`: `SOLI_NAV=off`.
    Prefetch,
    /// Neither: `SOLI_NAV=off` and `SOLI_PREFETCH=off`.
    Off,
}

impl Navigation {
    /// The process's choice. Nav subsumes hover prefetching with its own
    /// in-memory cache (a `fetch()` cannot consume `<link rel="prefetch">`
    /// entries), so the two scripts exclude each other: `prefetch.js` only
    /// goes out when nav is off.
    pub(crate) fn from_env() -> Self {
        if nav::is_enabled() {
            Navigation::Nav
        } else if prefetch::is_enabled() {
            Navigation::Prefetch
        } else {
            Navigation::Off
        }
    }
}

/// What a feature module answers for a page: the `<script>` tag it still
/// needs, or `None`.
pub(crate) type TagFor = fn(&Probe<'_>) -> Option<Cow<'static, str>>;

/// `html` with the framework's scripts it still needs, each where the
/// injectors put it one at a time: before the last `</body>`, else the last
/// `</html>`, else at the end. In this order: the navigation script, then the
/// native bridge (a page that called `native_channel(...)`), the camera (a
/// `data-soli-camera` element), the motion sensors (`motion_sensors()` or a
/// `soli.sensors` reference) and proof of work (a `pow_field()` challenge).
///
/// Asking every question of one [`Probe`] and placing every tag at one
/// position gives the bytes the one-at-a-time injection gave because no tag
/// holds `</body>`, `</html>`, or another script's gate or marker: adding one
/// never changes what the next one sees.
pub(crate) fn add_framework_scripts(mut html: String, navigation: Navigation) -> String {
    let tags = tags_for(&Probe::new(&html), navigation);
    let added: usize = tags.iter().flatten().map(|tag| tag.len()).sum();
    if added == 0 {
        return html;
    }
    html.reserve(added);
    let mut at = body_close_position(&html);
    for tag in tags.iter().flatten() {
        html.insert_str(at, tag);
        at += tag.len();
    }
    html
}

/// What each script answers for `page`, in the order the tags go in.
fn tags_for(page: &Probe<'_>, navigation: Navigation) -> [Option<Cow<'static, str>>; 5] {
    [
        match navigation {
            Navigation::Nav => nav::tag_for(page),
            Navigation::Prefetch => prefetch::tag_for(page),
            Navigation::Off => None,
        },
        native::tag_for(page),
        camera::tag_for(page),
        sensors::tag_for(page),
        pow::tag_for(page),
    ]
}

/// One script's injection on its own, behind each module's `inject_*_tag`.
/// A page that needs nothing is handed back without a copy.
pub(crate) fn inject_with(html: String, tag_for: TagFor) -> String {
    let tag = tag_for(&Probe::new(&html));
    match tag {
        Some(tag) => insert_before_body_close(html, &tag),
        None => html,
    }
}

/// Insert `tag` into `html` in place at [`body_close_position`] and hand the
/// same buffer back, so no injector copies the body to add a few dozen bytes.
pub(crate) fn insert_before_body_close(mut html: String, tag: &str) -> String {
    let at = body_close_position(&html);
    html.insert_str(at, tag);
    html
}

/// Where a script tag goes in `html`: at the last `</body>`, else at the last
/// `</html>` (both ASCII case-insensitive), else at the end.
///
/// One backward pass over the page's `<` bytes, which `memrchr` finds with
/// SIMD. Only a `<` can start either tag, so nothing else needs a look: a
/// page that ends in `</body></html>` is settled within its last bytes, and a
/// fragment with neither tag in one pass.
pub(crate) fn body_close_position(html: &str) -> usize {
    let bytes = html.as_bytes();
    let mut last_html_close = None;
    for at in memchr::memrchr_iter(b'<', bytes) {
        let Some(tag) = bytes.get(at..at + 7) else {
            continue;
        };
        if tag[1] != b'/' || tag[6] != b'>' {
            continue;
        }
        let name = &tag[2..6];
        if name.eq_ignore_ascii_case(b"body") {
            return at;
        }
        if last_html_close.is_none() && name.eq_ignore_ascii_case(b"html") {
            last_html_close = Some(at);
        }
    }
    last_html_close.unwrap_or(bytes.len())
}

/// Every string a framework script's injection asks about: the gates pages
/// opt in with, and the marker each tag starts with. Each holds `soli`, which
/// is what lets [`Probe::new`] find them all in one pass.
const NEEDLES: [&str; 11] = [
    nav::INJECTED_MARKER,
    prefetch::INJECTED_MARKER,
    native::CHANNEL_META,
    native::INJECTED_MARKER,
    camera::CAMERA_ATTR,
    camera::INJECTED_MARKER,
    sensors::SENSORS_META,
    sensors::SENSORS_USAGE,
    sensors::INJECTED_MARKER,
    pow::POW_ATTR,
    pow::INJECTED_MARKER,
];

const _: () = assert!(NEEDLES.len() <= u16::BITS as usize, "one bit per needle");

/// Where `soli` starts in each needle. The build fails for a needle without
/// it, or that ends with it: [`AFTER_SOLI`] sorts them by the byte that
/// follows.
const SOLI_AT: [usize; NEEDLES.len()] = {
    let mut offsets = [0; NEEDLES.len()];
    let mut i = 0;
    while i < NEEDLES.len() {
        offsets[i] = soli_in(NEEDLES[i].as_bytes());
        i += 1;
    }
    offsets
};

/// For each byte, the needles in which it follows `soli`, one bit per needle.
/// A `soli` followed by any other byte (`soli serve`, `solidb`) is part of no
/// needle and costs one lookup; one followed by `.` is compared with
/// `soli.sensors` alone.
const AFTER_SOLI: [u16; 256] = {
    let mut after = [0u16; 256];
    let mut i = 0;
    while i < NEEDLES.len() {
        after[NEEDLES[i].as_bytes()[SOLI_AT[i] + 4] as usize] |= 1 << i;
        i += 1;
    }
    after
};

const fn soli_in(needle: &[u8]) -> usize {
    let mut at = 0;
    while at + 4 < needle.len() {
        if needle[at] == b's'
            && needle[at + 1] == b'o'
            && needle[at + 2] == b'l'
            && needle[at + 3] == b'i'
        {
            return at;
        }
        at += 1;
    }
    panic!("each needle holds \"soli\" followed by another byte");
}

/// What a page holds, as far as the framework's scripts ask: each of
/// [`NEEDLES`], found in one forward pass. `memmem` finds every `soli` in the
/// page with SIMD (`soli` cannot overlap itself, so its non-overlapping
/// matches are all of them), and each one is compared with the needles not
/// found yet that continue with the byte after it.
pub(crate) struct Probe<'a> {
    html: &'a str,
    found: u16,
    /// How many times this probe was asked about a string the pass did not
    /// look for, and searched the page for it.
    #[cfg(test)]
    searches: std::cell::Cell<usize>,
}

impl<'a> Probe<'a> {
    pub(crate) fn new(html: &'a str) -> Self {
        static SOLI: OnceLock<memmem::Finder<'static>> = OnceLock::new();
        let bytes = html.as_bytes();
        let mut found = 0u16;
        for at in SOLI
            .get_or_init(|| memmem::Finder::new("soli"))
            .find_iter(bytes)
        {
            let Some(&next) = bytes.get(at + 4) else {
                continue;
            };
            let mut candidates = AFTER_SOLI[next as usize] & !found;
            while candidates != 0 {
                let bit = candidates.trailing_zeros() as usize;
                candidates &= candidates - 1;
                let needle = NEEDLES[bit].as_bytes();
                let Some(start) = at.checked_sub(SOLI_AT[bit]) else {
                    continue;
                };
                if bytes.get(start..start + needle.len()) == Some(needle) {
                    found |= 1 << bit;
                }
            }
        }
        Self {
            html,
            found,
            #[cfg(test)]
            searches: std::cell::Cell::new(0),
        }
    }

    /// Whether the page holds `needle`. A needle the pass did not look for is
    /// searched for, so the answer is always right; the tests hold every
    /// script to asking only about [`NEEDLES`].
    pub(crate) fn contains(&self, needle: &str) -> bool {
        match NEEDLES.iter().position(|known| *known == needle) {
            Some(bit) => self.found & (1 << bit) != 0,
            None => {
                #[cfg(test)]
                self.searches.set(self.searches.get() + 1);
                self.html.contains(needle)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search this module replaced, kept as the oracle: from the end of
    /// the page, byte by byte, lowercasing each byte.
    fn rfind_lowercased(haystack: &str, needle: &[u8]) -> Option<usize> {
        let bytes = haystack.as_bytes();
        if bytes.len() < needle.len() {
            return None;
        }
        (0..=bytes.len() - needle.len()).rev().find(|&i| {
            bytes[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(byte, expected)| byte.to_ascii_lowercase() == *expected)
        })
    }

    /// Where an injector put its tag before: the last `</body>`, else the
    /// last `</html>`, else the end.
    fn old_position(html: &str) -> usize {
        rfind_lowercased(html, b"</body>")
            .or_else(|| rfind_lowercased(html, b"</html>"))
            .unwrap_or(html.len())
    }

    /// `count` pages of up to eight pieces each, drawn by a fixed xorshift so
    /// that a failure reproduces.
    fn pages(pieces: &[&str], count: usize) -> Vec<String> {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as usize
        };
        (0..count)
            .map(|_| {
                let len = next() % 9;
                (0..len).map(|_| pieces[next() % pieces.len()]).collect()
            })
            .collect()
    }

    const TAG_PIECES: &[&str] = &[
        "</body>",
        "</BODY>",
        "</Body>",
        "</bOdY>",
        "</html>",
        "</HTML>",
        "</Html>",
        "<body>",
        "<html>",
        "</bod",
        "y>",
        "</htm",
        "l>",
        "<",
        "/",
        ">",
        "</",
        "<</",
        "</body >",
        "</ body>",
        "</bodyx>",
        "</span>",
        "<p>text</p>",
        "é",
        "日本語",
        "\n",
        "x",
    ];

    #[test]
    fn tags_go_before_the_last_body_close_else_the_last_html_close_else_at_the_end() {
        assert_eq!(body_close_position(""), 0);
        assert_eq!(body_close_position("<p>a fragment</p>"), 17);
        assert_eq!(body_close_position("<body>a</body>b</BODY></html>"), 15);
        assert_eq!(body_close_position("<html>a</html>b</HTML>c"), 15);
        assert_eq!(body_close_position("</body></html>"), 0);
        assert_eq!(body_close_position("</html></body>"), 7);
        // Truncated or spaced tags are not tags.
        assert_eq!(body_close_position("x</body"), 7);
        assert_eq!(body_close_position("</ body>"), 8);
        assert_eq!(body_close_position("</body >"), 8);
        // Positions are bytes, and always fall on a char boundary.
        assert_eq!(body_close_position("é</body>é"), 2);
    }

    #[test]
    fn tags_go_where_the_byte_by_byte_search_put_them() {
        for page in pages(TAG_PIECES, 20_000) {
            assert_eq!(body_close_position(&page), old_position(&page), "{page:?}");
        }
    }

    #[test]
    fn the_probe_answers_as_a_search_would() {
        let mut pieces = vec![
            "soli",
            "sosoli",
            "soli-",
            "soli_",
            "soli.",
            "__soli_",
            "solisoli",
            "soli serve",
            "Soli",
            "SOLI",
            "<p>",
            " ",
        ];
        for needle in NEEDLES {
            pieces.push(needle);
            pieces.push(&needle[..needle.len() - 1]);
            pieces.push(&needle[1..]);
        }
        for page in pages(&pieces, 20_000) {
            let probe = Probe::new(&page);
            for needle in NEEDLES {
                assert_eq!(
                    probe.contains(needle),
                    page.contains(needle),
                    "{needle:?} in {page:?}"
                );
            }
        }
    }

    #[test]
    fn a_string_the_pass_did_not_look_for_is_searched_for() {
        let probe = Probe::new("<p>hello</p>");
        assert!(probe.contains("hello"));
        assert!(!probe.contains("goodbye"));
    }

    /// The tag a gated module adds to a page that carries only its gate.
    fn tag_of(tag_for: TagFor, gate: &str) -> String {
        tag_for(&Probe::new(gate))
            .expect("the gate asks for the tag")
            .into_owned()
    }

    /// One script's injection as `finish_html` did it before: its questions
    /// asked of the page as it then stood, its own search for the place.
    fn add_one(html: &mut String, wanted: bool, marker: &str, tag: &str) {
        if wanted && !html.contains(marker) {
            let at = old_position(html);
            html.insert_str(at, tag);
        }
    }

    /// `finish_html` before the one-pass version: each script in turn.
    /// `navigation` is the marker and tag of nav.js or prefetch.js.
    fn one_at_a_time(page: &str, navigation: Option<(&str, &str)>) -> String {
        let mut html = page.to_string();
        if let Some((marker, tag)) = navigation {
            add_one(&mut html, true, marker, tag);
        }
        let wanted = html.contains(native::CHANNEL_META);
        let tag = tag_of(native::tag_for, native::CHANNEL_META);
        add_one(&mut html, wanted, native::INJECTED_MARKER, &tag);
        let wanted = html.contains(camera::CAMERA_ATTR);
        let tag = tag_of(camera::tag_for, camera::CAMERA_ATTR);
        add_one(&mut html, wanted, camera::INJECTED_MARKER, &tag);
        let wanted = html.contains(sensors::SENSORS_META) || html.contains(sensors::SENSORS_USAGE);
        let tag = tag_of(sensors::tag_for, sensors::SENSORS_META);
        add_one(&mut html, wanted, sensors::INJECTED_MARKER, &tag);
        let wanted = html.contains(pow::POW_ATTR);
        let tag = tag_of(pow::tag_for, pow::POW_ATTR);
        add_one(&mut html, wanted, pow::INJECTED_MARKER, &tag);
        html
    }

    /// Pages that ask for every script, already carry some, and put their
    /// closing tags anywhere.
    fn feature_pieces() -> Vec<String> {
        let mut pieces: Vec<String> = [
            "<html><head>",
            "</head><body>",
            "</body>",
            "</BODY>",
            "</html>",
            "<meta name=\"soli-native\" content=\"tok\">",
            "<video data-soli-camera autoplay></video>",
            "<meta name=\"soli-sensors\" content=\"1\">",
            "<script>soli.sensors.gyroscope(function(){});</script>",
            "<input data-soli-pow=\"x\">",
            "<p>soli serve</p>",
            "<tr><td>row</td></tr>",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        // A tag already in the page, as a middleware that re-enters
        // `html_response` leaves it.
        for marker in [
            nav::INJECTED_MARKER,
            prefetch::INJECTED_MARKER,
            native::INJECTED_MARKER,
            camera::INJECTED_MARKER,
            sensors::INJECTED_MARKER,
            pow::INJECTED_MARKER,
        ] {
            pieces.push(format!("<!-- {marker} -->"));
        }
        pieces
    }

    /// The nav tag `with_nav` gained over `page`. Under test the tag is
    /// rebuilt from `SOLI_PREFETCH` and friends on every call, which other
    /// tests change, so the comparison uses the tag this call added.
    fn added_nav_tag<'a>(page: &str, with_nav: &'a str) -> Option<&'a str> {
        if page.contains(nav::INJECTED_MARKER) {
            return None;
        }
        let start = with_nav
            .find(&format!("<!-- {} -->", nav::INJECTED_MARKER))
            .expect("nav.js goes on every page");
        let end = start + with_nav[start..].find("</script>").unwrap() + "</script>".len();
        Some(&with_nav[start..end])
    }

    #[test]
    fn one_pass_adds_the_bytes_the_injectors_added_one_at_a_time() {
        let pieces = feature_pieces();
        let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
        let prefetch_tag = prefetch::tag_for(&Probe::new("")).unwrap().into_owned();
        for page in pages(&pieces, 5_000) {
            assert_eq!(
                add_framework_scripts(page.clone(), Navigation::Off),
                one_at_a_time(&page, None),
                "{page:?}"
            );
            assert_eq!(
                add_framework_scripts(page.clone(), Navigation::Prefetch),
                one_at_a_time(
                    &page,
                    Some((prefetch::INJECTED_MARKER, prefetch_tag.as_str()))
                ),
                "{page:?}"
            );
            let with_nav = add_framework_scripts(page.clone(), Navigation::Nav);
            let nav_tag = added_nav_tag(&page, &with_nav);
            assert_eq!(
                with_nav,
                one_at_a_time(&page, nav_tag.map(|tag| (nav::INJECTED_MARKER, tag))),
                "{page:?}"
            );
        }
    }

    #[test]
    fn a_page_that_needs_no_script_is_handed_back_as_it_came() {
        let page = format!("<p>x</p><!-- {} -->", nav::INJECTED_MARKER);
        let before = page.as_ptr();
        let out = add_framework_scripts(page, Navigation::Nav);
        assert_eq!(out.as_ptr(), before, "no copy");
        assert_eq!(out, format!("<p>x</p><!-- {} -->", nav::INJECTED_MARKER));
    }

    /// What makes one pass exact: adding a tag changes nothing another script
    /// asks about. A tag that held `</body>`, or another script's gate or
    /// marker, would need the questions asked again after it went in.
    #[test]
    fn no_tag_holds_a_body_close_or_another_script_s_needle() {
        let tags = [
            (
                nav::tag_for(&Probe::new("")).unwrap().into_owned(),
                nav::INJECTED_MARKER,
            ),
            (
                prefetch::tag_for(&Probe::new("")).unwrap().into_owned(),
                prefetch::INJECTED_MARKER,
            ),
            (
                tag_of(native::tag_for, native::CHANNEL_META),
                native::INJECTED_MARKER,
            ),
            (
                tag_of(camera::tag_for, camera::CAMERA_ATTR),
                camera::INJECTED_MARKER,
            ),
            (
                tag_of(sensors::tag_for, sensors::SENSORS_META),
                sensors::INJECTED_MARKER,
            ),
            (tag_of(pow::tag_for, pow::POW_ATTR), pow::INJECTED_MARKER),
        ];
        for (tag, own_marker) in &tags {
            assert_eq!(
                old_position(tag),
                tag.len(),
                "{tag} holds </body> or </html>"
            );
            for needle in NEEDLES {
                assert_eq!(
                    tag.contains(needle),
                    needle == *own_marker,
                    "{tag} and {needle}"
                );
            }
        }
    }

    /// The fast path: every question the scripts ask is answered from the
    /// one pass. A gate missing from `NEEDLES` would still be answered right,
    /// by a search of the whole page on every response; this test says so.
    #[test]
    fn the_scripts_ask_only_what_the_one_pass_answers() {
        let pieces = feature_pieces();
        let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
        for page in pages(&pieces, 500) {
            let probe = Probe::new(&page);
            for navigation in [Navigation::Nav, Navigation::Prefetch, Navigation::Off] {
                tags_for(&probe, navigation);
            }
            assert_eq!(
                probe.searches.get(),
                0,
                "a script asked about a string outside NEEDLES: {page:?}"
            );
        }
        let probe = Probe::new("<p>");
        assert!(probe.contains("<p>"));
        assert_eq!(probe.searches.get(), 1, "a search is counted");
    }
}
