//! Netstring encoding and decoding utilities.
//!
//! Netstring format: `<length>:<data>,`
//! Example: `5:hello,` encodes the string "hello"

use std::collections::HashMap;

/// Encode bytes as a netstring.
pub fn encode(data: &[u8]) -> Vec<u8> {
    let length = data.len();
    let mut result = format!("{}:", length).into_bytes();
    result.extend_from_slice(data);
    result.push(b',');
    result
}

/// Encode a string as a netstring.
pub fn encode_str(s: &str) -> Vec<u8> {
    encode(s.as_bytes())
}

/// Decode a netstring from bytes.
///
/// Returns `Some((decoded_data, bytes_consumed))` or `None` if invalid.
pub fn decode(data: &[u8]) -> Option<(&[u8], usize)> {
    // Find the colon
    let colon_pos = data.iter().position(|&b| b == b':')?;

    // Parse the length
    let length_str = std::str::from_utf8(&data[..colon_pos]).ok()?;
    let length: usize = length_str.parse().ok()?;

    // Check if we have enough data
    let start = colon_pos + 1;
    let end = start + length;
    if data.len() < end + 1 {
        return None;
    }

    // Check for trailing comma
    if data[end] != b',' {
        return None;
    }

    Some((&data[start..end], end + 1))
}

/// Decode a netstring as a UTF-8 string.
///
/// Returns `Some((decoded_string, bytes_consumed))` or `None` if invalid.
pub fn decode_str(data: &[u8]) -> Option<(String, usize)> {
    let (bytes, consumed) = decode(data)?;
    let s = std::str::from_utf8(bytes).ok()?;
    Some((s.to_string(), consumed))
}

/// Parse an fs123 response body containing netstring key-value pairs.
///
/// The response format is:
/// ```text
/// <key_netstring> <value_netstring>\n
/// ...
/// ```
pub fn parse_response(body: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut result = HashMap::new();
    let mut offset = 0;

    while offset < body.len() {
        // Skip whitespace and newlines
        while offset < body.len() && matches!(body[offset], b' ' | b'\n' | b'\r') {
            offset += 1;
        }

        if offset >= body.len() {
            break;
        }

        // Decode key
        let Some((key_bytes, key_consumed)) = decode(&body[offset..]) else {
            break;
        };
        offset += key_consumed;

        // Skip space between key and value
        while offset < body.len() && body[offset] == b' ' {
            offset += 1;
        }

        // Decode value
        let Some((value_bytes, value_consumed)) = decode(&body[offset..]) else {
            break;
        };
        offset += value_consumed;

        // Try to decode key as UTF-8
        if let Ok(key) = std::str::from_utf8(key_bytes) {
            result.insert(key.to_string(), value_bytes.to_vec());
        }

        // Skip newline after value
        while offset < body.len() && matches!(body[offset], b'\n' | b'\r') {
            offset += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode() {
        assert_eq!(encode(b"hello"), b"5:hello,");
        assert_eq!(encode(b""), b"0:,");
        assert_eq!(encode(b"a"), b"1:a,");
    }

    #[test]
    fn test_encode_str() {
        assert_eq!(encode_str("hello"), b"5:hello,");
    }

    #[test]
    fn test_decode() {
        assert_eq!(decode(b"5:hello,"), Some((&b"hello"[..], 8)));
        assert_eq!(decode(b"0:,"), Some((&b""[..], 3)));
        assert_eq!(decode(b"1:a,"), Some((&b"a"[..], 4)));
    }

    #[test]
    fn test_decode_invalid() {
        assert_eq!(decode(b"hello"), None); // No colon
        assert_eq!(decode(b"5:hel"), None); // Too short
        assert_eq!(decode(b"5:hello"), None); // Missing comma
        assert_eq!(decode(b"abc:hello,"), None); // Invalid length
    }

    #[test]
    fn test_decode_str() {
        assert_eq!(decode_str(b"5:hello,"), Some(("hello".to_string(), 8)));
    }

    #[test]
    fn test_parse_response() {
        let body = b"5:errno, 1:0,\n7:content, 11:hello world,\n";
        let result = parse_response(body);

        assert_eq!(result.get("errno"), Some(&b"0".to_vec()));
        assert_eq!(result.get("content"), Some(&b"hello world".to_vec()));
    }
}
