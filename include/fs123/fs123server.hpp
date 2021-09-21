#pragma once

// To create an fs123 server you must:
//   - construct an instance of fs123p7::server_options
//   - construct an instance of an object derived from fs123p7::handler
//   - call fs123p7::server_run() with the above as arguments.
//
// The first is easily accomplished from the command line with
// functions in core123/opts.hpp.  Look at main() in
// exportd_handler.cpp for inspiration.
//
// The second requires code.  You have to write implementations of the
// abstract member functions:
//
//   a(), d(), f(), l(), s(), x(), p() n().
//
// Each handler is required to do exactly one of:
//   - call the corresponding ?_reply function.
//   - call redirect_reply
//   - call not_modified_reply (only for d(), f() and p())
//   - call errno_reply
//   - call exception_reply
//
// Note that the public xxx_reply functions take a req::up i.e., a
// unique_ptr as their first argument.  This should be the same
// unique_ptr that was passed to the handler.  In order to call
// xx_reply, the unique_ptr must be std::moved, thereby reliniquishing
// control and leaving the caller's 'copy' in an undefined state.
// Consequently, the handler can't do anything with the req after
// calling an _reply function.

// The ?_reply function may be called either synchronously or
// asynchronously.  I.e., it may be called from within the handler
// itself, synchronously.  Or the handler can arrange that it be
// called asynchronously, some time after the handler returns, by
// another thread.  (See below for restrictions imposed by the
// version of libevent).

// The errno_reply returns a "valid" errno to the client in a normal,
// unexceptional, HTTP 200 reply.  E.g., ENOENT for an non-existent
// directory entry.  On the other hand, req::exception_reply() sends a
// reply with a non-200 error code (use core123::http_exception to
// control the error code).  The exception's what() will be sent as
// the body of the reply (it may show up in client-side logs).
// Finally, if the handler's unique_ptr argument goes out of scope
// before any of the reply methods are called, the req's destructor
// will call exception_reply, sending a HTTP 500 status reply.

// handler_base::f() f_reply() has been "optimized" to minimize memory
// copies when returning large buffers.  handler_base::f()'s 'buf'
// argument is a pointer to the buffer into which f()'s virtual
// implementation should place the file's contents.  f_reply() takes
// an 'nbytes' argument, telling it exactly how many bytes have been
// placed at 'buf'.

// handler_base::d() the API is convoluted because of the
// idiosyncratic FUSE readdir API.  The d(req, inm64, begin, offset,
// db) method takes 4 arguments.  req is standard, and inm64 has the
// same meaning as for f() and p() (see below).  'begin' and 'offset'
// are tricky.  If 'begin' is true, the request starts at the
// beginning of the directory, i.e., there is no 'seek offset' implied
// in the request.  Conversely, if begin is false, then the
// implementation should 'seek' to the given 'offset' before listing
// entries.  The meaning of 'seek' and 'offset' is at the discretion
// of the callee.  A properly functioning client will only request
// offsets equal to values that had been provided previously by
// the callee.
//
// After seeking to the specified offset, the implementation should
// read entries from the directory, and for each one, call one of
// the overloads of req->add_dirent().
//
//  add_dirent(const ::dirent& de, uint64_t estale_cookie) - may
//     be called if the a POSIX dirent of the entry is known.
//  add_dirent(str_view name, long offset, int type, uint64_t estale_cookie) -
//     may be called if the name, offset and type of the
//     entry is known.  The 'offset' is the place to 'seek' to
//     restore the read position to the position immediately
//     following this entry.  The type is one of DT_BLK, DT_CHR,
//     DT_DIR, DT_FIFO, DT_LINK, DT_REG, DT_SOCK or DT_UNKNOWN,
//     defined in <dirent.h>.
//
//  add_dirent returns a bool, which is true if and only if the entry
//  was successfully added to the db. In general, d() should loop
//  until either the directory's EOF is reached, or req->add_dirent()
//  returns false.

// redirect_reply - the 'location' argument is the full uri to be
// placed in the reply's Location: header.  A handler that calls
// redirect_reply will probably look at the req::uri field.

// The 'f', 'd' and 'p' functions take an inm64 argument, copied from
// the incoming request's If-None-Match header.  Their corresponding
// _reply functions take an etag64 argument.  The handler should
// expect that the incoming inm64 will be the same as the value of an
// etag64 sent with a previous reply.  If the current contents
// "matches" the incoming inm64 value, then the handler *may* choose
// to respond with not_modified_reply rather than {f,d,p}_reply.

// fs123's etags are a bit idiosyncratic: they are 64-bit unsigned
// integers, not (as in HTTP) quoted strings.  The 0 value has special
// meaning: handlers should interpret inm64=0 to mean "there was no
// If-None-Match header in the request" and they may set etag64=0 in
// {f,d,p}_reply to mean "do not include an Etag header with the
// reply".

// Http status values, e.g., 200, 400, 502, etc.
//   - if a handler returns a reply the caller will assemble it
//     into an http reply with status 200 or 304 depending on
//     the value of not_modified.

// The 'p()' function is a 'passthru'.  The path and the query
// are passed through uninterpreted and unmodified.  The handler
// can do anything it wants with it (including defining its own
// 'sub-protocol' for implementing updates, etc.  The 'method'
// argument is an 'enum evhttp_command_type' with values:
//   EVHTTP_REQ_{GET, POST, HEAD, PUT, DELETE, OPTIONS, TRACE, CONNECT, PATCH}
// It's entirely up to the handler which (if any) of these to support.

// The 'logger' callback is for logging.  The 'const char* date'
// argument is the "Date" header formatted by the underlying http
// library.  Unfortunately, it has both too much information (day of
// week) and too little (no sub-second resolution), but on the plus
// side - it doesn't add many microseconds of formatting overhead to
// every request.

// Shared-key encryption is enabled by the --sharedkeydir option.  In
// general, if --sharedkeydir is specified, then encryption is
// *required*.  I.e., only encrypted requests will be accepted and
// only encrypted replies will be sent.  Unencrypted requests and
// requests that do not have an 'Accept-encoding' header that permits
// a reply with Content-encoding:fs123-secretbox will be rejected with
// a 406 Not Acceptable.  Note that the server library takes a
// hands-off approach to /p requests, so this rule DOES NOT apply to
// /p requests.  Unencrypted /p requests are permitted and replies are
// never re-encoded by the library, even if --sharedkeydir is
// specified.
//
// This rule is also relaxed by setting the server option:
//   --accept-plaintext-requests
// which allows plaintext (i.e., int /e/<ncrypted>) URLs even
// when the --sharedkeydir is specified.

// Short-circuiting HEAD requests is up to the handler.  I.e., the
// handler *may* look at the req::method field, and call f_reply,
// d_reply or p_reply with empty data arguments.  Even if the handler
// does populate the data arguments, the server will not send content
// in reply to a HEAD request.

// Versioning??  If it's implemented as a library, with a header file
// and a .a, then API/ABI versioning means that library users 
// must be careful to #include a header file that corresponds to
// the .a or .so file they link with.  The usual -Wl,-rpath conventions
// apply for .so.  If it's a plugin, we'll need some explicit versioning
// to make sure plugins "match" the program that dlopen's them.

// Asynchronous handlers:

//   Handler callbacks that call xxx_reply before returning are called
//   "synchronous".  If all the callbacks in a given handler are
//   always synchronous, then the handler itself is called "strictly
//   synchronous".
//
//   fs123server supports** handlers that are not strictly
//   synchronous, but doing so requires some version-dependent checks
//   and workarounds.  Therefore, handlers are required to provide a
//   boolean-valued member function: strictly_synchronous() that tells
//   the server whether handlers are guaranteed to be synchronous.
//
//   Synchronous handlers are easier to write and reason about.  A
//   synchronous handler can be made asynchronous by wrapping it with
//   'tp_handler' - a threadpool-based wrapper that farms out
//   synchronous handler invocations to an elastic_threadpool (see
//   core123/elasticthreadpool.hpp).  Lifetimes of wrapped and
//   wrapping objects are tricky in a way that suggests the API could
//   be improved, but something like this should work:

//    int threadpoolmax, threadpoolidle; // e.g., set by cmd-line --options
//    ...
//    my_handler_t my_handler(...);
//    std::optional<fs123p7::tp_handler<my_handler_t>> tph;
//    fs123p7::handler_base* handler_base_ptr = &my_handler;
//    if(threadpoolmax){
//        // Construct the tp_handler here, but don't
//        // destroy it until the outer block closes.
//        tph.emplace(gopts.threadpoolmax, gopts.threadpoolidle, my_handler);
//        handler_base_ptr = &tph.value();
//    }
//    fs123p7::server server(sopts, *handler_base_ptr);

//   ** NOTE - bugs in libevent through version 2.1.8 prevent the use
//   of asynchronous handlers.  If libfs123 is compiled and linked
//   with libevent-2.1.8 or earlier, the fs123::server constructor
//   will throw a runtime_error if the handler is not
//   'strictly_synchronous'.  Note that the libevent in many current
//   (as of Feb 2020) distros is 2.1.8 or earlier.  To use asynchronous
//   handlers, it may be necessary to use a very new distro (e.g.,
//   alpine-3.12, ubuntu-focal) or to build a standalone libevent
//   from sources.
//
// Exceptions
//
//   In general, the handler::xx() member functions should not throw.
//   Instead of throw-ing, if they encounter problems (bad arguments,
//   errors from underlying infrastructure, etc.), they *should* call
//   fs123p7::exception_reply which will sastify the client with an
//   appropriate reply.  If a handler::xx() member function throws,
//   the server will catch the thrown object and 'complain' to the
//   server's error log.  It will not (in the catch block) call
//   exception_reply.
//
//   However, if the 'fs123p7::req' owned by the unique_ptr passed to
//   the handler::xx() member function is destroyed for any reason
//   (e.g., the unique_ptr goes out-of-scope), before one of its
//   xx_reply member functions is called, then its destructor calls
//   exception_reply.  This is an "exceptional" code path and should
//   not be used for "normal" logic.  Normally, the handler::xx()
//   member functions should call a req->xxx_reply() themselves or
//   transfer ownership of the owned fs123p7::req and arrange that one
//   of its xxx_reply() members be called asynchronously in another
//   code path.
//
//   The req::reply_xxx methods do not throw.  Handlers are not
//   required to wrap them with try/catch.
//
// ISSUES
//
// - Should req::exception_reply and req::maybe_call_logger be
//   declared noexcept?  If a try{}catch(){complain} wrapper is
//   hopeless then maybe it's better to crash-and-burn on a noexcept
//   violation than to carry on only to crash-and-burn more
//   obscurely later.
//
// - option parsing is still pretty baroque.  Can we do better?
//
// - we still don't have any story at all for multi-selector/multi-handler.
//
// - leave termination-handling (SIGTERM, SIGHUP, etc.) to the caller.
//   
// - How "defensive" should we be about calling the handler or
//   trusting the arguments it gives to xxx_reply?  Note that it's
//   obviously possible for a malicious or buggy handler to cause a
//   segfault or other catastrophic failure.  Which then begs the
//   question of how much effort we should put into second-guessing.
//   We should avoid escalating the severity of mistakes.  E.g., a
//   handler mistakenly calling f_reply in response to a /d request
//   shouldn't segfault.  OTOH, there's nothing we can do if a handler
//   calls f_reply with a req* that has been free'ed or overruns
//   an allocated buffer.
//
// - Is a class derived from handler_base too-clever-by-half?  Isn't a
//   collection of std::functions actually more useful?  They can be
//   NULL, and therefore skipped.
//
// - The compulsive copy-avoidance is too-clever-by-half.  There are
//   too many str_views.  The padded_buffer is distressingly
//   complicated.

#include "fs123/sharedkeydir.hpp"
#include "fs123/content_codec.hpp"
#include "fs123/acfd.hpp"
#include "fs123/httpheaders.hpp"
#include <core123/strutils.hpp>
#include <core123/uchar_span.hpp>
#include <core123/str_view.hpp>
#include <core123/opt.hpp>
#include <core123/netstring.hpp>
#include <core123/unused.hpp>
#include <core123/elastic_threadpool.hpp>
#include <core123/threadpool.hpp>
#include <string>
#include <vector>
#include <list>
#include <cstdint>
#include <memory>
#include <utility>
#include <optional>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <dirent.h>

extern "C"{
//  Incomplete declarations of some stuff from libevent which we use internally,
//  but which is not meant to be exposed to callers.
struct evhttp_request;
struct event;
void event_free(event*);
struct event_base;
void event_base_free(event_base*);
struct evhttp;
void evhttp_free(evhttp*);
struct evhttp_bound_socket;
#ifdef _WIN32
#define EVUTIL_SOCKET_T intptr_t
#else
#define EVUTIL_SOCKET_T int
#endif
}

namespace fs123p7{

enum method_e { GET, POST, HEAD, PUT, DELETE, OPTIONS, TRACE, CONNECT,  // rfc2616
                PATCH, // rfc5789
                PROPFIND, PROPPATCH, MKCOL, LOCK, UNLOCK, COPY, MOVE // rfc4918
};

struct server;

struct async_reply_mechanism;

struct req{
    using up = std::unique_ptr<req>;
    // reqs are neither copy-able nor move-able.  The constructor is
    // private, so the only way they are constructed is with make_up
    // which returns a unique_ptr.  They stay where they were
    // constructed until they're destroyed.  These restrictions allow
    // us to check in the destructor for whether one of the 'reply'
    // members has been called, and if not, to call exception_reply.
    req(req&&) = delete;
    req(const req&) = delete;
    req& operator=(req&&) = delete;
    req& operator=(const req&) = delete;
    static std::unique_ptr<req> make_up(evhttp_request* evreq, server* _server, async_reply_mechanism* _arm){
        return std::unique_ptr<req>(new req(evreq, _server, _arm));
    }
    enum method_e method;
    // max_reply_size says how much space are we willing to
    // allocate for a reply.  Anything larger is rejected.  If it's a
    // request, e.g.,
    //    GET .../fs123/7/3/f/foo?1000000,0
    // the client gets a 400.  If the handler calls p_reply, n_reply,
    // x_reply, etc. with an excessively long 'body', it's a server-error
    // and the client gets a 500.
    //
    // Note that Linux kernel never requests more than 128k at a time.
    // We've experimented with increasing Fs123Chunk, but it's never
    // been advantageous.  So we make max_reply_size a bit more than 1MB.
    static const size_t max_reply_size = 1025 * 1024;
    // N.B.  The str_view members will typically "point" into data
    // that's owned by the evhr evhttp_request.  They are guaranteed
    // to remain valid until the one of the XXX_reply functions is
    // called with this as an argument.
    //
    // uri is the *original* uri, before any decryption or manipulation
    // are done.  I.e., uri might be:
    //   /sel/ec/tor/fs123/7/2/e/<base64-gibberish>
    // or:
    //   /sel/ec/tor/fs123/7/2/f/file/name?128,0
    core123::str_view uri;
    // The prefix is the part before the /FUNCTION designator.
    core123::str_view prefix;  // e.g., /sel/ec/tor/fs123/7/2/
    static constexpr int proto_major = fs123_protocol_major;
    int proto_minor;
    core123::str_view function; // e.g., f (not e, even if the original uri is /e/ncrypted!)
    // path_info is the CGI terminology for the part of the URI that
    // follows the script's name. In fs123, path_info is the part of
    // the URI that follows the /fs123/Major/Minor/Function and
    // precedes the query-string.  I.e., it's the part of the URI that
    // names the 'file' that is being asked about.  It is plaintext,
    // even if the original uri is /encrypted
    core123::str_view path_info; // e.g., /file/name
    // N.B. query is what comes *after* the optional '?'.
    // query.data() is nullptr if there was no query-string in the
    // uri.  Conversely, query.data() is non-null, and points to a
    // zero-length string if the query-string exists but is empty.
    core123::str_view query; // mostly of interest to p() handlers.
    // It's possible to reconstruct the decrypted uri as:
    //    prefix + function + path_info + (query->data()?"?":"") + query

    // Methods that may only be called from within a d() handler:
    bool add_dirent(core123::str_view name, int type, uint64_t esc);
    bool add_dirent(const ::dirent& de, uint64_t esc);
    size_t dirent_space_avail() const;
    // Method that may only be called from within a p() handler:
    void add_header(const std::string& name, const std::string& value);

    std::optional<std::string> get_header(const std::string& name);
    std::pair<std::string, uint16_t> get_peer() const;

    ~req();
    friend server; // so it can access http_cb
    // The public 'reply' API are friend functions that take a unique_pointer as their
    // first argument.  The caller is required to std::move the argument, relinquishing
    // ownership.  The request is effectively destroyed by calling xxx_reply;
    friend void exception_reply(up th, const std::exception& e) { th->exception_reply(e); }
    friend void errno_reply(up th, int fs123_errno,  const std::string& cc) { th->errno_reply(fs123_errno, cc); }
    friend void not_modified_reply(up th, const std::string& cc)  { th->not_modified_reply(cc); }
    friend void redirect_reply(up th, const std::string& location, const std::string& cc={}) { th->redirect_reply(location, cc); }
    friend void a_reply(up th, const struct stat& sb, uint64_t content_validator, uint64_t esc, const std::string& cc){
        th->a_reply(sb, content_validator, esc, cc); }
    friend void d_reply(up th, const std::string& nextstart, uint64_t etag64, uint64_t esc, const std::string& cc){
        th->d_reply(nextstart, etag64, esc, cc); }
    friend void f_reply(up th, size_t nbytes, uint64_t content_validator, uint64_t etag64, uint64_t esc, const std::string& cc){
        th->f_reply(nbytes, content_validator, etag64, esc, cc); }
    friend void l_reply(up th, const std::string& target, const std::string& cc){
        th->l_reply(target, cc); }
    friend void s_reply(up th, const struct statvfs& sv, const std::string& cc){
        th->s_reply(sv, cc); }
    friend void x_reply(up th, const std::string& xattr, const std::string& cc){
        th->x_reply(xattr, cc); }
    friend void n_reply(up th, const std::string& body, const std::string& cc){
        th->n_reply(body, cc); }
    friend void p_reply(up th, const std::string& body, uint64_t etag64, const std::string& cc){
        th->p_reply(body, etag64, cc); }
private:
    // The constructor and the remaining fields are "private".  They're
    // here so that the request-parser can communicate them to the
    // reply-sender.
    req(evhttp_request* evreq, server* _server, async_reply_mechanism* _arm);
    static void http_cb(evhttp_request* evreq, void *vserver);
    static void parse_and_handle(std::unique_ptr<fs123p7::req> req);
    const size_t secretbox_padding = 32; // command line option??
    const size_t secretbox_leadersz = sizeof(fs123_secretbox_header) + crypto_secretbox_MACBYTES;

    int16_t accept_encoding; // one of content_codec::CE_{IDENT,FS123_SECRETBOX,UNKNOWN}.  Should be an enum.
    evhttp_request* evhr;
    size_t requested_len; // so we don't exceed it when replying.  Only set in reqs for f() handlers.
    std::unique_ptr<char, decltype(std::free)*> decoded_path_up{nullptr, ::free}; // returned by evhttp_uridecode.  We must free.
    std::string decode64; // the result of base64-decode of the path *iff* the request was /e
    std::string envelope_sid; // the sid *iff* the request was a /e
    core123::str_view inm; // empty unless there was an If-Not-Modified header
    core123::uchar_blob blob;
    core123::padded_uchar_span buf;    // the body of the http reply (padded so we can prepend and append to it in-place)
    server& svr;
    async_reply_mechanism *arm;
    bool replied;
    bool synchronous_reply = false;
    std::vector<std::pair<std::string, std::string>> kvpairs;
    bool may_use_secrets() const;
    void common_reply200(const std::string& cc, uint64_t etag64 = 0);
    void encrypt_and_send200(const std::string& cc, uint64_t etag64);
    void log_and_send_destructively(int status);  // N.B.  *this is unusable after this!
    void maybe_call_logger(int status);
    std::string maybe_encode_content();
    static const size_t final_netstring_bytes = 2; // ",\n" that closes the value of the content kv-pair
    void allocate_pbuf(size_t sz){
        static const size_t fs123_max_headersz = 1024;
        if(blob)
            throw std::logic_error("allocate_pbuf called twice.  Definitely a logic error");
        if(sz > max_reply_size)
            throw std::runtime_error(core123::fmt("allocate_pbuf too large: %zd > %zd", sz, max_reply_size));
        blob = core123::uchar_blob(secretbox_leadersz + fs123_max_headersz + sz + secretbox_padding + final_netstring_bytes);
        buf = core123::padded_uchar_span(blob, secretbox_leadersz + fs123_max_headersz, 0);
    }
    void copy_to_pbuf(core123::str_view s){
        allocate_pbuf(s.size());
        buf = buf.append(s);
    }
    // How much space is "available" at the 'front' and 'back' of buffer - NOT counting
    // the space we pre-allocated for the secretbox leader and padding
    size_t buf_content_avail_back() const {
        if(buf.avail_back() < secretbox_padding + final_netstring_bytes)
            throw std::out_of_range("req::buf_content_avail_back: negative available space");
        return buf.avail_back() - secretbox_padding - final_netstring_bytes;
    }
    size_t buf_content_avail_front() const {
        if(buf.avail_front() < secretbox_leadersz)
            throw std::out_of_range("req::buf_content_avail_front: negative available space");
        return buf.avail_front() - secretbox_leadersz;
    }
    void internal_exception(const std::exception& e);
    // The reply methods are private.  There are friend versions that take
    // a up 'this' argument that are public.
    void exception_reply(const std::exception& e);
    void errno_reply(int fs123_errno,  const std::string& cc);
    void not_modified_reply(const std::string& cc);
    void redirect_reply(const std::string& location, const std::string& cc);
    void a_reply(const struct stat&, uint64_t content_validator, uint64_t esc, const std::string& cc);
    void d_reply(const std::string& nextstart, uint64_t etag64, uint64_t esc, const std::string& cc);
    void f_reply(size_t nbytes, uint64_t content_validator, uint64_t etag64, uint64_t esc, const std::string& cc);
    void l_reply(const std::string& target, const std::string& cc);
    void s_reply(const struct statvfs&, const std::string& cc);
    void x_reply(const std::string& xattr, const std::string& cc);
    void n_reply(const std::string& body, const std::string& cc);
    void p_reply(const std::string& body, uint64_t etag64, const std::string& cc);
};

struct handler_base{
    virtual bool strictly_synchronous() = 0;
    virtual void a(req::up) = 0;
    virtual void d(req::up, uint64_t inm64, std::string /*start*/) = 0;
    virtual void f(req::up, uint64_t inm64, size_t len, uint64_t offset, void *buf) = 0;
    virtual void l(req::up) = 0;
    virtual void s(req::up req) = 0;
    virtual void x(req::up req, size_t /*len*/, std::string /*name*/){
        errno_reply(std::move(req), ENOTSUP, "max-age=86400,stale-while-revalidate=864000");
    }
    virtual void p(req::up req, uint64_t /*etag64*/, std::istream& /*in*/){
        errno_reply(std::move(req), ENOTSUP, "max-age=86400,stale-while-revalidate=864000");
    }
    virtual void n(req::up req) {
        n_reply(std::move(req), {}, "max-age=30,stale-while-revalidate=30");
    }
    virtual void logger(const char* /*remote*/, method_e /*method*/, const char* /*uri*/, int /*status*/, size_t /*length*/, const char* /*date*/){
    }
    virtual ~handler_base(){}
};
              
template <typename H>
class tp_handler : public handler_base{
    core123::elastic_threadpool<void> tp;
    H& h;
public:
    tp_handler(size_t threadpool_max, size_t threadpool_idle, H& h_) :
        tp(threadpool_max, threadpool_idle),
        h(h_){

        if(!h.strictly_synchronous())
            throw std::runtime_error("tp_handler can only wrap synchronous handlers");
    }
    bool strictly_synchronous() override { return false; }
    ~tp_handler(){}
    void a(req::up req) override {
        tp.submit([=, p=req.release()](){
                      h.a(req::up(p));
                  });
    }
    void d(req::up req, uint64_t inm64, std::string start) override {
        tp.submit([=, p=req.release()](){
                      h.d(req::up(p), inm64, start);
                  });
    }
    void f(req::up req, uint64_t inm64, size_t len, uint64_t offset, void *buf) override {
        tp.submit([=, p=req.release()](){
                      h.f(req::up(p), inm64, len, offset, buf);
                  });
    }
    void l(req::up req) override {
        tp.submit([=, p=req.release()](){
                      h.l(req::up(p));
                  });
    }
    void s(req::up req) override {
        tp.submit([=, p=req.release()](){
                      h.s(req::up(p));
                  });
    }        
    void x(req::up req, size_t len, std::string name) override {
        tp.submit([=, p=req.release()](){
                      h.x(req::up(p), len, name);
                  });
    }
    void p(req::up req, uint64_t etag64, std::istream& in) override {
        tp.submit([=, &in, p=req.release()](){
                      h.p(req::up(p), etag64, in);
                  });
    }
    void n(req::up req) override {
        tp.submit([=, p=req.release()](){
                      h.n(req::up(p));
                  });
    }
    void logger(const char* remote, method_e method, const char* uri, int status, size_t length, const char* date) override {
        // DO NOT submit to threadpool!  The pointer lifetimes are not guaranteed past the return.
        // And in any case, we're already running in a thread in the pool.
        h.logger(remote, method, uri, status, length, date);
    }
};

#define ALLOPTS \
STD_OPTIONAL_OPTION(std::string, sharedkeydir, "path to directory containing shared secrets (pre-chroot!)"); \
OPTION(std::string, encoding_keyid_file, "encoding", "name of file containing the encoding secret. (if relative, then with respect to sharedkeydir, otherwise with respect to chroot)"); \
OPTION(uint64_t, sharedkeydir_refresh, 43200, "reread files in sharedkeydir after this many seconds"); \
OPTION(bool, accept_plaintext_requests, false, "if true, then unencrypted requests are allowed, even when secretbox encryption is enabled");\
OPTION(unsigned, nlisteners, 4, "run with this many listening processes");\
OPTION(std::string, bindaddr, "127.0.0.1", "bind to this address");\
STD_OPTIONAL_OPTION(uint16_t, port, "bind to this port.  If unspecified, an ephemeral port is chosen.  The port number in use is available via server::get_sockaddr_in.");\
STD_OPTIONAL_OPTION(double, exit_after_idle, "If specified, the server stops after this many seconds of idle time"); \
/* max_http_headers_size should be enough for typical headers */       \
OPTION(uint64_t, max_http_headers_size, 2000, "maximum bytes in incoming request HTTP headers");\
 /* max http_body_size can be small since fs123 has no incoming body */ \
OPTION(uint64_t, max_http_body_size, 500, "maximum bytes in incoming request HTTP body");\
/* if unspecified, libevent times out connections after 50 seconds.  \
 * To avoid 'AH01102: error reading status line' problems with httpd \
 * ProxyPass, it's necessary to make exportd's timeout longer than  \
 * httpd's.  httpd defaults to 60, but, of course, it's configurable. \
 * Also, a quiet server might get 'pinged' by external health-monitors at \
 * regular intervals, which can interact with the max_http_timeout. \
 * To avoid such issues, the default is a prime number \
 * of seconds, not particularly close to any "round" number of minutes or seconds. */  \
OPTION(uint64_t, max_http_timeout, 769, "http timeout on incoming request being complete");\
/* Calling back-to-back writes with sizes smaller than small multiples\
 * of MSS (TCP_MAXSEG) tickles the Nagle Algorithm which, in turn,\
 * interacts very badly with Delayed Ack resulting in                   \
 * multi-millisecond latencies on retrievals near the MSS size.  On     \
 * Linux, the localhost interface defaults to TCP_MAXSEG=65468.         \
 * Ethernet interfaces seem to default to 1448.  To avoid Nagle, we     \
 * want our outbound replies to go in a single write(), so if the       \
 * client is making requests with ckib of 128, then max_single_write should \
 * be at least 131072+maximum_http_header_size.  But we know of no       \
 * reason to try to "finesse" it.  Let's just make it big and            \
 * hopefully not worry about it ever again (famous last words).          \
 * N.B.  bufferevent_set_max_single_write was introduced in libevent 2.1.1-alpha \
 * Setting it when linked with an older libevent is a silent no-op.      \
 */ \
OPTION(uint64_t, max_single_write, 16*1024*1024, "maximum number of bytes in any single write to an http socket");\
/* setting TCP_NODELAY is another way to avoid the problems that arise  \
 * from the Nagle algorithm when write()s smaller than MSS.  Other\
 * alternatives are to adjust the MTU with ifconfig or the MSS with ip\
 * route.  The evhttp API in libevent doesn't give us a per-accept\
 * callback, so --tcp_nodelay does a setsockopt TCP_NODELAY for every\
 * request, even for keepalive connections, which seems wasteful.\
 * max_single_write is definitely preferred.                             \
 */ \
OPTION(bool, tcp_nodelay, false, "set TCP_NODELAY on accepted sockets"); \
OPTION(bool, libevent_debug, false, "direct libevent debug info to complain(LOG_DEBUG, ...) (this produces a lot of output)"); \
/* async_reply_mechanism is active only for handlers that are not strictly synchronous. \
 * It ensures that libevent functions are only called from the              \
 * same thread that's running the event loop.  It's useful only if \
 * libevent itself is not thread-safe.                             \
 */ \
OPTION(bool, async_reply_mechanism, false, "guarantee that evhttp_send_reply is called on the thread that's executing the event loop");

struct server_options {
#define OPTION(type, name, default, desc)        \
    type name
#define STD_OPTIONAL_OPTION(TYPE, NAME, DESC) std::optional<TYPE> NAME;
ALLOPTS
#undef OPTION
#undef STD_OPTIONAL_OPTION
    server_options(core123::option_parser& op){
#define OPTION(type, name, dflt, desc) \
        op.add_option(#name, core123::str(dflt), desc, core123::opt_setter(name))
#define STD_OPTIONAL_OPTION(TYPE, NAME, DESC) op.add_option(#NAME, {}, DESC, core123::opt_setter(NAME))
ALLOPTS
#undef OPTION
#undef STD_OPTIONAL_OPTION
#undef ALLOPTS
    }
};

using sig_cb_adapter_data = std::tuple<int, std::function<void(int, void*)>, void*>;
using http_cb_arg = std::tuple<server*, async_reply_mechanism*>;

struct server{
    server(const server_options&, handler_base&);
    void add_sig_handler(int signum, std::function<void(int, void*)>, void*);
    // server::set_signal_handlers is "like" fuse_set_signal_handlers:  it
    // arranges that INT, TERM, HUP and QUIT stop the server.
    void set_signal_handlers();
    void run();
    void stop();
    struct sockaddr_in get_sockaddr_in() const;
    std::string get_baseurl() const;
    ~server();
private:
    friend struct req;
    std::unique_ptr<const fs123p7::server_options> gopts;
    decltype(core123::make_autocloser((event_base*)nullptr, event_base_free)) ebac{nullptr, ::event_base_free};
    decltype(core123::make_autocloser((evhttp*)nullptr, evhttp_free)) ehac{nullptr, ::evhttp_free};
    decltype(core123::make_autocloser((event*)nullptr, event_free)) donecheck_ev{nullptr, ::event_free};
    std::unique_ptr<async_reply_mechanism> armup;
    std::optional<sharedkeydir> the_secret_manager;
    bool strictly_synchronous_handlers;
    fs123p7::handler_base& handler;
    struct evhttp_bound_socket* ehsock = nullptr;
    std::vector<event*> events2befreed;
    // N.B.  the adapter_data_ll grows whenever add_signal_handler is
    // called, but it never shrinks.
    std::list<sig_cb_adapter_data> sig_cb_adapter_data_ll;
    const long thread_done_delay_secs = 1;
    // machinery to shut everything down:
    static void donecheck_cb(EVUTIL_SOCKET_T, short, void *varg);
    using donecheck_cb_arg_t = std::tuple<event_base*, server*>;
    std::unique_ptr<donecheck_cb_arg_t> donecheck_cb_arg;
    std::atomic<bool> done{false};
    std::atomic<std::chrono::system_clock::time_point> time_of_last_request{std::chrono::system_clock::now()};
    std::chrono::system_clock::duration idle_timeout;

    // don't let the args passed to http_cb get destroyed until we're done with them.
    std::list<std::unique_ptr<http_cb_arg>> cbargs;

    void incast_collapse_workaround(evhttp_request *evreq);
    std::unique_ptr<async_reply_mechanism> setup_async_mechanism(struct event_base *eb);
    std::unique_ptr<async_reply_mechanism> setup_async(struct event_base *eb, struct evhttp* eh);
    void setup_evhttp(struct evhttp *eh, async_reply_mechanism* arm);
    void evhttp_bind_socket(struct event_base* eb, struct evhttp* eh); // called by secondary threads
};


} // namespace fs123p7

#include <core123/stats.hpp>
#define FS123_SERVER_STATISTICS \
  STATISTIC(requests) \
  STATISTIC(reply_bytes) \
  STATISTIC(INM_requests) \
  STATISTIC(a_requests) \
  STATISTIC(f_requests) \
  STATISTIC(d_requests) \
  STATISTIC(l_requests) \
  STATISTIC(x_requests) \
  STATISTIC(s_requests) \
  STATISTIC(n_requests) \
  STATISTIC(p_requests) \
  STATISTIC(reply_200s) \
  STATISTIC(reply_304s) \
  STATISTIC(reply_others)
#define STATS_STRUCT_TYPENAME server_stats_t
#define STATS_MACRO_NAME FS123_SERVER_STATISTICS
#include <core123/stats_struct_builder>
#undef FS123_SERVER_STATISTICS
extern server_stats_t server_stats;
