use std::{fmt, sync::Arc};

pub struct Callback<T, R> {
    cb: Arc<dyn Fn(T) -> R + 'static + Send + Sync>,
    #[cfg(debug_assertions)]
    dbg: (&'static str, &'static std::panic::Location<'static>),
}
impl<T, R> Clone for Callback<T, R> {
    fn clone(&self) -> Self {
        Self {
            cb: self.cb.clone(),
            #[cfg(debug_assertions)]
            dbg: self.dbg,
        }
    }
}
impl<T, R> fmt::Debug for Callback<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_tuple(std::any::type_name::<Self>());

        #[cfg(debug_assertions)]
        dbg.field(&fmt::from_fn(|f| {
            let (fn_type_name, fn_location) = self.dbg;
            write!(f, "{fn_type_name} @ {fn_location}")
        }));

        dbg.finish()
    }
}
impl<T, R> Callback<T, R> {
    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn from_fn_base<Base, F>(base: Base, to_callback: impl FnOnce(Base) -> F) -> Self
    where
        F: Fn(T) -> R + 'static + Send + Sync,
    {
        Self {
            cb: Arc::new(to_callback(base)),
            #[cfg(debug_assertions)]
            dbg: (
                std::any::type_name::<Base>(),
                std::panic::Location::caller(),
            ),
        }
    }

    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn from_fn(callback: impl Fn(T) -> R + 'static + Send + Sync) -> Self {
        Self::from_fn_base(callback, |cb| cb)
    }

    #[inline]
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn from_fn_ctx<C: 'static + Send + Sync>(
        ctx: C,
        callback: impl Fn(&C, T) -> R + 'static + Send + Sync,
    ) -> Self {
        Self::from_fn_base(callback, move |cb| move |arg| cb(&ctx, arg))
    }

    pub fn call(&self, arg: T) -> R {
        (self.cb)(arg)
    }
}
impl<T, R> From<&Callback<T, R>> for Callback<T, R> {
    fn from(value: &Callback<T, R>) -> Self {
        value.clone()
    }
}
