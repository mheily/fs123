//! Common types for fs123 protocol responses.

use crate::netstring;

/// Directory entry type constants (from dirent.h).
pub mod d_type {
    pub const DT_UNKNOWN: u8 = 0;
    pub const DT_FIFO: u8 = 1;
    pub const DT_CHR: u8 = 2;
    pub const DT_DIR: u8 = 4;
    pub const DT_BLK: u8 = 6;
    pub const DT_REG: u8 = 8;
    pub const DT_LNK: u8 = 10;
    pub const DT_SOCK: u8 = 12;
    pub const DT_WHT: u8 = 14;
}

/// Result of a stat operation.
#[derive(Debug, Clone, Default)]
pub struct Fs123StatResult {
    pub st_mode: u32,
    pub st_nlink: u64,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: i64,
    pub st_mtime: i64,
    pub st_ctime: i64,
    pub st_atime: i64,
    pub st_ino: u64,
    pub st_mtime_nsec: i64,
    pub st_ctime_nsec: i64,
    pub st_atime_nsec: i64,
    pub st_dev: u64,
    pub st_blocks: i64,
    pub st_blksize: i64,
    pub st_rdev: u64,
}

impl Fs123StatResult {
    /// Parse stat data from a space-separated string.
    ///
    /// Format: mode nlink uid gid size mtime ctime atime ino
    ///         mtime_nsec ctime_nsec atime_nsec dev blocks blksize rdev
    pub fn from_str(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() < 16 {
            return None;
        }

        Some(Fs123StatResult {
            st_mode: parts[0].parse().ok()?,
            st_nlink: parts[1].parse().ok()?,
            st_uid: parts[2].parse().ok()?,
            st_gid: parts[3].parse().ok()?,
            st_size: parts[4].parse().ok()?,
            st_mtime: parts[5].parse().ok()?,
            st_ctime: parts[6].parse().ok()?,
            st_atime: parts[7].parse().ok()?,
            st_ino: parts[8].parse().ok()?,
            st_mtime_nsec: parts[9].parse().ok()?,
            st_ctime_nsec: parts[10].parse().ok()?,
            st_atime_nsec: parts[11].parse().ok()?,
            st_dev: parts[12].parse().ok()?,
            st_blocks: parts[13].parse().ok()?,
            st_blksize: parts[14].parse().ok()?,
            st_rdev: parts[15].parse().ok()?,
        })
    }

    /// Check if this is a directory.
    pub fn is_dir(&self) -> bool {
        (self.st_mode & libc::S_IFMT as u32) == libc::S_IFDIR as u32
    }

    /// Check if this is a regular file.
    pub fn is_file(&self) -> bool {
        (self.st_mode & libc::S_IFMT as u32) == libc::S_IFREG as u32
    }

    /// Check if this is a symlink.
    pub fn is_symlink(&self) -> bool {
        (self.st_mode & libc::S_IFMT as u32) == libc::S_IFLNK as u32
    }
}

/// Result of a statvfs operation.
#[derive(Debug, Clone, Default)]
pub struct Fs123StatvfsResult {
    pub f_bsize: u64,
    pub f_frsize: u64,
    pub f_blocks: u64,
    pub f_bfree: u64,
    pub f_bavail: u64,
    pub f_files: u64,
    pub f_ffree: u64,
    pub f_favail: u64,
    pub f_fsid: u64,
    pub f_flag: u64,
    pub f_namemax: u64,
}

impl Fs123StatvfsResult {
    /// Parse statvfs data from a space-separated string.
    ///
    /// Format: f_bsize f_frsize f_blocks f_bfree f_bavail f_files f_ffree f_favail f_fsid f_flag f_namemax
    pub fn from_str(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() < 11 {
            return None;
        }

        Some(Fs123StatvfsResult {
            f_bsize: parts[0].parse().ok()?,
            f_frsize: parts[1].parse().ok()?,
            f_blocks: parts[2].parse().ok()?,
            f_bfree: parts[3].parse().ok()?,
            f_bavail: parts[4].parse().ok()?,
            f_files: parts[5].parse().ok()?,
            f_ffree: parts[6].parse().ok()?,
            f_favail: parts[7].parse().ok()?,
            f_fsid: parts[8].parse().ok()?,
            f_flag: parts[9].parse().ok()?,
            f_namemax: parts[10].parse().ok()?,
        })
    }
}

/// A directory entry.
#[derive(Debug, Clone)]
pub struct DirEntryData {
    /// Entry name
    pub name: String,
    /// Directory entry type (DT_* constant)
    pub d_type: u8,
    /// ESTALE cookie
    pub estalecookie: u64,
}

impl DirEntryData {
    /// Parse directory entries from /d response content.
    ///
    /// Format per line: `<name_netstring> <d_type_decimal> <estalecookie_decimal>\n`
    pub fn parse_entries(content: &[u8]) -> Vec<DirEntryData> {
        let mut entries = Vec::new();
        let mut offset = 0;

        while offset < content.len() {
            // Skip any leading whitespace
            while offset < content.len() && content[offset] == b' ' {
                offset += 1;
            }

            if offset >= content.len() {
                break;
            }

            // Decode name netstring
            let Some((name_bytes, consumed)) = netstring::decode(&content[offset..]) else {
                break;
            };
            offset += consumed;

            let name = match std::str::from_utf8(name_bytes) {
                Ok(s) => s.to_string(),
                Err(_) => break,
            };

            // Skip space
            while offset < content.len() && content[offset] == b' ' {
                offset += 1;
            }

            // Find end of line
            let line_end = content[offset..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| offset + p)
                .unwrap_or(content.len());

            // Parse d_type and estalecookie from remaining line content
            let line_content = &content[offset..line_end];
            let line_str = match std::str::from_utf8(line_content) {
                Ok(s) => s,
                Err(_) => break,
            };

            let parts: Vec<&str> = line_str.split_whitespace().collect();
            if parts.len() < 2 {
                break;
            }

            let d_type: u8 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => break,
            };

            let estalecookie: u64 = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => break,
            };

            entries.push(DirEntryData {
                name,
                d_type,
                estalecookie,
            });

            // Move past the newline
            offset = line_end + 1;
        }

        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stat_result_from_str() {
        let stat_str = "33188 1 1000 1000 1024 1700000000 1700000001 1700000002 12345 0 0 0 0 8 4096 0";
        let stat = Fs123StatResult::from_str(stat_str).unwrap();

        assert_eq!(stat.st_mode, 33188); // 0o100644
        assert_eq!(stat.st_nlink, 1);
        assert_eq!(stat.st_uid, 1000);
        assert_eq!(stat.st_gid, 1000);
        assert_eq!(stat.st_size, 1024);
        assert_eq!(stat.st_ino, 12345);
    }

    #[test]
    fn test_statvfs_result_from_str() {
        let statvfs_str = "4096 4096 1000000 500000 400000 100000 50000 40000 12345 1 255";
        let statvfs = Fs123StatvfsResult::from_str(statvfs_str).unwrap();

        assert_eq!(statvfs.f_bsize, 4096);
        assert_eq!(statvfs.f_blocks, 1000000);
        assert_eq!(statvfs.f_namemax, 255);
    }

    #[test]
    fn test_dir_entry_parse() {
        // Format: <name_netstring> <d_type> <estalecookie>\n
        let content = b"5:hello, 8 12345\n3:dir, 4 67890\n";
        let entries = DirEntryData::parse_entries(content);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "hello");
        assert_eq!(entries[0].d_type, 8); // DT_REG
        assert_eq!(entries[0].estalecookie, 12345);
        assert_eq!(entries[1].name, "dir");
        assert_eq!(entries[1].d_type, 4); // DT_DIR
    }
}
