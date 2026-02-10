//! FUSE filesystem implementation for fs123.

use fs123_core::{
    types::{d_type, DirEntryData, Fs123StatResult, Fs123StatvfsResult},
    Fs123Function, Fs123HttpClient,
};
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    ReplyStatfs, ReplyXattr, Request,
};
use libc::{ENOENT, ENOTDIR, ESTALE};
use log::{debug, error};
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::cache::{
    BackgroundRefresher, CacheConfig, CacheMetadata, CachedAttributes, CachedDirectory,
    Fs123Cache,
};
use crate::inode::{InodeManager, ROOT_INO};

/// File handle information.
struct FileHandle {
    /// Inode this handle refers to
    ino: u64,
    /// Path at time of open (for consistency checking)
    path: String,
    /// ESTALE cookie at time of open
    estalecookie: u64,
}

/// fs123 FUSE filesystem.
pub struct Fs123Filesystem {
    /// HTTP client for server communication
    client: Arc<Mutex<Fs123HttpClient>>,
    /// Inode manager
    inodes: InodeManager,
    /// Open file handles
    file_handles: Mutex<std::collections::HashMap<u64, FileHandle>>,
    /// Next file handle number
    next_fh: std::sync::atomic::AtomicU64,
    /// Attribute cache timeout
    attr_timeout: Duration,
    /// Entry cache timeout
    entry_timeout: Duration,
    /// Application-level cache
    cache: Option<Arc<Fs123Cache>>,
    /// Background refresh thread (kept to ensure it lives with filesystem)
    _background_refresher: Option<BackgroundRefresher>,
}

impl Fs123Filesystem {
    /// Create a new fs123 filesystem.
    pub fn new(
        host: &str,
        port: u16,
        proto: &str,
        attr_timeout: Duration,
        entry_timeout: Duration,
        cache_config: Option<CacheConfig>,
    ) -> Self {
        let client = Arc::new(Mutex::new(Fs123HttpClient::new(host, port, proto)));

        let (cache, background_refresher) = if let Some(config) = cache_config {
            let (cache, rx) = Fs123Cache::new(config.clone());
            let cache = Arc::new(cache);

            let refresher = if config.enable_background_refresh {
                Some(BackgroundRefresher::new(
                    rx,
                    Arc::clone(&client),
                    Arc::clone(&cache),
                    config.refresh_threads,
                ))
            } else {
                None
            };

            (Some(cache), refresher)
        } else {
            (None, None)
        };

        Fs123Filesystem {
            client,
            inodes: InodeManager::new(),
            file_handles: Mutex::new(std::collections::HashMap::new()),
            next_fh: std::sync::atomic::AtomicU64::new(1),
            attr_timeout,
            entry_timeout,
            cache,
            _background_refresher: background_refresher,
        }
    }

    /// Get attributes from server for a path.
    fn stat_path(&self, path: &str) -> Result<(Fs123StatResult, u64, u64), i32> {
        // Check cache first if enabled
        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get_attr(path) {
                return Ok((cached.stat, cached.validator, cached.estalecookie));
            }
        }

        // Cache miss or disabled - fetch from server
        let client = self.client.lock().unwrap();
        let response = client.request_raw(Fs123Function::Stat, path, None).map_err(|e| {
            error!("stat_path failed for {}: {}", path, e);
            e.to_errno()
        })?;

        // Check errno
        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(errno);
            }
        }

        let content = response.content_str().ok_or_else(|| {
            error!("Missing content in stat response for {}", path);
            libc::EIO
        })?;

        let stat = Fs123StatResult::from_str(&content).ok_or_else(|| {
            error!("Failed to parse stat response for {}: {}", path, content);
            libc::EIO
        })?;

        let validator = response.validator().unwrap_or(0);
        let estalecookie = response.estalecookie().unwrap_or(0);

        // Store in cache if enabled
        if let Some(ref cache) = self.cache {
            let cached_attrs = CachedAttributes {
                stat: stat.clone(),
                validator,
                estalecookie,
            };
            let metadata = CacheMetadata {
                validator,
                estalecookie,
                cached_at: Instant::now(),
                max_age: cache.config.default_max_age,
                stale_while_revalidate: cache.config.default_swr,
            };
            cache.put_attr(path, cached_attrs, metadata);
        }

        Ok((stat, validator, estalecookie))
    }

    /// Convert Fs123StatResult to FileAttr.
    fn stat_to_attr(&self, ino: u64, stat: &Fs123StatResult) -> FileAttr {
        let kind = mode_to_filetype(stat.st_mode);

        FileAttr {
            ino,
            size: stat.st_size as u64,
            blocks: stat.st_blocks as u64,
            atime: UNIX_EPOCH + Duration::new(stat.st_atime as u64, stat.st_atime_nsec as u32),
            mtime: UNIX_EPOCH + Duration::new(stat.st_mtime as u64, stat.st_mtime_nsec as u32),
            ctime: UNIX_EPOCH + Duration::new(stat.st_ctime as u64, stat.st_ctime_nsec as u32),
            crtime: UNIX_EPOCH, // Not available in fs123
            kind,
            perm: (stat.st_mode & 0o7777) as u16,
            nlink: stat.st_nlink as u32,
            uid: stat.st_uid,
            gid: stat.st_gid,
            rdev: stat.st_rdev as u32,
            blksize: stat.st_blksize as u32,
            flags: 0,
        }
    }

    /// Read file content from server.
    fn read_file(&self, path: &str, offset: i64, size: u32) -> Result<Vec<u8>, i32> {
        // Convert bytes to KiB (rounding down for offset, up for size)
        let offset_kib = offset / 1024;
        let end_offset = offset + size as i64;
        let end_kib = (end_offset + 1023) / 1024;
        let size_kib = end_kib - offset_kib;

        // Protocol: /f/path?Len;Offset (Len first, then Offset)
        let params = [size_kib.to_string(), offset_kib.to_string()];
        let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

        let client = self.client.lock().unwrap();
        let response = client
            .request(Fs123Function::Read, path, Some(&params_ref))
            .map_err(|e| e.to_errno())?;

        let content = response.content().unwrap_or(&[]);

        // Extract the requested portion (handle misalignment within KiB blocks)
        let start_in_block = (offset % 1024) as usize;
        let available = content.len().saturating_sub(start_in_block);
        let to_copy = std::cmp::min(available, size as usize);

        Ok(content[start_in_block..start_in_block + to_copy].to_vec())
    }

    /// Read directory entries from server.
    fn read_directory(&self, path: &str) -> Result<Vec<DirEntryData>, i32> {
        // Check cache first if enabled
        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get_dir(path) {
                return Ok(cached.entries);
            }
        }

        // Cache miss or disabled - fetch from server
        let mut all_entries = Vec::new();
        let mut nextstart: Option<String> = None;

        loop {
            let params = match &nextstart {
                Some(start) => vec!["64".to_string(), start.clone()],
                None => vec!["64".to_string(), String::new()],
            };
            let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

            let client = self.client.lock().unwrap();
            let response = client
                .request(Fs123Function::Readdir, path, Some(&params_ref))
                .map_err(|e| e.to_errno())?;

            if let Some(content) = response.content() {
                let entries = DirEntryData::parse_entries(content);
                all_entries.extend(entries);
            }

            // Check if there are more entries
            if response.has_more_entries() {
                if let Some(next) = response.nextstart() {
                    nextstart = Some(String::from_utf8_lossy(&next).to_string());
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Store in cache if enabled
        if let Some(ref cache) = self.cache {
            let cached_dir = CachedDirectory {
                entries: all_entries.clone(),
                estalecookie: 0,
            };
            let metadata = CacheMetadata {
                validator: 0,
                estalecookie: 0,
                cached_at: Instant::now(),
                max_age: cache.config.default_max_age,
                stale_while_revalidate: cache.config.default_swr,
            };
            cache.put_dir(path, cached_dir, metadata);
        }

        Ok(all_entries)
    }

    /// Read symlink target from server.
    fn read_link(&self, path: &str) -> Result<String, i32> {
        // Check cache first if enabled
        if let Some(ref cache) = self.cache {
            if let Some(cached) = cache.get_symlink(path) {
                return Ok(cached);
            }
        }

        // Cache miss or disabled - fetch from server
        let client = self.client.lock().unwrap();
        let response = client.request(Fs123Function::Readlink, path, None).map_err(|e| e.to_errno())?;

        let target = response.content_str().ok_or(libc::EIO)?;

        // Store in cache if enabled
        if let Some(ref cache) = self.cache {
            let metadata = CacheMetadata {
                validator: 0,
                estalecookie: 0,
                cached_at: Instant::now(),
                max_age: cache.config.default_max_age,
                stale_while_revalidate: cache.config.default_swr,
            };
            cache.put_symlink(path, target.clone(), metadata);
        }

        Ok(target)
    }

    /// Get statvfs from server.
    fn get_statvfs(&self, path: &str) -> Result<Fs123StatvfsResult, i32> {
        let client = self.client.lock().unwrap();
        let response = client.request(Fs123Function::Statvfs, path, None).map_err(|e| e.to_errno())?;

        let content = response.content_str().ok_or(libc::EIO)?;
        Fs123StatvfsResult::from_str(&content).ok_or(libc::EIO)
    }

    /// Get extended attribute value.
    fn get_xattr(&self, path: &str, name: &str) -> Result<Vec<u8>, i32> {
        // First get the attribute
        let params = ["128".to_string(), name.to_string()];
        let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

        let client = self.client.lock().unwrap();
        let response = client
            .request(Fs123Function::Getxattr, path, Some(&params_ref))
            .map_err(|e| e.to_errno())?;

        response
            .content()
            .map(|c| c.to_vec())
            .ok_or(libc::ENODATA)
    }

    /// List extended attribute names.
    fn list_xattr(&self, path: &str) -> Result<Vec<u8>, i32> {
        // First get the size
        let params = ["0".to_string()];
        let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

        let client = self.client.lock().unwrap();
        let response = client
            .request_raw(Fs123Function::Listxattr, path, Some(&params_ref))
            .map_err(|e| e.to_errno())?;

        // Check errno
        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(errno);
            }
        }

        let size_str = response.content_str().ok_or(libc::EIO)?;
        let _size: usize = size_str.trim().parse().map_err(|_| libc::EIO)?;

        // Now get the actual list
        let params = ["128".to_string(), String::new()];
        let params_ref: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

        let response = client
            .request(Fs123Function::Listxattr, path, Some(&params_ref))
            .map_err(|e| e.to_errno())?;

        response
            .content()
            .map(|c| c.to_vec())
            .ok_or_else(|| libc::ENODATA)
    }
}

impl Filesystem for Fs123Filesystem {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        debug!("fs123 filesystem initialized");
        Ok(())
    }

    fn destroy(&mut self) {
        debug!("fs123 filesystem destroyed");
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("lookup: parent={}, name={}", parent, name_str);

        // Build child path
        let child_path = match self.inodes.child_path(parent, name_str) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Get attributes from server
        match self.stat_path(&child_path) {
            Ok((stat, _validator, estalecookie)) => {
                let file_type = mode_to_dtype(stat.st_mode);
                let ino = self.inodes.get_or_create(&child_path, estalecookie, file_type);
                self.inodes.lookup(ino);

                let attr = self.stat_to_attr(ino, &stat);
                reply.entry(&self.entry_timeout, &attr, 0);
            }
            Err(errno) => {
                debug!("lookup failed for {}: errno={}", child_path, errno);
                reply.error(errno);
            }
        }
    }

    fn forget(&mut self, _req: &Request<'_>, ino: u64, nlookup: u64) {
        debug!("forget: ino={}, nlookup={}", ino, nlookup);
        self.inodes.forget(ino, nlookup);
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.stat_path(&path) {
            Ok((stat, _validator, estalecookie)) => {
                // Update estalecookie if it changed
                self.inodes.update_estalecookie(ino, estalecookie);

                let attr = self.stat_to_attr(ino, &stat);
                reply.attr(&self.attr_timeout, &attr);
            }
            Err(errno) => {
                reply.error(errno);
            }
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        debug!("readlink: ino={}", ino);

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.read_link(&path) {
            Ok(target) => reply.data(target.as_bytes()),
            Err(errno) => reply.error(errno),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}, flags={:#x}", ino, flags);

        // Check for write flags (we're read-only)
        let write_flags = libc::O_WRONLY | libc::O_RDWR | libc::O_TRUNC | libc::O_APPEND;
        if flags & write_flags != 0 {
            reply.error(libc::EROFS);
            return;
        }

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Verify the file still exists and get estalecookie
        match self.stat_path(&path) {
            Ok((_stat, _validator, estalecookie)) => {
                let fh = self
                    .next_fh
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let handle = FileHandle {
                    ino,
                    path,
                    estalecookie,
                };

                self.file_handles.lock().unwrap().insert(fh, handle);

                // Use keep_cache=true since we use estalecookies for consistency
                reply.opened(fh, fuser::consts::FOPEN_KEEP_CACHE);
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, fh={}, offset={}, size={}", ino, fh, offset, size);

        // Get file handle
        let handle = match self.file_handles.lock().unwrap().get(&fh) {
            Some(h) => h.clone(),
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // Check estalecookie for consistency
        if handle.estalecookie != 0 {
            if let Some(current_cookie) = self.inodes.get_estalecookie(ino) {
                if current_cookie != 0 && current_cookie != handle.estalecookie {
                    reply.error(ESTALE);
                    return;
                }
            }
        }

        match self.read_file(&handle.path, offset, size) {
            Ok(data) => reply.data(&data),
            Err(errno) => reply.error(errno),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("release: ino={}, fh={}", ino, fh);
        self.file_handles.lock().unwrap().remove(&fh);
        reply.ok();
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("opendir: ino={}, flags={:#x}", ino, flags);

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Verify it's a directory
        match self.stat_path(&path) {
            Ok((stat, _validator, estalecookie)) => {
                if !stat.is_dir() {
                    reply.error(ENOTDIR);
                    return;
                }

                let fh = self
                    .next_fh
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let handle = FileHandle {
                    ino,
                    path,
                    estalecookie,
                };

                self.file_handles.lock().unwrap().insert(fh, handle);
                reply.opened(fh, 0);
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, fh={}, offset={}", ino, fh, offset);

        // Get file handle
        let handle = match self.file_handles.lock().unwrap().get(&fh) {
            Some(h) => h.clone(),
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // Read directory entries
        let entries = match self.read_directory(&handle.path) {
            Ok(e) => e,
            Err(errno) => {
                reply.error(errno);
                return;
            }
        };

        // Add . and .. entries
        let mut all_entries = vec![
            (".", ino, FileType::Directory),
            ("..", ROOT_INO, FileType::Directory), // Simplified: use root for ..
        ];

        // Add regular entries
        for entry in &entries {
            let child_path = if handle.path == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", handle.path, entry.name)
            };

            let child_ino = self.inodes.get_or_create(&child_path, entry.estalecookie, entry.d_type);
            let file_type = dtype_to_filetype(entry.d_type);
            all_entries.push((entry.name.as_str(), child_ino, file_type));
        }

        // Skip entries before offset and add remaining
        for (i, (name, ino, kind)) in all_entries.iter().enumerate().skip(offset as usize) {
            // reply.add returns true if buffer is full
            if reply.add(*ino, (i + 1) as i64, *kind, name) {
                break;
            }
        }

        reply.ok();
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        reply: fuser::ReplyEmpty,
    ) {
        debug!("releasedir: ino={}, fh={}", ino, fh);
        self.file_handles.lock().unwrap().remove(&fh);
        reply.ok();
    }

    fn statfs(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyStatfs) {
        debug!("statfs: ino={}", ino);

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.get_statvfs(&path) {
            Ok(st) => {
                reply.statfs(
                    st.f_blocks,
                    st.f_bfree,
                    st.f_bavail,
                    st.f_files,
                    st.f_ffree,
                    st.f_bsize as u32,
                    st.f_namemax as u32,
                    st.f_frsize as u32,
                );
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        size: u32,
        reply: ReplyXattr,
    ) {
        debug!("getxattr: ino={}, name={:?}, size={}", ino, name, size);

        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.get_xattr(&path, name_str) {
            Ok(data) => {
                if size == 0 {
                    reply.size(data.len() as u32);
                } else if size < data.len() as u32 {
                    reply.error(libc::ERANGE);
                } else {
                    reply.data(&data);
                }
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: ReplyXattr) {
        debug!("listxattr: ino={}, size={}", ino, size);

        let path = match self.inodes.get_path(ino) {
            Some(p) => p,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match self.list_xattr(&path) {
            Ok(data) => {
                if size == 0 {
                    reply.size(data.len() as u32);
                } else if size < data.len() as u32 {
                    reply.error(libc::ERANGE);
                } else {
                    reply.data(&data);
                }
            }
            Err(errno) => reply.error(errno),
        }
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {
        debug!("access: ino={}, mask={:#x}", ino, mask);

        // For read-only filesystem, deny write access
        if mask & libc::W_OK != 0 {
            reply.error(libc::EROFS);
            return;
        }

        // Verify the inode exists
        if self.inodes.get_path(ino).is_some() {
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }
}

/// Convert st_mode to FileType.
fn mode_to_filetype(mode: u32) -> FileType {
    let file_type = mode & libc::S_IFMT as u32;
    match file_type {
        x if x == libc::S_IFREG as u32 => FileType::RegularFile,
        x if x == libc::S_IFDIR as u32 => FileType::Directory,
        x if x == libc::S_IFLNK as u32 => FileType::Symlink,
        x if x == libc::S_IFBLK as u32 => FileType::BlockDevice,
        x if x == libc::S_IFCHR as u32 => FileType::CharDevice,
        x if x == libc::S_IFIFO as u32 => FileType::NamedPipe,
        x if x == libc::S_IFSOCK as u32 => FileType::Socket,
        _ => FileType::RegularFile,
    }
}

/// Convert st_mode to d_type constant.
fn mode_to_dtype(mode: u32) -> u8 {
    let file_type = mode & libc::S_IFMT as u32;
    match file_type {
        x if x == libc::S_IFREG as u32 => d_type::DT_REG,
        x if x == libc::S_IFDIR as u32 => d_type::DT_DIR,
        x if x == libc::S_IFLNK as u32 => d_type::DT_LNK,
        x if x == libc::S_IFBLK as u32 => d_type::DT_BLK,
        x if x == libc::S_IFCHR as u32 => d_type::DT_CHR,
        x if x == libc::S_IFIFO as u32 => d_type::DT_FIFO,
        x if x == libc::S_IFSOCK as u32 => d_type::DT_SOCK,
        _ => d_type::DT_UNKNOWN,
    }
}

/// Convert d_type to FileType.
fn dtype_to_filetype(dtype: u8) -> FileType {
    match dtype {
        d_type::DT_REG => FileType::RegularFile,
        d_type::DT_DIR => FileType::Directory,
        d_type::DT_LNK => FileType::Symlink,
        d_type::DT_BLK => FileType::BlockDevice,
        d_type::DT_CHR => FileType::CharDevice,
        d_type::DT_FIFO => FileType::NamedPipe,
        d_type::DT_SOCK => FileType::Socket,
        _ => FileType::RegularFile,
    }
}

// Implement Clone for FileHandle so we can copy it out of the mutex
impl Clone for FileHandle {
    fn clone(&self) -> Self {
        FileHandle {
            ino: self.ino,
            path: self.path.clone(),
            estalecookie: self.estalecookie,
        }
    }
}
