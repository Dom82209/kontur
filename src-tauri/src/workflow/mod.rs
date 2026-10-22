//! Этап 8: постоянный рабочий цикл домашней работы без зависимости от GUI.
//!
//! - `dto` — точные camelCase DTO и исчерпывающие enum-преобразования;
//! - `repository` — SQL-граница (чтение и запись поверх `&Connection`);
//! - `bundle` — неизменяемый SourceBundle из подборки материалов этапа 7;
//! - `projections` — Readiness, IterationChain, сравнение версий, Главная;
//! - `state_machine` — единственная таблица допустимых переходов девяти этапов;
//! - `service` — транзакционные команды: идемпотентность, ревизия, журнал, события.

pub mod bundle;
pub mod dto;
pub mod projections;
pub mod repository;
pub mod service;
pub mod state_machine;

#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod tests;
