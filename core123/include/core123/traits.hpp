#pragma once

// Some simple type-traits that are better to code up once and
// for all than to repeat over and over...

// is_std_optional<T> - true if T is derived from std::optional

namespace core123{

template<typename>
struct is_std_optional : std::false_type {};

template<typename T>
struct is_std_optional<std::optional<T>> : std::true_type{};

} 
