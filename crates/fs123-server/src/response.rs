/// Response building utilities for fs123 protocol

use actix_web::HttpResponse;
use fs123_core::{netstring_encode, netstring_encode_str};

/// Builder for fs123 protocol responses
pub struct Fs123ResponseBuilder {
    errno: i32,
    content: Option<Vec<u8>>,
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

    /// Build the HTTP response with netstring-encoded body
    pub fn build(self) -> HttpResponse {
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

            // Add validator if present
            if let Some(validator) = self.validator {
                body.extend_from_slice(&netstring_encode_str("validator"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&validator.to_string()));
                body.push(b'\n');
            }

            // Add estalecookie if present
            if let Some(estalecookie) = self.estalecookie {
                body.extend_from_slice(&netstring_encode_str("estalecookie"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&estalecookie.to_string()));
                body.push(b'\n');
            }

            // Add nextstart if present
            if let Some(nextstart) = self.nextstart {
                body.extend_from_slice(&netstring_encode_str("nextstart"));
                body.push(b' ');
                body.extend_from_slice(&netstring_encode_str(&nextstart));
                body.push(b'\n');
            }
        }

        // Build Cache-Control header
        let cache_control = if self.stale_while_revalidate > 0 {
            format!(
                "max-age={}, stale-while-revalidate={}",
                self.max_age, self.stale_while_revalidate
            )
        } else {
            format!("max-age={}", self.max_age)
        };

        let mut response = HttpResponse::Ok();
        response.insert_header(("Cache-Control", cache_control));

        // Add ETag if present
        if let Some(etag) = self.etag {
            response.insert_header(("ETag", format!("\"{}\"", etag)));
        }

        response.body(body)
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
}
