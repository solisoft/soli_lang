//! Just enough Mach-O to make an appended payload survive code signing.
//!
//! Appending bytes to a Mach-O the naive way puts them *past* `__LINKEDIT` and
//! past the existing code signature. `codesign` then refuses the file with
//! "main executable failed strict validation", and signing tools that do accept
//! it (rcodesign) rewrite the binary and silently drop the trailing bytes — the
//! payload disappears. Either way the artifact is dead on Apple Silicon, which
//! SIGKILLs binaries whose signature does not validate.
//!
//! The fix is to make the payload part of the binary rather than debris after
//! it: drop the inherited signature, append, then grow `__LINKEDIT` to cover
//! what we appended. `codesign` then sees a well-formed Mach-O with no trailing
//! data, and writes its signature at the end where the format requires it.
//!
//! That moves the footer off EOF, so `find_footer_anchor` exists for the boot
//! side: on a signed artifact the payload ends where the signature begins.

const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM_64: u32 = 0xcffa_edfe;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;

/// Page size `__LINKEDIT`'s vmsize is rounded to. 16 KiB covers arm64; x86_64
/// uses 4 KiB, and rounding up to the larger of the two is valid for both.
const PAGE: u64 = 0x4000;

fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(off..off + 4)?.try_into().ok()?,
    ))
}

fn u64_at(bytes: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(off..off + 8)?.try_into().ok()?,
    ))
}

/// True for a little-endian 64-bit Mach-O. Fat/universal binaries and the
/// big-endian encoding are deliberately not handled: soli publishes thin
/// per-arch runtimes, and quietly mis-patching something else is worse than
/// leaving it alone.
pub fn is_macho64(bytes: &[u8]) -> bool {
    matches!(u32_at(bytes, 0), Some(MH_MAGIC_64))
}

/// A Mach-O we know enough about to patch.
struct Layout {
    /// Offset of the `__LINKEDIT` LC_SEGMENT_64 load command.
    linkedit_cmd: usize,
    linkedit_fileoff: u64,
    linkedit_filesize: u64,
    /// Offset of the LC_CODE_SIGNATURE load command, plus the blob it points at.
    sig: Option<(usize, u64, u64)>,
}

fn parse(bytes: &[u8]) -> Option<Layout> {
    if u32_at(bytes, 0) == Some(MH_CIGAM_64) {
        return None; // byte-swapped: not something we should be editing
    }
    if !is_macho64(bytes) {
        return None;
    }
    let ncmds = u32_at(bytes, 16)? as usize;
    let mut off = 32usize;
    let mut linkedit = None;
    let mut sig = None;

    for _ in 0..ncmds {
        let cmd = u32_at(bytes, off)?;
        let cmdsize = u32_at(bytes, off + 4)? as usize;
        if cmdsize < 8 || off + cmdsize > bytes.len() {
            return None;
        }
        match cmd {
            LC_SEGMENT_64 => {
                let name = bytes.get(off + 8..off + 24)?;
                let name = name.split(|b| *b == 0).next().unwrap_or(&[]);
                if name == b"__LINKEDIT" {
                    linkedit = Some((off, u64_at(bytes, off + 40)?, u64_at(bytes, off + 48)?));
                }
            }
            LC_CODE_SIGNATURE => {
                sig = Some((
                    off,
                    u32_at(bytes, off + 8)? as u64,
                    u32_at(bytes, off + 12)? as u64,
                ));
            }
            _ => {}
        }
        off += cmdsize;
    }

    let (linkedit_cmd, linkedit_fileoff, linkedit_filesize) = linkedit?;
    Some(Layout {
        linkedit_cmd,
        linkedit_fileoff,
        linkedit_filesize,
        sig,
    })
}

/// Where a signed artifact's payload footer ends.
///
/// Returns the offset of the code signature blob, which is exactly where the
/// appended region stops. `None` means the file carries no signature and the
/// footer is still at EOF.
pub fn find_footer_anchor(bytes: &[u8]) -> Option<u64> {
    parse(bytes)?.sig.map(|(_, dataoff, _)| dataoff)
}

/// Strip the inherited code signature so the runtime template can be appended
/// to. Returns the truncated image, or the input unchanged when there is no
/// signature to remove.
///
/// The LC_CODE_SIGNATURE load command is removed by shifting the commands after
/// it left and zeroing the tail of the load-command region. That keeps the size
/// of that region — and therefore every file offset in the binary — identical,
/// which is what makes this safe to do without relocating anything.
pub fn strip_signature(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let layout = match parse(bytes) {
        Some(l) => l,
        None => return Ok(bytes.to_vec()),
    };
    let (cmd_off, dataoff, _datasize) = match layout.sig {
        Some(s) => s,
        None => return Ok(bytes.to_vec()),
    };

    let cmdsize = u32_at(bytes, cmd_off + 4).ok_or("unreadable LC_CODE_SIGNATURE")? as usize;
    let ncmds = u32_at(bytes, 16).ok_or("unreadable mach header")?;
    let sizeofcmds = u32_at(bytes, 20).ok_or("unreadable mach header")? as usize;

    if dataoff < layout.linkedit_fileoff || dataoff as usize > bytes.len() {
        return Err("code signature lies outside __LINKEDIT".to_string());
    }

    // Drop the signature blob itself.
    let mut out = bytes[..dataoff as usize].to_vec();

    // Remove the load command, preserving the region's total size.
    let cmds_end = 32 + sizeofcmds;
    if cmd_off + cmdsize > cmds_end || cmds_end > out.len() {
        return Err("load commands are inconsistent".to_string());
    }
    out.copy_within(cmd_off + cmdsize..cmds_end, cmd_off);
    for byte in &mut out[cmds_end - cmdsize..cmds_end] {
        *byte = 0;
    }
    out[16..20].copy_from_slice(&(ncmds - 1).to_le_bytes());
    out[20..24].copy_from_slice(&((sizeofcmds - cmdsize) as u32).to_le_bytes());

    // __LINKEDIT ended at the signature; it now ends where the file does.
    let new_filesize = dataoff - layout.linkedit_fileoff;
    write_linkedit_size(&mut out, layout.linkedit_cmd, new_filesize);
    Ok(out)
}

/// Grow `__LINKEDIT` so it covers `extra` bytes appended at the end of `bytes`.
///
/// Without this the appended payload sits outside every segment, which is the
/// exact condition `codesign` rejects.
pub fn extend_linkedit(bytes: &mut [u8], extra: u64) -> Result<(), String> {
    let layout = parse(bytes).ok_or("not a patchable mach-o")?;
    let new_filesize = layout
        .linkedit_filesize
        .checked_add(extra)
        .ok_or("__LINKEDIT size overflow")?;
    write_linkedit_size(bytes, layout.linkedit_cmd, new_filesize);
    Ok(())
}

fn write_linkedit_size(bytes: &mut [u8], cmd_off: usize, filesize: u64) {
    // vmsize must cover filesize and stay page-aligned or dyld rejects the load.
    let vmsize = filesize.div_ceil(PAGE) * PAGE;
    bytes[cmd_off + 32..cmd_off + 40].copy_from_slice(&vmsize.to_le_bytes());
    bytes[cmd_off + 48..cmd_off + 56].copy_from_slice(&filesize.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mach-O skeleton: header, one __LINKEDIT segment, one LC_CODE_SIGNATURE,
    /// then linkedit data ending with a "signature" blob.
    fn fixture() -> Vec<u8> {
        let seg_cmd = 72usize; // LC_SEGMENT_64 with no sections
        let sig_cmd = 16usize;
        let sizeofcmds = seg_cmd + sig_cmd;
        let linkedit_fileoff = 512u64;
        let linkedit_filesize = 256u64; // includes the 64-byte signature
        let sig_dataoff = linkedit_fileoff + linkedit_filesize - 64;

        let mut out = vec![0u8; 32];
        out[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        out[16..20].copy_from_slice(&2u32.to_le_bytes()); // ncmds
        out[20..24].copy_from_slice(&(sizeofcmds as u32).to_le_bytes());

        let mut seg = vec![0u8; seg_cmd];
        seg[0..4].copy_from_slice(&LC_SEGMENT_64.to_le_bytes());
        seg[4..8].copy_from_slice(&(seg_cmd as u32).to_le_bytes());
        seg[8..18].copy_from_slice(b"__LINKEDIT");
        seg[32..40].copy_from_slice(&PAGE.to_le_bytes()); // vmsize
        seg[40..48].copy_from_slice(&linkedit_fileoff.to_le_bytes());
        seg[48..56].copy_from_slice(&linkedit_filesize.to_le_bytes());
        out.extend_from_slice(&seg);

        let mut sig = vec![0u8; sig_cmd];
        sig[0..4].copy_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
        sig[4..8].copy_from_slice(&(sig_cmd as u32).to_le_bytes());
        sig[8..12].copy_from_slice(&(sig_dataoff as u32).to_le_bytes());
        sig[12..16].copy_from_slice(&64u32.to_le_bytes());
        out.extend_from_slice(&sig);

        out.resize((linkedit_fileoff + linkedit_filesize) as usize, 0xAB);
        out
    }

    #[test]
    fn finds_the_signature_anchor() {
        assert_eq!(find_footer_anchor(&fixture()), Some(512 + 256 - 64));
    }

    #[test]
    fn a_non_macho_has_no_anchor() {
        assert!(find_footer_anchor(b"\x7fELF and then some padding bytes").is_none());
    }

    #[test]
    fn stripping_removes_the_blob_the_command_and_shrinks_linkedit() {
        let stripped = strip_signature(&fixture()).unwrap();
        // The signature blob is gone from the tail...
        assert_eq!(stripped.len() as u64, 512 + 256 - 64);
        // ...along with its load command.
        assert!(find_footer_anchor(&stripped).is_none());
        assert_eq!(u32_at(&stripped, 16), Some(1)); // ncmds
                                                    // __LINKEDIT now ends with the file.
        let layout = parse(&stripped).unwrap();
        assert_eq!(
            layout.linkedit_fileoff + layout.linkedit_filesize,
            stripped.len() as u64
        );
    }

    #[test]
    fn stripping_preserves_every_file_offset() {
        // The load-command region keeps its size, so __LINKEDIT stays put —
        // this is what lets us edit without relocating section data.
        let before = parse(&fixture()).unwrap().linkedit_fileoff;
        let after = parse(&strip_signature(&fixture()).unwrap())
            .unwrap()
            .linkedit_fileoff;
        assert_eq!(before, after);
    }

    #[test]
    fn extending_covers_the_appended_bytes() {
        let mut img = strip_signature(&fixture()).unwrap();
        let before = img.len() as u64;
        img.extend_from_slice(&[0u8; 100]);
        extend_linkedit(&mut img, 100).unwrap();
        let layout = parse(&img).unwrap();
        assert_eq!(
            layout.linkedit_fileoff + layout.linkedit_filesize,
            before + 100,
            "__LINKEDIT must end exactly at EOF, leaving no trailing data"
        );
    }

    #[test]
    fn stripping_is_a_no_op_without_a_signature() {
        let unsigned = strip_signature(&fixture()).unwrap();
        assert_eq!(strip_signature(&unsigned).unwrap(), unsigned);
    }

    #[test]
    fn non_macho_input_passes_through_untouched() {
        let elf = b"\x7fELF plus a payload".to_vec();
        assert_eq!(strip_signature(&elf).unwrap(), elf);
    }
}
