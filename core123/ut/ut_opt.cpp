#include <core123/opt.hpp>
#include <core123/ut.hpp>
#include <core123/diag.hpp>
#include <core123/complaints.hpp>
#include <string>
#include <cinttypes>
#include <sstream>

using namespace std;
using namespace core123;
using std::optional;

namespace {

const auto _main = diag_name("main");

string diagon;
bool debug;
uint32_t u32;
uint64_t u64;
double dbl;
std::string path1, path2;
std::string path3, path4, path5;
int vs;
//bool help;

void printopts() {
#define prtopt(name) std::cout << #name << " = " << name << std::endl;    
    prtopt(debug);
    prtopt(u32);
    prtopt(path1);
    prtopt(u64);
    prtopt(path2);
    prtopt(dbl);
    prtopt(vs);
    prtopt(path3);
    prtopt(path4);
    prtopt(path5);
    //prtopt(help);
}

#define TEST_PREFIX "TESTOPT_"
} // namespace <anon>

void dependent_defaults(int argc, char **argv, bool expected_foo, bool expected_bar){
    // This is schematically how our --decrypt-requests and --require-encrypted-requests
    // are related.  Let's make sure the cases work out...
    option_parser op;
#if 0
    optional<bool> foo, bar;
    op.add_option("foo", "false", "the  value of foo", opt_setter(foo));
    op.add_option("bar", {}, "the  value of bar", opt_setter(bar));
    op.setopts_from_argv(argc, argv);
    if(!bar)
        bar = *foo;
    EQUAL(*foo, expected_foo);
    EQUAL(*bar, expected_bar);
#else
    optional<bool> maybe_bar;
    bool foo, bar;
    op.add_option("foo", "false", "the  value of foo", opt_setter(foo));
    op.add_option("bar", {}, "the  value of bar", opt_setter(maybe_bar));
    op.setopts_from_argv(argc, argv);
    bar = (maybe_bar) ? *maybe_bar : foo;
    EQUAL(foo, expected_foo);
    EQUAL(bar, expected_bar);
#endif
}

// Can we design a "generic" test that we can apply many different times?
//
// First, we create to have a test_result 'object' with a rich variety
// of members, with names that hint at how their corresponding
// options.

struct test_result{
    int no_default_int;
    int default99_int;
    bool no_default_bool;
    optional<int> opt_int;
    optional<bool> opt_bool;
};

// Second, we create a do_test() function with an
// option_parser and a rich variety of add_option options.
test_result do_test(std::initializer_list<const char*> il){
    option_parser op;
    test_result t;
    op.add_option("no_default_int", {}, "", opt_setter(t.no_default_int));
    op.add_option("default99_int", "99", "", opt_setter(t.default99_int));
    op.add_option("opt_int", {}, "", opt_setter(t.opt_int));
    op.add_option("no_default_bool", {}, "", opt_true_setter(t.no_default_bool));
    op.add_option("opt_bool", {}, "", opt_true_setter(t.opt_bool));
    op.add_option("no_opt_bool", {}, "", opt_false_setter(t.opt_bool));
    op.setopts_from_range(begin(il), end(il));
    return t;
}

// Finally, we create a set of generic tests that  calls do_test
// several times with different argument strings, and then checks
// that values in the returned test_result are "correct"
void generic_tests(){
    test_result t;
    t = do_test({"--no_default-int", "3"});
    EQUAL(t.no_default_int,  3);
    EQUAL(t.default99_int, 99);
    CHECK(!t.opt_int);
    CHECK(!t.opt_bool);

    // same thing,  but with an =
    t = do_test({"--no_default-int=3"});
    EQUAL(t.no_default_int,  3);
    EQUAL(t.default99_int, 99);
    CHECK(!t.opt_int);
    CHECK(!t.opt_bool);

    t = do_test({"--opt_bool", "not_an_arg", "--default99_int", "88"});
    CHECK(t.opt_bool && *t.opt_bool);
    EQUAL(t.default99_int, 88);
    CHECK(!t.opt_int);

    t = do_test({"--nooptbool"});
    CHECK(t.opt_bool && !*t.opt_bool);

    bool missing_argument = false;
    try{
        do_test({"--default99_int=33", "--no-default-int"});
        CHECK(false);  // shouldn't get here
    }catch(option_missing_argument_error&){
        missing_argument = true;
    }
    CHECK(missing_argument);
}

int main(int, char **) try
{
    bool help = false;
    option_parser op;
    
    generic_tests();

    // Followed by a bunch of ad hoc tests with little rhyme or reason, other
    // than attempt to explore corner cases.
    string refhelp{"    flagfile (no default) : read flags from the named file\n"};
    EQUAL(op.helptext(), refhelp);

    op.add_option("help", {}, "Produce this message", opt_true_setter(help));
    refhelp += "    help (no default) : Produce this message\n";
    EQUAL(help, false);
    EQUAL(op.helptext(), refhelp);
    
    op.add_option("debug", "0", "turns on debug", opt_setter(debug));
    refhelp = "    debug (default=0) : turns on debug\n" + refhelp;
    EQUAL(debug, 0);
    EQUAL(op.helptext(), refhelp);
    
    op.add_option("path1",  "/x", "set a string", opt_setter(path1));
    refhelp += "    path1 (default=/x) : set a string\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("path2", "", "set another string", opt_setter(path2));
    refhelp += "    path2 (default=) : set another string\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("path3", "", "third string", opt_setter(path3));
    refhelp += "    path3 (default=) : third string\n";
    EQUAL(op.helptext(), refhelp);
    
    option& opt_p4 = op.add_option("path4", {}, "fourth string", opt_setter(path4));
    refhelp += "    path4 (no default) : fourth string\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("path5", "", "fifth string", opt_setter(path5));
    refhelp += "    path5 (default=) : fifth string\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("u32",  "101", "set a 32bit unsigned", opt_setter(u32));
    refhelp += "    u32 (default=101) : set a 32bit unsigned\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("u64", "0xffffffffffffffff", "set a 64bit unsigned", opt_setter(u64));
    refhelp += "    u64 (default=0xffffffffffffffff) : set a 64bit unsigned\n";
    EQUAL(op.helptext(), refhelp);

    op.add_option("dbl", "-3.14e-9", "set a double", opt_setter(dbl));
    refhelp = "    dbl (default=-3.14e-9) : set a double\n" + refhelp;

    op.add_option("verify-something", "-795", "set an int", opt_setter(vs));
    refhelp += "    verify-something (default=-795) : set an int\n";
    EQUAL(op.helptext(), refhelp);

    op.setopts_from_defaults();
    EQUAL(path1, "/x");
    EQUAL(path2, "");
    EQUAL(path3, "");
    CHECK(!opt_p4.get_as_optional()); // no default, not assigned a value
    EQUAL(path5, "");
    EQUAL(u32, 101);
    EQUAL(u64, 0xffffffffffffffff);
    EQUAL(vs, -795);
    EQUAL(u32, 101); // default unchanged
    EQUAL(path1, "/x"); // default unchanged
    //EQUAL(op.helptext(), refhelp);

    const char *xv1[] = {"prognamexv1", "--u64=0xfeeeeeeeeeeeeeee", "--help"};
    std::vector<std::string> leftover;

    leftover = op.setopts_from_argv(sizeof(xv1)/sizeof(xv1[0]), (char **) xv1);
    if (_main) {
        DIAG(_main, "--- after xv1:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    EQUAL(help, true);
    EQUAL(leftover.size(), 0);
    EQUAL(u64, 0xfeeeeeeeeeeeeeee);
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default

    const char *xv2[] = {"prognamexv2", "--verify-something=123", "foo1"};
    leftover = op.setopts_from_argv(sizeof(xv2)/sizeof(xv2[0]), (char **) xv2);
    if (_main) {
        DIAG(_main, "--- after xv2:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(vs, 123);
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default
    EQUAL(leftover.size(), 1);
    EQUAL(leftover[0], "foo1");

    const char *xv2b[] = {"prognamexv2", "--verify-something", "124", "foo1"};
    leftover = op.setopts_from_argv(sizeof(xv2b)/sizeof(xv2b[0]), (char **) xv2b);
    if (_main) {
        DIAG(_main, "--- after xv2:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(vs, 124);
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default
    EQUAL(leftover.size(), 1);
    EQUAL(leftover[0], "foo1");

    const char *xv3[] = {"prognamexv3", "foo2", "", "bar2"};
    leftover = op.setopts_from_argv(sizeof(xv3)/sizeof(xv3[0]), (char **) xv3);
    if (_main) {
        DIAG(_main, "--- after xv3:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default
    EQUAL(leftover.size(), 3);
    EQUAL(leftover[0], "foo2");
    EQUAL(leftover[1], "");
    EQUAL(leftover[2], "bar2");

    const char *xv4[] = {"prognamexv4", "--verify-something", "bar3"};
    bool got_expected_exception1 = false, should_not_reach1 = false;
    try{
        leftover = op.setopts_from_argv(sizeof(xv4)/sizeof(xv4[0]), (char **) xv4);
        CHECK(should_not_reach1);
    }catch(option_error& oe){
        got_expected_exception1 = true;
        CHECK(strcmp(oe.what(), "setopts_from_range: error while processing --verify-something") == 0);
        if (_main) complain(oe, "setopts_from_xv4:");
    }
    if (_main) {
        DIAG(_main, "--- after xv4:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    CHECK(got_expected_exception1);

    const char *xv4b[] = {"prognamexv4", "--verify-something"};
    got_expected_exception1 = false, should_not_reach1 = false;
    try{
        leftover = op.setopts_from_argv(sizeof(xv4b)/sizeof(xv4b[0]), (char **) xv4b);
        CHECK(should_not_reach1);
    }catch(option_error& oe){
        got_expected_exception1 = true;
        EQUAL(std::string(oe.what()), "argument required for option: --verify-something");
        if (_main) complain(oe, "setopts_from_xv4:");
    }
    if (_main) {
        DIAG(_main, "--- after xv4:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    CHECK(got_expected_exception1);

    const char *xv5[] = {"prognamexv5", "--help=10", "bleep"};
    bool got_expected_exception2 = false, should_not_reach2 = false;
    try{
        leftover = op.setopts_from_argv(sizeof(xv5)/sizeof(xv5[0]), (char **) xv5);
        CHECK(should_not_reach2);
    }catch(option_error& oe){
        got_expected_exception2 = true;
        EQUAL(string(oe.what()), "unexpected argument for option: --help");
        if (_main) complain(oe, "setopts_from_xv5:");
    }
    if (_main) {
        DIAG(_main, "--- after xv5:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    CHECK(got_expected_exception2);

    const char *xv6[] = {"prognamexv6", "--u321=99", ""};
    leftover = op.setopts_from_argv(sizeof(xv6)/sizeof(xv6[0]), (char **) xv6);
    if (_main) {
        DIAG(_main, "--- after xv6:\n");
        printopts();
        DIAG(_main, "--- leftover: " << strbe(leftover) << "\n--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default
    EQUAL(leftover.size(), 2);
    EQUAL(leftover[0], "--u321=99");
    EQUAL(leftover[1], "");

    op.setopts_from_env(TEST_PREFIX);
    if (_main) {
        DIAG(_main, "--- after env:\n");
        printopts();
        DIAG(_main, "--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(u32, 101); // default
    EQUAL(path1, "/x"); // default

#define ENAME TEST_PREFIX "PATH1"
    string xv7env{ENAME "="};
    xv7env += "yz";
    putenv((char *)xv7env.c_str());
    DIAG(_main, ENAME << "=" << getenv(ENAME));
    op.setopts_from_env(TEST_PREFIX);
    if (_main) {
        DIAG(_main, "--- after putenv:\n");
        printopts();
        DIAG(_main, "--- \n");
    }
    EQUAL(help, true); // still true
    EQUAL(u64, 0xfeeeeeeeeeeeeeee); // value from xv1
    EQUAL(u32, 101); // default
    EQUAL(path1, "yz");
    EQUAL(path2, ""); //default

    bool got_expected_exception3 = false, should_not_reach3 = false;
#define UNAME TEST_PREFIX "U32"
    string xv8env{UNAME "="};
    xv8env += "yz";
    putenv((char *)xv8env.c_str());
    DIAG(_main, UNAME << "=" << getenv(UNAME));
    try{
        op.setopts_from_env(TEST_PREFIX);
        CHECK(should_not_reach3);
    }catch(option_error& oe){
        got_expected_exception3 = true;
        CHECK(strcmp(oe.what(), "option_error::set(u32, yz)") == 0);
        if (_main) complain(oe, "setopts_from_xv8:");
    }
    if (_main) {
        DIAG(_main, "--- after xv8:\n");
        printopts();
        DIAG(_main, "--- \n");
    }
    CHECK(got_expected_exception3);

    stringstream xv9ss;
    xv9ss << "--verify-something 99\n";
    xv9ss << "--u64= 0xfeeeeeeeeeeeeeee   \n";
    xv9ss << "--help  \n";
    xv9ss << "--path1 =   \" starts with a space\"\n";
    xv9ss << "--path2 = contains  embedded spaces and ends with quote\"   \n";
    xv9ss << "--path3 = \"abc d\n";
    xv9ss << "--path4 = \"\"starts and ends with quotes\"\"  \n";
    op.setopts_from_istream(xv9ss);
    EQUAL(path1, " starts with a space");
    EQUAL(path2, "contains  embedded spaces and ends with quote\"");
    EQUAL(path3, "\"abc d");
    EQUAL(path4, "\"starts and ends with quotes\"");

    // Can we make undefaulted options work?
    const char* xv10[] = {"main"};
    dependent_defaults(1, (char **)xv10, false, false);
    const char* xv11[] = {"main", "--foo=true"};
    dependent_defaults(2, (char **)xv11, true, true);
    const char* xv12[] = {"main", "--foo=false"};
    dependent_defaults(2, (char **)xv12, false, false);
    const char* xv13[] = {"main", "--foo=true", "--bar=false"};
    dependent_defaults(3, (char **)xv13, true, false);
    const char* xv14[] = {"main", "--foo=true", "--bar=true"};
    dependent_defaults(3, (char **)xv14, true, true);

    return utstatus();
}catch(std::exception& e){
    complain(e, "Caught exception in main");
    return 1;
 }
