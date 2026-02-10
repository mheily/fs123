//! fs123 protocol implementation.
//!
//! Handles URL building, HTTP requests, and response parsing.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use std::collections::HashMap;
use std::io::Read;

use crate::error::{Fs123Error, Result};
use crate::netstring::parse_response;

/// Protocol function identifiers.
///
/// Maps between internal operation names and protocol-version-specific URL strings.
/// v7.x uses single-letter function names, v8+ uses descriptive multi-letter names.
///
/// The `Xattr` variant represents v7's combined `/x` endpoint, which handles both
/// getxattr and listxattr. The distinction between the two is made by the query
/// parameters, not the function name. Clients should use `Getxattr` or `Listxattr`
/// directly; `Xattr` only appears when parsing v7 server requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fs123Function {
    Stat,
    Read,
    Readdir,
    Readlink,
    Statvfs,
    /// v7 combined xattr endpoint — query params determine get vs list.
    /// Only produced by `from_url_str("x")`; clients should use `Getxattr`/`Listxattr`.
    Xattr,
    Getxattr,
    Listxattr,
    ServerStats,
    Passthrough,
}

impl Fs123Function {
    /// Parse a function name from a URL string (either v7 or v8 format).
    ///
    /// v7's `"x"` maps to `Xattr` (ambiguous), while v8's `"getxattr"` and
    /// `"listxattr"` map to their specific variants.
    pub fn from_url_str(s: &str) -> Option<Self> {
        match s {
            "a" | "stat" => Some(Self::Stat),
            "f" | "read" => Some(Self::Read),
            "d" | "readdir" => Some(Self::Readdir),
            "l" | "readlink" => Some(Self::Readlink),
            "s" | "statvfs" => Some(Self::Statvfs),
            "x" => Some(Self::Xattr),
            "getxattr" => Some(Self::Getxattr),
            "listxattr" => Some(Self::Listxattr),
            "n" => Some(Self::ServerStats),
            "p" => Some(Self::Passthrough),
            _ => None,
        }
    }

    /// Return the URL string for this function in the given protocol major version.
    ///
    /// `Getxattr` and `Listxattr` both emit `"x"` for v7. `Xattr` always emits `"x"`
    /// regardless of version (it should not be used for v8 outbound requests).
    pub fn to_url_str(self, major_version: u32) -> &'static str {
        if major_version >= 8 {
            match self {
                Self::Stat => "stat",
                Self::Read => "read",
                Self::Readdir => "readdir",
                Self::Readlink => "readlink",
                Self::Statvfs => "statvfs",
                Self::Xattr | Self::Getxattr => "getxattr",
                Self::Listxattr => "listxattr",
                Self::ServerStats => "n",
                Self::Passthrough => "p",
            }
        } else {
            match self {
                Self::Stat => "a",
                Self::Read => "f",
                Self::Readdir => "d",
                Self::Readlink => "l",
                Self::Statvfs => "s",
                Self::Xattr | Self::Getxattr | Self::Listxattr => "x",
                Self::ServerStats => "n",
                Self::Passthrough => "p",
            }
        }
    }
}

// Define a custom encoding set for fs123 URLs.
// We need to encode spaces and other problematic characters,
// but we should NOT encode dots, dashes, underscores, and tildes
// which are safe in URLs.
const FS123_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b']')
    .add(b'{')
    .add(b'}')
    .add(b';'); // semicolon is used as query separator, so encode it in path

/// Parsed fs123 protocol request (for servers/testing).
#[derive(Debug, Clone)]
pub struct Fs123Request {
    pub selector: Vec<String>,
    pub major_version: u32,
    pub minor_version: u32,
    pub function: Fs123Function,
    pub path: String,
    pub query_params: Vec<String>,
}

/// Parse an fs123 URL into its components.
///
/// URL format: /SELECTOR/fs123/MAJOR/MINOR/FUNCTION/PATH?QUERY
pub fn parse_url(url: &str) -> std::result::Result<Fs123Request, String> {
    // Split URL into path and query
    let (path_part, query_part) = match url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url, None),
    };

    // Split path into components
    let components: Vec<&str> = path_part.split('/').filter(|s| !s.is_empty()).collect();

    // Find the fs123 sigil
    let sigil_pos = components
        .iter()
        .position(|&s| s == "fs123")
        .ok_or("URL must contain /fs123/ sigil")?;

    // Extract selector (everything before fs123)
    let selector: Vec<String> = components[..sigil_pos].iter().map(|s| s.to_string()).collect();

    // After fs123, we expect: MAJOR [/MINOR] /FUNCTION /PATH...
    let after_sigil = &components[sigil_pos + 1..];
    if after_sigil.is_empty() {
        return Err("Missing protocol version after /fs123/".to_string());
    }

    // Parse major version
    let major_version: u32 = after_sigil[0]
        .parse()
        .map_err(|_| format!("Invalid major version: {}", after_sigil[0]))?;

    // Determine if next component is minor version or function
    let mut idx = 1;
    let minor_version: u32;

    if idx < after_sigil.len() {
        // Try to parse as number - if it works, it's minor version
        if let Ok(minor) = after_sigil[idx].parse::<u32>() {
            minor_version = minor;
            idx += 1;
        } else {
            // If major is 7 and next is not a number, minor defaults to 0
            if major_version == 7 {
                minor_version = 0;
            } else {
                return Err("Minor version required for non-7 major versions".to_string());
            }
        }
    } else {
        // No more components - minor defaults to 0 for major version 7
        if major_version == 7 {
            minor_version = 0;
        } else {
            return Err("Minor version required".to_string());
        }
    }

    // Next component is the function
    if idx >= after_sigil.len() {
        return Err("Missing function".to_string());
    }
    let function = Fs123Function::from_url_str(after_sigil[idx])
        .ok_or_else(|| format!("Unknown function: {}", after_sigil[idx]))?;
    idx += 1;

    // Remaining components form the path (URL-decode each component)
    let path_components = &after_sigil[idx..];
    let path = if path_components.is_empty() {
        String::from("/")
    } else {
        let decoded_components: Vec<String> = path_components
            .iter()
            .map(|s| {
                percent_decode_str(s)
                    .decode_utf8()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        format!("/{}", decoded_components.join("/"))
    };

    // Parse query parameters (semicolon-separated)
    let query_params = if let Some(query) = query_part {
        query
            .split(';')
            .map(|s| {
                // URL-decode each parameter
                percent_decode_str(s)
                    .decode_utf8()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    } else {
        vec![]
    };

    Ok(Fs123Request {
        selector,
        major_version,
        minor_version,
        function,
        path,
        query_params,
    })
}

/// Build an fs123 protocol URL.
///
/// # Arguments
/// * `proto` - Protocol version (e.g., "7.2" or "7.3")
/// * `function` - Single-letter function code (a, f, d, l, s, x, n, p)
/// * `path` - Path relative to export root
/// * `query_params` - Optional list of query parameters (semicolon-separated)
///
/// # Returns
/// The URL path (without scheme/host)
pub fn build_url(proto: &str, function: Fs123Function, path: &str, query_params: Option<&[&str]>) -> String {
    // Parse protocol version
    let (major, minor) = if let Some(pos) = proto.find('.') {
        (&proto[..pos], &proto[pos + 1..])
    } else {
        (proto, "0")
    };

    let major_version: u32 = major.parse().unwrap_or(7);
    let function = function.to_url_str(major_version);

    // Ensure path starts with /
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    // URL-encode path components
    let encoded_path: String = path
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|p| utf8_percent_encode(p, FS123_ENCODE_SET).to_string())
        .collect::<Vec<_>>()
        .join("/");

    let encoded_path = if encoded_path.is_empty() {
        String::new()
    } else {
        format!("/{}", encoded_path)
    };

    // Build base URL
    let mut url = format!("/fs123/{}/{}/{}{}", major, minor, function, encoded_path);

    // Add query parameters if present
    if let Some(params) = query_params {
        if !params.is_empty() {
            let encoded_params: Vec<String> = params
                .iter()
                .map(|p| utf8_percent_encode(p, FS123_ENCODE_SET).to_string())
                .collect();
            url.push('?');
            url.push_str(&encoded_params.join(";"));
        }
    }

    url
}

/// Parsed response from an fs123 server.
#[derive(Debug)]
pub struct Fs123Response {
    /// Key-value pairs from the response body
    pub fields: HashMap<String, Vec<u8>>,
}

impl Fs123Response {
    /// Get a field as a string.
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.fields
            .get(key)
            .and_then(|v| std::str::from_utf8(v).ok())
            .map(|s| s.to_string())
    }

    /// Get a field as bytes.
    pub fn get_bytes(&self, key: &str) -> Option<&[u8]> {
        self.fields.get(key).map(|v| v.as_slice())
    }

    /// Get the content field as bytes.
    pub fn content(&self) -> Option<&[u8]> {
        self.get_bytes("content")
    }

    /// Get the content field as a string.
    pub fn content_str(&self) -> Option<String> {
        self.get_str("content")
    }

    /// Get the errno field.
    pub fn errno(&self) -> Option<i32> {
        self.get_str("errno").and_then(|s| s.parse().ok())
    }

    /// Get the validator field.
    pub fn validator(&self) -> Option<u64> {
        self.get_str("validator").and_then(|s| s.parse().ok())
    }

    /// Get the estalecookie field.
    pub fn estalecookie(&self) -> Option<u64> {
        self.get_str("estalecookie").and_then(|s| s.parse().ok())
    }

    /// Get the nextstart field (for directory iteration).
    pub fn nextstart(&self) -> Option<Vec<u8>> {
        self.fields.get("nextstart").cloned()
    }

    /// Check if nextstart indicates more entries.
    pub fn has_more_entries(&self) -> bool {
        self.fields
            .get("nextstart")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

/// HTTP client for fs123 protocol.
pub struct Fs123HttpClient {
    host: String,
    port: u16,
    proto: String,
    agent: ureq::Agent,
}

impl Fs123HttpClient {
    /// Create a new HTTP client.
    pub fn new(host: &str, port: u16, proto: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(30))
            .timeout_read(std::time::Duration::from_secs(60))
            .build();

        Fs123HttpClient {
            host: host.to_string(),
            port,
            proto: proto.to_string(),
            agent,
        }
    }

    /// Send a request to the server.
    pub fn request(
        &self,
        function: Fs123Function,
        path: &str,
        query_params: Option<&[&str]>,
    ) -> Result<Fs123Response> {
        let url_path = build_url(&self.proto, function, path, query_params);
        let full_url = format!("http://{}:{}{}", self.host, self.port, url_path);

        let response = self.agent.get(&full_url).call().map_err(|e| match e {
            ureq::Error::Status(status, response) => {
                let body = response.into_string().unwrap_or_default();
                Fs123Error::HttpError {
                    status,
                    message: body,
                }
            }
            ureq::Error::Transport(t) => Fs123Error::ConnectionFailed(t.to_string()),
        })?;

        // Read body
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(Fs123Error::IoError)?;

        // Parse netstring response
        let fields = parse_response(&body);

        let response = Fs123Response { fields };

        // Check errno
        if let Some(errno) = response.errno() {
            if errno != 0 {
                return Err(Fs123Error::FilesystemError {
                    errno,
                    message: format!("Operation failed for {}", path),
                });
            }
        }

        Ok(response)
    }

    /// Send a request and return raw response without checking errno.
    /// Useful when you need to handle errno yourself.
    pub fn request_raw(
        &self,
        function: Fs123Function,
        path: &str,
        query_params: Option<&[&str]>,
    ) -> Result<Fs123Response> {
        let url_path = build_url(&self.proto, function, path, query_params);
        let full_url = format!("http://{}:{}{}", self.host, self.port, url_path);

        let response = self.agent.get(&full_url).call().map_err(|e| match e {
            ureq::Error::Status(status, response) => {
                let body = response.into_string().unwrap_or_default();
                Fs123Error::HttpError {
                    status,
                    message: body,
                }
            }
            ureq::Error::Transport(t) => Fs123Error::ConnectionFailed(t.to_string()),
        })?;

        // Read body
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(Fs123Error::IoError)?;

        // Parse netstring response
        let fields = parse_response(&body);

        Ok(Fs123Response { fields })
    }

    /// Get the host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Get the port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the protocol version.
    pub fn proto(&self) -> &str {
        &self.proto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url_basic() {
        let url = build_url("7.2", Fs123Function::Stat, "/test/file.txt", None);
        assert_eq!(url, "/fs123/7/2/a/test/file.txt");
    }

    #[test]
    fn test_build_url_with_query() {
        let url = build_url("7.3", Fs123Function::Read, "/file", Some(&["128", "0"]));
        assert_eq!(url, "/fs123/7/3/f/file?128;0");
    }

    #[test]
    fn test_build_url_empty_path() {
        let url = build_url("7.2", Fs123Function::Statvfs, "/", None);
        assert_eq!(url, "/fs123/7/2/s");
    }

    #[test]
    fn test_build_url_special_chars() {
        let url = build_url("7.2", Fs123Function::Stat, "/path with spaces/file", None);
        assert!(url.contains("path%20with%20spaces"));
    }

    #[test]
    fn test_parse_basic_url() {
        let req = parse_url("/fs123/7/3/a/foo/bar").unwrap();
        assert_eq!(req.major_version, 7);
        assert_eq!(req.minor_version, 3);
        assert_eq!(req.function, Fs123Function::Stat);
        assert_eq!(req.path, "/foo/bar");
        assert!(req.selector.is_empty());
    }

    #[test]
    fn test_parse_with_selector() {
        let req = parse_url("/sel/ector/fs123/7/3/f/path").unwrap();
        assert_eq!(req.selector, vec!["sel", "ector"]);
        assert_eq!(req.function, Fs123Function::Read);
        assert_eq!(req.path, "/path");
    }

    #[test]
    fn test_parse_with_query() {
        let req = parse_url("/fs123/7/3/f/file?128;0").unwrap();
        assert_eq!(req.query_params, vec!["128", "0"]);
    }

    #[test]
    fn test_parse_url_encoded_query() {
        let req = parse_url("/fs123/7/3/d/dir?64;foo%20bar").unwrap();
        assert_eq!(req.query_params, vec!["64", "foo bar"]);
    }

    #[test]
    fn test_parse_default_minor_version() {
        let req = parse_url("/fs123/7/a/file").unwrap();
        assert_eq!(req.major_version, 7);
        assert_eq!(req.minor_version, 0);
        assert_eq!(req.function, Fs123Function::Stat);
    }

    #[test]
    fn test_parse_root_path() {
        let req = parse_url("/fs123/7/3/a").unwrap();
        assert_eq!(req.path, "/");
    }

    #[test]
    fn test_parse_missing_sigil() {
        assert!(parse_url("/foo/bar/7/3/a/path").is_err());
    }

    #[test]
    fn test_parse_missing_version() {
        assert!(parse_url("/fs123/").is_err());
    }

    #[test]
    fn test_parse_query_two_params() {
        let req = parse_url("/fs123/7/3/f/file?128;0").unwrap();
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.query_params[0], "128");
        assert_eq!(req.query_params[1], "0");
    }

    #[test]
    fn test_parse_query_three_params() {
        let req = parse_url("/fs123/7/3/x/file?128;user.xattr;").unwrap();
        assert_eq!(req.query_params.len(), 3);
        assert_eq!(req.query_params[0], "128");
        assert_eq!(req.query_params[1], "user.xattr");
        assert_eq!(req.query_params[2], "");
    }

    #[test]
    fn test_parse_query_single_param() {
        let req = parse_url("/fs123/7/3/d/dir?64").unwrap();
        assert_eq!(req.query_params.len(), 1);
        assert_eq!(req.query_params[0], "64");
    }

    #[test]
    fn test_parse_query_empty() {
        let req = parse_url("/fs123/7/3/a/file?").unwrap();
        assert_eq!(req.query_params.len(), 1);
        assert_eq!(req.query_params[0], "");
    }

    #[test]
    fn test_parse_query_trailing_semicolon() {
        let req = parse_url("/fs123/7/3/d/dir?64;").unwrap();
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.query_params[0], "64");
        assert_eq!(req.query_params[1], "");
    }

    #[test]
    fn test_parse_query_url_encoding() {
        let req = parse_url("/fs123/7/3/d/dir?64;foo%20bar%2Fbaz").unwrap();
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.query_params[0], "64");
        assert_eq!(req.query_params[1], "foo bar/baz");
    }

    #[test]
    fn test_parse_query_special_chars() {
        let req = parse_url("/fs123/7/3/x/file?128;user.test%40attr;").unwrap();
        assert_eq!(req.query_params.len(), 3);
        assert_eq!(req.query_params[0], "128");
        assert_eq!(req.query_params[1], "user.test@attr");
        assert_eq!(req.query_params[2], "");
    }

    #[test]
    fn test_parse_no_query() {
        let req = parse_url("/fs123/7/3/a/file").unwrap();
        assert_eq!(req.query_params.len(), 0);
        assert!(req.query_params.is_empty());
    }

    #[test]
    fn test_parse_file_read_params() {
        let req = parse_url("/fs123/7/3/f/path/to/file?128;0").unwrap();
        assert_eq!(req.function, Fs123Function::Read);
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.query_params[0], "128");
        assert_eq!(req.query_params[1], "0");
    }

    #[test]
    fn test_parse_directory_params() {
        let req = parse_url("/fs123/7/3/d/mydir?64;lastfile").unwrap();
        assert_eq!(req.function, Fs123Function::Readdir);
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.query_params[0], "64");
        assert_eq!(req.query_params[1], "lastfile");
    }
}
