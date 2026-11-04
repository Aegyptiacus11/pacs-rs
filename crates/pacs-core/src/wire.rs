//! A walk over the bytes of a Part 10 file before dicom-rs reads it, for two things dicom-rs does
//! not do.
//!
//! UN (PS3.5 6.2.2): the value of an element written as UN "is encoded in Little Endian byte
//! ordering with Implicit VR encoding, irrespective of the current Transfer Syntax". dicom-rs
//! reads an undefined-length UN as a sequence in the file's own encoding, so implicit content
//! comes apart, and keeps a standard element in UN as bytes, so a UID in UN is not a string.
//! [`checked`] gives such a file back with those elements under their VR. As in DCMTK
//! (`DcmItem::newDicomElement`) and dcm4che (`DicomInputStream`), a defined-length UN is reverted
//! by the dictionary, and only for a standard tag; undefined length means a sequence; and, as in
//! dcm4che (`probeExplicitVR`), a sequence in UN whose first item looks explicit is read so.
//!
//! Lengths: dicom-rs allocates and zeroes the length an element declares before reading any of
//! it, so one wrong length costs up to 4 GiB, however small the file. [`checked`] refuses a value
//! longer than what is left, as dcm4che's `testNoOutOfMemoryErrorOnInvalidLength` asks of it.
//!
//! Apart from nesting too deep, the walk refuses only what dicom-rs would refuse too: it follows
//! dicom-rs's reading of each header. A file with no UN to revert comes back as the same bytes.

use std::borrow::Cow;

use dicom_core::dictionary::{DataDictionary, DataDictionaryEntry, VirtualVr};
use dicom_core::{Tag, VR};
use dicom_dictionary_std::{StandardDataDictionary, uids};

use crate::Error;

const UNDEFINED: u32 = u32::MAX;
const ITEM: Tag = Tag(0xFFFE, 0xE000);
const ITEM_END: Tag = Tag(0xFFFE, 0xE00D);
const SEQUENCE_END: Tag = Tag(0xFFFE, 0xE0DD);
const PIXEL_DATA: Tag = Tag(0x7FE0, 0x0010);
const PIXEL_REPRESENTATION: Tag = Tag(0x0028, 0x0103);
const TRANSFER_SYNTAX: Tag = Tag(0x0002, 0x0010);
const EXPLICIT_VR_BIG_ENDIAN: &str = "1.2.840.10008.1.2.2"; // retired, still legal to receive

/// Sequences nested deeper are refused. PS3.5 sets no limit, but dicom-rs reads them by
/// recursion, so some megabytes of nested items would overflow the stack and end the process.
const MAX_DEPTH: usize = 64;

/// The bytes to give dicom-rs for this Part 10 file: the file itself or, when it has UN elements
/// to revert, a copy with them under their VR, in which every sequence and item has undefined
/// length (a reverted element changes the size of what holds it). Refuses a value longer than
/// the file and nesting beyond [`MAX_DEPTH`]. What is not a Part 10 file passes, for dicom-rs to
/// refuse.
pub fn checked(bytes: &[u8]) -> Result<Cow<'_, [u8]>, Error> {
    let Some((start, encoding)) = data_set_start(bytes)? else {
        return Ok(Cow::Borrowed(bytes));
    };
    let mut scan = Walk::new(bytes, encoding.big, None);
    scan.data_set(start, Some(bytes.len()), encoding, 0)
        .map_err(Error::Malformed)?;
    if !scan.rewrite {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 8);
    out.extend_from_slice(&bytes[..start]);
    let mut copy = Walk::new(bytes, encoding.big, Some(out));
    copy.data_set(start, Some(bytes.len()), encoding, 0)
        .map_err(Error::Malformed)?;
    Ok(Cow::Owned(copy.out.unwrap_or_default()))
}

#[derive(Debug, Clone, Copy)]
struct Encoding {
    explicit: bool,
    big: bool,
}

const IMPLICIT_LE: Encoding = Encoding {
    explicit: false,
    big: false,
};
const EXPLICIT_LE: Encoding = Encoding {
    explicit: true,
    big: false,
};

/// Where the data set starts and how it is encoded. `None` when it is not to be walked: not a
/// Part 10 file, a File Meta group dicom-rs will refuse, or a deflated data set, which this build
/// of dicom-rs refuses too.
fn data_set_start(bytes: &[u8]) -> Result<Option<(usize, Encoding)>, Error> {
    let start = if bytes.len() >= 132 && &bytes[128..132] == b"DICM" {
        132
    } else if bytes.len() >= 4 && &bytes[..4] == b"DICM" {
        4
    } else {
        return Ok(None);
    };
    // Group 2 is always Explicit VR Little Endian (PS3.10 7.1).
    let walk = Walk::new(bytes, false, None);
    let Ok(group_length) = walk.header(start, EXPLICIT_LE) else {
        return Ok(None);
    };
    let mut pos = start + group_length.size + 4;
    if group_length.tag != Tag(0x0002, 0x0000) || group_length.len != 4 || bytes.len() < pos {
        return Ok(None);
    }
    let meta_end = pos.saturating_add(walk.u32_at(pos - 4, false) as usize);
    let mut transfer_syntax = None;
    while pos < meta_end {
        let Ok(header) = walk.header(pos, EXPLICIT_LE) else {
            return Ok(None);
        };
        pos += header.size;
        if header.len == UNDEFINED {
            return Ok(None);
        }
        let value = walk
            .value(header.tag, pos, header.len, bytes.len())
            .map_err(Error::Malformed)?;
        if header.tag == TRANSFER_SYNTAX {
            transfer_syntax = Some(value);
        }
        pos += value.len();
    }
    let Some(transfer_syntax) = transfer_syntax else {
        return Ok(None);
    };
    let encoding = match String::from_utf8_lossy(transfer_syntax).trim_end_matches(['\0', ' ']) {
        uids::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN
        | uids::JPIP_REFERENCED_DEFLATE
        | uids::JPIPHTJ2K_REFERENCED_DEFLATE => return Ok(None),
        uids::IMPLICIT_VR_LITTLE_ENDIAN => IMPLICIT_LE,
        EXPLICIT_VR_BIG_ENDIAN => Encoding {
            explicit: true,
            big: true,
        },
        _ => EXPLICIT_LE,
    };
    Ok(Some((pos, encoding)))
}

struct Header {
    tag: Tag,
    /// As written; `None` in implicit VR and for items and delimiters. Unknown is UN, as in dicom-rs.
    vr: Option<VR>,
    len: u32,
    size: usize,
}

struct Walk<'a> {
    bytes: &'a [u8],
    /// The byte order of the file's data set, which the copy keeps.
    big: bool,
    /// The copy, on the second pass.
    out: Option<Vec<u8>>,
    /// Whether dicom-rs has to read a copy instead of the file.
    rewrite: bool,
    /// The last Pixel Representation: whether a "US or SS" element is SS.
    signed: bool,
}

impl<'a> Walk<'a> {
    fn new(bytes: &'a [u8], big: bool, out: Option<Vec<u8>>) -> Self {
        Walk {
            bytes,
            big,
            out,
            rewrite: false,
            signed: false,
        }
    }

    fn u16_at(&self, pos: usize, big: bool) -> u16 {
        let raw = [self.bytes[pos], self.bytes[pos + 1]];
        if big {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        }
    }

    fn u32_at(&self, pos: usize, big: bool) -> u32 {
        let raw = [
            self.bytes[pos],
            self.bytes[pos + 1],
            self.bytes[pos + 2],
            self.bytes[pos + 3],
        ];
        if big {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        }
    }

    fn tag_at(&self, pos: usize, big: bool) -> Tag {
        Tag(self.u16_at(pos, big), self.u16_at(pos + 2, big))
    }

    /// The element header at `pos`, read as dicom-rs reads it.
    fn header(&self, pos: usize, encoding: Encoding) -> Result<Header, String> {
        let big = encoding.big;
        let left = self.bytes.len().saturating_sub(pos);
        if left < 8 {
            return Err(format!("the element header at byte {pos} is cut short"));
        }
        let tag = self.tag_at(pos, big);
        if !encoding.explicit || tag.group() == 0xFFFE {
            return Ok(Header {
                tag,
                vr: None,
                len: self.u32_at(pos + 4, big),
                size: 8,
            });
        }
        let vr = VR::from_binary([self.bytes[pos + 4], self.bytes[pos + 5]]).unwrap_or(VR::UN);
        if has_short_length(vr) {
            return Ok(Header {
                tag,
                vr: Some(vr),
                len: u32::from(self.u16_at(pos + 6, big)),
                size: 8,
            });
        }
        if left < 12 {
            return Err(format!(
                "the header of element {tag} at byte {pos} is cut short"
            ));
        }
        Ok(Header {
            tag,
            vr: Some(vr),
            len: self.u32_at(pos + 8, big),
            size: 12,
        })
    }

    /// The value of `len` bytes at `pos`, which must end by `end`.
    fn value(&self, tag: Tag, pos: usize, len: u32, end: usize) -> Result<&'a [u8], String> {
        let value_end = pos.saturating_add(len as usize);
        if value_end > end {
            return Err(format!(
                "element {tag} at byte {pos} declares a length of {len} with {} bytes left",
                end.saturating_sub(pos)
            ));
        }
        Ok(&self.bytes[pos..value_end])
    }

    /// The elements from `pos` to `end`, or, without one, to an item delimiter. The position
    /// after them.
    fn data_set(
        &mut self,
        mut pos: usize,
        end: Option<usize>,
        encoding: Encoding,
        depth: usize,
    ) -> Result<usize, String> {
        let limit = end.unwrap_or(self.bytes.len());
        while pos < limit {
            if self.bytes.len() - pos < 4 {
                if depth == 0 {
                    return Ok(self.bytes.len()); // dicom-rs: the end of the file, cleanly.
                }
                break;
            }
            let header = self.header(pos, encoding)?;
            pos += header.size;
            if header.tag == ITEM_END {
                if depth > 0 {
                    return Ok(pos);
                }
                continue; // dicom-rs ignores one outside a sequence.
            }
            pos = self.element(header, pos, limit, encoding, depth)?;
        }
        if end.is_none() || pos < limit {
            return Err(format!("an item at depth {depth} never ends"));
        }
        Ok(pos)
    }

    /// The element whose value starts at `pos`, which must end by `end`.
    fn element(
        &mut self,
        header: Header,
        pos: usize,
        end: usize,
        encoding: Encoding,
        depth: usize,
    ) -> Result<usize, String> {
        let Header { tag, vr, len, .. } = header;
        let written_un = vr == Some(VR::UN);
        if len == UNDEFINED {
            if tag == PIXEL_DATA {
                return self.fragments(pos, encoding);
            }
            let content = if written_un {
                self.rewrite = true;
                self.content_of_un(pos, encoding)
            } else {
                encoding
            };
            return self.sequence(tag, pos, None, content, depth + 1);
        }
        let value = self.value(tag, pos, len, end)?;
        let after = pos + value.len();
        // A value written as UN or in implicit VR is little endian, whatever the file's order.
        let little_endian = written_un || !encoding.big;
        if tag == PIXEL_REPRESENTATION && value.len() == 2 {
            self.signed = self.u16_at(pos, !little_endian) != 0;
        }
        let known = match vr {
            Some(VR::UN) | None => self.dictionary_vr(tag, value.len()),
            Some(vr) => Some(vr),
        };
        if written_un && known.is_some() {
            self.rewrite = true;
        }
        let vr = known.unwrap_or(VR::UN);
        if vr == VR::SQ {
            return if written_un {
                self.sequence_in_un(tag, value, pos, encoding, depth)
            } else {
                self.sequence(tag, pos, Some(after), encoding, depth + 1)
            };
        }
        // What cannot be written under its VR stays UN (PS3.5 6.2.2): a value too long for the
        // 16-bit length, or of a size the VR does not come in, which dicom-rs would misread.
        let reverted = written_un || !encoding.explicit;
        let vr = if !reverted || fits(vr, value.len()) {
            vr
        } else {
            VR::UN
        };
        self.put_header(tag, vr, len);
        if self.big && little_endian {
            self.put_big_endian(value, vr);
        } else {
            self.put(value);
        }
        Ok(after)
    }

    /// A defined-length sequence written as UN. When its content does not read as a sequence
    /// that ends where the length says, it stays what it was, bytes (dcm4che: "treat SQ as UN").
    fn sequence_in_un(
        &mut self,
        tag: Tag,
        value: &[u8],
        pos: usize,
        encoding: Encoding,
        depth: usize,
    ) -> Result<usize, String> {
        let after = pos + value.len();
        let mark = self.out.as_ref().map(Vec::len);
        let signed = self.signed;
        let content = self.content_of_un(pos, encoding);
        match self.sequence(tag, pos, Some(after), content, depth + 1) {
            Ok(end) if end == after => Ok(after),
            _ => {
                if let (Some(out), Some(mark)) = (&mut self.out, mark) {
                    out.truncate(mark);
                }
                self.signed = signed;
                self.put_header(tag, VR::UN, value.len() as u32);
                self.put(value);
                Ok(after)
            }
        }
    }

    /// How the sequence in UN at `pos` is encoded: implicit (PS3.5 6.2.2), unless the bytes after
    /// the first item's first tag are a VR. dcm4che's `probeExplicitVR`.
    fn content_of_un(&self, pos: usize, encoding: Encoding) -> Encoding {
        match self.bytes.get(pos + 12..pos + 14) {
            Some(&[a, b]) if VR::from_binary([a, b]).is_some() => encoding,
            _ => IMPLICIT_LE,
        }
    }

    /// The items of a sequence from `pos`, to `end` or, without one, to a sequence delimiter.
    fn sequence(
        &mut self,
        tag: Tag,
        mut pos: usize,
        end: Option<usize>,
        encoding: Encoding,
        depth: usize,
    ) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err(format!("sequences nested deeper than {MAX_DEPTH}"));
        }
        self.put_header(tag, VR::SQ, UNDEFINED);
        while end.is_none_or(|end| pos < end) {
            if self.bytes.len().saturating_sub(pos) < 8 {
                return Err(format!("sequence {tag} never ends"));
            }
            let item = self.tag_at(pos, encoding.big);
            let len = self.u32_at(pos + 4, encoding.big);
            pos += 8;
            if item == SEQUENCE_END {
                self.put_delimiter(SEQUENCE_END);
                return Ok(pos);
            }
            if item != ITEM {
                return Err(format!(
                    "{item} at byte {} in sequence {tag} is not an item",
                    pos - 8
                ));
            }
            self.put_item(UNDEFINED);
            let item_end = if len == UNDEFINED {
                None
            } else {
                let item_end = pos.saturating_add(len as usize);
                if end.is_some_and(|end| item_end > end) || item_end > self.bytes.len() {
                    return Err(format!(
                        "an item of sequence {tag} at byte {} declares a length of {len}, past its end",
                        pos - 8
                    ));
                }
                Some(item_end)
            };
            pos = self.data_set(pos, item_end, encoding, depth)?;
            self.put_delimiter(ITEM_END);
        }
        self.put_delimiter(SEQUENCE_END);
        Ok(pos)
    }

    /// Encapsulated pixel data: an offset table and fragments (PS3.5 A.4), whatever the VR says.
    /// dicom-rs reads an offset table in full, so one too long is refused; but it keeps what is
    /// there of a fragment cut short, so such a file stays readable, its fragment's length
    /// corrected for it to allocate no more than that.
    fn fragments(&mut self, mut pos: usize, encoding: Encoding) -> Result<usize, String> {
        self.put_header(PIXEL_DATA, VR::OB, UNDEFINED);
        let mut first = true;
        loop {
            if self.bytes.len().saturating_sub(pos) < 8 {
                return Ok(self.bytes.len()); // dicom-rs: the end of the file, cleanly.
            }
            let item = self.tag_at(pos, encoding.big);
            let len = self.u32_at(pos + 4, encoding.big);
            pos += 8;
            if item == SEQUENCE_END {
                self.put_delimiter(SEQUENCE_END);
                return Ok(pos);
            }
            if item == ITEM_END {
                self.put_delimiter(ITEM_END); // dicom-rs lets it pass; what follows, it judges.
                continue;
            }
            if item != ITEM || len == UNDEFINED {
                return Err(format!(
                    "pixel data has {item} at byte {} instead of a fragment",
                    pos - 8
                ));
            }
            let left = self.bytes.len() - pos;
            let kept = (len as usize).min(left);
            if kept < len as usize {
                if first {
                    self.value(PIXEL_DATA, pos, len, self.bytes.len())?;
                }
                self.rewrite = true;
            }
            self.put_item(kept as u32);
            let bytes = self.bytes;
            self.put(&bytes[pos..pos + kept]);
            pos += kept;
            first = false;
        }
    }

    /// The VR of a standard element by the dictionary, for one with `len` bytes of value.
    fn dictionary_vr(&self, tag: Tag, len: usize) -> Option<VR> {
        if !tag.group().is_multiple_of(2) || tag.group() == 0xFFFE {
            return None;
        }
        match StandardDataDictionary.by_tag(tag)?.vr() {
            VirtualVr::Exact(VR::UN) => None,
            VirtualVr::Exact(vr) => Some(vr),
            VirtualVr::Xs => Some(if self.signed { VR::SS } else { VR::US }),
            VirtualVr::Ox | VirtualVr::Px | VirtualVr::Lt => Some(if len.is_multiple_of(2) {
                VR::OW
            } else {
                VR::OB
            }),
            _ => None,
        }
    }

    fn put(&mut self, bytes: &[u8]) {
        if let Some(out) = &mut self.out {
            out.extend_from_slice(bytes);
        }
    }

    fn put_u16(&mut self, value: u16) {
        let raw = if self.big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        self.put(&raw);
    }

    fn put_u32(&mut self, value: u32) {
        let raw = if self.big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        };
        self.put(&raw);
    }

    fn put_tag(&mut self, tag: Tag) {
        self.put_u16(tag.group());
        self.put_u16(tag.element());
    }

    fn put_item(&mut self, len: u32) {
        self.put_tag(ITEM);
        self.put_u32(len);
    }

    fn put_delimiter(&mut self, tag: Tag) {
        self.put_tag(tag);
        self.put_u32(0);
    }

    /// An explicit VR header (PS3.5 7.1.2); items and delimiters have no VR.
    fn put_header(&mut self, tag: Tag, vr: VR, len: u32) {
        self.put_tag(tag);
        if tag.group() == 0xFFFE {
            self.put_u32(len);
            return;
        }
        self.put(&vr.to_bytes());
        if has_short_length(vr) {
            self.put_u16(len as u16);
        } else {
            self.put_u16(0);
            self.put_u32(len);
        }
    }

    /// A little-endian value of `vr` in a big-endian copy.
    fn put_big_endian(&mut self, value: &[u8], vr: VR) {
        let width = swap_width(vr);
        for unit in value.chunks(width) {
            for &byte in unit.iter().rev() {
                self.put(&[byte]);
            }
        }
    }
}

/// The VRs with a 16-bit length in explicit VR (PS3.5 7.1.2), as in dicom-rs.
fn has_short_length(vr: VR) -> bool {
    use VR::*;
    matches!(
        vr,
        AE | AS
            | AT
            | CS
            | DA
            | DS
            | DT
            | FL
            | FD
            | IS
            | LO
            | LT
            | PN
            | SH
            | SL
            | SS
            | ST
            | TM
            | UI
            | UL
            | US
    )
}

/// The size of the numbers in a value of `vr`; 1 for text and bytes.
fn swap_width(vr: VR) -> usize {
    use VR::*;
    match vr {
        AT | OW | SS | US => 2,
        FL | OF | OL | SL | UL => 4,
        FD | OD | OV | SV | UV => 8,
        _ => 1,
    }
}

/// Whether a value of `len` bytes can be written and read back under `vr`.
fn fits(vr: VR, len: usize) -> bool {
    let whole = match vr {
        VR::AT => len.is_multiple_of(4),
        _ => len.is_multiple_of(swap_width(vr)),
    };
    whole && (!has_short_length(vr) || len <= usize::from(u16::MAX))
}
