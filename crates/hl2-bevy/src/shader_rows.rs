//! Row buffers (`Vec4` rows in a [`ShaderBuffer`]) over Bevy's aligned byte storage: whole
//! writes that keep the allocation, and byte views for in-place updates and tests.
use bevy::{math::Vec4, render::storage::ShaderBuffer};

pub(crate) trait ShaderRows {
    /// Replaces the contents with `rows`; the allocation is reused when it is large enough.
    fn set_rows(&mut self, rows: &[Vec4]);
    /// The initialized bytes, or `None` while the buffer has no CPU data.
    fn bytes(&self) -> Option<&[u8]>;
    /// The bytes, zero-filled to `len` first when absent or of another size; `true` when they
    /// were (re)created and so must all be written.
    fn bytes_sized(&mut self, len: usize) -> (&mut [u8], bool);
}

impl ShaderRows for ShaderBuffer {
    fn set_rows(&mut self, rows: &[Vec4]) {
        self.clear();
        self.extend_from_slice(rows);
    }
    fn bytes(&self) -> Option<&[u8]> {
        self.cast_slice::<u8>()
    }
    fn bytes_sized(&mut self, len: usize) -> (&mut [u8], bool) {
        let created = self.bytes().is_none_or(|bytes| bytes.len() != len);
        if created {
            self.clear();
            self.extend(std::iter::repeat_n(Vec4::ZERO, len.div_ceil(16)));
            self.resize(len as u64);
        }
        let bytes = self
            .cast_slice_mut::<u8>()
            .expect("an initialized row buffer has bytes");
        (bytes, created)
    }
}

#[cfg(test)]
mod tests {
    use super::ShaderRows;
    use bevy::{math::Vec4, render::storage::ShaderBuffer};

    #[test]
    fn set_rows_replaces_contents_and_keeps_the_allocation() {
        let mut buffer = ShaderBuffer::default();
        assert!(buffer.bytes().is_none());
        buffer.set_rows(&[Vec4::ONE; 4]);
        assert_eq!(buffer.bytes().map(<[u8]>::len), Some(64));
        assert_eq!(buffer.buffer_size(), 64);
        let first = buffer.bytes().unwrap().as_ptr();
        buffer.set_rows(&[Vec4::splat(2.); 4]);
        assert_eq!(buffer.bytes().unwrap().as_ptr(), first);
        assert_eq!(&buffer.bytes().unwrap()[..4], 2f32.to_le_bytes());
    }

    #[test]
    fn bytes_sized_zero_fills_once_then_reuses() {
        let mut buffer = ShaderBuffer::default();
        let (bytes, created) = buffer.bytes_sized(32);
        assert!(created);
        assert_eq!(bytes, [0; 32]);
        bytes[0] = 7;
        let (bytes, created) = buffer.bytes_sized(32);
        assert!(!created);
        assert_eq!(bytes[0], 7);
        assert_eq!(buffer.buffer_size(), 32);
    }
}
