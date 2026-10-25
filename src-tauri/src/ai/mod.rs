//! Этап 9: безопасный рабочий процесс Codex/Claude без хранения учётных
//! данных и без GUI-автоматизации.
//!
//! - `locate` — поиск официального CLI только в фиксированных каталогах;
//! - `process` — запуск argv без shell, чистое окружение, тайм-аут, отмена;
//! - `adapters` — Codex CLI (`codex exec`) и Claude Code (`claude -p`),
//!   проверка версии/флагов/входа, разбор вывода;
//! - `context` — ровно тот текст, который увидит провайдер, с лимитом объёма;
//! - `response` — разбор ответа в черновик (полный/частичный/повреждённый);
//! - `redact` — журнал без секретов, почты и абсолютных путей;
//! - `store` — SQL заданий `ai_jobs` (схема v7) и флага подключения CLI;
//! - `service` — статусы провайдеров, подготовка, отправка, отмена, ответы.

pub mod adapters;
pub mod context;
pub mod isolation;
pub mod locate;
pub mod model;
pub mod process;
pub mod redact;
pub mod response;
pub mod service;
pub mod store;

#[cfg(test)]
mod tests;

pub use service::{AiEventSink, AiService, ProviderRuntime, TransportPreference};
