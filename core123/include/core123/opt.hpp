// Small option parsing utility class.  Only intended for name=value
// format, which can directly come from argv via --name=value, or
// environment variables {SOMEPREFIX}{NAME}=value, or from the caller.
//
// IMPORTANT: the API was was changed in May 2021.  See
// INCOMPATIBILITIES below.

// Quick start:
//
//    using namespace core123;
//    using namepsce std;
//    option_parser p;
//    int nwidgets;
//    bool verbose;
//    optional<int> maybe_nthreads;
//    int nthreads;
//    p.add_option("nwidgets" "10", "How many widgets", opt_setter(nwidgets));
//    p.add_option("verbose", "Chattiness", opt_true_setter(verbose));
//    p.add_option("nthreads", {}, opt_setter(maybe_nthreads)); // no default!
//    p.setopts_from_env("MYPROG_");
//    p.setopts_from_argv(argc, argv);
//    // if nthreads was set by the caller, then use the caller-specified value.
//    // Otherwise,  make it 2*nwidgets:
//    nthreads = maybe_nthreads ? *maybe_nthreads : 2*nwidgets;
//
//    Options can be  specified in several ways:
//
//    # On the command line:
//    $ myprog --verbose --nwidgets=50 ...
//    # In the environment:
//    $ env MYPROG_nwidgets=99 myprog --verbose
//    # In a config file:
//    $ echo nwidgets=22 > optfile
//    $ myprog --flagfile optfile
//    
//
// Details:
// 
// An option_parser is declared with:
//
//   option_parser p;
//   ...
//
// Individual options are declared with:
//
//   p.add_option("name", "default_value", "description", callback);
//
// Note that the second argument (the default value) is an optional<string>.
// So one can specify an option with no default as:
//
//   p.add_option("nodefault, {}, "this option has no default", callback);
//
// Which is different from an option with an empty string as a default:
//
//   p.add_option("emptydefault", "", "this option's default is an empty string", callback);
//
// Options are parsed by one of:
// - from the command line:
//     p.setopts_from_argv(int argc, const char** argv, size_t startindex=1, bool no_defaults=false);
// - from a range:
//     p.setopts_from_range(ITER b, ITER e, bool no_defaults=false);
//     N.B.  the iterator's value-type must be *convertible* to std::string.
// - from the environment:
//     p.setopts_from_env("MYPROG_", bool no_defaults=false);
// - from a stream:
//     p.setopts_from_istream(ifstream("foo.opts"), bool no_defaults=false);
// - explicitly:
//     p.set("name", "value");
//     p.set("verbose");  // a no-value option
// - from their default values:
//     p.setopts_from_defaults()
//
// Note that it's up to the caller whether, and in what order to call
// the different setopts members, giving precedence to one or another
// source (config files, command line, environment).  Also note that
// the setopts_from_ member functions take an optional no_defaults
// boolean argument which determines whether, immediately before
// returning, they invoke callback(default_value, opt) for options
// that are not yet explicitly specified.
//
// The callback argument to add_option has a signature like:
//
//    void callable(const std::optional<std::string> newvalue, const option& before);
//
// The callable can do whatever it likes with the new value and the
// previous state of the option.  E.g., it can convert the new value
// to a type and/or write it to an external location.  In fact, the
// 'opt_setter' function returns a callable that does exactly that.
// E.g.,
//
//    int nthreads;
//    p.add_option("nthreads", "4", "number of threads", opt_setter(nthreads));
//
// When the '--nthreads=4' option is parsed, the functor returned by
// opt_setter(nthreads) will assign 4 to its argument, nthreads.  Note
// that the opt_setter stores a reference to its argument, so its
// argument should generally be a long-lived object.  Also note that
// opt_setter "works" if its argument is a reference to a std::optional
// value.  E.g.,
//
//   std::optional<int> stdopt_nthreads;
//   p.add_option("nthreads", {}, "number of threads", opt_setter(stdopt_nthreads));
//   ...
//   p.setopts_from_argv(argc, argv);
//   ...
//   if(stdopt_nthreads){
//     // there was definitely a --nthreads on the command line
//     nthreads = *stdopt_nthreads;
//   }else{
//     // --nthreads wasn't specified.  Choose a value based on
//     // other things we may know, possibly even values set
//     // by other command-line options...
//     nthreads = ...;
//   }
//  
//
// The 'opt_true_setter' and 'opt_false_setter' callbacks are
// convenient for value-less options that set a boolean:
//
//   bool verbose = false;
//   p.add_option("verbose", {}, "enable chattiness", opt_true_setter(verbose));
//
// When the '--verbose' option is parsed, the functor returned by
// opt_true_setter(verbose) will assign true to its argument, verbose.
//
// When writing a custom callback, note that the first argument to the
// callback is an std::optional<string>.  If the option was provided
// without an argument (e.g., a command line --option without an '=')
// the callback's std::optional<string> argument does not contain a value.
// Conversely, if the option was provided with a argument, then the
// std::optional<string> contains a copy of of the argument.  It's up to
// the callback to decide whether it requires an argument.  When
// invoked without an argument, a callback that requires an argument
// should:
//    throw option_missing_argument_error("...");
// Conversely, when invoked with an argument, a callback that doesn't
// expect an argument should:
//    throw option_unexpected_argument_error("...");
//
// There is one automatically declared option:
//
//   --flagfile=FILENAME - invokes a callback that calls:
//           setopts_from_istream(ifstream(FILENAME), false)
//
// Any option (including --flagfile) can be removed with del_option, after
// which its name will no longer be recognized by the setopts functions.
//
// A rudimentary, but passable usage string is produced by:
//   std::cerr << p.helptext();
//
//
// In all cases, option names are case-insensitive and hyphens and underscores
// in option names are ignored.  So,
//    --max-open-files=1024
//    --max_open_files=1024
//    --maxopenfiles=1024
//    --MaxOpenFiles=1024
//    --max_OpENFil-es=1024
//    env MYPROG_MAX_OPEN_FILES=1024 ...
//    env MYPROG_max_open_files=1024 ...
// are all equivalent:  they will all invoke the callback associated with the "max-openfiles" option.
//
// When reading options from a stream, the stream is read one line at
// a time.  Whitespace is trimmed from the beginning and end of each
// line, and if the result is either empty or starts with a '#', the
// line is ignored.  Otherwise, the line must match the regex:
//
//    std::regex re("(--)?([-_[:alnum:]]+)\\s*(=?)\\s*(\"?)(.*?)(\"?)\\s*");
//
// I.e., the line must start with an optional leading "--", followed
// by the option name (one or more hyphens, underscores or
// alphanumerics), followed by an optional "=" (surrounded by optional
// whitespace), followed by an optional double-quote, the value,
// another optional double-quote and optional whitespace.  If *both*
// optional double-quotes are present, the value is the text between
// the double-quotes.  Otherwise, the value is the text between the
// optional blocks of whitespace.  Thus, if an istream contains:
//   --foo = " abc d "
// foo's value will be a 7-letter string that starts ends with <space>,
// not an 9-letter string that starts and ends with double-quotes.  If
// an istream contains:
//   --bar =   abc d
// bar's value will be a 5-letter string that starts with 'a' and ends with
// 'd'.  And if an istream contains:
//   --baz = "abc d
// baz's value will be a six-letter string that starts with a double-quote
// and ends with 'd'.  If an istream contains:
//   --bletch=""xyz abc""
// bletch's value is a 7-letter string that starts and ends with double-quote.
// 
// If there is no "=", *and* if there is only whitespace after the
// name, the option's callback is called with a default-constructed
// std::optional value with no contained value.
//
//     --foo             // callback({})
//     --foo=bar         // callback("bar")
//     --foo =  bar      // callback("bar")
//     --foo== bar       // callback("= bar")
//     --foo bar         // callback("bar")
//     --foo     bar     // callback("bar")
//     --foo bar=baz     // callback("bar=baz")
//     --foo=bar=baz     // callback("bar=baz")
//     #--foo            // comment
//     foo               // callback({})
//     foo=bar           // callback("bar")
//     foo    bar        // callback("bar")
//     #foo              // comment
//
// The option_parser methods generally throw option_error exceptions
// (possibly with other exceptions nested inside) if they encounter
// any unexpected conditions.
//
// Advanced usage:  (subject to change!):
//
// The option class provides access to option details:
// The p.add_option method returns an 'option' class.
//   option opt = p.add_option(...);
// The option class has the following methods:
//   opt.get_name()  - returns the name by which this option may be set
//   opt.get_value() - returns an option<string> with the specified value of the option (if any)
//   opt.get_default() - returns an option<string> with the default value of the option (if any)
//   opt.get_desc()  - returns the option's description
//   opt.set(optional<string>) - sets the value and calls the callback
//
// The option_parser class exposes the details of all options via
// the get_map method:
//
//   class option_parser{
//     public:
//       typedef std::map<std::string, option> OptMap;
//       const option_parser::OptMap& p.get_map() const;
//       ...
//   };
//
// INCOMPATIBILITIES
//
// Code that does:
//    parser.add_option(name, {}, "descrip", callback);
// now behaves differently from code that does:
//    parser.add_option(name, "", "descrip", callback);
//
// Furthermore, parser.add_option does *not* invoke:
//    callback(default_value, opt);
// Callbacks are invoked only by the setopts_xxx member functions.


#pragma once
#include <core123/svto.hpp>
#include <core123/throwutils.hpp>
#include <string>
#include <map>
#include <cstring>
#include <iostream>
#include <fstream>
#include <regex>
#include <stdexcept>
#include <optional>

namespace core123{

struct option_error : public std::runtime_error{
    explicit option_error(const std::string& what_arg) : std::runtime_error(what_arg){}
    explicit option_error(const char* what_arg) : std::runtime_error(what_arg){}
    ~option_error() = default;
};

struct option_missing_argument_error : public option_error{
    explicit option_missing_argument_error(const std::string& what_arg) :
        option_error(what_arg)
    {}
};

struct option_unexpected_argument_error : public option_error{
    explicit option_unexpected_argument_error(const std::string& what_arg) :
        option_error(what_arg)
    {}
};

struct option{
    using opt_string = std::optional<std::string>;
    using callback_t = std::function<void(opt_string, const option&)>;
    std::string name;    // identical to the key in optmap.
    std::string desc;    // description for help text
    opt_string valstr;  // current value, initialized to dflt
    opt_string dflt;    // default value
    callback_t callback;
    friend class option_parser;
    // N.B.  the only way to construct an option is through option_parser::add_option
    option(const std::string& name_, opt_string dflt_, const std::string& desc_,
           callback_t cb_):
        name(name_), desc(desc_), dflt(dflt_), callback(cb_)
    { }
public:
    std::string get_name() const { return name; }
    std::optional<std::string> get_value() const {
        return valstr;
    }
    std::optional<std::string> get_default() const {
        return dflt;
    }
    void set_default(opt_string newdflt){
        dflt = newdflt;
    }
    void apply_default(){
        if(!valstr && dflt)
            set(dflt);
    }
    std::string get_desc() const { return desc; }
    void set(opt_string newval){
        callback(newval, *this);
        valstr = newval;
    }
};

class option_parser {
public:
    using opt_string = std::optional<std::string>;
    using callback_t = std::function<void(opt_string, const option&)>;
    typedef std::map<std::string, option> OptMap;
private:
    // Note that add_option returns a reference to the newly added
    // option *in* the optmap_.  Therefore, nothing may ever be
    // removed from optmap_!
    OptMap optmap_;
    // When parsing command-line options, we ignore hyphens, underscores
    // and case.  Canonicalize is called before keys are inserted
    // or looked up in optmap_.
    std::string canonicalize(const std::string& word){
        std::string ret;
        for(auto letter : word){
            if(letter == '-' || letter == '_')
                continue;
            ret.append(1, std::tolower(letter));
        }
        return ret;
    }

    option& at(const std::string& k) try {
        return optmap_.at(canonicalize(k));
    }catch(std::out_of_range&){
        throw option_error("option_parser:  unknown option: " + k);
    }

    auto find(const std::string& k){
        return optmap_.find(canonicalize(k));
    }

public:
    option_parser(){
        static int flagfile_depth = 0;
        add_option("flagfile", {}, "read flags from the named file",
                   [this](opt_string fname, const option&){
                       if(!fname)
                           throw option_missing_argument_error("--flagfile with no value");
                       if(flagfile_depth++ > 10)
                           throw option_error("flagfile recursion depth exceeds limit (10) processing:" + *fname);
                       std::ifstream ifs(*fname);
                       setopts_from_istream(ifs, false);
                       if(!ifs && !ifs.eof())
                           throw option_error("error reading from --flagfile=" + *fname);
                       flagfile_depth--;
                   });
    }
    // creates and returns a new option.
    option& add_option(const std::string& name, opt_string dflt, const std::string& desc, callback_t cb) try {
        auto ibpair = optmap_.emplace(std::piecewise_construct, std::forward_as_tuple(canonicalize(name)), std::forward_as_tuple(name, dflt, desc, cb));
        if(!ibpair.second)
            throw option_error("opt_parser::add_option(" + name + ") already exists.");
        return ibpair.first->second;
    }
    catch(option_error&){throw;}
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, name, dflt?*dflt:"<none>", desc, &cb)));}

    // The three-argument form is deprecated.  But to support existing
    // code, it just forwards to the "no-default" 4-argument form.
    option& add_option(const std::string& name, const std::string& desc, callback_t cb){
        return add_option(name, {}, desc, cb);
    }

    void clear(){
        optmap_.clear();
    }
    
    void del_option(const std::string& name)try{
        optmap_.erase(name);
    }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, name)));}

    // set one option, by name, to val, and call the option's callback.
    void set(const std::string &name, opt_string val)try{
        // .at throws if name is not a known option.
        at(name).set(val);
    }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, name, val)));}
       
    // How much visibility should we offer into internals?  With a
    // const reference to the optmap, a determined caller can
    // enumerate the current option settings or generate its
    // own helptxt.  Sufficient?
    const OptMap& get_map() const { return optmap_; }

    // parses any --foo=bar from argv[startindex] onwards.
    // Stops at -- (gobbling it).  An option_error is thrown if foo
    // was not previously add_option()-ed.  N.B. - only *weakly*
    // exception-safe.  *This will have been modified in unpredictable
    // ways if the operation throws.
    std::vector<std::string>
    setopts_from_argv(int argc, char *argv[], int startindex = 1, bool no_defaults=false)try{
        return setopts_from_range(argv+startindex, argv+argc, no_defaults);
    }
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, argc, argv, startindex)));}
    
    template <typename ITER>
    std::vector<std::string>
    setopts_from_range(ITER b, ITER e, bool no_defaults=false)try{
        std::vector<std::string> leftover;
        for (auto i=b ; i!=e; ++i){
            std::string cp = std::string(*i);
            try{
                if(!startswith(cp, "--")){
                    leftover.push_back(cp);
                    continue;
                }
                if(cp == "--"){
                    // Stop when we see --.  Anything left gets pushed
                    // onto the leftover vector.
                    for( ++i ; i!=e; ++i)
                        leftover.push_back(std::string(*i));
                    break;
                }
                auto eqpos = cp.find("=", 2);
                if(eqpos == std::string::npos){
                    // No '='.
                    auto optiter = find(cp);
                    if(optiter == optmap_.end()){
                        leftover.push_back(cp);
                        continue;
                    }
                    auto& opt = optiter->second;
                    try{
                        // First, try calling set with no arguments.
                        opt.set({});
                    }catch(option_missing_argument_error& omae){
                        // If set doesn't like no-arguments, try again
                        // with *i++ as argument.
                        if(++i == e)
                            throw; // we're already at the end
                        opt.set(std::string(*i));
                    }
                }else{
                    // --name=something
                    auto optiter = find(cp.substr(2, eqpos-2));
                    if(optiter == optmap_.end()){
                        leftover.push_back(cp);
                        continue;
                    }
                    auto& opt = optiter->second;
                    opt.set(cp.substr(eqpos+1));
                }
            }
            catch(option_error&){ throw; }
            catch(std::exception& ){
                std::throw_with_nested(option_error(std::string(__func__) + ": error while processing " + cp));
            }
        }
        if(!no_defaults)
            setopts_from_defaults();
        return leftover;
    }        
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, &*b, &*e)));}

    // sees if any environment variable names of the form opt_env_prefix
    // concatenated before the uppercase option name exist, and if so, set
    // that option to the corresp value (for all names provided in opts to init)
    // N.B. - only *weakly* exception-safe.  *This will have been modified in
    // unpredictable ways if the operation throws.
    void setopts_from_env(const char *opt_env_prefix, bool no_defaults=false)try{
        std::string pfx(opt_env_prefix);
        for (const auto& o : optmap_) {
            std::string ename(opt_env_prefix);
            for (const char *s = o.first.c_str(); *s; s++) {
                unsigned char c = *s;
                ename += toupper(c);
            }
            auto ecp = getenv(ename.c_str());
            if (ecp) set(o.first, ecp);
        }
        if(!no_defaults)
            setopts_from_defaults();
    }
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, opt_env_prefix)));}

    // read options from the specified istream
    // N.B. - only *weakly* exception-safe.  *This will have been modified in
    // unpredictable ways if the operation throws.
    void setopts_from_istream(std::istream& inpf, bool no_defaults=false)try{
        // N.B. - reading to the end of inpf this way may (usually!)
        // set inpf's failbit.
        for (std::string line; getline(inpf, line);) {
            std::string s = strip(line); // remove leading and trailing whitespace
            if(startswith(s, "#") || s.empty())
                continue;
            if(startswith(s, "--"))
                s = s.substr(2);
            static std::regex re("(--)?([-_[:alnum:]]+)\\s*(=?)\\s*(\"?)(.*?)(\"?)\\s*");
            std::smatch mr;
            if(!std::regex_match(s, mr, re))
                throw option_error("setopts_from_istream: failed to parse line: " +line);
            if(mr.size() != 7)
                throw std::logic_error("Uh oh. We're very confused about regex_match");
            auto name = mr.str(2);
            auto equals = mr.str(3);
            std::string rhs;
            if(!mr.str(4).empty() && !mr.str(6).empty())
                rhs = mr.str(5); // discard enclosing quotes 
            else
                rhs = mr.str(4) + mr.str(5) + mr.str(6);
            if(!equals.empty() || !rhs.empty())
                set(name, rhs);
            else
                set(name, {});
        }
        if(!no_defaults)
            setopts_from_defaults();
    }        
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, &inpf)));}
    
    // setopts_from_defaults: Any remaining unset options that have a
    // default are assigned from their defaults.
    void setopts_from_defaults() try {
        for(auto& [k, opt] : optmap_){
            opt.apply_default();
        }
    }
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::setopts_from_defaults"));}

    // returns help text derived from names, defaults and descriptions.
    std::string helptext(size_t indent = 4) const try{
        std::string ret;
        for (const auto& o : optmap_) {
            const option& opt = o.second;
            ret.append(indent, ' ');
            ret.append(opt.name); // not o.first,  which is canonicalized
            if(opt.dflt){
                ret.append(" (default=");
                ret.append(*opt.dflt);
            }else{
                ret.append(" (no default");
            }
            ret.append(")");
            ret.append(" : ");
            ret.append(opt.desc);
            ret.append(1, '\n');
        }
        return ret;
    }
    catch(option_error&){ throw; }
    catch(std::exception&){std::throw_with_nested(option_error("option_error::" + strfunargs(__func__, indent)));}
};

// helper classes:
//   opt_setter
//   true_setter
//   false_setter
namespace detail{
template<typename>
struct is_std_optional : std::false_type {};

template<typename T>
struct is_std_optional<std::optional<T>> : std::true_type{};

template<typename T>
class _setter{
    T& v;
public:
    void operator()(std::optional<std::string> newv, const option& opt){
        if(!newv)
            throw option_missing_argument_error("argument required for option: --" + opt.name);
        if constexpr (is_std_optional<T>::value)
            v = svto<typename T::value_type>(*newv);
        else
            v = svto<T>(*newv);
    }
    _setter(T& v_) : v(v_){}
};

template<>
inline void _setter<std::string>::operator()(std::optional<std::string> newv, const option& opt){
    if(!newv)
        throw option_missing_argument_error("argument required for option: --" + opt.name);
    v = *newv;
}
template<>
inline void _setter<std::optional<std::string>>::operator()(std::optional<std::string> newv, const option& opt){
    if(!newv)
        throw option_missing_argument_error("argument required for option: --" + opt.name);
    v = *newv;
}
} // namespace detail

template<typename T>
detail::_setter<T>
opt_setter(T& v){ return detail::_setter<T>(v); }

namespace detail{
template<typename T, bool B>
class _bool_setter{
    static_assert( std::is_same_v<bool, T> || std::is_same_v<std::optional<bool>, T> );
    T& v;
public:
    void operator()(std::optional<std::string> s, const option& opt){
        if(s)
            throw option_unexpected_argument_error("unexpected argument for option: --" + opt.name);
        v = B;
    }
    _bool_setter(T& v_) : v(v_){}
};
} // namespace detail
template<typename T>
detail::_bool_setter<T, true>
opt_true_setter(T& v){ return detail::_bool_setter<T, true>(v); }

template<typename T>
detail::_bool_setter<T, false>
opt_false_setter(T& v){ return detail::_bool_setter<T,  false>(v); }


} // namespace core123
