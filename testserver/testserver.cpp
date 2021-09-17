// The test server tries to use as many features as possible/practical
// so that we can exercise them in various testing scenarios.

// Obscure configuration options and odd behavior are its whole reason
// for being.  Don't use this as a "how to"!

#include "fs123/fs123server.hpp"
#include <core123/http_error_category.hpp>
#include <core123/svto.hpp>
#include <core123/opt.hpp>
#include <core123/strutils.hpp>
#include <core123/diag.hpp>
#include <core123/unused.hpp>
#include <regex>

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

static const std::string cc = "max-age=3600,stale-while-revalidate=7200,stale-if-error=86400";

// For debugging and bug-hunting!  Sleep for a random time to give
// callers a chance to exercise timeout paths, expose data races,
// etc.
void random_sleep(double b){
    if(b==0.)
        return;
    thread_local std::mt19937 g(std::hash<std::thread::id>()(std::this_thread::get_id()));
    thread_local std::cauchy_distribution<> cd(0., b);  // cauchy is a very wide.  In fact, the mean is undefined/infinite.
    double howlong = std::abs(cd(g));
    DIAGf(_testserver, "random_sleep for %f\n", howlong);
    std::this_thread::sleep_for(std::chrono::duration<double>(howlong));
}

// /bigdir.NNNN is a listable directory with NNNN entries numbered 0 through
// NNNN-1.
void d_for_bigdir(fs123p7::req::up reqp, uint64_t /*inm64*/, const std::string& start){
    // We know it starts with /bigdir., so
    size_t off = sizeof("/bigdir.") - 1;
    size_t istart, num;
    try{
        num = svto<size_t>(reqp->path_info, off);
        istart = (start.empty()) ? 0 : svto<size_t>(start);
    }catch(std::exception&){
        return exception_reply(move(reqp), http_exception(400, "/d request doesn't look right"));
    }
    size_t i = istart;
    while(i<num){
        if(reqp->add_dirent(to_string(i), DT_REG, 0))
            i++;
        else
            break;
    }
    std::string more = (i>=num) ? "" : to_string(i);
    d_reply(move(reqp), more, etag, estale_cookie, cc);
}

struct bench_handler: public fs123p7::handler_base{
    // N.B.  The cc could be a constructor-argument...
    bool strictly_synchronous() override { return true; }
    void a(fs123p7::req::up reqp) override {
        struct stat sb = {}; // all zeros!
        static regex bigdir_re("/bigdir.[0-9]+");
        if(reqp->path_info.empty()){
            // asking about the root.  It's executable but niether
            // readable nor writable.
            sb.st_mode = S_IFDIR | 0111;
        } else if( regex_match(string(reqp->path_info), bigdir_re) ) {
            sb.st_mode = S_IFDIR | 0555;
        }else{
            // Otherwise, the only entries are regular files
            // whose last components be parsed as numbers.
            sb.st_mode = S_IFREG | 0444;
            try{
                auto [dirpart, filepart] = rsplit1(reqp->path_info, '/');
                unused(dirpart);
                sb.st_size = svto<size_t>(filepart);
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
    void d(fs123p7::req::up reqp, uint64_t inm64, std::string start) override{
        if(inm64 == etag)
            return not_modified_reply(move(reqp), cc);
        if( startswith(reqp->path_info, "/bigdir.") ){
            return d_for_bigdir(move(reqp), inm64, start);
        }else{
            // There are files here, but you can't list them.
            return d_reply(std::move(reqp), {}, validator, estale_cookie, cc);
        }
    }
    void f(fs123p7::req::up reqp, uint64_t inm64, size_t len, uint64_t offset, void* buf) override{
        size_t sz;
        // The only files that exist have names that can be parsed as
        // numbers.  Their contents is the letter 'x' repeated
        // <filename> times.
        try{
            auto [dirpart, filepart] = rsplit1(reqp->path_info, '/');
            unused(dirpart);
            sz = svto<size_t>(filepart);
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
    void p(fs123p7::req::up reqp, uint64_t /*etag64*/, std::istream& /*in*/) override{
        // it's the test-server.  Gotta' do *something*...  Let's echo
        // the uri.
        std::string uri{reqp->uri};
        p_reply(std::move(reqp), uri, 0/*etag*/, cc);
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
