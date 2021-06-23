#include "fs123/fs123server.hpp"
#include <core123/opt.hpp>
#include <core123/diag.hpp>
#include <sys/stat.h>
#include <unistd.h>

using namespace core123;
using namespace std;

auto _ex1server = diag_name("ex1server");

// Nothing ever changes, so we can use the same validator,
// estale_cookie and etag everywhere.  There's no reason
// for them to be different, except to make it a little
// more obvious for debugging.
static constexpr uint64_t validator = 12345;
static constexpr uint64_t estale_cookie = 54321;
static constexpr uint64_t etag = 31415;

struct example_handler: public fs123p7::handler_base{
    // N.B.  The cc could be a constructor-argument...
    string cc = "max-age=3600,stale-while-revalidate=7200,stale-if-error=86400";
    bool strictly_synchronous() override { return true; }
    void a(fs123p7::req::up reqp) override {
        DIAGf(_ex1server, "a(%s)", string(reqp->path_info).c_str());
        struct stat sb = {}; // all zeros!
        sb.st_uid = geteuid();
        sb.st_gid = getegid();
        // st_mtime, st_ctime, etc.  Leave them at zero?  Set them to the current time?
        // Some other time?  
        if(reqp->path_info == ""){
            // the root of the filesystem
            sb.st_mode = S_IFDIR | 0555;
            sb.st_nlink = 2;
        }else if(reqp->path_info == "/hello"){
            // the file called "hello"
            sb.st_mode = S_IFREG | 0444;
            sb.st_nlink = 1;
            sb.st_size = 6; // "world\n"
        }else if(reqp->path_info == "/hi"){
            // the symlink called "hi"
            sb.st_mode = S_IFLNK | 0777;
            sb.st_nlink = 1;
            sb.st_size = 5; // "hello"
        }else{
            return errno_reply(move(reqp), ENOENT, cc);
        }
        a_reply(move(reqp), sb, validator, estale_cookie, cc);
    }
    void d(fs123p7::req::up reqp, uint64_t /*inm64*/, bool /*begin*/, int64_t /*offset*/) override{
        DIAGf(_ex1server, "d(%s)", string(reqp->path_info).c_str());
        if(reqp->path_info != "")
            return errno_reply(move(reqp), ENOTDIR, cc);
        reqp->add_dirent("hello", 1, DT_REG, estale_cookie);
        reqp->add_dirent("hi", 2, DT_LNK, estale_cookie);
        reqp->add_dirent(".", 3, DT_DIR, estale_cookie);
        reqp->add_dirent("..",  4, DT_DIR, estale_cookie);
        d_reply(move(reqp), true, validator, estale_cookie, cc);
    }
    void f(fs123p7::req::up reqp, uint64_t inm64, size_t len, uint64_t offset, void* buf) override{
        DIAGf(_ex1server, "f(%s)", string(reqp->path_info).c_str());
        if(reqp->path_info != "/hello")
            return errno_reply(move(reqp), ENOENT, cc);
        if(inm64 == etag)
            return not_modified_reply(move(reqp), cc);
        size_t sz = sizeof("world\n")-1; // exclude terminal NUL
        size_t n;
        if(offset > sz)
            n = 0;
        else if(len > sz-offset)
            n = sz - offset;
        else
            n = len;
        ::memcpy(buf, &"world\n"[offset], n);
        f_reply(move(reqp), n, validator, etag, estale_cookie, cc);
    }
    void l(fs123p7::req::up reqp) override{
        DIAGf(_ex1server, "l(%s)", string(reqp->path_info).c_str());
        if(reqp->path_info != "/hi")
            return errno_reply(move(reqp), ENOENT, cc);
        l_reply(move(reqp), "hello", cc);
    }
    void s(fs123p7::req::up reqp) override{
        errno_reply(move(reqp), ENOTSUP, cc);
    }

    example_handler() = default;
    ~example_handler() = default;
};

int main(int argc, char *argv[]) try
{
    // Associate an option_parser with instances of the generic
    // server_options.
    core123::option_parser opt_parser;
    fs123p7::server_options server_opts(opt_parser);
    // Add a few more options of our own:
    bool help=false;
    opt_parser.add_option("help", "produce this message", opt_true_setter(help));

    // Parse all options together, populating both server_opts and
    // our own options.
    auto more_args = opt_parser.setopts_from_argv(argc, argv);
    // Help only?
    if(help){
        cerr << opt_parser.helptext() << "\n";
        return 0;
    }
    if(!more_args.empty())
        throw runtime_error("unrecognized arguments:" + strbe(more_args));

    // Construct a handler, a server, and call run().
    example_handler h;
    fs123p7::server s(server_opts, h);
    s.run();
    return 0;
 }catch(exception& e){
    core123::complain(e, "Shutting down because of exception caught in main");
    return 1;
 }
