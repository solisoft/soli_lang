//! Encryption at rest for stored blobs: the SBE ("Soli Blob Encryption") format.
//!
//! An `uploader(..., {"encrypted": true})` or `has_*_attached(..., {"encrypted":
//! true})` seals the bytes before they reach SoliDB, the disk, S3 or R2, and
//! opens them on the way back. Model fields have `encrypts`, which seals one
//! string with one AES-GCM call; a blob is a file, so it is sealed in segments.
//!
//! ```text
//! header (19 bytes)
//!   "SBE" | version = 1 | key_id[4] | segment_size: u32 BE | nonce_prefix[7]
//! segments
//!   AES-256-GCM(plain[i]) ‖ tag[16]
//!   nonce = nonce_prefix ‖ i: u32 BE ‖ last: u8 (1 on the final segment)
//!   aad   = header ‖ purpose            (purpose = "soli-blob:<collection>")
//! ```
//!
//! This is the STREAM construction (Hoang, Reyhanitabar, Rogaway, Vizár): the
//! counter in the nonce stops segments being reordered or replayed, the `last`
//! flag stops a sealed blob being cut short at a segment boundary, and the
//! header in every segment's AAD stops the segment size or key id being edited.
//! Every plaintext segment but the last is exactly `segment_size` bytes and the
//! last is shorter (empty when the length divides evenly), so where the
//! segments split is never ambiguous.
//!
//! What it borrows from Fernet is the envelope, not the cipher: a version byte
//! so the format can change without stranding stored blobs, and a key id so a
//! key can be rotated: new blobs are sealed with `SOLI_ENCRYPTION_KEY`, older
//! ones open with any key in `SOLI_ENCRYPTION_OLD_KEYS`.
//!
//! The blob key is not the `encrypts` field key: both come from the same
//! material, under different labels, so a field ciphertext and a blob segment
//! can never be passed off as each other.

use aes_gcm::{aead::Aead, Aes256Gcm, Nonce};
use argon2::password_hash::rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 3] = b"SBE";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 3 + 1 + 4 + 4 + 7;
const TAG_LEN: usize = 16;
/// 64 KiB of plaintext per segment: 16 bytes of tag each, about 0.02 %.
pub const DEFAULT_SEGMENT: u32 = 64 * 1024;
/// Bounds a header may name. The header is authenticated, but only by the
/// first segment it opens: refuse absurd sizes before allocating for them.
const MIN_SEGMENT: u32 = 1024;
const MAX_SEGMENT: u32 = 16 * 1024 * 1024;

/// A blob key and the four bytes that name it in a header.
#[derive(Clone, PartialEq, Eq)]
pub struct BlobKey {
    key: [u8; 32],
    id: [u8; 4],
}

impl std::fmt::Debug for BlobKey {
    // The key never reaches a log line; its id is public anyway.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlobKey({:02x?})", self.id)
    }
}

impl BlobKey {
    /// Derive the blob key from key material (the value of
    /// `SOLI_ENCRYPTION_KEY`, or one of the old keys).
    pub fn from_material(material: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"soli-blob-v1\0");
        hasher.update(material);
        let key: [u8; 32] = hasher.finalize().into();
        let mut hasher = Sha256::new();
        hasher.update(b"soli-blob-key-id\0");
        hasher.update(key);
        let digest = hasher.finalize();
        let mut id = [0u8; 4];
        id.copy_from_slice(&digest[..4]);
        BlobKey { key, id }
    }
}

/// The keys this process can use: the write key first, then the old keys,
/// which only open.
pub fn keyring() -> Result<Vec<BlobKey>, String> {
    let current = crate::platform::env::var("SOLI_ENCRYPTION_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "encrypted attachments need SOLI_ENCRYPTION_KEY".to_string())?;
    let mut keys = vec![BlobKey::from_material(current.as_bytes())];
    if let Ok(old) = crate::platform::env::var("SOLI_ENCRYPTION_OLD_KEYS") {
        for material in old.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            keys.push(BlobKey::from_material(material.as_bytes()));
        }
    }
    Ok(keys)
}

/// The purpose a blob is sealed for: the collection it is stored in. A sealed
/// blob copied into another uploader's collection does not open there.
pub fn purpose(collection: &str) -> Vec<u8> {
    format!("soli-blob:{collection}").into_bytes()
}

/// Whether `data` starts with an SBE header this build reads.
pub fn is_sealed(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN && &data[..3] == MAGIC && data[3] == VERSION
}

/// Seal `plain` with the current key.
pub fn seal(plain: &[u8], purpose: &[u8]) -> Result<Vec<u8>, String> {
    let keys = keyring()?;
    seal_with(plain, purpose, &keys[0], DEFAULT_SEGMENT)
}

/// Open a sealed blob with whichever key of the ring its header names.
pub fn open(data: &[u8], purpose: &[u8]) -> Result<Vec<u8>, String> {
    open_with(data, purpose, &keyring()?)
}

fn cipher(key: &BlobKey) -> Result<Aes256Gcm, String> {
    <Aes256Gcm as aes_gcm::aead::KeyInit>::new_from_slice(&key.key)
        .map_err(|e| format!("bad blob key: {e}"))
}

fn nonce(prefix: &[u8], index: u32, last: bool) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..7].copy_from_slice(prefix);
    nonce[7..11].copy_from_slice(&index.to_be_bytes());
    nonce[11] = u8::from(last);
    nonce
}

fn segment_aad(header: &[u8], purpose: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(header.len() + purpose.len());
    aad.extend_from_slice(header);
    aad.extend_from_slice(purpose);
    aad
}

pub fn seal_with(
    plain: &[u8],
    purpose: &[u8],
    key: &BlobKey,
    segment: u32,
) -> Result<Vec<u8>, String> {
    if !(MIN_SEGMENT..=MAX_SEGMENT).contains(&segment) {
        return Err(format!("blob segment size {segment} is out of range"));
    }
    let seg = segment as usize;
    let full_segments = plain.len() / seg;
    // Every full segment is followed by one more, so the count is one above.
    if full_segments >= u32::MAX as usize {
        return Err("blob is too large to seal".to_string());
    }

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.extend_from_slice(&key.id);
    header.extend_from_slice(&segment.to_be_bytes());
    let mut prefix = [0u8; 7];
    OsRng.fill_bytes(&mut prefix);
    header.extend_from_slice(&prefix);

    let aad = segment_aad(&header, purpose);
    let cipher = cipher(key)?;
    let mut out = Vec::with_capacity(HEADER_LEN + plain.len() + (full_segments + 1) * TAG_LEN);
    out.extend_from_slice(&header);
    for index in 0..=full_segments {
        let start = index * seg;
        let end = (start + seg).min(plain.len());
        let last = index == full_segments;
        let nonce = nonce(&prefix, index as u32, last);
        #[allow(deprecated)]
        let sealed = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                aes_gcm::aead::Payload {
                    msg: &plain[start..end],
                    aad: &aad,
                },
            )
            .map_err(|e| format!("blob encryption failed: {e}"))?;
        out.extend_from_slice(&sealed);
    }
    Ok(out)
}

pub fn open_with(data: &[u8], purpose: &[u8], keys: &[BlobKey]) -> Result<Vec<u8>, String> {
    if !is_sealed(data) {
        return Err("not an encrypted blob".to_string());
    }
    let opener = Opener::new(&data[..HEADER_LEN], data.len() as u64, purpose, keys)?;
    let mut out = Vec::with_capacity(opener.plain_len() as usize);
    for index in 0..=opener.last_segment() {
        let (first, last) = opener.sealed_span(index, index);
        out.extend_from_slice(&opener.open_segment(index, &data[first as usize..=last as usize])?);
    }
    Ok(out)
}

/// Opens one sealed blob a segment at a time: what a reader that streams it,
/// or serves a byte range of it, needs. Built from the header and the sealed
/// blob's total length, which together fix where every segment lies.
pub struct Opener {
    cipher: Aes256Gcm,
    aad: Vec<u8>,
    prefix: [u8; 7],
    /// Plaintext bytes per segment.
    segment: u64,
    /// Index of the last segment (there are `last + 1`).
    last: u64,
    /// Sealed bytes of the last segment: its plaintext plus the tag.
    last_sealed: u64,
}

impl Opener {
    /// `header` is the blob's first [`HEADER_LEN`] bytes, `total` the sealed
    /// blob's length.
    pub fn new(
        header: &[u8],
        total: u64,
        purpose: &[u8],
        keys: &[BlobKey],
    ) -> Result<Self, String> {
        if !is_sealed(header) {
            return Err("not an encrypted blob".to_string());
        }
        let header = &header[..HEADER_LEN];
        let segment = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        if !(MIN_SEGMENT..=MAX_SEGMENT).contains(&segment) {
            return Err("encrypted blob has a corrupt header".to_string());
        }
        let key_id = &header[4..8];
        let key = keys.iter().find(|k| k.id == key_id).ok_or_else(|| {
            "encrypted blob was sealed with a key that is neither SOLI_ENCRYPTION_KEY \
             nor one of SOLI_ENCRYPTION_OLD_KEYS"
                .to_string()
        })?;
        // Every segment but the last is full; the last holds less than a full
        // segment's plaintext (none at all when the length divides evenly).
        let sealed_segment = segment as u64 + TAG_LEN as u64;
        let body = total
            .checked_sub(HEADER_LEN as u64)
            .ok_or_else(|| "encrypted blob is truncated".to_string())?;
        let last = body / sealed_segment;
        let last_sealed = body % sealed_segment;
        if last_sealed < TAG_LEN as u64 {
            return Err("encrypted blob is truncated".to_string());
        }
        if last > u32::MAX as u64 {
            return Err("encrypted blob has too many segments".to_string());
        }
        let mut prefix = [0u8; 7];
        prefix.copy_from_slice(&header[12..19]);
        Ok(Opener {
            cipher: cipher(key)?,
            aad: segment_aad(header, purpose),
            prefix,
            segment: segment as u64,
            last,
            last_sealed,
        })
    }

    /// The length of the file the blob seals.
    pub fn plain_len(&self) -> u64 {
        self.last * self.segment + (self.last_sealed - TAG_LEN as u64)
    }

    /// Plaintext bytes per segment: segment `i` holds bytes from
    /// `i * segment_size()`.
    pub fn segment_size(&self) -> u64 {
        self.segment
    }

    pub fn last_segment(&self) -> u64 {
        self.last
    }

    /// Sealed bytes segment `index` takes.
    pub fn sealed_len(&self, index: u64) -> u64 {
        if index == self.last {
            self.last_sealed
        } else {
            self.segment + TAG_LEN as u64
        }
    }

    /// The sealed blob's byte offsets (inclusive) holding segments
    /// `first..=last`.
    pub fn sealed_span(&self, first: u64, last: u64) -> (u64, u64) {
        let sealed_segment = self.segment + TAG_LEN as u64;
        let start = HEADER_LEN as u64 + first * sealed_segment;
        let end = HEADER_LEN as u64 + last * sealed_segment + self.sealed_len(last) - 1;
        (start, end)
    }

    /// Authenticate and decrypt segment `index` (exactly `sealed_len(index)`
    /// bytes). Nothing is returned for a segment that does not authenticate.
    pub fn open_segment(&self, index: u64, sealed: &[u8]) -> Result<Vec<u8>, String> {
        if index > self.last || sealed.len() as u64 != self.sealed_len(index) {
            return Err("encrypted blob is truncated".to_string());
        }
        let nonce = nonce(&self.prefix, index as u32, index == self.last);
        #[allow(deprecated)]
        self.cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                aes_gcm::aead::Payload {
                    msg: sealed,
                    aad: &self.aad,
                },
            )
            .map_err(|_| {
                "encrypted blob does not open (wrong key, wrong collection, or corrupt data)"
                    .to_string()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEG: u32 = 1024;

    fn key(material: &str) -> BlobKey {
        BlobKey::from_material(material.as_bytes())
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn round_trips_every_boundary() {
        let k = key("k1");
        for len in [0, 1, 1023, 1024, 1025, 2048, 5000] {
            let plain = sample(len);
            let sealed = seal_with(&plain, b"p", &k, SEG).unwrap();
            assert!(is_sealed(&sealed));
            let segments = len / SEG as usize + 1;
            assert_eq!(
                sealed.len(),
                HEADER_LEN + len + segments * TAG_LEN,
                "len {len}"
            );
            assert_eq!(
                open_with(&sealed, b"p", std::slice::from_ref(&k)).unwrap(),
                plain,
                "len {len}"
            );
        }
    }

    #[test]
    fn two_seals_of_the_same_bytes_differ() {
        let k = key("k1");
        let a = seal_with(b"same", b"p", &k, SEG).unwrap();
        let b = seal_with(b"same", b"p", &k, SEG).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn truncation_at_a_segment_boundary_is_refused() {
        let k = key("k1");
        let sealed = seal_with(&sample(3000), b"p", &k, SEG).unwrap();
        // Drop the last segment: what remains ends on a full segment, which a
        // reader must not take as the end.
        let cut = HEADER_LEN + 2 * (SEG as usize + TAG_LEN);
        assert!(open_with(&sealed[..cut], b"p", std::slice::from_ref(&k)).is_err());
        // And keep only the first full segment.
        let cut = HEADER_LEN + SEG as usize + TAG_LEN;
        assert!(open_with(&sealed[..cut], b"p", std::slice::from_ref(&k)).is_err());
        assert!(open_with(&sealed[..HEADER_LEN + 3], b"p", std::slice::from_ref(&k)).is_err());
    }

    #[test]
    fn reordered_segments_are_refused() {
        let k = key("k1");
        let sealed = seal_with(&sample(2500), b"p", &k, SEG).unwrap();
        let s = SEG as usize + TAG_LEN;
        let mut swapped = sealed[..HEADER_LEN].to_vec();
        swapped.extend_from_slice(&sealed[HEADER_LEN + s..HEADER_LEN + 2 * s]);
        swapped.extend_from_slice(&sealed[HEADER_LEN..HEADER_LEN + s]);
        swapped.extend_from_slice(&sealed[HEADER_LEN + 2 * s..]);
        assert!(open_with(&swapped, b"p", std::slice::from_ref(&k)).is_err());
    }

    #[test]
    fn another_purpose_does_not_open() {
        let k = key("k1");
        let sealed = seal_with(b"secret", &purpose("invoices"), &k, SEG).unwrap();
        assert!(open_with(&sealed, &purpose("avatars"), std::slice::from_ref(&k)).is_err());
        assert_eq!(
            open_with(&sealed, &purpose("invoices"), std::slice::from_ref(&k)).unwrap(),
            b"secret"
        );
    }

    #[test]
    fn a_flipped_bit_or_edited_header_is_refused() {
        let k = key("k1");
        let sealed = seal_with(&sample(1500), b"p", &k, SEG).unwrap();
        let mut body = sealed.clone();
        body[HEADER_LEN + 10] ^= 1;
        assert!(open_with(&body, b"p", std::slice::from_ref(&k)).is_err());
        let mut header = sealed.clone();
        header[14] ^= 1; // nonce prefix
        assert!(open_with(&header, b"p", std::slice::from_ref(&k)).is_err());
        let mut segment = sealed;
        segment[10] = 0x08; // segment size 1024 -> 2048
        assert!(open_with(&segment, b"p", std::slice::from_ref(&k)).is_err());
    }

    #[test]
    fn rotation_opens_with_an_old_key_and_refuses_an_unknown_one() {
        let old = key("old");
        let new = key("new");
        let sealed = seal_with(b"kept", b"p", &old, SEG).unwrap();
        assert_eq!(
            open_with(&sealed, b"p", &[new.clone(), old]).unwrap(),
            b"kept"
        );
        let err = open_with(&sealed, b"p", &[new]).unwrap_err();
        assert!(err.contains("SOLI_ENCRYPTION_OLD_KEYS"), "{err}");
    }

    #[test]
    fn the_opener_reads_any_segment_on_its_own() {
        let k = key("k1");
        for len in [0usize, 5, 1024, 3000] {
            let plain = sample(len);
            let sealed = seal_with(&plain, b"p", &k, SEG).unwrap();
            let opener = Opener::new(
                &sealed[..HEADER_LEN],
                sealed.len() as u64,
                b"p",
                std::slice::from_ref(&k),
            )
            .unwrap();
            assert_eq!(opener.plain_len(), len as u64);
            assert_eq!(opener.last_segment(), (len / SEG as usize) as u64);
            // Backwards, one segment at a time.
            for index in (0..=opener.last_segment()).rev() {
                let (a, b) = opener.sealed_span(index, index);
                let got = opener
                    .open_segment(index, &sealed[a as usize..=b as usize])
                    .unwrap();
                let from = index as usize * SEG as usize;
                assert_eq!(got, plain[from..(from + SEG as usize).min(len)]);
            }
            let (a, b) = opener.sealed_span(0, opener.last_segment());
            assert_eq!((a, b), (HEADER_LEN as u64, sealed.len() as u64 - 1));
        }
    }

    #[test]
    fn a_segment_does_not_open_at_another_index() {
        let k = key("k1");
        let sealed = seal_with(&sample(3000), b"p", &k, SEG).unwrap();
        let opener = Opener::new(
            &sealed[..HEADER_LEN],
            sealed.len() as u64,
            b"p",
            std::slice::from_ref(&k),
        )
        .unwrap();
        let (a, b) = opener.sealed_span(0, 0);
        assert!(opener
            .open_segment(1, &sealed[a as usize..=b as usize])
            .is_err());
        // A total length that cuts the last segment short is refused up front.
        let short = HEADER_LEN as u64 + 2 * (SEG as u64 + 16) + 3;
        assert!(Opener::new(&sealed[..HEADER_LEN], short, b"p", &[k]).is_err());
    }

    #[test]
    fn plaintext_is_not_taken_for_a_sealed_blob() {
        assert!(!is_sealed(b"\x89PNG\r\n\x1a\n...................."));
        assert!(!is_sealed(b"SBE"));
    }

    #[test]
    fn blob_key_is_not_the_field_key() {
        let material = b"same material";
        assert_ne!(
            BlobKey::from_material(material).key,
            crate::interpreter::builtins::crypto::derive_aes_key(material)
        );
    }
}
