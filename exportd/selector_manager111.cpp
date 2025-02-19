#include "selector_manager111.hpp"
#include "crfio.hpp"
#include "cc_rules.hpp"
#include "fs123/stat_serializev3.hpp" // for ifdef APPLE mtim
#include "fs123/content_codec.hpp"
#include "fs123/sharedkeydir.hpp"
#include <core123/complaints.hpp>
#include <core123/base64.hpp>
#include <core123/http_error_category.hpp>
#include <core123/autoclosers.hpp>
#include <core123/sew.hpp>
#include <core123/diag.hpp>
#include <core123/strutils.hpp>
#include <core123/datetimeutils.hpp>
#include <gflags/gflags.h>

using namespace core123;

namespace{
auto _cache_control = diag_name("cache_control");
auto _secretbox = diag_name("secretbox");
}

DEFINE_string(export_root, "/", "root of exported tree (post-chroot)");
DEFINE_string(estale_cookie_src, "ioc_getversion", "source for estale_cookie.  Must be one of ioc_getversion|getxattr|setxattr|st_ino|none");
DEFINE_string(cache_control_file, "", "cache control file (post-chroot) - empty means all cache control replies will carry the short timeout");
DEFINE_uint64(max_age_short, 60, "max-age (sec) for short-timeout objects");
DEFINE_uint64(max_age_long, 86400, "max-age (sec) for long-timeout objects");
DEFINE_uint64(stale_while_revalidate_short, 30, "stale-while-revalidate time (sec) for short-timeout objects");
DEFINE_uint64(stale_while_revalidate_long, 43200, "stale-while-revalidate time (sec) for long-timeout objects");

// Unless you're doing something very fancy, you can't "rotate" keys much faster than
// max_age_long, so there's no point in refreshing much more often.
DEFINE_uint64(sharedkeydir_refresh, 43200, "reread files in sharedkeydir after this many seconds");
DEFINE_string(encoding_keyid_file, "encoding", "name of file containing the encoding secret. (if relative, then with respect to sharedkeydir, otherwise with respect to chroot)");
DECLARE_bool(allow_unencrypted_replies);

// SECURITY IMPLICATIONS!
// We have always automatically prepended the 'public' directive to
// cache-control.  The 'public' directive tells proxy- and
// reverse-caches that they can cache and serve the resource even
// though it might have required authentication.  Some simple
// experiments with squid suggest that it's not completely trivial to
// defeat Basic-Authorization.  I.e., it's not immediately obvious how
// to retrieve a 'public' resource from squid if you don't have the
// resource's authorization key.  "not immediately obvious" is not a
// strong assurance.
//
// FIXME - Verify that our production varnish configs "work" for
// non-Authenticated content without "public" and then don't make
// 'public' the default!
DEFINE_string(cache_control_directives, "public", "extra cache-control directives to add to every reply");

DEFINE_string(cache_control_regex, "", "three-part colon-separated arg CC-good:CC-enoent:regex.  CC-xx are strings, not integers and may contain either or both max-age and stale-while-revalidate directives");

DEFINE_bool(decentralized_cache_control, false, "use rules found in .fs123_cc_rules files under <export_root> to construct cache-control headers.  Fall back to --cache_control_file if no rules are found");
DEFINE_uint64(dcc_cache_size, 1000000, "maximum size of the cache of .fs123_cc_rules files");
DEFINE_int32(dcc_rulesfile_max_age, -1, "recheck rules files after this many seconds unless explicitly specified in the rules-file itself.  Default value of -1 means same as --max-age-short");

static const std::string FALLBACK = "cc-fallback";

selector_manager111::selector_manager111(int sharedkeydir_fd):
    oneseldata(std::make_shared<per_selector111>(sharedkeydir_fd))
{
    // Note that it is pointless to make max_age_short smaller than
    // the caching timeout of the nfs (or other) mount point that
    // serves --export_root

    // arbitrary limit of 1 year, seems pointless to have higher ages than that?
    const unsigned max_max_age = 365*86400;

    if (FLAGS_max_age_long > max_max_age || FLAGS_max_age_long < FLAGS_max_age_short || FLAGS_stale_while_revalidate_long < FLAGS_stale_while_revalidate_short)
	throw se(EINVAL, "inconsistent FS123_CACHE_CONTROL parameters");
}

per_selector111::per_selector111(int sharedkeydir_fd) :
    longtimeoutroot{std::make_unique<stringtree>()},
    ltrsb{}
{
    // validate the command line args and throw at constructor time if they don't look good.
    validate_basepath(FLAGS_export_root);
    validate_estale_cookie(FLAGS_estale_cookie_src);

    if( !FLAGS_cache_control_regex.empty() ){
        // Expect a three-part colon-separated arg, e.g.,
        //    'max-age=10,stale-while-revalidate=90:max-age=0:.*\.dms'
        try{
            auto idx1 = FLAGS_cache_control_regex.find(':', 0);
            if(idx1==std::string::npos) throw se(EINVAL, "didn't find first colon in --cache-control-regex option");
            cc_good = FLAGS_cache_control_regex.substr(0, idx1);
            auto idx2 = FLAGS_cache_control_regex.find(':', idx1+1);
            if(idx2==std::string::npos) throw se(EINVAL, "didn't find second colon in --cache-control-regex option");
            cc_enoent = FLAGS_cache_control_regex.substr(idx1+1, idx2-(idx1+1));
            auto cc_re = FLAGS_cache_control_regex.substr(idx2+1);
            cc_regex = std::unique_ptr<std::regex>(new std::regex(cc_re, std::regex::extended)); // avoid make_unique to work around https://gcc.gnu.org/bugzilla/show_bug.cgi?id=85098
            DIAGkey(_cache_control, "parsed ccre:  good: " << cc_good << " noent: " << cc_enoent << " re: " << cc_re << "\n");
        }catch(std::runtime_error& e){
            std::throw_with_nested(std::runtime_error("failed to parse --cache-control-regex: " + FLAGS_cache_control_regex));
        }
    }
    if( !FLAGS_cache_control_directives.empty() && !endswith(FLAGS_cache_control_directives, ",") )
        FLAGS_cache_control_directives += ',';

    if(sharedkeydir_fd >= 0)
        sm = std::make_unique<sharedkeydir>(sharedkeydir_fd, FLAGS_encoding_keyid_file, FLAGS_sharedkeydir_refresh);

    short_timeout_cc = FLAGS_cache_control_directives + "max-age=" + std::to_string(FLAGS_max_age_short);
    if(FLAGS_stale_while_revalidate_short)
        short_timeout_cc += ",stale-while-revalidate="+ std::to_string(FLAGS_stale_while_revalidate_short);

    if( FLAGS_decentralized_cache_control ){
        if(FLAGS_dcc_rulesfile_max_age == -1)
            FLAGS_dcc_rulesfile_max_age = FLAGS_max_age_short;
        DIAGkey(_cache_control, "create rule cache with cache_size:" << FLAGS_dcc_cache_size << " rulesfile_max_age: " << FLAGS_dcc_rulesfile_max_age << " max_age_short: " << FLAGS_max_age_short);
        rule_cache = std::make_unique<cc_rule_cache>(FLAGS_export_root, FLAGS_dcc_cache_size, FLAGS_dcc_rulesfile_max_age, FALLBACK);
    }
}

const std::string&
per_selector111::basepath() const{
    return FLAGS_export_root;
}

const std::string&
per_selector111::estale_cookie_src() const{
    return FLAGS_estale_cookie_src;
}
  
void
per_selector111::regular_maintenance() try {
    if(sm)
        sm->regular_maintenance();
    
    if (FLAGS_cache_control_file.empty()) {
        // longtimeoutroot was initialized in the constructor to a
        // unique_ptr to an empty longtimeoutnode.  As a result,
        // 'search()' in cachecontrol.cpp will return
        // PATH_NOT_IN_TREE, resulting in short timeouts for
        // everything.
	return;
    }
    struct stat sb;
    sew::stat(FLAGS_cache_control_file.c_str(), &sb);
    std::lock_guard<std::mutex> lg(ltrmtx);
    if( sb.st_ctim == ltrsb.st_ctim && sb.st_mtim == ltrsb.st_mtim && sb.st_size == ltrsb.st_size )
        return;
    log_notice("timeout data file %s has changed.  Reading...", FLAGS_cache_control_file.c_str());
    acFILE fp = sew::fopen(FLAGS_cache_control_file.c_str(), "r");
    std::string k;
    std::string v;
    auto newltr = std::make_unique<stringtree>();
    size_t nrecords = 0;
    while( crfio::in(fp, k, v) ){
        if(!k.empty() && k[0] == '/'){
            try{
                add_prefix(k.substr(1), newltr.get());
            }catch(std::exception& e){
                std::throw_with_nested(std::runtime_error(str("thrown frm add_prefix k =", k)));
            }
            nrecords++;
        }
    }
    longtimeoutroot = std::move(newltr);
    sew::fstat(fileno(fp), &ltrsb);
    log_notice("new timeout data with %zd long-timeout records", nrecords);
    return;
 }catch(std::exception& e){
    std::throw_with_nested(std::runtime_error("problem in maybe_update_cache_control"));
 }


std::string
per_selector111::get_cache_control(const std::string& func, const std::string& path_info, const struct stat* sb, int eno,
		  unsigned max_max_age){
    // If eno is non-zero, the reply will contain the specified
    // errno=eno, but it will *not* carry any data or metadata.
    // Nevertheless, we have to specify cache-control.  Current policy
    // is that the cache-control for an ENOENT should be the same as
    // for a successful reply.  This particular policy was very much
    // the original motivation for all of fs123!  Cache the ENOENTs so
    // that we pay for python's stupid search heuristics only once,
    // but then the negative result sits in cache (maybe even in the
    // kernel) for a long time thereafter.
    //
    // But what about other errnos?  We currently give them short
    // timeouts on the grounds that they *might* be transient.  I.e.,
    // we wouldn't want a transient EIO to stick around in caches for
    // months or years!  This policy may be refined in the future...
    if(eno != 0 && eno != ENOENT)
        return short_timeout_cc;

    // Another parameter??  Are we likely to be DoS'ed by this?
    // maybe max-age=1 ? 
    if(func == "n") // 'numbers',  i.e., stats shouldn't be cached
        return "max-age=0"; 
    
    // strip off the leading '/'.  Note that we've already 'validated'
    // the path_info to guarantee that it is either empty or starts
    // with '/'.
    std::string pi = path_info.empty()? path_info : path_info.substr(1);

    // First look for "decentralized" rules files
    if(rule_cache){
        // gets called with sb==nullptr for /l, /n, etc.  But
        // none of those are directories, so:
        bool isdir = sb && S_ISDIR(sb->st_mode);
        std::string cc = rule_cache->get_cc(pi, isdir); 
        if(cc != FALLBACK)
            return cc;
    }

    // Then look for the command-line --cache-control-regex option
    if(cc_regex){
        if(std::regex_match(pi, *cc_regex)){
            DIAGkey(_cache_control, "get_cache_control(func=" << func << ", path_info=" << pi << ") -> matched regex eno=" << eno << "\n");
            switch(eno){
            case 0:
                return FLAGS_cache_control_directives + cc_good;
            case ENOENT:
                return FLAGS_cache_control_directives + cc_enoent;
            // anything else falls through, eventually returning
            // max_age_short and swr-short (see below)
            }
        }
    }

    // Look in a "database" of "long-timeout paths" (represented by
    // the stringtree longtimeoutroot).  If any prefix of relpath is
    // in the database, then return Cache-control with max-age set to
    // FLAGS_max_age_long or max_max_age, whichever is smaller.  If no
    // prefix of relpath is in the database, return a Cache-control
    // with max-age set to FLAGS_max_age_short (or max_max_age,
    // whichever is smaller).  Note that 'path_info' generally starts
    // with '/', unless it's referring to the root, in which case it's
    // empty.  max_max_age is basically an upper-limit override, see
    // the comment on the heuristic implemented by time_since_change_
    // in do_request.

    unsigned max_age = 0;
    auto srch = search(pi, longtimeoutroot.get());
    DIAGkey(_cache_control, "search(pi=" << pi << ") -> " << srch << "\n");
    std::string ret;
    switch(srch){
    case stringtree::PATH_IN_TREE:
    case stringtree::PATH_NOT_IN_TREE:
        ret = short_timeout_cc;
        break;
    case stringtree::TREE_PREFIXES_PATH:
    case stringtree::PATH_TERMINATES_TREE:
        max_age = std::min(static_cast<decltype(max_max_age)>(FLAGS_max_age_long),  max_max_age);
        ret = FLAGS_cache_control_directives+"max-age=" + std::to_string(max_age);
        if(FLAGS_stale_while_revalidate_long){
            ret += ",stale-while-revalidate=" + std::to_string(FLAGS_stale_while_revalidate_long);
        }
        break;
    }
    DIAGkey(_cache_control, "get_cache_control(func=" << func << ", path_info=" << path_info << ") -> " << ret << "\n");
    return ret;
}

std::string
per_selector111::get_encode_secretid(){
    return sm ? 
        sm->get_encode_sid() :
        secret_manager::DO_NOT_ENCODE_SID;
}

std::pair<str_view, std::string>
per_selector111::encode_content(const fs123Req& req, const std::string& esid, str_view in,
                                str_view workspace){
    if(!sm)
        return {in, ""};
    if(esid == secret_manager::DO_NOT_ENCODE_SID)
        return {in, ""};
    auto ace = content_codec::encoding_stoi(req.accept_encoding_);
    // N.B.  ace=CE_UNKNOWN is perfectly reasonable, e.g., when
    // a cache accepts "gzip".  Just ignore it.
    if(ace != content_codec::CE_FS123_SECRETBOX){
        if(FLAGS_allow_unencrypted_replies)
            return {in, ""};
        else
            httpthrow(406, "Request must specify Accept-encoding: fs123-secretbox");
    }
    auto esecret = sm->get_sharedkey(esid);
    return {content_codec::encode(ace, esid, esecret, in, workspace, 32), "fs123-secretbox"};
}

std::string
per_selector111::decode_envelope(const std::string& path_info) /*override*/ try {
    if(path_info.empty() || path_info[0] != '/')
        httpthrow(400, "path_info must be of the form /<base64(path_info)>");
    if(!sm)
        httpthrow(400, "per_selector111::decode_envelope:  no sharedkeydir.  Can't decode");
    std::string decode64 = macaron::Base64::Decode(path_info.substr(1));
    return content_codec::decode(content_codec::CE_FS123_SECRETBOX, decode64, *sm);
 } catch(std::exception& e){
    std::throw_with_nested(http_exception(400, "per_selector111::decode: codec->decode failed"));
 }

std::ostream&
per_selector111::report_stats(std::ostream& os){
    if(rule_cache)
        rule_cache->report_stats(os);
    content_codec::report_stats(os);
    if(sm)
        sm->report_stats(os);
    return os;
}
