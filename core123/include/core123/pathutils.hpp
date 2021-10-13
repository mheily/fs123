#pragma once
// Various utilities to handle pathnames, dirs and files
#include <string>
#include <vector>
#include <utility>
#include <limits.h> // PATH_MAX
#include <fcntl.h>  // AT_FDCWD
#include <sys/stat.h> // mkdir
#include <core123/str_view.hpp>
#include <core123/throwutils.hpp>
#include <core123/strutils.hpp>
#include <unistd.h>
#include <dirent.h>

namespace core123 {

// apath - return an absolute path corresponding to rel:
//  if rel starts with /, then return it.
//  else if rel is empty, return cwd(),
//  else return cwd()/rel.
inline std::string apath(str_view rel={}){
    if( !rel.empty() && rel[0] == '/' )
        return std::string(rel);
    char buf[PATH_MAX];
    if( ::getcwd(buf, sizeof(buf)) == (char *)0 )
        throw se(str("getcwd", (void*)buf, sizeof(buf)));
    if(rel.empty())
        return std::string(buf);
    else
        return std::string(buf) + "/" + std::string(rel);
}

// pathsplit - Deprecated.  Use sv_pathsplit instead.  Split a path
//  into directory and file parts.  If the argument contains no
//  slashes, then the returned dirpart is empty.  Otherwise, the dir
//  part is everything before the last slash and the file part is
//  everything after the last slash.  Multiple slashes get no special
//  handling.  Also note that pathsplit("foo") is indistinguishable
//  from pathsplit("/foo").
//      pathsplit("foo/bar/baz") -> ("foo/bar", "baz")
//      pathsplit("foo/bar") -> ("foo", "bar")
//      pathsplit("foo/") -> ("foo", "")
//      pathsplit("foo")  -> ("", "foo")
//      // pathsplit can't tell the difference between "foo" and "/foo"!
//      pathsplit("/foo")    -> ("", "foo")
//      pathsplit("foo")     -> ("", "foo")
//      // pathsplit doesn't recognize the semantics of multiple slashes.
//      pathsplit("foo//bar") -> ("foo/", "bar")
//      pathsplit("//foo///bar//baz") -> ("//foo///bar/", "baz")
inline std::pair<std::string, std::string> pathsplit(const std::string& p){
    auto last = p.find_last_of('/');
    if(last == std::string::npos)
        return std::make_pair( std::string(), p );
    else
        return std::make_pair( p.substr(0, last), p.substr(last+1) );
}

// sv_pathsplit - splits its argument into a dirpart and a filepart.
//   Returns a pair<optional<str_view>, str_view>.  When the argument
//   contains no slashes, the dirpart of the returned value is an
//   empty std::optional and the filepart is the argument.  Otherwise,
//   the dirpart is whatever came before the last contiguous group of
//   slashes and the filepart is whatever came after the last
//   contiguous group of slashes, both of which may be the empty
//   string.
//
//   The dirpart never ends with a slash.  The filepart never contains
//   a slash.
//
//   sv_pathsplit("foo/bar") -> ("foo", "bar")
//   sv_pathsplit("foo/")    -> ("foo", "")
//   sv_pathsplit("/foo")    -> ("", foo")
//   sv_pathsplit("/")       -> ("", "")
//   sv_pathsplit("")        -> ({}, "")
//   sv_pathsplit("foo")     -> ({}, "foo")
//
//   Note that in the last two cases, the first value returned is an
//   empty std::optional, distinct from a non-empty std::optional that
//   contains an empty str_view (the previous two cases).
//
//   Also note that only the *last* contiguous group of slashes is
//   treated as a group:
//
//   sv_pathsplit("//a///b/foo//bar") -> ("//a///b/foo", "bar")
//   sv_pathsplit("//a///b/foo//")    -> ("//a///b/foo", "")
//   sv_pathsplit("//a///b//foo")     -> ("//a///b", foo")
//   sv_pathsplit("//a///b//")        -> ("//a///b", "")
//   sv_pathsplit("//a")              -> ("", "a")
//   sv_pathsplit("///")              -> ("", "")
//
//   Note that core123::rsplit1(str_view) is the same as sv_pathsplit,
//   except that it does not treat multiple slashes as a single
//   separator.
inline std::pair<std::optional<str_view>, str_view>
sv_pathsplit(str_view p){
    auto ret = rsplit1(p, '/');
    static_assert(str_view::npos+1U == 0, "Hang on.  Isn't str_view::npos == -1?");
    if(ret.first)
        *ret.first = ret.first->substr(0, ret.first->find_last_not_of('/')+1U);
    return ret;
}

// _makedirsat - used internally by makedirs.
// Preconditions:
//   p must be writable!  It may be modified.
//   p[len] == '\0'       It must be a NUL-terminated string of length  len
//   len==0 || p[len-1] != '/' It must not have trailing slashes
inline int _makedirsat(int dirfd, char *p, size_t len, int mode){
    int ret = ::mkdirat(dirfd, p, mode);
    if(ret==0 || errno != ENOENT)
        return ret;
    auto lastslash = core123::str_view(p, len).find_last_of('/');
    if(lastslash == core123::str_view::npos)
        return ret;  // no slashes.  No parent to try
    // Check for a group of lastslashes
    auto lastnotslash = core123::str_view(p, lastslash).find_last_not_of('/');
    // We're done if there's nothing to the left of the last group of slashes,
    // i.e., if p is "/xyz" or "///xyz"
    if(lastnotslash == core123::str_view::npos)
        return ret;
    lastslash = lastnotslash+1; // actually, first_slash_in_last_group_of_slashes
    p[lastslash] = '\0';
    ret = _makedirsat(dirfd, p, lastslash, mode|S_IWUSR);
    p[lastslash] = '/';
    if(ret != 0 && errno != EEXIST)
        return ret;
    return ::mkdirat(dirfd, p, mode);
}

// makedirsat - inspired by python's os.makedirs.  Call mkdirat, but
//  if it fails with ENOENT, try to recursively create parent
//  directories with mode=mode|S_IWUSR.  If an error occurs, throw a
//  system_error with errno set by the last mkdirat that failed (which
//  may have been an attempt to create a parent of the argument).  If
//  an exception is thrown, the filesystem may be left in a modified
//  state (some parent directories created).  If a parent mkdirat fails
//  with EEXIST, ignore it.

//  If exist_ok is true, then if the final mkdir fails with EEXIST,
//  and if the final path satisfies S_ISDIR when fstatat'ed, then
//  consider the result a success and do not throw.
//
//  In other words - when called with exist_ok=true, it either throws
//  or satisfies the post-condition that the specified path exists and
//  is a directory.  N.B. the post-condition says nothing about the
//  ownership or mode of the specified path or any of its parents.
//
//  When called with exist_ok=false, it either throws or satisfies the
//  post-condition that the specified path was created by the caller
//  with the given mode.  N.B.  the post-condition says nothing about
//  the ownership or mode of the specified path's parents.
//
//  Note that the path argument is a by-value std::string, not a const
//  reference.  Callers concerned about copies can pass an rvalue
//  reference, e.g., std::move.
//  
inline void makedirsat(int dirfd, std::string path, int mode, bool exist_ok = false){
    auto lastnotslash = path.find_last_not_of('/');
    auto pathlen = path.size();
    if(lastnotslash < pathlen){
        // Trailing slashes will confuse _makedirsat
        pathlen = lastnotslash;
        path[pathlen+1] = '\0';
    }
    // Corner cases:
    // - if path is empty, pathlen is zero and _makedirsat calls
    //   calls mkdirat(dirfd, "", mode),  which returns ENOENT.
    // - if path consists of nothing but one or more slashes,
    //   _mkdirsat calls mkdirat(dirfd, "///", mode), which
    //   returns EEXIST
    if (_makedirsat(dirfd, &path[0], pathlen, mode) != 0){
        auto eno = errno;
        struct stat sb;
        if(eno == EEXIST && exist_ok && ::fstatat(dirfd, path.data(), &sb, 0)==0 && S_ISDIR(sb.st_mode))
            return;
        throw se(eno, strfunargs("makedirsat", dirfd, path, mode));
    }
}

// makedirs - fall through to makedirsat, inspired by python's os.makedirs.
inline void makedirs(std::string path, int mode, bool exist_ok = false){
    return makedirsat(AT_FDCWD, std::move(path), mode, exist_ok);
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
