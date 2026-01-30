use actix_web::{web, App, HttpServer, HttpRequest, HttpResponse};
use clap::Parser;
use fs123_core::parse_url;
use fs123_server::{handlers, ServerConfig};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "fs123-server")]
#[command(about = "fs123 protocol HTTP server", long_about = None)]
struct Args {
    /// Address and port to bind to (e.g., 0.0.0.0:8123)
    #[arg(short, long, default_value = "127.0.0.1:8123")]
    bind: String,

    /// Root directory to export
    #[arg(short, long, default_value = "/tmp/fs123-export")]
    export_root: PathBuf,

    /// Default max-age for Cache-Control header (seconds)
    #[arg(long, default_value = "300")]
    max_age: u32,

    /// Default stale-while-revalidate for Cache-Control header (seconds)
    #[arg(long, default_value = "60")]
    stale_while_revalidate: u32,
}

/// Main request handler that routes to specific function handlers
async fn handle_request(
    req: HttpRequest,
    config: web::Data<ServerConfig>,
) -> HttpResponse {
    // Parse the URL to extract protocol components
    // Combine path and query string to form complete URL
    let path = req.path();
    let query = req.query_string();
    let full_url = if query.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, query)
    };

    eprintln!("DEBUG: path='{}', query='{}', full_url='{}'", path, query, full_url);

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
                _ => HttpResponse::BadRequest()
                    .body(format!("Unknown function: {}", request.function)),
            }
        }
        Err(e) => {
            HttpResponse::BadRequest().body(format!("Protocol error: {}", e))
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let config = ServerConfig {
        export_root: args.export_root,
        default_max_age: args.max_age,
        default_stale_while_revalidate: args.stale_while_revalidate,
    };

    println!("Starting fs123 server on {}", args.bind);
    println!("Export root: {:?}", config.export_root);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(config.clone()))
            .default_service(web::route().to(handle_request))
    })
    .bind(&args.bind)?
    .run()
    .await
}
