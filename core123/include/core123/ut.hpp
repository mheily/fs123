#pragma once
#include <iostream>
#include <core123/diag.hpp>
#include <core123/strutils.hpp>

// Very minimal unit test header with some handy macros
// to test and keep count of failures, and
// function to print results and return exit status.

namespace {
static auto _ut = core123::diag_name("ut");
static unsigned utfail = 0, utpass = 0;

#define EQUAL(x, y) if ((x) != (y)) { utfail++; std::cerr << __LINE__ << ": FAILED " #x " " << core123::ins(x) << " != " #y " " << core123::ins(y) << std::endl; } else {utpass++; DIAG(_ut, "PASSED " #x " " << core123::ins(x) << " == " #y " " << core123::ins(y));}
#define NOTEQUAL(x, y) if ((x) == (y)) { utfail++; std::cerr << __LINE__ << ": FAILED " #x " line " << __LINE__ << core123::ins(x) << " != " #y " " << core123::ins(y) << std::endl; } else {utpass++; DIAG(_ut, "PASSED " #x " " << core123::ins(x) << " == " #y " " << core123::ins(y));}
#define CHECK(expr) if(expr) { utpass++; DIAG(_ut, "PASSED " #expr " is true");} else {utfail++; std::cerr << __LINE__ << ": FAILED " << #expr << " is false\n";}

#define EQSTR(x, y) _EQSTR(x, y, #x)
inline void _EQSTR(const std::string& x, const std::string& y, const char *xexpr){
    if ((x) != (y)) {
        utfail++;
        std::cerr << "FAILED " << xexpr << "-> '" << x << "' != " << (y) << std::endl;
    } else {
        utpass++; DIAG(_ut, "PASSED " << xexpr << "-> '" << x << "' == '" << y << "'");
    }
}

// returns 0 if all tests passed, 1 if some tests failed.
// prints some chatter to stdout if verbose is true
inline int utstatus(bool verbose = true) {
    if (verbose) {
        std::cout << (utfail == 0 ? "OK, All " : "ERROR "); 
        std::cout << utpass << " tests passed, " << utfail << " failed." << std::endl;
    }
    return utfail == 0 ? 0 : 1;
}

} // namespace <anon>
