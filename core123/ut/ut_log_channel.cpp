// This one is hard to test properly.  To really
// be sure, we'd have to figure out how syslogs are actually
// delivered and go look there.

#include "core123/log_channel.hpp"
#include "core123/strutils.hpp"
#include "core123/sew.hpp"
#include <time.h>
#include <stdlib.h>

using core123::log_channel;
using core123::fmt;
namespace sew = core123::sew;

int main(int argc, char **argv){
    log_channel lc("%syslog%LOG_INFO%LOG_USER", 0666);
    lc.send("This should go to syslog LOG_USER with level LOG_INFO");

    lc.open("/tmp/logchannel.test", 0666);
    
    time_t rawtime ;
    sew::time(&rawtime);
    lc.send(fmt("This should go to /tmp/logchannel.test.  The time is now: %s",
                ::ctime(&rawtime)));

    sew::system("cat /tmp/logchannel.test");
    return 0;
}
