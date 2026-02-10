/// FileBackend - serves files from the local filesystem

use async_trait::async_trait;
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::traits::{Backend, WritableBackend};
use super::types::{
    AttributeInfo, BackendError, BackendResult, DirEntry, DirectoryListing, FileContent,
    StatfsInfo,
};

/// Strategy for computing ESTALE cookies
/// Corresponds to C++ exportd_options::esc_source_e
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstaleCookieSource {
    /// Use FS_IOC_GETVERSION ioctl (recommended for ext3/ext4/xfs/btrfs)
    GetVersionIoctl,
    /// Use extended attribute user.fs123.estalecookie
    ExtendedAttribute,
    /// Use inode number (st_ino) - not recommended, inodes can be reused
    Inode,
    /// Return 0 (disabled) - only for immutable filesystems
    None,
}

/// Backend implementation that serves files from the local filesystem
pub struct FileBackend {
    root: PathBuf,
    estalecookie_src: EstaleCookieSource,
}

impl FileBackend {
    /// Create a new FileBackend with the given root directory
    pub fn new(root: PathBuf) -> Self {
        FileBackend {
            root,
            estalecookie_src: EstaleCookieSource::GetVersionIoctl,
        }
    }

    /// Create a new FileBackend with specified ESTALE cookie strategy
    pub fn with_estale_strategy(root: PathBuf, estalecookie_src: EstaleCookieSource) -> Self {
        FileBackend {
            root,
            estalecookie_src,
        }
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

    /// Xattr name used to mark a file as having an active write session.
    const WRITE_SESSION_XATTR: &'static str = "user.fs123.write_session_active";

    /// Set an extended attribute on a path.
    fn set_xattr(path: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
        let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let c_name = std::ffi::CString::new(name)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let ret = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::setxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    value.as_ptr() as *const libc::c_void,
                    value.len(),
                    0,
                )
            }
            #[cfg(target_os = "macos")]
            {
                libc::setxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    value.as_ptr() as *const libc::c_void,
                    value.len(),
                    0,
                    0,
                )
            }
        };
        if ret != 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Get an extended attribute value from a path. Returns None if not found.
    fn get_xattr_value(path: &Path, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let c_name = std::ffi::CString::new(name)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let size = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::getxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                )
            }
            #[cfg(target_os = "macos")]
            {
                libc::getxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                )
            }
        };
        if size < 0 {
            let err = std::io::Error::last_os_error();
            #[cfg(target_os = "linux")]
            let not_found = err.raw_os_error() == Some(libc::ENODATA);
            #[cfg(target_os = "macos")]
            let not_found = err.raw_os_error() == Some(libc::ENOATTR);
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            let not_found = false;
            if not_found {
                return Ok(None);
            }
            return Err(err);
        }
        let mut buf = vec![0u8; size as usize];
        let ret = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::getxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            }
            #[cfg(target_os = "macos")]
            {
                libc::getxattr(
                    c_path.as_ptr(),
                    c_name.as_ptr(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    0,
                    0,
                )
            }
        };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            buf.truncate(ret as usize);
            Ok(Some(buf))
        }
    }

    /// Remove an extended attribute from a path.
    fn remove_xattr(path: &Path, name: &str) -> std::io::Result<()> {
        let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let c_name = std::ffi::CString::new(name)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let ret = unsafe {
            #[cfg(target_os = "linux")]
            {
                libc::removexattr(c_path.as_ptr(), c_name.as_ptr())
            }
            #[cfg(target_os = "macos")]
            {
                libc::removexattr(c_path.as_ptr(), c_name.as_ptr(), 0)
            }
        };
        if ret != 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Check if a file has an active write session.
    fn has_write_session(path: &Path) -> BackendResult<bool> {
        match Self::get_xattr_value(path, Self::WRITE_SESSION_XATTR) {
            Ok(Some(v)) => Ok(v == b"true"),
            Ok(None) => Ok(false),
            Err(e) => Err(BackendError::from_io_error(&e)),
        }
    }

    /// Get estalecookie for a file based on the configured strategy
    ///
    /// # Arguments
    /// * `path` - Full filesystem path (for opening file when needed)
    /// * `metadata` - File metadata
    ///
    /// # Returns
    /// A 64-bit cookie that changes when the inode at this path is replaced
    fn get_estalecookie(&self, path: &std::path::Path, metadata: &fs::Metadata) -> u64 {
        match self.estalecookie_src {
            EstaleCookieSource::GetVersionIoctl => {
                // Use FS_IOC_GETVERSION ioctl to get filesystem generation counter
                // This is the recommended strategy for ext3/ext4/xfs/btrfs
                #[cfg(target_os = "linux")]
                {
                    use std::os::unix::io::AsRawFd;

                    // FS_IOC_GETVERSION requires a file descriptor
                    // Open the file (using O_RDONLY, O_NOFOLLOW to avoid following symlinks)
                    match std::fs::File::open(path) {
                        Ok(file) => {
                            let fd = file.as_raw_fd();
                            let mut generation: libc::c_ulong = 0;

                            // FS_IOC_GETVERSION is defined as _IOR('v', 1, long)
                            // The value is 0x80087601 on most systems
                            const FS_IOC_GETVERSION: libc::c_ulong = 0x80087601;

                            unsafe {
                                if libc::ioctl(fd, FS_IOC_GETVERSION, &mut generation) == 0 {
                                    return generation as u64;
                                }
                            }
                            // If ioctl failed, fall back to inode number
                            // This happens on filesystems that don't support FS_IOC_GETVERSION
                            eprintln!("FS_IOC_GETVERSION failed for {:?}, falling back to st_ino", path);
                            metadata.ino()
                        }
                        Err(_) => {
                            // If we can't open the file, fall back to inode number
                            metadata.ino()
                        }
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    // FS_IOC_GETVERSION is Linux-specific, fall back to inode on other platforms
                    eprintln!(
                        "FS_IOC_GETVERSION not available on this platform for {:?}, using st_ino",
                        path
                    );
                    metadata.ino()
                }
            }
            EstaleCookieSource::ExtendedAttribute => {
                // TODO: Implement extended attribute-based strategy
                // This would read/write user.fs123.estalecookie xattr
                // For now, fall back to inode number
                eprintln!("ExtendedAttribute strategy not yet implemented, falling back to st_ino");
                metadata.ino()
            }
            EstaleCookieSource::Inode => {
                // Use inode number directly
                // Warning: This is not ideal as inodes can be reused after files are deleted
                metadata.ino()
            }
            EstaleCookieSource::None => {
                // Return 0 - indicates immutable data (inode never changes)
                0
            }
        }
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
            self.get_estalecookie(&full_path, &metadata)
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
            estalecookie: self.get_estalecookie(&full_path, &metadata),
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
            let entry_path = entry.path();
            let estalecookie = match entry_path.symlink_metadata() {
                Ok(meta) => self.get_estalecookie(&entry_path, &meta),
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
            estalecookie: self.get_estalecookie(&full_path, &metadata),
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

#[async_trait]
impl WritableBackend for FileBackend {
    async fn mkdir(&self, path: &str, mode: u32) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        fs::create_dir(&full_path).map_err(|e| BackendError::from_io_error(&e))?;
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let ret = unsafe { libc::chmod(c_path.as_ptr(), mode as libc::mode_t) };
        if ret != 0 {
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        Ok(())
    }

    async fn rmdir(&self, path: &str) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        fs::remove_dir(&full_path).map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn chmod(&self, path: &str, mode: u32) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let ret = unsafe { libc::chmod(c_path.as_ptr(), mode as libc::mode_t) };
        if ret != 0 {
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        Ok(())
    }

    async fn chown(&self, path: &str, uid: u32, gid: u32) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let ret = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
        if ret != 0 {
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        Ok(())
    }

    async fn utimens(
        &self,
        path: &str,
        atime: (i64, i64),
        mtime: (i64, i64),
    ) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let times = [
            libc::timespec {
                tv_sec: atime.0,
                tv_nsec: atime.1,
            },
            libc::timespec {
                tv_sec: mtime.0,
                tv_nsec: mtime.1,
            },
        ];
        let ret = unsafe {
            libc::utimensat(
                libc::AT_FDCWD,
                c_path.as_ptr(),
                times.as_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if ret != 0 {
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        Ok(())
    }

    async fn symlink(&self, target: &str, linkpath: &str) -> BackendResult<()> {
        let full_linkpath = self.resolve_path(linkpath);
        std::os::unix::fs::symlink(target, &full_linkpath)
            .map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn link(&self, oldpath: &str, newpath: &str) -> BackendResult<()> {
        let full_old = self.resolve_path(oldpath);
        let full_new = self.resolve_path(newpath);
        fs::hard_link(&full_old, &full_new).map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn unlink(&self, path: &str) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        fs::remove_file(&full_path).map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> BackendResult<()> {
        let full_from = self.resolve_path(from);
        let full_to = self.resolve_path(to);
        fs::rename(&full_from, &full_to).map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn setxattr(
        &self,
        _path: &str,
        _name: &str,
        _value: &[u8],
        _flags: u32,
    ) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOTSUP, "Extended attributes not supported"))
    }

    async fn removexattr(&self, _path: &str, _name: &str) -> BackendResult<()> {
        Err(BackendError::new(libc::ENOTSUP, "Extended attributes not supported"))
    }

    async fn check_access(&self, path: &str, mask: u32) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let ret = unsafe { libc::access(c_path.as_ptr(), mask as libc::c_int) };
        if ret != 0 {
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        Ok(())
    }

    async fn open_write(&self, path: &str, mode: u32) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        // Create file with O_CREAT|O_EXCL semantics (fails if exists)
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full_path)
            .map_err(|e| BackendError::from_io_error(&e))?;
        drop(file);
        // Set mode
        let c_path = std::ffi::CString::new(full_path.to_string_lossy().as_bytes())
            .map_err(|_| BackendError::new(libc::EINVAL, "Invalid path"))?;
        let ret = unsafe { libc::chmod(c_path.as_ptr(), mode as libc::mode_t) };
        if ret != 0 {
            // Clean up the file on failure
            let _ = fs::remove_file(&full_path);
            return Err(BackendError::from_io_error(&std::io::Error::last_os_error()));
        }
        // Set write session xattr
        Self::set_xattr(&full_path, Self::WRITE_SESSION_XATTR, b"true")
            .map_err(|e| {
                let _ = fs::remove_file(&full_path);
                BackendError::from_io_error(&e)
            })?;
        Ok(())
    }

    async fn write_data(&self, path: &str, data: &[u8]) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        // Verify write session is active
        if !Self::has_write_session(&full_path)? {
            return Err(BackendError::new(libc::EACCES, "No active write session"));
        }
        // Append data
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&full_path)
            .map_err(|e| BackendError::from_io_error(&e))?;
        file.write_all(data).map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }

    async fn close_write(&self, path: &str) -> BackendResult<()> {
        let full_path = self.resolve_path(path);
        // Verify write session is active
        if !Self::has_write_session(&full_path)? {
            return Err(BackendError::new(libc::EINVAL, "No active write session"));
        }
        // Remove write session xattr
        Self::remove_xattr(&full_path, Self::WRITE_SESSION_XATTR)
            .map_err(|e| BackendError::from_io_error(&e))?;
        Ok(())
    }
}
