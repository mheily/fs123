/// Request handlers for fs123 protocol endpoints

use crate::response::Fs123ResponseBuilder;
use crate::ServerConfig;
use fs123_core::Fs123Request;
use actix_web::HttpResponse;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::UNIX_EPOCH;

/// Convert errno to i32
fn errno_to_i32(err: &std::io::Error) -> i32 {
    err.raw_os_error().unwrap_or(libc::EIO)
}

/// Get validator for a file (use mtime nanoseconds as simple validator)
fn get_validator(metadata: &fs::Metadata) -> u64 {
    let mtime = metadata.modified().unwrap_or(UNIX_EPOCH);
    mtime.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Get estalecookie for a file
/// For now, use inode number (note: not ideal, see CLAUDE.md)
/// TODO: Use FS_IOC_GETVERSION or extended attributes
fn get_estalecookie(metadata: &fs::Metadata) -> u64 {
    metadata.ino()
}

/// Handle /a endpoint - file/directory attributes
pub async fn handle_attributes(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let full_path = config.export_root.join(&request.path[1..]); // Remove leading /

    match fs::symlink_metadata(&full_path) {
        Ok(metadata) => {
            // Build stat string with space-separated values
            let stat_str = format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                metadata.mode(),
                metadata.nlink(),
                metadata.uid(),
                metadata.gid(),
                metadata.size(),
                metadata.mtime(),
                metadata.ctime(),
                metadata.atime(),
                metadata.ino(),
                metadata.mtime_nsec(),
                metadata.ctime_nsec(),
                metadata.atime_nsec(),
                metadata.dev(),
                metadata.blocks(),
                metadata.blksize(),
                metadata.rdev()
            );

            let validator = get_validator(&metadata);
            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .content_str(&stat_str)
                .validator(validator)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            // Add estalecookie for regular files and directories
            let file_type = metadata.file_type();
            if file_type.is_file() || file_type.is_dir() {
                builder = builder.estalecookie(get_estalecookie(&metadata));
            }

            builder.build()
        }
        Err(e) => {
            let errno = errno_to_i32(&e);
            Fs123ResponseBuilder::new()
                .errno(errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    }
}

/// Handle /f endpoint - file read
pub async fn handle_file_read(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let full_path = config.export_root.join(&request.path[1..]);

    // Parse query parameters: Len;Offset (in KiB)
    if request.query_params.len() < 2 {
        return Fs123ResponseBuilder::build_error(400, "Missing length and offset parameters");
    }

    let len_kib: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    let offset_kib: u64 = match request.query_params[1].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid offset parameter"),
    };

    let len_bytes = len_kib * 1024;
    let offset_bytes = offset_kib * 1024;

    match fs::symlink_metadata(&full_path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Fs123ResponseBuilder::new()
                    .errno(libc::EINVAL)
                    .max_age(config.default_max_age)
                    .stale_while_revalidate(config.default_stale_while_revalidate)
                    .build();
            }

            match fs::read(&full_path) {
                Ok(file_content) => {
                    let start = offset_bytes as usize;
                    let end = std::cmp::min(start + len_bytes, file_content.len());

                    let content = if start < file_content.len() {
                        file_content[start..end].to_vec()
                    } else {
                        vec![]
                    };

                    let validator = get_validator(&metadata);
                    let estalecookie = get_estalecookie(&metadata);

                    Fs123ResponseBuilder::new()
                        .errno(0)
                        .content(content)
                        .validator(validator)
                        .estalecookie(estalecookie)
                        .max_age(config.default_max_age)
                        .stale_while_revalidate(config.default_stale_while_revalidate)
                        .build()
                }
                Err(e) => {
                    let errno = errno_to_i32(&e);
                    Fs123ResponseBuilder::new()
                        .errno(errno)
                        .max_age(config.default_max_age)
                        .stale_while_revalidate(config.default_stale_while_revalidate)
                        .build()
                }
            }
        }
        Err(e) => {
            let errno = errno_to_i32(&e);
            Fs123ResponseBuilder::new()
                .errno(errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    }
}

/// Handle /d endpoint - directory listing
pub async fn handle_directory(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let full_path = config.export_root.join(&request.path[1..]);

    // Parse query parameters: Len;Start
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing length parameter");
    }

    let len_kib: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    let start_after = if request.query_params.len() > 1 {
        &request.query_params[1]
    } else {
        ""
    };

    match fs::symlink_metadata(&full_path) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Fs123ResponseBuilder::new()
                    .errno(libc::ENOTDIR)
                    .max_age(config.default_max_age)
                    .stale_while_revalidate(config.default_stale_while_revalidate)
                    .build();
            }

            match fs::read_dir(&full_path) {
                Ok(entries) => {
                    let mut entry_list: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                    entry_list.sort_by_key(|e| e.file_name());

                    // Skip entries until we find start_after
                    let mut started = start_after.is_empty();
                    let mut content = Vec::new();
                    let mut last_entry_name = String::new();
                    let max_bytes = len_kib * 1024;

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
                            Ok(meta) => get_estalecookie(&meta),
                            Err(_) => 0,
                        };

                        // Format: <netstring_name> <d_type> <estalecookie>\n
                        let mut line = Vec::new();
                        line.extend_from_slice(&fs123_core::netstring_encode_str(&name_str));
                        line.push(b' ');
                        line.extend_from_slice(d_type.to_string().as_bytes());
                        line.push(b' ');
                        line.extend_from_slice(estalecookie.to_string().as_bytes());
                        line.push(b'\n');

                        if content.len() + line.len() > max_bytes {
                            break;
                        }

                        content.extend_from_slice(&line);
                        last_entry_name = name_str;
                    }

                    let nextstart = if content.is_empty() {
                        String::new()
                    } else {
                        last_entry_name
                    };

                    let estalecookie = get_estalecookie(&metadata);

                    Fs123ResponseBuilder::new()
                        .errno(0)
                        .content(content)
                        .estalecookie(estalecookie)
                        .nextstart(nextstart)
                        .max_age(config.default_max_age)
                        .stale_while_revalidate(config.default_stale_while_revalidate)
                        .build()
                }
                Err(e) => {
                    let errno = errno_to_i32(&e);
                    Fs123ResponseBuilder::new()
                        .errno(errno)
                        .max_age(config.default_max_age)
                        .stale_while_revalidate(config.default_stale_while_revalidate)
                        .build()
                }
            }
        }
        Err(e) => {
            let errno = errno_to_i32(&e);
            Fs123ResponseBuilder::new()
                .errno(errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    }
}

/// Handle /l endpoint - symbolic link
pub async fn handle_symlink(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let full_path = config.export_root.join(&request.path[1..]);

    match fs::read_link(&full_path) {
        Ok(target) => {
            let target_str = target.to_string_lossy().to_string();
            Fs123ResponseBuilder::new()
                .errno(0)
                .content_str(&target_str)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
        Err(e) => {
            let errno = errno_to_i32(&e);
            Fs123ResponseBuilder::new()
                .errno(errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    }
}

/// Handle /s endpoint - statfs
pub async fn handle_statfs(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let full_path = config.export_root.join(&request.path[1..]);

    // Use libc to get statfs information
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path_cstr = match CString::new(full_path.to_string_lossy().as_bytes()) {
        Ok(s) => s,
        Err(_) => {
            return Fs123ResponseBuilder::new()
                .errno(libc::EINVAL)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    };

    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        let ret = libc::statvfs(path_cstr.as_ptr(), stat.as_mut_ptr());

        if ret == 0 {
            let stat = stat.assume_init();
            let statfs_str = format!(
                "{} {} {} {} {} {} {} {} {} {} {}",
                stat.f_bsize,
                stat.f_frsize,
                stat.f_blocks,
                stat.f_bfree,
                stat.f_bavail,
                stat.f_files,
                stat.f_ffree,
                stat.f_favail,
                stat.f_fsid,
                stat.f_flag,
                stat.f_namemax
            );

            Fs123ResponseBuilder::new()
                .errno(0)
                .content_str(&statfs_str)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        } else {
            let errno = errno_to_i32(&std::io::Error::last_os_error());
            Fs123ResponseBuilder::new()
                .errno(errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
    }
}

/// Handle /x endpoint - extended attributes
pub async fn handle_xattr(
    request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    // Query format: Len;Name;
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing length parameter");
    }

    let _len_kib: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    // For now, return ENOTSUP - extended attributes need platform-specific implementation
    Fs123ResponseBuilder::new()
        .errno(libc::ENOTSUP)
        .max_age(config.default_max_age)
        .stale_while_revalidate(config.default_stale_while_revalidate)
        .build()
}

/// Handle /n endpoint - server statistics
pub async fn handle_server_stats(
    _request: Fs123Request,
    config: &ServerConfig,
) -> HttpResponse {
    let stats = format!(
        "fs123-server: {}\nexport_root: {}\nmax_age: {}\nstale_while_revalidate: {}\n",
        env!("CARGO_PKG_VERSION"),
        config.export_root.display(),
        config.default_max_age,
        config.default_stale_while_revalidate
    );

    Fs123ResponseBuilder::new()
        .errno(0)
        .content_str(&stats)
        .max_age(config.default_max_age)
        .stale_while_revalidate(config.default_stale_while_revalidate)
        .build()
}

/// Handle /p endpoint - passthrough (not supported)
pub async fn handle_passthrough(
    _request: Fs123Request,
    _config: &ServerConfig,
) -> HttpResponse {
    Fs123ResponseBuilder::build_error(501, "Passthrough endpoint not implemented")
}
