#include <core123/envto.hpp>
#include <core123/ut.hpp>
#include <optional>

int main(int, char **) {
    auto x = core123::envto<long>("WINDOWID", -1);
    if (x == -1) {
        std::cout << "no WINDOWID" << std::endl;
    } else {
        std::cout << "WINDOWID=" << x << std::endl;
    }
    const char *testname = "_UT_ENVTO_TEST_";
    const char *testval = "hello world";
    ::setenv(testname, testval, 1);
    auto xenv = core123::envto<std::string>(testname);
    EQSTR (xenv, testval);

    // test the std::optional specialization
    auto foo = core123::envto<std::optional<int>>("bogus");
    CHECK(!foo);
    ::setenv("i", "11", 1);
    foo = core123::envto<std::optional<int>>("i");
    CHECK(foo);
    EQUAL(*foo, 11);
    
    // and that the std::string specialization also works
    std::optional<std::string> bar;
    CHECK(!bar);
    ::setenv("bar", "a string with spaces", 1);
    bar = core123::envto<std::optional<std::string>>("bar");
    CHECK(bar);
    EQUAL(*bar, "a string with spaces");

    return utstatus();
}
