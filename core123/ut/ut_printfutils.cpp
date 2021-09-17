// unit tests for fmt (formerly stringprintf)
#include "core123/strutils.hpp"
#include "core123/datetimeutils.hpp"
#include <limits>
#include <string>
#include <sstream>
#include <cstring>
//#include <boost/test/minimal.hpp>
//#define  ASSERT BOOST_REQUIRE
//#define  MAIN test_main
#include <cassert>
#define  ASSERT assert
#define  MAIN main
#include <cstdlib>
#include <cinttypes>

using namespace std;
using core123::fmt;
using core123::str;
using core123::nanos;

// FIXME - test more, e.g., especially things going out of scope.
// A wider variety of printf formats would be nice, but do we
// really think that the underlying vsnprintf is broken?
int MAIN(int, char **){

    string s;
    char buf[8192];
    stringstream ss;

    s = fmt("%f", 3.1415);
    ASSERT(s == "3.141500");
    
    core123::str_view sv = "hello world";
    s = fmt("%f %" PRIsv " world", 3.1415, PRIsvarg(sv.substr(0,5)));
    ASSERT(s == "3.141500 hello world");

#pragma GCC diagnostic ignored "-Wformat-zero-length"
    s = fmt("");
    ASSERT(s == "");

    s = fmt("%34s", "hello world");
    sprintf(buf, "%34s", "hello world");
    ASSERT(s == buf);

    // The initial buffer in printfutils.hpp is
    // 512 bytes long.  Let's try 512, then 511
    // and then 513.
    s = fmt("%512s", "hello world");
    sprintf(buf, "%512s", "hello world");
    ASSERT(s == buf);
    
    s = fmt("%511s", "hello world");
    sprintf(buf, "%511s", "hello world");
    ASSERT(s == buf);

    s = fmt("%513s", "hello world");
    sprintf(buf, "%513s", "hello world");
    ASSERT(s == buf);
    
    s = fmt("%5000s", "hello world");
    sprintf(buf, "%5000s", "hello world");
    ASSERT(s == buf);
    ASSERT(s.size() == 5000);
                  
    // strns and fmtdur are handy formatters for integer nanosecond
    // counters and chrono::durations.  Let's exercise them a bit:
    s = str(nanos(0));
    using std::chrono::nanoseconds;
    std::string durs;
    durs = str(nanoseconds(0));
    ASSERT(durs == s);
    ASSERT(s == "0.000000000");

    s = str(nanos(-1));
    durs = str(nanoseconds(-1));
    ASSERT(durs == s);
    ASSERT(s == "-0.000000001");

    s = str(nanos(1234567890));
    durs = str(nanoseconds(1234567890));
    ASSERT(durs == s);
    ASSERT(s == "1.234567890");

    s = str(nanos(std::numeric_limits<long long>::min()));
    ASSERT(s == "-9223372036.854775808"); // assumes 64-bit long long

#if 0
    // This code should generate a warning under -Wall and an error
    // with -Wall -Werror.  Keep it commented out, except to exercise
    // the __attribute__(__format__(__printf__...)) checker.
    //
    // This was an actual error that slipped by because we left out
    // -Wall...
    {
        using core123::tohex;
        std::string idxurl="url";
        std::string idxmnt="mnt";
        std::string idxhash="hash";
        struct {
            uint64_t ninodes=6;
            uint64_t arcnum=9;
            uint64_t arcid=15;
            std::pair<uint64_t, uint64_t> archash = {55,44};
        } ai;
            
        auto reply = fmt("%" PRIu64 " inodes added from \"%s\" at \"%s\""
                "arcnum %" PRIu64 " arcid %" PRIu64 " idxhash %d%s archash %s%s\n",
                ai.ninodes, idxurl.c_str(), idxmnt.c_str(), ai.arcnum, ai.arcid,
                idxhash.c_str(),
                         tohex(ai.archash.first).c_str(), tohex(ai.archash.second).c_str());
    }
#endif

    return 0;
}
