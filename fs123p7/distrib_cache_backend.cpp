#include "distrib_cache_backend.hpp"
#include "fs123/httpheaders.hpp"
#include <core123/strutils.hpp>
#include <core123/diag.hpp>
#include <core123/scoped_timer.hpp>
#include <core123/non_null_or_throw.hpp>
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netdb.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <poll.h>
#include <sodium.h>

// N.B.  It's confusing.  Extensive comments are in distrib_cache_backend.hpp.

using namespace core123;
using namespace fs123p7;
using namespace std;

// DiagName=distrib_cache produces chatter about udp control messages,
// the comings and goings of peers, etc.  In "steady state", it should
// be O(#peers) messages per minute.
auto _distrib_cache = diag_name("distrib_cache");
// DiagName=distrib_cache_requests produces chatter about every
// request that passes through the cache.  It's *a lot* on a busy
// server.
auto _distrib_cache_requests = diag_name("distrib_cache_requests");
auto _shutdown = diag_name("shutdown");

distrib_cache_statistics_t distrib_cache_stats;

// distrib_cache_message:  encapsulate some of the details of sending, receiving
// and "parsing" udp messages.  It's still *very* raw, but maybe better than
// just having this code inline.  "Messages" are concatenations of NUL-terminated
// strings.  They're (currently) limited to 512 bytes.

// To send a message, we take a [begin, end) collection of
// string_view's and bundle them up for sendmsg, making sure to
// NUL-terminate each one.  We stick a version, 'scope' and a
// 'secretid' at the front and a binary timestamp and an hmac at the
// end.  The version=2 looks something like this:

//    '2' \0 scope \0 sid \0 part[0] \0 ... part[n-1] \0 tstamp(8) hmac(32)

// If the message is authenticated, then the sid is non-empty and the
// hmac is non-zero.  Otherwise, the sid is empty and the hmac is all
// zeros.  So in either case, it's a sequence of NUL-terminated strings
// followed by a 40 bytes of authentication.  The timestamp is used to
// reject "replay attacks".  

// To receive a message, we instantiate an empty distrib_cache_message
// and call its 'recv' method.  The recv method checks version, scope,
// timestamp and hmac, and if all are good, it fills the 'parts'
// member with str_views that point into a copy the received message.
//
// Note that recv returns a bool which is true when the incoming
// message has been successfully parsed into 'parts'.  When recv
// encounters an "expected" error, it calls 'complain' and returns
// false with 'parts' empty.  When it encounters an 'unexpected'
// error, it throws an exception and 'parts' is undefined.

struct distrib_cache_message{
    // VERSION is semantically just a string that either matches or doesn't.
    // It's easy to manage if we keep it numeric.
    static constexpr str_view VERSION{"2"};
    // Permitted timestamp skew (in milliseconds).  Should these be configurable??
    const int64_t skew_wide_window = 30000; // 30 seconds
    const int64_t skew_narrow_window = 2000; // 2 seconds

    std::array<char,512> data;
    std::vector<core123::str_view> parts;
    const distrib_cache_backend& dbe;
    distrib_cache_message(const distrib_cache_backend& _dbe):
        dbe(_dbe)
    {}
    template <typename ITER>
    void send(int sockfd, const struct sockaddr_in& dest, ITER b, ITER e);
    bool recv(int fd);
private:
    // Some methods for walking over the data.  wxxx members are for
    // writing, rxxx members are for reading.  Have we just
    // re-implemented rdbufs?
    char *_wptr = &data[0];
    char *wptr() const {
        return  _wptr;
    }
    size_t wlen() const {
        return _wptr - &data[0];
    }
    char* wptr(size_t need){
        if(wlen() + need > data.size())
            throw std::runtime_error("out of space");
        auto ret = _wptr;
        _wptr += need;
        return ret;
    }
    void wpush(str_view sv){
        DIAG(_distrib_cache, "wpush(" + std::string(sv) + ")");
        char* to = wptr(sv.size()+1);
        ::memcpy(to, sv.data(), sv.size());
        to[sv.size()] = '\0';
    }

    char *_rptr = &data[0];
    char *_rendptr = &data[0];
    char* rptr() const {
        return _rptr;
    }
    char* rendptr() const {
        return _rendptr;
    }
    str_view rpop() {
        if(_rptr >= _rendptr)
            throw std::runtime_error("rpop: past last word");
        char *nextnul = std::find(_rptr, _rendptr, '\0');
        str_view ret{_rptr, size_t(nextnul - _rptr)};
        _rptr = nextnul + 1;
        return ret;
    }

    void set_rptrs(size_t recvd){
        // Note that we haven't checked the version yet, but if there
        // aren't even enough bytes for a tstamp and an hmac, then
        // it doesn't matter what the version is.  Whatever
        // it is - it's not meant for *us*.
        if( recvd < sizeof(int64_t) + crypto_auth_BYTES + 1 )
            throw std::runtime_error("message too short");
        _rendptr = &data[recvd] - sizeof(int64_t) - crypto_auth_BYTES;
        if( _rendptr[-1] != '\0' )
            throw std::runtime_error("words don't end with NUL");
    }

    // use millisecond-resolution system-clock for timestamps.
    static int64_t now_millis(){
        using namespace std::chrono;
        return duration_cast<milliseconds>(system_clock::now().time_since_epoch()).count();
    }

};

template <typename ITER>
void
distrib_cache_message::send(int sockfd, const struct sockaddr_in& dest,
                 ITER b, ITER e){
    wpush(VERSION);
    wpush(dbe.scope);
    std::string sid;
    secret_sp key;
    if(dbe.secret_mgr){
        sid = dbe.secret_mgr->get_indirect_sid("multicast");
        DIAG(_distrib_cache, "send:  sid=" + sid);
        key = dbe.secret_mgr->get_sharedkey(sid);
        if(key->size() < crypto_auth_KEYBYTES) // crypto_auth_KEYBYTES == 32
            throw std::runtime_error("key found, but it's too short to be used in crypto_auth");
        DIAG(_distrib_cache, "send: key=" + quopri(str_view((char*)key->data(), crypto_auth_KEYBYTES)));
    }
    wpush(sid);
    while(b != e)
        wpush(*b++);
    int64_t tstamp = now_millis();
    ::memcpy(wptr(sizeof(tstamp)), &tstamp, sizeof(tstamp));
    char *hmac = wptr(crypto_auth_BYTES); // crypto_auth_BYTES == 32
    if(key){
        crypto_auth((unsigned char*)hmac, (unsigned char*)&data[0], hmac-&data[0], key->data());
    }else{
        ::memset(hmac, 0, crypto_auth_BYTES);
    }
    DIAG(_distrib_cache, "sendto(len=" + str(wlen()) + "): " + quopri(str_view{&data[0], wlen()}));
    core123::sew::sendto(sockfd, &data[0], wlen(), 0, (const sockaddr*)&dest, sizeof(dest));
}

bool distrib_cache_message::recv(int fd){
    // See above for the "format".

    // + check that we're not re-using *this
    using namespace core123;
    if(!parts.empty())
        throw std::logic_error("distrib_cache_messages::recv:  may only be called once");

    // + read a message.  check for os-level errors.
    // MSG_DONTWAIT may be superfluous because we've just poll'ed, but
    // it shouldn't do any harm, and protects if poll is subject to
    // "spurious" wakeups (can that happen?)
    auto recvd = ::recv(fd, data.data(), data.size(), MSG_DONTWAIT|MSG_TRUNC);
    if(recvd < 0){
        if(errno == EAGAIN){
            complain(LOG_WARNING, "udp_listener:  unexpected EAGAIN from recv(MSG_DONTWAIT)");
            return false;
        }
        throw se("recv(udp_fd) in udp_listener");
    }
    if(recvd == 0){
        complain(LOG_WARNING, "distrib_cache_messages::recv:  empty message");
        return false;
    }
    if(size_t(recvd) > data.size()){ // see MSG_TRUNC
        complain(LOG_WARNING, "distrib_cache_message::recv:  message is too long.  Treating as empty");
        return false;
    }

    DIAG(_distrib_cache, "recv(len=" + str(recvd) + "): " + quopri(str_view{&data[0], size_t(recvd)}));
    // + initialize rptr() and rendptr() so that we can safely use 'rpop'.
    try{
        set_rptrs(recvd);
    }catch(std::exception& re){
        // set_rptrs assumes that the packet conforms to a minimal
        // template.  Otherwise, it does the safe thing and throws.
        // In practice, there's probably an older version of the code
        // "sharing" our multicast channel.  Don't panic.  Complain
        // and carry on.
        auto firstnull = find(&data[0], &data[recvd], '\0');
        if( &data[recvd] == firstnull )
            complain(LOG_NOTICE, re, "distrib_cache_message::recv: packet does not start with an NTCS.  Definitely not meant for us");
        else if( str_view(&data[0], firstnull - &data[0]) == VERSION )
            complain(LOG_WARNING, re, "distrib_cache_message::recv: the VERSION is (surprsingly) correct, but the packet looks bad");
        else
            complain(LOG_NOTICE, re, "distrib_cache_message::recv: incorrect version");
        return false;
    }

    // + check the version.
    str_view version = rpop();
    // As in diskcache.cpp, a bad version/magic number is reported
    // with severity LOG_NOTICE.  It's typical to get a large number
    // of these when upgrading the software version of machines on a
    // network from cross-talk between old and new code.
    if(version != VERSION){
        complain(LOG_NOTICE, "distrib_cache_message::recv:  incorrect version");
        return false;
    }

    // check the scope.
    str_view msg_scope = rpop();
    if(msg_scope != dbe.scope){
        complain(LOG_WARNING, "distrib_cache_message::recv: unexpected scope.  Is somebody on our multicast channel?");
        return false;
    }

    // + check the timestamp: if it's outside a very generous window,
    //     return/complain about skewed clocks or possible replay
    //     attack.  If it's outside a narrow window, warn and return
    //     with an empty parts.
    int64_t tstamp;
    ::memcpy(&tstamp, rendptr(), sizeof(tstamp));
    int64_t now = now_millis();
    int64_t absdelta = abs(now - tstamp);
    if( absdelta > dbe.vols.multicast_timestamp_skew * 1000 ){
        // Don't cut this too thin.  See the comment in udp_listener
        // about handlers that take a long time.
        distrib_cache_stats.distc_delayed_packets++;
        throw std::runtime_error(fmt("unacceptable timestamp: %.3f, now: %.3f.  Clock skew?  Corrupted data?  Badly delayed listener loop? Replay attack?",
                                     tstamp*1.e-3, now*1.e-3));
    }
    
    // + read the sid, look it up and verify the hmac.
    str_view sid = rpop();
    if(dbe.secret_mgr){
        char *msg_hmac = rendptr() + sizeof(tstamp);
        auto key = dbe.secret_mgr->get_sharedkey(std::string{sid});
        if(key->size() < crypto_auth_KEYBYTES)
            throw std::runtime_error("key found, but it's too short to be used in crypto_auth");
        if(crypto_auth_verify((unsigned char*)msg_hmac, (unsigned char*)&data[0], msg_hmac-&data[0], key->data()) != 0)
            throw std::runtime_error("crypto_auth_verify failed.  Data corruption?  Forgery?");
    }
    // if we're not configured to reject_untrusted_multicast, we could
    // still use the the 32 bytes of hmac as a checksum to detect
    // network corruption.  E.g., we could put a 16-byte threeroe in
    // the hmac?  Or we could compute a libsodium hmac with a
    // non-secret key (all zeros?  the version?).  It doesn't protect
    // us from an MitM, but it does protect us from network
    // corruption.
    
    // SUCCESS!!
    // + copy the remaining words into 'parts'
    parts.reserve(2); // we're expecting 2
    while(rptr() < rendptr())
        parts.push_back(rpop());
    return true;
}

namespace {

bool is_multicast(const sockaddr_in& sai){
    return (ntohl(sai.sin_addr.s_addr)>>28 == 14); // 224.X.X.X through 239.X.X.X, top 4 bits are 1110
}

string cache_control(const reply123& r) {
    using namespace chrono;
    return str_sep("", "max-age=", 
                   duration_cast<seconds>(r.max_age()).count(),
                   ",stale_while_revalidate=",
                   duration_cast<seconds>(r.stale_while_revalidate).count());
}
} // namespace <anon>

distrib_cache_backend::distrib_cache_backend(backend123* upstream, backend123* server, const std::string& _scope,
                                             secret_manager* _secret_mgr,
                                             addrinfo_cache& _aicache, volatiles_t& volatiles) :
    upstream_backend(upstream),
    server_backend(server),
    scope(_scope),
    aicache(_aicache),
    vols(volatiles),
    secret_mgr(_secret_mgr)
{
    // - instantiate an fs123p7::server.
    DIAG(_distrib_cache, "distrib_cache_backend(scope=" + scope + ")");
    option_parser op;
    server_options sopts(op); // most of the defaults are  fine?
    op.set("bindaddr", "0.0.0.0");
    op.setopts_from_defaults();
    peer_handler = std::make_unique<peer_handler_t>(*this);
    myserver = make_unique<fs123p7::server>(sopts, *peer_handler);
    server_url = myserver->get_baseurl();
    sockaddr_in sain = myserver->get_sockaddr_in();
    char sockname[INET_ADDRSTRLEN];
    if(!inet_ntop(AF_INET, &sain.sin_addr, sockname, sizeof(sockname)))
        throw se("inet_ntop failed");
    complain(LOG_NOTICE, "Distributed cache server listening on %s port %d.  Unique name: %s\n", sockname, ntohs(sain.sin_port), get_uuid().c_str());

    auto self = make_unique<peer>(get_uuid(), server_url, upstream_backend);
    peer_map.insert_peer(move(self));

    // figure out where to send suggestions and discouragement packets:
    initialize_reflector_addr(envto<string>("Fs123DistribCacheReflector", "<unset>"));

    udp_fd = sew::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    int yes = 1;
    sew::setsockopt(udp_fd, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));
    if(is_multicast(reflector_addr)){
        // Online advice suggests use of IP_MULTICAST_TTL,
        // IP_MULTICAST_IF and IP_MULTICAST_LOOP on the sending side.
        //
        // The default IP_MULTICAST_TTL is 1, which seems fine.
        //
        // Normally, we don't want to hear our own chatter, so we
        // disable IP_MULTICAST_LOOP by default.  But if we're running
        // multiple peers on the same host (e.g., for debugging or
        // regression testing), then we need to enable it.
        // WARNING - if we use this for testing, our regression config
        // will be meaningfully different from our production config.
        multicast_loop = envto<bool>("Fs123DistribCacheMulticastLoop", false);
        int enabled = multicast_loop;
        sew::setsockopt(udp_fd, IPPROTO_IP, IP_MULTICAST_LOOP, &enabled, sizeof(enabled));
        //
        // We're not trying to bridge interfaces, so no need for
        // IP_MULTICAST_IF

        // Apparently, we have to bind the address *before* we join it?
        sew::bind(udp_fd, (const struct sockaddr*)&reflector_addr, sizeof(reflector_addr));

        // To receive packets, we have to join the multicast group:
        struct ip_mreq mreq;;
        mreq.imr_multiaddr = reflector_addr.sin_addr;
        mreq.imr_interface.s_addr = htonl(INADDR_ANY);
        sew::setsockopt(udp_fd, IPPROTO_IP, IP_ADD_MEMBERSHIP, &mreq, sizeof(mreq));
    }else{
        struct sockaddr_in recv_addr;
        // We're not sending to a multicast address, so assume that
        // there's a repeater out there that will send back to us
        recv_addr.sin_family = AF_INET;
        recv_addr.sin_addr.s_addr = htonl(INADDR_ANY);
        recv_addr.sin_port = htons(0);
        sew::bind(udp_fd, (const struct sockaddr*)&recv_addr, sizeof(recv_addr));
    }

    // no throw after this point!  The destructor won't be called, so the
    // threads won't clean up properly.
    udp_future = async(launch::async, &distrib_cache_backend::udp_listener, this);
    server_future = async(launch::async,
                          [&]() { try {
                                  complain(LOG_NOTICE, "calling myserver->run in async thread");
                                  myserver->run();
                                  complain(LOG_NOTICE, "returned from myserver->run in async thread");
                              }catch(exception& e){
                                  complain(e, "server thread exiting on exception.");
                              }
                          });
}

void
distrib_cache_backend::regular_maintenance() try {
    // Announce that we're 'present' as a peer to our group.
    //
    // FIXME - Make this conditional on some kind of self assessment.
    // I.e., don't advertise ourselves as a peer if our load average
    // is so high that we can't respond in a timely manner.  Or
    // consider whether there have recently been 'discouraging' 
    // messages about us.
    send_present();
    if(secret_mgr)
        secret_mgr->regular_maintenance();
 }catch(exception& e){
    complain(e, "Exception thrown by distib_cache_backend::regular_maintenance:");
 }

std::ostream&
distrib_cache_backend::report_stats(std::ostream& os){
    os << distrib_cache_stats;
    peer_map.forall_peers([&os,this](const pair<string, peer::sp>& p){
                              if(p.second->be == upstream_backend)
                                  return;
                              os << "BEGIN_peer_" << p.first << ":\n";
                              // Add two spaces to each line so that this looks
                              // like a 'block' if we parse .fs123_statistics as
                              // yaml.
                              core123::osvstream osvs;
                              p.second->be->report_stats(osvs);
                              for(auto line : svsplit_exact(sv_rstrip(osvs.sv()), "\n"))
                                  os << "  " << line << "\n";
                          });
    return os;
}

distrib_cache_backend::~distrib_cache_backend() try {
    if(!envto<bool>("Fs123DangerousNoDistribCacheAbsentOnShutdown", false)){ // for testing only.  Never in production!
        // Turning this off requires an intentionally long and hard-to-type command-line option.
        // Tell the world we're closing up shop
        DIAG(_shutdown, "~distrib_cache_backend: send_absent()");
        send_absent();
    }else
        complain(LOG_NOTICE, "~distrib_cache_backend:  Fs123DangerousNoDistrbCacheAbsentOnShutdown is set.  Absent not sent on multicast channel");
    // Shut down the server
    DIAG(_shutdown, "~distrib_cache_backend: myserver->stop");
    myserver->stop();
    DIAG(_shutdown, "~distrib_cache_backend: server_future.wait()");
    server_future.wait();
    // Shutting down the udp listener is tricky.  It will exit on the
    // next loop after we set udp_done.  That's under the control
    // of the poll(..., timeout), which is 100msec, so we shouldn't
    // have to wait long.
    udp_done = true;
    // We could *try* to wake the udp listener sooner by sending a
    // packet to it.  But we'd have to jump through hoops if
    // IP_MULTICAST_LOOP isn't enabled (it usually isn't), and it
    // would be unreliable anyway (c.f. the U in UDP).
    //
    // What if udp_listener is hung?  We can't carry on with the
    // destructor because udp_listener would access free'ed memory if
    // and when it ever wakes up.  "Hung" is tricky, though.  See the
    // comment in udp_listener about handle_present.
    //
    // This may be a situation where std::terminate is the right/only
    // answer?
    chrono::seconds how_long(vols.peer_connect_timeout + vols.peer_transfer_timeout + 10);
    DIAG(_shutdown, "~distrib_cache_backend: begin loop on udp_future.wait_for(" << ins(how_long) << ")");
    while(udp_future.wait_for(how_long) == future_status::timeout){
        complain(LOG_CRIT, "~distrib_cache_backend's udp_listner is hung.  You may have to kill -9 this process.");
        DIAG(_shutdown, "~distrib_cache_backend: iterate loop udp_future.wait_for(" << ins(how_long) << ")");
        //std::terminate();
    }
    DIAG(_shutdown, "~distrib_cache_backend: udp_future.get()");
    udp_future.get();
    complain(LOG_NOTICE, "distrib_cache_backend: udp_listener exited cleanly");
    DIAG(_shutdown, "~distrib_cache_backend:  done!");
 }catch(exception& e){
        complain(e, "distrib_cache_backend:~distrib_cache_backend threw an exception.  Something is probably wrong but carry on and hope for the best.");
        //std::terminate();  // is this another case where terminate is the lesser of evils ?
 }

void
distrib_cache_backend::initialize_reflector_addr(const std::string& reflector) /* private */ try {
    // Aack.  Nothing but boilerplate.  Lots of boilerplate...
    auto first_colon = reflector.find(':');
    if(first_colon == string::npos)
        throw invalid_argument("No colon found.  Expected IP:PORT");
    std::string reflector_ip = reflector.substr(0, first_colon);
    std::string reflector_port = reflector.substr(first_colon+1);
    struct addrinfo* addrinfo;
    struct addrinfo hints = {};
    hints.ai_family = AF_INET;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;
    // FIXME:  sew::getaddrinfo and sew::getnameinfo
    auto gairet = ::getaddrinfo(reflector_ip.c_str(), reflector_port.c_str(),
                              &hints, &addrinfo);
    if(gairet)
        throw runtime_error(strfunargs("getaddrinfo", reflector_ip, reflector_port, "...") + ": " + gai_strerror(gairet));
    if(addrinfo->ai_addrlen > sizeof(reflector_addr))
        throw runtime_error("getaddrinfo returned a struct bigger than a sockaddr_in ??");
    ::memcpy(&reflector_addr, addrinfo->ai_addr, addrinfo->ai_addrlen);
    ::freeaddrinfo(addrinfo);
    char hbuf[NI_MAXHOST], sbuf[NI_MAXSERV];
    gairet = ::getnameinfo((const sockaddr*)&reflector_addr, sizeof(reflector_addr),
                            hbuf, sizeof(hbuf),
                            sbuf, sizeof(sbuf),
                            NI_NUMERICHOST | NI_NUMERICSERV);
    if(gairet)
        throw runtime_error("getnameinfo couldn't make sense of reflector_addr");
    complain(LOG_NOTICE, "Sending distrib_cache peer discovery messages to %s:%s\n", hbuf, sbuf);
 }catch(std::exception&  e){
    throw_with_nested(runtime_error("error in distrib_cache_backend::initialize_reflector_addr(" + reflector + ")"));
 }

bool
distrib_cache_backend::refresh(const req123& req, reply123* reply) /*override*/ {
    if(req.no_peer_cache)
        return upstream_backend->refresh(req, reply);
    // Figure out which peer.
    peer::sp p = peer_map.lookup(req.urlstem);
    if(p->be == upstream_backend)
        return upstream_backend->refresh(req, reply);
    // We replace the /urlstem in 'req' with /p/urlstem
    DIAG(_distrib_cache_requests, "forwarding to " << ((p->be == upstream_backend) ? "local: " : "remote: ") << p->uuid);
    auto myreq = req;
    try{
        myreq.urlstem = "/p" + peer_handler_t::VERSION + req.urlstem;
        return p->be->refresh(myreq, reply);
    }catch(exception& e){
        handle_peer_error(p, myreq, e);
        return upstream_backend->refresh(req, reply);
    }
 }

void
distrib_cache_backend::handle_present(const string& peerurl){
    distrib_cache_stats.distc_presents_recvd++;
    // If it's already in the peer_map there's nothing to do.  Note that this
    // naturally handles loopbacks on the multicast channel.
    if(peer_map.check_url(peerurl)){
        DIAG(_distrib_cache, "handle_present(" +  peerurl +"): already known");
        return;
    }

    distrib_cache_stats.distc_presents_checked++;
    reply123 rep;
    unique_ptr<backend123_http> be;
    try{
        be = make_unique<backend123_http>(add_sigil_version(peerurl), "",
                                          aicache, vols, backend123_http::distrib_cache);
        // Get the uuid, which also checks connectivity.
        req123 req("/p" + peer_handler_t::VERSION + "/p/uuid", req123::MAX_STALE_UNSPECIFIED);
        // FIXME?  - time the be->refresh().  If it's slow (whatever
        // that means) return before calling insert_peer.
        be->refresh(req, &rep);
        DIAG(_distrib_cache, "handle_present: new url: " + peerurl + " uuid: " + rep.content);
    }catch(exception& e){
        DIAGf(_distrib_cache, "handle_present: Failed to connect with new peer: %s", peerurl.c_str());
        // Should we discourage others?  It's unlikely to help much because any
        // others are probably already executing this code path.  It might be
        // our problem and not the peer.  Let's not add to the noise.
        return;
    }
    // More checks??  E.g., check that /p/a should give us something
    // that is consistent with our own notion of the root's
    // attributes?

    // SECURITY: verify that the new peer isn't a malicious (or
    // misconfigured) man-in-the-middle?  Note that if we are using
    // secretbox, then we should have end-to-end confidentiality and
    // integrity, so a MitM isn't a huge problem.  But if we're not
    // using secretbox, this is an easy way to create an MitM!
    peer_map.insert_peer(make_unique<peer>(rep.content, peerurl, move(be)));
}

void
distrib_cache_backend::handle_absent(const string& peerurl){
    distrib_cache_stats.distc_absents_recvd++;
    if(peerurl == server_url){
        distrib_cache_stats.distc_self_absents_recvd++;
        return;
    }
    peer_map.remove_url(peerurl);
}

void
distrib_cache_backend::handle_peer_error(peer::sp p, const req123& req, std::exception& e){
    distrib_cache_stats.distc_peer_errors++;
    // Typically, the exception is something thrown by
    // backend_http::refresh, FIXME: look inside e to decide how
    // severe it really is, whether it looks permanent or transient,
    // is it part of a pattern, etc.  Should we stop talking to that
    // peer?  Should we discourage everyone else from talking to that
    // peer?
    complain(LOG_WARNING, e, "handle_peer_error:  client side error requesting %s from %s",
             req.urlstem.c_str(), p->url.c_str());
    send_discourage_peer(p->url);
    peer_map.remove_url(p->url);
}

void
distrib_cache_backend::handle_discourage_peer(const string& peerurl){
    DIAG(_distrib_cache, "discourage_peer(" + peerurl + ")");
    distrib_cache_stats.distc_discourages_recvd++;
    if(peerurl == server_url){
        // Can we use this to judge whether our peers are having
        // trouble talking to us?  Can we use that in
        // regular_maintenance to decide whether to announce ourselves
        // to the world with send_present.
        distrib_cache_stats.distc_self_discourages_recvd++;
        return;
    }
    // If this log message appears ahead of "peer->be->refresh threw"
    // messages, it's an indicator that we could/should have acted on
    // the 'discourage'.
    complain(LOG_WARNING, "handle_discourage_peer:  peer=%s.  Ignored", peerurl.c_str());
#if 1
    // Do nothing.  If peerurl is "bad", we'll find out for ourselves
    // soon enough.
    return;
#else
    // This code may get used one day.  But not until we've carefully
    // evaluated the pros and cons.  If we remove the peer without
    // checking then a misconfigured or overloaded node can convince
    // us to avoid peers that are perfectly fine.  If we do check,
    // then the same misconfigured node can initiate a thundering herd
    // of checking - which, if we're already teetering - might push
    // the whole network into an avalanche of failures.
    //
    // Note that neither false positives nor false negatives results
    // for peer_is_down are terribly disruptive.  A false positive
    // means we use the origin server for traffic that might have been
    // successfully handled by the peer (but note that we already have
    // a strong prior that the peer is down).  A false negative means
    // we keep talking to the peer, and we'll eventually figure out
    // that it's down, but our own clients will experience a delay
    // that might have been avoided.
    peer::sp peer = peer_map.lookup_peerurl(peerurl);
    if(!peer)
        return; // somebody removed it already
    bool peer_is_down = false;
    try{
        req123 req("/p" + peer_handler_t::VERSION + "/p/uuid", req123::MAX_STALE_UNSPECIFIED);
        reply123 reply;
        timer tmr;
        peer->be->refresh(req, &reply);
        auto howlong = tmr.elapsed();
        // if acceptable is too large, return false negatives
        // if it's too small, return false positives
        const auto acceptable = std::chrono::microseconds(100);
        if( howlong > acceptable )
            peer_is_down = true;
        else
            complain(LOG_WARNING, "not acting on discourage_peer(" + peerurl + ").  Peer is up.  Response time: " + str(howlong));
    }catch(std::exception& e){
        peer_is_down = true;
    }

    if(peer_is_down){
        distrib_cache_stats.distc_discourages_enacted++;
        peer_map.remove_url(peerurl);
    }
#endif
}

void
distrib_cache_backend::send_present() const noexcept try {
    DIAG(_distrib_cache, "send_present(" + server_url + ")");
    distrib_cache_message msg{*this};
    str_view parts[2] = {str_view("P"), str_view(server_url)};
    msg.send(udp_fd, reflector_addr, &parts[0], &parts[2]);
    distrib_cache_stats.distc_presents_sent++;
 }catch(std::exception& e){
    complain(e, "send_present(): exception caught and ignored:");
 }

void
distrib_cache_backend::send_absent() const noexcept try {
    DIAG(_distrib_cache, "send_absent(" + server_url + ")");
    distrib_cache_message msg{*this};
    str_view parts[2] = {str_view("A"), str_view(server_url)};
    msg.send(udp_fd, reflector_addr, &parts[0], &parts[2]);
    distrib_cache_stats.distc_absents_sent++;
 }catch(std::exception& e){
    complain(e, "send_absent(""): exception caught and ignored:");
 }

void
distrib_cache_backend::send_discourage_peer(const string& peer_url) const noexcept try {
    DIAG(_distrib_cache, "send_discourage_peer(" + peer_url + ")");
    distrib_cache_message msg{*this};
    str_view parts[2] = {str_view("D"), str_view(peer_url)};
    msg.send(udp_fd, reflector_addr, &parts[0], &parts[2]);
    distrib_cache_stats.distc_discourages_sent++;
 }catch(std::exception& e){
    complain(e, "send_discourage_peer(" + peer_url + "): exception caught and ignored:");
 }

void
distrib_cache_backend::udp_listener() {
    struct pollfd pfds[1];
    pfds[0].fd = udp_fd;
    pfds[0].events = POLLIN;
    while(!udp_done) try {
        distrib_cache_message msg{*this};
        // N.B.  we can't just call blocking recv because if no
        // messages arrive, we would never check udp_done.
        if(sew::poll(pfds, 1, 100) == 0)
            continue; // check udp_done if quiet for 100msec.
        if(!msg.recv(udp_fd))
            continue; // assume msg.recv already complained

        // version 2 messages have two parts:
        //   parts[0]: A command, either 'P'resent, 'A'bsent' or 'D'iscourage
        //   parts[1]: A URL
        if(msg.parts.size() != 2){
            complain("udp_listener: expected exactly 2 parts.  Got %zd", msg.parts.size());
            continue;
        }
        if(msg.parts[0].size() != 1){
            complain("udp_listener: expected a single-letter parts[0].  Got: " + strbe(msg.parts));
            continue;
        }
        // N.B.  handle_present can take a long time.  It might have to wait
        // for a refresh on the new peer to time out.  We can spin up
        // yet another thread,  or we can live with it.  The consequences
        // of living with it are:
        //  a) when one peer is flakey, we might not quickly respond
        //     to other peers coming and going.
        //  b) when a peer is flakey, it might take us approximately
        //     the http-timeout to shut down.
        //  c) we might be stuck for long enough that the next udp
        //     packet appears to be coming from the past.  We'll reject
        //     it with an error in the next msg.recv().
        // None of these seem worth adding more complexity to solve.
        switch(msg.parts[0][0]){
        case 'P': handle_present(string(msg.parts[1])); break;
        case 'A': handle_absent(string(msg.parts[1])); break;
        case 'D': handle_discourage_peer(string(msg.parts[1])); break;
        default:
            complain("udp_listener: unexpected msg.parts[0]: " + strbe(msg.parts));
            break;
        }
    }catch(std::exception& e){
        distrib_cache_stats.distc_recvd_errors++;
        complain(e, "exception thrown in udp_listener loop.  Continuing...");
        continue;
    }
    complain(LOG_NOTICE, "udp_listener shutting down cleanly with udp_done = %d (should be true)", udp_done.load());
 }

void
peer_handler_t::p(req::up req, uint64_t etag64, istream&) try {
    string versioned_url = urlescape(req->path_info);
    if(!startswith(versioned_url, VERSION))
        return exception_reply(move(req), http_exception(400, "Incorrect /p/sub-version"));
    if(req->query.data()) // req->query is non-null.  It might still be 0-length
        versioned_url += "?" + string(req->query);
    req123 myreq(versioned_url.substr(VERSION.size()), req123::MAX_STALE_UNSPECIFIED);
    myreq.no_peer_cache = true;
    reply123 reply123;
    if(etag64){
        // make the reply 'valid' by setting eno and set a non-zero
        // etag64 so that backend_http adds an INM header.
        reply123.eno = 0;
        reply123.etag64 = etag64;
    }
    if(startswith(myreq.urlstem, "/p")){
        // We're looking at a /p/p/XXX request.  I.e., a request
        // that we are *not* supposed to forward to server_backend!
        // It's unlikely that there will ever be more than a couple
        // of these, so an if/else if/else pattern should do fine...
        if(myreq.urlstem == "/p/uuid"){
            req->add_header(HHERRNO, "0"); // needed to get through backend_http on the other end.
            return p_reply(move(req), be.get_uuid(), 0, "max-age=86400");
        }
        return exception_reply(move(req), http_exception(404, "Unknown /p request: " + myreq.urlstem));
    }
        
    DIAG(_distrib_cache_requests, "/p request for " << myreq.urlstem);
    // N.B.  These requests will also be tallied in the statistics of
    // the server_backend, but the server_backend may also be getting
    // requests from others (see the ascii art in
    // distrib_cache_backend.hpp).
    atomic_scoped_nanotimer _t(&distrib_cache_stats.distc_server_refresh_sec);
#if 0
    // Ideally, we would have a stress test that causes bona fide
    // server side errors.  Until then, we can exercise some of the
    // error handling code by uncommenting this.
    if(threeroe(myreq.urlstem).hash64() % 100 == 0)
        throw libcurl_category_t::make_libcurl_error(CURLE_OPERATION_TIMEDOUT, ETIMEDOUT, "Randomly generated server error!");
#endif

    bool modified = be.server_backend->refresh(myreq, &reply123);
    distrib_cache_stats.distc_server_refreshes++;
    distrib_cache_stats.distc_server_refresh_bytes += reply123.content.size();
    string cc = cache_control(reply123);
    if(!modified){
        distrib_cache_stats.distc_server_refresh_not_modified++;
        return not_modified_reply(move(req), cc);
    }
    req->add_header(HHCOOKIE, str(reply123.estale_cookie));
    req->add_header(HHERRNO, str(reply123.eno));
    switch(reply123.content_encoding){
    case content_codec::CE_IDENT:
        break;
    case content_codec::CE_FS123_SECRETBOX:
        req->add_header("Content-encoding", "fs123-secretbox");
        break;
    case content_codec::CE_UNKNOWN:
        throw http_exception(500, "reply has unknown encoding.  This should have been caught earlier");
    }
    if(reply123.chunk_next_meta != reply123::CNO_MISSING){
        // Ugh...
        const char *xtra = (reply123.chunk_next_meta == reply123::CNO_EOF) ?
            " EOF" : "";
        req->add_header(HHNO, str(reply123.chunk_next_offset) + xtra);
    }
    // HHTRSUM
    return p_reply(move(req), reply123.content, reply123.etag64, cc);
 }catch(std::exception& e){
    try{
        std::throw_with_nested(http_exception(500, "distrib_cache_backend::peer_handler::p: url:" + string(req->uri)));
    }catch(std::exception& ne){
        // level?  Clients typically use LOG_WARNING for backend http errors, so ...
        complain(LOG_WARNING, ne, "this is the server-side complaint.  Look for a matching complaint on the client side");
        exception_reply(move(req), ne);
    }
 }
