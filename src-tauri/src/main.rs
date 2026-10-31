// в релизной сборке не открывать консольное окно на Windows
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    audio_graph_lib::run()
}
