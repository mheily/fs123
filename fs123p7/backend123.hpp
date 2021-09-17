#pragma once

#include <core123/expiring.hpp>
#include <core123/threeroe.hpp>
#include <core123/diag.hpp>
#include <core123/throwutils.hpp>
#include <core123/datetimeutils.hpp>
#include <type_traits>
#include <string>
#include <sys/stat.h>
#include <chrono>
#include <atomic>
#include <stddef.h>

using clk123_t = std::chrono::system_clock;

struct reply123{
    // *this is valid() iff eno is non-negative.
    // 
    // If eno is zero, content contains values from the server.
    // If eno is non-zero, content is empty.
    //
    // Cache-control metadata, and the fresh(), age() and ttl()
    // methods are usable in all cases, i.e., even if *this is
    // invalid.  If *this is invalid, fresh() is false, age() is
    // seconds-since-1970 and ttl is -age.
    //
    // Note that several members are marked 'obsolete/unused in the
    // 7.3 protocol' and are named blah72.  In 7.2 and earlier, this
    // information was carried in http headers and initialized by the
    // code in backend123_http.cpp.  In 7.3 this information is
    // carried in the content (which might be encrypted) and isn't
    // accessible to backends.  The members must remain in the
    // backend123 struct until we no longer support 7.2.  (See the
    // comments near proto_minor, below, and in app_mount.cpp).  But
    // they should only be used conditionally, e.g.,
    // if(backend123::proto_minor<3){
    //     ... code may use eno72, estale_cookie72, etc.
    // }
    // 
    int32_t magic;
    int32_t eno72;              // *almost* obsolete/unused in 7.3 protocol (see valid())
    clk123_t::time_point expires;
    uint64_t etag64;  
    clk123_t::time_point last_refresh;
    clk123_t::duration stale_while_revalidate;
    uint64_t estale_cookie72;     // obsolete/unused in 7.3 protocol
    int64_t chunk_next_offset72;  // obsolete/unused in 7.3 protocol
    // N.B.  chunk_next_meta and content_encoding are actually much
    // narrower than 16 bits.
    int16_t chunk_next_meta72;    // obsolete/unused in 7.3 protocol
    int16_t content_encoding;
    char content_threeroe[32];
    std::string content;
    // MAGIC history:
    //   - original value:  27182835
    //   - changed to 141421356 when we appended url/url_len/magic to file
    //     Note that this change wasn't strictly necessary.  Old cache-files
    //     were still readable, but a new magic number makes it
    //     much easier to revert the change, at the expense of a cache
    //     refresh for the beta testers.
    //   - changed to 314159265 when we added estale_cookie and content_threeroe
    //   - changed to 577215664 when we made estale_cookie uint64_t
    //   - changed to 618033989 when we added chunk_next_offset and chunk_next_meta
    //   - changed to 137035999 when we replaced last_modified with etag64.
    //   - changed to 915965594 when we dropped the struct stat sb.
    //   - changed to 495569519 when we dropped sbp.
    //   - changed to 223606797 when we added content_encoding.
    static const int32_t MAGIC = 223606797; // change me whenever the serialization changes
    enum {              // possible values of chunk_next_meta
        CNO_MISSING=0,  // HHCNO not in reply
        CNO_NOT_EOF,    // HHCNO in reply without "EOF" decorator
        CNO_EOF};       // HHCNO in reply with "EOF" decorator

    // Called in various places when the details aren't known yet.
    reply123() : 
        magic(MAGIC), eno72(-1), expires{}, etag64{}, last_refresh{},
	stale_while_revalidate{}, estale_cookie72{},
	chunk_next_offset72{-1}, chunk_next_meta72{CNO_MISSING}, content_encoding{}, content{}
    {
        fill_content_threeroe();
    }

    // Called in curl_handler::getreply when proto_minor==2
    reply123(int _eno72, uint64_t _esc, std::string&& _content, int16_t _content_encoding, time_t age, time_t max_age, uint64_t et64, time_t stale_while_reval):
        magic(MAGIC), eno72(_eno72), etag64{et64}, estale_cookie72{_esc},
	chunk_next_offset72{-1}, chunk_next_meta72{CNO_MISSING}, content_encoding(_content_encoding), content{_content}
    {
        if(eno72!=0 && estale_cookie72!=0)
            throw core123::se(EINVAL, "reply123 constructor with eno72!=0 && estale_cookie!=0.  This can't happen");
        set_times(age, max_age, stale_while_reval);
        fill_content_threeroe();
    }
 
    // Called in curl_handler::getreply when proto_minor>2
    reply123(std::string&& _content, int16_t _content_encoding, time_t age, time_t max_age, uint64_t et64, time_t stale_while_reval):
        magic(MAGIC), eno72{}, etag64{et64}, estale_cookie72{},
	chunk_next_offset72{-1}, chunk_next_meta72{CNO_MISSING}, content_encoding(_content_encoding), content{_content}
    {
        set_times(age, max_age, stale_while_reval);
        fill_content_threeroe();
    }

    // Called in begetattr when we get a reply from the attrcache.
    template<class Rep, class Period>
    reply123(std::string&& _content, int16_t _content_encoding, uint64_t _cookie, std::chrono::duration<Rep, Period> ttl):
        magic(MAGIC), eno72(0), etag64(0), estale_cookie72(_cookie),
        chunk_next_offset72{-1}, chunk_next_meta72{CNO_MISSING},
        content_encoding(_content_encoding),
        content(_content)
    {
        set_times(0, ttl, 0 /*stale_while_reval*/);
        fill_content_threeroe();
    }
        
    // We need to copy the reply123 in certain special circumstances,
    // but we don't want to "accidentally" do so.  So we delete the
    // copy-assignment operator and we make the copy-constructor
    // private.  But there's a public const copy() method that "can't"
    // be used accidentally.
    reply123& operator=(const reply123&) = delete;
    reply123 copy() const{
        return *this; // uses private copy-constructor
    }
    reply123(reply123&&) = default;
    reply123& operator=(reply123&&) = default;

    auto max_age() const{
        return expires - last_refresh;
    }

    auto age() const{
        return clk123_t::now() - last_refresh; // might be negative!!
    }

    auto ttl() const{
        return expires - clk123_t::now(); // might be negative!
    }

    // Note that valid() uses eno72 even when proto_minor is 3 or more.
    bool valid() const { return eno72>=0;}
    bool fresh() const { return valid() && clk123_t::now() < expires; }
private:
    reply123(const reply123&) = default; // see copy() above
    void fill_content_threeroe(){
        auto hd = core123::threeroe(content).hexdigest();
        ::memcpy(content_threeroe, hd.data(), 32);
    }

    // set_times's arguments are chosen so it's easy to call it while
    // looking at an http reply.  An http reply will have an 'age',
    // a max-age and a stale_while_reval, all in integer seconds (time_t).
    void set_times(time_t age, time_t max_age, time_t stale_while_reval_){
        set_times(age, std::chrono::seconds(max_age), stale_while_reval_);
    }

    template <class Rep, class Period>
    void set_times(time_t age, std::chrono::duration<Rep, Period> max_age, time_t stale_while_reval_){
        static_assert(std::is_signed<time_t>::value, "Uh oh.  Unsigned time_t?!  We'll have to be a lot more careful about subtraction");
	static auto _refresh = core123::diag_name("refresh");
        if(!valid())
            throw core123::se(EINVAL, "cannot set_times an invalid backend123");
	auto now = clk123_t::now();
	DIAGkey(_refresh, "now=" << core123::str(now) << " age=" << age << " max_age=" << max_age.count() << " swr=" << stale_while_reval_ << "\n");
        last_refresh = now - std::chrono::seconds(age);
        expires = last_refresh + max_age;
        stale_while_revalidate = std::chrono::seconds(stale_while_reval_);
	DIAGkey(_refresh, "last_refresh=" << core123::str(last_refresh) << " expires=" << core123::str(expires) << "\n");
    }
};

static const size_t reply123_pod_begin = offsetof(struct reply123, magic);
static const size_t reply123_pod_length = offsetof(struct reply123, content_threeroe) + sizeof(reply123::content_threeroe) - reply123_pod_begin;

struct req123{
    static std::atomic<int> default_stale_if_error;
    static std::atomic<int> default_past_stale_while_revalidate;
    std::string urlstem;
    int stale_if_error = default_stale_if_error.load();
    int past_stale_while_revalidate = default_past_stale_while_revalidate.load();
    // The remaining members: no_cache, etc., are set to
    // reasonable defaults by the ctor.  The caller can set them after
    // construction if desired.
    bool no_cache = false;
    // The only values of max_stale actually used is 0.
    // Should it be a bool instead?  And does
    // max-stale=0 really do what we want?  I.e., does it override the
    // stale-while-revalidate attribute of cached resources?  If the
    // request says max-stale=0 and the cached resource is stale by
    // less than swr, how does the cache behave??  We're hoping it
    // refreshes rather than returning the stale data, but I can't
    // find words to support that in the rfcs(!?).  In any case,
    // max-stale=0 is very different from no-cache.  The former will
    // accept cached but not-stale resources while the latter demands
    // a refresh, even for non-stale resources.
    std::optional<int> max_stale;
    // no_peer_cache breaks loops in the distributed cache backend.
    // The fact that we need a flag in the generic 'req' strongly
    // suggests a mis-design.
    bool no_peer_cache = false;
    req123() = delete;
    req123(const std::string& _urlstem) :
        urlstem(_urlstem)
    {}
    static std::atomic<unsigned long> cachetag;
    static req123 attrreq(const std::string& name);
    static req123 dirreq(const std::string& name, uint64_t ckib, const std::string& chunkstart);
    static req123 filereq(const std::string& name, uint64_t ckib, int64_t chunkstartkib);
    static req123 linkreq(const std::string& name);
    static req123 statfsreq(const std::string& name);
    static req123 xattrreq(const std::string& name, uint64_t chunksize,
			 const char *attrname = nullptr);
    static req123 statsreq();
};

// backend123 is an abstract base class.  Descendents are:
//   backend_http
//   diskcache
struct backend123{
    virtual ~backend123() {};
    // The exact semantics of refresh are confusing (probably
    // indicating poor design).
    //
    // It returns true if it replaced the contents of the reply123
    // with fresh data from upstream.  (E.g., a 200 OK)
    //
    // It returns false if the contents of reply is unchanged.
    //   (E.g., 304 Not Modified).
    //
    // It throws, leaving the reply123* in a valid but undefined state
    // if, for any reason, it is unable to obtain a 'satisfactory'
    // reply, where 'satisfactory' means either a valid() reply, or in
    // the case of an upstream error, a cached reply that is within
    // the stale-if-error window.  If it throws, the only things the
    // caller may do with the reply are destroy it or reassign to it
    // (think of it as though it had been std::move'ed).
    //
    // It should not return false if req.no_cache is true.  I.e., for
    // a no_cache request, a backend should fully refresh the data,
    // ignoring any "validators" that might be present elsewhere in
    // the request or reply.
    //
    // In practice, there are only two overrides of the virtual:
    //   diskcache::refresh always returns true (or throws).
    //   backend_http::refresh has INM/304 logic if the
    //     reply looks like it might be usable.  But it
    //     avoids INM/304 if no_cache is true.
    virtual bool refresh(const req123& req, reply123*) = 0;
    virtual std::string get_uuid() { throw std::runtime_error("get_uuid not overridden by derived class"); }
    virtual std::ostream& report_stats(std::ostream&) = 0;
    static std::string add_sigil_version(const std::string& urlpfx);
    // proto_minor is a dangerous hack.  It is initialized to
    // proto_minor_default.  It may be changed, but only *BEFORE* any
    // backend123-derived class is instantiated.  The code itself
    // supports only a limited number of permissible values, but that
    // set occasionally changes and is poorly enforced (in
    // app_mount.cpp)!  FWIW, the client-side code currently (July
    // 2021) supports *only* proto_minor=2 or 3.
    static int proto_minor;
    constexpr static int proto_minor_default = 3;
};

