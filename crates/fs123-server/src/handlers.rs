/// Request handlers for fs123 protocol endpoints

use crate::backends::{AttributeInfo, DirectoryListing, StatfsInfo};
use crate::response::Fs123ResponseBuilder;
use crate::ServerConfig;
use actix_web::HttpResponse;
use fs123_core::Fs123Request;
use serde_json::json;

/// Format attribute info as a space-separated stat string (v7)
fn format_stat_string(attr: &AttributeInfo) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        attr.mode,
        attr.nlink,
        attr.uid,
        attr.gid,
        attr.size,
        attr.mtime,
        attr.ctime,
        attr.atime,
        attr.ino,
        attr.mtime_nsec,
        attr.ctime_nsec,
        attr.atime_nsec,
        attr.dev,
        attr.blocks,
        attr.blksize,
        attr.rdev
    )
}

/// Format attribute info as a JSON value (v8)
fn format_stat_json(attr: &AttributeInfo) -> serde_json::Value {
    json!({
        "st_mode": attr.mode,
        "st_nlink": attr.nlink,
        "st_uid": attr.uid,
        "st_gid": attr.gid,
        "st_size": attr.size,
        "st_mtim": attr.mtime,
        "st_ctim": attr.ctime,
        "st_atim": attr.atime,
        "st_ino": attr.ino,
        "st_mtim_nsec": attr.mtime_nsec,
        "st_ctim_nsec": attr.ctime_nsec,
        "st_atim_nsec": attr.atime_nsec,
        "st_dev": attr.dev,
        "st_blocks": attr.blocks,
        "st_blksize": attr.blksize,
        "st_rdev": attr.rdev
    })
}

/// Format directory listing as content bytes (v7)
fn format_directory_content(listing: &DirectoryListing) -> Vec<u8> {
    let mut content = Vec::new();
    for entry in &listing.entries {
        // Format: <netstring_name> <d_type> <estalecookie>\n
        let mut line = Vec::new();
        line.extend_from_slice(&fs123_core::netstring_encode_str(&entry.name));
        line.push(b' ');
        line.extend_from_slice(entry.d_type.to_string().as_bytes());
        line.push(b' ');
        line.extend_from_slice(entry.estalecookie.to_string().as_bytes());
        line.push(b'\n');
        content.extend_from_slice(&line);
    }
    content
}

/// Format directory listing as JSON value (v8)
fn format_directory_json(listing: &DirectoryListing) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = listing
        .entries
        .iter()
        .map(|e| {
            json!({
                "d_name": e.name,
                "d_type": e.d_type,
                "estalecookie": e.estalecookie
            })
        })
        .collect();
    json!({ "entries": entries })
}

/// Format statfs info as JSON value (v8)
fn format_statfs_json(info: &StatfsInfo) -> serde_json::Value {
    json!({
        "f_bsize": info.bsize,
        "f_frsize": info.frsize,
        "f_blocks": info.blocks,
        "f_bfree": info.bfree,
        "f_bavail": info.bavail,
        "f_files": info.files,
        "f_ffree": info.ffree,
        "f_favail": info.favail,
        "f_fsid": info.fsid,
        "f_flag": info.flag,
        "f_namemax": info.namemax
    })
}

fn is_v8(request: &Fs123Request) -> bool {
    request.major_version >= 8
}

/// Handle /a (v7) or /stat (v8) endpoint - file/directory attributes
pub async fn handle_attributes(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);
    match config.backend.get_attributes(&request.path).await {
        Ok(attr) => {
            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .validator(attr.validator)
                .estalecookie(attr.estalecookie)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            if v8 {
                builder = builder.json_content(format_stat_json(&attr));
                builder.build_json()
            } else {
                builder = builder.content_str(&format_stat_string(&attr));
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_json()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /f (v7) or /read (v8) endpoint - file read
pub async fn handle_file_read(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);

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

    match config
        .backend
        .read_file(&request.path, offset_bytes, len_bytes)
        .await
    {
        Ok(content) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(0)
                .content(content.data)
                .validator(content.validator)
                .estalecookie(content.estalecookie)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /d (v7) or /readdir (v8) endpoint - directory listing
pub async fn handle_directory(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);

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

    let max_bytes = len_kib * 1024;

    match config
        .backend
        .read_directory(&request.path, max_bytes, start_after)
        .await
    {
        Ok(listing) => {
            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .estalecookie(listing.estalecookie)
                .nextstart(listing.nextstart.clone())
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            if v8 {
                builder = builder.json_content(format_directory_json(&listing));
                builder.build_json()
            } else {
                builder = builder.content(format_directory_content(&listing));
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_json()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /l (v7) or /readlink (v8) endpoint - symbolic link
pub async fn handle_symlink(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);
    match config.backend.read_symlink(&request.path).await {
        Ok(target) => {
            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            if v8 {
                builder = builder.json_content(json!({ "target": target }));
                builder.build_json()
            } else {
                builder = builder.content_str(&target);
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_json()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /s (v7) or /statvfs (v8) endpoint - statfs
pub async fn handle_statfs(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);
    match config.backend.statfs(&request.path).await {
        Ok(info) => {
            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            if v8 {
                builder = builder.json_content(format_statfs_json(&info));
                builder.build_json()
            } else {
                let statfs_str = format!(
                    "{} {} {} {} {} {} {} {} {} {} {}",
                    info.bsize,
                    info.frsize,
                    info.blocks,
                    info.bfree,
                    info.bavail,
                    info.files,
                    info.ffree,
                    info.favail,
                    info.fsid,
                    info.flag,
                    info.namemax
                );
                builder = builder.content_str(&statfs_str);
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_json()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle v7 /x endpoint - combined getxattr/listxattr (query params disambiguate).
pub async fn handle_xattr(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    // Query format: Len;Name;
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing length parameter");
    }

    let max_size: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    let name = if request.query_params.len() > 1 {
        &request.query_params[1]
    } else {
        ""
    };

    match config
        .backend
        .get_xattr(&request.path, name, max_size * 1024)
        .await
    {
        Ok(data) => Fs123ResponseBuilder::new()
            .errno(0)
            .content(data)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build(),
        Err(e) => Fs123ResponseBuilder::new()
            .errno(e.errno)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build(),
    }
}

/// Handle v8 /getxattr endpoint - get a specific extended attribute (name required).
pub async fn handle_getxattr(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);

    // Query format: Len;Name;
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing length parameter");
    }

    if request.query_params.len() < 2 || request.query_params[1].is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing attribute name parameter");
    }

    let max_size: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    let name = &request.query_params[1];

    match config
        .backend
        .get_xattr(&request.path, name, max_size * 1024)
        .await
    {
        Ok(data) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(0)
                .content(data)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /listxattr endpoint - list all extended attribute names (v8)
pub async fn handle_listxattr(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);

    // Query format: Len;
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing length parameter");
    }

    let max_size: usize = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid length parameter"),
    };

    // Empty name means list all xattrs
    match config
        .backend
        .get_xattr(&request.path, "", max_size * 1024)
        .await
    {
        Ok(data) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(0)
                .content(data)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
        Err(e) => {
            let builder = Fs123ResponseBuilder::new()
                .errno(e.errno)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);
            if v8 {
                builder.build_binary()
            } else {
                builder.build()
            }
        }
    }
}

/// Handle /n endpoint - server statistics
pub async fn handle_server_stats(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let v8 = is_v8(&request);

    let stats_str = format!(
        "fs123-server: {}\nbackend: {}\nmax_age: {}\nstale_while_revalidate: {}\n",
        env!("CARGO_PKG_VERSION"),
        config.backend.describe(),
        config.default_max_age,
        config.default_stale_while_revalidate
    );

    if v8 {
        Fs123ResponseBuilder::new()
            .errno(0)
            .json_content(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "backend": config.backend.describe(),
                "max_age": config.default_max_age,
                "stale_while_revalidate": config.default_stale_while_revalidate
            }))
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build_json()
    } else {
        Fs123ResponseBuilder::new()
            .errno(0)
            .content_str(&stats_str)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build()
    }
}

/// Handle /p endpoint - passthrough (not supported)
pub async fn handle_passthrough(
    _request: Fs123Request,
    _config: &ServerConfig,
) -> HttpResponse {
    Fs123ResponseBuilder::build_error(501, "Passthrough endpoint not implemented")
}

// --- v8 write operation handlers ---

use crate::backends::WritableBackend;
use std::sync::Arc;

/// Helper: get writable backend or return EROFS response
fn get_writable_backend(config: &ServerConfig) -> Result<&Arc<dyn WritableBackend>, HttpResponse> {
    config.writable_backend.as_ref().ok_or_else(|| {
        Fs123ResponseBuilder::new()
            .errno(libc::EROFS)
            .build()
    })
}

/// Helper: build success response (errno=0) with no cache
fn write_ok() -> HttpResponse {
    Fs123ResponseBuilder::new().errno(0).max_age(0).build()
}

/// Helper: build error response from BackendError with no cache
fn write_err(e: crate::backends::BackendError) -> HttpResponse {
    Fs123ResponseBuilder::new().errno(e.errno).max_age(0).build()
}

/// Handle mkdir - query: mode
pub async fn handle_mkdir(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing mode parameter");
    }
    let mode: u32 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mode parameter"),
    };
    match wb.mkdir(&request.path, mode).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle rmdir
pub async fn handle_rmdir(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    match wb.rmdir(&request.path).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle chmod - query: mode
pub async fn handle_chmod(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing mode parameter");
    }
    let mode: u32 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mode parameter"),
    };
    match wb.chmod(&request.path, mode).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle chown - query: uid;gid
pub async fn handle_chown(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.len() < 2 {
        return Fs123ResponseBuilder::build_error(400, "Missing uid;gid parameters");
    }
    let uid: u32 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid uid parameter"),
    };
    let gid: u32 = match request.query_params[1].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid gid parameter"),
    };
    match wb.chown(&request.path, uid, gid).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle utimens - query: atime_sec;atime_nsec;mtime_sec;mtime_nsec
pub async fn handle_utimens(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.len() < 4 {
        return Fs123ResponseBuilder::build_error(400, "Missing time parameters (atime_sec;atime_nsec;mtime_sec;mtime_nsec)");
    }
    let atime_sec: i64 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid atime_sec"),
    };
    let atime_nsec: i64 = match request.query_params[1].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid atime_nsec"),
    };
    let mtime_sec: i64 = match request.query_params[2].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mtime_sec"),
    };
    let mtime_nsec: i64 = match request.query_params[3].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mtime_nsec"),
    };
    match wb.utimens(&request.path, (atime_sec, atime_nsec), (mtime_sec, mtime_nsec)).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle symlink - query: target (the URL path is the linkpath)
pub async fn handle_create_symlink(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing target parameter");
    }
    let target = &request.query_params[0];
    match wb.symlink(target, &request.path).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle link - query: newpath (the URL path is the oldpath)
pub async fn handle_link(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing newpath parameter");
    }
    let newpath = &request.query_params[0];
    match wb.link(&request.path, newpath).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle unlink
pub async fn handle_unlink(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    match wb.unlink(&request.path).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle rename - query: newpath
pub async fn handle_rename(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing newpath parameter");
    }
    let newpath = &request.query_params[0];
    match wb.rename(&request.path, newpath).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle setxattr - query: name;flags, value in request body
/// Note: body handling requires changes at the routing level; for now, value is empty.
pub async fn handle_setxattr(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.len() < 2 {
        return Fs123ResponseBuilder::build_error(400, "Missing name;flags parameters");
    }
    let name = &request.query_params[0];
    let flags: u32 = match request.query_params[1].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid flags parameter"),
    };
    // TODO: read value from request body
    match wb.setxattr(&request.path, name, &[], flags).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Handle removexattr - query: name
pub async fn handle_removexattr(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing name parameter");
    }
    let name = &request.query_params[0];
    match wb.removexattr(&request.path, name).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}

/// Helper: build JSON success response with extra fields
fn write_ok_json_with(extra: serde_json::Value) -> HttpResponse {
    let mut obj = serde_json::json!({"errno": 0});
    if let (Some(base), Some(ext)) = (obj.as_object_mut(), extra.as_object()) {
        for (k, v) in ext {
            base.insert(k.clone(), v.clone());
        }
    }
    HttpResponse::Ok().json(obj)
}

/// Helper: build JSON error response
fn write_err_json(errno: i32) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"errno": errno}))
}

/// Handle create_upload - query: mode
pub async fn handle_create_upload(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing mode parameter");
    }
    let mode: u32 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mode parameter"),
    };
    match wb.create_upload(&request.path, mode).await {
        Ok(upload_id) => write_ok_json_with(serde_json::json!({"upload_id": upload_id})),
        Err(e) => write_err_json(e.errno),
    }
}

/// Handle upload_part - query: upload_id;part_number, data in request body
pub async fn handle_upload_part(request: Fs123Request, config: &ServerConfig, body: &[u8]) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.len() < 2 {
        return Fs123ResponseBuilder::build_error(400, "Missing upload_id;part_number parameters");
    }
    let upload_id = &request.query_params[0];
    let part_number: u32 = match request.query_params[1].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid part_number"),
    };
    match wb.upload_part(upload_id, part_number, body).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({"errno": 0})),
        Err(e) => write_err_json(e.errno),
    }
}

/// Handle complete_upload - query: upload_id
pub async fn handle_complete_upload(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing upload_id parameter");
    }
    let upload_id = &request.query_params[0];
    match wb.complete_upload(upload_id).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({"errno": 0})),
        Err(e) => write_err_json(e.errno),
    }
}

/// Handle abort_upload - query: upload_id
pub async fn handle_abort_upload(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing upload_id parameter");
    }
    let upload_id = &request.query_params[0];
    match wb.abort_upload(upload_id).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({"errno": 0})),
        Err(e) => write_err_json(e.errno),
    }
}

/// Handle access - query: mask
pub async fn handle_access(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let wb = match get_writable_backend(config) {
        Ok(wb) => wb,
        Err(resp) => return resp,
    };
    if request.query_params.is_empty() {
        return Fs123ResponseBuilder::build_error(400, "Missing mask parameter");
    }
    let mask: u32 = match request.query_params[0].parse() {
        Ok(v) => v,
        Err(_) => return Fs123ResponseBuilder::build_error(400, "Invalid mask parameter"),
    };
    match wb.check_access(&request.path, mask).await {
        Ok(()) => write_ok(),
        Err(e) => write_err(e),
    }
}
