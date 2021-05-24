// The test server tries to use as many features as possible/practical
// so that we can exercise them in various testing scenarios.

// Obscure configuration options and odd behavior are its whole reason
// for being.  Don't use this as a "how to"!

#include "fs123/fs123server.hpp"
#include "fs123/sharedkeydir.hpp"
#include <core123/svto.hpp>
#include <core123/opt.hpp>
#include <core123/strutils.hpp>
#include <core123/diag.hpp>

using namespace core123;
using namespace std;

auto _testserver = diag_name("testserver");

// Nothing ever changes, so we can use the same validator,
// estale_cookie and etag everywhere.  There's no reason
// for them to be different, except to make it a little
// more obvious for debugging.
static constexpr uint64_t validator = 12345;
static constexpr uint64_t estale_cookie = 54321;
static constexpr uint64_t etag = 31415;

// These shouldn't be global.  Can't we find a way to push them into the
// server library?  If not, then maybe into the bench_handler?
acfd sharedkeydir_fd;
unique_ptr<secret_manager> secret_mgr;

struct bench_handler: public fs123p7::handler_base{
    // N.B.  The cc could be a constructor-argument...
    std::string cc = "max-age=3600,stale-while-revalidate=7200,stale-if-error=86400";
    bool strictly_synchronous() override { return true; }
    void a(fs123p7::req::up reqp) override {
        struct stat sb = {}; // all zeros!
        if(reqp->path_info.empty()){
            // asking about the root.  It's executable but niether
            // readable nor writable.
            sb.st_mode = S_IFDIR | 0111;
        }else{
            // Other than the root, the only entries are regular files
            // whose names can be parsed as numbers.
            sb.st_mode = S_IFREG | 0444;
            try{
                sb.st_size = svto<size_t>(reqp->path_info, 1);
            }catch(std::exception&){
                return errno_reply(std::move(reqp), ENOENT, cc);
            }
        }
        //  Redirect if sz is divisible by 17:
        if(sb.st_size%17 == 0 &&  sb.st_size){
            return redirect_without_17(std::move(reqp), sb.st_size);
        }                
        a_reply(std::move(reqp), sb, validator, estale_cookie, cc);
    }
    void d(fs123p7::req::up reqp, uint64_t /*inm64*/, bool /*begin*/, int64_t /*offset*/) override{
        // There are files here, but you can't list them.
        d_reply(std::move(reqp), true, validator, estale_cookie, cc);
    }
    void f(fs123p7::req::up reqp, uint64_t inm64, size_t len, uint64_t offset, void* buf) override{
        size_t sz;
        // The only files that exist have names that can be parsed as
        // numbers.  Their contents is the letter 'x' repeated
        // <filename> times.
        try{
            sz = svto<size_t>(reqp->path_info, 1);
        }catch(std::exception&){
            return errno_reply(std::move(reqp), ENOENT, cc);
        }
        //  Redirect if sz is divisible by 17:
        if(sz%17 == 0 &&  sz){
            return redirect_without_17(std::move(reqp), sz);
        }                

        if(inm64 == etag)
            return not_modified_reply(std::move(reqp), cc);
        size_t n;
        if(offset > sz)
            n = 0;
        else if(len + offset > sz)
            n = sz - offset;
        else
            n = len;
        ::memset(buf, 'x', n);
        f_reply(std::move(reqp), n, validator, etag, estale_cookie, cc);
    }
    void l(fs123p7::req::up reqp) override{
        errno_reply(std::move(reqp), ENOENT, cc);
    }
    void s(fs123p7::req::up reqp) override{
        errno_reply(std::move(reqp), ENOTSUP, cc);
    }

    secret_manager* get_secret_manager(){
        return secret_mgr.get();
    }
    bench_handler(){}
    ~bench_handler(){}
private:
    void redirect_without_17(fs123p7::req::up reqp, size_t sz){
        do{
            sz /= 17;
        }while(sz%17 == 0 && sz);
        auto pi = reqp->uri.find(reqp->path_info);
        DIAG(_testserver, "redirect_without_17: reqp->uri: " + string(reqp->uri) + " pi: " + str(pi));
        //auto redirect = "http://" + (*reqp->get_header("Host")) + string(reqp->uri.substr(0, pi+1)) + str(sz);
        // The rfcs say that  relative urls are ok in a 302 Location.  Does our code?
        auto redirect = string(reqp->prefix) + string(reqp->function) + "/" + str(sz);
        if(reqp->query.data())
            redirect += "?"  + string(reqp->query);
        DIAG(_testserver, "redirect_without_17: " + redirect);
        return redirect_reply(std::move(reqp), redirect, cc);
    }
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
    int threadpool_max;
    opt_parser.add_option("threadpool_max", "0", "maximum number of handler threads", opt_setter(threadpool_max));
    int threadpool_idle;
    opt_parser.add_option("threadpool_idle", "1", "number of handler threads at zero load", opt_setter(threadpool_idle));
    optional<string> opt_sharedkeydir;
    opt_parser.add_option("sharedkeydir", {}, "where to find shared keys", opt_setter(opt_sharedkeydir));
    
    optional<string> opt_diag_names;
    string diag_destination;
    opt_parser.add_option("diag_names", {}, "diagnostics enabled", opt_setter(opt_diag_names));
    opt_parser.add_option("diag_destination", "%stderr", "diagnostics destination", opt_setter(diag_destination));

    // Parse all options together, populating both server_opts and
    // our own options.
    auto more_args = opt_parser.setopts_from_argv(argc, argv);
    // Help only?
    if(help){
        std::cerr << opt_parser.helptext() << "\n";
        return 0;
    }
    if(opt_diag_names){
        set_diag_names(*opt_diag_names);
        set_diag_destination(diag_destination);
        DIAG(true, "diags:\n" << get_diag_names() << "\n");
    }
    the_diag().opt_tstamp = true;

    if(!more_args.empty())
        throw std::runtime_error("unrecognized arguments:" + strbe(more_args));

    if(opt_sharedkeydir){
        sharedkeydir_fd = sew::open(opt_sharedkeydir->c_str(), O_DIRECTORY|O_RDONLY);
        secret_mgr = make_unique<sharedkeydir>(sharedkeydir_fd, "encoding", 90);
    }
    // Create a bench_handler, optionally wrap it with a threadpool wrapper,
    // and then create a server with the handler.
    bench_handler h;
    std::unique_ptr<fs123p7::tp_handler<bench_handler>> tph;
    std::unique_ptr<fs123p7::server> s;
    if(threadpool_max){
        tph = std::make_unique<fs123p7::tp_handler<bench_handler>>(threadpool_max,
                                                                   threadpool_idle, h);
        s = std::make_unique<fs123p7::server>(server_opts, *tph);
    }else{
        s = std::make_unique<fs123p7::server>(server_opts, h);
    }
    // run the server (forever - until we get kill()-ed)
    s->run();
    return 0;
 }catch(std::exception& e){
    core123::complain(e, "Shutting down because of exception caught in main");
    return 1;
 }
