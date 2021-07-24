#pragma once

#include <core123/stats.hpp>
#include <optional>
#include <chrono>
#include <cstdint>
#include <fuse/fuse_lowlevel.h>
#include <errno.h>
#include <iosfwd>
#include "backend123.hpp"

// Declarations of things defined in mount.fs123.p7.cpp but that we use
// elsewhere in the client-side code.

reply123 begetattr(fuse_ino_t ino, std::optional<int> max_stale, bool no_cache);
uint64_t validator_from_a_reply(const reply123& r);
std::ostream& report_stats(std::ostream&);
std::ostream& report_config(std::ostream&);
extern int proto_minor;

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
