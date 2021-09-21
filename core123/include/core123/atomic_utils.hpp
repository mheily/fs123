#pragma once

#include <atomic>

namespace core123{
// scoped_fetch_add: A handy way to increment and decrement a counter
//  with RAII scoping: The constructor performs a fetch_add, adding
//  its second argument (default 1) to its first.  The destructor
//  performs a corresponding fetch_sub.
//
//  Usage:
//
//  std::atomic<int> ctr; // any integral type is fine.
// 
// In many threads:
//  {
//     scoped_fetch_add sfa(ctr); // ctr is incremented by 1
//     ... // ok for this code to throw
//  }  // ctr is decremented by 1 when sfa goes out of scope
//  
// Public types and methods:
//  sfa.value_type   -> the value type of the underlying std:atomic
//  sfa.get_atomic() -> returns a reference to the underlying atomic object
//  sfa.get_fetched()-> returns the value that fetch_add returned when
//                      it was called by the constructor
//  sfa.get_added()  -> returns the amount that was added (the second
//                      constructor argument).
//
// atomic_max(std::atomic<T>& stored, const T& newval):
//     atomically replace stored with newval if newval is greater than stored.
//
// atomic_min(std::atomic<T>& stored, const T& newval):
//     atomically replace stored with newval if newval is less than stored.

template <typename IType>
class scoped_fetch_add{
public:
    using value_type = IType;
    scoped_fetch_add(std::atomic<value_type>& _aref, value_type _added=1) :
        aref(_aref), added(_added), fetched(_aref.fetch_add(_added))
    {}
    ~scoped_fetch_add(){
        aref.fetch_sub(added);
    }
    auto& get_atomic() const{
        return aref;
    }
    auto get_fetched() const{
        return fetched;
    }
    auto get_added() const{
        return added;
    }
private:
    std::atomic<value_type>& aref;
    value_type added;
    value_type fetched;
};
// Woohoo!  It's C++17 - we can use CTAD
template<class IType>
scoped_fetch_add(std::atomic<IType>, IType) -> scoped_fetch_add<IType>;

template <typename A>
static void atomic_max(std::atomic<A>& max_val, const A& newval){
    auto prev = max_val.load();
    while(prev < newval &&
          !max_val.compare_exchange_weak(prev, newval));
}

template <typename A>
static void atomic_min(std::atomic<A>& min_val, const A& newval){
    auto prev = min_val.load();
    while(prev > newval &&
          !min_val.compare_exchange_weak(prev, newval));
}

} // namespace core123
