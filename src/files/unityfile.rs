#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub offset: i64,
    pub size: i64,
    pub flags: u32,
    pub path: String,
}
impl FileEntry {
    pub const FLAG_SERIALIZEDFILE: u32 = 4;
    pub fn end(&self) -> i64 {
        self.offset + self.size
    }
}

impl FileEntry {
    // pub fn new(offset: i64, size: i64, flags: u32, path: String) -> Self {
    //     Self {
    //         offset,
    //         size,
    //         flags,
    //         path,
    //     }
    // }

    pub fn get_offset(&self) -> i64 {
        self.offset
    }

    pub fn get_size(&self) -> i64 {
        self.size
    }

    pub fn get_flags(&self) -> u32 {
        self.flags
    }

    pub fn get_path(&self) -> &String {
        &self.path
    }
}
