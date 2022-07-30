use core::mem::{ManuallyDrop, MaybeUninit};
use core::ops::{Deref, DerefMut};
use core::pin::Pin;

use futures::Future;

#[derive(Clone, Copy)]
#[repr(align(64))]
struct A64;

#[derive(Clone, Copy)]
pub struct AlignedBuffer<const SIZE: usize> {
    _ensure_alignment: A64,
    inner: [u8; SIZE],
}

impl<const SIZE: usize> AlignedBuffer<SIZE> {
    const fn zeroed() -> Self {
        Self {
            _ensure_alignment: A64,
            inner: [0; SIZE],
        }
    }
}

impl<const SIZE: usize> Deref for AlignedBuffer<SIZE> {
    type Target = [u8; SIZE];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<const SIZE: usize> DerefMut for AlignedBuffer<SIZE> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

macro_rules! count_idents {
    () => { 0 };
    ($x:ident) => { 1 };
    ($x:ident, $($xs:ident),*) => {
        1 + count_idents!($($xs),*)
    }
}

macro_rules! impl_unit_fut_tuple {
    ($($generic_ident:ident),*) => {
        impl<$($generic_ident: Future<Output = ()> + 'static),*> UnitFutTuple<{ count_idents!($($generic_ident),*)}> for ($($generic_ident,)*) {
            #[allow(non_snake_case)]
            fn tuple_to_pin_array(
                tuple: &'static mut Self,
            ) -> [Pin<&'static mut dyn Future<Output = ()>>; { count_idents!($($generic_ident),*)}] {
                let ($(ref mut $generic_ident,)*) = tuple;
                unsafe {
                    [
                        $(Pin::new_unchecked($generic_ident),)*
                    ]
                }
            }
        }
    };
}

pub trait UnitFutTuple<const NUM_ELEMENTS: usize>: 'static {
    fn tuple_to_pin_array(
        tuple: &'static mut Self,
    ) -> [Pin<&'static mut dyn Future<Output = ()>>; NUM_ELEMENTS];
}

impl_unit_fut_tuple!(F1);
impl_unit_fut_tuple!(F1, F2);
impl_unit_fut_tuple!(F1, F2, F3);
impl_unit_fut_tuple!(F1, F2, F3, F4);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7, F8);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7, F8, F9);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7, F8, F9, F10);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11);
impl_unit_fut_tuple!(F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12);

pub fn get_pin_muts<FutTuple: UnitFutTuple<NUM_FUTS>, const NUM_FUTS: usize, const SIZE: usize>(
    futs: FutTuple,
    buffer_to_insert_futs: &'static mut AlignedBuffer<SIZE>,
) -> [Pin<&'static mut dyn Future<Output = ()>>; NUM_FUTS] {
    {
        let size = core::mem::size_of::<FutTuple>();
        let futs_alignment = core::mem::align_of::<FutTuple>();
        assert!(
            SIZE >= size,
            "not enough bytes in the buffer to store the futures"
        );
        assert!(
            core::mem::align_of::<AlignedBuffer<SIZE>>() >= futs_alignment,
            "aligned buffer has insufficient alignment to hold the futures"
        );
    }
    let futs: &'static mut FutTuple = unsafe {
        union Helper<Inner, const SIZE: usize> {
            inner: ManuallyDrop<MaybeUninit<Inner>>,
            _buffer: AlignedBuffer<SIZE>,
        }
        let banana: &'static mut Helper<_, SIZE> =
            &mut *buffer_to_insert_futs.as_mut_ptr().cast::<Helper<_, SIZE>>();
        banana.inner.write(futs)
    };

    FutTuple::tuple_to_pin_array(futs)
}

#[macro_export]
macro_rules! dont_use_val {
    ($anything:tt) => {
        #[allow(unreachable_code)]
        {
            unreachable!()
        }
    };
}

pub const fn store_futs_statically<FutTuple, const SIZE: usize>(
    __type_inference_hack: ManuallyDrop<Option<FutTuple>>,
) -> AlignedBuffer<SIZE> {
    // Verify that the buffer is big enough to store our futures
    {
        let size = core::mem::size_of::<FutTuple>();
        let futs_alignment = core::mem::align_of::<FutTuple>();
        assert!(
            SIZE >= size,
            "not enough bytes in the buffer to store the futures"
        );
        assert!(
            core::mem::align_of::<AlignedBuffer<SIZE>>() >= futs_alignment,
            "aligned buffer has insufficient alignment to hold the futures"
        );
    }
    AlignedBuffer::zeroed()
}

#[macro_export]
macro_rules! store_futs_statically {
    ($storage_size:expr; $(
        $async_fun:ident($($param:tt),*)
    ),*) => {{
        use $crate::static_fut_storage::{AlignedBuffer, store_futs_statically, get_pin_muts};
        use ::core::mem::ManuallyDrop;

        static mut FUT_STORAGE: AlignedBuffer<{ $storage_size }> = store_futs_statically({
            ManuallyDrop::new(if true {
                None
            } else {
                // stupid type inference hack lol
                Some(
                    ($(
                        $async_fun($($crate::dont_use_val!($param)),*)
                    ),*)
                )
            })
        });

        get_pin_muts(($(
            $async_fun($($param),*)
        ),*), unsafe { &mut FUT_STORAGE })
    }};
}
