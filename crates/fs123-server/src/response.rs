/// Response building utilities for fs123 protocol

use actix_web::HttpResponse;
use fs123_core::{netstring_encode, netstring_encode_str};
use serde_json::Value;

/// Builder for fs123 protocol responses
pub struct Fs123ResponseBuilder {
    errno: i32,
    content: Option<Vec<u8>>,
    json_content: Option<Value>,
    validator: Option<u64>,
    estalecookie: Option<u64>,
    nextstart: Option<String>,
    max_age: u32,
    stale_while_revalidate: u32,
    etag: Option<String>,
}

impl Fs123ResponseBuilder {
    pub fn new() -> Self {
        Fs123ResponseBuilder {
            errno: 0,
            content: None,
            json_content: None,
            validator: None,
            estalecookie: None,
            nextstart: None,
            max_age: 300,
            stale_while_revalidate: 60,
            etag: None,
        }
    }

    pub fn errno(mut self, errno: i32) -> Self {
        self.errno = errno;
        self
    }

    pub fn content(mut self, content: Vec<u8>) -> Self {
        self.content = Some(content);
        self
    }

    pub fn content_str(mut self, content: &str) -> Self {
        self.content = Some(content.as_bytes().to_vec());
        self
    }

    pub fn json_content(mut self, value: Value) -> Self {
        self.json_content = Some(value);
        self
    }

    pub fn validator(mut self, validator: u64) -> Self {
        self.validator = Some(validator);
        self
    }

    pub fn estalecookie(mut self, cookie: u64) -> Self {
        self.estalecookie = Some(cookie);
        self
    }

    pub fn nextstart(mut self, nextstart: String) -> Self {
        self.nextstart = Some(nextstart);
        self
    }

    pub fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = seconds;
        self
    }

    pub fn stale_while_revalidate(mut self, seconds: u32) -> Self {
        self.stale_while_revalidate = seconds;
        self
    }

    pub fn etag(mut self, etag: String) -> Self {
        self.etag = Some(etag);
        self
    }

    fn cache_control_header(&self) -> String {
        if self.stale_while_revalidate > 0 {
            format!(
                "max-age={}, stale-while-revalidate={}",
                self.max_age, self.stale_while_revalidate
            )
        } else {
            format!("max-age={}", self.max_age)
        }
    }

    /// Build the HTTP response with netstring-encoded body (v7)
    pub fn build(self) -> HttpResponse {
        let cache_control = self.cache_control_header();
        let mut body = Vec::new();

        // Add errno key-value pair
        body.extend_from_slice(&netstring_encode_str("errno"));
        body.push(b' ');
        body.extend_from_slice(&netstring_encode_str(&self.errno.to_string()));
        body.push(b'\n');

        // Add content if present (and errno is 0)
        if self.errno == 0 {
            if let Some(content) = self.content {
                body.extend_from_slice(&netstring_encode_str("content"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode(&content));
                body.push(b'\n');
            }

            if let Some(validator) = self.validator {
                body.extend_from_slice(&netstring_encode_str("validator"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&validator.to_string()));
                body.push(b'\n');
            }

            if let Some(estalecookie) = self.estalecookie {
                body.extend_from_slice(&netstring_encode_str("estalecookie"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&estalecookie.to_string()));
                body.push(b'\n');
            }

            if let Some(nextstart) = self.nextstart {
                body.extend_from_slice(&netstring_encode_str("nextstart"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&nextstart));
                body.push(b'\n');
            }
        }

        let mut response = HttpResponse::Ok();
        response.insert_header(("Cache-Control", cache_control));

        // Add ETag if present
        if let Some(etag) = self.etag {
            response.insert_header(("ETag", format!("\"{}\"", etag)));
        }

        response.body(body)
    }

    /// Build a v8 JSON response (for /stat, /readdir, /readlink, /statvfs, /n endpoints)
    pub fn build_json(self) -> HttpResponse {
        let cache_control = self.cache_control_header();
        let json = if self.errno != 0 {
            serde_json::json!({ "errno": self.errno })
        } else {
            let mut obj = serde_json::json!({
                "errno": 0,
            });
            let map = obj.as_object_mut().unwrap();
            if let Some(content) = self.json_content {
                map.insert("content".to_string(), content);
            }
            if let Some(validator) = self.validator {
                map.insert("validator".to_string(), Value::Number(validator.into()));
            }
            if let Some(estalecookie) = self.estalecookie {
                map.insert("estalecookie".to_string(), Value::Number(estalecookie.into()));
            }
            if let Some(nextstart) = self.nextstart {
                map.insert("nextstart".to_string(), Value::String(nextstart));
            }
            obj
        };
        let mut response = HttpResponse::Ok();
        response.insert_header(("Cache-Control", cache_control));
        response.insert_header(("Content-Type", "application/json"));

        if let Some(etag) = self.etag {
            response.insert_header(("ETag", format!("\"{}\"", etag)));
        }

        response.json(json)
    }

    /// Build a v8 binary response (for /read, /getxattr, /listxattr endpoints).
    /// Metadata is conveyed in HTTP headers; body is raw bytes.
    pub fn build_binary(self) -> HttpResponse {
        let cache_control = self.cache_control_header();

        let mut response = HttpResponse::Ok();
        response.insert_header(("Cache-Control", cache_control));
        response.insert_header(("Content-Type", "application/octet-stream"));
        response.insert_header(("X-Fs123-Errno", self.errno.to_string()));

        if self.errno != 0 {
            if let Some(etag) = self.etag {
                response.insert_header(("ETag", format!("\"{}\"", etag)));
            }
            return response.body(Vec::new());
        }

        if let Some(validator) = self.validator {
            response.insert_header(("X-Fs123-Validator", validator.to_string()));
        }
        if let Some(estalecookie) = self.estalecookie {
            response.insert_header(("X-Fs123-Estalecookie", estalecookie.to_string()));
        }
        if let Some(etag) = self.etag {
            response.insert_header(("ETag", format!("\"{}\"", etag)));
        }

        response.body(self.content.unwrap_or_default())
    }

    /// Build an error response (4xx or 5xx)
    pub fn build_error(status_code: u16, message: &str) -> HttpResponse {
        let message_owned = message.to_string();
        match status_code {
            400 => HttpResponse::BadRequest().body(message_owned),
            401 => HttpResponse::Unauthorized().body(message_owned),
            403 => HttpResponse::Forbidden().body(message_owned),
            404 => HttpResponse::NotFound().body(message_owned),
            405 => HttpResponse::MethodNotAllowed().body(message_owned),
            501 => HttpResponse::NotImplemented().body(message_owned),
            502 => HttpResponse::BadGateway().body(message_owned),
            503 => HttpResponse::ServiceUnavailable().body(message_owned),
            _ => HttpResponse::InternalServerError().body(message_owned),
        }
    }
}

impl Default for Fs123ResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(0)
            .content_str("hello")
            .build();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_error_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(2) // ENOENT
            .build();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_with_validator() {
        let response = Fs123ResponseBuilder::new()
            .errno(0)
            .content_str("test")
            .validator(12345)
            .estalecookie(67890)
            .build();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_json_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(0)
            .json_content(serde_json::json!({"st_mode": 33188}))
            .validator(12345)
            .estalecookie(67890)
            .build_json();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_json_error_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(2)
            .build_json();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_binary_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(0)
            .content(vec![1, 2, 3])
            .validator(100)
            .estalecookie(200)
            .build_binary();

        assert_eq!(response.status(), 200);
    }

    #[test]
    fn test_binary_error_response() {
        let response = Fs123ResponseBuilder::new()
            .errno(2)
            .build_binary();

        assert_eq!(response.status(), 200);
    }
}
