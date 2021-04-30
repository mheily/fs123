#pragma once

// Tiny helpers that throw a logic error when given a null pointer.

// Any code that needs these already has a bit of a code-smell - but
// it's better than segfaulting...

// Usage:
//
// Suppose you have some code like this:
//
//  if(this && that || whatever){
//      ...
//      p->method();
//      somefunc(*p);
//      ...
//  }
//
// and it's *supposed to be the case* that this && that || whatver
// implies that p is non-null.  But the logic that guaranteed that
// happened long ago and far away, and you're not sure you can trust
// all the possible ways that things might have been touched between
// then and now.  If you're wrong, the code segfaults, which is pretty
// undesirable.  Instead, you can write:
//
//  if(this && that || whatever){
//    ...
//    non_null_or_throw(p)->method();
//    somefunc(deref_or_throw(p));
//    ...
//  }
//
//  Now, if you're wrong, the code throws a logic_error, which is
//  generally better than segfaulting.
//
//  Each call to ..._or_throw(p) is roughly the same as having typed:
//     if(!p)
//        throw logic_error("...");
//     ...p...
//  but it's a lot less to type.
//
//  Note that if p is *never* NULL, then you'd be better off with
//  something like not_null from:
//    https://isocpp.github.io/CppCoreGuidelines/CppCoreGuidelines#Rf-nullptr
//  

#include <stdexcept>

namespace core123{
template <typename T, typename E=std::logic_error>
auto& non_null_or_throw(T& t){
    if(!t)
        throw E("null pointer where non-null was expected");
    return t;
}

template <typename T, typename E=std::logic_error>
auto& non_null_or_throw(const T& t){
    if(!t)
        throw E("null pointer where non-null was expected");
    return t;
}

template  <typename T, typename E=std::logic_error>
auto& deref_or_throw(T& t){
    return *non_null_or_throw<T, E>(t);
}

template  <typename T, typename E=std::logic_error>
auto& deref_or_throw(const T& t){
    return *non_null_or_throw<T, E>(t);
}
}
