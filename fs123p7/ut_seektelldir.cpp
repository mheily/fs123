#include "fs123/acfd.hpp"
#include <random>
#include <core123/sew.hpp>
#include <core123/ut.hpp>
#include <iostream>
#include <vector>
#include <cassert>
#include <algorithm>
#include <cstring>
#include <chrono>

using namespace core123;

#ifdef __linux__
#include <sys/syscall.h>
auto gd(int fd, void *buf, size_t bufsz){
    auto nread = ::syscall(SYS_getdents64, fd, buf, bufsz);
    if(nread < 0)
        throw se("SYS_getdents failed");
    return nread;
}

void test_getdents(const char *dirname){
    auto fd = sew::open(dirname, O_RDONLY|O_DIRECTORY);
    // The main point here is to confirm that even if
    // we call getdents with a large (32k) size argument,
    // the fuse layer only sees a readdir callback
    // with a 4k size.  That's "ok" because fs123
    // makes a 128k-byte http request, which it
    // buffers up for subsequent callbacks.
    char buffer[32768];
    auto nread = gd(fd, buffer, sizeof(buffer));
    if(nread < 0)
        throw se("Oops.  getdents failed");
    printf("getdents64 returned %zd\n", size_t(nread));
    sew::close(fd);
}    
#endif

int main(int argc, char **argv){
    const char *dirname = argc>1 ? argv[1] : ".";
#if 0 && defined __linux__
    test_getdents(dirname);
    return 0;
#endif
    acDIR dirp = sew::opendir(dirname);
    struct dirent* de;
    char space[sizeof(struct dirent) + 128] = {};
    struct dirent* eofdirent = (struct dirent*)&space[0];
    strcpy(eofdirent->d_name, "/EOF");
    std::cout.setf(std::ios::unitbuf);
    off_t last_d_off = 0;
    std::vector<std::pair<off_t, std::string> > offmap;
    std::mt19937 g; // default seed - always the same!
    size_t next_print = 20;
    size_t i = 0;
    do{
        auto off = sew::telldir(dirp);
        // check that the offsets reported by telldir
        // are 'consistent' with those in the dirent
        // returned by readdir.
        assert(off == last_d_off);
        //std::cout << "off = " << off;
        de = sew::readdir(dirp);
        if(!de)
            de = eofdirent;
#ifndef __APPLE__
        last_d_off = de->d_off;
#else
	last_d_off = sew::telldir(dirp);
#endif
        if(i<next_print/2 || i==next_print){
            if(i==next_print){
                std::cout << "...\n";
                next_print *= 2;
            }
            std::cout << " " << de->d_name << " de->d_ino=" << de->d_ino << " de->d_off=" << last_d_off << " de->d_type=" << (int)de->d_type << "\n";
        }
        i++;
        offmap.push_back( std::make_pair(off, de->d_name) );
    }while(de != eofdirent);
    
    // Now check that we can seek 'randomly' to the offsets
    // we extracted above:
    i = 0;
    bool done = false;
    auto start_time = std::chrono::system_clock::now();
    while(!done){
        std::cout << "Shuffle " << i++ << "\n";
        std::shuffle(offmap.begin(), offmap.end(), g);
        for(const auto& kv : offmap){
            sew::seekdir(dirp, kv.first);
            de = sew::readdir(dirp);
            if(de){
                //std::cout << kv.first << " " << kv.second << " " << de->d_name << "\n";
                EQUAL(kv.second, de->d_name);
            }else{
                //std::cout << kv.first << " " << kv.second << " " << "/EOF" << "\n";
                EQUAL(kv.second, "/EOF");
            }
            if(utfail)
                return utstatus(1);
            if((utpass%1000) == 0){
                if( (std::chrono::system_clock::now() - start_time) > std::chrono::seconds(5) ){
                    std::cout << "Time's up\n";
                    done = true;
                    break;
                }
            }
        }
    }
    return utstatus(1);
}
    
