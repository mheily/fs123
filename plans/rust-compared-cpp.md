# Behavioral Differences: C++ vs Rust fs123 Server

This document compares the legacy C++ fs123 server (`legacy/lib/fs123server.cpp`) with the Rust `fs123-server` crate to identify undocumented behaviors and implementation gaps.

## 1. Missing HTTP Headers in Rust

| Header | C++ Server | Rust Server |
|--------|-----------|-------------|
| `fs123-trsum` | Sends 32-hex-digit threeroe hash of body for GET requests | Missing |
| `Content-Type: application/octet-stream` | Explicit | Relies on framework default |
| `ETag` | Uses format `"<etag64>"` with optional mangling | Builder exists but handlers don't set it |
| `Content-encoding: fs123-secretbox` | For encrypted responses | No encryption support |

**Impact**: Clients relying on `fs123-trsum` for integrity checking won't work with the Rust server.

---

## 2. If-None-Match / 304 Not Modified Responses

**C++ Server** (`fs123server.cpp:603-615, 1084-1091`):
- Parses `If-None-Match` header
- Tracks `inm64` (demangled ETag value)
- Supports `not_modified_reply()` returning HTTP 304
- Increments `reply_304s` counter

**Rust Server**: No conditional request handling at all. Every request returns a full response.

---

## 3. Protocol 7.2 Backward Compatibility

**C++ Server** (`fs123server.cpp:783-790, 1076-1080, etc.`):
- Supports both 7.2 and 7.3 protocols
- For 7.2: Uses HTTP headers (`fs123-errno`, `fs123-estalecookie`, `fs123-nextoffset`)
- For 7.3: Uses netstring key-value pairs in body

**Rust Server**: Only supports 7.3 format. Older clients using 7.2 won't work.

---

## 4. Response Body: Missing `req123` Key

**C++ Server** (`fs123server.cpp:598`):
```cpp
req->kvpairs.emplace_back(FS123_REQUEST, where); // adds "req123" key
```
Contains the original request URL from sigil to end for debugging/logging.

**Rust Server**: Does not include `req123` key in responses.

---

## 5. Path Validation

**C++ Server** (`fs123server.cpp:172-193`):
```cpp
void validate_path(str_view pi){
    if(pi.front() != '/') httpthrow(400, "path must start with a /");
    if(pi.find('\0') != std::string::npos) httpthrow(400, "path may not contain NUL");
    if(pi.find("/../") != std::string::npos) httpthrow(400, "path may not contain /../");
    if(endswith(pi, "/..")) httpthrow(400, "path may not end with /..");
}
```

**Rust Server**: No path validation for directory traversal or NUL bytes.

---

## 6. HTTP Method Handling

**C++ Server** (`fs123server.cpp:580-587`):
- Only accepts GET and HEAD for fs123 functions (not `/p`)
- Returns 403 for unsupported methods

**Rust Server**: No explicit method checking--relies on actix-web routing.

---

## 7. Server Statistics (`/n` endpoint)

**C++ Server** returns actual runtime statistics:
```
requests, reply_bytes, INM_requests, a_requests, f_requests, d_requests,
l_requests, x_requests, s_requests, n_requests, p_requests,
reply_200s, reply_304s, reply_others
```

**Rust Server** (`handlers.rs:251-266`): Returns only config info:
```
fs123-server: <version>
backend: <description>
max_age: <value>
stale_while_revalidate: <value>
```

---

## 8. Secretbox Encryption

**C++ Server**: Full support for `Content-encoding: fs123-secretbox`:
- Checks `Accept-Encoding` header
- Encrypts response bodies
- Supports encrypted request paths (`/e/` function)
- ETag mangling with encryption key

**Rust Server**: No encryption support whatsoever.

---

## 9. Validator Handling for `/d` (Directory) Responses

**C++ Server** (`fs123server.cpp:1214`): Passes `etag64` to `common_reply200()` for `/d`:
```cpp
common_reply200(cc, etag64);
```

**Rust Server** (`handlers.rs:151`): Does not set ETag for directory responses.

---

## 10. Maximum Reply Size Enforcement

**C++ Server** (`fs123server.cpp:660-661, 677-678, 700-701`):
```cpp
if(lenkib > max_reply_size/1024)
    httpthrow(400, "/f requested length too large: ...");
```
Enforces `max_reply_size = 1025 * 1024` bytes.

**Rust Server**: No maximum size enforcement on request parameters.

---

## 11. TCP/Connection Tuning

**C++ Server** (`fs123server.cpp:403-412`):
- `TCP_NODELAY` option support
- `bufferevent_set_max_single_write()` for incast collapse workaround

**Rust Server**: Relies on actix-web defaults.

---

## 12. Error Response Format

**C++ Server** (`fs123server.cpp:843-858`):
- Clears any partial headers/content before sending error
- Includes nested exception `what()` messages in body
- Returns appropriate HTTP status codes from exceptions

**Rust Server** (`response.rs:143-156`): Simple status code mapping with plain message.

---

## Summary of Critical Gaps

### High Priority
1. **`fs123-trsum` header** - Required for integrity checking
2. **ETag/If-None-Match support** - Needed for efficient caching
3. **Path validation** - Security concern (directory traversal)
4. **Maximum request size validation** - DoS protection

### Medium Priority
5. **Protocol 7.2 support** - Required for backward compatibility with older clients
6. **`req123` key** - Used for debugging/correlation
7. **Request statistics** - Operational monitoring

### Lower Priority (Feature Parity)
8. **Secretbox encryption** - Only needed if encryption is required
9. **TCP tuning options** - Performance optimization
10. **HEAD method handling** - May be handled by framework