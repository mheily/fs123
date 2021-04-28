#include "distrib_cache_backend.hpp"
#include "fs123/httpheaders.hpp"
#include <core123/strutils.hpp>
#include <core123/diag.hpp>
#include <core123/scoped_timer.hpp>
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netdb.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <poll.h>

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

namespace {

// distrib_cache_message:  encapsulate some of the details of sending, receiving
// and "parsing" udp messages.  It's still *very* raw, but maybe better than
// just having this code inline.  "Messages" are concatenations of NUL-terminated
// strings.  They're (currently) limited to 512 bytes.

// To send a message, we take a [begin, end) collection of string_view's
// and bundle them up for sendmsg, making sure to NUL-terminate each one.
//
// To receive a message, we instantiate an empty distrib_cache_message
// and call its 'recv' method.  After which, the 'parts' member is a
// vector of str_views to a local copy of the sender's NUL-terminated
// str_views.
//
// N.B.  We're breaking our standing rule about exceptions here: recv
// *only* throw an exception when the fd itself looks completely
// borked.  If the only problem is garbled data, e.g., missing NULs,
// or an incorrect version prefix, it just complains and returns with
// an empty 'parts'.
struct distrib_cache_message{
    std::array<char,512> data;
    std::vector<core123::str_view> parts;
    // VERSION is semantically just a string that either matches or doesn't.
    // But it's easier to think about if we keep it numeric.
    static constexpr str_view VERSION{"1"};
    template <typename ITER>
    static void send(int sockfd, const struct sockaddr_in& dest, ITER b, ITER e);
    void recv(int fd);
};

template <typename ITER>
/*static */
void
distrib_cache_message::send(int sockfd, const struct sockaddr_in& dest,
                 ITER b, ITER e){
    char zero = '\0';
    struct msghdr msghdr = {};
    msghdr.msg_name = (void*)&dest;
    msghdr.msg_namelen = sizeof(struct sockaddr_in);
    msghdr.msg_iovlen = 1 + 2*(e-b);
    struct iovec iov[msghdr.msg_iovlen];
    iov[0].iov_base = const_cast<char*>(VERSION.data());
    iov[0].iov_len = VERSION.size()+1; // including the NUL
    msghdr.msg_iov = iov;
    int i=1;
    while(b != e){
        iov[i].iov_base = const_cast<char*>(b->data());
        iov[i].iov_len = b->size();
        iov[i+1].iov_base = &zero;
        iov[i+1].iov_len = 1;
        i+=2;
        b++;
    }
    core123::sew::sendmsg(sockfd, &msghdr, 0);
}

void distrib_cache_message::recv(int fd){
    using namespace core123;
    if(!parts.empty())
        throw std::logic_error("distrib_cache_messages::recv:  may only be called once");
    // MSG_DONTWAIT may be superfluous because we've just poll'ed,
    // but it shouldn't do any harm, and protects us against
    // "spurious" wakeups (can that happen?)
    auto recvd = ::recv(fd, data.data(), data.size(), MSG_DONTWAIT|MSG_TRUNC);
    if(recvd < 0){
        if(errno == EAGAIN)
            return complain(LOG_WARNING, "udp_listener:  unexpected EAGAIN from recv(MSG_DONTWAIT)");
        throw se("recv(udp_fd) in udp_listener");
    }
    if(recvd == 0)
        return complain(LOG_WARNING, "distrib_cache_messages::recv:  empty message");
    if(size_t(recvd) > data.size()) // see MSG_TRUNC
        return complain(LOG_WARNING, "distrib_cache_message::recv:  message is too long.  Treating as empty");
    if(data[recvd-1] != '\0')
        return complain(LOG_WARNING, "distrib_cache_message::recv:  message is not NUL-terminated.  Treating as empty.");
    // check the version, but don't include it in 'parts'
    str_view version{&data[0]}; // we just checked that data is NUL-terminated
    // As in diskcache.cpp, a bad version/magic number is reported
    // with severity LOG_NOTICE.  It's typical to get a large number
    // of these when upgrading the software version of machines on a
    // network, and there's cross-talk between old and new code.
    if(version != VERSION)
        return complain(LOG_NOTICE, "distrib_cache_message::recv:  incorrect version");
    
    parts.reserve(3); // we're expecting 3
    char *b = &data[version.size()+1];
    char *e = &data[recvd];
    while(b < e){
        char* nextnul = std::find(b, e, '\0');
        parts.push_back(str_view(b, nextnul-b)); // N.B.  The NUL isn't *in* the str_view, but it's guaranteed to follow it.
        b = nextnul+1;
    }
}

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
                                             addrinfo_cache& _aicache, volatiles_t& volatiles) :
    upstream_backend(upstream),
    server_backend(server),
    scope(_scope),
    peer_handler(*this),
    aicache(_aicache),
    vols(volatiles)
{
    // - instantiate an fs123p7::server.
    option_parser op;
    server_options sopts(op); // most of the defaults are  fine?
    op.set("bindaddr", "0.0.0.0");
    myserver = make_unique<fs123p7::server>(sopts, peer_handler);
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
        complain(LOG_WARNING, e, "peer->be->refresh threw.  Discouraging future attempts to use that peer: " + p->url);
        handle_peer_error(p->url);
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
        // accept_encodings is empty.  We make '/p' requests and get
        // uninterpreted binary data back.  The data *may* have an
        // encoding, but we're oblivious to that, and we don't want
        // another layer of encryption or encoding added.
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
distrib_cache_backend::handle_peer_error(const string& peerurl){
    distrib_cache_stats.distc_peer_errors++;
    send_discourage_peer(peerurl);
    peer_map.remove_url(peerurl);
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
    DIAG(_distrib_cache, "send_present(" + server_url + ", scope=" + scope + ")");
    str_view parts[3] = {str_view("P"), str_view(server_url), str_view(scope)};
    distrib_cache_message::send(udp_fd, reflector_addr, &parts[0], &parts[3]);
    distrib_cache_stats.distc_presents_sent++;
 }catch(std::exception& e){
    complain(e, "send_present(): exception caught and ignored:");
 }

void
distrib_cache_backend::send_absent() const noexcept try {
    DIAG(_distrib_cache, "send_absent(" + server_url + "), scope=" + scope);
    str_view parts[3] = {str_view("A"), str_view(server_url), str_view(scope)};
    distrib_cache_message::send(udp_fd, reflector_addr, &parts[0], &parts[3]);
    distrib_cache_stats.distc_absents_sent++;
 }catch(std::exception& e){
    complain(e, "send_absent(""): exception caught and ignored:");
 }

void
distrib_cache_backend::send_discourage_peer(const string& peer_url) const noexcept try {
    DIAG(_distrib_cache, "send_discourage_peer(" + peer_url + "), scope=" + scope);
    str_view parts[3] = {str_view("D"), str_view(peer_url), str_view(scope)};
    distrib_cache_message::send(udp_fd, reflector_addr, &parts[0], &parts[3]);
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
        distrib_cache_message msg;
        // N.B.  we can't just call blocking recv because if no
        // messages arrive, we would never check udp_done.
        if(sew::poll(pfds, 1, 100) == 0)
            continue; // check udp_done if quiet for 100msec.
        msg.recv(udp_fd);
        // messages have three parts:
        //   parts[0]: A command, either 'P'resent, 'A'bsent' or 'C'heck
        //   parts[1]: A URL
        //   parts[2]: The scope of the sender (to avoid crosstalk);
        if(msg.parts.size() != 3){
            complain("udp_listener: garbled msg with %zd NUL-terminated parts (expected 3)", msg.parts.size());
            continue;
        }
        if(msg.parts[2] != scope){
            // FIXME - it would be nice to report what what we know about ports and IP addresses,
            // but unfortunately, that's all now "hidden" inside msg.recv.
            complain(LOG_WARNING, "udp_listener: received message with incorrect scope. Got %s, expected %s. Is somebody else on our channel?",
                     msg.parts[2].data(), scope.c_str());
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
        // Neither of these seem worth adding more complexity to solve.
        switch(msg.parts[0][0]){
        case 'P': handle_present(string(msg.parts[1])); break;
        case 'A': handle_absent(string(msg.parts[1])); break;
        case 'D': handle_discourage_peer(string(msg.parts[1])); break;
        default:
            complain("udp_listener: garbled msg: " + strbe(msg.parts));
            break;
        }
    }catch(std::exception& e){
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
    if(reply123.chunk_next_meta != reply123::CNO_MISSING){
        // Ugh...
        const char *xtra = (reply123.chunk_next_meta == reply123::CNO_EOF) ?
            " EOF" : "";
        req->add_header(HHNO, str(reply123.chunk_next_offset) + xtra);
    }
    // HHTRSUM
    return p_reply(move(req), reply123.content, reply123.etag64, cc);
 }catch(std::exception& e){
    complain(e, "Exception thrown by distrib_cache_backend::peer_handler::p.");
    // don't pass 'e' to exception_reply.  It will just issue the same complaint again, to no useful purpose.
    exception_reply(move(req), http_exception(500, "distrib_cache_backend::peer_handler::p:  Client will see 500 and will discourage others from connecting to us."));
 }
