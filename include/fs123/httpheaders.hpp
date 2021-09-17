#pragma once

#include <core123/scanint.hpp>

// In 7.2 and earlier, fs123-specific metatdata is sent in HTTP headers:
#define HHCOOKIE    "fs123-estalecookie"  // obsolete from 7.3 on
#define HHERRNO	    "fs123-errno"         // obsolete from 7.3 on
#define HHNO	    "fs123-nextoffset"    // obsolete from 7.3 on

// The threeroe sum is (in the language of RFC7231, section 3.3) a
// "payload header field".  I.e., it "describes the payload rather
// than the associated representation".  It stays in the HTTP
// header, even in 7.3.
#define HHTRSUM	    "fs123-trsum"

// In 7.3, fs123-specific data and metadata are in key-value pairs in
// the HTTP message body.  These are the keys:
#define FS123_ERRNO     "errno"        // required in all replies
#define FS123_CONTENT   "content"      // required in all replies
#define FS123_COOKIE    "estalecookie" // required in /a, /f and /d replies
#define FS123_VALIDATOR "validator"    // required in /a and /f replies
#define FS123_NEXTSTART "nextstart"    // opaque start-key for next /d request
#define FS123_REQUEST   "req123"       // the requested url from sigil to end-of-string

static const int fs123_protocol_major = 7;
static const int fs123_protocol_minor_min = 2;
static const int fs123_protocol_minor_max = 3;
// On the client side, also see proto_minor and proto_minor_default in backend123.[ch]pp

// parse_quoted_etag is used on server-side and client-side.  This
// seems like as good a place as any...

// parse_quoted_etag: parse a quoted ETag into a uint64.  Throws an
// invalid_argument if it doesn't parse cleanly into a uint64, or if
// it parses in a way that might be ambiguous (e.g., "0123" is a
// numerical match, but not a char-by-char match to "123").
inline
uint64_t parse_quoted_etag(core123::str_view et_sv) {
    // Ignore anything preceding the first ".  This incorrectly
    // permits bogus contents like: abcd"1234", but so what...
    auto qidx = et_sv.find('"');
    if(qidx >= et_sv.size()) // no " or ends with "
        throw std::invalid_argument("parse_quoted_etag:  no double-quote");
    if(et_sv[qidx+1] == '0') // ambiguous.  
        throw std::invalid_argument("parse_quoted_etag:  ambiguous leading 0");
    uint64_t et64;
    // rfc7232 says ETag can't contain whitespace.  If it does,
    // scanint<..,..,false> will throw.
    auto qidx2 = core123::scanint<uint64_t, 10, false>(et_sv, &et64, qidx+1);
    if(qidx2 >= et_sv.size() || et_sv[qidx2] != '"')
        throw std::invalid_argument("parse_quoted_etag:  no trailing double-quote");
    return et64;
 }
