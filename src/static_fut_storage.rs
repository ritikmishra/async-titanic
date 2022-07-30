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

// 1. store them statically

union Helper<Inner, const SIZE: usize> {
    inner: ManuallyDrop<MaybeUninit<Inner>>,
    buffer: AlignedBuffer<SIZE>,
}
pub const fn store_futs_statically<F1, F2, const SIZE: usize>(
    __type_inference_hack: ManuallyDrop<Option<(F1, F2)>>,
) -> AlignedBuffer<SIZE> {
    {
        let size = core::mem::size_of::<(F1, F2)>();
        let futs_alignment = core::mem::align_of::<(F1, F2)>();
        assert!(
            SIZE >= size,
            "not enough bytes in the buffer to store the futures"
        );
        assert!(
            core::mem::align_of::<AlignedBuffer<SIZE>>() >= futs_alignment,
            "aligned buffer has insufficient alignment to hold the futures"
        );
    }

    unsafe {
        let ret: Helper<(F1, F2), SIZE> = Helper {
            buffer: (AlignedBuffer::zeroed()),
        };

        ret.buffer
    }
}

// 2. from the static allocation, produce some Pin<&mut dyn Future<Output = ()>>

pub fn get_pin_muts<
    F1: Future<Output = ()> + 'static,
    F2: Future<Output = ()> + 'static,
    const SIZE: usize,
>(
    futs: (F1, F2),
    aa: &'static mut AlignedBuffer<SIZE>,
) -> [Pin<&'static mut dyn Future<Output = ()>>; 2] {
    {
        let size = core::mem::size_of::<(F1, F2)>();
        let futs_alignment = core::mem::align_of::<(F1, F2)>();
        assert!(
            SIZE >= size,
            "not enough bytes in the buffer to store the futures"
        );
        assert!(
            core::mem::align_of::<AlignedBuffer<SIZE>>() >= futs_alignment,
            "aligned buffer has insufficient alignment to hold the futures"
        );
    }
    let banana: &'static mut (F1, F2) = unsafe {
        let banana: &'static mut Helper<_, SIZE> = &mut *aa.as_mut_ptr().cast::<Helper<_, SIZE>>();
        banana.inner.write(futs)
    };

    unsafe {
        [
            Pin::new_unchecked(&mut banana.0),
            Pin::new_unchecked(&mut banana.1),
        ]
    }
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
