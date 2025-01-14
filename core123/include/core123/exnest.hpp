// C++11 supports "nested" exceptions.  These provide an easy-to-use
// mechanism for annotating and rethrowing exceptions.  They do much
// of what the far more elaborate boost::exceptions do, but with a lot
// less visible machinery.  Throwing and annotating exceptions is
// trivially easy.  Unpacking a nested exception is a bit tricky, and
// is facilitated by the 'exnest' and 'rexnest' factory functions
// defined in this file.
//
// Usage:
//  - In your code, you throw exceptions derived from std::exception.
//    E.g.,
//      if(something_bad)
//         throw std::runtime_error("Uh oh.  Something bad happened");
//
//    The thrown object need not be a std::runtime_error.  It can be
//    any type derived from std::exception, e.g., std::system_error,
//    a user-defined exception type, etc.
//
//  - When you catch an exception and wish to rethrow it with a
//    "nested annotation", you say:
//      catch(std::exception& e){
//         ...
//         std::throw_with_nested( std::runtime_error("more information") );
//      }
//
//    Again, the rethrown object need not be a std::runtime_error.  It
//    can be any type derived from std::exception.  Similarly, the
//    caught exception need not be std::exception, but may be any
//    exception type that matches the circumstance.
//
//  - When you catch an error and wish to finally dispose of it, it
//    might contain one or more of these fancy "nested annotations".
//    Use the functions defined here to unpack it.  If all you want is
//    the innermost (original) exception, use innermost:
//
//    catch(std::exception& e){
//       std::exception& inner = innermost(e);
//       ...
//    }
//
//    Iterate through the entire stack of nested exceptions them with
//    exnest:
//
//    catch(std::exception& e){
//       for(auto& a : exnest(e)){
//          std::cout << a.what() << "\n";
//       }
//    }
//
//    Don't forget to make auto& a reference unless you really want to
//    copy-construct each iterate.
// 
//    exnest iterates from the outermost "nested annotation" to the
//    innermost original exception.  If there are no nested
//    annotations (i.e., e is a plain-old thrown exception with no
//    fancy rethrow_with_nested invocations during unwinding), then
//    the only iterate is a reference to e itself.
//
//    To iterate from inner to outer, use rexnest.  E.g.,
//
//    catch(std::exception& e){
//       for(auto& a : rexnest(e)){
//          std::cout << a.what() << "\n";
//       }
//    }
//
//    Note that the exnest and rexnest iterators will call
//    std::terminate() if any of the the nested extensions within e
//    contain a null nested_ptr.  This can happen if throw_with_nested
//    was called outside of a catch handler.  Don't do that.

#pragma once
#include <iterator>
#include <stdexcept>
#include <exception>
#include <vector>
#include <functional>
#include <string>
#include <mutex>
#include <atomic>
#include <syslog.h>

namespace core123{
// This seems to be one of those cases where you have to implement
// const and non-const versions of everything.  Avoid writing the same
// code twice by templating _exnest over a type, and then providing an
// overloaded function exnest that takes const and non-const
// references to std::exception and returns an analogous _exnest.
template <class EXTYPE>
struct _exnest{
    EXTYPE& exref;
    _exnest(EXTYPE& e) : exref(e){}
    class iterator : public std::iterator<
        std::forward_iterator_tag,
        EXTYPE // value_type
        >{
    public:
        typename iterator::pointer ep;
        explicit iterator(typename iterator::pointer _ep = nullptr) : ep(_ep){}
        iterator& operator++(){ // ++It
            try{
                std::rethrow_if_nested(*ep);
                ep = nullptr;
            }catch(typename iterator::reference nested){
                ep = &nested;
            }
            return *this;
        }
        iterator operator++(int){ // It++
            iterator ret = *this;
            ++(*this);
            return ret;
        }
        bool operator==(iterator other) const { return ep == other.ep; }
        bool operator!=(iterator other) const { return !(*this == other); }
        typename iterator::reference operator*() const { return *ep; }
    };

    iterator begin(){
        return iterator(&exref);
    }
    iterator end(){
        return iterator();
    }
};

inline _exnest<std::exception>
exnest(std::exception& e){
    return {e};
}

inline _exnest<const std::exception>
exnest(const std::exception& e){
    return {e};
}

// I don't see how to iterate from inner to outer without first
// constructing a vector from outer to inner, and then iterating
// over it in reverse...
template<class EXTYPE>
class _rexnest{
    using vrwex = std::vector<EXTYPE*>;
    vrwex ev;
    using vrwexri = typename vrwex::reverse_iterator;
public:
    _rexnest(EXTYPE& ex){
        for(auto& n : exnest(ex))
            ev.push_back(&n);
    }
    // our iterator IS A reverse iterator on ev, with an additional dereference.
    struct iterator : public vrwexri{
        iterator(const vrwexri& _base) : vrwexri(_base){}
        EXTYPE& operator*() const { return *vrwexri::operator*(); }
    };
    iterator begin(){
        return ev.rbegin();
    }
    iterator end(){
        return  ev.rend();
    }
};

inline _rexnest<std::exception>
rexnest(std::exception& e){
    return {e};
}

inline _rexnest<const std::exception>
rexnest(const std::exception& e){
    return {e};
}

// innermost also needs the const/non-const treatment, but it's
// so short that it doesn't seem worth templating.
inline std::exception&
innermost(std::exception& e){
    try{
        std::rethrow_if_nested(e);
    }catch(std::exception& nested){
        return innermost(nested);
    }
    return e;
}

inline const std::exception&
innermost(const std::exception& e){
    try{
        std::rethrow_if_nested(e);
    }catch(const std::exception& nested){
        return innermost(nested);
    }
    return e;
}

} // namespace core123
