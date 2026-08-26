#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(windows)]
mod app;

#[cfg(windows)]
mod recorder;

fn main() {
    #[cfg(windows)]
    app::run();

    #[cfg(not(windows))]
    eprintln!("This screen recorder only runs on Windows.");
}
