/// FileBackend - serves files from the local filesystem

use async_trait::async_trait;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use super::traits::Backend;
use super::types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo,
};

/// Backend implementation that serves files from the local filesystem
pub struct FileBackend {
    root: PathBuf,
}

impl FileBackend {
    /// Create a new FileBackend with the given root directory
    pub fn new(root: PathBuf) -> Self {
        FileBackend { root }
    }

    /// Resolve a request path to a full filesystem path
    fn resolve_path(&self, path: &str) -> PathBuf {
        // Remove leading slash from path
        let relative = path.strip_prefix('/').unwrap_or(path);
        self.root.join(relative)
    }

    /// Get validator for a file (use mtime nanoseconds as simple validator)
    fn get_validator(metadata: &fs::Metadata) -> u64 {
        let mtime = metadata.modified().unwrap_or(UNIX_EPOCH);
        mtime.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
    }

    /// Get estalecookie for a file
    /// For now, use inode number (note: not ideal, see CLAUDE.md)
    /// TODO: Use FS_IOC_GETVERSION or extended attributes
    fn get_estalecookie(metadata: &fs::Metadata) -> u64 {
        metadata.ino()
    }
}

#[async_trait]
impl Backend for FileBackend {
    async fn get_attributes(&self, path: &str) -> BackendResult<AttributeInfo> {
        let full_path = self.resolve_path(path);

        let metadata = fs::symlink_metadata(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        let file_type = metadata.file_type();
        let is_file = file_type.is_file();
        let is_dir = file_type.is_dir();

        let validator = Self::get_validator(&metadata);
        let estalecookie = if is_file || is_dir {
            Self::get_estalecookie(&metadata)
        } else {
            0
        };

        Ok(AttributeInfo {
            mode: metadata.mode(),
            nlink: metadata.nlink(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            size: metadata.size() as i64,
            mtime: metadata.mtime(),
            ctime: metadata.ctime(),
            atime: metadata.atime(),
            ino: metadata.ino(),
            mtime_nsec: metadata.mtime_nsec(),
            ctime_nsec: metadata.ctime_nsec(),
            atime_nsec: metadata.atime_nsec(),
            dev: metadata.dev(),
            blocks: metadata.blocks() as i64,
            blksize: metadata.blksize() as i64,
            rdev: metadata.rdev(),
            validator,
            estalecookie,
            is_file,
            is_dir,
        })
    }

    async fn read_file(&self, path: &str, offset: u64, length: usize) -> BackendResult<FileContent> {
        let full_path = self.resolve_path(path);

        let metadata = fs::symlink_metadata(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        if !metadata.is_file() {
            return Err(BackendError::new(libc::EINVAL, "Not a regular file"));
        }

        let file_content = fs::read(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        let start = offset as usize;
        let end = std::cmp::min(start + length, file_content.len());

        let data = if start < file_content.len() {
            file_content[start..end].to_vec()
        } else {
            vec![]
        };

        Ok(FileContent {
            data,
            validator: Self::get_validator(&metadata),
            estalecookie: Self::get_estalecookie(&metadata),
        })
    }

    async fn read_directory(
        &self,
        path: &str,
        max_bytes: usize,
        start_after: &str,
    ) -> BackendResult<DirectoryListing> {
        let full_path = self.resolve_path(path);

        let metadata = fs::symlink_metadata(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        if !metadata.is_dir() {
            return Err(BackendError::new(libc::ENOTDIR, "Not a directory"));
        }

        let read_dir = fs::read_dir(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        let mut entry_list: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
        entry_list.sort_by_key(|e| e.file_name());

        // Skip entries until we find start_after
        let mut started = start_after.is_empty();
        let mut entries = Vec::new();
        let mut last_entry_name = String::new();
        let mut current_size = 0;

        for entry in entry_list {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().to_string();

            if !started {
                if name_str == start_after {
                    started = true;
                }
                continue;
            }

            // Get d_type
            let d_type = match entry.file_type() {
                Ok(ft) => {
                    if ft.is_dir() {
                        libc::DT_DIR
                    } else if ft.is_file() {
                        libc::DT_REG
                    } else if ft.is_symlink() {
                        libc::DT_LNK
                    } else {
                        libc::DT_UNKNOWN
                    }
                }
                Err(_) => libc::DT_UNKNOWN,
            };

            // Get estalecookie (use symlink_metadata to not follow symlinks)
            let estalecookie = match entry.path().symlink_metadata() {
                Ok(meta) => Self::get_estalecookie(&meta),
                Err(_) => 0,
            };

            // Estimate size of this entry in netstring format
            // Format: <netstring_name> <d_type> <estalecookie>\n
            let entry_size = name_str.len() + 20 + d_type.to_string().len() + estalecookie.to_string().len() + 3;

            if current_size + entry_size > max_bytes {
                break;
            }

            entries.push(DirEntry {
                name: name_str.clone(),
                d_type,
                estalecookie,
            });
            current_size += entry_size;
            last_entry_name = name_str;
        }

        let nextstart = if entries.is_empty() {
            String::new()
        } else {
            last_entry_name
        };

        Ok(DirectoryListing {
            entries,
            nextstart,
            estalecookie: Self::get_estalecookie(&metadata),
        })
    }

    async fn read_symlink(&self, path: &str) -> BackendResult<String> {
        let full_path = self.resolve_path(path);

        let target = fs::read_link(&full_path).map_err(|e| BackendError::from_io_error(&e))?;

        Ok(target.to_string_lossy().to_string())
    }

    async fn statfs(&self, path: &str) -> BackendResult<StatfsInfo> {
        let full_path = self.resolve_path(path);

        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let path_cstr = CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;

        unsafe {
            let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
            let ret = libc::statvfs(path_cstr.as_ptr(), stat.as_mut_ptr());

            if ret == 0 {
                let stat = stat.assume_init();
                Ok(StatfsInfo {
                    bsize: stat.f_bsize as u64,
                    frsize: stat.f_frsize as u64,
                    blocks: stat.f_blocks as u64,
                    bfree: stat.f_bfree as u64,
                    bavail: stat.f_bavail as u64,
                    files: stat.f_files as u64,
                    ffree: stat.f_ffree as u64,
                    favail: stat.f_favail as u64,
                    fsid: stat.f_fsid as u64,
                    flag: stat.f_flag as u64,
                    namemax: stat.f_namemax as u64,
                })
            } else {
                Err(BackendError::from_io_error(&std::io::Error::last_os_error()))
            }
        }
    }

    async fn get_xattr(&self, _path: &str, _name: &str, _max_size: usize) -> BackendResult<Vec<u8>> {
        // For now, return ENOTSUP - extended attributes need platform-specific implementation
        Err(BackendError::new(libc::ENOTSUP, "Extended attributes not supported"))
    }

    fn describe(&self) -> String {
        format!("file://{}", self.root.display())
    }
}
