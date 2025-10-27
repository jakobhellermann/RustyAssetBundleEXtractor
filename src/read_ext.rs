use byteorder::{ByteOrder, ReadBytesExt};
use std::io::Seek;

const MAX_READ_BYTES_LEN: usize = 1024 * 1024 * 1024 * 1; // 1GiB

macro_rules! generate_read_array_method {
    ($name:ident $read_into_name:ident $typ:ty) => {
        // #[doc = "Reads an array of [`" $typ "`]s."]
        fn $name<B: ByteOrder>(&mut self) -> Result<Vec<$typ>, std::io::Error> {
            let len = self.read_array_len::<B>()?;
            if len * std::mem::size_of::<$typ>() > MAX_READ_BYTES_LEN {
                return Err(error_oom(len));
            }
            let mut buf = vec![0; len];
            self.$read_into_name::<B>(&mut buf)?;
            Ok(buf)
        }
    };
}

pub trait ReadUrexExt: ReadBytesExt {
    fn read_array_len<T: ByteOrder>(&mut self) -> Result<usize, std::io::Error> {
        let len = self.read_u32::<T>()?;
        Ok(len as usize)
    }

    fn read_cstr(&mut self) -> Result<String, std::io::Error> {
        let mut bytes = Vec::new();
        loop {
            let byte = self.read_u8()?;
            if byte == 0 {
                break;
            }
            bytes.push(byte);
        }

        match String::from_utf8(bytes) {
            Ok(s) => Ok(s),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        }
    }

    fn read_string<T: ByteOrder>(&mut self) -> Result<String, std::io::Error> {
        let len = self.read_array_len::<T>()?;
        self.read_string_sized(len)
    }

    fn read_string_sized(&mut self, len: usize) -> Result<String, std::io::Error> {
        match String::from_utf8(self.read_bytes_sized(len)?) {
            Ok(s) => Ok(s),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        }
    }

    fn read_bytes<T: ByteOrder>(&mut self) -> Result<Vec<u8>, std::io::Error> {
        let len = self.read_array_len::<T>()?;
        self.read_bytes_sized(len)
    }

    fn read_bytes_sized(&mut self, len: usize) -> Result<Vec<u8>, std::io::Error> {
        if len > MAX_READ_BYTES_LEN {
            return Err(error_oom(len));
        }
        let mut buf = vec![0; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_bytes_array<const L: usize>(&mut self) -> Result<[u8; L], std::io::Error> {
        let mut buf = [0; L];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn read_bool(&mut self) -> Result<bool, std::io::Error> {
        let b = self.read_u8()?;
        Ok(b != 0)
    }

    fn read_char(&mut self) -> Result<char, std::io::Error> {
        let c = self.read_u8()? as char;
        Ok(c)
    }

    generate_read_array_method!(read_i32_array read_i32_into i32);
}

pub trait ReadSeekUrexExt: ReadUrexExt + Seek {
    fn align(&mut self, align: usize) -> Result<(), std::io::Error> {
        let pos = self.stream_position()?;
        let new_pos = (pos + align as u64 - 1) & !(align as u64 - 1);
        let diff = new_pos - pos;
        if diff > 0 {
            self.seek(std::io::SeekFrom::Current(diff as i64))?;
        }
        Ok(())
    }

    fn align4(&mut self) -> Result<(), std::io::Error> {
        self.align(4)
    }
}

impl<R: std::io::Read + ?Sized> ReadUrexExt for R {}
impl<R: std::io::Read + Seek + ?Sized> ReadSeekUrexExt for R {}

fn error_oom(len: usize) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        format!(
            "Attempted to make an allocation of {len} bytes, exceeding the threshold {}",
            MAX_READ_BYTES_LEN
        ),
    )
}

pub(crate) fn invalid_data<E>(error: E) -> std::io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}
