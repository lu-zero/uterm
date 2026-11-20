fn main() -> std::process::ExitCode {
    if let Err(err) = uterm::run() {
        eprintln!("uterm: {err}");
        std::process::ExitCode::from(1)
    } else {
        std::process::ExitCode::SUCCESS
    }
}
