//! Этап 8: постоянный рабочий цикл домашней работы без зависимости от GUI.
//!
//! - `dto` — точные camelCase DTO и исчерпывающие enum-преобразования;
//! - `repository` — SQL-граница (чтение и запись поверх `&Connection`);
//! - `bundle` — неизменяемый SourceBundle из подборки материалов этапа 7;
//! - `projections` — Readiness, IterationChain, сравнение версий, Главная.

pub mod bundle;
pub mod dto;
pub mod projections;
pub mod repository;

#[cfg(test)]
mod tests;
