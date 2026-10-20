// Минимальная оболочка. Бизнес-логики здесь нет: команды backend
// регистрируются здесь позже (см. FRONTEND_HANDOFF.md, раздел «Подключение backend»).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kontur_lib::run()
}
