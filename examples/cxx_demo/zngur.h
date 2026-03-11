

#pragma once

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <csignal>
#include <array>
#include <iostream>
#include <functional>
#include <math.h>
#include <type_traits>


  namespace rust {
      class Panic {};
  }
  inline thread_local bool __zngur_rust_panicked_flag(false);
#if defined(__GNUC__) || defined(__clang__)
#define ZNGUR_USED __attribute__((used))
#else
#define ZNGUR_USED
#endif
  extern "C" inline ZNGUR_USED void __zngur_mark_panicked() {
    // TODO: While thread safe, you may end up with the wrong thread being
    // notified of a panic
    __zngur_rust_panicked_flag = true;
  }
  inline bool __zngur_read_and_reset_rust_panic() {
    bool out = __zngur_rust_panicked_flag;
    __zngur_rust_panicked_flag = false;
    return out;
  }


#define zngur_dbg(x) (::rust::zngur_dbg_impl(__FILE__, __LINE__, #x, x))

namespace rust {

  template<typename T>
  struct __zngur_internal {
    static inline uint8_t* data_ptr(const T& t) noexcept;
    static void assume_init(T& t) noexcept ;
    static void assume_deinit(T& t) noexcept ;
    static inline void check_init(const T&) noexcept;
    static inline size_t size_of() noexcept ;
  };

  template<typename T>
  inline uint8_t* __zngur_internal_data_ptr(const T& t) noexcept {
    return __zngur_internal<T>::data_ptr(t);
  }

  template<typename T>
  void __zngur_internal_assume_init(T& t) noexcept {
    __zngur_internal<T>::assume_init(t);
  }

  template<typename T>
  void __zngur_internal_assume_deinit(T& t) noexcept {
    __zngur_internal<T>::assume_deinit(t);
  }

  template<typename T>
  inline size_t __zngur_internal_size_of() noexcept {
    return __zngur_internal<T>::size_of();
  }

  template<typename T>
  inline void __zngur_internal_move_to_rust(uint8_t* dst, T& t) noexcept {
    memcpy(dst, ::rust::__zngur_internal_data_ptr(t), ::rust::__zngur_internal_size_of<T>());
    ::rust::__zngur_internal_assume_deinit(t);
  }

  template<typename T>
  inline T __zngur_internal_move_from_rust(uint8_t* src) noexcept {
    T t;
    ::rust::__zngur_internal_assume_init(t);
    memcpy(::rust::__zngur_internal_data_ptr(t), src, ::rust::__zngur_internal_size_of<T>());
    return t;
  }

  template<typename T>
  inline void __zngur_internal_check_init(const T& t) noexcept {
    __zngur_internal<T>::check_init(t);
  }

  class ZngurCppOpaqueOwnedObject {
    uint8_t* __zngur_data;
    void (*destructor)(uint8_t*);

  public:
    template<typename T, typename... Args>
    inline static ZngurCppOpaqueOwnedObject build(Args&&... args) {
        ZngurCppOpaqueOwnedObject o;
        o.__zngur_data = reinterpret_cast<uint8_t*>(new T(::std::forward<Args>(args)...));
        o.destructor = [](uint8_t* d) {
            delete reinterpret_cast<T*>(d);
        };
        return o;
    }

    template<typename T>
    inline T& as_cpp() { return *reinterpret_cast<T *>(__zngur_data); }
  };

  template<typename T>
  struct Ref;

  template<typename T>
  struct RefMut;


  // offset encoded in type
  template <typename Parent, size_t OFFSET>
  struct FieldStaticOffset {};

  // specialized per type to acquire offset from `extern const size_t`
  template <typename Parent, size_t INDEX>
  struct FieldAutoOffset {
      static size_t offset() noexcept;
  };

  // specialize to record allocation strategy into type system
  template <typename T>
  struct zngur_heap_allocated : ::std::false_type {};

  namespace zngur_detail {
    // std::conjunction analog for c++11 support
    template <typename... Conds>
    struct conjunction : std::true_type {};

    template <typename Cond, typename... Conds>
    struct conjunction<Cond, Conds...>
        : std::conditional<Cond::value, conjunction<Conds...>,
                          std::false_type>::type {};

    // Offset can be computed at compile time
    template <typename...>
    struct is_static_offset : std::false_type {};

    // a static offset not behind a pointer can be computed
    template <typename Parent, size_t OFFSET>
    struct is_static_offset<FieldStaticOffset<Parent, OFFSET>>
        : std::true_type {};

    // All offsets can be compile time computed
    template <typename First, typename... Rest>
    using all_static_offset =
        conjunction<is_static_offset<First>, is_static_offset<Rest>...>;
  
  }  // namespace zngur_detail

  template <typename T>
  struct __zngur_internal_field {
      // offset for this type and offset
      static constexpr size_t offset();
      // is this offset behind a pointer
      static constexpr bool heap_allocated();
  };

  template <typename Parent, size_t OFFSET>
  struct __zngur_internal_field<FieldStaticOffset<Parent, OFFSET>> {
      static constexpr size_t offset() { return OFFSET; }
      static constexpr bool heap_allocated() { return zngur_heap_allocated<Parent>::value; }
  };

  template <typename Parent, size_t INDEX>
  struct __zngur_internal_field<FieldAutoOffset<Parent, INDEX>> {
      static size_t offset() {
          return FieldAutoOffset<Parent, INDEX>::offset();
      }
      static constexpr bool heap_allocated() { return zngur_heap_allocated<Parent>::value; }
  };

  // NOTE: Offsets are in reverse order, last offset first.
  // this allows offsets to be prepended in versions of c++ that don't have fold 
  // expressions (pre c++17)
  template <typename... Offsets>
  struct __zngur_internal_calc_field {
      // used to statically calculate a total offset
      static constexpr size_t offset();
      // used to check if the owner of the first offset is heap allocated
      static constexpr bool heap_allocated();
  };

  // base case
  template <typename Offset>
  struct __zngur_internal_calc_field<Offset> {
      static constexpr size_t offset() {
          return __zngur_internal_field<Offset>::offset();
      }
      static constexpr bool heap_allocated() {
          return __zngur_internal_field<Offset>::heap_allocated();
      }
  };

  // recursively calculate offsets for Fields
  template <typename LastOffset, typename PrevOffset, typename... Offsets>
  struct __zngur_internal_calc_field<LastOffset, PrevOffset, Offsets...> {
      static constexpr size_t offset() {
          return __zngur_internal_calc_field<PrevOffset, Offsets...>::offset() +
                __zngur_internal_field<LastOffset>::offset();
      }
      static constexpr bool heap_allocated() {
          return __zngur_internal_calc_field<PrevOffset, Offsets...>::heap_allocated();
      }
  };

  template<typename T, typename Offset, typename... Offsets>
  struct FieldOwned {
    inline operator T() const noexcept { return *::rust::Ref<T>(*this); }
  };

  template<typename T, typename Offset, typename... Offsets>
  struct FieldRef {
    inline operator T() const noexcept { return *::rust::Ref<T>(*this); }
  };

  template<typename T, typename Offset, typename... Offsets>
  struct FieldRefMut {
    inline operator T() const noexcept { return *::rust::Ref<T>(*this); }
  };

#if __cplusplus >= 201703L
  // deduction guides
  template<typename T, typename Offset, typename... Offsets>
  Ref(const FieldOwned<T, Offset, Offsets...> &f) -> Ref<T>;
  template<typename T, typename Offset, typename... Offsets>
  Ref(const FieldRef<T, Offset, Offsets...> &f) -> Ref<T>;
  template<typename T, typename Offset, typename... Offsets>
  Ref(const FieldRefMut<T, Offset, Offsets...> &f) -> Ref<T>;
  template<typename T, typename Offset, typename... Offsets>
  RefMut(const FieldOwned<T, Offset, Offsets...> &f) -> RefMut<T>;
  template<typename T, typename Offset, typename... Offsets>
  RefMut(const FieldRef<T, Offset, Offsets...> &f) -> RefMut<T>;
  template<typename T, typename Offset, typename... Offsets>
  RefMut(const FieldRefMut<T, Offset, Offsets...> &f) -> RefMut<T>;
#endif

  template<typename T>
  struct zngur_is_unsized : ::std::false_type {};
  struct zngur_fat_pointer {
    uint8_t* __zngur_data;
    size_t metadata;
  };
  template<typename T>
  struct Raw {
      using DataType = typename ::std::conditional< zngur_is_unsized<T>::value, zngur_fat_pointer, uint8_t* >::type;
      DataType __zngur_data;
      Raw() {}
      Raw(Ref<T> value) {
          memcpy(&__zngur_data, __zngur_internal_data_ptr<Ref<T>>(value), __zngur_internal_size_of<Ref<T>>());
      }
      Raw(RefMut<T> value) {
          memcpy(&__zngur_data, __zngur_internal_data_ptr<RefMut<T>>(value), __zngur_internal_size_of<RefMut<T>>());
      }
      Raw(DataType data) : __zngur_data(data) {
      }
      Raw<T> offset(ptrdiff_t n) {
          return Raw(__zngur_data + n * __zngur_internal_size_of<T>());
      }
      Ref<T> read_ref() {
          Ref<T> value;
          memcpy(__zngur_internal_data_ptr<Ref<T>>(value), &__zngur_data, __zngur_internal_size_of<Ref<T>>());
          __zngur_internal_assume_init<Ref<T>>(value);
          return value;
      }
  };
  template<typename T>
  struct RawMut {
      using DataType = typename ::std::conditional< zngur_is_unsized<T>::value, zngur_fat_pointer, uint8_t* >::type;
      DataType __zngur_data;
      RawMut() {}
      RawMut(RefMut<T> value) {
          memcpy(&__zngur_data, __zngur_internal_data_ptr<RefMut<T>>(value), __zngur_internal_size_of<RefMut<T>>());
      }
      RawMut(DataType data) : __zngur_data(data) {
      }
      RawMut<T> offset(ptrdiff_t n) {
          return RawMut(__zngur_data + n * __zngur_internal_size_of<T>());
      }
      T read() {
          T value;
          memcpy(__zngur_internal_data_ptr<T>(value), __zngur_data, __zngur_internal_size_of<T>());
          __zngur_internal_assume_init<T>(value);
          return value;
      }
      Ref<T> read_ref() {
          Ref<T> value;
          memcpy(__zngur_internal_data_ptr<Ref<T>>(value), &__zngur_data, __zngur_internal_size_of<Ref<T>>());
          __zngur_internal_assume_init<Ref<T>>(value);
          return value;
      }
      RefMut<T> read_mut() {
          RefMut<T> value;
          memcpy(__zngur_internal_data_ptr<RefMut<T>>(value), &__zngur_data, __zngur_internal_size_of<RefMut<T>>());
          __zngur_internal_assume_init<RefMut<T>>(value);
          return value;
      }
      void write(T value) {
          memcpy(__zngur_data, __zngur_internal_data_ptr<T>(value), __zngur_internal_size_of<T>());
          __zngur_internal_assume_deinit<T>(value);
      }
  };
  template<typename... T>
  struct Tuple;

  using Unit = Tuple<>;

  template<typename T>
  struct ZngurPrettyPrinter;

  class Inherent;

  template<typename Type, typename Trait = Inherent>
  class Impl;

  template<typename T>
  T&& zngur_dbg_impl(const char* file_name, int line_number, const char* exp, T&& input) {
    ::std::cerr << "[" << file_name << ":" << line_number << "] " << exp << " = ";
    ZngurPrettyPrinter<typename ::std::remove_reference<T>::type>::print(input);
    return ::std::forward<T>(input);
  }

  // specializations for Refs of Refs

  

  template<typename T>
  struct Ref < Ref < T > > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const Ref < T >& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }
    
    // construct a ref from a FieldOwned if it's offset can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< Ref < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if it's offset can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< Ref < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    
    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< Ref < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< Ref < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }
    

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< Ref < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< Ref < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    Ref< T >& operator*() {
      return *reinterpret_cast< Ref < T >*>(__zngur_data);
    }
    Ref< T > const& operator*() const {
      return *reinterpret_cast< Ref < T >*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal< Ref < Ref < T > > >;
    friend ::rust::ZngurPrettyPrinter< Ref < Ref < T > > >;

  };

  template<typename T>
  struct __zngur_internal< Ref < Ref < T > > > {
    static inline uint8_t* data_ptr(const Ref < Ref < T > >& t) noexcept {
        return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t.__zngur_data));
    }
    static inline void assume_init(Ref < Ref < T > >&) noexcept {}

    static inline void check_init(const Ref < Ref < T > >&) noexcept {}

    static inline void assume_deinit(Ref < Ref < T > >&) noexcept {}

    static inline size_t size_of() noexcept {
        return __zngur_internal_size_of< Ref < T > >({});
    }
  };

  template<typename T>
  struct ZngurPrettyPrinter< Ref < Ref < T > > > {
    static inline void print(Ref < Ref < T > > const& t) {
      ::rust::__zngur_internal_check_init(t);
      ::rust::ZngurPrettyPrinter< Ref < T > >::print( reinterpret_cast< const Ref < T > &>(t.__zngur_data) );
    }
  };

  

  template<typename T>
  struct Ref < RefMut < T > > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const RefMut < T >& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }
    
    // construct a ref from a FieldOwned if it's offset can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< RefMut < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if it's offset can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< RefMut < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    
    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< RefMut < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< RefMut < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }
    

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< RefMut < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< RefMut < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    RefMut< T >& operator*() {
      return *reinterpret_cast< RefMut < T >*>(__zngur_data);
    }
    RefMut< T > const& operator*() const {
      return *reinterpret_cast< RefMut < T >*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal< Ref < RefMut < T > > >;
    friend ::rust::ZngurPrettyPrinter< Ref < RefMut < T > > >;

  };

  template<typename T>
  struct __zngur_internal< Ref < RefMut < T > > > {
    static inline uint8_t* data_ptr(const Ref < RefMut < T > >& t) noexcept {
        return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t.__zngur_data));
    }
    static inline void assume_init(Ref < RefMut < T > >&) noexcept {}

    static inline void check_init(const Ref < RefMut < T > >&) noexcept {}

    static inline void assume_deinit(Ref < RefMut < T > >&) noexcept {}

    static inline size_t size_of() noexcept {
        return __zngur_internal_size_of< RefMut < T > >({});
    }
  };

  template<typename T>
  struct ZngurPrettyPrinter< Ref < RefMut < T > > > {
    static inline void print(Ref < RefMut < T > > const& t) {
      ::rust::__zngur_internal_check_init(t);
      ::rust::ZngurPrettyPrinter< RefMut < T > >::print( reinterpret_cast< const RefMut < T > &>(t.__zngur_data) );
    }
  };

  

  

  template<typename T>
  struct RefMut < Ref < T > > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(const Ref < T >& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }
    
    // construct a ref from a FieldOwned if it's offset can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< Ref < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if it's offset can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< Ref < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< Ref < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< Ref < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    Ref< T >& operator*() {
      return *reinterpret_cast< Ref < T >*>(__zngur_data);
    }
    Ref< T > const& operator*() const {
      return *reinterpret_cast< Ref < T >*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal< RefMut < Ref < T > > >;
    friend ::rust::ZngurPrettyPrinter< RefMut < Ref < T > > >;

  };

  template<typename T>
  struct __zngur_internal< RefMut < Ref < T > > > {
    static inline uint8_t* data_ptr(const RefMut < Ref < T > >& t) noexcept {
        return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t.__zngur_data));
    }
    static inline void assume_init(RefMut < Ref < T > >&) noexcept {}

    static inline void check_init(const RefMut < Ref < T > >&) noexcept {}

    static inline void assume_deinit(RefMut < Ref < T > >&) noexcept {}

    static inline size_t size_of() noexcept {
        return __zngur_internal_size_of< Ref < T > >({});
    }
  };

  template<typename T>
  struct ZngurPrettyPrinter< RefMut < Ref < T > > > {
    static inline void print(RefMut < Ref < T > > const& t) {
      ::rust::__zngur_internal_check_init(t);
      ::rust::ZngurPrettyPrinter< Ref < T > >::print( reinterpret_cast< const Ref < T > &>(t.__zngur_data) );
    }
  };

  

  template<typename T>
  struct RefMut < RefMut < T > > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(const RefMut < T >& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }
    
    // construct a ref from a FieldOwned if it's offset can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< RefMut < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if it's offset can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< RefMut < T >, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< RefMut < T >, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< RefMut < T >, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    RefMut< T >& operator*() {
      return *reinterpret_cast< RefMut < T >*>(__zngur_data);
    }
    RefMut< T > const& operator*() const {
      return *reinterpret_cast< RefMut < T >*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal< RefMut < RefMut < T > > >;
    friend ::rust::ZngurPrettyPrinter< RefMut < RefMut < T > > >;

  };

  template<typename T>
  struct __zngur_internal< RefMut < RefMut < T > > > {
    static inline uint8_t* data_ptr(const RefMut < RefMut < T > >& t) noexcept {
        return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t.__zngur_data));
    }
    static inline void assume_init(RefMut < RefMut < T > >&) noexcept {}

    static inline void check_init(const RefMut < RefMut < T > >&) noexcept {}

    static inline void assume_deinit(RefMut < RefMut < T > >&) noexcept {}

    static inline size_t size_of() noexcept {
        return __zngur_internal_size_of< RefMut < T > >({});
    }
  };

  template<typename T>
  struct ZngurPrettyPrinter< RefMut < RefMut < T > > > {
    static inline void print(RefMut < RefMut < T > > const& t) {
      ::rust::__zngur_internal_check_init(t);
      ::rust::ZngurPrettyPrinter< RefMut < T > >::print( reinterpret_cast< const RefMut < T > &>(t.__zngur_data) );
    }
  };

  



  
  

  template<>
  struct __zngur_internal< int8_t > {
    static inline uint8_t* data_ptr(const int8_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int8_t&) noexcept {}
    static inline void assume_deinit(int8_t&) noexcept {}
    static inline void check_init(int8_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int8_t);
    }
  };

  template<>
  struct __zngur_internal< int8_t* > {
    static inline uint8_t* data_ptr(int8_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int8_t*&) noexcept {}
    static inline void assume_deinit(int8_t*&) noexcept {}
    static inline void check_init(int8_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int8_t);
    }
  };

  template<>
  struct __zngur_internal< int8_t const* > {
    static inline uint8_t* data_ptr(int8_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int8_t const*&) noexcept {}
    static inline void assume_deinit(int8_t const*&) noexcept {}
    static inline void check_init(int8_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int8_t);
    }
  };


  template<>
  struct Ref< int8_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const int8_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int8_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int8_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int8_t& operator*() {
      return *reinterpret_cast< int8_t*>(__zngur_data);
    }

    int8_t const& operator*() const {
      return *reinterpret_cast< int8_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< int8_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int8_t > >;

  };

  template<>
  struct RefMut< int8_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(int8_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int8_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int8_t& operator*() {
        return *reinterpret_cast< int8_t*>(__zngur_data);
    }

    int8_t const& operator*() const {
        return *reinterpret_cast< int8_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< int8_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int8_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< int8_t > {
      static inline void print(int8_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< int8_t > > {
      static inline void print(Ref< int8_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< int8_t > > {
      static inline void print(RefMut< int8_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<int8_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<int8_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int8_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int8_t>&) noexcept {}
    static inline void check_init(::rust::Ref<int8_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int8_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<int8_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int8_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int8_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<int8_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int8_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<int8_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int8_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int8_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<int8_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int8_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<int8_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<int8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int8_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<int8_t>*>(__zngur_data);
    }

    ::rust::Ref<int8_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<int8_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<int8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int8_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<int8_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<int8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int8_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<int8_t>*>(__zngur_data);
    }

    ::rust::Ref<int8_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<int8_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<int8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int8_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<int8_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<int8_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int8_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int8_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<int8_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int8_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int8_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int8_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int8_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<int8_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int8_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int8_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int8_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int8_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<int8_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int8_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<int8_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<int8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int8_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<int8_t>*>(__zngur_data);
    }

    ::rust::RefMut<int8_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<int8_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<int8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int8_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<int8_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<int8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int8_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<int8_t>*>(__zngur_data);
    }

    ::rust::RefMut<int8_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<int8_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<int8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int8_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< uint8_t > {
    static inline uint8_t* data_ptr(const uint8_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint8_t&) noexcept {}
    static inline void assume_deinit(uint8_t&) noexcept {}
    static inline void check_init(uint8_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint8_t);
    }
  };

  template<>
  struct __zngur_internal< uint8_t* > {
    static inline uint8_t* data_ptr(uint8_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint8_t*&) noexcept {}
    static inline void assume_deinit(uint8_t*&) noexcept {}
    static inline void check_init(uint8_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint8_t);
    }
  };

  template<>
  struct __zngur_internal< uint8_t const* > {
    static inline uint8_t* data_ptr(uint8_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint8_t const*&) noexcept {}
    static inline void assume_deinit(uint8_t const*&) noexcept {}
    static inline void check_init(uint8_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint8_t);
    }
  };


  template<>
  struct Ref< uint8_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const uint8_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint8_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint8_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint8_t& operator*() {
      return *reinterpret_cast< uint8_t*>(__zngur_data);
    }

    uint8_t const& operator*() const {
      return *reinterpret_cast< uint8_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< uint8_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint8_t > >;

  };

  template<>
  struct RefMut< uint8_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(uint8_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint8_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint8_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint8_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint8_t& operator*() {
        return *reinterpret_cast< uint8_t*>(__zngur_data);
    }

    uint8_t const& operator*() const {
        return *reinterpret_cast< uint8_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< uint8_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint8_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< uint8_t > {
      static inline void print(uint8_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< uint8_t > > {
      static inline void print(Ref< uint8_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< uint8_t > > {
      static inline void print(RefMut< uint8_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<uint8_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<uint8_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint8_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint8_t>&) noexcept {}
    static inline void check_init(::rust::Ref<uint8_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint8_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint8_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint8_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint8_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<uint8_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint8_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint8_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint8_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint8_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<uint8_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint8_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<uint8_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<uint8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint8_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<uint8_t>*>(__zngur_data);
    }

    ::rust::Ref<uint8_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<uint8_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<uint8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint8_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<uint8_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<uint8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint8_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<uint8_t>*>(__zngur_data);
    }

    ::rust::Ref<uint8_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<uint8_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<uint8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint8_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<uint8_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<uint8_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint8_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint8_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<uint8_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint8_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint8_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint8_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint8_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint8_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint8_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint8_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint8_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint8_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint8_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint8_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint8_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<uint8_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<uint8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint8_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<uint8_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint8_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<uint8_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<uint8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint8_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<uint8_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<uint8_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint8_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint8_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint8_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<uint8_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint8_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<uint8_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<uint8_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint8_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< int16_t > {
    static inline uint8_t* data_ptr(const int16_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int16_t&) noexcept {}
    static inline void assume_deinit(int16_t&) noexcept {}
    static inline void check_init(int16_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int16_t);
    }
  };

  template<>
  struct __zngur_internal< int16_t* > {
    static inline uint8_t* data_ptr(int16_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int16_t*&) noexcept {}
    static inline void assume_deinit(int16_t*&) noexcept {}
    static inline void check_init(int16_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int16_t);
    }
  };

  template<>
  struct __zngur_internal< int16_t const* > {
    static inline uint8_t* data_ptr(int16_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int16_t const*&) noexcept {}
    static inline void assume_deinit(int16_t const*&) noexcept {}
    static inline void check_init(int16_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int16_t);
    }
  };


  template<>
  struct Ref< int16_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const int16_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int16_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int16_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int16_t& operator*() {
      return *reinterpret_cast< int16_t*>(__zngur_data);
    }

    int16_t const& operator*() const {
      return *reinterpret_cast< int16_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< int16_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int16_t > >;

  };

  template<>
  struct RefMut< int16_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(int16_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int16_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int16_t& operator*() {
        return *reinterpret_cast< int16_t*>(__zngur_data);
    }

    int16_t const& operator*() const {
        return *reinterpret_cast< int16_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< int16_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int16_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< int16_t > {
      static inline void print(int16_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< int16_t > > {
      static inline void print(Ref< int16_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< int16_t > > {
      static inline void print(RefMut< int16_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<int16_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<int16_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int16_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int16_t>&) noexcept {}
    static inline void check_init(::rust::Ref<int16_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int16_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<int16_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int16_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int16_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<int16_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int16_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<int16_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int16_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int16_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<int16_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int16_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<int16_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<int16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int16_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<int16_t>*>(__zngur_data);
    }

    ::rust::Ref<int16_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<int16_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<int16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int16_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<int16_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<int16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int16_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<int16_t>*>(__zngur_data);
    }

    ::rust::Ref<int16_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<int16_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<int16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int16_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<int16_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<int16_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int16_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int16_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<int16_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int16_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int16_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int16_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int16_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<int16_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int16_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int16_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int16_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int16_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<int16_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int16_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<int16_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<int16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int16_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<int16_t>*>(__zngur_data);
    }

    ::rust::RefMut<int16_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<int16_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<int16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int16_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<int16_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<int16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int16_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<int16_t>*>(__zngur_data);
    }

    ::rust::RefMut<int16_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<int16_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<int16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int16_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< uint16_t > {
    static inline uint8_t* data_ptr(const uint16_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint16_t&) noexcept {}
    static inline void assume_deinit(uint16_t&) noexcept {}
    static inline void check_init(uint16_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint16_t);
    }
  };

  template<>
  struct __zngur_internal< uint16_t* > {
    static inline uint8_t* data_ptr(uint16_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint16_t*&) noexcept {}
    static inline void assume_deinit(uint16_t*&) noexcept {}
    static inline void check_init(uint16_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint16_t);
    }
  };

  template<>
  struct __zngur_internal< uint16_t const* > {
    static inline uint8_t* data_ptr(uint16_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint16_t const*&) noexcept {}
    static inline void assume_deinit(uint16_t const*&) noexcept {}
    static inline void check_init(uint16_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint16_t);
    }
  };


  template<>
  struct Ref< uint16_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const uint16_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint16_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint16_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint16_t& operator*() {
      return *reinterpret_cast< uint16_t*>(__zngur_data);
    }

    uint16_t const& operator*() const {
      return *reinterpret_cast< uint16_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< uint16_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint16_t > >;

  };

  template<>
  struct RefMut< uint16_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(uint16_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint16_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint16_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint16_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint16_t& operator*() {
        return *reinterpret_cast< uint16_t*>(__zngur_data);
    }

    uint16_t const& operator*() const {
        return *reinterpret_cast< uint16_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< uint16_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint16_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< uint16_t > {
      static inline void print(uint16_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< uint16_t > > {
      static inline void print(Ref< uint16_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< uint16_t > > {
      static inline void print(RefMut< uint16_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<uint16_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<uint16_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint16_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint16_t>&) noexcept {}
    static inline void check_init(::rust::Ref<uint16_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint16_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint16_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint16_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint16_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<uint16_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint16_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint16_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint16_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint16_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<uint16_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint16_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<uint16_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<uint16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint16_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<uint16_t>*>(__zngur_data);
    }

    ::rust::Ref<uint16_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<uint16_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<uint16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint16_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<uint16_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<uint16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint16_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<uint16_t>*>(__zngur_data);
    }

    ::rust::Ref<uint16_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<uint16_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<uint16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint16_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<uint16_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<uint16_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint16_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint16_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<uint16_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint16_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint16_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint16_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint16_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint16_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint16_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint16_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint16_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint16_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint16_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint16_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint16_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<uint16_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<uint16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint16_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<uint16_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint16_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<uint16_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<uint16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint16_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<uint16_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<uint16_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint16_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint16_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint16_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<uint16_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint16_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<uint16_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<uint16_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint16_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< int32_t > {
    static inline uint8_t* data_ptr(const int32_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int32_t&) noexcept {}
    static inline void assume_deinit(int32_t&) noexcept {}
    static inline void check_init(int32_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int32_t);
    }
  };

  template<>
  struct __zngur_internal< int32_t* > {
    static inline uint8_t* data_ptr(int32_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int32_t*&) noexcept {}
    static inline void assume_deinit(int32_t*&) noexcept {}
    static inline void check_init(int32_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int32_t);
    }
  };

  template<>
  struct __zngur_internal< int32_t const* > {
    static inline uint8_t* data_ptr(int32_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int32_t const*&) noexcept {}
    static inline void assume_deinit(int32_t const*&) noexcept {}
    static inline void check_init(int32_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int32_t);
    }
  };


  template<>
  struct Ref< int32_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const int32_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int32_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int32_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int32_t& operator*() {
      return *reinterpret_cast< int32_t*>(__zngur_data);
    }

    int32_t const& operator*() const {
      return *reinterpret_cast< int32_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< int32_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int32_t > >;

  };

  template<>
  struct RefMut< int32_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(int32_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int32_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int32_t& operator*() {
        return *reinterpret_cast< int32_t*>(__zngur_data);
    }

    int32_t const& operator*() const {
        return *reinterpret_cast< int32_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< int32_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int32_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< int32_t > {
      static inline void print(int32_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< int32_t > > {
      static inline void print(Ref< int32_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< int32_t > > {
      static inline void print(RefMut< int32_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<int32_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<int32_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int32_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int32_t>&) noexcept {}
    static inline void check_init(::rust::Ref<int32_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int32_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<int32_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int32_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int32_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<int32_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int32_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<int32_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int32_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int32_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<int32_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int32_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<int32_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<int32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int32_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<int32_t>*>(__zngur_data);
    }

    ::rust::Ref<int32_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<int32_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<int32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int32_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<int32_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<int32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int32_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<int32_t>*>(__zngur_data);
    }

    ::rust::Ref<int32_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<int32_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<int32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int32_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<int32_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<int32_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int32_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int32_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<int32_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int32_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int32_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int32_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int32_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<int32_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int32_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int32_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int32_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int32_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<int32_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int32_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<int32_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<int32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int32_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<int32_t>*>(__zngur_data);
    }

    ::rust::RefMut<int32_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<int32_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<int32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int32_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<int32_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<int32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int32_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<int32_t>*>(__zngur_data);
    }

    ::rust::RefMut<int32_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<int32_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<int32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int32_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< uint32_t > {
    static inline uint8_t* data_ptr(const uint32_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint32_t&) noexcept {}
    static inline void assume_deinit(uint32_t&) noexcept {}
    static inline void check_init(uint32_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint32_t);
    }
  };

  template<>
  struct __zngur_internal< uint32_t* > {
    static inline uint8_t* data_ptr(uint32_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint32_t*&) noexcept {}
    static inline void assume_deinit(uint32_t*&) noexcept {}
    static inline void check_init(uint32_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint32_t);
    }
  };

  template<>
  struct __zngur_internal< uint32_t const* > {
    static inline uint8_t* data_ptr(uint32_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint32_t const*&) noexcept {}
    static inline void assume_deinit(uint32_t const*&) noexcept {}
    static inline void check_init(uint32_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint32_t);
    }
  };


  template<>
  struct Ref< uint32_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const uint32_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint32_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint32_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint32_t& operator*() {
      return *reinterpret_cast< uint32_t*>(__zngur_data);
    }

    uint32_t const& operator*() const {
      return *reinterpret_cast< uint32_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< uint32_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint32_t > >;

  };

  template<>
  struct RefMut< uint32_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(uint32_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint32_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint32_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint32_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint32_t& operator*() {
        return *reinterpret_cast< uint32_t*>(__zngur_data);
    }

    uint32_t const& operator*() const {
        return *reinterpret_cast< uint32_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< uint32_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint32_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< uint32_t > {
      static inline void print(uint32_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< uint32_t > > {
      static inline void print(Ref< uint32_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< uint32_t > > {
      static inline void print(RefMut< uint32_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<uint32_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<uint32_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint32_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint32_t>&) noexcept {}
    static inline void check_init(::rust::Ref<uint32_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint32_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint32_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint32_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint32_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<uint32_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint32_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint32_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint32_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint32_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<uint32_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint32_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<uint32_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<uint32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint32_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<uint32_t>*>(__zngur_data);
    }

    ::rust::Ref<uint32_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<uint32_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<uint32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint32_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<uint32_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<uint32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint32_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<uint32_t>*>(__zngur_data);
    }

    ::rust::Ref<uint32_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<uint32_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<uint32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint32_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<uint32_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<uint32_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint32_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint32_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<uint32_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint32_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint32_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint32_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint32_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint32_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint32_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint32_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint32_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint32_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint32_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint32_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint32_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<uint32_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<uint32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint32_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<uint32_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint32_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<uint32_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<uint32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint32_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<uint32_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<uint32_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint32_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint32_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint32_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<uint32_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint32_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<uint32_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<uint32_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint32_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< int64_t > {
    static inline uint8_t* data_ptr(const int64_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int64_t&) noexcept {}
    static inline void assume_deinit(int64_t&) noexcept {}
    static inline void check_init(int64_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int64_t);
    }
  };

  template<>
  struct __zngur_internal< int64_t* > {
    static inline uint8_t* data_ptr(int64_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int64_t*&) noexcept {}
    static inline void assume_deinit(int64_t*&) noexcept {}
    static inline void check_init(int64_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int64_t);
    }
  };

  template<>
  struct __zngur_internal< int64_t const* > {
    static inline uint8_t* data_ptr(int64_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(int64_t const*&) noexcept {}
    static inline void assume_deinit(int64_t const*&) noexcept {}
    static inline void check_init(int64_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(int64_t);
    }
  };


  template<>
  struct Ref< int64_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const int64_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< int64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< int64_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< int64_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int64_t& operator*() {
      return *reinterpret_cast< int64_t*>(__zngur_data);
    }

    int64_t const& operator*() const {
      return *reinterpret_cast< int64_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< int64_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int64_t > >;

  };

  template<>
  struct RefMut< int64_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(int64_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< int64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< int64_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    int64_t& operator*() {
        return *reinterpret_cast< int64_t*>(__zngur_data);
    }

    int64_t const& operator*() const {
        return *reinterpret_cast< int64_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< int64_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< int64_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< int64_t > {
      static inline void print(int64_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< int64_t > > {
      static inline void print(Ref< int64_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< int64_t > > {
      static inline void print(RefMut< int64_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<int64_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<int64_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int64_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int64_t>&) noexcept {}
    static inline void check_init(::rust::Ref<int64_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int64_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<int64_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int64_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int64_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<int64_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<int64_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<int64_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<int64_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<int64_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<int64_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<int64_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<int64_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<int64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int64_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<int64_t>*>(__zngur_data);
    }

    ::rust::Ref<int64_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<int64_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<int64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int64_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<int64_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<int64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<int64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<int64_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<int64_t>*>(__zngur_data);
    }

    ::rust::Ref<int64_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<int64_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<int64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<int64_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<int64_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<int64_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int64_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int64_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<int64_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int64_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int64_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int64_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int64_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<int64_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<int64_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<int64_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<int64_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<int64_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<int64_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<int64_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<int64_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<int64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int64_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<int64_t>*>(__zngur_data);
    }

    ::rust::RefMut<int64_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<int64_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<int64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int64_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<int64_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<int64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<int64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<int64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<int64_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<int64_t>*>(__zngur_data);
    }

    ::rust::RefMut<int64_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<int64_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<int64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<int64_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< uint64_t > {
    static inline uint8_t* data_ptr(const uint64_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint64_t&) noexcept {}
    static inline void assume_deinit(uint64_t&) noexcept {}
    static inline void check_init(uint64_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint64_t);
    }
  };

  template<>
  struct __zngur_internal< uint64_t* > {
    static inline uint8_t* data_ptr(uint64_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint64_t*&) noexcept {}
    static inline void assume_deinit(uint64_t*&) noexcept {}
    static inline void check_init(uint64_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint64_t);
    }
  };

  template<>
  struct __zngur_internal< uint64_t const* > {
    static inline uint8_t* data_ptr(uint64_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(uint64_t const*&) noexcept {}
    static inline void assume_deinit(uint64_t const*&) noexcept {}
    static inline void check_init(uint64_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(uint64_t);
    }
  };


  template<>
  struct Ref< uint64_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const uint64_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< uint64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< uint64_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< uint64_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint64_t& operator*() {
      return *reinterpret_cast< uint64_t*>(__zngur_data);
    }

    uint64_t const& operator*() const {
      return *reinterpret_cast< uint64_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< uint64_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint64_t > >;

  };

  template<>
  struct RefMut< uint64_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(uint64_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< uint64_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint64_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< uint64_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    uint64_t& operator*() {
        return *reinterpret_cast< uint64_t*>(__zngur_data);
    }

    uint64_t const& operator*() const {
        return *reinterpret_cast< uint64_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< uint64_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< uint64_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< uint64_t > {
      static inline void print(uint64_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< uint64_t > > {
      static inline void print(Ref< uint64_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< uint64_t > > {
      static inline void print(RefMut< uint64_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<uint64_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<uint64_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint64_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint64_t>&) noexcept {}
    static inline void check_init(::rust::Ref<uint64_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint64_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint64_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint64_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint64_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<uint64_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<uint64_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<uint64_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<uint64_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<uint64_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<uint64_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<uint64_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<uint64_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<uint64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint64_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<uint64_t>*>(__zngur_data);
    }

    ::rust::Ref<uint64_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<uint64_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<uint64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint64_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<uint64_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<uint64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<uint64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<uint64_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<uint64_t>*>(__zngur_data);
    }

    ::rust::Ref<uint64_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<uint64_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<uint64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<uint64_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<uint64_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<uint64_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint64_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint64_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<uint64_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint64_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint64_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint64_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint64_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint64_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint64_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<uint64_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<uint64_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<uint64_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<uint64_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<uint64_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<uint64_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<uint64_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<uint64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint64_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<uint64_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint64_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<uint64_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<uint64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint64_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<uint64_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<uint64_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<uint64_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<uint64_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<uint64_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<uint64_t>*>(__zngur_data);
    }

    ::rust::RefMut<uint64_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<uint64_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<uint64_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<uint64_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::double_t > {
    static inline uint8_t* data_ptr(const ::double_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::double_t&) noexcept {}
    static inline void assume_deinit(::double_t&) noexcept {}
    static inline void check_init(::double_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::double_t);
    }
  };

  template<>
  struct __zngur_internal< ::double_t* > {
    static inline uint8_t* data_ptr(::double_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::double_t*&) noexcept {}
    static inline void assume_deinit(::double_t*&) noexcept {}
    static inline void check_init(::double_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::double_t);
    }
  };

  template<>
  struct __zngur_internal< ::double_t const* > {
    static inline uint8_t* data_ptr(::double_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::double_t const*&) noexcept {}
    static inline void assume_deinit(::double_t const*&) noexcept {}
    static inline void check_init(::double_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::double_t);
    }
  };


  template<>
  struct Ref< ::double_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::double_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::double_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::double_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::double_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::double_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::double_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::double_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::double_t& operator*() {
      return *reinterpret_cast< ::double_t*>(__zngur_data);
    }

    ::double_t const& operator*() const {
      return *reinterpret_cast< ::double_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::double_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::double_t > >;

  };

  template<>
  struct RefMut< ::double_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::double_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::double_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::double_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::double_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::double_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::double_t& operator*() {
        return *reinterpret_cast< ::double_t*>(__zngur_data);
    }

    ::double_t const& operator*() const {
        return *reinterpret_cast< ::double_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::double_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::double_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< ::double_t > {
      static inline void print(::double_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< ::double_t > > {
      static inline void print(Ref< ::double_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< ::double_t > > {
      static inline void print(RefMut< ::double_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<::double_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<::double_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::double_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::double_t>&) noexcept {}
    static inline void check_init(::rust::Ref<::double_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::double_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<::double_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<::double_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::double_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::double_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<::double_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::double_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<::double_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<::double_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::double_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::double_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<::double_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::double_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<::double_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<::double_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<::double_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<::double_t>*>(__zngur_data);
    }

    ::rust::Ref<::double_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<::double_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<::double_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<::double_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<::double_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<::double_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<::double_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<::double_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<::double_t>*>(__zngur_data);
    }

    ::rust::Ref<::double_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<::double_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<::double_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<::double_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<::double_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<::double_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::double_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::double_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<::double_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::double_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<::double_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<::double_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::double_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::double_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<::double_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::double_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<::double_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<::double_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::double_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::double_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<::double_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::double_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<::double_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<::double_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<::double_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<::double_t>*>(__zngur_data);
    }

    ::rust::RefMut<::double_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<::double_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<::double_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<::double_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<::double_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<::double_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<::double_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<::double_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<::double_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<::double_t>*>(__zngur_data);
    }

    ::rust::RefMut<::double_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<::double_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<::double_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<::double_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::float_t > {
    static inline uint8_t* data_ptr(const ::float_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::float_t&) noexcept {}
    static inline void assume_deinit(::float_t&) noexcept {}
    static inline void check_init(::float_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::float_t);
    }
  };

  template<>
  struct __zngur_internal< ::float_t* > {
    static inline uint8_t* data_ptr(::float_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::float_t*&) noexcept {}
    static inline void assume_deinit(::float_t*&) noexcept {}
    static inline void check_init(::float_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::float_t);
    }
  };

  template<>
  struct __zngur_internal< ::float_t const* > {
    static inline uint8_t* data_ptr(::float_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::float_t const*&) noexcept {}
    static inline void assume_deinit(::float_t const*&) noexcept {}
    static inline void check_init(::float_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::float_t);
    }
  };


  template<>
  struct Ref< ::float_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::float_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::float_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::float_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::float_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::float_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::float_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::float_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::float_t& operator*() {
      return *reinterpret_cast< ::float_t*>(__zngur_data);
    }

    ::float_t const& operator*() const {
      return *reinterpret_cast< ::float_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::float_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::float_t > >;

  };

  template<>
  struct RefMut< ::float_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::float_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::float_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::float_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::float_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::float_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::float_t& operator*() {
        return *reinterpret_cast< ::float_t*>(__zngur_data);
    }

    ::float_t const& operator*() const {
        return *reinterpret_cast< ::float_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::float_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::float_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< ::float_t > {
      static inline void print(::float_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< ::float_t > > {
      static inline void print(Ref< ::float_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< ::float_t > > {
      static inline void print(RefMut< ::float_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::Ref<::float_t> > {
    static inline uint8_t* data_ptr(const ::rust::Ref<::float_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::float_t>&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::float_t>&) noexcept {}
    static inline void check_init(::rust::Ref<::float_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::float_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<::float_t>* > {
    static inline uint8_t* data_ptr(::rust::Ref<::float_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::float_t>*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::float_t>*&) noexcept {}
    static inline void check_init(::rust::Ref<::float_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::float_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::Ref<::float_t> const* > {
    static inline uint8_t* data_ptr(::rust::Ref<::float_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::Ref<::float_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::Ref<::float_t> const*&) noexcept {}
    static inline void check_init(::rust::Ref<::float_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::Ref<::float_t>);
    }
  };


  template<>
  struct Ref< ::rust::Ref<::float_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::Ref<::float_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::Ref<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<::float_t>& operator*() {
      return *reinterpret_cast< ::rust::Ref<::float_t>*>(__zngur_data);
    }

    ::rust::Ref<::float_t> const& operator*() const {
      return *reinterpret_cast< ::rust::Ref<::float_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::Ref<::float_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<::float_t> > >;

  };

  template<>
  struct RefMut< ::rust::Ref<::float_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::Ref<::float_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::Ref<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::Ref<::float_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::Ref<::float_t>& operator*() {
        return *reinterpret_cast< ::rust::Ref<::float_t>*>(__zngur_data);
    }

    ::rust::Ref<::float_t> const& operator*() const {
        return *reinterpret_cast< ::rust::Ref<::float_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::Ref<::float_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::Ref<::float_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::RefMut<::float_t> > {
    static inline uint8_t* data_ptr(const ::rust::RefMut<::float_t>& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::float_t>&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::float_t>&) noexcept {}
    static inline void check_init(::rust::RefMut<::float_t>&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::float_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<::float_t>* > {
    static inline uint8_t* data_ptr(::rust::RefMut<::float_t>* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::float_t>*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::float_t>*&) noexcept {}
    static inline void check_init(::rust::RefMut<::float_t>*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::float_t>);
    }
  };

  template<>
  struct __zngur_internal< ::rust::RefMut<::float_t> const* > {
    static inline uint8_t* data_ptr(::rust::RefMut<::float_t> const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::RefMut<::float_t> const*&) noexcept {}
    static inline void assume_deinit(::rust::RefMut<::float_t> const*&) noexcept {}
    static inline void check_init(::rust::RefMut<::float_t> const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::RefMut<::float_t>);
    }
  };


  template<>
  struct Ref< ::rust::RefMut<::float_t> > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::RefMut<::float_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::RefMut<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<::float_t>& operator*() {
      return *reinterpret_cast< ::rust::RefMut<::float_t>*>(__zngur_data);
    }

    ::rust::RefMut<::float_t> const& operator*() const {
      return *reinterpret_cast< ::rust::RefMut<::float_t>*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::RefMut<::float_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<::float_t> > >;

  };

  template<>
  struct RefMut< ::rust::RefMut<::float_t> > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::RefMut<::float_t>& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::RefMut<::float_t>, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::RefMut<::float_t>, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::RefMut<::float_t>& operator*() {
        return *reinterpret_cast< ::rust::RefMut<::float_t>*>(__zngur_data);
    }

    ::rust::RefMut<::float_t> const& operator*() const {
        return *reinterpret_cast< ::rust::RefMut<::float_t>*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::RefMut<::float_t> > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::RefMut<::float_t> > >;
  };

  
  

  

// end builtin types

  
  

  template<>
  struct __zngur_internal< ::rust::ZngurCppOpaqueOwnedObject > {
    static inline uint8_t* data_ptr(const ::rust::ZngurCppOpaqueOwnedObject& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::ZngurCppOpaqueOwnedObject&) noexcept {}
    static inline void assume_deinit(::rust::ZngurCppOpaqueOwnedObject&) noexcept {}
    static inline void check_init(::rust::ZngurCppOpaqueOwnedObject&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::ZngurCppOpaqueOwnedObject);
    }
  };

  template<>
  struct __zngur_internal< ::rust::ZngurCppOpaqueOwnedObject* > {
    static inline uint8_t* data_ptr(::rust::ZngurCppOpaqueOwnedObject* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::ZngurCppOpaqueOwnedObject*&) noexcept {}
    static inline void assume_deinit(::rust::ZngurCppOpaqueOwnedObject*&) noexcept {}
    static inline void check_init(::rust::ZngurCppOpaqueOwnedObject*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::ZngurCppOpaqueOwnedObject);
    }
  };

  template<>
  struct __zngur_internal< ::rust::ZngurCppOpaqueOwnedObject const* > {
    static inline uint8_t* data_ptr(::rust::ZngurCppOpaqueOwnedObject const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::rust::ZngurCppOpaqueOwnedObject const*&) noexcept {}
    static inline void assume_deinit(::rust::ZngurCppOpaqueOwnedObject const*&) noexcept {}
    static inline void check_init(::rust::ZngurCppOpaqueOwnedObject const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::rust::ZngurCppOpaqueOwnedObject);
    }
  };


  template<>
  struct Ref< ::rust::ZngurCppOpaqueOwnedObject > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::rust::ZngurCppOpaqueOwnedObject& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::ZngurCppOpaqueOwnedObject& operator*() {
      return *reinterpret_cast< ::rust::ZngurCppOpaqueOwnedObject*>(__zngur_data);
    }

    ::rust::ZngurCppOpaqueOwnedObject const& operator*() const {
      return *reinterpret_cast< ::rust::ZngurCppOpaqueOwnedObject*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::rust::ZngurCppOpaqueOwnedObject > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::ZngurCppOpaqueOwnedObject > >;

  };

  template<>
  struct RefMut< ::rust::ZngurCppOpaqueOwnedObject > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::rust::ZngurCppOpaqueOwnedObject& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::rust::ZngurCppOpaqueOwnedObject, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::rust::ZngurCppOpaqueOwnedObject& operator*() {
        return *reinterpret_cast< ::rust::ZngurCppOpaqueOwnedObject*>(__zngur_data);
    }

    ::rust::ZngurCppOpaqueOwnedObject const& operator*() const {
        return *reinterpret_cast< ::rust::ZngurCppOpaqueOwnedObject*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::rust::ZngurCppOpaqueOwnedObject > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::rust::ZngurCppOpaqueOwnedObject > >;
  };

  
  

  

// end builtin types

  
  
    #if defined(__APPLE__) || defined(__wasm__)
  

  template<>
  struct __zngur_internal< ::size_t > {
    static inline uint8_t* data_ptr(const ::size_t& t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::size_t&) noexcept {}
    static inline void assume_deinit(::size_t&) noexcept {}
    static inline void check_init(::size_t&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::size_t);
    }
  };

  template<>
  struct __zngur_internal< ::size_t* > {
    static inline uint8_t* data_ptr(::size_t* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::size_t*&) noexcept {}
    static inline void assume_deinit(::size_t*&) noexcept {}
    static inline void check_init(::size_t*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::size_t);
    }
  };

  template<>
  struct __zngur_internal< ::size_t const* > {
    static inline uint8_t* data_ptr(::size_t const* const & t) noexcept {
      return const_cast<uint8_t*>(reinterpret_cast<const uint8_t*>(&t));
    }
    static inline void assume_init(::size_t const*&) noexcept {}
    static inline void assume_deinit(::size_t const*&) noexcept {}
    static inline void check_init(::size_t const*&) noexcept {}
    static inline size_t size_of() noexcept {
      return sizeof(::size_t);
    }
  };


  template<>
  struct Ref< ::size_t > {
    Ref() {
      __zngur_data = 0;
    }

    Ref(const ::size_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::size_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldOwned< ::size_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRef if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::size_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRef if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRef< ::size_t, Offset, Offsets...>& f) {
        size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::size_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    Ref(const FieldRefMut< ::size_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::size_t& operator*() {
      return *reinterpret_cast< ::size_t*>(__zngur_data);
    }

    ::size_t const& operator*() const {
      return *reinterpret_cast< ::size_t*>(__zngur_data);
    }

  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<Ref< ::size_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::size_t > >;

  };

  template<>
  struct RefMut< ::size_t > {
    RefMut() {
      __zngur_data = 0;
    }

    RefMut(::size_t& t) {
      __zngur_data = reinterpret_cast<size_t>(__zngur_internal_data_ptr(t));
    }

    // construct a ref from a FieldOwned if all of it's offsets can be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::size_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldOwned if any of it's offsets can not be calculated at
    // compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldOwned< ::size_t, Offset, Offsets... >& f) {
      constexpr bool heap_allocated = __zngur_internal_calc_field<Offset, Offsets...>::heap_allocated();
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      if (heap_allocated) {
        __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
      } else {
        __zngur_data = reinterpret_cast<size_t>(&f) + offset;
      }
    }

    // construct a ref from a FieldRefMut if all of it's offsets can be calculated
    // at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::size_t, Offset, Offsets...>& f) {
      constexpr size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    // construct a ref from a FieldRefMut if any of it's offsets can not be
    // calculated at compile time
    template <
        typename Offset,
        typename... Offsets,
        typename ::std::enable_if<
            !zngur_detail::all_static_offset<Offset, Offsets...>::value,
            bool>::type = true>
    RefMut(const FieldRefMut< ::size_t, Offset, Offsets...>& f) {
      size_t offset = __zngur_internal_calc_field<Offset, Offsets...>::offset();
      __zngur_data = *reinterpret_cast<const size_t*>(&f) + offset;
    }

    ::size_t& operator*() {
        return *reinterpret_cast< ::size_t*>(__zngur_data);
    }

    ::size_t const& operator*() const {
        return *reinterpret_cast< ::size_t*>(__zngur_data);
    }
  private:
    size_t __zngur_data;
    friend ::rust::__zngur_internal<RefMut< ::size_t > >;
    friend ::rust::ZngurPrettyPrinter< Ref< ::size_t > >;
  };

  
  
    template<>
    struct ZngurPrettyPrinter< ::size_t > {
      static inline void print(::size_t const& t) {
        ::std::cerr << t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< Ref< ::size_t > > {
      static inline void print(Ref< ::size_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
    template<>
    struct ZngurPrettyPrinter< RefMut< ::size_t > > {
      static inline void print(RefMut< ::size_t > const& t) {
        ::std::cerr << *t << ::std::endl;
      }
    };
  

  
    #endif
  

// end builtin types


} // namespace rust
