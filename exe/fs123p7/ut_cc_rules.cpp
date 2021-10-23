#include "exportd_cc_rules.hpp"
#include <core123/autoclosers.hpp>
#include <core123/exnest.hpp>
#include <core123/ut.hpp>
#include <core123/sew.hpp>
#include <core123/unused.hpp>
#include <iostream>

using namespace core123;

void check(const std::string& cc, time_t age, const std::string& expected){
    struct stat sb{};
    sb.st_mtime = sew::time(nullptr) - age;
    std::string result = cc_rule_cache::bounded_max_age(cc, sb);
    EQUAL(result, expected);
}

int main(int argc, char **argv) try {
    // A reasonable test of cc-rules requires setting
    // up a directory with some target files and some
    // rules files and running queries.  A very rudimentary
    // attempt at that is in TOP/tests/t_14ccrules.
    //
    // For now, all we do here is run some tests on the
    // cc_rules_cache::bounded_max_age member function.
    unused(argc, argv);
    check("max-age=99", 999, "max-age=99");
    check("max-age=99", 17, "max-age=17");
    check("max-age=99", 5, "max-age=5");
    check("max-age=99", 1, "max-age=1");
    check("max-age=99", 0, "max-age=1");
    check("max-age=99", -1, "max-age=1");
    check("max-age=99", -99, "max-age=1");
    
    // Don't be fooled by s-max-age
    check("s-max-age=77, max-age = 99", 999, "s-max-age=77, max-age = 99");
    check("s-max-age=77, max-age = 99", 17, "s-max-age=77, max-age = 17");
    check("s-max-age=77, max-age = 99", 5, "s-max-age=77, max-age = 5");
    check("s-max-age=77, max-age = 99", 1, "s-max-age=77, max-age = 1");
    check("s-max-age=77, max-age = 99", 0, "s-max-age=77, max-age = 1");
    check("s-max-age=77, max-age = 99", -1, "s-max-age=77, max-age = 1");
    check("s-max-age=77, max-age = 99", -99, "s-max-age=77, max-age = 1");
    
    // Or by max-agent
    check("max-agent=77, max-age = 99", 999, "max-agent=77, max-age = 99");
    check("max-agent=77, max-age = 99", 17, "max-agent=77, max-age = 17");
    check("max-agent=77, max-age = 99", 5, "max-agent=77, max-age = 5");
    check("max-agent=77, max-age = 99", 1, "max-agent=77, max-age = 1");
    check("max-agent=77, max-age = 99", 0, "max-agent=77, max-age = 1");
    check("max-agent=77, max-age = 99", -1, "max-agent=77, max-age = 1");
    check("max-agent=77, max-age = 99", -99, "max-agent=77, max-age = 1");

    // Try a variety of combinations of whitespace, beginning, end, etc.
    // (not necessarily well-formed  cache-control strings)
    check("max-age=99", 5, "max-age=5");
    check(",max-age=99", 5, ",max-age=5");
    check(",max-age=99", 5, ",max-age=5");
    check(" max-age=99 ", 5, " max-age=5 ");
    check(" max-age =99 ", 5, " max-age =5 ");
    check(" max-age   =99 ", 5, " max-age   =5 ");
    check(" max-age   = 99 ", 5, " max-age   = 5 ");
    check(" max-age   =    99 ", 5, " max-age   =    5 ");

    check("public,max-age=99", 5, "public,max-age=5");
    check("public,,max-age=99", 5, "public,,max-age=5");
    check("public,,max-age=99", 5, "public,,max-age=5");
    check("public, max-age=99 ", 5, "public, max-age=5 ");
    check("public, max-age =99 ", 5, "public, max-age =5 ");
    check("public, max-age   =99 ", 5, "public, max-age   =5 ");
    check("public, max-age   = 99 ", 5, "public, max-age   = 5 ");
    check("public, max-age   =    99 ", 5, "public, max-age   =    5 ");
    
    check("public,max-age=99, stale-while-revalidate = 33", 5, "public,max-age=5, stale-while-revalidate = 33");
    check("public,,max-age=99, stale-while-revalidate = 33", 5, "public,,max-age=5, stale-while-revalidate = 33");
    check("public,,max-age=99, stale-while-revalidate = 33", 5, "public,,max-age=5, stale-while-revalidate = 33");
    check("public, max-age=99 , stale-while-revalidate = 33", 5, "public, max-age=5 , stale-while-revalidate = 33");
    check("public, max-age =99 , stale-while-revalidate = 33", 5, "public, max-age =5 , stale-while-revalidate = 33");
    check("public, max-age   =99 , stale-while-revalidate = 33", 5, "public, max-age   =5 , stale-while-revalidate = 33");
    check("public, max-age   = 99 , stale-while-revalidate = 33", 5, "public, max-age   = 5 , stale-while-revalidate = 33");
    check("public, max-age   =    99 , stale-while-revalidate = 33", 5, "public, max-age   =    5 , stale-while-revalidate = 33");

    // Try a few more corner cases 
    std::cerr << "This check is expected to produce a warning" << std::endl;
    check("max-agent=77, max-age = 999999999999999999999999999999", 888888, "max-agent=77, max-age = 888888");
    // even some that aren't well-formed cache-control strings.
    // I.e., don't throw, segfault or mangle them any further.
    check("public, max-age=", 999, "public, max-age=");
    check("public, max-agemax-age=99", 999, "public, max-agemax-age=99");
    check("public,max-age+=99", 999, "public,max-age+=99");

    return utstatus(true);
 }catch(std::exception& e){
    for(auto& m : exnest(e))
        std::cout << m.what() << "\n";
    exit(1);
 }


