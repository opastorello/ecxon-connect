// Ecxon Connect — entry point.
// Em release, o subsystem de Windows é definido para esconder o console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ecxon_connect_lib::run();
}
