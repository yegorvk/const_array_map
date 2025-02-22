#![no_std]
#![allow(private_bounds)]

//! A `no_std`-compatible, const-capable associative array with minimal or no runtime overhead.
//!
//! Currently, keys are limited to enums with a primitive representation. In the future,
//! it might be possible to support other types, possibly at the expense of not exposing
//! `const`-qualified methods for these key types or some runtime overhead.
//!
//! # Example
//! ```
//! use const_assoc::{assoc, PrimitiveEnum};
//!
//! #[repr(u8)]
//! #[derive(Copy, Clone, PrimitiveEnum)]
//! enum Letter {
//!     A,
//!     B,
//!     C,
//! }
//!
//! let letters = assoc! {
//!     Letter::A => 'a',
//!     Letter::B => 'b',
//!     Letter::C => 'c',
//! };
//!
//! assert_eq!(letters[Letter::A], 'a');
//! assert_eq!(letters[Letter::C], 'c');
//! ```

mod utils;

use crate::utils::{
    assume_init_array, into_usize, transmute_safe, ConstIntoUSize, ConstUSize, Is, IsConstUSize,
    TransmuteSafe,
};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Index, IndexMut};
use derive_where::derive_where;

// Re-export `const_default::ConstDefault`.
pub use const_default::ConstDefault;

// Re-export the derive macro for `PrimitiveEnum`.
pub use const_assoc_derive::PrimitiveEnum;

/// Provides an easy, const-friendly way to construct a new [`Assoc`] instance.
///
/// # Example
/// ```
/// use const_assoc::{assoc, PrimitiveEnum};
///
/// #[repr(u8)]
/// #[derive(Copy, Clone, PrimitiveEnum)]
/// enum Letter {
///     A,
///     B,
///     C,
/// }
///
/// let letters = assoc! {
///     Letter::A => 'a',
///     Letter::B => 'b',
///     Letter::C => 'c',
/// };
/// ```
#[macro_export]
macro_rules! assoc {
    ($($key:expr => $value:expr),* $(,)?) => {
        {
            let phantom_values = $crate::assoc_macro_private::PhantomArray::new(&[$($value),*]);

            if $crate::assoc_macro_private::has_duplicate_keys(&[$($key),*], phantom_values) {
                panic!("A `ConstArrayMap` cannot have two values with identical keys.");
            }

            let mut map = $crate::Assoc::<_, _>::new_uninit();

            $(
                *map.const_get_mut($key) = ::core::mem::MaybeUninit::new($value);
            )*

            // SAFETY:
            // - `has_duplicate_keys` ensures that the code won't compile if
            //   there are not as many keys as values.
            // - `has_duplicate_keys` checks for duplicate keys and panics
            //   otherwise.
            //
            // Thus, since there are exactly as many keys as values and no
            // duplicate keys, each key corresponds to exactly one value and
            // each value corresponds to a unique key, which implies that
            // `map` must have been fully initialized.
            unsafe { map.assume_init() }
        }
    };
}

#[doc(hidden)]
pub mod assoc_macro_private {
    use crate::{key_to_index, Key, KeyImpl};
    use core::marker::PhantomData;

    pub const fn has_duplicate_keys<K: Key, V, const N: usize>(
        keys: &[K; N],
        _values: PhantomArray<V, N>,
    ) -> bool
    where
        K::Impl: KeyImpl<Storage<V> = [V; N]>,
    {
        let mut i = 0;

        while i < N {
            let mut j = i + 1;

            while j < N {
                if key_to_index(keys[i]) == key_to_index(keys[j]) {
                    return true;
                }

                j += 1;
            }

            i += 1;
        }

        false
    }

    pub struct PhantomArray<T, const N: usize>(PhantomData<[T; N]>);

    impl<T, const N: usize> PhantomArray<T, N> {
        #[inline(always)]
        pub const fn new(_array: &[T; N]) -> Self {
            PhantomArray(PhantomData)
        }
    }
}

/// Associates keys with values with minimal or no runtime overhead.
#[repr(transparent)]
pub struct Assoc<K: Key, V> {
    storage: <K::Impl as KeyImpl>::Storage<V>,
}

impl<K: Key, V: ConstDefault> ConstDefault for Assoc<K, V>
where
    <K::Impl as KeyImpl>::Storage<V>: ConstDefault,
{
    const DEFAULT: Self = Self {
        storage: <<K::Impl as KeyImpl>::Storage<V>>::DEFAULT,
    };
}

impl<K: Key, V: ConstDefault> Default for Assoc<K, V>
where
    <K::Impl as KeyImpl>::Storage<V>: Default,
{
    fn default() -> Self {
        Self {
            storage: <<K::Impl as KeyImpl>::Storage<V> as Default>::default(),
        }
    }
}

impl<K: Key, V, const N: usize> Assoc<K, V>
where
    K::Impl: KeyImpl<Storage<V> = [V; N]>,
{
    pub const LEN: usize = N;

    pub const fn from_values(values: [V; N]) -> Self {
        Self { storage: values }
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        Self::LEN
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a reference to the value associated with the given key.
    #[inline(always)]
    pub fn get(&self, key: K) -> &V {
        let idx = key_to_index(key);
        // SAFETY: The invariant of `KeyImpl` guarantees that
        // `idx` is always less than `self.storage.len()`.
        unsafe { self.storage.get_unchecked(idx) }
    }

    /// Returns a reference to the value associated with the given key.
    ///
    /// This version does bounds-checking therefore can be used in const
    /// contexts, unlike `get`.
    #[inline(always)]
    pub const fn const_get(&self, key: K) -> &V {
        let idx = key_to_index(key);
        &self.storage[idx]
    }

    /// Returns a mutable reference to the value associated with the given key.
    #[inline(always)]
    pub fn get_mut(&mut self, key: K) -> &mut V {
        let idx = key_to_index(key);
        // SAFETY: The invariant of `KeyImpl` guarantees that
        // `idx` is always less than `self.storage.len()`.
        unsafe { self.storage.get_unchecked_mut(idx) }
    }

    /// Returns a mutable reference to the value associated with the given key.
    ///
    /// This version does bounds-checking, therefore can be used in const
    /// contexts, unlike `get`.
    #[inline(always)]
    pub const fn const_get_mut(&mut self, key: K) -> &mut V {
        let idx = key_to_index(key);
        &mut self.storage[idx]
    }

    /// Takes `self` by value and returns an iterator over all the values
    /// stored in this map.
    pub fn into_values(self) -> impl Iterator<Item = V> {
        self.storage.into_iter()
    }

    /// Returns an iterator over shared references to all values stored in this
    /// map in arbitrary order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.storage.iter()
    }

    /// Returns an iterator over mutable references to all values stored in
    /// this map in arbitrary order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.storage.iter_mut()
    }
}

impl<K: Key, V, const N: usize> Index<K> for Assoc<K, V>
where
    K::Impl: KeyImpl<Storage<V> = [V; N]>,
{
    type Output = V;

    #[inline(always)]
    fn index(&self, index: K) -> &Self::Output {
        self.get(index)
    }
}

impl<K: Key, V, const N: usize> IndexMut<K> for Assoc<K, V>
where
    K::Impl: KeyImpl<Storage<V> = [V; N]>,
{
    #[inline(always)]
    fn index_mut(&mut self, index: K) -> &mut Self::Output {
        self.get_mut(index)
    }
}

impl<K: Key, V, const N: usize> Assoc<K, MaybeUninit<V>>
where
    K::Impl: KeyImpl<Storage<V> = [V; N]>,
    K::Impl: KeyImpl<Storage<MaybeUninit<V>> = [MaybeUninit<V>; N]>,
{
    pub const fn new_uninit() -> Self {
        // SAFETY: we are transmute an uninitialized array to an array with
        // uninitialized elements, which is always safe.
        let storage: [MaybeUninit<V>; N] = unsafe { MaybeUninit::uninit().assume_init() };
        Self { storage }
    }

    /// Interprets this map as fully initialized, meaning that all its values
    /// have been given a concrete value.
    ///
    /// # Safety
    /// The caller must ensure that the map has actually been fully
    /// initialized, meaning that all values have been given an initialized
    /// value such that calling `MaybeUninit::assume_init` on them would be
    /// safe.
    pub const unsafe fn assume_init(self) -> Assoc<K, V> {
        Assoc {
            // SAFETY: the caller guarantees that all elements of
            // `self.storage` have been initialized.
            storage: unsafe { assume_init_array(self.storage) },
        }
    }
}

/// Describes a key type for [Assoc].
///
/// # Safety
/// Whenever `Storage<V>` is an array `[V; N]` for some N, `Self` must be less
/// than `N` when converted to `usize` via `key_impl_to_index`.
unsafe trait KeyImpl: Copy + TransmuteSafe<Self::Repr> {
    type Storage<V>;
    type Repr: Copy + ConstIntoUSize;
}

#[doc(hidden)]
#[inline(always)]
pub const fn key_to_index<K: Key>(key: K) -> usize {
    let key_impl = transmute_safe(key);
    key_impl_to_index(key_impl)
}

#[inline(always)]
const fn key_impl_to_index<K: KeyImpl>(key: K) -> usize {
    let repr: K::Repr = transmute_safe(key);
    into_usize(repr)
}

/// Indirectly defines a way to use `Self` as a key for [Assoc].
trait Key: TransmuteSafe<Self::Impl> {
    type Impl: KeyImpl;
}

#[repr(transparent)]
#[derive_where(Copy, Clone)]
struct EnumKeyImpl<T: PrimitiveEnum, _U>(T, PhantomData<_U>);

// SAFETY: `EnumKey<T>` has the same representation as `T`.
unsafe impl<T: PrimitiveEnum, _U> TransmuteSafe<EnumKeyImpl<T, _U>> for T {}

// SAFETY: `EnumKey<T>` has the same representation as `T`, while `T` has the
// same representation as `<T::Layout as EnumLayoutTrait>::Discriminant`, since
// `PrimitiveEnum` implies `TransmuteSafe<<T::Layout as EnumLayoutTrait>::Discriminant>`.
unsafe impl<T: PrimitiveEnum, _U>
    TransmuteSafe<<T::Layout as PrimitiveEnumLayoutTrait>::Discriminant> for EnumKeyImpl<T, _U>
{
}

impl<T: PrimitiveEnum> Key for T
where
    EnumKeyImpl<T, <<T as PrimitiveEnum>::Layout as PrimitiveEnumLayoutTrait>::MaxVariants>:
        KeyImpl,
{
    type Impl = EnumKeyImpl<T, <T::Layout as PrimitiveEnumLayoutTrait>::MaxVariants>;
}

/// Indicates that `Self` is a primitive enum type, meaning that it is an enum
/// with a `#[repr(primitive_type)]` attribute.
///
/// # Safety
/// The implementors must ensure that `Layout` exactly describes `Self`.
pub unsafe trait PrimitiveEnum: Copy {
    /// The layout of `Self`.
    type Layout: PrimitiveEnumLayoutTrait;
}

// SAFETY: The invariant of `PrimitiveEnum` implies that `Self` always
// represents a valid enum discriminant when converted to usize, so it must be
// a non-negative integer that is less than `MAX_VARIANTS`.
unsafe impl<T: PrimitiveEnum, const MAX_VARIANTS: usize> KeyImpl
    for EnumKeyImpl<T, ConstUSize<MAX_VARIANTS>>
where
    <<T as PrimitiveEnum>::Layout as PrimitiveEnumLayoutTrait>::MaxVariants:
        Is<ConstUSize<MAX_VARIANTS>>,
    EnumKeyImpl<T, ConstUSize<MAX_VARIANTS>>:
        TransmuteSafe<<<T as PrimitiveEnum>::Layout as PrimitiveEnumLayoutTrait>::Discriminant>,
{
    type Storage<V> = [V; MAX_VARIANTS];
    type Repr = <<T as PrimitiveEnum>::Layout as PrimitiveEnumLayoutTrait>::Discriminant;
}

// See `PrimitiveEnumLayout`.
trait PrimitiveEnumLayoutTrait {
    type Discriminant: Copy + ConstIntoUSize;
    type MaxVariants: IsConstUSize;
}

/// Describes the layout of an enum with a `#[repr(primitive_type)]` attribute.
///
/// # Parameters
/// * `Discriminant` - The underlying numerical type used to represent enum variants.
/// * `MAX_MAX_VARIANTS` - The maximum number of variants this enum can have, equal to
///   the greatest discriminant value among the enum's variants plus 1.
pub struct PrimitiveEnumLayout<Discriminant, const MAX_MAX_VARIANTS: usize> {
    _marker: PhantomData<Discriminant>,
}

impl<Discriminant, const MAX_VARIANTS: usize> PrimitiveEnumLayoutTrait
    for PrimitiveEnumLayout<Discriminant, MAX_VARIANTS>
where
    Discriminant: Copy + ConstIntoUSize,
{
    type Discriminant = Discriminant;
    type MaxVariants = ConstUSize<MAX_VARIANTS>;
}
