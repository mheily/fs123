#pragma once

#include "backend123.hpp"
#include "fs123/httpheaders.hpp"
#include <core123/svto.hpp>
#include <core123/str_view.hpp>
#include <core123/expiring.hpp>
#include <core123/stats.hpp>
#include <chrono>
#include <fuse/fuse_lowlevel.h>
#include <optional>
#include <iosfwd>
#include <cstdint>

// Declarations of things defined in app_mount.cpp but that we use
// elsewhere (special_ino.cpp, openfilemap.cpp).

struct decoded_reply{ // needed in special_ino.cpp
public:
    decoded_reply() = delete;
    decoded_reply(const decoded_reply&) = delete;
    decoded_reply(decoded_reply&&) = default;
    decoded_reply& operator=(const decoded_reply&) = delete;
    decoded_reply& operator=(decoded_reply&&) = default;

    clk123_t::time_point expires;
    clk123_t::duration stale_while_revalidate;
    bool cacheable;

    int eno;
    std::string _plaintext;
    core123::str_view _content;
    core123::str_view content() const { return _content; }
    // N.B.  estale_cookie, chunk_next_start and validator throw an exception
    // if the requested key wasn't in the header.  Don't ask if you don't expect
    // it to be there!
#define RETHROW catch(std::exception&) { std::throw_with_nested(std::runtime_error(__func__)); }
    uint64_t estale_cookie() const try { return core123::svto<uint64_t>(kvmap.at(FS123_COOKIE));} RETHROW
    core123::str_view chunk_next_start() const try { return kvmap.at(FS123_NEXTSTART); } RETHROW
    uint64_t validator() const try { return core123::svto<uint64_t>(kvmap.at(FS123_VALIDATOR)); } RETHROW
#undef RETHROW    
    // Do we really want a map?  We have O(3) key-value pairs!  Would a vector be better?
    std::map<core123::str_view, core123::str_view> kvmap;
    std::string kvinputstring7_2;

    decoded_reply(reply123&& from, std::string&& plaintext, const std::string& urlstem); // in app_mount.cpp
};

// The attrcache's API is not consistent with the other backends.  The begetattr
// variants all return an expiring<attrcache_value_t>.

struct attrcache_value_t{ // needed in openfilemap.cpp
    int eno;
    uint64_t estale_cookie;
    clk123_t::duration stale_while_revalidate;
    bool cacheable;
    struct stat sb;
    uint64_t validator;
    attrcache_value_t() : eno{-1}{}
    attrcache_value_t(const decoded_reply& dr); // in app_mount.cpp
};

using begetattr_t = core123::expiring<attrcache_value_t>;
begetattr_t begetattr(fuse_ino_t ino, std::optional<int> max_stale, bool no_cache); // used in openfilemap.cpp
decoded_reply begetserver_stats(fuse_ino_t ino); // used in special_ino.cpp
std::ostream& report_stats(std::ostream&);       // used in special_ino.cpp
std::ostream& report_config(std::ostream&);      // used in special_ino.cpp

#define FS123_STATISTICS \
    STATISTIC_NANOTIMER(elapsed_sec)            \
    STATISTIC(lookups)                          \
    STATISTIC_NANOTIMER(lookup_sec)             \
    STATISTIC(lookup_enoents)                   \
    STATISTIC(lookup_other_errno)               \
    STATISTIC(getattrs)                         \
    STATISTIC_NANOTIMER(getattr_sec)            \
    STATISTIC(getattrs_with_fi)                 \
    STATISTIC(getattr_enoents)                  \
    STATISTIC(getattr_other_errno)              \
    STATISTIC(stat_scans)                       \
    STATISTIC(validator_scans)                  \
    STATISTIC(getxattrs)                        \
    STATISTIC_NANOTIMER(getxattr_sec)           \
    STATISTIC(listxattrs)                       \
    STATISTIC_NANOTIMER(listxattr_sec)          \
    STATISTIC(readlinks)                        \
    STATISTIC_NANOTIMER(readlink_sec)           \
    STATISTIC(shortcircuit_readlinks)           \
    STATISTIC(opendirs)                         \
    STATISTIC_NANOTIMER(opendir_sec)            \
    STATISTIC(readdirs)                         \
    STATISTIC_NANOTIMER(readdir_sec)            \
    STATISTIC(releasedirs)                      \
    STATISTIC(forget_calls)                     \
    STATISTIC(forget_inos)                      \
    STATISTIC(ioctls)                           \
    STATISTIC(opens)                            \
    STATISTIC_NANOTIMER(open_sec)               \
    STATISTIC(reads)                            \
    STATISTIC_NANOTIMER(read_sec)               \
    STATISTIC(bytes_read)                       \
    STATISTIC(direct_io_opens)                  \
    STATISTIC(no_keep_cache_opens)              \
    STATISTIC(reread_no_cache)                  \
    STATISTIC(non_monotonic_validators)         \
    STATISTIC(fake_ino_dirents)                 \
    STATISTIC(estale_thrown)                    \
    STATISTIC(estale_retries)                   \
    STATISTIC(estale_ignored)                   \
    STATISTIC(req123_mismatch)                  \
    STATISTIC(releases)                         \
    STATISTIC(caught_system_errors)             \
    STATISTIC(caught_std_exceptions)            \
    STATISTIC(toplevel_retries)                 \
    STATISTIC(aicache_checks)                   \
    STATISTIC_NANOTIMER(aicache_check_sec)      \
    STATISTIC(of_notify_invals)                 \
    STATISTIC(of_getattrs)                      \
    STATISTIC(of_throwing_getattrs)             \
    STATISTIC(of_immediate_expirations)         \
    STATISTIC(of_failed_getattrs)               \
    STATISTIC(of_pq_reinserted)                 \
    STATISTIC(of_pq_scanraces)                  \
    STATISTIC(of_pq_stale_ctors)                \
    STATISTIC(of_wakeups)
    
#define STATS_STRUCT_TYPENAME fs123_stats_t
#define STATS_MACRO_NAME FS123_STATISTICS
#include <core123/stats_struct_builder>
#undef FS123_STATISTICS
inline fs123_stats_t stats;
