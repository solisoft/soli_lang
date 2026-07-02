//! Password protection: encrypt the rendered PDF with AES-128 (PDF standard
//! security handler, revision 4) via lopdf. Runs as the final post-pass, after
//! stationery/attachments, so it sees every object.
//!
//! Two passwords: the **user** password opens the document (with the permitted
//! actions), the **owner** password additionally lifts the restrictions. When
//! only one is supplied the other mirrors it. `allow` whitelists actions the
//! user may perform — an empty list permits everything (a pure open-password).
//!
//! Encryption is incompatible with PDF/A (Factur-X); callers must not combine
//! them (the builtin/CLI enforce this).

use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
use lopdf::Document;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{PdfError, Result};

/// Password/permission settings for [`apply_encryption`].
#[derive(Debug, Clone, Default)]
pub struct EncryptOptions {
    /// Password required to open the document (empty = openable by anyone,
    /// but still restricted to `allow`).
    pub user_password: String,
    /// Password that lifts all restrictions (defaults to the user password).
    pub owner_password: String,
    /// Actions the user password permits: any of `print`, `copy`, `modify`,
    /// `annotate`. Empty = allow everything.
    pub allow: Vec<String>,
}

/// Encrypt `pdf` per `opts`, returning the protected bytes.
pub fn apply_encryption(pdf: &[u8], opts: &EncryptOptions) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf)
        .map_err(|e| PdfError::Backend(format!("encrypt: could not parse the render: {e}")))?;

    let owner = if opts.owner_password.is_empty() {
        opts.user_password.as_str()
    } else {
        opts.owner_password.as_str()
    };

    let permissions = resolve_permissions(&opts.allow);
    let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);

    // The version borrows `&doc` only to derive the keys; `try_from` returns an
    // owned state, releasing the borrow before the `&mut` encrypt call.
    let state = {
        let version = EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), crypt_filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: owner,
            user_password: &opts.user_password,
            permissions,
        };
        EncryptionState::try_from(version)
            .map_err(|e| PdfError::Backend(format!("encrypt: key setup failed: {e}")))?
    };

    doc.encrypt(&state)
        .map_err(|e| PdfError::Backend(format!("encrypt: {e}")))?;

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfError::Backend(format!("encrypt: could not save: {e}")))?;
    Ok(out)
}

/// Map the `allow` list to a permission bitmask. Empty = all permissions;
/// otherwise only the named actions (plus their high-quality/accessibility
/// companions) are granted.
fn resolve_permissions(allow: &[String]) -> Permissions {
    if allow.is_empty() {
        return Permissions::all();
    }
    let mut perms = Permissions::empty();
    for action in allow {
        match action.trim().to_ascii_lowercase().as_str() {
            "print" => perms |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY,
            "copy" => perms |= Permissions::COPYABLE | Permissions::COPYABLE_FOR_ACCESSIBILITY,
            "modify" => perms |= Permissions::MODIFIABLE | Permissions::ASSEMBLABLE,
            "annotate" => perms |= Permissions::ANNOTABLE | Permissions::FILLABLE,
            _ => {} // unknown action: ignore (documented)
        }
    }
    perms
}
