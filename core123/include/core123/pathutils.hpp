#pragma once
// Various utilities to handle pathnames, dirs and files
#include <string>
#include <vector>
#include <limits.h> // PATH_MAX
#include <sys/stat.h> // mkdir
#include <core123/throwutils.hpp>
#include <core123/strutils.hpp>
#include <unistd.h>
#include <dirent.h>

namespace core123 {

// apath - return an absolute path corresponding to rel:
//  if rel starts with /, then return it.
//  else if rel is empty, return cwd(),
//  else return cwd()/rel.
inline std::string apath(const std::string& rel=std::string()){
    if( !rel.empty() && rel[0] == '/' )
        return rel;
    char buf[PATH_MAX];
    // don't use system_error_wrapper to preserve pathutils.hpp's header-only status
    if( ::getcwd(buf, sizeof(buf)) == (char *)0 )
        throw se(str("getcwd", (void*)buf, sizeof(buf)));
    if(rel.empty())
        return std::string(buf);
    else
        return std::string(buf) + "/" + rel;
}

// pathsplit - split a path into directory and file parts.
//  If the argument contains no slashes, then the returned
//  dirpart is empty.
inline std::pair<std::string, std::string> pathsplit(const std::string& p){
    auto last = p.find_last_of('/');
    if(last == std::string::npos)
        return make_pair( std::string(), p );
    else
        return make_pair( p.substr(0, last), p.substr(last+1) );
}

// _makedirs - used internally by makedirs.
// Preconditions:
//   p must be writable!  It may be modified.
//   p[len] == '\0' 
//   p[len-1] != '/'
inline int _makedirs(char *p, size_t len, int mode){
    int ret = ::mkdir(p, mode);
    if(ret==0 || errno != ENOENT)
        return ret;
    // trust that fiddling with string_view won't set errno!
    auto lastslash = core123::str_view(p, len).find_last_of('/');
    if(lastslash == core123::str_view::npos)
        return ret;  // no slashes.  No parent to try
    // Check for a group of lastslashes
    auto lastnotslash = core123::str_view(p, lastslash).find_last_not_of('/');
    // We're done if p is "/xyz" or "///xyz"
    if(lastnotslash == core123::str_view::npos)
        return ret;
    lastslash = lastnotslash+1; // actually, first_slash_in_last_group_of_slashes
    p[lastslash] = '\0';
    ret = _makedirs(p, lastslash, mode);
    if(ret != 0 && errno != EEXIST)
        return ret;
    p[lastslash] = '/';
    return ::mkdir(p, mode);
}

// we'd like an sew::makedirs, but we don't want to put it in
// system_error_wrapper to keep sew.hpp for only system call analogs,
// not for improved APIs that we prefer

// makedirs - like python's os.makedirs.  Call mkdir, but if it fails
//  with ENOENT, try to recursively create parent directories.  If an
//  error occurs, throw with errno set by the last mkdir that failed
//  (which may have been an attempt to create a parent of the
//  argument).
inline void makedirs(core123::str_view d, int mode){
    // make a temporary, writeabl copy of d, up to, but not including
    // any trailing slashes to hand over to _makedirs.
    auto lastnotslash = d.find_last_not_of('/');
    if(lastnotslash == core123::str_view::npos){
        // It's all slashes.  Should we call ::mkdir,
        // or are we confident that we know the result.
        throw se(EEXIST, strfunargs("makedirs", d, mode));
    }
    auto irrelevant = lastnotslash + 1; // makedirs doesn't need to see this
    std::vector<char> vc(d.data(), d.data()+irrelevant+1);
    vc[irrelevant] = '\0';
    if (_makedirs(vc.data(), irrelevant, mode) != 0)
        throw se(strfunargs("makedirs", d, mode));
}

// Some transformations between st_mode (in stat) and d_type (in dirent)
// S_IFMT is 0170000.  
static const unsigned ifmtbits = 4;
static const unsigned ifmtfirstbit = 12;
static_assert( S_IFMT == ((1<<ifmtbits)-1)<<ifmtfirstbit, "S_FMTBITS is goofy");
// filetype maps a mode into a 4-bit representation of the permission bits.
inline unsigned filetype(mode_t mode){
    return (mode&S_IFMT)>>ifmtfirstbit;
}

inline mode_t dtype_to_mode(int dtype){
    switch(dtype){
    case DT_REG: return S_IFREG;
    case DT_DIR: return S_IFDIR;
    case DT_LNK: return S_IFLNK;
    case DT_BLK: return S_IFBLK;
    case DT_CHR: return S_IFCHR;
    case DT_FIFO: return S_IFIFO;
    case DT_SOCK: return S_IFSOCK;
    case DT_UNKNOWN:
    default:
        return 0;
    }
}

inline mode_t mode_to_dtype(mode_t mode){
    switch(mode & S_IFMT){
    case S_IFREG: return DT_REG;
    case S_IFDIR: return DT_DIR;
    case S_IFLNK: return DT_LNK;
    case S_IFBLK: return DT_BLK;
    case S_IFCHR: return DT_CHR;
    case S_IFIFO: return DT_FIFO;
    case S_IFSOCK: return DT_SOCK;
    default:
        return DT_UNKNOWN;
    }
}

} // namespace core123
