#include "core123/pathutils.hpp"
#include "core123/sew.hpp"
#include "core123/autoclosers.hpp"
#include "core123/ut.hpp"
#include <iostream>

namespace sew = core123::sew;
namespace ac = core123::ac;

void test_makedirs(){
    // N.B.  If this fails, use strace to see what it's doing,
    // which usually makes the error obvious.

    // First, let's check a couple of pathological calls..
    try {
        core123::makedirs("///", 0777);
	CHECK(false);
    } catch (std::system_error& xe) {
	// Linux returns EEXIST, but MacOS (and perhaps others)
	// returns EISDIR
	auto ev = xe.code().value();
        CHECK (ev==EEXIST || ev==EISDIR);
	if(ev == EEXIST)
	    core123::makedirs("///", 0777, true); //shouldn't throw
    }

    try {
        core123::makedirs("", 0777);
    } catch (std::system_error& xe) {
        // According to POSIX, mkdir("") sets errno to ENOENT
        EQUAL(xe.code().value(), ENOENT);
    } 

    char name[] = "/tmp/test_makedirsXXXXXX";
    sew::mkdtemp(name);
    std::string name_s(name);
    core123::makedirs(name_s + "///abc", 0777);
    try {
        core123::makedirs(name_s + "/abc/", 0777);
        CHECK(false);
    } catch (std::system_error& xe) {
        EQUAL (xe.code().value(), EEXIST);
    }
    core123::makedirs(name_s + "/abc/", 0777,  true);

    core123::makedirs(name_s + "/abc/def", 0777);
    core123::makedirs(name_s + "/abc/hij/klm/nop", 0777);
    // Make some with relative paths
    char cwd[PATH_MAX+1];
    sew::getcwd(cwd, sizeof(cwd));
    sew::chdir(name);
    core123::makedirs("xyz/uvw", 0777);
    core123::makedirs("xyz///mnop//pqr/", 0777);
    sew::chdir(cwd);
    
    // check that we really do get an EEXIST when a path component
    // is not a directory, even with exist_ok=true
    sew::close(sew::open((name_s + "/xyz///mnop//pqr/file").c_str(), O_CREAT|O_WRONLY, 0666));
    try{
        core123::makedirs(name_s + "/xyz///mnop//pqr/file", true);
        CHECK(false);
    }catch(std::system_error& xe){
        EQUAL (xe.code().value(), EEXIST);
    }
    try{
        core123::makedirs(name_s + "/xyz///mnop//pqr/file/wontwork", true);
        CHECK(false);
    }catch(std::system_error& xe){
        EQUAL (xe.code().value(), ENOTDIR);
    }
    sew::unlink((name_s + "/xyz///mnop//pqr/file").c_str());

    core123::sew::chmod((name_s + "/abc/hij").c_str(), 0500);
    try {
        core123::makedirs(name_s + "/abc//hij/xyzz/", 0777);
        CHECK(geteuid() == 0); // root doesn't get EACCES
        sew::rmdir((name_s + "/abc//hij/xyzz/").c_str());
    } catch (std::system_error& xe) {
        EQUAL (xe.code().value(), EACCES);
    }
    // Clean up.  Also checks that directories were created.
    sew::rmdir((name_s + "/xyz/mnop/pqr").c_str());
    sew::rmdir((name_s + "/xyz/mnop").c_str());
    sew::rmdir((name_s + "/xyz/uvw").c_str());
    sew::rmdir((name_s + "/xyz").c_str());
    sew::rmdir((name_s + "/abc/hij/klm/nop").c_str());
    sew::chmod((name_s + "/abc/hij").c_str(), 0700);
    sew::rmdir((name_s + "/abc/hij/klm").c_str());
    sew::rmdir((name_s + "/abc/hij").c_str());
    sew::rmdir((name_s + "/abc/def").c_str());
    sew::rmdir((name_s + "/abc").c_str());

    //
    // Now do it all again with makedirsat...
    //
    ac::fd_t<> namefd = sew::open(name, O_DIRECTORY);
    core123::makedirsat(namefd, "abc", 0777);
    try {
        core123::makedirsat(namefd, "abc/", 0777);
        CHECK(false);
    } catch (std::system_error& xe) {
        EQUAL (xe.code().value(), EEXIST);
    }
    core123::makedirsat(namefd, "abc/", 0777,  true);
    
    core123::makedirsat(namefd, "abc/def", 0777);
    core123::makedirsat(namefd, "abc/hij/klm/nop", 0777);
    // Make some with relative paths
    sew::getcwd(cwd, sizeof(cwd));
    sew::chdir(name);
    core123::makedirsat(AT_FDCWD, "xyz/uvw", 0777);
    core123::makedirsat(AT_FDCWD, "xyz///mnop//pqr/", 0777);
    sew::chdir(cwd);
    
    // check that we really do get an EEXIST when a path component
    // is not a directory, even with exist_ok=true
    sew::close(sew::openat(namefd, "xyz///mnop//pqr/file", O_CREAT|O_WRONLY, 0666));
    try{
        core123::makedirsat(namefd, "xyz///mnop//pqr/file", true);
        CHECK(false);
    }catch(std::system_error& xe){
        EQUAL (xe.code().value(), EEXIST);
    }
    try{
        core123::makedirsat(namefd, "xyz///mnop//pqr/file/wontwork", true);
        CHECK(false);
    }catch(std::system_error& xe){
        EQUAL (xe.code().value(), ENOTDIR);
    }
    sew::unlinkat(namefd, "xyz///mnop//pqr/file", 0);

    core123::sew::fchmodat(namefd, "abc/hij", 0500, 0);
    try {
        core123::makedirsat(namefd, "abc//hij/xyzz/", 0777);
        CHECK(geteuid() == 0); // root doesn't get EACCES
        sew::unlinkat(namefd, "abc//hij/xyzz/", AT_REMOVEDIR);
    } catch (std::system_error& xe) {
        EQUAL (xe.code().value(), EACCES);
    }
    // Clean up.  Also checks that directories were created.
    sew::unlinkat(namefd, "xyz/mnop/pqr", AT_REMOVEDIR);
    sew::unlinkat(namefd, "xyz/mnop", AT_REMOVEDIR);
    sew::unlinkat(namefd, "xyz/uvw", AT_REMOVEDIR);
    sew::unlinkat(namefd, "xyz", AT_REMOVEDIR);
    sew::unlinkat(namefd, "abc/hij/klm/nop", AT_REMOVEDIR);
    sew::fchmodat(namefd, "abc/hij", 0700, 0);
    sew::unlinkat(namefd, "abc/hij/klm", AT_REMOVEDIR);
    sew::unlinkat(namefd, "abc/hij", AT_REMOVEDIR);
    sew::unlinkat(namefd, "abc/def", AT_REMOVEDIR);
    sew::unlinkat(namefd, "abc", AT_REMOVEDIR);
    
    sew::rmdir(name);
}

void chk_pathsplit(const char* in, const char* dexpect, const char* fexpect){
    auto [dpart, fpart] = core123::pathsplit(in);
    EQUAL(dpart, dexpect);
    EQUAL(fpart, fexpect);
}

void test_pathsplit(){
    chk_pathsplit("foo/bar/baz", "foo/bar", "baz");
    chk_pathsplit("foo/bar", "foo", "bar");
    chk_pathsplit("foo/", "foo", "");
    chk_pathsplit("foo", "", "foo");
    // pathsplit can't tell the difference between "foo" and "/foo"!
    chk_pathsplit("/foo", "", "foo");
    chk_pathsplit("foo", "", "foo");
    // pathsplit doesn't recognize the semantics of multiple slashes.
    chk_pathsplit("foo//bar", "foo/", "bar");
    chk_pathsplit("//foo///bar//baz", "//foo///bar/", "baz");
}


void chk_sv_pathsplit(const char* in, std::optional<core123::str_view> dexpect, core123::str_view fexpect){
    auto [dpart, fpart] = core123::sv_pathsplit(in);
    EQUAL(dpart, dexpect);
    EQUAL(fpart, fexpect);
}
    
void test_sv_pathsplit(){
    chk_sv_pathsplit("foo/bar", "foo", "bar");
    chk_sv_pathsplit("foo/", "foo", "");
    chk_sv_pathsplit("/foo", "", "foo");
    chk_sv_pathsplit("/", "", "");
    chk_sv_pathsplit("bar", {}, "bar");
    chk_sv_pathsplit("", {}, "");
    chk_sv_pathsplit("//a///b/foo//bar", "//a///b/foo", "bar");
    chk_sv_pathsplit("//a///b/foo//", "//a///b/foo", "");
    chk_sv_pathsplit("//a///b//foo", "//a///b", "foo");
    chk_sv_pathsplit("//a///b//", "//a///b", "");
    chk_sv_pathsplit("//a", "", "a");
    chk_sv_pathsplit("///", "", "");
}

int main(int, char **){
    test_makedirs();
    test_pathsplit();
    test_sv_pathsplit();
    return utstatus();
}
