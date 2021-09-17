#include "exportd_handler.hpp"
#include <core123/throwutils.hpp>
#include <core123/syslog_number.hpp>
#include <core123/diag.hpp>
#include <core123/sew.hpp>
#include <core123/log_channel.hpp>

using namespace core123;

namespace{
char PROGNAME[] = "exportd_handler";

void early_global_setup(const exportd_options& exportd_opts){
    // Configure things that aren't associated with a particular
    // handler, but that it's desirable to have set up before we
    // construct the server and handler.  E.g., complaints and
    // diagnostics, daemonizing, etc.
    //
    if(exportd_opts.daemonize && exportd_opts.pidfile.empty())
        throw se(EINVAL, "You must specify a --pidfile=XXX if you --daemonize");
    
    if(exportd_opts.daemonize){
        // We'll do the chdir ourselves after chroot, but allow
        // daemon(3) to dup2 /dev/null onto fd=0, 1 and 2.
        // Otherwise, our caller (e.g. sshd) might not realize that
        // we've disconnected.  As a result, sending logs or diags to
        // %stdout or %stderr is unproductive with with --daemonize.
#ifndef __APPLE__
        sew::daemon(true/*nochdir*/, false/*noclose*/);
#else
	throw se(EINVAL, "MacOS deprecates daemon().  Run in foreground and use launchd");
#endif
    }

    // N.B.  core123's log_channel doesn't call openlog.  But it
    // *does* explicitly provide a facility every time it calls
    // syslog.  So the third arg to openlog shouldn't matter.  But
    // just in case, glibc's openlog(...,0) leaves the default
    // facility alone if it was previously set, and sets it to
    // LOG_USER if it wasn't.
    unsigned logflags = LOG_PID|LOG_NDELAY;  // LOG_NDELAY essential for chroot!
    openlog(exportd_opts.PROGNAME, logflags, 0);
    set_complaint_destination(exportd_opts.log_destination, 0666);
    set_complaint_max_hourly_rate(exportd_opts.log_max_hourly_rate);
    set_complaint_averaging_window(exportd_opts.log_rate_window);
    if(!startswith(exportd_opts.log_destination, "%syslog"))
        start_complaint_delta_timestamps();

    if(!exportd_opts.diag_names.empty()){
        set_diag_names(exportd_opts.diag_names);
        set_diag_destination(exportd_opts.diag_destination);
        DIAG(true, "diags:\n" << get_diag_names() << "\n");
    }
    the_diag().opt_tstamp = true;

    if(!exportd_opts.pidfile.empty()){
        std::ofstream ofs(exportd_opts.pidfile.c_str());
        ofs << sew::getpid() << "\n";
        ofs.close();
        if(!ofs)
            throw se("Could not write to pidfile");
    }
}

// late_global_setup - called *after* we've constructed the server and handler,
// but immediately before we call s->run.  This is the place for writing
// out a 'portfile' (which isn't known until the server is constructed),
// establishing signal handlers (via server member functions), and chroot.
void late_global_setup(fs123p7::server* s, exportd_handler* h){
    s->set_signal_handlers(); // stop on TERM, INT, HUP and QUIT
    if(!h->opts.portfile.empty()){
        std::ofstream ofs(h->opts.portfile.c_str());
        sockaddr_in sain = s->get_sockaddr_in();
        ofs << ntohs(sain.sin_port) << "\n";
        ofs.close();
        if(!ofs)
            throw se("Could not write to portfile");
    }
    
    // If --chroot is empty (the default) then do neither chdir nor
    // chroot.  The process stays in its original cwd, relative paths
    // are relative to cwd, etc.  Add a SIGUSR1 handler that reopens
    // the acceslog and complaing destination.
    //
    // If --chroot is non-empty, then chdir first and, if chroot
    // is not "/", then chroot(".").  Thus it's possible to say
    // --chroot=/ even without cap_sys_chroot, but
    // --chroot=/anything/else requires cap_sys_chroot.
    // If --chroot is non-empty, there is no SIGUSR1 handler.
    if(h->opts.chroot.empty()){
        // handle SIGUSR1 only if there's no chroot!
        s->add_sig_handler(SIGUSR1,
                           [&](int, void*){
                               complain(LOG_NOTICE, "caught SIGUSR1.  Re-opening accesslog and complaint log");
                               h->accesslog_channel.reopen();
                               reopen_complaint_destination();
                           },
                           nullptr);
    }else{
        sew::chdir(h->opts.chroot.c_str());
        log_notice("chdir(%s) successful",  h->opts.chroot.c_str());
        if(h->opts.chroot != "/"){
            try{
                sew::chroot(".");
                log_notice("chroot(.) (relative to chdir'ed cwd) successful");
            }catch(std::system_error& se){
                std::throw_with_nested(std::runtime_error("\n"
"chroot(.) failed after a successful chdir to the intended root\n"
"Workarounds:\n"
"   --chroot=/      # chdir(\"/\") but does not make chroot syscall\n"
"   --chroot=       # runs in cwd.  Does neither chdir nor chroot\n"
"  run with euid=0  # root is permitted to chroot\n"
"  give the executable the cap_sys_chroot capability, e.g.,:\n"
"    sudo setcap cap_sys_chroot=pe /path/to/executable\n"
"  but not if /path/to/executable is on NFS.\n"));
                // P.S.  There may be a way to do this with capsh, but only
                // if the kernel supports 'ambient' capabilities (>=4.3).
                // sudo capsh --keep=1 --uid=$(id -u) --caps="cap_sys_chroot=pei"  -- -c "obj/fs123p7 exportd --chroot=/scratch ..."
                // only gets us '[P]ermitted' cap_sys_chroot, but not [E]ffective.
                // Maybe with more code we could upgrade from P to E?
            }
        }
        // Re-open the cc_rule_cache so it's export_root is opened after the chroot.
        h->rule_cache = std::make_unique<cc_rule_cache>(h->opts.export_root, h->opts.rc_size, h->opts.default_rulesfile_maxage, h->opts.no_rules_cc);
    }
}

} // namespace <anon>

int app_exportd(int argc, char *argv[]) try
{
    the_diag().opt_tid = true;
    // There is one option_parser.
    core123::option_parser op;
    // Associate the option_parser with instances of the generic
    // server_options and the specific exportd_options.
    fs123p7::server_options server_opts(op);
    exportd_options exportd_opts(op, PROGNAME);
    // Parse all options together, populating server_opts
    // and exportd_opts
    auto more_args = op.setopts_from_argv(argc, argv);
    // Help only?
    if(exportd_opts.help){
        std::cerr << op.helptext() << "\n";
        return 0;
    }
    if(!more_args.empty())
        throw std::runtime_error("unrecognized arguments:" + strbe(more_args));
    // configure logs, diags, daemonization, etc. (things we want to do
    // *before* constructing the server)
    early_global_setup(exportd_opts);
    // Boilerplate to construct a server attached to a handler...
    exportd_handler h(exportd_opts);
    std::unique_ptr<fs123p7::server> s;
    std::unique_ptr<fs123p7::tp_handler<exportd_handler>> tph;
    if(exportd_opts.threadpool_max){
        tph = std::make_unique<fs123p7::tp_handler<exportd_handler>>(exportd_opts.threadpool_max,
                                                            exportd_opts.threadpool_idle, h);
        s = std::make_unique<fs123p7::server>(server_opts, *tph);
    }else{
        s = std::make_unique<fs123p7::server>(server_opts, h);
    }
    // configure signal handlers, create a portfile, chroot, etc.  (things we
    // want to or can only do *after* constructing the server).
    late_global_setup(s.get(), &h);
    // argcheck verifies (among other things) that we can actually construct
    // a server with the given arguments.  It's the most reliable
    // way to check that the libevent we're linked with works with non-zero
    // --threadpool-max.
    if(exportd_opts.argcheck)
        return 0;
    s->run(); // normally runs forever.
    return 0;
 }catch(std::exception& e){
    core123::complain(e, "Shutting down because of exception caught in main");
    return 1;
 }
