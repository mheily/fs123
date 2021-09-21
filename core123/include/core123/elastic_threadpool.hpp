#pragma once

#include "producerconsumerqueue.hpp"
#include "strutils.hpp"
#include "complaints.hpp"
#include "atomic_utils.hpp"
#include <future>
#include <atomic>
#include <thread>
#include <stdexcept>
#include <optional>

// ELASTIC_THREADPOOL_FUZZ allows the source-file that #includes
// elastic_threadpool.hpp to introduce some "fuzzing" at strategic
// points in the code.  E.g., ut_elastic_threadpool can introduce
// "gratuitous" this_thread::sleep_for and/or this_thread::yield calls
// into elastic_threadpool's logic.  In production,
// ELASTIC_THREADPOOL_FUZZ should be undefined or empty.
#ifndef ELASTIC_THREADPOOL_FUZZ
#define ELASTIC_THREADPOOL_FUZZ
#endif

#ifndef LWG_ISSUE_3343_IS_FIXED
// See extensive comments below.  Some day this will depend on
// compiler and/or library versions.
#define LWG_ISSUE_3343_IS_FIXED 0
#endif
#if !LWG_ISSUE_3343_IS_FIXED
#include <memory>
#include <vector>
#endif

// elastic_threadpool: A pool of threads that execute T-valued
// functors of no arguments.
//
// Usage:
//   elastic_threadpool<someType> tp(50, 5);
//   ...
//   auto f = [...]()->someType{...};
//   auto fut = tp.submit(f);
//   // decltype(fut) is std::future<someType>
//  
//   // f() will eventually be called by one of the threads in the pool
//   // The value it returns (or anything it throws) will be made
//   // available to the caller (or anyone else) in fut.  E.g.,
//   someType st = fut.get();  // will either return what f returned or will 
//                             // rethrow whatever f threw.
//
// elastic_threadpool's constructor takes two arguments:
//   elastic_threadpool<T> nthreadmax(int nthreadmax, int nidlemax)
//
// The thread pool will adapt to load by creating and destroying
// threads.  Threads are either idle, i.e., waiting for work to be
// submit()-ed, or carrying out submit()-ed work.  There will be no
// more than nthreadmax threads in existence, and no more than
// nidlemax will ever be idle.  The submit() method is non-blocking,
// regardless of load.  It places a task on the work queue and
// immediately returns to its caller.
//
// Under "reasonable" load conditions, there will usually be close to
// nidlemax idle threads and the work queue will be empty.  Under high
// load, when work is being submit()-ed faster than nthreadmax threads
// can retire it, a 'backlog' of tasks will collect in the work queue.
// Each queue entry consumes about 16 bytes (plus malloc overhead), so
// there is little value in limiting the depth of the backlog queue.
//
// The shutdown() method drains the work queue and waits until all
// previously submit()-ed work has been retired.  It is an error if
// submit() is called after shutdown().  It's up to the caller to
// provide any necessary synchronization.
//
// elastic_threadpool's destructor calls shutdown().  Hence, it also
// waits for all submit()-ed work to be retired.
//
// Methods nidle(), nthread() and backlog() report the number of idle
// threads, the number of executing threads and the length of the
// backlog.  Callers should note that underlying value may change
// before the caller "looks at" the result.
//
// The nthread_hwm() method reports the largest number of threads over
// the life of the elastic_threadpool object.

namespace core123{

// N.B.  notify_all_at_thread_exit is broken.  See:
// https://stackoverflow.com/questions/59130819/tsan-complaints-with-notify-all-at-thread-exit
// https://cplusplus.github.io/LWG/issue3343

// The LWG has a "proposed resolution" from the 2020-02 meeting, but neither
// libstdc++ nor libc++ have implemented it as of Oct 2021.
//
// Until this is resolved, it's not possible to know when it's safe to
// destroy the cv used by elastic_threadpool.  So instead of making it
// a private member, we allocated it on the heap and never delete it.
// This leaks 48 bytes per elastic_threadpool.  (Not per thread or per
// submission.  Just per elastic_threadpool).  In actual practice, threadpools
// are pretty long-lived and there aren't that many of them, so this is
// a negligible cost.  The biggest headache is silencing warnings from
// tools like valgrind and LeakSanitizer.
//
// Encapsulate the leaky new in a function so we can "easily" suppress
// complaints.  Valgrind's leak complaint can be suppressed with
// something like:
//
// {
//    LWG_issue_3343_workaround
//    Memcheck:Leak
//    match-leak-kinds: definite
//    fun:LWGissue3343
// }

#if !LWG_ISSUE_3343_IS_FIXED
inline auto LWGissue3343(){
    auto ret = new std::condition_variable;
#ifndef __has_feature
#define __has_feature(x) 0
#endif
#if __has_feature(address_sanitizer) || defined(__SANITIZE_ADDRESS__)
    // Arrange to delete ret with static destructors, if and only if
    // we're running with asan.  This silences complaints from
    // LeakSanitizer, but actually uses more memory, more time, and
    // doesn't actually fix the issue.  I.e., the race condition
    // identified by LWG 3343 remains even if we defer the cv
    // destructors till static-destructor time.
    // 
    // In fact, tsan sometimes (not always) correctly detects the LWG
    // 3343 race condition, so definitely don't do this with tsan
    // enabled!
    static std::vector<std::unique_ptr<std::condition_variable>> v;
    v.emplace_back(ret);
#endif
    return ret;
}
#endif

template <typename T>
class elastic_threadpool{
    using workunit_t = std::packaged_task<T()>;

    bool enough_idle_threads(){
        // nidl counts the number of threads waiting on the workq. If
        // adding this thread to the workq would exceed nidlemax, AND
        // this isn't the last thread standing then decrement nth and
        // return true, i.e., there are enough_idle_threads.  Otherwise
        // return false.  Note that nth is decremented with the
        // nth_mtx held.
        if(nidl.load() < nidlemax)
            return false;
        auto old = nth.load();
        while(!nth.compare_exchange_weak(old, old-1 ? old-1 : 1));
        return old>1;
    }        

    // wait_for_work: called by worker_loop, only in detached threads
    std::optional<workunit_t> wait_for_work(){
        if(enough_idle_threads())
            return {};
        ELASTIC_THREADPOOL_FUZZ;
        // dequeue blocks until either there is work to do or the
        // workq is closed.  In the latter case, it returns an empty
        // optional<workunit_t>.
        //
        // Note that nidl.load() was instantaneously less than nidlemax
        // when we checked it in enough_idle_threads, but it might not
        // be now.  I.e., it's possible for nidl to be briefly larger
        // than nidlemax.  We *could* keep track of nidl_hwm, but it
        // would probably be more confusing than illuminating.
        nidl.fetch_add(1);
        auto ret = workq.dequeue();
        nidl.fetch_sub(1);
        return ret;
    }

    // worker_loop is the top-level function executed by the detached
    // threads in the pool.  Don't kid ourselves about being able to
    // handle exceptions here.  We really don't expect anything in
    // here to throw, and if something does, we're better off with a
    // core dump than a complaint.
    void worker_loop() noexcept {
        while(auto wu = wait_for_work()){
            (*wu)();
        }
        std::unique_lock lk(all_done_mtx);
        std::notify_all_at_thread_exit(cv, std::move(lk));
        ndet.fetch_sub(1);
        core123_soft_assert(ndet.load()>0 || workq.closed());
    }

    bool no_more_threads(){
        return nidl.load()> 0 || nth.load()>=nthreadmax;
    }

    // maybe_start_thread:  called by submit, i.e., by threads that
    // are submitting work to the pool, not by the threads in
    // the pool.
    void maybe_start_thread() try {
        if(no_more_threads())
            return;
        std::thread(&elastic_threadpool::worker_loop, this).detach();
        ndet.fetch_add(1);
        atomic_max(th_hwm, nth.fetch_add(1)+1);
    }catch(std::exception& e){
        // N.B.  Under very heavy load the thread constructor can fail
        // with std::errc::resource_unavailable_try_again.  That's only
        // a problem if a) there are no other threads running (i.e.,
        // nth==0) and b) no other tasks are submit()-ed (so we don't
        // try again).
        complain(e, "elastic_threadpool:  failed to start thread");
        if(nth.load() == 0)
            complain("elastic_threadpool:  submitted tasks will hang until the next call to submit()");
    }

    const int nthreadmax;
    const int nidlemax;
    std::atomic<int> nth{0};
    std::atomic<int> nidl{0};
    std::atomic<int> ndet{0};
    std::atomic<int> th_hwm{0};
    std::mutex all_done_mtx;
#if LWG_ISSUE_3343_IS_FIXED
    std::condition_variable cv;
#else
    std::condition_variable* cvp = LWGissue3343();
    std::condition_variable& cv = *cvp;
#endif
    producerconsumerqueue<workunit_t> workq;
    
public:
    elastic_threadpool(int _nthreadmax, int _nidlemax)
        : nthreadmax(_nthreadmax), nidlemax(_nidlemax)
    {
        if(nidlemax <= 0 ||  nthreadmax < nidlemax)
            throw std::invalid_argument(fmt("elastic_threadpool(nthreadmax=%d, nidlemax=%d):  must have nthreadmax>=nidlemax>0", nthreadmax, nidlemax));
    }
        
    void shutdown(){
        // workq.close() prevents subsequent workq.enqueue.  It allows
        // worker threads to drain the queue, after which
        // workq.dequeue returns false - causing worker threads
        // to exit, notifying the cv.
        workq.close(); 
        std::unique_lock lk(all_done_mtx);
        cv.wait(lk, [this]{ return ndet.load()==0; });
    }
    
    ~elastic_threadpool(){
        try{ shutdown(); }
        catch(std::exception& e){  complain(e, "elastic_threadpool::~elastic_threadpool:  ignore exception thrown by shutdown:"); }
        catch(...){ complain("elastic_threadpool::~elastic_threadpool:  ignore ... thrown by shutdown"); }
    }
    
    template <typename CallBackFunction>
    std::future<T> submit(CallBackFunction&&  f){
        workunit_t wu(std::move(f));
        auto fut = wu.get_future();
        if( !workq.enqueue(std::move(wu)) )
            throw std::logic_error("could not enqueue workunit into threadpool queue.  Threadpool has probably been shutdown");
        maybe_start_thread();
        ELASTIC_THREADPOOL_FUZZ;
        core123_soft_assert(ndet.load()>0);
        return fut;
    }

    auto backlog() const{
        return workq.size();
    }

    auto nidle() const{
        return nidl.load();
    }

    auto nthreads() const{
        return nth.load();
    }

    auto nthread_hwm() const{
        return th_hwm.load();
    }
};

        
} // namespace core123
