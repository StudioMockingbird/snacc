//! Stable Rust-side ABI used by generated Snacc objects.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::collections::{HashMap, HashSet};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnaccString {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnaccView {
    pub ptr: *const u8,
    pub len: usize,
}

/// Erased, borrow-only input to one compiler-canonicalized concatenation.
/// `first` and `second` contain either a pointer/length pair or the exact bits
/// of one scalar selected by `tag`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnaccConcatPart {
    pub tag: u64,
    pub first: u64,
    pub second: u64,
}

/// The private descriptor used by compiler-generated `List<T>` values. Its
/// layout mirrors the compiler's `{ pointer, length, capacity }` collection
/// representation on every supported target.
#[repr(C)]
pub struct SnaccList {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

/// Private descriptors for compiler-owned hash collections. The pointed-to
/// allocation is one concrete Rust `HashMap`/`HashSet` selected by the
/// compiler for the collection's key and value types; the descriptor itself
/// is the only representation crossing the LLVM/runtime boundary.
#[repr(C)]
pub struct SnaccMap {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

#[repr(C)]
pub struct SnaccSet {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

struct StringI64Map {
    map: HashMap<String, i64>,
    order: Vec<String>,
}

struct I64I64Map {
    map: HashMap<i64, i64>,
    order: Vec<i64>,
}

struct StringSet {
    set: HashSet<String>,
    order: Vec<String>,
}

struct I64Set {
    set: HashSet<i64>,
    order: Vec<i64>,
}

macro_rules! scalar_map_runtime {
    (
        $store:ident, $mut_fn:ident, $ref_fn:ident,
        $insert:ident, $contains:ident, $index:ident, $key_at:ident,
        $value_at:ident, $delete:ident, $take:ident, $clear:ident,
        $reserve:ident, $drop:ident, $key:ty
    ) => {
        struct $store {
            map: HashMap<$key, i64>,
            order: Vec<$key>,
        }

        fn $mut_fn<'a>(map: &'a mut SnaccMap) -> &'a mut $store {
            if map.ptr.is_null() {
                map.ptr = Box::into_raw(Box::new($store {
                    map: HashMap::new(),
                    order: Vec::new(),
                }))
                .cast();
            }
            // Safety: this descriptor is initialized with exactly this store
            // type by the corresponding compiler-selected runtime entry point.
            unsafe { &mut *map.ptr.cast::<$store>() }
        }

        fn $ref_fn<'a>(map: &'a SnaccMap) -> Option<&'a $store> {
            if map.ptr.is_null() {
                None
            } else {
                // Safety: this descriptor was initialized by this store's
                // corresponding mutable helper.
                Some(unsafe { &*map.ptr.cast::<$store>() })
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $insert(map: *mut SnaccMap, key: $key, value: i64) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.insert received a null descriptor") };
            let store = $mut_fn(map);
            let fresh = !store.map.contains_key(&key);
            if fresh {
                store.order.push(key);
            }
            store.map.insert(key, value);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            u8::from(fresh)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $contains(map: *const SnaccMap, key: $key) -> u8 {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return 0;
            };
            u8::from($ref_fn(map).is_some_and(|store| store.map.contains_key(&key)))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $index(map: *const SnaccMap, key: $key) -> i64 {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(value) = $ref_fn(map).and_then(|store| store.map.get(&key)) else {
                snacc_collection_bounds_fail()
            };
            *value
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $key_at(map: *const SnaccMap, index: i64) -> $key {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            *store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail())
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $value_at(map: *const SnaccMap, index: i64) -> i64 {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            let key = store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            *store
                .map
                .get(key)
                .unwrap_or_else(|| snacc_collection_bounds_fail())
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $delete(map: *mut SnaccMap, key: $key) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.delete received a null descriptor") };
            let store = $mut_fn(map);
            let existed = store.map.remove(&key).is_some();
            if existed {
                store.order.retain(|entry| entry != &key);
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            u8::from(existed)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $take(map: *mut SnaccMap, key: $key) -> i64 {
            let map = unsafe { map.as_mut().expect("Map.take received a null descriptor") };
            let store = $mut_fn(map);
            let Some(value) = store.map.remove(&key) else {
                snacc_collection_bounds_fail()
            };
            store.order.retain(|entry| entry != &key);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            value
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $clear(map: *mut SnaccMap) {
            let map = unsafe { map.as_mut().expect("Map.clear received a null descriptor") };
            let store = $mut_fn(map);
            store.map.clear();
            store.order.clear();
            let cap = store.map.capacity().max(store.order.capacity());
            sync_map(map, 0, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $reserve(map: *mut SnaccMap, minimum: i64) {
            let map = unsafe {
                map.as_mut()
                    .expect("Map.reserve received a null descriptor")
            };
            let store = $mut_fn(map);
            let additional = reserve_target(minimum, store.map.len());
            if additional != 0 {
                store.map.reserve(additional);
                store.order.reserve(additional);
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $drop(map: *const SnaccMap) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return;
            };
            if !map.ptr.is_null() {
                // Safety: this descriptor was initialized as this concrete
                // store and has not previously been destroyed.
                unsafe { drop(Box::from_raw(map.ptr.cast::<$store>())) };
            }
        }
    };
}

/// Copies an opaque compiler-owned value between aligned storage and a byte
/// vector. The runtime never interprets the value or runs its destructor;
/// typed ownership remains with compiler-generated lowering.
unsafe fn copy_raw_bytes(src: *const u8, dst: *mut u8, size: usize) {
    if size != 0 {
        // Safety: callers provide initialized, non-overlapping ranges of the
        // exact byte count represented by the checked value type.
        unsafe { std::ptr::copy_nonoverlapping(src, dst, size) };
    }
}

macro_rules! raw_scalar_map_runtime {
    (
        $store:ident, $mut_fn:ident, $ref_fn:ident,
        $insert:ident, $contains:ident, $index:ident, $key_at:ident,
        $value_at:ident, $delete:ident, $take:ident, $clear:ident,
        $reserve:ident, $drop:ident, $key:ty
    ) => {
        struct $store {
            map: HashMap<$key, Vec<u8>>,
            order: Vec<$key>,
        }

        fn $mut_fn<'a>(map: &'a mut SnaccMap) -> &'a mut $store {
            if map.ptr.is_null() {
                map.ptr = Box::into_raw(Box::new($store {
                    map: HashMap::new(),
                    order: Vec::new(),
                }))
                .cast();
            }
            // Safety: this descriptor is initialized with this store by the
            // compiler-selected raw runtime entry points.
            unsafe { &mut *map.ptr.cast::<$store>() }
        }

        fn $ref_fn<'a>(map: &'a SnaccMap) -> Option<&'a $store> {
            if map.ptr.is_null() {
                None
            } else {
                // Safety: this descriptor was initialized by this store's
                // corresponding mutable helper.
                Some(unsafe { &*map.ptr.cast::<$store>() })
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $insert(
            map: *mut SnaccMap,
            key: $key,
            value: *const u8,
            value_size: usize,
            old: *mut u8,
        ) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.insert received a null descriptor") };
            let store = $mut_fn(map);
            let fresh = !store.map.contains_key(&key);
            if fresh {
                store.order.push(key);
            }
            let bytes = if value_size == 0 {
                Vec::new()
            } else {
                // Safety: `value` points to the initialized temporary value
                // created by the compiler for exactly `value_size` bytes.
                unsafe { std::slice::from_raw_parts(value, value_size).to_vec() }
            };
            if let Some(previous) = store.map.insert(key, bytes) {
                // Safety: `old` is an aligned compiler-owned slot whenever
                // the caller can need the replaced value.
                unsafe { copy_raw_bytes(previous.as_ptr(), old, previous.len()) };
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            u8::from(fresh)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $contains(map: *const SnaccMap, key: $key) -> u8 {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return 0;
            };
            u8::from($ref_fn(map).is_some_and(|store| store.map.contains_key(&key)))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $index(map: *const SnaccMap, key: $key, out: *mut u8, value_size: usize) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(bytes) = $ref_fn(map).and_then(|store| store.map.get(&key)) else {
                snacc_collection_bounds_fail()
            };
            if bytes.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: `out` is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(bytes.as_ptr(), out, value_size) };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $key_at(map: *const SnaccMap, index: i64) -> $key {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            *store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail())
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $value_at(
            map: *const SnaccMap,
            index: i64,
            out: *mut u8,
            value_size: usize,
        ) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            let key = store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            let bytes = store
                .map
                .get(key)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            if bytes.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: `out` is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(bytes.as_ptr(), out, value_size) };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $delete(
            map: *mut SnaccMap,
            key: $key,
            old: *mut u8,
            value_size: usize,
        ) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.delete received a null descriptor") };
            let store = $mut_fn(map);
            let Some(previous) = store.map.remove(&key) else {
                return 0;
            };
            if previous.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: `old` is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(previous.as_ptr(), old, value_size) };
            store.order.retain(|entry| entry != &key);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            1
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $take(map: *mut SnaccMap, key: $key, out: *mut u8, value_size: usize) {
            let map = unsafe { map.as_mut().expect("Map.take received a null descriptor") };
            let store = $mut_fn(map);
            let Some(value) = store.map.remove(&key) else {
                snacc_collection_bounds_fail()
            };
            if value.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: `out` is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(value.as_ptr(), out, value_size) };
            store.order.retain(|entry| entry != &key);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $clear(map: *mut SnaccMap) {
            let map = unsafe { map.as_mut().expect("Map.clear received a null descriptor") };
            let store = $mut_fn(map);
            store.map.clear();
            store.order.clear();
            let cap = store.map.capacity().max(store.order.capacity());
            sync_map(map, 0, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $reserve(map: *mut SnaccMap, minimum: i64) {
            let map = unsafe {
                map.as_mut()
                    .expect("Map.reserve received a null descriptor")
            };
            let store = $mut_fn(map);
            let additional = reserve_target(minimum, store.map.len());
            if additional != 0 {
                store.map.reserve(additional);
                store.order.reserve(additional);
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $drop(map: *const SnaccMap) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return;
            };
            if !map.ptr.is_null() {
                // Safety: this descriptor was initialized as this concrete
                // store and has not previously been destroyed.
                unsafe { drop(Box::from_raw(map.ptr.cast::<$store>())) };
            }
        }
    };
}

/// Raw map storage for String keys. Keys are owned by the runtime, while the
/// value bytes remain opaque and are copied into compiler-owned typed slots.
/// String queries use borrowed UTF-8 views so lookup never consumes a query.
macro_rules! raw_string_map_runtime {
    (
        $store:ident, $mut_fn:ident, $ref_fn:ident,
        $insert:ident, $contains:ident, $index:ident, $key_at:ident,
        $value_at:ident, $delete:ident, $take:ident, $clear:ident,
        $reserve:ident, $drop:ident
    ) => {
        struct $store {
            map: HashMap<String, Vec<u8>>,
            order: Vec<String>,
        }

        fn $mut_fn<'a>(map: &'a mut SnaccMap) -> &'a mut $store {
            if map.ptr.is_null() {
                map.ptr = Box::into_raw(Box::new($store {
                    map: HashMap::new(),
                    order: Vec::new(),
                }))
                .cast();
            }
            // Safety: the compiler selects this store for String-keyed raw maps.
            unsafe { &mut *map.ptr.cast::<$store>() }
        }

        fn $ref_fn<'a>(map: &'a SnaccMap) -> Option<&'a $store> {
            if map.ptr.is_null() {
                None
            } else {
                // Safety: this descriptor was initialized by the matching helper.
                Some(unsafe { &*map.ptr.cast::<$store>() })
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $insert(
            map: *mut SnaccMap,
            key: *const SnaccString,
            value: *const u8,
            value_size: usize,
            old: *mut u8,
        ) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.insert received a null descriptor") };
            let key = unsafe { key.as_ref().expect("Map.insert received a null key") };
            let key = take_string(*key);
            let store = $mut_fn(map);
            let fresh = !store.map.contains_key(&key);
            if fresh {
                store.order.push(key.clone());
            }
            let bytes = if value_size == 0 {
                Vec::new()
            } else {
                // Safety: the compiler passes one initialized value slot.
                unsafe { std::slice::from_raw_parts(value, value_size).to_vec() }
            };
            if let Some(previous) = store.map.insert(key, bytes) {
                // Safety: old is an aligned compiler-owned replacement slot.
                unsafe { copy_raw_bytes(previous.as_ptr(), old, previous.len()) };
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            u8::from(fresh)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $contains(map: *const SnaccMap, key: *const SnaccView) -> u8 {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return 0;
            };
            let key = unsafe { key.as_ref().expect("Map.contains received a null key") };
            let Some(key) = view_text(*key) else { return 0 };
            u8::from($ref_fn(map).is_some_and(|store| store.map.contains_key(key)))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $index(
            map: *const SnaccMap,
            key: *const SnaccView,
            out: *mut u8,
            value_size: usize,
        ) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let key = unsafe { key.as_ref().expect("Map indexing received a null key") };
            let Some(key) = view_text(*key) else {
                snacc_collection_bounds_fail()
            };
            let Some(bytes) = $ref_fn(map).and_then(|store| store.map.get(key)) else {
                snacc_collection_bounds_fail()
            };
            if bytes.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: out is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(bytes.as_ptr(), out, value_size) };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $key_at(out: *mut SnaccString, map: *const SnaccMap, index: i64) {
            let out = unsafe { out.as_mut().expect("Map key lookup received a null output") };
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            let key = store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            // cap=0 marks a borrowed descriptor; the compiler owns no key data.
            *out = SnaccString {
                ptr: key.as_ptr().cast_mut(),
                len: key.len(),
                cap: 0,
            };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $value_at(
            map: *const SnaccMap,
            index: i64,
            out: *mut u8,
            value_size: usize,
        ) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(map) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            let key = store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            let bytes = store
                .map
                .get(key)
                .unwrap_or_else(|| snacc_collection_bounds_fail());
            if bytes.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: out is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(bytes.as_ptr(), out, value_size) };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $delete(
            map: *mut SnaccMap,
            key: *const SnaccView,
            old: *mut u8,
            value_size: usize,
        ) -> u8 {
            let map = unsafe { map.as_mut().expect("Map.delete received a null descriptor") };
            let key = unsafe { key.as_ref().expect("Map.delete received a null key") };
            let Some(key) = view_text(*key).map(str::to_owned) else {
                return 0;
            };
            let store = $mut_fn(map);
            let Some(previous) = store.map.remove(&key) else {
                return 0;
            };
            if previous.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: old is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(previous.as_ptr(), old, value_size) };
            store.order.retain(|entry| entry != &key);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
            1
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $take(
            map: *mut SnaccMap,
            key: *const SnaccView,
            out: *mut u8,
            value_size: usize,
        ) {
            let map = unsafe { map.as_mut().expect("Map.take received a null descriptor") };
            let key = unsafe { key.as_ref().expect("Map.take received a null key") };
            let Some(key) = view_text(*key).map(str::to_owned) else {
                snacc_collection_bounds_fail()
            };
            let store = $mut_fn(map);
            let Some(value) = store.map.remove(&key) else {
                snacc_collection_bounds_fail()
            };
            if value.len() != value_size {
                snacc_collection_bounds_fail()
            }
            // Safety: out is the compiler's aligned result slot.
            unsafe { copy_raw_bytes(value.as_ptr(), out, value_size) };
            store.order.retain(|entry| entry != &key);
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $clear(map: *mut SnaccMap) {
            let map = unsafe { map.as_mut().expect("Map.clear received a null descriptor") };
            let store = $mut_fn(map);
            store.map.clear();
            store.order.clear();
            let cap = store.map.capacity().max(store.order.capacity());
            sync_map(map, 0, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $reserve(map: *mut SnaccMap, minimum: i64) {
            let map = unsafe {
                map.as_mut()
                    .expect("Map.reserve received a null descriptor")
            };
            let store = $mut_fn(map);
            let additional = reserve_target(minimum, store.map.len());
            if additional != 0 {
                store.map.reserve(additional);
                store.order.reserve(additional);
            }
            let (len, cap) = (
                store.map.len(),
                store.map.capacity().max(store.order.capacity()),
            );
            sync_map(map, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $drop(map: *const SnaccMap) {
            let Some(map) = (unsafe { map.as_ref() }) else {
                return;
            };
            if !map.ptr.is_null() {
                // Safety: this descriptor was initialized by this store.
                unsafe { drop(Box::from_raw(map.ptr.cast::<$store>())) };
            }
        }
    };
}

raw_string_map_runtime!(
    StringRawMap,
    string_raw_map_mut,
    string_raw_map_ref,
    snacc_map_string_raw_insert,
    snacc_map_string_raw_contains,
    snacc_map_string_raw_index,
    snacc_map_string_raw_key_at,
    snacc_map_string_raw_value_at,
    snacc_map_string_raw_delete,
    snacc_map_string_raw_take,
    snacc_map_string_raw_clear,
    snacc_map_string_raw_reserve,
    snacc_map_string_raw_drop
);

macro_rules! scalar_set_runtime {
    (
        $store:ident, $mut_fn:ident, $ref_fn:ident,
        $insert:ident, $contains:ident, $at:ident, $delete:ident,
        $clear:ident, $reserve:ident, $drop:ident, $elem:ty
    ) => {
        struct $store {
            set: HashSet<$elem>,
            order: Vec<$elem>,
        }

        fn $mut_fn<'a>(set: &'a mut SnaccSet) -> &'a mut $store {
            if set.ptr.is_null() {
                set.ptr = Box::into_raw(Box::new($store {
                    set: HashSet::new(),
                    order: Vec::new(),
                }))
                .cast();
            }
            // Safety: this descriptor is initialized with exactly this store
            // type by the corresponding compiler-selected runtime entry point.
            unsafe { &mut *set.ptr.cast::<$store>() }
        }

        fn $ref_fn<'a>(set: &'a SnaccSet) -> Option<&'a $store> {
            if set.ptr.is_null() {
                None
            } else {
                // Safety: this descriptor was initialized by this store's
                // corresponding mutable helper.
                Some(unsafe { &*set.ptr.cast::<$store>() })
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $insert(set: *mut SnaccSet, value: $elem) -> u8 {
            let set = unsafe { set.as_mut().expect("Set.insert received a null descriptor") };
            let store = $mut_fn(set);
            let fresh = store.set.insert(value);
            if fresh {
                store.order.push(value);
            }
            let (len, cap) = (
                store.set.len(),
                store.set.capacity().max(store.order.capacity()),
            );
            sync_set(set, len, cap);
            u8::from(fresh)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $contains(set: *const SnaccSet, value: $elem) -> u8 {
            let Some(set) = (unsafe { set.as_ref() }) else {
                return 0;
            };
            u8::from($ref_fn(set).is_some_and(|store| store.set.contains(&value)))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $at(set: *const SnaccSet, index: i64) -> $elem {
            let Some(set) = (unsafe { set.as_ref() }) else {
                snacc_collection_bounds_fail()
            };
            let Some(store) = $ref_fn(set) else {
                snacc_collection_bounds_fail()
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            *store
                .order
                .get(index)
                .unwrap_or_else(|| snacc_collection_bounds_fail())
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $delete(set: *mut SnaccSet, value: $elem) -> u8 {
            let set = unsafe { set.as_mut().expect("Set.delete received a null descriptor") };
            let store = $mut_fn(set);
            let existed = store.set.remove(&value);
            if existed {
                store.order.retain(|entry| entry != &value);
            }
            let (len, cap) = (
                store.set.len(),
                store.set.capacity().max(store.order.capacity()),
            );
            sync_set(set, len, cap);
            u8::from(existed)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $clear(set: *mut SnaccSet) {
            let set = unsafe { set.as_mut().expect("Set.clear received a null descriptor") };
            let store = $mut_fn(set);
            store.set.clear();
            store.order.clear();
            let cap = store.set.capacity().max(store.order.capacity());
            sync_set(set, 0, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $reserve(set: *mut SnaccSet, minimum: i64) {
            let set = unsafe {
                set.as_mut()
                    .expect("Set.reserve received a null descriptor")
            };
            let store = $mut_fn(set);
            let additional = reserve_target(minimum, store.set.len());
            if additional != 0 {
                store.set.reserve(additional);
                store.order.reserve(additional);
            }
            let (len, cap) = (
                store.set.len(),
                store.set.capacity().max(store.order.capacity()),
            );
            sync_set(set, len, cap);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $drop(set: *const SnaccSet) {
            let Some(set) = (unsafe { set.as_ref() }) else {
                return;
            };
            if !set.ptr.is_null() {
                // Safety: this descriptor was initialized as this concrete
                // store and has not previously been destroyed.
                unsafe { drop(Box::from_raw(set.ptr.cast::<$store>())) };
            }
        }
    };
}

scalar_map_runtime!(
    ByteI64Map,
    byte_i64_map_mut,
    byte_i64_map_ref,
    snacc_map_u8_i64_insert,
    snacc_map_u8_i64_contains,
    snacc_map_u8_i64_index,
    snacc_map_u8_i64_key_at,
    snacc_map_u8_i64_value_at,
    snacc_map_u8_i64_delete,
    snacc_map_u8_i64_take,
    snacc_map_u8_i64_clear,
    snacc_map_u8_i64_reserve,
    snacc_map_u8_i64_drop,
    u8
);
scalar_map_runtime!(
    U16I64Map,
    u16_i64_map_mut,
    u16_i64_map_ref,
    snacc_map_u16_i64_insert,
    snacc_map_u16_i64_contains,
    snacc_map_u16_i64_index,
    snacc_map_u16_i64_key_at,
    snacc_map_u16_i64_value_at,
    snacc_map_u16_i64_delete,
    snacc_map_u16_i64_take,
    snacc_map_u16_i64_clear,
    snacc_map_u16_i64_reserve,
    snacc_map_u16_i64_drop,
    u16
);
scalar_map_runtime!(
    U32I64Map,
    u32_i64_map_mut,
    u32_i64_map_ref,
    snacc_map_u32_i64_insert,
    snacc_map_u32_i64_contains,
    snacc_map_u32_i64_index,
    snacc_map_u32_i64_key_at,
    snacc_map_u32_i64_value_at,
    snacc_map_u32_i64_delete,
    snacc_map_u32_i64_take,
    snacc_map_u32_i64_clear,
    snacc_map_u32_i64_reserve,
    snacc_map_u32_i64_drop,
    u32
);
scalar_map_runtime!(
    U64I64Map,
    u64_i64_map_mut,
    u64_i64_map_ref,
    snacc_map_u64_i64_insert,
    snacc_map_u64_i64_contains,
    snacc_map_u64_i64_index,
    snacc_map_u64_i64_key_at,
    snacc_map_u64_i64_value_at,
    snacc_map_u64_i64_delete,
    snacc_map_u64_i64_take,
    snacc_map_u64_i64_clear,
    snacc_map_u64_i64_reserve,
    snacc_map_u64_i64_drop,
    u64
);
scalar_map_runtime!(
    BoolI64Map,
    bool_i64_map_mut,
    bool_i64_map_ref,
    snacc_map_bool_i64_insert,
    snacc_map_bool_i64_contains,
    snacc_map_bool_i64_index,
    snacc_map_bool_i64_key_at,
    snacc_map_bool_i64_value_at,
    snacc_map_bool_i64_delete,
    snacc_map_bool_i64_take,
    snacc_map_bool_i64_clear,
    snacc_map_bool_i64_reserve,
    snacc_map_bool_i64_drop,
    u8
);
scalar_map_runtime!(
    UnicodeI64Map,
    unicode_i64_map_mut,
    unicode_i64_map_ref,
    snacc_map_unicode_i64_insert,
    snacc_map_unicode_i64_contains,
    snacc_map_unicode_i64_index,
    snacc_map_unicode_i64_key_at,
    snacc_map_unicode_i64_value_at,
    snacc_map_unicode_i64_delete,
    snacc_map_unicode_i64_take,
    snacc_map_unicode_i64_clear,
    snacc_map_unicode_i64_reserve,
    snacc_map_unicode_i64_drop,
    u32
);

raw_scalar_map_runtime!(
    ByteRawMap,
    byte_raw_map_mut,
    byte_raw_map_ref,
    snacc_map_u8_raw_insert,
    snacc_map_u8_raw_contains,
    snacc_map_u8_raw_index,
    snacc_map_u8_raw_key_at,
    snacc_map_u8_raw_value_at,
    snacc_map_u8_raw_delete,
    snacc_map_u8_raw_take,
    snacc_map_u8_raw_clear,
    snacc_map_u8_raw_reserve,
    snacc_map_u8_raw_drop,
    u8
);
raw_scalar_map_runtime!(
    U16RawMap,
    u16_raw_map_mut,
    u16_raw_map_ref,
    snacc_map_u16_raw_insert,
    snacc_map_u16_raw_contains,
    snacc_map_u16_raw_index,
    snacc_map_u16_raw_key_at,
    snacc_map_u16_raw_value_at,
    snacc_map_u16_raw_delete,
    snacc_map_u16_raw_take,
    snacc_map_u16_raw_clear,
    snacc_map_u16_raw_reserve,
    snacc_map_u16_raw_drop,
    u16
);
raw_scalar_map_runtime!(
    U32RawMap,
    u32_raw_map_mut,
    u32_raw_map_ref,
    snacc_map_u32_raw_insert,
    snacc_map_u32_raw_contains,
    snacc_map_u32_raw_index,
    snacc_map_u32_raw_key_at,
    snacc_map_u32_raw_value_at,
    snacc_map_u32_raw_delete,
    snacc_map_u32_raw_take,
    snacc_map_u32_raw_clear,
    snacc_map_u32_raw_reserve,
    snacc_map_u32_raw_drop,
    u32
);
raw_scalar_map_runtime!(
    U64RawMap,
    u64_raw_map_mut,
    u64_raw_map_ref,
    snacc_map_u64_raw_insert,
    snacc_map_u64_raw_contains,
    snacc_map_u64_raw_index,
    snacc_map_u64_raw_key_at,
    snacc_map_u64_raw_value_at,
    snacc_map_u64_raw_delete,
    snacc_map_u64_raw_take,
    snacc_map_u64_raw_clear,
    snacc_map_u64_raw_reserve,
    snacc_map_u64_raw_drop,
    u64
);
raw_scalar_map_runtime!(
    BoolRawMap,
    bool_raw_map_mut,
    bool_raw_map_ref,
    snacc_map_bool_raw_insert,
    snacc_map_bool_raw_contains,
    snacc_map_bool_raw_index,
    snacc_map_bool_raw_key_at,
    snacc_map_bool_raw_value_at,
    snacc_map_bool_raw_delete,
    snacc_map_bool_raw_take,
    snacc_map_bool_raw_clear,
    snacc_map_bool_raw_reserve,
    snacc_map_bool_raw_drop,
    u8
);
raw_scalar_map_runtime!(
    UnicodeRawMap,
    unicode_raw_map_mut,
    unicode_raw_map_ref,
    snacc_map_unicode_raw_insert,
    snacc_map_unicode_raw_contains,
    snacc_map_unicode_raw_index,
    snacc_map_unicode_raw_key_at,
    snacc_map_unicode_raw_value_at,
    snacc_map_unicode_raw_delete,
    snacc_map_unicode_raw_take,
    snacc_map_unicode_raw_clear,
    snacc_map_unicode_raw_reserve,
    snacc_map_unicode_raw_drop,
    u32
);
raw_scalar_map_runtime!(
    I64RawMap,
    i64_raw_map_mut,
    i64_raw_map_ref,
    snacc_map_i64_raw_insert,
    snacc_map_i64_raw_contains,
    snacc_map_i64_raw_index,
    snacc_map_i64_raw_key_at,
    snacc_map_i64_raw_value_at,
    snacc_map_i64_raw_delete,
    snacc_map_i64_raw_take,
    snacc_map_i64_raw_clear,
    snacc_map_i64_raw_reserve,
    snacc_map_i64_raw_drop,
    i64
);

scalar_set_runtime!(
    ByteSet,
    byte_set_mut,
    byte_set_ref,
    snacc_set_u8_insert,
    snacc_set_u8_contains,
    snacc_set_u8_at,
    snacc_set_u8_delete,
    snacc_set_u8_clear,
    snacc_set_u8_reserve,
    snacc_set_u8_drop,
    u8
);
scalar_set_runtime!(
    U16Set,
    u16_set_mut,
    u16_set_ref,
    snacc_set_u16_insert,
    snacc_set_u16_contains,
    snacc_set_u16_at,
    snacc_set_u16_delete,
    snacc_set_u16_clear,
    snacc_set_u16_reserve,
    snacc_set_u16_drop,
    u16
);
scalar_set_runtime!(
    U32Set,
    u32_set_mut,
    u32_set_ref,
    snacc_set_u32_insert,
    snacc_set_u32_contains,
    snacc_set_u32_at,
    snacc_set_u32_delete,
    snacc_set_u32_clear,
    snacc_set_u32_reserve,
    snacc_set_u32_drop,
    u32
);
scalar_set_runtime!(
    U64Set,
    u64_set_mut,
    u64_set_ref,
    snacc_set_u64_insert,
    snacc_set_u64_contains,
    snacc_set_u64_at,
    snacc_set_u64_delete,
    snacc_set_u64_clear,
    snacc_set_u64_reserve,
    snacc_set_u64_drop,
    u64
);
scalar_set_runtime!(
    BoolSet,
    bool_set_mut,
    bool_set_ref,
    snacc_set_bool_insert,
    snacc_set_bool_contains,
    snacc_set_bool_at,
    snacc_set_bool_delete,
    snacc_set_bool_clear,
    snacc_set_bool_reserve,
    snacc_set_bool_drop,
    u8
);
scalar_set_runtime!(
    UnicodeSet,
    unicode_set_mut,
    unicode_set_ref,
    snacc_set_unicode_insert,
    snacc_set_unicode_contains,
    snacc_set_unicode_at,
    snacc_set_unicode_delete,
    snacc_set_unicode_clear,
    snacc_set_unicode_reserve,
    snacc_set_unicode_drop,
    u32
);

/// Contract version for generated objects, runtime imports, and Rust bridges.
pub const ABI_VERSION: u32 = 12;

/// Terminates execution when a floating-point operation or bridge boundary
/// would expose an IEEE NaN as a Snacc value. The compiler emits an
/// unreachable edge after this call, so returning is not part of the ABI.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_invalid_floating_operation() -> ! {
    panic!("snacc: InvalidFloatingOperation")
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_f64(value: f64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_i64(value: i64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_bool(value: u8) {
    println!("{}", value != 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u8(value: u8) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u16(value: u16) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u32(value: u32) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_u64(value: u64) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_f32(value: f32) {
    println!("{value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_unicode(value: u32) {
    let scalar = char::from_u32(value).unwrap_or('\u{FFFD}');
    println!("{scalar}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_string(value: SnaccString) {
    // Safety: the compiler/runtime string invariant guarantees that `ptr`
    // points to `len` initialized UTF-8 bytes for every live descriptor.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let text = std::str::from_utf8(bytes).unwrap_or("�");
    println!("{text}");
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_string_ptr(value: *const SnaccString) {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return;
    };
    snacc_print_string(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_new(ptr: *const u8, len: usize) -> SnaccString {
    if len == 0 {
        return SnaccString {
            ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
            len: 0,
            cap: 0,
        };
    }
    // Safety: callers pass a compiler-created global or another validated
    // string range with at least `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut owned = Vec::with_capacity(len);
    owned.extend_from_slice(bytes);
    let result = SnaccString {
        ptr: owned.as_mut_ptr(),
        len,
        cap: owned.capacity(),
    };
    std::mem::forget(owned);
    result
}

/// Pointer-output form used by LLVM lowering because a three-word descriptor
/// has an indirect return convention on the Windows C ABI.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_new_out(out: *mut SnaccString, ptr: *const u8, len: usize) {
    let out = unsafe {
        out.as_mut()
            .expect("String construction received a null output")
    };
    *out = snacc_string_new(ptr, len);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_clone(value: SnaccString) -> SnaccString {
    snacc_string_new(value.ptr, value.len)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_clone_out(out: *mut SnaccString, value: *const SnaccString) {
    let out = unsafe { out.as_mut().expect("String clone received a null output") };
    let value = unsafe { value.as_ref().expect("String clone received a null input") };
    *out = snacc_string_clone(*value);
}

const CONCAT_TEXT: u64 = 0;
const CONCAT_I64: u64 = 1;
const CONCAT_U8: u64 = 2;
const CONCAT_U16: u64 = 3;
const CONCAT_U32: u64 = 4;
const CONCAT_U64: u64 = 5;
const CONCAT_F32: u64 = 6;
const CONCAT_F64: u64 = 7;
const CONCAT_BOOL: u64 = 8;
const CONCAT_UNICODE: u64 = 9;

struct StackText {
    bytes: [u8; 64],
    len: usize,
}

impl StackText {
    fn new() -> Self {
        Self {
            bytes: [0; 64],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl std::fmt::Write for StackText {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(std::fmt::Error)?;
        let destination = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        destination.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn scalar_concat_text(part: SnaccConcatPart) -> StackText {
    use std::fmt::Write as _;

    let mut text = StackText::new();
    let result = match part.tag {
        CONCAT_I64 => write!(&mut text, "{}", part.first as i64),
        CONCAT_U8 => write!(&mut text, "{}", part.first as u8),
        CONCAT_U16 => write!(&mut text, "{}", part.first as u16),
        CONCAT_U32 => write!(&mut text, "{}", part.first as u32),
        CONCAT_U64 => write!(&mut text, "{}", part.first as u64),
        CONCAT_F32 => write!(&mut text, "{}", f32::from_bits(part.first as u32)),
        CONCAT_F64 => write!(&mut text, "{}", f64::from_bits(part.first as u64)),
        CONCAT_BOOL => write!(&mut text, "{}", part.first != 0),
        CONCAT_UNICODE => {
            let scalar = char::from_u32(part.first as u32)
                .unwrap_or_else(|| panic!("snacc: invalid Unicode scalar in concatenation"));
            write!(&mut text, "{scalar}")
        }
        _ => panic!("snacc: invalid scalar concatenation tag"),
    };
    result.unwrap_or_else(|_| panic!("snacc: scalar formatting exceeded its fixed buffer"));
    text
}

unsafe fn concat_part_bytes<'a>(part: SnaccConcatPart) -> Option<&'a [u8]> {
    if part.tag != CONCAT_TEXT {
        return None;
    }
    // Safety: compiler lowering constructs text parts from a live String,
    // Unicode view, or global literal and retains every owner through this
    // call. The descriptor carries exactly `second` readable UTF-8 bytes.
    Some(unsafe {
        std::slice::from_raw_parts(part.first as usize as *const u8, part.second as usize)
    })
}

/// Builds a complete concat/interpolation plan with exactly one allocation for
/// the resulting String. Scalar formatting uses bounded stack storage.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_concat_parts_out(
    out: *mut SnaccString,
    parts: *const SnaccConcatPart,
    count: usize,
) {
    let out = unsafe {
        out.as_mut()
            .expect("String concatenation received a null output")
    };
    // Safety: lowering passes an array of exactly `count` initialized parts.
    let parts = unsafe { std::slice::from_raw_parts(parts, count) };
    let mut total = 0usize;
    for part in parts {
        // Safety: every text part points to storage retained through this call.
        let length = match unsafe { concat_part_bytes(*part) } {
            Some(bytes) => bytes.len(),
            None => scalar_concat_text(*part).len,
        };
        total = total
            .checked_add(length)
            .unwrap_or_else(|| panic!("snacc: string length overflow"));
    }
    if total == 0 {
        *out = SnaccString {
            ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
            len: 0,
            cap: 0,
        };
        return;
    }
    let mut owned = Vec::with_capacity(total);
    for part in parts {
        // Safety: every text part points to storage retained through this call.
        match unsafe { concat_part_bytes(*part) } {
            Some(bytes) => owned.extend_from_slice(bytes),
            None => owned.extend_from_slice(scalar_concat_text(*part).as_bytes()),
        }
    }
    debug_assert_eq!(owned.len(), total);
    *out = SnaccString {
        ptr: owned.as_mut_ptr(),
        len: owned.len(),
        cap: owned.capacity(),
    };
    std::mem::forget(owned);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_equal(left: SnaccString, right: SnaccString) -> u8 {
    // Safety: descriptors are validated live string ranges.
    let left_bytes = unsafe { std::slice::from_raw_parts(left.ptr, left.len) };
    let right_bytes = unsafe { std::slice::from_raw_parts(right.ptr, right.len) };
    u8::from(left_bytes == right_bytes)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_equal_ptr(
    left: *const SnaccString,
    right: *const SnaccString,
) -> u8 {
    let left = unsafe { left.as_ref().expect("String equality received a null left") };
    let right = unsafe {
        right
            .as_ref()
            .expect("String equality received a null right")
    };
    snacc_string_equal(*left, *right)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_drop(value: SnaccString) {
    if value.cap == 0 {
        return;
    }
    // Safety: this descriptor was produced by the string runtime and has not
    // previously been dropped; capacity and length are the original Vec facts.
    unsafe {
        drop(Vec::from_raw_parts(value.ptr, value.len, value.cap));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_drop_ptr(value: *const SnaccString) {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return;
    };
    snacc_string_drop(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_bytes(value: SnaccString) -> SnaccView {
    SnaccView {
        ptr: value.ptr,
        len: value.len,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_bytes_out(out: *mut SnaccView, value: *const SnaccString) {
    let out = unsafe { out.as_mut().expect("String view received a null output") };
    let value = unsafe { value.as_ref().expect("String view received a null input") };
    *out = snacc_string_bytes(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_unicode(value: SnaccString) -> SnaccView {
    snacc_string_bytes(value)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_unicode_out(out: *mut SnaccView, value: *const SnaccString) {
    let out = unsafe { out.as_mut().expect("String view received a null output") };
    let value = unsafe { value.as_ref().expect("String view received a null input") };
    *out = snacc_string_unicode(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_from_view(value: SnaccView) -> SnaccString {
    // Safety: the Unicode-view constructor accepts only valid UTF-8 ranges.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    snacc_string_new(bytes.as_ptr(), bytes.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_from_view_out(out: *mut SnaccString, value: *const SnaccView) {
    let out = unsafe {
        out.as_mut()
            .expect("String conversion received a null output")
    };
    let value = unsafe {
        value
            .as_ref()
            .expect("String conversion received a null view")
    };
    *out = snacc_string_from_view(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_from_utf8(value: SnaccView) -> SnaccString {
    // Safety: this view is a compiler-created range; validation happens before
    // the bytes are copied into the new owner.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    if std::str::from_utf8(bytes).is_err() {
        return SnaccString {
            ptr: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    snacc_string_new(bytes.as_ptr(), bytes.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_string_from_utf8_out(out: *mut SnaccString, value: *const SnaccView) {
    let out = unsafe {
        out.as_mut()
            .expect("UTF-8 conversion received a null output")
    };
    let value = unsafe {
        value
            .as_ref()
            .expect("UTF-8 conversion received a null view")
    };
    *out = snacc_string_from_utf8(*value);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_length(value: SnaccView) -> i64 {
    i64::try_from(value.len).unwrap_or_else(|_| panic!("snacc: view length overflow"))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_length_ptr(value: *const SnaccView) -> i64 {
    let value = unsafe { value.as_ref().expect("View length received a null view") };
    snacc_view_byte_length(*value)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_length(value: SnaccView) -> i64 {
    // Safety: a Unicode view is created only from a valid SnaccString range.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let text = std::str::from_utf8(bytes).unwrap_or("�");
    i64::try_from(text.chars().count()).unwrap_or_else(|_| panic!("snacc: view length overflow"))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_length_ptr(value: *const SnaccView) -> i64 {
    let value = unsafe { value.as_ref().expect("View length received a null view") };
    snacc_view_unicode_length(*value)
}

/// Compares two borrowed views by their encoded element sequence. Both view
/// families use UTF-8 storage in the current runtime, so byte equality is
/// also scalar-sequence equality for Unicode views.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_equal(left: SnaccView, right: SnaccView) -> u8 {
    // Safety: views are created only from live string ranges by the compiler.
    let left_bytes = unsafe { std::slice::from_raw_parts(left.ptr, left.len) };
    let right_bytes = unsafe { std::slice::from_raw_parts(right.ptr, right.len) };
    u8::from(left_bytes == right_bytes)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_equal_ptr(left: *const SnaccView, right: *const SnaccView) -> u8 {
    let left = unsafe { left.as_ref().expect("View equality received a null left") };
    let right = unsafe { right.as_ref().expect("View equality received a null right") };
    snacc_view_equal(*left, *right)
}

/// Returns the byte at `index`, or `-1` for a negative/out-of-range index.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_at(value: SnaccView, index: i64) -> i64 {
    if index < 0 {
        return -1;
    }
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    if index >= value.len {
        return -1;
    }
    // Safety: the checked compiler/runtime view invariant supplies this range.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    i64::from(bytes[index])
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_at_ptr(value: *const SnaccView, index: i64) -> i64 {
    let value = unsafe { value.as_ref().expect("View lookup received a null view") };
    snacc_view_byte_at(*value, index)
}

/// Returns the Unicode scalar at `index`, or `-1` for a negative/out-of-range
/// index. The scan is intentionally linear because UTF-8 scalars are variable
/// width.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_at(value: SnaccView, index: i64) -> i64 {
    if index < 0 {
        return -1;
    }
    // Safety: a Unicode view is created only from a valid SnaccString range.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let Some(scalar) = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.chars().nth(usize::try_from(index).ok()?))
    else {
        return -1;
    };
    i64::from(u32::from(scalar))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_at_ptr(value: *const SnaccView, index: i64) -> i64 {
    let value = unsafe { value.as_ref().expect("View lookup received a null view") };
    snacc_view_unicode_at(*value, index)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_slice(value: SnaccView, start: i64, end: i64) -> SnaccView {
    if start < 0 || end < start {
        return SnaccView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
    let start = usize::try_from(start).unwrap_or(usize::MAX);
    let end = usize::try_from(end).unwrap_or(usize::MAX);
    if end > value.len {
        return SnaccView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
    // Safety: the compiler-created view owns a valid readable range.
    let ptr = unsafe { value.ptr.add(start) };
    SnaccView {
        ptr,
        len: end - start,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_byte_slice_out(
    out: *mut SnaccView,
    value: *const SnaccView,
    start: i64,
    end: i64,
) {
    let out = unsafe { out.as_mut().expect("View slice received a null output") };
    let value = unsafe { value.as_ref().expect("View slice received a null input") };
    *out = snacc_view_byte_slice(*value, start, end);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_slice(value: SnaccView, start: i64, end: i64) -> SnaccView {
    if start < 0 || end < start {
        return SnaccView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
    let start = usize::try_from(start).unwrap_or(usize::MAX);
    let end = usize::try_from(end).unwrap_or(usize::MAX);
    // Safety: a Unicode view is created only from valid UTF-8 storage.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return SnaccView {
            ptr: std::ptr::null(),
            len: 0,
        };
    };
    let mut offsets = text
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    offsets.push(bytes.len());
    if end > offsets.len() - 1 {
        return SnaccView {
            ptr: std::ptr::null(),
            len: 0,
        };
    }
    let byte_start = offsets[start];
    let byte_end = offsets[end];
    // Safety: both offsets came from the valid UTF-8 range above.
    let ptr = unsafe { value.ptr.add(byte_start) };
    SnaccView {
        ptr,
        len: byte_end - byte_start,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_view_unicode_slice_out(
    out: *mut SnaccView,
    value: *const SnaccView,
    start: i64,
    end: i64,
) {
    let out = unsafe { out.as_mut().expect("View slice received a null output") };
    let value = unsafe { value.as_ref().expect("View slice received a null input") };
    *out = snacc_view_unicode_slice(*value, start, end);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_collection_bounds_fail() -> ! {
    panic!("snacc: collection index out of bounds")
}

fn list_reserve_to(
    list: &mut SnaccList,
    minimum: usize,
    element_size: usize,
    element_align: usize,
) {
    if minimum <= list.cap {
        return;
    }
    let mut new_cap = list.cap.max(4);
    while new_cap < minimum {
        new_cap = new_cap
            .checked_mul(2)
            .unwrap_or_else(|| panic!("snacc: list capacity overflow"));
    }
    let new_size = new_cap
        .checked_mul(element_size)
        .unwrap_or_else(|| panic!("snacc: list allocation overflow"));
    let new_ptr = snacc_alloc(new_size, element_align);
    if list.len != 0 {
        // Safety: the old descriptor contains `len` initialized scalar
        // elements, and the new allocation has room for all of them.
        unsafe {
            std::ptr::copy_nonoverlapping(
                list.ptr,
                new_ptr,
                list.len
                    .checked_mul(element_size)
                    .unwrap_or_else(|| panic!("snacc: list copy overflow")),
            );
        }
    }
    if list.cap != 0 {
        let old_size = list
            .cap
            .checked_mul(element_size)
            .unwrap_or_else(|| panic!("snacc: list allocation overflow"));
        snacc_dealloc(list.ptr, old_size, element_align);
    }
    list.ptr = new_ptr;
    list.cap = new_cap;
}

fn list_reserve(list: &mut SnaccList, element_size: usize, element_align: usize) {
    let minimum = list
        .len
        .checked_add(1)
        .unwrap_or_else(|| panic!("snacc: list length overflow"));
    list_reserve_to(list, minimum, element_size, element_align);
}

fn list_push<T: Copy>(list: *mut SnaccList, value: T) {
    if list.is_null() {
        panic!("snacc: List.push received a null descriptor")
    }
    // Safety: the compiler passes the address of a live List descriptor.
    let list = unsafe { &mut *list };
    let element_size = std::mem::size_of::<T>();
    list_reserve(list, element_size, std::mem::align_of::<T>());
    // Safety: reserve established an allocation with enough space and the
    // destination is the next uninitialized scalar slot.
    unsafe {
        std::ptr::write(list.ptr.add(list.len * element_size).cast::<T>(), value);
    }
    list.len += 1;
}

macro_rules! list_push_export {
    ($name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(list: *mut SnaccList, value: $ty) {
            list_push(list, value);
        }
    };
}

list_push_export!(snacc_list_push_i64, i64);
list_push_export!(snacc_list_push_u8, u8);
list_push_export!(snacc_list_push_u16, u16);
list_push_export!(snacc_list_push_u32, u32);
list_push_export!(snacc_list_push_u64, u64);
list_push_export!(snacc_list_push_f32, f32);
list_push_export!(snacc_list_push_f64, f64);
list_push_export!(snacc_list_push_bool, u8);
list_push_export!(snacc_list_push_unicode, u32);

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_clear(list: *mut SnaccList) {
    if list.is_null() {
        panic!("snacc: List.clear received a null descriptor")
    }
    // Safety: the compiler passes the address of a live List descriptor.
    unsafe {
        (*list).len = 0;
    }
}

fn list_pop<T: Copy>(list: *mut SnaccList) -> T {
    if list.is_null() {
        panic!("snacc: List.pop received a null descriptor")
    }
    // Safety: the compiler passes the address of a live List descriptor.
    let list = unsafe { &mut *list };
    if list.len == 0 {
        panic!("snacc: List.pop on an empty list")
    }
    list.len -= 1;
    // Safety: the final slot is initialized and remains within the allocation.
    unsafe {
        std::ptr::read(
            list.ptr
                .add(list.len * std::mem::size_of::<T>())
                .cast::<T>(),
        )
    }
}

fn list_insert<T: Copy>(list: *mut SnaccList, index: i64, value: T) {
    if list.is_null() {
        panic!("snacc: List.insert received a null descriptor")
    }
    if index < 0 {
        snacc_collection_bounds_fail();
    }
    // Safety: the compiler passes the address of a live List descriptor.
    let list = unsafe { &mut *list };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    if index > list.len {
        snacc_collection_bounds_fail();
    }
    let size = std::mem::size_of::<T>();
    list_reserve(list, size, std::mem::align_of::<T>());
    // Safety: reserve established one additional slot, and `copy` handles the
    // overlapping shift toward the end of the initialized range.
    unsafe {
        let ptr = list.ptr.cast::<T>();
        std::ptr::copy(ptr.add(index), ptr.add(index + 1), list.len - index);
        std::ptr::write(ptr.add(index), value);
    }
    list.len += 1;
}

fn list_remove<T: Copy>(list: *mut SnaccList, index: i64) -> T {
    if list.is_null() {
        panic!("snacc: List.remove received a null descriptor")
    }
    if index < 0 {
        snacc_collection_bounds_fail();
    }
    // Safety: the compiler passes the address of a live List descriptor.
    let list = unsafe { &mut *list };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    if index >= list.len {
        snacc_collection_bounds_fail();
    }
    // Safety: the indexed slot is initialized and the shifted range overlaps
    // only where `copy` is specifically permitted to handle it.
    unsafe {
        let ptr = list.ptr.cast::<T>();
        let removed = std::ptr::read(ptr.add(index));
        std::ptr::copy(ptr.add(index + 1), ptr.add(index), list.len - index - 1);
        list.len -= 1;
        removed
    }
}

macro_rules! list_scalar_exports {
    ($pop:ident, $insert:ident, $remove:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $pop(list: *mut SnaccList) -> $ty {
            list_pop(list)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $insert(list: *mut SnaccList, index: i64, value: $ty) {
            list_insert(list, index, value);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn $remove(list: *mut SnaccList, index: i64) -> $ty {
            list_remove(list, index)
        }
    };
}

list_scalar_exports!(
    snacc_list_pop_i64,
    snacc_list_insert_i64,
    snacc_list_remove_i64,
    i64
);
list_scalar_exports!(
    snacc_list_pop_u8,
    snacc_list_insert_u8,
    snacc_list_remove_u8,
    u8
);
list_scalar_exports!(
    snacc_list_pop_u16,
    snacc_list_insert_u16,
    snacc_list_remove_u16,
    u16
);
list_scalar_exports!(
    snacc_list_pop_u32,
    snacc_list_insert_u32,
    snacc_list_remove_u32,
    u32
);
list_scalar_exports!(
    snacc_list_pop_u64,
    snacc_list_insert_u64,
    snacc_list_remove_u64,
    u64
);
list_scalar_exports!(
    snacc_list_pop_f32,
    snacc_list_insert_f32,
    snacc_list_remove_f32,
    f32
);
list_scalar_exports!(
    snacc_list_pop_f64,
    snacc_list_insert_f64,
    snacc_list_remove_f64,
    f64
);
list_scalar_exports!(
    snacc_list_pop_bool,
    snacc_list_insert_bool,
    snacc_list_remove_bool,
    u8
);
list_scalar_exports!(
    snacc_list_pop_unicode,
    snacc_list_insert_unicode,
    snacc_list_remove_unicode,
    u32
);

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_reserve(
    list: *mut SnaccList,
    minimum: i64,
    element_size: usize,
    element_align: usize,
) {
    if list.is_null() {
        panic!("snacc: List.reserve received a null descriptor")
    }
    if minimum < 0 {
        snacc_collection_bounds_fail();
    }
    // Safety: the compiler passes the address of a live List descriptor.
    let list = unsafe { &mut *list };
    let minimum = usize::try_from(minimum).unwrap_or(usize::MAX);
    list_reserve_to(list, minimum, element_size, element_align);
}

/// Moves opaque compiler-owned list elements by bytes. The compiler uses this
/// path for non-`Copy` elements such as strings, boxes, and aggregates; it
/// performs element destruction separately, so the runtime never guesses a
/// Rust drop implementation for a Snacc value.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_push_raw(
    list: *mut SnaccList,
    value: *const u8,
    element_size: usize,
    element_align: usize,
) {
    let list = unsafe { list.as_mut().expect("List.push received a null descriptor") };
    let index = list.len;
    list_reserve(list, element_size, element_align);
    if element_size != 0 {
        // Safety: `value` points to one initialized value and reserve created
        // one uninitialized destination slot of exactly this byte size.
        unsafe {
            std::ptr::copy_nonoverlapping(value, list.ptr.add(index * element_size), element_size);
        }
    }
    list.len += 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_pop_raw(list: *mut SnaccList, out: *mut u8, element_size: usize) {
    let list = unsafe { list.as_mut().expect("List.pop received a null descriptor") };
    if list.len == 0 {
        panic!("snacc: List.pop on an empty list")
    }
    list.len -= 1;
    if element_size != 0 {
        // Safety: the final slot is initialized and `out` points to storage
        // allocated by the compiler for one returned value.
        unsafe {
            std::ptr::copy_nonoverlapping(list.ptr.add(list.len * element_size), out, element_size);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_insert_raw(
    list: *mut SnaccList,
    index: i64,
    value: *const u8,
    element_size: usize,
    element_align: usize,
) {
    let list = unsafe {
        list.as_mut()
            .expect("List.insert received a null descriptor")
    };
    if index < 0 {
        snacc_collection_bounds_fail();
    }
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    if index > list.len {
        snacc_collection_bounds_fail();
    }
    list_reserve(list, element_size, element_align);
    if element_size != 0 {
        // Safety: reserve created one additional slot; `copy` handles the
        // overlapping shift and the incoming value remains live until copied.
        unsafe {
            let start = list.ptr.add(index * element_size);
            std::ptr::copy(
                start,
                start.add(element_size),
                (list.len - index) * element_size,
            );
            std::ptr::copy_nonoverlapping(value, start, element_size);
        }
    }
    list.len += 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_remove_raw(
    list: *mut SnaccList,
    index: i64,
    out: *mut u8,
    element_size: usize,
) {
    let list = unsafe {
        list.as_mut()
            .expect("List.remove received a null descriptor")
    };
    if index < 0 {
        snacc_collection_bounds_fail();
    }
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    if index >= list.len {
        snacc_collection_bounds_fail();
    }
    if element_size != 0 {
        // Safety: the index is initialized, `out` is one compiler-owned
        // result slot, and the remaining initialized range is overlapping-safe.
        unsafe {
            let start = list.ptr.add(index * element_size);
            std::ptr::copy_nonoverlapping(start, out, element_size);
            std::ptr::copy(
                start.add(element_size),
                start,
                (list.len - index - 1) * element_size,
            );
        }
    }
    list.len -= 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_list_clear_raw(list: *mut SnaccList) {
    let list = unsafe {
        list.as_mut()
            .expect("List.clear received a null descriptor")
    };
    list.len = 0;
}

fn view_text(value: SnaccView) -> Option<&'static str> {
    // The returned reference is used only for the duration of the caller's
    // operation. The runtime cannot express that relationship in this C ABI,
    // so the helper is kept private and never stores the reference.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    std::str::from_utf8(bytes).ok()
}

fn take_string(value: SnaccString) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let text = String::from_utf8(bytes.to_vec()).expect("SnaccString must contain UTF-8");
    snacc_string_drop(value);
    text
}

fn map_string_i64_mut<'a>(map: &'a mut SnaccMap) -> &'a mut StringI64Map {
    if map.ptr.is_null() {
        map.ptr = Box::into_raw(Box::new(StringI64Map {
            map: HashMap::new(),
            order: Vec::new(),
        }))
        .cast();
    }
    // Safety: a StringI64Map is installed by this module for this descriptor.
    unsafe { &mut *map.ptr.cast::<StringI64Map>() }
}

fn map_string_i64_ref<'a>(map: &'a SnaccMap) -> Option<&'a StringI64Map> {
    if map.ptr.is_null() {
        None
    } else {
        // Safety: a StringI64Map is installed by this module for this descriptor.
        Some(unsafe { &*map.ptr.cast::<StringI64Map>() })
    }
}

fn sync_map(map: &mut SnaccMap, len: usize, cap: usize) {
    map.len = len;
    map.cap = cap;
}

fn sync_set(set: &mut SnaccSet, len: usize, cap: usize) {
    set.len = len;
    set.cap = cap;
}

fn reserve_target(minimum: i64, len: usize) -> usize {
    if minimum < 0 {
        snacc_collection_bounds_fail();
    }
    usize::try_from(minimum)
        .unwrap_or(usize::MAX)
        .saturating_sub(len)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_insert(
    map: *mut SnaccMap,
    key: *const SnaccString,
    value: i64,
) -> u8 {
    let map = unsafe { map.as_mut().expect("Map.insert received a null descriptor") };
    let key = unsafe { key.as_ref().expect("Map.insert received a null key") };
    let key = take_string(*key);
    let store = map_string_i64_mut(map);
    let fresh = !store.map.contains_key(&key);
    if fresh {
        store.order.push(key.clone());
    }
    store.map.insert(key, value);
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    u8::from(fresh)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_contains(map: *const SnaccMap, key: *const SnaccView) -> u8 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        return 0;
    };
    let key = unsafe { key.as_ref().expect("Map.contains received a null key") };
    let Some(key) = view_text(*key) else { return 0 };
    u8::from(map_string_i64_ref(map).is_some_and(|store| store.map.contains_key(key)))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_key_at(map: *const SnaccMap, index: i64) -> SnaccString {
    let Some(map) = (unsafe { map.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(store) = map_string_i64_ref(map) else {
        snacc_collection_bounds_fail()
    };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let key = store
        .order
        .get(index)
        .unwrap_or_else(|| snacc_collection_bounds_fail());
    // The zero capacity marks this as a borrowed descriptor. Loop bindings
    // cannot consume it, and `String.clone()` copies its bytes before any
    // owning operation can occur.
    SnaccString {
        ptr: key.as_ptr().cast_mut(),
        len: key.len(),
        cap: 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_key_at_out(
    out: *mut SnaccString,
    map: *const SnaccMap,
    index: i64,
) {
    let out = unsafe { out.as_mut().expect("Map key lookup received a null output") };
    *out = snacc_map_string_i64_key_at(map, index);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_index(map: *const SnaccMap, key: *const SnaccView) -> i64 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let key = unsafe { key.as_ref().expect("Map indexing received a null key") };
    let Some(key) = view_text(*key) else {
        snacc_collection_bounds_fail()
    };
    let Some(value) = map_string_i64_ref(map).and_then(|store| store.map.get(key)) else {
        snacc_collection_bounds_fail()
    };
    *value
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_delete(map: *mut SnaccMap, key: *const SnaccView) -> u8 {
    let map = unsafe { map.as_mut().expect("Map.delete received a null descriptor") };
    let key = unsafe { key.as_ref().expect("Map.delete received a null key") };
    let Some(key) = view_text(*key).map(str::to_owned) else {
        return 0;
    };
    let store = map_string_i64_mut(map);
    let existed = store.map.remove(&key).is_some();
    if existed {
        store.order.retain(|entry| entry != &key);
    }
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    u8::from(existed)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_take(map: *mut SnaccMap, key: *const SnaccView) -> i64 {
    let map = unsafe { map.as_mut().expect("Map.take received a null descriptor") };
    let key = unsafe { key.as_ref().expect("Map.take received a null key") };
    let Some(key) = view_text(*key).map(str::to_owned) else {
        snacc_collection_bounds_fail()
    };
    let store = map_string_i64_mut(map);
    let Some(value) = store.map.remove(&key) else {
        snacc_collection_bounds_fail()
    };
    store.order.retain(|entry| entry != &key);
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    value
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_clear(map: *mut SnaccMap) {
    let map = unsafe { map.as_mut().expect("Map.clear received a null descriptor") };
    let store = map_string_i64_mut(map);
    store.map.clear();
    store.order.clear();
    let cap = store.map.capacity();
    sync_map(map, 0, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_reserve(map: *mut SnaccMap, minimum: i64) {
    let map = unsafe {
        map.as_mut()
            .expect("Map.reserve received a null descriptor")
    };
    let store = map_string_i64_mut(map);
    let additional = reserve_target(minimum, store.map.len());
    if additional != 0 {
        store.map.reserve(additional);
        store.order.reserve(additional);
    }
    let (len, cap) = (
        store.map.len(),
        store.map.capacity().max(store.order.capacity()),
    );
    sync_map(map, len, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_string_i64_drop(map: *const SnaccMap) {
    let Some(map) = (unsafe { map.as_ref() }) else {
        return;
    };
    if !map.ptr.is_null() {
        // Safety: this descriptor was created for a StringI64Map by this module.
        unsafe { drop(Box::from_raw(map.ptr.cast::<StringI64Map>())) };
    }
}

fn map_i64_i64_mut<'a>(map: &'a mut SnaccMap) -> &'a mut I64I64Map {
    if map.ptr.is_null() {
        map.ptr = Box::into_raw(Box::new(I64I64Map {
            map: HashMap::new(),
            order: Vec::new(),
        }))
        .cast();
    }
    // Safety: an I64I64Map is installed by this module for this descriptor.
    unsafe { &mut *map.ptr.cast::<I64I64Map>() }
}

fn map_i64_i64_ref<'a>(map: &'a SnaccMap) -> Option<&'a I64I64Map> {
    if map.ptr.is_null() {
        None
    } else {
        // Safety: an I64I64Map is installed by this module for this descriptor.
        Some(unsafe { &*map.ptr.cast::<I64I64Map>() })
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_insert(map: *mut SnaccMap, key: i64, value: i64) -> u8 {
    let map = unsafe { map.as_mut().expect("Map.insert received a null descriptor") };
    let store = map_i64_i64_mut(map);
    let fresh = !store.map.contains_key(&key);
    if fresh {
        store.order.push(key);
    }
    store.map.insert(key, value);
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    u8::from(fresh)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_contains(map: *const SnaccMap, key: i64) -> u8 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        return 0;
    };
    u8::from(map_i64_i64_ref(map).is_some_and(|store| store.map.contains_key(&key)))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_index(map: *const SnaccMap, key: i64) -> i64 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(value) = map_i64_i64_ref(map).and_then(|store| store.map.get(&key)) else {
        snacc_collection_bounds_fail()
    };
    *value
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_key_at(map: *const SnaccMap, index: i64) -> i64 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(store) = map_i64_i64_ref(map) else {
        snacc_collection_bounds_fail()
    };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    *store
        .order
        .get(index)
        .unwrap_or_else(|| snacc_collection_bounds_fail())
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_value_at(map: *const SnaccMap, index: i64) -> i64 {
    let Some(map) = (unsafe { map.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(store) = map_i64_i64_ref(map) else {
        snacc_collection_bounds_fail()
    };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let key = store
        .order
        .get(index)
        .unwrap_or_else(|| snacc_collection_bounds_fail());
    *store
        .map
        .get(key)
        .unwrap_or_else(|| snacc_collection_bounds_fail())
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_delete(map: *mut SnaccMap, key: i64) -> u8 {
    let map = unsafe { map.as_mut().expect("Map.delete received a null descriptor") };
    let store = map_i64_i64_mut(map);
    let existed = store.map.remove(&key).is_some();
    if existed {
        store.order.retain(|entry| entry != &key);
    }
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    u8::from(existed)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_take(map: *mut SnaccMap, key: i64) -> i64 {
    let map = unsafe { map.as_mut().expect("Map.take received a null descriptor") };
    let store = map_i64_i64_mut(map);
    let Some(value) = store.map.remove(&key) else {
        snacc_collection_bounds_fail()
    };
    store.order.retain(|entry| entry != &key);
    let (len, cap) = (store.map.len(), store.map.capacity());
    sync_map(map, len, cap);
    value
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_clear(map: *mut SnaccMap) {
    let map = unsafe { map.as_mut().expect("Map.clear received a null descriptor") };
    let store = map_i64_i64_mut(map);
    store.map.clear();
    store.order.clear();
    let cap = store.map.capacity();
    sync_map(map, 0, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_reserve(map: *mut SnaccMap, minimum: i64) {
    let map = unsafe {
        map.as_mut()
            .expect("Map.reserve received a null descriptor")
    };
    let store = map_i64_i64_mut(map);
    let additional = reserve_target(minimum, store.map.len());
    if additional != 0 {
        store.map.reserve(additional);
        store.order.reserve(additional);
    }
    let (len, cap) = (
        store.map.len(),
        store.map.capacity().max(store.order.capacity()),
    );
    sync_map(map, len, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_map_i64_i64_drop(map: *const SnaccMap) {
    let Some(map) = (unsafe { map.as_ref() }) else {
        return;
    };
    if !map.ptr.is_null() {
        // Safety: this descriptor was created for an I64I64Map by this module.
        unsafe { drop(Box::from_raw(map.ptr.cast::<I64I64Map>())) };
    }
}

fn set_string_mut<'a>(set: &'a mut SnaccSet) -> &'a mut StringSet {
    if set.ptr.is_null() {
        set.ptr = Box::into_raw(Box::new(StringSet {
            set: HashSet::new(),
            order: Vec::new(),
        }))
        .cast();
    }
    // Safety: a StringSet is installed by this module for this descriptor.
    unsafe { &mut *set.ptr.cast::<StringSet>() }
}

fn set_string_ref<'a>(set: &'a SnaccSet) -> Option<&'a StringSet> {
    if set.ptr.is_null() {
        None
    } else {
        // Safety: a StringSet is installed by this module for this descriptor.
        Some(unsafe { &*set.ptr.cast::<StringSet>() })
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_insert(set: *mut SnaccSet, value: *const SnaccString) -> u8 {
    let set = unsafe { set.as_mut().expect("Set.insert received a null descriptor") };
    let value = unsafe { value.as_ref().expect("Set.insert received a null value") };
    let value = take_string(*value);
    let store = set_string_mut(set);
    let fresh = store.set.insert(value.clone());
    if fresh {
        store.order.push(value);
    }
    let (len, cap) = (store.set.len(), store.set.capacity());
    sync_set(set, len, cap);
    u8::from(fresh)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_contains(set: *const SnaccSet, value: *const SnaccView) -> u8 {
    let Some(set) = (unsafe { set.as_ref() }) else {
        return 0;
    };
    let value = unsafe { value.as_ref().expect("Set.contains received a null value") };
    let Some(value) = view_text(*value) else {
        return 0;
    };
    u8::from(set_string_ref(set).is_some_and(|store| store.set.contains(value)))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_at(set: *const SnaccSet, index: i64) -> SnaccString {
    let Some(set) = (unsafe { set.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(store) = set_string_ref(set) else {
        snacc_collection_bounds_fail()
    };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let value = store
        .order
        .get(index)
        .unwrap_or_else(|| snacc_collection_bounds_fail());
    // See `snacc_map_string_i64_key_at`: this is a borrowed descriptor, not
    // an allocation owned by the loop binding.
    SnaccString {
        ptr: value.as_ptr().cast_mut(),
        len: value.len(),
        cap: 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_at_out(out: *mut SnaccString, set: *const SnaccSet, index: i64) {
    let out = unsafe {
        out.as_mut()
            .expect("Set element lookup received a null output")
    };
    *out = snacc_set_string_at(set, index);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_delete(set: *mut SnaccSet, value: *const SnaccView) -> u8 {
    let set = unsafe { set.as_mut().expect("Set.delete received a null descriptor") };
    let value = unsafe { value.as_ref().expect("Set.delete received a null value") };
    let Some(value) = view_text(*value).map(str::to_owned) else {
        return 0;
    };
    let store = set_string_mut(set);
    let existed = store.set.remove(&value);
    if existed {
        store.order.retain(|entry| entry != &value);
    }
    let (len, cap) = (store.set.len(), store.set.capacity());
    sync_set(set, len, cap);
    u8::from(existed)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_clear(set: *mut SnaccSet) {
    let set = unsafe { set.as_mut().expect("Set.clear received a null descriptor") };
    let store = set_string_mut(set);
    store.set.clear();
    store.order.clear();
    let cap = store.set.capacity();
    sync_set(set, 0, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_reserve(set: *mut SnaccSet, minimum: i64) {
    let set = unsafe {
        set.as_mut()
            .expect("Set.reserve received a null descriptor")
    };
    let store = set_string_mut(set);
    let additional = reserve_target(minimum, store.set.len());
    if additional != 0 {
        store.set.reserve(additional);
        store.order.reserve(additional);
    }
    let (len, cap) = (
        store.set.len(),
        store.set.capacity().max(store.order.capacity()),
    );
    sync_set(set, len, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_string_drop(set: *const SnaccSet) {
    let Some(set) = (unsafe { set.as_ref() }) else {
        return;
    };
    if !set.ptr.is_null() {
        // Safety: this descriptor was created for a StringSet by this module.
        unsafe { drop(Box::from_raw(set.ptr.cast::<StringSet>())) };
    }
}

fn set_i64_mut<'a>(set: &'a mut SnaccSet) -> &'a mut I64Set {
    if set.ptr.is_null() {
        set.ptr = Box::into_raw(Box::new(I64Set {
            set: HashSet::new(),
            order: Vec::new(),
        }))
        .cast();
    }
    // Safety: an I64Set is installed by this module for this descriptor.
    unsafe { &mut *set.ptr.cast::<I64Set>() }
}

fn set_i64_ref<'a>(set: &'a SnaccSet) -> Option<&'a I64Set> {
    if set.ptr.is_null() {
        None
    } else {
        // Safety: an I64Set is installed by this module for this descriptor.
        Some(unsafe { &*set.ptr.cast::<I64Set>() })
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_insert(set: *mut SnaccSet, value: i64) -> u8 {
    let set = unsafe { set.as_mut().expect("Set.insert received a null descriptor") };
    let store = set_i64_mut(set);
    let fresh = store.set.insert(value);
    if fresh {
        store.order.push(value);
    }
    let (len, cap) = (store.set.len(), store.set.capacity());
    sync_set(set, len, cap);
    u8::from(fresh)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_contains(set: *const SnaccSet, value: i64) -> u8 {
    let Some(set) = (unsafe { set.as_ref() }) else {
        return 0;
    };
    u8::from(set_i64_ref(set).is_some_and(|store| store.set.contains(&value)))
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_at(set: *const SnaccSet, index: i64) -> i64 {
    let Some(set) = (unsafe { set.as_ref() }) else {
        snacc_collection_bounds_fail()
    };
    let Some(store) = set_i64_ref(set) else {
        snacc_collection_bounds_fail()
    };
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    *store
        .order
        .get(index)
        .unwrap_or_else(|| snacc_collection_bounds_fail())
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_delete(set: *mut SnaccSet, value: i64) -> u8 {
    let set = unsafe { set.as_mut().expect("Set.delete received a null descriptor") };
    let store = set_i64_mut(set);
    let existed = store.set.remove(&value);
    if existed {
        store.order.retain(|entry| entry != &value);
    }
    let (len, cap) = (store.set.len(), store.set.capacity());
    sync_set(set, len, cap);
    u8::from(existed)
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_clear(set: *mut SnaccSet) {
    let set = unsafe { set.as_mut().expect("Set.clear received a null descriptor") };
    let store = set_i64_mut(set);
    store.set.clear();
    store.order.clear();
    let cap = store.set.capacity();
    sync_set(set, 0, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_reserve(set: *mut SnaccSet, minimum: i64) {
    let set = unsafe {
        set.as_mut()
            .expect("Set.reserve received a null descriptor")
    };
    let store = set_i64_mut(set);
    let additional = reserve_target(minimum, store.set.len());
    if additional != 0 {
        store.set.reserve(additional);
        store.order.reserve(additional);
    }
    let (len, cap) = (
        store.set.len(),
        store.set.capacity().max(store.order.capacity()),
    );
    sync_set(set, len, cap);
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_set_i64_drop(set: *const SnaccSet) {
    let Some(set) = (unsafe { set.as_ref() }) else {
        return;
    };
    if !set.ptr.is_null() {
        // Safety: this descriptor was created for an I64Set by this module.
        unsafe { drop(Box::from_raw(set.ptr.cast::<I64Set>())) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn snacc_print_unicode_view(value: SnaccView) {
    // Safety: the compiler only sends string-backed Unicode views here.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    println!("{}", std::str::from_utf8(bytes).unwrap_or("�"));
}

/// Specification 016 section 8.2: `box(expression)` lowers to a call to this
/// allocator, which either returns valid, non-null, `align`-aligned storage
/// for `size` bytes or terminates the process -- never a null pointer, an
/// error code, or any other recoverable outcome.
///
/// A zero-sized pointee (Specification 016 section 8.2's closing paragraph)
/// is special-cased to a fixed non-null, well-aligned sentinel rather than
/// calling the global allocator, whose own safety contract forbids a
/// zero-size request (`std::alloc::alloc`'s documentation: "Undefined
/// behavior [...] if layout has zero size"). This is the same
/// never-read-or-written dangling-but-valid convention Rust's own
/// `Layout`/`NonNull::dangling()` use for a zero-sized type, so no real
/// allocation, and therefore no matching `snacc_dealloc` call, ever happens
/// for it -- `snacc_dealloc` mirrors this same `size == 0` special case.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_alloc(size: usize, align: usize) -> *mut u8 {
    if size == 0 {
        return align.max(1) as *mut u8;
    }
    let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| {
        panic!("snacc: invalid allocation request (size {size}, align {align})")
    });
    // Safety: `size` is non-zero, checked above, and `layout` was just built
    // by `Layout::from_size_align`, so it satisfies `alloc`'s only
    // requirement (non-zero size).
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}

/// Releases one allocation `snacc_alloc` returned for the exact same `size`
/// and `align` (Specification 016 section 8.1: a box releases its allocation
/// on normal destruction). A `size == 0` request never reaches the global
/// allocator here either, mirroring `snacc_alloc`'s own zero-size sentinel,
/// which must never be passed to `dealloc`.
#[unsafe(no_mangle)]
pub extern "C" fn snacc_dealloc(ptr: *mut u8, size: usize, align: usize) {
    if size == 0 {
        return;
    }
    let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| {
        panic!("snacc: invalid allocation request (size {size}, align {align})")
    });
    // Safety: the checked cleanup plan that emits this call always pairs it
    // with the `snacc_alloc` call that produced `ptr`, using that same
    // pointee's size and alignment, so `ptr` was allocated by the global
    // allocator with this exact `layout` and has not been freed already.
    unsafe { dealloc(ptr, layout) };
}

#[doc(hidden)]
#[inline(never)]
pub fn force_link() {
    let symbols = [
        snacc_print_f64 as *const () as usize,
        snacc_invalid_floating_operation as *const () as usize,
        snacc_print_i64 as *const () as usize,
        snacc_print_bool as *const () as usize,
        snacc_print_u8 as *const () as usize,
        snacc_print_u16 as *const () as usize,
        snacc_print_u32 as *const () as usize,
        snacc_print_u64 as *const () as usize,
        snacc_print_f32 as *const () as usize,
        snacc_print_unicode as *const () as usize,
        snacc_print_string as *const () as usize,
        snacc_string_new as *const () as usize,
        snacc_string_new_out as *const () as usize,
        snacc_string_clone as *const () as usize,
        snacc_string_equal as *const () as usize,
        snacc_string_drop as *const () as usize,
        snacc_string_drop_ptr as *const () as usize,
        snacc_string_bytes as *const () as usize,
        snacc_string_unicode as *const () as usize,
        snacc_string_from_view as *const () as usize,
        snacc_string_from_utf8 as *const () as usize,
        snacc_view_byte_length as *const () as usize,
        snacc_view_unicode_length as *const () as usize,
        snacc_view_equal as *const () as usize,
        snacc_view_byte_at as *const () as usize,
        snacc_view_unicode_at as *const () as usize,
        snacc_view_byte_slice as *const () as usize,
        snacc_view_unicode_slice as *const () as usize,
        snacc_collection_bounds_fail as *const () as usize,
        snacc_list_push_i64 as *const () as usize,
        snacc_list_push_u8 as *const () as usize,
        snacc_list_push_u16 as *const () as usize,
        snacc_list_push_u32 as *const () as usize,
        snacc_list_push_u64 as *const () as usize,
        snacc_list_push_f32 as *const () as usize,
        snacc_list_push_f64 as *const () as usize,
        snacc_list_push_bool as *const () as usize,
        snacc_list_push_unicode as *const () as usize,
        snacc_list_clear as *const () as usize,
        snacc_list_pop_i64 as *const () as usize,
        snacc_list_pop_u8 as *const () as usize,
        snacc_list_pop_u16 as *const () as usize,
        snacc_list_pop_u32 as *const () as usize,
        snacc_list_pop_u64 as *const () as usize,
        snacc_list_pop_f32 as *const () as usize,
        snacc_list_pop_f64 as *const () as usize,
        snacc_list_pop_bool as *const () as usize,
        snacc_list_pop_unicode as *const () as usize,
        snacc_list_insert_i64 as *const () as usize,
        snacc_list_insert_u8 as *const () as usize,
        snacc_list_insert_u16 as *const () as usize,
        snacc_list_insert_u32 as *const () as usize,
        snacc_list_insert_u64 as *const () as usize,
        snacc_list_insert_f32 as *const () as usize,
        snacc_list_insert_f64 as *const () as usize,
        snacc_list_insert_bool as *const () as usize,
        snacc_list_insert_unicode as *const () as usize,
        snacc_list_remove_i64 as *const () as usize,
        snacc_list_remove_u8 as *const () as usize,
        snacc_list_remove_u16 as *const () as usize,
        snacc_list_remove_u32 as *const () as usize,
        snacc_list_remove_u64 as *const () as usize,
        snacc_list_remove_f32 as *const () as usize,
        snacc_list_remove_f64 as *const () as usize,
        snacc_list_remove_bool as *const () as usize,
        snacc_list_remove_unicode as *const () as usize,
        snacc_list_reserve as *const () as usize,
        snacc_list_push_raw as *const () as usize,
        snacc_list_pop_raw as *const () as usize,
        snacc_list_insert_raw as *const () as usize,
        snacc_list_remove_raw as *const () as usize,
        snacc_list_clear_raw as *const () as usize,
        snacc_map_u8_raw_insert as *const () as usize,
        snacc_map_u8_raw_contains as *const () as usize,
        snacc_map_u8_raw_index as *const () as usize,
        snacc_map_u8_raw_key_at as *const () as usize,
        snacc_map_u8_raw_value_at as *const () as usize,
        snacc_map_u8_raw_delete as *const () as usize,
        snacc_map_u8_raw_take as *const () as usize,
        snacc_map_u8_raw_clear as *const () as usize,
        snacc_map_u8_raw_reserve as *const () as usize,
        snacc_map_u8_raw_drop as *const () as usize,
        snacc_map_u16_raw_insert as *const () as usize,
        snacc_map_u16_raw_contains as *const () as usize,
        snacc_map_u16_raw_index as *const () as usize,
        snacc_map_u16_raw_key_at as *const () as usize,
        snacc_map_u16_raw_value_at as *const () as usize,
        snacc_map_u16_raw_delete as *const () as usize,
        snacc_map_u16_raw_take as *const () as usize,
        snacc_map_u16_raw_clear as *const () as usize,
        snacc_map_u16_raw_reserve as *const () as usize,
        snacc_map_u16_raw_drop as *const () as usize,
        snacc_map_u32_raw_insert as *const () as usize,
        snacc_map_u32_raw_contains as *const () as usize,
        snacc_map_u32_raw_index as *const () as usize,
        snacc_map_u32_raw_key_at as *const () as usize,
        snacc_map_u32_raw_value_at as *const () as usize,
        snacc_map_u32_raw_delete as *const () as usize,
        snacc_map_u32_raw_take as *const () as usize,
        snacc_map_u32_raw_clear as *const () as usize,
        snacc_map_u32_raw_reserve as *const () as usize,
        snacc_map_u32_raw_drop as *const () as usize,
        snacc_map_u64_raw_insert as *const () as usize,
        snacc_map_u64_raw_contains as *const () as usize,
        snacc_map_u64_raw_index as *const () as usize,
        snacc_map_u64_raw_key_at as *const () as usize,
        snacc_map_u64_raw_value_at as *const () as usize,
        snacc_map_u64_raw_delete as *const () as usize,
        snacc_map_u64_raw_take as *const () as usize,
        snacc_map_u64_raw_clear as *const () as usize,
        snacc_map_u64_raw_reserve as *const () as usize,
        snacc_map_u64_raw_drop as *const () as usize,
        snacc_map_bool_raw_insert as *const () as usize,
        snacc_map_bool_raw_contains as *const () as usize,
        snacc_map_bool_raw_index as *const () as usize,
        snacc_map_bool_raw_key_at as *const () as usize,
        snacc_map_bool_raw_value_at as *const () as usize,
        snacc_map_bool_raw_delete as *const () as usize,
        snacc_map_bool_raw_take as *const () as usize,
        snacc_map_bool_raw_clear as *const () as usize,
        snacc_map_bool_raw_reserve as *const () as usize,
        snacc_map_bool_raw_drop as *const () as usize,
        snacc_map_unicode_raw_insert as *const () as usize,
        snacc_map_unicode_raw_contains as *const () as usize,
        snacc_map_unicode_raw_index as *const () as usize,
        snacc_map_unicode_raw_key_at as *const () as usize,
        snacc_map_unicode_raw_value_at as *const () as usize,
        snacc_map_unicode_raw_delete as *const () as usize,
        snacc_map_unicode_raw_take as *const () as usize,
        snacc_map_unicode_raw_clear as *const () as usize,
        snacc_map_unicode_raw_reserve as *const () as usize,
        snacc_map_unicode_raw_drop as *const () as usize,
        snacc_map_i64_raw_insert as *const () as usize,
        snacc_map_i64_raw_contains as *const () as usize,
        snacc_map_i64_raw_index as *const () as usize,
        snacc_map_i64_raw_key_at as *const () as usize,
        snacc_map_i64_raw_value_at as *const () as usize,
        snacc_map_i64_raw_delete as *const () as usize,
        snacc_map_i64_raw_take as *const () as usize,
        snacc_map_i64_raw_clear as *const () as usize,
        snacc_map_i64_raw_reserve as *const () as usize,
        snacc_map_i64_raw_drop as *const () as usize,
        snacc_map_string_raw_insert as *const () as usize,
        snacc_map_string_raw_contains as *const () as usize,
        snacc_map_string_raw_index as *const () as usize,
        snacc_map_string_raw_key_at as *const () as usize,
        snacc_map_string_raw_value_at as *const () as usize,
        snacc_map_string_raw_delete as *const () as usize,
        snacc_map_string_raw_take as *const () as usize,
        snacc_map_string_raw_clear as *const () as usize,
        snacc_map_string_raw_reserve as *const () as usize,
        snacc_map_string_raw_drop as *const () as usize,
        snacc_map_string_i64_insert as *const () as usize,
        snacc_map_string_i64_contains as *const () as usize,
        snacc_map_string_i64_key_at as *const () as usize,
        snacc_map_string_i64_key_at_out as *const () as usize,
        snacc_map_string_i64_index as *const () as usize,
        snacc_map_string_i64_delete as *const () as usize,
        snacc_map_string_i64_take as *const () as usize,
        snacc_map_string_i64_clear as *const () as usize,
        snacc_map_string_i64_reserve as *const () as usize,
        snacc_map_string_i64_drop as *const () as usize,
        snacc_map_i64_i64_insert as *const () as usize,
        snacc_map_i64_i64_contains as *const () as usize,
        snacc_map_i64_i64_index as *const () as usize,
        snacc_map_i64_i64_key_at as *const () as usize,
        snacc_map_i64_i64_value_at as *const () as usize,
        snacc_map_i64_i64_delete as *const () as usize,
        snacc_map_i64_i64_take as *const () as usize,
        snacc_map_i64_i64_clear as *const () as usize,
        snacc_map_i64_i64_reserve as *const () as usize,
        snacc_map_i64_i64_drop as *const () as usize,
        snacc_map_u8_i64_insert as *const () as usize,
        snacc_map_u8_i64_contains as *const () as usize,
        snacc_map_u8_i64_index as *const () as usize,
        snacc_map_u8_i64_key_at as *const () as usize,
        snacc_map_u8_i64_value_at as *const () as usize,
        snacc_map_u8_i64_delete as *const () as usize,
        snacc_map_u8_i64_take as *const () as usize,
        snacc_map_u8_i64_clear as *const () as usize,
        snacc_map_u8_i64_reserve as *const () as usize,
        snacc_map_u8_i64_drop as *const () as usize,
        snacc_map_u16_i64_insert as *const () as usize,
        snacc_map_u16_i64_contains as *const () as usize,
        snacc_map_u16_i64_index as *const () as usize,
        snacc_map_u16_i64_key_at as *const () as usize,
        snacc_map_u16_i64_value_at as *const () as usize,
        snacc_map_u16_i64_delete as *const () as usize,
        snacc_map_u16_i64_take as *const () as usize,
        snacc_map_u16_i64_clear as *const () as usize,
        snacc_map_u16_i64_reserve as *const () as usize,
        snacc_map_u16_i64_drop as *const () as usize,
        snacc_map_u32_i64_insert as *const () as usize,
        snacc_map_u32_i64_contains as *const () as usize,
        snacc_map_u32_i64_index as *const () as usize,
        snacc_map_u32_i64_key_at as *const () as usize,
        snacc_map_u32_i64_value_at as *const () as usize,
        snacc_map_u32_i64_delete as *const () as usize,
        snacc_map_u32_i64_take as *const () as usize,
        snacc_map_u32_i64_clear as *const () as usize,
        snacc_map_u32_i64_reserve as *const () as usize,
        snacc_map_u32_i64_drop as *const () as usize,
        snacc_map_u64_i64_insert as *const () as usize,
        snacc_map_u64_i64_contains as *const () as usize,
        snacc_map_u64_i64_index as *const () as usize,
        snacc_map_u64_i64_key_at as *const () as usize,
        snacc_map_u64_i64_value_at as *const () as usize,
        snacc_map_u64_i64_delete as *const () as usize,
        snacc_map_u64_i64_take as *const () as usize,
        snacc_map_u64_i64_clear as *const () as usize,
        snacc_map_u64_i64_reserve as *const () as usize,
        snacc_map_u64_i64_drop as *const () as usize,
        snacc_map_bool_i64_insert as *const () as usize,
        snacc_map_bool_i64_contains as *const () as usize,
        snacc_map_bool_i64_index as *const () as usize,
        snacc_map_bool_i64_key_at as *const () as usize,
        snacc_map_bool_i64_value_at as *const () as usize,
        snacc_map_bool_i64_delete as *const () as usize,
        snacc_map_bool_i64_take as *const () as usize,
        snacc_map_bool_i64_clear as *const () as usize,
        snacc_map_bool_i64_reserve as *const () as usize,
        snacc_map_bool_i64_drop as *const () as usize,
        snacc_map_unicode_i64_insert as *const () as usize,
        snacc_map_unicode_i64_contains as *const () as usize,
        snacc_map_unicode_i64_index as *const () as usize,
        snacc_map_unicode_i64_key_at as *const () as usize,
        snacc_map_unicode_i64_value_at as *const () as usize,
        snacc_map_unicode_i64_delete as *const () as usize,
        snacc_map_unicode_i64_take as *const () as usize,
        snacc_map_unicode_i64_clear as *const () as usize,
        snacc_map_unicode_i64_reserve as *const () as usize,
        snacc_map_unicode_i64_drop as *const () as usize,
        snacc_set_string_insert as *const () as usize,
        snacc_set_string_contains as *const () as usize,
        snacc_set_string_at as *const () as usize,
        snacc_set_string_at_out as *const () as usize,
        snacc_set_string_delete as *const () as usize,
        snacc_set_string_clear as *const () as usize,
        snacc_set_string_reserve as *const () as usize,
        snacc_set_string_drop as *const () as usize,
        snacc_set_i64_insert as *const () as usize,
        snacc_set_i64_contains as *const () as usize,
        snacc_set_i64_at as *const () as usize,
        snacc_set_i64_delete as *const () as usize,
        snacc_set_i64_clear as *const () as usize,
        snacc_set_i64_reserve as *const () as usize,
        snacc_set_i64_drop as *const () as usize,
        snacc_set_u8_insert as *const () as usize,
        snacc_set_u8_contains as *const () as usize,
        snacc_set_u8_at as *const () as usize,
        snacc_set_u8_delete as *const () as usize,
        snacc_set_u8_clear as *const () as usize,
        snacc_set_u8_reserve as *const () as usize,
        snacc_set_u8_drop as *const () as usize,
        snacc_set_u16_insert as *const () as usize,
        snacc_set_u16_contains as *const () as usize,
        snacc_set_u16_at as *const () as usize,
        snacc_set_u16_delete as *const () as usize,
        snacc_set_u16_clear as *const () as usize,
        snacc_set_u16_reserve as *const () as usize,
        snacc_set_u16_drop as *const () as usize,
        snacc_set_u32_insert as *const () as usize,
        snacc_set_u32_contains as *const () as usize,
        snacc_set_u32_at as *const () as usize,
        snacc_set_u32_delete as *const () as usize,
        snacc_set_u32_clear as *const () as usize,
        snacc_set_u32_reserve as *const () as usize,
        snacc_set_u32_drop as *const () as usize,
        snacc_set_u64_insert as *const () as usize,
        snacc_set_u64_contains as *const () as usize,
        snacc_set_u64_at as *const () as usize,
        snacc_set_u64_delete as *const () as usize,
        snacc_set_u64_clear as *const () as usize,
        snacc_set_u64_reserve as *const () as usize,
        snacc_set_u64_drop as *const () as usize,
        snacc_set_bool_insert as *const () as usize,
        snacc_set_bool_contains as *const () as usize,
        snacc_set_bool_at as *const () as usize,
        snacc_set_bool_delete as *const () as usize,
        snacc_set_bool_clear as *const () as usize,
        snacc_set_bool_reserve as *const () as usize,
        snacc_set_bool_drop as *const () as usize,
        snacc_set_unicode_insert as *const () as usize,
        snacc_set_unicode_contains as *const () as usize,
        snacc_set_unicode_at as *const () as usize,
        snacc_set_unicode_delete as *const () as usize,
        snacc_set_unicode_clear as *const () as usize,
        snacc_set_unicode_reserve as *const () as usize,
        snacc_set_unicode_drop as *const () as usize,
        snacc_print_unicode_view as *const () as usize,
        snacc_print_string_ptr as *const () as usize,
        snacc_string_new_out as *const () as usize,
        snacc_string_clone_out as *const () as usize,
        snacc_string_concat_parts_out as *const () as usize,
        snacc_string_equal_ptr as *const () as usize,
        snacc_string_bytes_out as *const () as usize,
        snacc_string_unicode_out as *const () as usize,
        snacc_string_from_view_out as *const () as usize,
        snacc_string_from_utf8_out as *const () as usize,
        snacc_view_byte_length_ptr as *const () as usize,
        snacc_view_unicode_length_ptr as *const () as usize,
        snacc_view_equal_ptr as *const () as usize,
        snacc_view_byte_at_ptr as *const () as usize,
        snacc_view_unicode_at_ptr as *const () as usize,
        snacc_view_byte_slice_out as *const () as usize,
        snacc_view_unicode_slice_out as *const () as usize,
        snacc_alloc as *const () as usize,
        snacc_dealloc as *const () as usize,
    ];
    std::hint::black_box(symbols);
}
