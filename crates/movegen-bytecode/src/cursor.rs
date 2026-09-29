//! A bounded, strict byte cursor.

use crate::error::Error;

/// Reads from a byte slice; every read is bounds-checked.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    /// The bytes from `start` to the current position.
    pub(crate) fn slice_from(&self, start: usize) -> &'a [u8] {
        &self.bytes[start..self.pos]
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8, Error> {
        let byte = *self.bytes.get(self.pos).ok_or(Error::Eof(what))?;
        self.pos += 1;
        Ok(byte)
    }

    pub(crate) fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(len).ok_or(Error::Eof(what))?;
        let slice = self.bytes.get(self.pos..end).ok_or(Error::Eof(what))?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], Error> {
        let slice = self.take(N, what)?;
        Ok(slice.try_into().expect("take returned N bytes"))
    }

    pub(crate) fn u32_le(&mut self, what: &'static str) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    /// Canonical ULEB128 (B-ULEB-1), mirroring `read_uleb128_as_u64`: rejects
    /// overflow and a final zero group after the first byte.
    pub(crate) fn uleb(&mut self, what: &'static str) -> Result<u64, Error> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.u8(what)?;
            let cur = u64::from(byte & 0x7f);
            if shift >= 64 || (cur << shift) >> shift != cur {
                return Err(Error::BadUleb(what));
            }
            value |= cur << shift;
            if byte & 0x80 == 0 {
                if shift > 0 && cur == 0 {
                    return Err(Error::BadUleb(what));
                }
                return Ok(value);
            }
            shift += 7;
        }
    }

    /// A ULEB bounded by `max` (B-ULEB-1).
    pub(crate) fn uleb_max(&mut self, what: &'static str, max: u64) -> Result<u64, Error> {
        let value = self.uleb(what)?;
        if value > max {
            return Err(Error::TooLarge { what, value, max });
        }
        Ok(value)
    }

    /// A table index (≤ 65535).
    pub(crate) fn index(&mut self, what: &'static str) -> Result<u16, Error> {
        // Bounded by TABLE_INDEX_MAX, so the cast is lossless.
        #[allow(clippy::cast_possible_truncation)]
        Ok(self.uleb_max(what, u64::from(u16::MAX))? as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uleb(bytes: &[u8]) -> Result<u64, Error> {
        Cursor::new(bytes).uleb("test")
    }

    // spec: B-ULEB-1
    #[test]
    fn uleb_canonical() {
        assert_eq!(uleb(&[0x00]), Ok(0));
        assert_eq!(uleb(&[0x7f]), Ok(127));
        assert_eq!(uleb(&[0x80, 0x01]), Ok(128));
        assert_eq!(uleb(&[0xff, 0xff, 0x03]), Ok(65535));
        assert_eq!(
            uleb(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]),
            Ok(u64::MAX)
        );
        // Redundant zero group, overflow, truncation.
        assert_eq!(uleb(&[0x80, 0x00]), Err(Error::BadUleb("test")));
        assert_eq!(
            uleb(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]),
            Err(Error::BadUleb("test"))
        );
        assert_eq!(uleb(&[0x80]), Err(Error::Eof("test")));
        assert_eq!(
            Cursor::new(&[0x80, 0x80, 0x04]).uleb_max("test", 65535),
            Err(Error::TooLarge {
                what: "test",
                value: 65536,
                max: 65535
            })
        );
    }
}
