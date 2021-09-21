#define CORE123_DIAG_FLOOD_ENABLE 1

// set up 'fuzzing' before we #include "core123/elastic_threadpool.hpp"
#define ELASTIC_THREADPOOL_FUZZ  ::ut_etp_fuzz()
#include "core123/threefry.hpp"
#include <atomic>
#include <thread>
#include <chrono>
#include <cmath>

double expvariate(uint32_t i){
    return -::log(core123::threefry<2, uint32_t>()({0u, i})[0]/0x1.p32);
}

void ut_etp_fuzz(){
    static std::atomic<uint32_t> f;
    static const int mean = 2000;   // 2 musec sleep
    int nanos = expvariate(f++) * mean;
    if(nanos > mean){
        std::this_thread::sleep_for(std::chrono::nanoseconds(nanos));
    }else if(nanos > mean/2){
        std::this_thread::yield();
    }
}

#include "core123/elastic_threadpool.hpp"
#include "core123/scoped_timer.hpp"
#include "core123/datetimeutils.hpp"
#include "core123/sew.hpp"
#include "core123/ut.hpp"
#include "core123/envto.hpp"
#include <condition_variable>
#include <unistd.h>
#include <vector>
#include <iostream>
#include <sstream>
#include <string>
#include <cassert>

using core123::elastic_threadpool;
namespace sew = core123::sew;
using core123::str;

std::atomic<int> ai;
class Foo {
private:
    int divisor_;
public:
    Foo(int divisor) : divisor_{divisor} {}
    int operator()() {
	std::this_thread::sleep_for( std::chrono::milliseconds(10) );
	auto k = ai++;
	if(k%divisor_==0)
	    throw std::runtime_error(str("Foo: won't return", 10*k, "because", k, "is divisible by", divisor_));
	return 10*k;
    }
};

void timecheck(){
    time_t now;
    ::time(&now);
    char buf[26];
    std::cout << sew::ctime_r(&now, buf) << std::flush;
}

void stress_test(int tmax, int imax){
    std::vector<std::future<int>> results;
    core123::timer<> t;
    elastic_threadpool<int> tp(tmax, imax);
    auto elapsed = t.elapsed();
    timecheck();
    std::cout << "construction of elastic_threadpool(" << tmax << ", " <<  imax << "):  " << str(elapsed) << std::endl;
    
    static const int N=10000;
    t.restart();
    bool ready = false;
    std::condition_variable cv;
    std::mutex m;
    std::vector<int> writeback(N);
    for(int i=0; i<N; ++i){
        results.push_back(tp.submit([i, &m, &cv, &ready, &writeback](){
                                         std::unique_lock<std::mutex> lk(m);
                                         cv.wait(lk, [&ready]{return ready;});
                                         writeback[i] = i;
                                         return i;}));
    }
    elapsed = t.elapsed();
    std::cout << "after submitting " << N << " requests:  " << str(elapsed/N) << " seconds per submission" << std::endl;
    std::cout << "backlog: " << tp.backlog() << std::endl;

    t.restart();
    {
        std::unique_lock<std::mutex> lk(m);
        ready = true;
    }
    cv.notify_all();
    for(int i=0; i<N; ++i){
        auto ri = results[i].get();
        assert(ri == i);
        assert(ri == writeback[i]);
        // Note:  we intentionally DO NOT catch future_error.  If we get one,
        // study the core dump.
    }
    elapsed = t.elapsed();
    std::cout << "after get-ing " << N << " futures:  " << str(elapsed/N) << " seconds per get\n";
    std::cout << core123::fmt("threadmax/threadhwm: %d %d\n",
                              tmax, tp.nthread_hwm());
    std::cout << std::endl;
    assert(tmax >= tp.nthread_hwm());
    // it's possible (but unlikely) that the number of idle threads
    // exceeds imax.  If so, idle threads should drop away as they
    // are exercised.
    int waitsec = 0;
    while(tp.nidle() > imax){
        assert(waitsec++<10);
        for(int j=0; j<tp.nidle() - imax; ++j)
            tp.submit([](){return 0;});
        std::cout << "Wait for idle threads to run down" << std::endl;
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
    CHECK(true); // if we got here, we passed LOTS of assertions
}

// Slightly different... There's no barrier at the start,
// and there are random delays introduced between submissions
// and within each submitted task.
void stress_test2(int tmax, int imax)  {
    std::vector<std::future<int>> results;
    elastic_threadpool<int> tp(tmax, imax);
    timecheck();
    std::cout << "construction of elastic_threadpool(" << tmax << ", " <<  imax << ")" << std::endl;
    static const int N=10000; // 10k
    std::vector<int> writeback(N);
    for(int i=0; i<N; ++i){
        // sleep randomly before each insertion:
        unsigned int nsec = expvariate(N+i)*50000.; // 50musec * 10k = 500 msec
        struct timespec ts = {0, nsec};
        nanosleep(&ts, nullptr);
        results.push_back(tp.submit([i, &writeback](){
                                         // sleep randomly in each thread:
                                         unsigned int _nsec = expvariate(i)*40000.; // 40musec * 10k = 400msec
                                         struct timespec _ts = {0, _nsec};
                                         nanosleep(&_ts, nullptr);
                                         writeback[i] = i;
                                         return i;}));
    }
    std::cout << "after submitting " << N << " requests with random inter-request delays\n";
    std::cout << "backlog: " << tp.backlog() << std::endl;

    for(int i=0; i<N; ++i){
        auto ri = results[i].get();
        assert(ri == i);
        assert(ri == writeback[i]);
        // Note:  we intentionally DO NOT catch future_error.  If we get one,
        // study the core dump.
    }
    std::cout << "returned " << N << " results (each of which slept for a short time)\n";
    std::cout << core123::fmt("threadmax/threadhwm: %d %d\n",
                              tmax, tp.nthread_hwm());
    std::cout << std::endl;
    assert(tmax >= tp.nthread_hwm());
    // it's possible (but unlikely) that the number of idle threads
    // exceeds imax.  If so, idle threads should drop away as they
    // are exercised.
    int waitsec = 0;
    while(tp.nidle() > imax){
        assert(waitsec++<10);
        for(int j=0; j<tp.nidle() - imax; ++j)
            tp.submit([](){return 0;});
        std::cout << "Wait for idle threads to run down" << std::endl;
        std::this_thread::sleep_for(std::chrono::seconds(1));
    }
    CHECK(true); // if we got here, we passed LOTS of assertions
}

// Can we submit to a threadpool from a function running in one of the
// threadpool's threads?  This is a dumb way to compute fib(n), but
// it's a good way to spawn lots of threads-within-threads.
int fib_inner(int n, elastic_threadpool<int>& etp, std::atomic<size_t>& tasks){
    if(n==0 || n==1)
        return 1;
    auto fn1 = etp.submit([n, &etp, &tasks]() mutable { return fib_inner(n-1, etp, tasks); });
    auto fn2 = etp.submit([n, &etp, &tasks]() mutable { return fib_inner(n-2, etp, tasks); });
    tasks.fetch_add(2);
    return fn1.get() + fn2.get();
}

void fib(int n){
    // NOTE - will deadlock if n is too large.  This isn't a problem
    // with the threadpool.  This is just a particularly inefficient
    // way to compute fib(n), and for n>16 or 17, we would need more
    // than O(1000) threads.
    elastic_threadpool<int> tp(1000,5);
    std::atomic<size_t> tasks{1};
    auto fut = tp.submit([n, &tp, &tasks]() mutable { return fib_inner(n, tp, tasks); });
    auto ret = fut.get();
    std::cout << "fib(" << n << ") -> " << ret << ".  Submitted " << tasks.load() << " tasks.  hwm: " << tp.nthread_hwm() << std::endl;
}

int main(int, char**){
    core123::set_soft_assert_terminates(true);
    std::vector<std::future<int>> results;
    int tmax=10;
    int imax=1;
    elastic_threadpool<int> tp(tmax, imax);

    // Just for informational purposes - how big is each of the entries
    // in the elastic_threadpool's pcq?
    timecheck();
    std::cout << "sizeof(elastic_threadpool<int>'s packaged_task) " << sizeof(std::packaged_task<int()>) << std::endl;
    std::cout << "sizeof(elastic_threadpool<double>'s packaged_task) " << sizeof(std::packaged_task<double()>) << std::endl;
    std::cout << "sizeof(elastic_threadpool<array<int, 64>>'s packaged_task) " << sizeof(std::packaged_task<std::array<int, 64>()>) << std::endl;

    // what happens if a threadpool gets deleted with only a few
    // (sometimes zero) submissions.
    timecheck();
    std::cout << "Create 10k threadpools and make a few submissions to each" << std::endl;
    for(int i=0; i<10000; ++i){
        static std::atomic<uint32_t> ctr{0};
        elastic_threadpool<int> et(3,1);
        int howmany = expvariate(ctr.fetch_add(1))*5;
        for(int j=0; j<howmany; ++j){
            et.submit( [=](){ return j; });
        }
    }

    // Can tasks submit other tasks?
    timecheck();
    std::cout << "Compute fib(8) very inefficiently with lots of threads" << std::endl;
    fib(8);

    static const int NLOOP=200;
    auto divisor = core123::envto("UT_THREADPOOL_DIVISOR", 5);
    for(int i=0; i<NLOOP; ++i){
        // Note that we're pushing 2*NLOOP tasks.  NLOOP of them return their lambda parameter.
        // And NLOOP of them return 10*std_atomic_counter++ (unless the counter is divisible
        // by 5, in which case they throw!
        results.push_back( tp.submit( [=](){ return i; }) );
	auto f = Foo(divisor);
        results.push_back(tp.submit(f));
    }
    int slept = 0;
    size_t bl;
    while((bl = tp.backlog())){
        std::cout << getpid() << ": backlog=" << bl << " after "<< slept << "sec" << std::endl;
        ::sleep(1);
        if(slept++ >= 10){
            std::cerr << "Threadpool not clearing backlog.  Abort!" << std::endl;
            abort();
        }
    }
    int nr = 0;
    std::vector<int> seen(NLOOP);
    for( auto& r : results ){
        try{
            auto rg = r.get();
            if(nr%2 == 0){
                assert(rg == nr/2);
            }else{
                // it must be divisible by 10.
                assert(rg%10 == 0);
                seen.at(rg/10) = 1;
            }
            //std::cout << nr << " " << rg << std::endl;
        }catch(std::runtime_error& e){
            //std::cout << nr << " exception delivered by r.get: " << e.what() << std::endl;
        }
        // Note:  we intentionally DO NOT catch future_error.  If we get one,
        // study the core dump.
        nr++;
    }
    for( size_t i=0; i<NLOOP; ++i){
        assert(i%divisor==0 || seen[i]);
    }

    timecheck();
    // Try that again, but this time, discard the futures returned by
    // submit.  The standard says that ~future is non-blocking, unless
    // it came from std::async.  And our futures don't come from
    // std::async, so we should be ok...
    for(int i=0; i<NLOOP; ++i){
        tp.submit( [=](){ return i; });
	auto f = Foo(divisor);
        tp.submit(f);
    }
    slept=0;
    while(tp.backlog()){
        ::sleep(1);
        if(slept++ > 10){
            std::cout << "Threadpool not clearing backlog.  Abort!" << std::endl;
            abort();
        }
    }
    tp.shutdown();
    assert(tmax >= tp.nthread_hwm());
    
    // elastic_threadpool has some tricky logic related to idling
    // threads.  If the last thread returns before the last submission
    // there may be nothing to service the last submission.  That
    // shouldn't happen, but let's confirm it by aggressively shutting
    // down some threadpools.
    for(int i=0; i<1000; ++i){
        elastic_threadpool<int> etp(5,1);
        for(int j=0; j<10; ++j){
            etp.submit( [=](){ return j; });
        }
        etp.shutdown();
    }
    std::cout << "Created and destroyed 1000 elastic_threadpools with 10 submissions each" << std::endl;
            
    stress_test(1, 1);
    stress_test(30, 6);
    stress_test2(1, 1);
    stress_test2(10, 6);
    stress_test2(10, 1);
    
    return utstatus(1);
}
