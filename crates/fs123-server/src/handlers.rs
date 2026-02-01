/// Request handlers for fs123 protocol endpoints

use crate::backends::{AttributeInfo, DirectoryListing};
use crate::response::Fs123ResponseBuilder;
use crate::ServerConfig;
use actix_web::HttpResponse;
use fs123_core::Fs123Request;

/// Format attribute info as a stat string
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

/// Format directory listing as content bytes
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

/// Handle /a endpoint - file/directory attributes
pub async fn handle_attributes(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    match config.backend.get_attributes(&request.path).await {
        Ok(attr) => {
            let stat_str = format_stat_string(&attr);

            let mut builder = Fs123ResponseBuilder::new()
                .errno(0)
                .content_str(&stat_str)
                .validator(attr.validator)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate);

            // Add estalecookie for regular files and directories
            if attr.is_file || attr.is_dir {
                builder = builder.estalecookie(attr.estalecookie);
            }

            builder.build()
        }
        Err(e) => Fs123ResponseBuilder::new()
            .errno(e.errno)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build(),
    }
}

/// Handle /f endpoint - file read
pub async fn handle_file_read(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
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
        Ok(content) => Fs123ResponseBuilder::new()
            .errno(0)
            .content(content.data)
            .validator(content.validator)
            .estalecookie(content.estalecookie)
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

/// Handle /d endpoint - directory listing
pub async fn handle_directory(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
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
            let content = format_directory_content(&listing);

            Fs123ResponseBuilder::new()
                .errno(0)
                .content(content)
                .estalecookie(listing.estalecookie)
                .nextstart(listing.nextstart)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
        Err(e) => Fs123ResponseBuilder::new()
            .errno(e.errno)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build(),
    }
}

/// Handle /l endpoint - symbolic link
pub async fn handle_symlink(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    match config.backend.read_symlink(&request.path).await {
        Ok(target) => Fs123ResponseBuilder::new()
            .errno(0)
            .content_str(&target)
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

/// Handle /s endpoint - statfs
pub async fn handle_statfs(request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    match config.backend.statfs(&request.path).await {
        Ok(info) => {
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

            Fs123ResponseBuilder::new()
                .errno(0)
                .content_str(&statfs_str)
                .max_age(config.default_max_age)
                .stale_while_revalidate(config.default_stale_while_revalidate)
                .build()
        }
        Err(e) => Fs123ResponseBuilder::new()
            .errno(e.errno)
            .max_age(config.default_max_age)
            .stale_while_revalidate(config.default_stale_while_revalidate)
            .build(),
    }
}

/// Handle /x endpoint - extended attributes
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

/// Handle /n endpoint - server statistics
pub async fn handle_server_stats(_request: Fs123Request, config: &ServerConfig) -> HttpResponse {
    let stats = format!(
        "fs123-server: {}\nbackend: {}\nmax_age: {}\nstale_while_revalidate: {}\n",
        env!("CARGO_PKG_VERSION"),
        config.backend.describe(),
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
