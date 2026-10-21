//! Детерминированные точки вмешательства для race-тестов.
//!
//! В production-сборке `fire` — пустая inline-функция: замыкание не вызывается,
//! данные точки не создаются. В `cfg(test)` хук устанавливается thread-local и
//! вызывается синхронно в том же потоке, поэтому тест точно знает, между какими
//! системными вызовами происходит подмена, без sleep и без гонок между тестами.

use std::ffi::OsString;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum HookPoint {
    /// Родитель уже открыт (и при необходимости создан), temp ещё не создан.
    BeforeTempCreate,
    /// Temp записан и проверен, `renameat` ещё не вызван.
    BeforeRename,
    /// Файл опубликован, финальная проверка ещё не выполнена.
    BeforeFinalVerify,
    /// Guard собирается удалить созданный операцией файл.
    BeforeCleanup { file_name: OsString },
    /// Planning открыл родительский каталог назначения.
    PlanningParentOpened,
    /// Undo-validation открыла родительский каталог проверяемого файла.
    UndoParentOpened,
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn fire(_point: impl FnOnce() -> HookPoint) {}

#[cfg(test)]
pub(crate) use imp::{fire, install};

#[cfg(test)]
mod imp {
    use std::cell::RefCell;

    use super::HookPoint;

    type Hook = Box<dyn FnMut(&HookPoint)>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) struct HookGuard;

    impl Drop for HookGuard {
        fn drop(&mut self) {
            HOOK.with(|slot| slot.borrow_mut().take());
        }
    }

    pub(crate) fn install(hook: impl FnMut(&HookPoint) + 'static) -> HookGuard {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        HookGuard
    }

    pub(crate) fn fire(point: impl FnOnce() -> HookPoint) {
        let taken = HOOK.with(|slot| slot.borrow_mut().take());
        if let Some(mut hook) = taken {
            hook(&point());
            HOOK.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(hook);
                }
            });
        }
    }
}
