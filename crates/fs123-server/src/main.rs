use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use clap::Parser;
use fs123_core::parse_url;
use fs123_server::{backends, handlers, ServerConfig};
use url::Url;

#[derive(Parser, Debug)]
#[command(name = "fs123-server")]
#[command(about = "fs123 protocol HTTP server", long_about = None)]
struct Args {
    /// Address and port to bind to (e.g., 0.0.0.0:8123)
    #[arg(short, long, default_value = "127.0.0.1:8123")]
    bind: String,

    /// Root directory or URL to export (e.g., /path, file:///path, or file:///path?estalecookie=inode)
    /// URL parameters:
    ///   estalecookie=ioc_getversion (default) - Use FS_IOC_GETVERSION ioctl
    ///   estalecookie=xattr - Use extended attributes (not yet implemented)
    ///   estalecookie=inode - Use inode number (not recommended)
    ///   estalecookie=none - Disabled (return 0)
    #[arg(short, long, default_value = "/srv/fs123")]
    export_root: String,

    /// Default max-age for Cache-Control header (seconds)
    #[arg(long, default_value = "300")]
    max_age: u32,

    /// Default stale-while-revalidate for Cache-Control header (seconds)
    #[arg(long, default_value = "60")]
    stale_while_revalidate: u32,
}

/// Main request handler that routes to specific function handlers
async fn handle_request(req: HttpRequest, config: web::Data<ServerConfig>) -> HttpResponse {
    // Parse the URL to extract protocol components
    // Combine path and query string to form complete URL
    let path = req.path();
    let query = req.query_string();
    let full_url = if query.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, query)
    };

    eprintln!(
        "DEBUG: path='{}', query='{}', full_url='{}'",
        path, query, full_url
    );

    match parse_url(&full_url) {
        Ok(request) => {
            // Route to the appropriate handler based on function
            match request.function.as_str() {
                "a" => handlers::handle_attributes(request, &config).await,
                "f" => handlers::handle_file_read(request, &config).await,
                "d" => handlers::handle_directory(request, &config).await,
                "l" => handlers::handle_symlink(request, &config).await,
                "s" => handlers::handle_statfs(request, &config).await,
                "x" => handlers::handle_xattr(request, &config).await,
                "n" => handlers::handle_server_stats(request, &config).await,
                "p" => handlers::handle_passthrough(request, &config).await,
                _ => {
                    HttpResponse::BadRequest().body(format!("Unknown function: {}", request.function))
                }
            }
        }
        Err(e) => HttpResponse::BadRequest().body(format!("Protocol error: {}", e)),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();

    // Parse export_root as URL
    // If it's a bare path (starts with / or ./), convert to file:// URL
    let url_str = if args.export_root.starts_with('/') {
        // Absolute path - convert to file:// URL
        format!("file://{}", args.export_root)
    } else if args.export_root.starts_with("./") || args.export_root.starts_with("../") {
        // Relative path - convert to file:// URL
        format!("file://{}", args.export_root)
    } else if args.export_root.contains("://") {
        // Already a URL
        args.export_root.clone()
    } else {
        // Assume it's a relative path
        format!("file://{}", args.export_root)
    };

    let url = Url::parse(&url_str).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Invalid URL '{}': {}", url_str, e),
        )
    })?;

    // Create backend from parsed URL
    let backend = backends::create_backend(&url)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    let config = ServerConfig {
        backend,
        default_max_age: args.max_age,
        default_stale_while_revalidate: args.stale_while_revalidate,
    };

    println!("Starting fs123 server on {}", args.bind);
    println!("Backend: {}", config.backend.describe());

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .default_service(web::route().to(handle_request))
    })
    .bind(&args.bind)?
    .run()
    .await
}
